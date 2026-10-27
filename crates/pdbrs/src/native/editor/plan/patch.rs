//! Patched streams (#48): base bytes and fixed-width values that the plan
//! writes over them while it writes the output.
//!
//! A change that gives a stream only new indices or offsets keeps the size of
//! the stream. The plan then keeps the base and a sorted list of patches. It
//! holds no copy of the stream.

use alloc::vec::Vec;

use crate::io::ReadAt;
use crate::native::container::Stream;
use crate::{Error, ErrorKind, Result};

/// The size of the value of a patch.
pub(super) const PATCH_SIZE: u64 = 4;

/// A little-endian `u32` value at a stream offset.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Patch {
    /// The stream offset of the value.
    offset: u64,
    /// The value.
    value: u32,
}

impl Patch {
    /// Make a patch that writes `value` at the stream offset `offset`.
    pub(super) fn new(offset: usize, value: u32) -> Self {
        Self {
            offset: offset as u64,
            value,
        }
    }

    /// Return the offset after the value.
    const fn end(self) -> u64 {
        self.offset + PATCH_SIZE
    }
}

/// The bytes under the patches.
pub(super) enum Base<'e, S> {
    /// The source stream.
    Source(Stream<'e, S>),
    /// Bytes that the source session holds.
    Session(&'e [u8]),
}

impl<S: ReadAt> Base<'_, S> {
    /// Return the length of the base in bytes.
    fn len(&self) -> Result<u64> {
        match self {
            Self::Source(stream) => stream.len(),
            Self::Session(bytes) => ReadAt::len(*bytes),
        }
    }
}

/// A stream with patches.
pub(in super::super) struct Patched<'e, S> {
    /// The bytes under the patches.
    base: Base<'e, S>,
    /// The patches in offset order. Two patches do not overlap.
    patches: Vec<Patch>,
}

impl<'e, S: ReadAt> Patched<'e, S> {
    /// Put the patches on `base`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if two patches overlap or a
    /// patch is outside the base.
    pub(super) fn new(base: Base<'e, S>, mut patches: Vec<Patch>) -> Result<Self> {
        patches.sort_unstable();
        let length = base.len()?;
        let overlap = patches
            .windows(2)
            .any(|pair| pair[0].end() > pair[1].offset);
        if overlap || patches.last().is_some_and(|last| last.end() > length) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("a stream patch overlaps another patch or the stream end"));
        }
        Ok(Self { base, patches })
    }
}

impl<S: ReadAt> ReadAt for Patched<'_, S> {
    fn len(&self) -> Result<u64> {
        self.base.len()
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        match &self.base {
            Base::Source(stream) => stream.read_exact_at(offset, buffer)?,
            Base::Session(bytes) => bytes.read_exact_at(offset, buffer)?,
        }
        // The base read checked the range, so the end does not overflow.
        let end = offset + buffer.len() as u64;
        let index = |value: u64| {
            usize::try_from(value).map_err(|_| Error::new(ErrorKind::OffsetOverflow, None))
        };
        let first = self.patches.partition_point(|patch| patch.end() <= offset);
        for patch in &self.patches[first..] {
            if patch.offset >= end {
                break;
            }
            let value = patch.value.to_le_bytes();
            let start = patch.offset.max(offset);
            let stop = patch.end().min(end);
            // Both ranges are inside the buffer and inside the value.
            buffer[index(start - offset)?..index(stop - offset)?]
                .copy_from_slice(&value[index(start - patch.offset)?..index(stop - patch.offset)?]);
        }
        Ok(())
    }
}
