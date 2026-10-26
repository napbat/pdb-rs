//! The optional debug header of the DBI stream.
//!
//! The substream is an array of 16-bit stream indexes. The value `0xffff`
//! marks an empty slot. [`DebugStream`] names the first 11 slots. Writers can
//! add more slots, so the decoder and the encoder keep all slots.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{stream_id, stream_index};
use crate::Result;
use crate::native::msf::StreamId;
use crate::native::output::Output;

/// A slot in the optional debug header.
///
/// Each slot holds the index of a stream with one kind of debug data.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DebugStream {
    /// Old frame pointer omission data.
    Fpo,
    /// Exception data.
    Exception,
    /// Fixup data.
    Fixup,
    /// The address map from the image to the source image.
    OmapToSource,
    /// The address map from the source image to the image.
    OmapFromSource,
    /// The section headers of the image.
    SectionHeader,
    /// The token to RID map.
    TokenRidMap,
    /// The `.xdata` section contents.
    Xdata,
    /// The `.pdata` section contents.
    Pdata,
    /// New frame pointer omission data.
    NewFpo,
    /// The section headers of the image before address remapping.
    OriginalSectionHeader,
}

impl DebugStream {
    /// Return the slot index of this kind in the optional debug header.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// An iterator over the slots of the optional debug header.
///
/// Each item is `None` for a slot that holds the nil stream index.
#[derive(Clone, Debug)]
pub struct DebugStreams<'a> {
    /// The remaining slots.
    pub(super) slots: core::slice::Iter<'a, [u8; 2]>,
}

impl Iterator for DebugStreams<'_> {
    type Item = Option<StreamId>;

    fn next(&mut self) -> Option<Self::Item> {
        let slot = self.slots.next()?;
        Some(stream_id(u16::from_le_bytes(*slot)))
    }

    fn nth(&mut self, count: usize) -> Option<Self::Item> {
        let slot = self.slots.nth(count)?;
        Some(stream_id(u16::from_le_bytes(*slot)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl ExactSizeIterator for DebugStreams<'_> {}

impl FusedIterator for DebugStreams<'_> {}

/// Append an optional debug header with the given slots to `out`.
///
/// A `None` slot gets the nil index `0xffff`.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a
/// stream index is larger than `0xfffe` or if the output cannot grow.
pub fn encode_debug_streams<I>(slots: I, out: &mut Vec<u8>) -> Result<()>
where
    I: IntoIterator<Item = Option<StreamId>>,
    I::IntoIter: ExactSizeIterator,
{
    let slots = slots.into_iter();
    let mut output = Output::new(out);
    output.reserve(slots.len().saturating_mul(2))?;
    for slot in slots {
        output.u16(stream_index(slot)?)?;
    }
    Ok(())
}
