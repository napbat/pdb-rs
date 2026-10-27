//! Variable, def-range, data, constant and user-defined type records.
//!
//! The layouts follow `LocalSym`, the `DefRange*Sym` classes,
//! `RegRelativeSym`, `DataSym`, `ThreadLocalDataSym`, `ConstantSym` and
//! `UDTSym` in LLVM `SymbolRecord.h` and `SymbolRecordMapping.cpp`.
//!
//! Register fields stay raw `u16` values. A value is a `CV_HREG_e` register
//! number for the CPU of the module. Read the CPU from the `machine` field of
//! the compile record. LLVM `CodeViewRegisters.def` lists the numbers, for
//! example 328 for `RAX` and 335 for `RSP` on x64.

use core::iter::FusedIterator;

use super::super::index::TypeIndex;
use super::super::numeric::Numeric;
use super::super::{Fields, SymbolKind};
use super::SectionOffset;
use crate::bytes::into_owned;
use crate::native::output::Output;
use crate::{Bytes, Error, ErrorKind, Result};

flags! {
    /// The flags of a local variable (`CV_LVARFLAGS`).
    LocalFlags(u16) {
        /// The variable is a parameter.
        PARAMETER = 1 << 0;
        /// The address of the variable is taken.
        ADDRESS_TAKEN = 1 << 1;
        /// The compiler generated the variable.
        COMPILER_GENERATED = 1 << 2;
        /// The variable is an aggregate of other variables.
        AGGREGATE = 1 << 3;
        /// The variable is a part of an aggregate.
        AGGREGATED = 1 << 4;
        /// The variable has aliases.
        ALIASED = 1 << 5;
        /// The variable is an alias of another variable.
        ALIAS = 1 << 6;
        /// The variable holds the return value.
        RETURN_VALUE = 1 << 7;
        /// The variable has no storage in the optimized code.
        OPTIMIZED_OUT = 1 << 8;
        /// The variable is an enregistered global.
        ENREGISTERED_GLOBAL = 1 << 9;
        /// The variable is an enregistered static.
        ENREGISTERED_STATIC = 1 << 10;
    }
}

/// A local variable record (`S_LOCAL`).
///
/// The def-range records that follow give the location of the variable.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Local<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The flags.
    pub flags: LocalFlags,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct Local { name; type_index, flags });

impl<'a> Local<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let flags = LocalFlags(fields.u16()?);
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            type_index,
            flags,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        out.u16(self.flags.0)?;
        out.c_string(&self.name)
    }
}

/// The code range where a def-range location is valid
/// (`LocalVariableAddrRange`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AddressRange {
    /// The first address of the range.
    pub start: SectionOffset,
    /// The length of the range in bytes.
    pub length: u16,
}

impl AddressRange {
    /// Read a `u32` offset, a `u16` section and a `u16` length.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let start = SectionOffset::read(fields)?;
        let length = fields.u16()?;
        Ok(Self { start, length })
    }

    /// Write the start and the length.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.start.write(out)?;
        out.u16(self.length)
    }
}

/// A part of an [`AddressRange`] where the location is not valid
/// (`LocalVariableAddrGap`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AddressGap {
    /// The start of the gap, relative to the start of the range.
    pub start: u16,
    /// The length of the gap in bytes.
    pub length: u16,
}

/// The gaps at the end of a def-range record.
///
/// The gaps fill the rest of the record. Each gap has 4 bytes, so the
/// record needs no padding.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AddressGaps<'a> {
    /// The 4-byte gaps.
    entries: Bytes<'a>,
}

into_owned!(struct AddressGaps { entries; });

impl<'a> AddressGaps<'a> {
    /// Read all remaining bytes as gaps.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let length = fields.rest().len();
        if !length.is_multiple_of(4) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("def-range gap array is not a multiple of 4 bytes"));
        }
        let entries = Bytes::new(fields.bytes(length)?);
        Ok(Self { entries })
    }

    /// Return the number of gaps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len() / 4
    }

    /// Return `true` if there are no gaps.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return an iterator over the gaps.
    #[must_use]
    pub fn iter(&self) -> AddressGapsIter<'_> {
        AddressGapsIter {
            entries: self.entries.as_chunks().0.iter(),
        }
    }

    /// Create gaps from their little-endian 4-byte form.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length of `bytes` is not
    /// a multiple of 4.
    pub fn parse(bytes: impl Into<Bytes<'a>>) -> Result<Self> {
        let entries = bytes.into();
        if !entries.len().is_multiple_of(4) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("def-range gap array is not a multiple of 4 bytes"));
        }
        Ok(Self { entries })
    }

    /// Write all gaps.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.bytes(&self.entries)
    }
}

impl<'a> IntoIterator for &'a AddressGaps<'_> {
    type Item = AddressGap;
    type IntoIter = AddressGapsIter<'a>;

    fn into_iter(self) -> AddressGapsIter<'a> {
        self.iter()
    }
}

/// An iterator over [`AddressGaps`].
#[derive(Clone, Debug)]
pub struct AddressGapsIter<'a> {
    /// The 4-byte gaps that are not returned yet.
    entries: core::slice::Iter<'a, [u8; 4]>,
}

impl Iterator for AddressGapsIter<'_> {
    type Item = AddressGap;

    fn next(&mut self) -> Option<AddressGap> {
        self.entries.next().map(|&[s0, s1, l0, l1]| AddressGap {
            start: u16::from_le_bytes([s0, s1]),
            length: u16::from_le_bytes([l0, l1]),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl ExactSizeIterator for AddressGapsIter<'_> {}

impl FusedIterator for AddressGapsIter<'_> {}

/// A frame-pointer-relative def-range (`S_DEFRANGE_FRAMEPOINTER_REL`).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefRangeFramePointerRelative<'a> {
    /// The offset of the variable from the frame pointer.
    pub offset: i32,
    /// The code range where the location is valid.
    pub range: AddressRange,
    /// The parts of the range where the location is not valid.
    pub gaps: AddressGaps<'a>,
}

into_owned!(struct DefRangeFramePointerRelative { gaps; offset, range });

impl<'a> DefRangeFramePointerRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.i32()?;
        let range = AddressRange::read(fields)?;
        let gaps = AddressGaps::read(fields)?;
        Ok(Self {
            offset,
            range,
            gaps,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.i32(self.offset)?;
        self.range.write(out)?;
        self.gaps.write(out)
    }
}

/// A register def-range (`S_DEFRANGE_REGISTER`).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefRangeRegister<'a> {
    /// The register that holds the variable. See the module documentation.
    pub register: u16,
    /// The range attributes (`CV_RANGEATTR`). Bit 0 is the "may have no
    /// name" bit. LLVM keeps all 16 bits as `MayHaveNoName`.
    pub attributes: u16,
    /// The code range where the location is valid.
    pub range: AddressRange,
    /// The parts of the range where the location is not valid.
    pub gaps: AddressGaps<'a>,
}

into_owned!(struct DefRangeRegister { gaps; register, attributes, range });

impl<'a> DefRangeRegister<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let register = fields.u16()?;
        let attributes = fields.u16()?;
        let range = AddressRange::read(fields)?;
        let gaps = AddressGaps::read(fields)?;
        Ok(Self {
            register,
            attributes,
            range,
            gaps,
        })
    }

    /// Return `true` if the "may have no name" bit is set.
    #[must_use]
    pub const fn may_have_no_name(&self) -> bool {
        self.attributes & 1 != 0
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.register)?;
        out.u16(self.attributes)?;
        self.range.write(out)?;
        self.gaps.write(out)
    }
}

/// A def-range for a part of a variable in a register
/// (`S_DEFRANGE_SUBFIELD_REGISTER`).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefRangeSubfieldRegister<'a> {
    /// The register that holds the part. See the module documentation.
    pub register: u16,
    /// The range attributes, with the same bits as
    /// [`DefRangeRegister::attributes`].
    pub attributes: u16,
    /// The raw offset field. `cvinfo.h` gives the offset of the part in the
    /// parent variable in bits 0 to 11 and padding in bits 12 to 31. LLVM
    /// keeps all 32 bits as `OffsetInParent`.
    pub parent_offset: u32,
    /// The code range where the location is valid.
    pub range: AddressRange,
    /// The parts of the range where the location is not valid.
    pub gaps: AddressGaps<'a>,
}

into_owned!(struct DefRangeSubfieldRegister { gaps; register, attributes, parent_offset, range });

impl<'a> DefRangeSubfieldRegister<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let register = fields.u16()?;
        let attributes = fields.u16()?;
        let parent_offset = fields.u32()?;
        let range = AddressRange::read(fields)?;
        let gaps = AddressGaps::read(fields)?;
        Ok(Self {
            register,
            attributes,
            parent_offset,
            range,
            gaps,
        })
    }

    /// Return `true` if the "may have no name" bit is set.
    #[must_use]
    pub const fn may_have_no_name(&self) -> bool {
        self.attributes & 1 != 0
    }

    /// Return the offset of the part in the parent variable (bits 0 to 11).
    #[must_use]
    pub const fn offset_in_parent(&self) -> u16 {
        let [low, high, _, _] = (self.parent_offset & 0xfff).to_le_bytes();
        u16::from_le_bytes([low, high])
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.register)?;
        out.u16(self.attributes)?;
        out.u32(self.parent_offset)?;
        self.range.write(out)?;
        self.gaps.write(out)
    }
}

/// Return the "spilled member" bit of def-range register flags.
const fn spilled_member(flags: u16) -> bool {
    flags & 1 != 0
}

/// Return the offset in the parent from def-range register flags.
const fn offset_in_parent(flags: u16) -> u16 {
    flags >> 4
}

/// A register-relative def-range (`S_DEFRANGE_REGISTER_REL`).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefRangeRegisterRelative<'a> {
    /// The base register. See the module documentation.
    pub register: u16,
    /// The raw flags. Bit 0 is the "spilled member" bit, bits 1 to 3 are
    /// padding and bits 4 to 15 hold the offset in the parent variable.
    pub flags: u16,
    /// The offset of the variable from the base register.
    pub base_offset: i32,
    /// The code range where the location is valid.
    pub range: AddressRange,
    /// The parts of the range where the location is not valid.
    pub gaps: AddressGaps<'a>,
}

into_owned!(struct DefRangeRegisterRelative { gaps; register, flags, base_offset, range });

impl<'a> DefRangeRegisterRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let register = fields.u16()?;
        let flags = fields.u16()?;
        let base_offset = fields.i32()?;
        let range = AddressRange::read(fields)?;
        let gaps = AddressGaps::read(fields)?;
        Ok(Self {
            register,
            flags,
            base_offset,
            range,
            gaps,
        })
    }

    /// Return `true` if the variable is a spilled member of a user-defined type.
    #[must_use]
    pub const fn spilled_member(&self) -> bool {
        spilled_member(self.flags)
    }

    /// Return the offset of this part in the parent variable (bits 4 to 15).
    #[must_use]
    pub const fn offset_in_parent(&self) -> u16 {
        offset_in_parent(self.flags)
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.register)?;
        out.u16(self.flags)?;
        out.i32(self.base_offset)?;
        self.range.write(out)?;
        self.gaps.write(out)
    }
}

/// An indirect register-relative def-range (`S_DEFRANGE_REGISTER_REL_INDIR`).
///
/// The variable is at `*(register + base_offset) + offset_in_udt`.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefRangeRegisterRelativeIndirect<'a> {
    /// The base register. See the module documentation.
    pub register: u16,
    /// The raw flags, with the same bits as [`DefRangeRegisterRelative::flags`].
    pub flags: u16,
    /// The offset of the pointer from the base register.
    pub base_offset: i32,
    /// The offset to add after the pointer is dereferenced. LLVM
    /// `DefRangeRegisterRelIndirHeader` stores it as a signed `little32_t`.
    pub offset_in_udt: i32,
    /// The code range where the location is valid.
    pub range: AddressRange,
    /// The parts of the range where the location is not valid.
    pub gaps: AddressGaps<'a>,
}

into_owned!(struct DefRangeRegisterRelativeIndirect {
    gaps;
    register, flags, base_offset, offset_in_udt, range
});

impl<'a> DefRangeRegisterRelativeIndirect<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let register = fields.u16()?;
        let flags = fields.u16()?;
        let base_offset = fields.i32()?;
        let offset_in_udt = fields.i32()?;
        let range = AddressRange::read(fields)?;
        let gaps = AddressGaps::read(fields)?;
        Ok(Self {
            register,
            flags,
            base_offset,
            offset_in_udt,
            range,
            gaps,
        })
    }

    /// Return `true` if the variable is a spilled member of a user-defined type.
    #[must_use]
    pub const fn spilled_member(&self) -> bool {
        spilled_member(self.flags)
    }

    /// Return the offset of this part in the parent variable (bits 4 to 15).
    #[must_use]
    pub const fn offset_in_parent(&self) -> u16 {
        offset_in_parent(self.flags)
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.register)?;
        out.u16(self.flags)?;
        out.i32(self.base_offset)?;
        out.i32(self.offset_in_udt)?;
        self.range.write(out)?;
        self.gaps.write(out)
    }
}

/// A register-relative variable (`S_REGREL32` and `S_REGREL32_ENCTMP`).
///
/// MSVC writes `S_REGREL32_ENCTMP` for compiler temporaries, for example
/// `__$ReturnUdt`, in code compiled with `/ZI` (Edit and Continue). No
/// reference gives its layout. The MSVC incremental fixture confirms
/// the `S_REGREL32` layout.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct RegisterRelative<'a> {
    /// The offset from the register. LLVM `RegRelativeSym` reads it as
    /// `uint32_t`.
    pub offset: u32,
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The base register. See the module documentation.
    pub register: u16,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct RegisterRelative { name; offset, type_index, register });

impl<'a> RegisterRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.u32()?;
        let type_index = fields.type_index()?;
        let register = fields.u16()?;
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            offset,
            type_index,
            register,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.offset)?;
        out.type_index(self.type_index)?;
        out.u16(self.register)?;
        out.c_string(&self.name)
    }
}

/// A data record (`S_GDATA32`, `S_LDATA32`, `S_GTHREAD32` and `S_LTHREAD32`).
///
/// For a thread-local record, the address is in the thread storage section,
/// for example `.tls`.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Data<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The address.
    pub address: SectionOffset,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct Data { name; type_index, address });

impl<'a> Data<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let address = SectionOffset::read(fields)?;
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            type_index,
            address,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        self.address.write(out)?;
        out.c_string(&self.name)
    }
}

/// A constant record (`S_CONSTANT`).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Constant<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The value: a numeric leaf with its exact encoding.
    pub value: Numeric<'a>,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct Constant { value, name; type_index });

impl<'a> Constant<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let value = fields.numeric()?;
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            type_index,
            value,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        self.value.encode(out.out())?;
        out.c_string(&self.name)
    }
}

/// The kind of a COBOL user-defined type name (`S_COBOLUDT`). It has the
/// layout of `S_UDT` (`UDTSYM` in `cvinfo.h`).
pub const S_COBOLUDT: SymbolKind = SymbolKind(0x1109);

/// A user-defined type name record (`S_UDT` and [`S_COBOLUDT`]).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct UserDefinedType<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct UserDefinedType { name; type_index });

impl<'a> UserDefinedType<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let name = Bytes::new(fields.name()?);
        Ok(Self { type_index, name })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        out.c_string(&self.name)
    }
}
