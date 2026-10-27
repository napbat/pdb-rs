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

use super::super::index::TypeIndex;
use super::super::{Fields, SymbolKind};
use super::{LocalFlags, SectionOffset};
use crate::bytes::into_owned;
use crate::native::output::Output;
use crate::{Bytes, Error, ErrorKind, Result};

/// The kind of a managed type reference (`S_MANTYPREF`).
pub const S_MANTYPREF: SymbolKind = SymbolKind(0x1028);
/// The kind of a variable in several 8-bit register numbers (`S_MANYREG`).
pub const S_MANYREG: SymbolKind = SymbolKind(0x110a);
/// The kind of a variable in several 16-bit register numbers (`S_MANYREG2`).
pub const S_MANYREG2: SymbolKind = SymbolKind(0x1117);
/// The kind of a local variable in an IL slot (`S_LOCALSLOT`).
pub const S_LOCALSLOT: SymbolKind = SymbolKind(0x111a);
/// The kind of a parameter in an IL slot (`S_PARAMSLOT`).
pub const S_PARAMSLOT: SymbolKind = SymbolKind(0x111b);
/// The kind of a module-local managed data symbol (`S_LMANDATA`).
pub const S_LMANDATA: SymbolKind = SymbolKind(0x111c);
/// The kind of a global managed data symbol (`S_GMANDATA`).
pub const S_GMANDATA: SymbolKind = SymbolKind(0x111d);
/// The kind of a managed frame-relative variable (`S_MANFRAMEREL`).
pub const S_MANFRAMEREL: SymbolKind = SymbolKind(0x111e);
/// The kind of a managed register variable (`S_MANREGISTER`).
pub const S_MANREGISTER: SymbolKind = SymbolKind(0x111f);
/// The kind of a managed variable in several 8-bit register numbers (`S_MANMANYREG`).
pub const S_MANMANYREG: SymbolKind = SymbolKind(0x1121);
/// The kind of a managed register-relative variable (`S_MANREGREL`).
pub const S_MANREGREL: SymbolKind = SymbolKind(0x1122);
/// The kind of a managed variable in several 16-bit register numbers (`S_MANMANYREG2`).
pub const S_MANMANYREG2: SymbolKind = SymbolKind(0x1123);
/// The kind of a frame-relative variable with attributes (`S_ATTR_FRAMEREL`).
pub const S_ATTR_FRAMEREL: SymbolKind = SymbolKind(0x112e);
/// The kind of a register variable with attributes (`S_ATTR_REGISTER`).
pub const S_ATTR_REGISTER: SymbolKind = SymbolKind(0x112f);
/// The kind of a register-relative variable with attributes (`S_ATTR_REGREL`).
pub const S_ATTR_REGREL: SymbolKind = SymbolKind(0x1130);
/// The kind of a variable in several registers with attributes (`S_ATTR_MANYREG`).
pub const S_ATTR_MANYREG: SymbolKind = SymbolKind(0x1131);

/// The attributes of a variable (`CV_lvar_attr`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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
    fn write(self, out: &mut Output<'_>) -> Result<()> {
        self.address.write(out)?;
        out.u16(self.flags.0)
    }
}

/// A counted list of register numbers.
///
/// The 8-bit kinds store a `u8` count and `u8` registers. The 16-bit kinds
/// store a `u16` count and `u16` registers. The most significant register
/// is first.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct RegisterList<'a> {
    /// The register bytes.
    bytes: Bytes<'a>,
    /// `true` for 16-bit register numbers.
    wide: bool,
}

into_owned!(struct RegisterList { bytes; wide });

impl<'a> RegisterList<'a> {
    /// Create a list from register bytes: one byte for each register, or two
    /// little-endian bytes if `wide` is set.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `wide` is set and the length is
    /// odd. Returns [`ErrorKind::LimitExceeded`] if the count does not fit the
    /// count field.
    pub fn parse(bytes: impl Into<Bytes<'a>>, wide: bool) -> Result<Self> {
        let bytes = bytes.into();
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
            bytes: Bytes::new(fields.bytes(length)?),
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
    fn write(&self, out: &mut Output<'_>) -> Result<()> {
        let count = self.count()?;
        if self.wide {
            out.u16(count)?;
        } else {
            // The count fits in a `u8` for an 8-bit list.
            out.u8(count.to_le_bytes()[0])?;
        }
        out.bytes(&self.bytes)
    }

    /// Return `true` for 16-bit register numbers.
    #[must_use]
    pub const fn is_wide(&self) -> bool {
        self.wide
    }

    /// Return the number of registers.
    #[must_use]
    pub fn len(&self) -> usize {
        if self.wide {
            self.bytes.len() / 2
        } else {
            self.bytes.len()
        }
    }

    /// Return `true` if the list has no registers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the register numbers.
    #[must_use]
    pub fn iter(&self) -> RegisterListIter<'_> {
        let step = if self.wide { 2 } else { 1 };
        RegisterListIter {
            chunks: self.bytes.chunks_exact(step),
        }
    }
}

/// An iterator over the register numbers of a [`RegisterList`].
#[derive(Clone, Debug)]
pub struct RegisterListIter<'a> {
    /// The 1-byte or 2-byte register numbers that are not returned yet.
    chunks: core::slice::ChunksExact<'a, u8>,
}

impl Iterator for RegisterListIter<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<u16> {
        self.chunks.next().map(|chunk| match *chunk {
            [low, high] => u16::from_le_bytes([low, high]),
            [byte] => u16::from(byte),
            _ => 0,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl ExactSizeIterator for RegisterListIter<'_> {}

impl core::iter::FusedIterator for RegisterListIter<'_> {}

impl<'a> IntoIterator for &'a RegisterList<'_> {
    type Item = u16;
    type IntoIter = RegisterListIter<'a>;

    fn into_iter(self) -> RegisterListIter<'a> {
        self.iter()
    }
}

/// A variable in several registers.
///
/// The kinds are [`S_MANYREG`], [`S_MANYREG2`], [`S_MANMANYREG`],
/// [`S_MANMANYREG2`] and [`S_ATTR_MANYREG`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ManyRegisters<'a> {
    /// The type index, or a metadata token for the managed kinds.
    pub type_index: TypeIndex,
    /// The attributes of the kinds with attributes, or `None`.
    pub attributes: Option<VariableAttributes>,
    /// The registers.
    pub registers: RegisterList<'a>,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct ManyRegisters { registers, name; type_index, attributes });

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
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            type_index,
            attributes,
            registers,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        if let Some(attributes) = self.attributes {
            attributes.write(out)?;
        }
        self.registers.write(out)?;
        out.c_string(&self.name)
    }
}

/// A variable in an IL slot ([`S_LOCALSLOT`] and [`S_PARAMSLOT`]).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Slot<'a> {
    /// The slot index.
    pub slot: u32,
    /// A type index or a metadata token.
    pub type_or_token: u32,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct Slot { name; slot, type_or_token });

impl<'a> Slot<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let slot = fields.u32()?;
        let type_or_token = fields.u32()?;
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            slot,
            type_or_token,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.slot)?;
        out.u32(self.type_or_token)?;
        out.c_string(&self.name)
    }
}

/// A frame-relative variable with attributes ([`S_MANFRAMEREL`] and
/// [`S_ATTR_FRAMEREL`]).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AttributedFrameRelative<'a> {
    /// The offset from the frame. `cvinfo.h` gives a signed `CV_off32_t`.
    pub offset: i32,
    /// The type index, or a metadata token for [`S_MANFRAMEREL`].
    pub type_index: TypeIndex,
    /// The attributes.
    pub attributes: VariableAttributes,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct AttributedFrameRelative { name; offset, type_index, attributes });

impl<'a> AttributedFrameRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.i32()?;
        let type_index = fields.type_index()?;
        let attributes = VariableAttributes::read(fields)?;
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            offset,
            type_index,
            attributes,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.i32(self.offset)?;
        out.type_index(self.type_index)?;
        self.attributes.write(out)?;
        out.c_string(&self.name)
    }
}

/// A register variable with attributes ([`S_MANREGISTER`] and
/// [`S_ATTR_REGISTER`]).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AttributedRegister<'a> {
    /// The type index, or a metadata token for [`S_MANREGISTER`].
    pub type_index: TypeIndex,
    /// The attributes.
    pub attributes: VariableAttributes,
    /// The register, as a `CV_HREG_e` number for the CPU of the module.
    pub register: u16,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct AttributedRegister { name; type_index, attributes, register });

impl<'a> AttributedRegister<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let attributes = VariableAttributes::read(fields)?;
        let register = fields.u16()?;
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            type_index,
            attributes,
            register,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        self.attributes.write(out)?;
        out.u16(self.register)?;
        out.c_string(&self.name)
    }
}

/// A register-relative variable with attributes ([`S_MANREGREL`] and
/// [`S_ATTR_REGREL`]).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
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
    pub name: Bytes<'a>,
}

into_owned!(struct AttributedRegisterRelative { name; offset, type_index, register, attributes });

impl<'a> AttributedRegisterRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.u32()?;
        let type_index = fields.type_index()?;
        let register = fields.u16()?;
        let attributes = VariableAttributes::read(fields)?;
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            offset,
            type_index,
            register,
            attributes,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.offset)?;
        out.type_index(self.type_index)?;
        out.u16(self.register)?;
        self.attributes.write(out)?;
        out.c_string(&self.name)
    }
}
