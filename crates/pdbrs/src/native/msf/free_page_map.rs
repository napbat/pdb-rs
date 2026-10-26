//! Free page map checks for Big MSF 7.00.
//!
//! The active free page map (FPM) is a bitmap with one bit for each declared page. Bit `p % 8`
//! of byte `p / 8` describes page `p`. A set bit marks a free page. The bitmap has
//! `ceil(page_count / 8)` bytes. Its bytes continue across the FPM pages `active`,
//! `active + page_size`, `active + 2 * page_size` and so on (LLVM `getFpmStreamLayout` in
//! `MSFCommon.cpp`, Microsoft `MSF_HB::serializeFpm` in `msf.cpp`). Bits after the declared
//! pages in the last byte are ignored.
//!
//! The Microsoft PDB documentation `msf.md` and both writers (`MSF_HB` in `msf.cpp` and LLVM
//! `MSFBuilder`) define these page states:
//!
//! - The header page, both FPM pages of each interval, the page map, the stream directory and
//!   the pages of streams 1 and higher are allocated.
//! - The pages of stream 0 (the previous stream directory) are free after a commit. The
//!   committed FPM releases them (`fpm.add(fpmFreed)` in `MSF_HB::internalCommit`).
//! - Other pages are free.
//!
//! A referenced page that the FPM marks as free is an error. The pages of stream 0 can have
//! either state. An allocated page without a reference is a leak, not a structural error: the
//! report lists such pages.

use alloc::vec::Vec;
use core::ops::Range;

use super::{Msf, NIL_STREAM_SIZE, is_reserved_page};
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The result of [`Msf::check_free_page_map`].
///
/// The report keeps one range for each run of allocated pages without a reference. The number
/// of ranges is at most half the declared page count, rounded up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreePageMapReport {
    active: u32,
    old_directory_free: u32,
    old_directory_allocated: u32,
    unreferenced: Vec<Range<u32>>,
}

impl FreePageMapReport {
    /// Return the free page map that the report describes, 1 or 2.
    #[must_use]
    pub const fn active(&self) -> u32 {
        self.active
    }

    /// Return the number of stream 0 pages that the free page map marks as free.
    #[must_use]
    pub const fn old_directory_free_pages(&self) -> u32 {
        self.old_directory_free
    }

    /// Return the number of stream 0 pages that the free page map marks as allocated.
    #[must_use]
    pub const fn old_directory_allocated_pages(&self) -> u32 {
        self.old_directory_allocated
    }

    /// Return the runs of allocated pages that no structure references, in page order.
    #[must_use]
    pub fn unreferenced_allocated(&self) -> &[Range<u32>] {
        &self.unreferenced
    }

    /// Return the number of allocated pages that no structure references.
    #[must_use]
    pub fn unreferenced_allocated_count(&self) -> u32 {
        self.unreferenced
            .iter()
            .map(|run| run.end - run.start)
            .sum()
    }
}

impl<S: ReadAt> Msf<S> {
    /// Read the active free page map and compare it with the page references.
    ///
    /// The function uses memory for one page and for a sorted copy of the stream directory
    /// page lists. It reads each FPM page that holds the bitmap once.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the free page map marks a referenced page as free.
    ///   The error offset is the file offset of the bitmap byte.
    /// - [`ErrorKind::InvalidFormat`] if an FPM page is not a declared page.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    /// - An error from the source if a read fails.
    pub fn check_free_page_map(&self) -> Result<FreePageMapReport> {
        let (required, old) = self.sorted_references()?;
        let mut walk = Walk {
            block_size: self.geometry.block_size,
            required: required.iter().copied().peekable(),
            old: old.iter().copied().peekable(),
            report: FreePageMapReport {
                active: self.active_fpm,
                old_directory_free: 0,
                old_directory_allocated: 0,
                unreferenced: Vec::new(),
            },
        };
        let block_size = self.geometry.block_size;
        let page_count = self.geometry.page_count;
        let bitmap_len = page_count.div_ceil(8);
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(to_usize(block_size)?)
            .map_err(|_| allocation_failed())?;
        buffer.resize(to_usize(block_size)?, 0);
        let mut page = 0_u32;
        let mut done = 0_u32;
        let mut fpm_page = self.active_fpm;
        while done < bitmap_len {
            if fpm_page >= page_count {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("free page map page is not a declared page"));
            }
            let count = (bitmap_len - done).min(block_size);
            let bytes = &mut buffer[..to_usize(count)?];
            let base = self.geometry.offset(fpm_page);
            self.source.read_exact_at(base, bytes)?;
            for (at, &byte) in (base..).zip(bytes.iter()) {
                for bit in 0..8 {
                    if page == page_count {
                        break;
                    }
                    walk.visit(page, byte >> bit & 1 != 0, at)?;
                    page += 1;
                }
            }
            done += count;
            fpm_page = fpm_page.saturating_add(block_size);
        }
        Ok(walk.report)
    }
}

impl<S> Msf<S> {
    /// Return the sorted pages that must be allocated and the sorted pages of stream 0.
    fn sorted_references(&self) -> Result<(Vec<u32>, Vec<u32>)> {
        let old_count = self.streams.first().map_or(0, |entry| {
            if entry.size == NIL_STREAM_SIZE {
                0
            } else {
                entry.size.div_ceil(self.geometry.block_size)
            }
        });
        let (old_pages, stream_pages) = self
            .pages
            .split_at_checked(to_usize(old_count)?)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("stream 0 page list is longer than the directory page lists")
            })?;
        let mut old = Vec::new();
        old.try_reserve_exact(old_pages.len())
            .map_err(|_| allocation_failed())?;
        old.extend_from_slice(old_pages);
        old.sort_unstable();
        let mut required = Vec::new();
        required
            .try_reserve_exact(
                self.map_pages.len() + self.directory_pages.len() + stream_pages.len(),
            )
            .map_err(|_| allocation_failed())?;
        required.extend_from_slice(&self.map_pages);
        required.extend_from_slice(&self.directory_pages);
        required.extend_from_slice(stream_pages);
        required.sort_unstable();
        Ok((required, old))
    }
}

/// The state of a walk over the bitmap bits in page order.
struct Walk<R: Iterator<Item = u32>, O: Iterator<Item = u32>> {
    block_size: u32,
    required: core::iter::Peekable<R>,
    old: core::iter::Peekable<O>,
    report: FreePageMapReport,
}

impl<R: Iterator<Item = u32>, O: Iterator<Item = u32>> Walk<R, O> {
    /// Check the state of `page`. `at` is the file offset of its bitmap byte.
    fn visit(&mut self, page: u32, free: bool, at: u64) -> Result<()> {
        let reserved = is_reserved_page(page, self.block_size);
        if reserved || self.required.next_if_eq(&page).is_some() {
            if free {
                let context = if reserved {
                    "free page map marks the header page or a free page map page as free"
                } else {
                    "free page map marks a referenced page as free"
                };
                return Err(Error::new(ErrorKind::InvalidFormat, Some(at)).with_context(context));
            }
        } else if self.old.next_if_eq(&page).is_some() {
            if free {
                self.report.old_directory_free += 1;
            } else {
                self.report.old_directory_allocated += 1;
            }
        } else if !free {
            let runs = &mut self.report.unreferenced;
            match runs.last_mut() {
                Some(run) if run.end == page => run.end += 1,
                _ => {
                    runs.try_reserve(1).map_err(|_| allocation_failed())?;
                    runs.push(page..page + 1);
                }
            }
        }
        Ok(())
    }
}

/// Return the error for a failed allocation.
fn allocation_failed() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("free page map allocation failed")
}

/// Convert a page count to `usize`.
fn to_usize(value: u32) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("free page map count does not fit in memory")
    })
}
