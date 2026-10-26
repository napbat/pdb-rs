//! Variables in many registers, slots and variables with attributes.
//!
//! The layouts follow `MANYREGSYM`, `MANYREGSYM2`, `SLOTSYM32`,
//! `FRAMERELSYM`, `ATTRREGSYM`, `ATTRMANYREGSYM`, `ATTRMANYREGSYM2`,
//! `ATTRREGREL`, `DATASYM32` and `MANTYPREF` in `cvinfo.h`. The header packs
//! the structures to one byte, so no padding separates the fields. In the
//! 32-bit kinds, names are NUL-terminated.
//!
//! The managed kinds (`S_MAN*`) store a metadata token where the native
//! kinds store a type index. [`Symbol::type_index`](super::Symbol::type_index)
//! gives the value only for the native kinds.

use super::super::Fields;
use super::super::index::TypeIndex;
use super::super::writer::FieldWriter;
use super::{LocalFlags, SectionOffset};
use crate::{Error, ErrorKind, Result};

/// The kind of a managed type reference (`S_MANTYPREF`).
pub const S_MANTYPREF: u16 = 0x1028;
/// The kind of a variable in several 8-bit register numbers (`S_MANYREG`).
pub const S_MANYREG: u16 = 0x110a;
/// The kind of a variable in several 16-bit register numbers (`S_MANYREG2`).
pub const S_MANYREG2: u16 = 0x1117;
/// The kind of a local variable in an IL slot (`S_LOCALSLOT`).
pub const S_LOCALSLOT: u16 = 0x111a;
/// The kind of a parameter in an IL slot (`S_PARAMSLOT`).
pub const S_PARAMSLOT: u16 = 0x111b;
/// The kind of a module-local managed data symbol (`S_LMANDATA`).
pub const S_LMANDATA: u16 = 0x111c;
/// The kind of a global managed data symbol (`S_GMANDATA`).
pub const S_GMANDATA: u16 = 0x111d;
/// The kind of a managed frame-relative variable (`S_MANFRAMEREL`).
pub const S_MANFRAMEREL: u16 = 0x111e;
/// The kind of a managed register variable (`S_MANREGISTER`).
pub const S_MANREGISTER: u16 = 0x111f;
/// The kind of a managed variable in several 8-bit register numbers (`S_MANMANYREG`).
pub const S_MANMANYREG: u16 = 0x1121;
/// The kind of a managed register-relative variable (`S_MANREGREL`).
pub const S_MANREGREL: u16 = 0x1122;
/// The kind of a managed variable in several 16-bit register numbers (`S_MANMANYREG2`).
pub const S_MANMANYREG2: u16 = 0x1123;
/// The kind of a frame-relative variable with attributes (`S_ATTR_FRAMEREL`).
pub const S_ATTR_FRAMEREL: u16 = 0x112e;
/// The kind of a register variable with attributes (`S_ATTR_REGISTER`).
pub const S_ATTR_REGISTER: u16 = 0x112f;
/// The kind of a register-relative variable with attributes (`S_ATTR_REGREL`).
pub const S_ATTR_REGREL: u16 = 0x1130;
/// The kind of a variable in several registers with attributes (`S_ATTR_MANYREG`).
pub const S_ATTR_MANYREG: u16 = 0x1131;

/// The attributes of a variable (`CV_lvar_attr`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct VariableAttributes {
    /// The first code address where the variable is live.
    pub address: SectionOffset,
    /// The flags.
    pub flags: LocalFlags,
}

impl VariableAttributes {
    /// Read the address and the flags.
    fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let address = SectionOffset::read(fields)?;
        let flags = LocalFlags(fields.u16()?);
        Ok(Self { address, flags })
    }

    /// Write the address and the flags.
    fn write(self, out: &mut FieldWriter<'_>) -> Result<()> {
        self.address.write(out)?;
        out.u16(self.flags.0)
    }
}

/// A counted list of register numbers.
///
/// The 8-bit kinds store a `u8` count and `u8` registers. The 16-bit kinds
/// store a `u16` count and `u16` registers. The most significant register
/// is first.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RegisterList<'a> {
    /// The register bytes.
    bytes: &'a [u8],
    /// `true` for 16-bit register numbers.
    wide: bool,
}

impl<'a> RegisterList<'a> {
    /// Create a list from register bytes: one byte for each register, or two
    /// little-endian bytes if `wide` is set.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `wide` is set and the length is
    /// odd. Returns [`ErrorKind::LimitExceeded`] if the count does not fit the
    /// count field.
    pub fn new(bytes: &'a [u8], wide: bool) -> Result<Self> {
        if wide && !bytes.len().is_multiple_of(2) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("16-bit register list has an odd length"));
        }
        let list = Self { bytes, wide };
        list.count()?;
        Ok(list)
    }

    /// Read the count and the registers.
    fn read(fields: &mut Fields<'a>, wide: bool) -> Result<Self> {
        let length = if wide {
            usize::from(fields.u16()?) * 2
        } else {
            usize::from(fields.u8()?)
        };
        Ok(Self {
            bytes: fields.bytes(length)?,
            wide,
        })
    }

    /// Return the number of registers as the count field stores it.
    fn count(&self) -> Result<u16> {
        let count = if self.wide {
            u16::try_from(self.bytes.len() / 2).ok()
        } else {
            u8::try_from(self.bytes.len()).ok().map(u16::from)
        };
        count.ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("register list is too long for its count field")
        })
    }

    /// Write the count and the registers.
    fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        let count = self.count()?;
        if self.wide {
            out.u16(count)?;
        } else {
            // The count fits in a `u8` for an 8-bit list.
            out.u8(count.to_le_bytes()[0])?;
        }
        out.bytes(self.bytes)
    }

    /// Return `true` for 16-bit register numbers.
    #[must_use]
    pub const fn is_wide(&self) -> bool {
        self.wide
    }

    /// Return the number of registers.
    #[must_use]
    pub const fn len(&self) -> usize {
        if self.wide {
            self.bytes.len() / 2
        } else {
            self.bytes.len()
        }
    }

    /// Return `true` if the list has no registers.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the register numbers.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = u16> + 'a {
        let step = if self.wide { 2 } else { 1 };
        self.bytes.chunks_exact(step).map(|chunk| match *chunk {
            [low, high] => u16::from_le_bytes([low, high]),
            [byte] => u16::from(byte),
            _ => 0,
        })
    }
}

/// A variable in several registers.
///
/// The kinds are [`S_MANYREG`], [`S_MANYREG2`], [`S_MANMANYREG`],
/// [`S_MANMANYREG2`] and [`S_ATTR_MANYREG`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ManyRegisters<'a> {
    /// The type index, or a metadata token for the managed kinds.
    pub type_index: TypeIndex,
    /// The attributes of the kinds with attributes, or `None`.
    pub attributes: Option<VariableAttributes>,
    /// The registers.
    pub registers: RegisterList<'a>,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> ManyRegisters<'a> {
    /// Read the fields. Set `with_attributes` for the kinds with attributes
    /// and `wide` for 16-bit register numbers.
    pub(super) fn read(fields: &mut Fields<'a>, with_attributes: bool, wide: bool) -> Result<Self> {
        let type_index = fields.type_index()?;
        let attributes = if with_attributes {
            Some(VariableAttributes::read(fields)?)
        } else {
            None
        };
        let registers = RegisterList::read(fields, wide)?;
        let name = fields.name()?;
        Ok(Self {
            type_index,
            attributes,
            registers,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        if let Some(attributes) = self.attributes {
            attributes.write(out)?;
        }
        self.registers.write(out)?;
        out.name(self.name)
    }
}

/// A variable in an IL slot ([`S_LOCALSLOT`] and [`S_PARAMSLOT`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Slot<'a> {
    /// The slot index.
    pub slot: u32,
    /// A type index or a metadata token.
    pub type_or_token: u32,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> Slot<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let slot = fields.u32()?;
        let type_or_token = fields.u32()?;
        let name = fields.name()?;
        Ok(Self {
            slot,
            type_or_token,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.slot)?;
        out.u32(self.type_or_token)?;
        out.name(self.name)
    }
}

/// A frame-relative variable with attributes ([`S_MANFRAMEREL`] and
/// [`S_ATTR_FRAMEREL`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AttributedFrameRelative<'a> {
    /// The offset from the frame. `cvinfo.h` gives a signed `CV_off32_t`.
    pub offset: i32,
    /// The type index, or a metadata token for [`S_MANFRAMEREL`].
    pub type_index: TypeIndex,
    /// The attributes.
    pub attributes: VariableAttributes,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> AttributedFrameRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.i32()?;
        let type_index = fields.type_index()?;
        let attributes = VariableAttributes::read(fields)?;
        let name = fields.name()?;
        Ok(Self {
            offset,
            type_index,
            attributes,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.i32(self.offset)?;
        out.type_index(self.type_index)?;
        self.attributes.write(out)?;
        out.name(self.name)
    }
}

/// A register variable with attributes ([`S_MANREGISTER`] and
/// [`S_ATTR_REGISTER`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AttributedRegister<'a> {
    /// The type index, or a metadata token for [`S_MANREGISTER`].
    pub type_index: TypeIndex,
    /// The attributes.
    pub attributes: VariableAttributes,
    /// The register, as a `CV_HREG_e` number for the CPU of the module.
    pub register: u16,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> AttributedRegister<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let attributes = VariableAttributes::read(fields)?;
        let register = fields.u16()?;
        let name = fields.name()?;
        Ok(Self {
            type_index,
            attributes,
            register,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        self.attributes.write(out)?;
        out.u16(self.register)?;
        out.name(self.name)
    }
}

/// A register-relative variable with attributes ([`S_MANREGREL`] and
/// [`S_ATTR_REGREL`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AttributedRegisterRelative<'a> {
    /// The offset from the register.
    pub offset: u32,
    /// The type index, or a metadata token for [`S_MANREGREL`].
    pub type_index: TypeIndex,
    /// The base register, as a `CV_HREG_e` number for the CPU of the module.
    pub register: u16,
    /// The attributes.
    pub attributes: VariableAttributes,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> AttributedRegisterRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.u32()?;
        let type_index = fields.type_index()?;
        let register = fields.u16()?;
        let attributes = VariableAttributes::read(fields)?;
        let name = fields.name()?;
        Ok(Self {
            offset,
            type_index,
            register,
            attributes,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.offset)?;
        out.type_index(self.type_index)?;
        out.u16(self.register)?;
        self.attributes.write(out)?;
        out.name(self.name)
    }
}
