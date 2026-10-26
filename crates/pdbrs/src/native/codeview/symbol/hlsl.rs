//! HLSL and data-parallel C++ (DPC) records.
//!
//! The layouts follow `DEFRANGESYMHLSL`, `DATASYMHLSL`, `DATASYMHLSL32`,
//! `DATASYMHLSL32_EX`, `LOCALDPCGROUPSHAREDSYM`, `DPCSYMTAGMAP` and
//! `CV_DPC_SYM_TAG_MAP_ENTRY` in `cvinfo.h`. `cvinfo.h` defines the DPC
//! structures only if `CC_DP_CXX` is set. The comment of `DEFRANGESYMHLSL`
//! names `S_DEFRANGE_HLSL` and `S_DEFRANGE_DPC_PTR_TAG`, so both kinds use
//! that layout.

use super::super::index::TypeIndex;
use super::super::{Fields, SymbolKind};
use super::{AddressGaps, AddressRange, LocalFlags};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The kind of an HLSL def-range (`S_DEFRANGE_HLSL`).
pub const S_DEFRANGE_HLSL: SymbolKind = SymbolKind(0x1150);
/// The kind of a global HLSL data symbol (`S_GDATA_HLSL`).
pub const S_GDATA_HLSL: SymbolKind = SymbolKind(0x1151);
/// The kind of a module-local HLSL data symbol (`S_LDATA_HLSL`).
pub const S_LDATA_HLSL: SymbolKind = SymbolKind(0x1152);
/// The kind of a DPC group-shared local variable (`S_LOCAL_DPC_GROUPSHARED`).
pub const S_LOCAL_DPC_GROUPSHARED: SymbolKind = SymbolKind(0x1154);
/// The kind of a DPC pointer tag def-range (`S_DEFRANGE_DPC_PTR_TAG`).
pub const S_DEFRANGE_DPC_PTR_TAG: SymbolKind = SymbolKind(0x1157);
/// The kind of a map from DPC pointer tags to symbols (`S_DPC_SYM_TAG_MAP`).
pub const S_DPC_SYM_TAG_MAP: SymbolKind = SymbolKind(0x1158);
/// The kind of a global HLSL data symbol with 32-bit slots (`S_GDATA_HLSL32`).
pub const S_GDATA_HLSL32: SymbolKind = SymbolKind(0x1162);
/// The kind of a module-local HLSL data symbol with 32-bit slots (`S_LDATA_HLSL32`).
pub const S_LDATA_HLSL32: SymbolKind = SymbolKind(0x1163);
/// The kind of a global HLSL data symbol with a binding space (`S_GDATA_HLSL32_EX`).
pub const S_GDATA_HLSL32_EX: SymbolKind = SymbolKind(0x1164);
/// The kind of a module-local HLSL data symbol with a binding space (`S_LDATA_HLSL32_EX`).
pub const S_LDATA_HLSL32_EX: SymbolKind = SymbolKind(0x1165);

/// An HLSL or DPC pointer tag def-range ([`S_DEFRANGE_HLSL`] and
/// [`S_DEFRANGE_DPC_PTR_TAG`]).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefRangeHlsl<'a> {
    /// The register type (`CV_HLSLREG_e`).
    pub register_type: u16,
    /// The raw bit fields: the number of register indices in bits 0 and 1,
    /// the "spilled member" bit 2, the memory space in bits 3 to 6 and
    /// padding in bits 7 to 15.
    pub flags: u16,
    /// The offset in the parent variable.
    pub parent_offset: u16,
    /// The size of the part in a register.
    pub size_in_parent: u16,
    /// The code range where the location is valid.
    pub range: AddressRange,
    /// The parts of the range where the location is not valid.
    pub gaps: AddressGaps<'a>,
    /// The offsets of the variable in the register space: one
    /// little-endian `u32` for each register index.
    offsets: &'a [[u8; 4]],
}

impl<'a> DefRangeHlsl<'a> {
    /// Read the fields.
    ///
    /// The gaps and then one offset for each register index fill the rest of
    /// the record (`CV_DEFRANGESYMHLSL_GAPS_COUNT` in `cvinfo.h`).
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let register_type = fields.u16()?;
        let flags = fields.u16()?;
        let parent_offset = fields.u16()?;
        let size_in_parent = fields.u16()?;
        let range = AddressRange::read(fields)?;
        let offset_bytes = usize::from(flags & 0b11) * 4;
        let gap_bytes = fields
            .rest()
            .len()
            .checked_sub(offset_bytes)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("HLSL def-range is too short for its register offsets")
            })?;
        let gaps = AddressGaps::parse(fields.bytes(gap_bytes)?)?;
        let (offsets, _) = fields.bytes(offset_bytes)?.as_chunks();
        Ok(Self {
            register_type,
            flags,
            parent_offset,
            size_in_parent,
            range,
            gaps,
            offsets,
        })
    }

    /// Write the fields.
    ///
    /// The number of offsets must equal the register index count in
    /// [`DefRangeHlsl::flags`].
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        if self.offsets.len() != usize::from(self.flags & 0b11) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("HLSL def-range offset count does not match its flags"));
        }
        out.u16(self.register_type)?;
        out.u16(self.flags)?;
        out.u16(self.parent_offset)?;
        out.u16(self.size_in_parent)?;
        self.range.write(out)?;
        self.gaps.write(out)?;
        for offset in self.offsets {
            out.bytes(offset)?;
        }
        Ok(())
    }

    /// Return the number of register indices (bits 0 and 1 of the flags).
    #[must_use]
    pub const fn register_index_count(&self) -> u16 {
        self.flags & 0b11
    }

    /// Return `true` if the variable is a spilled member (bit 2 of the flags).
    #[must_use]
    pub const fn spilled_member(&self) -> bool {
        self.flags & 0b100 != 0
    }

    /// Return the memory space (bits 3 to 6 of the flags).
    #[must_use]
    pub const fn memory_space(&self) -> u16 {
        (self.flags >> 3) & 0xf
    }

    /// Return the offsets in the register space.
    pub fn offsets(&self) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.offsets.iter().map(|bytes| u32::from_le_bytes(*bytes))
    }
}

/// The slots of an HLSL data symbol.
///
/// [`S_GDATA_HLSL`] and [`S_LDATA_HLSL`] store 16-bit slots.
/// [`S_GDATA_HLSL32`] and [`S_LDATA_HLSL32`] store 32-bit slots and put the
/// register type after them. The value keeps the slots as `u32`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct HlslData<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The register type (`CV_HLSLREG_e`).
    pub register_type: u16,
    /// The base data slot, for example a constant buffer.
    pub data_slot: u32,
    /// The byte offset in the base data.
    pub data_offset: u32,
    /// The first texture slot.
    pub texture_slot: u32,
    /// The first sampler slot.
    pub sampler_slot: u32,
    /// The first unordered access view slot.
    pub uav_slot: u32,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> HlslData<'a> {
    /// Read the fields. Set `wide` for the 32-bit kinds.
    pub(super) fn read(fields: &mut Fields<'a>, wide: bool) -> Result<Self> {
        let type_index = fields.type_index()?;
        let mut register_type = 0;
        if !wide {
            register_type = fields.u16()?;
        }
        let mut slots = [0; 5];
        for slot in &mut slots {
            *slot = if wide {
                fields.u32()?
            } else {
                u32::from(fields.u16()?)
            };
        }
        if wide {
            register_type = fields.u16()?;
        }
        let name = fields.name()?;
        let [data_slot, data_offset, texture_slot, sampler_slot, uav_slot] = slots;
        Ok(Self {
            type_index,
            register_type,
            data_slot,
            data_offset,
            texture_slot,
            sampler_slot,
            uav_slot,
            name,
        })
    }

    /// Write the fields. Set `wide` for the 32-bit kinds.
    pub(super) fn write(&self, out: &mut Output<'_>, wide: bool) -> Result<()> {
        out.type_index(self.type_index)?;
        if !wide {
            out.u16(self.register_type)?;
        }
        for slot in [
            self.data_slot,
            self.data_offset,
            self.texture_slot,
            self.sampler_slot,
            self.uav_slot,
        ] {
            if wide {
                out.u32(slot)?;
            } else {
                out.u16(u16::try_from(slot).map_err(|_| {
                    Error::new(ErrorKind::LimitExceeded, None)
                        .with_context("HLSL data slot does not fit in 16 bits")
                })?)?;
            }
        }
        if wide {
            out.u16(self.register_type)?;
        }
        out.c_string(self.name)
    }
}

/// An HLSL data symbol with a binding space ([`S_GDATA_HLSL32_EX`] and
/// [`S_LDATA_HLSL32_EX`]).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct HlslDataEx<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The register index.
    pub register_id: u32,
    /// The byte offset in the base data.
    pub data_offset: u32,
    /// The binding space.
    pub bind_space: u32,
    /// The lower bound in the binding space.
    pub bind_slot: u32,
    /// The register type (`CV_HLSLREG_e`).
    pub register_type: u16,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> HlslDataEx<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let register_id = fields.u32()?;
        let data_offset = fields.u32()?;
        let bind_space = fields.u32()?;
        let bind_slot = fields.u32()?;
        let register_type = fields.u16()?;
        let name = fields.name()?;
        Ok(Self {
            type_index,
            register_id,
            data_offset,
            bind_space,
            bind_slot,
            register_type,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        out.u32(self.register_id)?;
        out.u32(self.data_offset)?;
        out.u32(self.bind_space)?;
        out.u32(self.bind_slot)?;
        out.u16(self.register_type)?;
        out.c_string(self.name)
    }
}

/// A DPC group-shared local variable ([`S_LOCAL_DPC_GROUPSHARED`]).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DpcGroupSharedLocal<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The flags.
    pub flags: LocalFlags,
    /// The base data slot.
    pub data_slot: u16,
    /// The byte offset in the base data.
    pub data_offset: u16,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> DpcGroupSharedLocal<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let flags = LocalFlags(fields.u16()?);
        let data_slot = fields.u16()?;
        let data_offset = fields.u16()?;
        let name = fields.name()?;
        Ok(Self {
            type_index,
            flags,
            data_slot,
            data_offset,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        out.u16(self.flags.0)?;
        out.u16(self.data_slot)?;
        out.u16(self.data_offset)?;
        out.c_string(self.name)
    }
}

/// A map from DPC pointer tag values to symbol records ([`S_DPC_SYM_TAG_MAP`]).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct DpcTagMap<'a> {
    /// The 8-byte entries.
    entries: &'a [[u8; 8]],
}

/// One entry of a [`DpcTagMap`] (`CV_DPC_SYM_TAG_MAP_ENTRY`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DpcTag {
    /// The pointer tag value of the symbol.
    pub tag: u32,
    /// The offset of the symbol record from the enclosing `S_LPROC32_DPC`
    /// record. `cvinfo.h` gives a signed `CV_off32_t`.
    pub record_offset: i32,
}

impl<'a> DpcTagMap<'a> {
    /// Create a map from 8-byte little-endian entries.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length of `bytes` is not
    /// a multiple of 8.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (entries, rest) = bytes.as_chunks();
        if !rest.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("S_DPC_SYM_TAG_MAP is not a multiple of 8 bytes"));
        }
        Ok(Self { entries })
    }

    /// Read all remaining bytes as entries.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        Self::parse(fields.bytes(fields.rest().len())?)
    }

    /// Write the entries.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        for entry in self.entries {
            out.bytes(entry)?;
        }
        Ok(())
    }

    /// Return the number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` if the map has no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return an iterator over the entries.
    #[must_use]
    pub fn iter(&self) -> DpcTagMapIter<'a> {
        DpcTagMapIter {
            entries: self.entries.iter(),
        }
    }
}

/// An iterator over the entries of a [`DpcTagMap`].
#[derive(Clone, Debug)]
pub struct DpcTagMapIter<'a> {
    /// The 8-byte entries that are not returned yet.
    entries: core::slice::Iter<'a, [u8; 8]>,
}

impl Iterator for DpcTagMapIter<'_> {
    type Item = DpcTag;

    fn next(&mut self) -> Option<DpcTag> {
        self.entries
            .next()
            .map(|&[t0, t1, t2, t3, o0, o1, o2, o3]| DpcTag {
                tag: u32::from_le_bytes([t0, t1, t2, t3]),
                record_offset: i32::from_le_bytes([o0, o1, o2, o3]),
            })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl ExactSizeIterator for DpcTagMapIter<'_> {}

impl core::iter::FusedIterator for DpcTagMapIter<'_> {}

impl<'a> IntoIterator for DpcTagMap<'a> {
    type Item = DpcTag;
    type IntoIter = DpcTagMapIter<'a>;

    fn into_iter(self) -> DpcTagMapIter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &DpcTagMap<'a> {
    type Item = DpcTag;
    type IntoIter = DpcTagMapIter<'a>;

    fn into_iter(self) -> DpcTagMapIter<'a> {
        self.iter()
    }
}
