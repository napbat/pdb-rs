//! The Edit and Continue local slot map.
//!
//! The blob is an optional baseline and then one entry for each slot of the
//! local signature (or each hoisted field of a state machine):
//!
//! 1. Optional: the byte `0xff` and the negated syntax offset baseline as a
//!    compressed unsigned integer. Without it, the baseline is -1.
//! 2. For each slot, a byte. 0 marks a short-lived temporary without other
//!    fields. Otherwise, bit 7 tells if an ordinal follows, and bits 0 to 6
//!    hold the Roslyn `SynthesizedLocalKind` plus 1. Then the syntax offset
//!    minus the baseline and the optional ordinal follow as compressed
//!    unsigned integers.

use alloc::vec::Vec;

use super::super::blob::{Reader, Writer, invalid};
use super::super::{Entries, count_to_end};
use super::{read_baseline, read_syntax_offset, write_baseline, write_syntax_offset};
use crate::Result;
use crate::bytes::{Bytes, into_owned};

/// The largest slot kind (`SynthesizedLocalKind`) that the map can hold.
pub const MAX_LOCAL_SLOT_KIND: u8 = 0x7d;

/// The byte that starts a stored baseline.
const BASELINE_MARKER: u8 = 0xff;

/// The flag of a slot byte that tells that an ordinal follows.
const HAS_ORDINAL: u8 = 0x80;

/// The bits of a slot byte that hold the kind plus 1.
const KIND_MASK: u8 = 0x7f;

/// The baseline when the blob does not store one.
const DEFAULT_BASELINE: i32 = -1;

/// One slot of an Edit and Continue local slot map.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LocalSlot {
    /// A short-lived temporary without information.
    Temporary,
    /// A long-lived local variable.
    Variable {
        /// The Roslyn `SynthesizedLocalKind`: 0 for a variable from the source
        /// code. The value is at most [`MAX_LOCAL_SLOT_KIND`].
        kind: u8,
        /// The syntax offset of the declaration.
        syntax_offset: i32,
        /// The ordinal among the slots with the same syntax offset and kind,
        /// if the blob stores one. Roslyn stores it only if it is not 0.
        ordinal: Option<u32>,
    },
}

/// An Edit and Continue local slot map.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct EncLocalSlotMap<'a> {
    /// The blob.
    bytes: Bytes<'a>,
    /// The blob offset of the slots, to the end of the blob.
    /// [`EncLocalSlotMap::parse`] checked them.
    slots: usize,
    /// The syntax offset baseline.
    baseline: i32,
    /// The number of slots.
    len: usize,
}

into_owned!(struct EncLocalSlotMap { bytes; slots, baseline, len });

impl<'a> EncLocalSlotMap<'a> {
    /// Decode a local slot map blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the
    ///   blob ends inside a slot or the baseline.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   compressed integer is not valid, if the byte `0xff` is not the first
    ///   byte, or if a slot byte holds the kind field 0 or `0x7f`.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(blob);
        let baseline = if blob.first() == Some(&BASELINE_MARKER) {
            reader.u8("local slot map is truncated")?;
            read_baseline(&mut reader)?
        } else {
            DEFAULT_BASELINE
        };
        let slots = reader.rest();
        let len = count_to_end(slots, baseline, read_slot)?;
        Ok(Self {
            bytes: Bytes::new(blob),
            slots: blob.len() - slots.len(),
            baseline,
            len,
        })
    }

    /// Return the blob bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the syntax offset baseline: the stored value, or -1.
    #[must_use]
    pub const fn syntax_offset_baseline(&self) -> i32 {
        self.baseline
    }

    /// Return the number of slots.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no slots.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the slots.
    #[must_use]
    pub fn iter(&self) -> EncLocalSlotMapIter<'_> {
        EncLocalSlotMapIter(Entries::new(
            &self.bytes[self.slots..],
            self.len,
            self.baseline,
            read_slot,
        ))
    }

    /// Encode a local slot map blob as Roslyn does, and append it to `out`.
    ///
    /// The encoder stores a baseline only if a syntax offset is smaller than -1.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   kind is larger than [`MAX_LOCAL_SLOT_KIND`].
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a
    ///   relative syntax offset or an ordinal does not fit in a compressed
    ///   integer, or if an allocation fails.
    ///
    /// On error, `out` does not change.
    pub fn encode_parts(
        slots: impl IntoIterator<Item = LocalSlot, IntoIter: Clone>,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let slots = slots.into_iter();
        let baseline = slots
            .clone()
            .filter_map(|slot| match slot {
                LocalSlot::Temporary => None,
                LocalSlot::Variable { syntax_offset, .. } => Some(syntax_offset),
            })
            .fold(DEFAULT_BASELINE, i32::min);
        let mut writer = Writer::append(out);
        if baseline != DEFAULT_BASELINE {
            writer.u8(BASELINE_MARKER)?;
            write_baseline(&mut writer, baseline)?;
        }
        for slot in slots {
            let LocalSlot::Variable {
                kind,
                syntax_offset,
                ordinal,
            } = slot
            else {
                writer.u8(0)?;
                continue;
            };
            if kind > MAX_LOCAL_SLOT_KIND {
                return Err(invalid("local slot kind is too large"));
            }
            let flag = if ordinal.is_some() { HAS_ORDINAL } else { 0 };
            writer.u8((kind + 1) | flag)?;
            write_syntax_offset(&mut writer, syntax_offset, baseline)?;
            if let Some(ordinal) = ordinal {
                writer.unsigned(ordinal)?;
            }
        }
        writer.finish();
        Ok(())
    }
}

iterable!(EncLocalSlotMap => EncLocalSlotMapIter, LocalSlot);

/// Read one slot.
fn read_slot(reader: &mut Reader<'_>, baseline: i32) -> Result<LocalSlot> {
    let byte = reader.u8("local slot map is truncated")?;
    if byte == 0 {
        return Ok(LocalSlot::Temporary);
    }
    if byte == BASELINE_MARKER {
        return Err(invalid("local slot map baseline is not at the start"));
    }
    let stored = byte & KIND_MASK;
    if stored == 0 || stored == KIND_MASK {
        return Err(invalid("local slot kind is not valid"));
    }
    let syntax_offset = read_syntax_offset(reader, baseline)?;
    let ordinal = if byte & HAS_ORDINAL == 0 {
        None
    } else {
        Some(reader.unsigned()?)
    };
    Ok(LocalSlot::Variable {
        kind: stored - 1,
        syntax_offset,
        ordinal,
    })
}
