//! The once-cells of the reader session and the counter of the cached bytes.
//!
//! Under `std`, the cells are `OnceLock` values and the counter is an `AtomicUsize`, so a
//! [`super::Pdb`] is `Sync` if its source is `Sync`. Without `std`, the cells are `OnceCell`
//! values and the counter is a `Cell<usize>`, as the MSFZ chunk cache uses a `RefCell`.
//!
//! `get_or_try_init` is not stable. Thus [`Cache::get_or_load`] calls `get`, loads, and then
//! stores with `get_or_init`. Two threads can load the same structure at the same time. The
//! cell keeps one result, and the other thread releases its byte count. A failed load stores
//! nothing.

use crate::{Error, ErrorKind, Result};

/// A cell that the session fills once.
#[cfg(feature = "std")]
pub(super) type Cell<T> = std::sync::OnceLock<T>;

/// A cell that the session fills once.
#[cfg(not(feature = "std"))]
pub(super) type Cell<T> = core::cell::OnceCell<T>;

/// The number of cached bytes.
#[cfg(feature = "std")]
#[derive(Debug, Default)]
struct Counter(core::sync::atomic::AtomicUsize);

#[cfg(feature = "std")]
impl Counter {
    /// Add `bytes` if the sum is not larger than `limit`. Return `false` if it is larger.
    fn reserve(&self, bytes: usize, limit: usize) -> bool {
        use core::sync::atomic::Ordering;
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|&total| total <= limit)
            })
            .is_ok()
    }

    /// Subtract `bytes` that [`Counter::reserve`] added.
    fn release(&self, bytes: usize) {
        self.0
            .fetch_sub(bytes, core::sync::atomic::Ordering::AcqRel);
    }
}

/// The number of cached bytes.
#[cfg(not(feature = "std"))]
#[derive(Debug, Default)]
struct Counter(core::cell::Cell<usize>);

#[cfg(not(feature = "std"))]
impl Counter {
    /// Add `bytes` if the sum is not larger than `limit`. Return `false` if it is larger.
    fn reserve(&self, bytes: usize, limit: usize) -> bool {
        match self.0.get().checked_add(bytes) {
            Some(total) if total <= limit => {
                self.0.set(total);
                true
            }
            _ => false,
        }
    }

    /// Subtract `bytes` that [`Counter::reserve`] added.
    fn release(&self, bytes: usize) {
        self.0.set(self.0.get() - bytes);
    }
}

/// The byte limit of the session cache and the bytes that the cells hold.
#[derive(Debug)]
pub(super) struct Cache {
    /// The bytes that the cells hold.
    used: Counter,
    /// The largest number of bytes that the cells can hold.
    limit: usize,
}

impl Cache {
    /// Make an empty cache with a limit of `limit` bytes.
    pub(super) fn new(limit: usize) -> Self {
        Self {
            used: Counter::default(),
            limit,
        }
    }

    /// Return the value of `cell`. If `cell` is empty, call `load` and store its value.
    ///
    /// `load` returns the value and the number of bytes that the value holds.
    ///
    /// # Errors
    ///
    /// - The errors of `load`.
    /// - [`ErrorKind::LimitExceeded`] if the value makes the cached bytes larger than the
    ///   limit. The cell then stays empty.
    pub(super) fn get_or_load<'c, T>(
        &self,
        cell: &'c Cell<T>,
        load: impl FnOnce() -> Result<(T, usize)>,
    ) -> Result<&'c T> {
        if let Some(value) = cell.get() {
            return Ok(value);
        }
        let (value, bytes) = load()?;
        if !self.used.reserve(bytes, self.limit) {
            return Err(Error::new(ErrorKind::LimitExceeded, None)
                .with_context("the session cache is larger than the cached byte limit"));
        }
        let mut stored = false;
        let value = cell.get_or_init(|| {
            stored = true;
            value
        });
        if !stored {
            self.used.release(bytes);
        }
        Ok(value)
    }
}
