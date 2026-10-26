//! Records about code regions: `with` blocks, separated code, static
//! locals, virtual function tables, annotations, hybrid ranges,
//! associations and hot-patch functions.
//!
//! The layouts of `S_WITH32`, `S_SEPCODE`, `S_VFTABLE32` and
//! `S_ANNOTATION` follow `WITHSYM32`, `SEPCODESYM`, `VPATHSYM32` and
//! `ANNOTATIONSYM` in `cvinfo.h`. LLVM gives `HotPatchFuncSym` and
//! `AssociationSym`. The pinned references do not give `S_STATICLOCAL` and
//! `S_HYBRIDRANGE`: the debug type records of `cvinfo.h` in the DIA SDK of
//! Visual Studio 18 (`lib/amd64/diaguids.lib`) give `STATICLOCAL` and
//! `HYBRIDRANGESYM`, and they confirm `HOTPATCHFUNC` and `ASSOCIATION`.

use super::super::Fields;
use super::super::index::{IdIndex, TypeIndex};
use super::SectionOffset;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The kind of a virtual function table path (`S_VFTABLE32`).
pub const S_VFTABLE32: u16 = 0x100c;
/// The kind of a list of annotation strings (`S_ANNOTATION`).
pub const S_ANNOTATION: u16 = 0x1019;
/// The kind of a `with` block start (`S_WITH32`).
pub const S_WITH32: u16 = 0x1104;
/// The kind of a separated code block start (`S_SEPCODE`).
pub const S_SEPCODE: u16 = 0x1132;
/// The kind of a hot-patch function record (`S_HOTPATCHFUNC`).
pub const S_HOTPATCHFUNC: u16 = 0x1169;
/// The kind of a static local variable location (`S_STATICLOCAL`).
pub const S_STATICLOCAL: u16 = 0x1176;
/// The kind of an association between two symbols (`S_ASSOCIATION`).
pub const S_ASSOCIATION: u16 = 0x117c;
/// The kind of a code range of one architecture in a hybrid image (`S_HYBRIDRANGE`).
pub const S_HYBRIDRANGE: u16 = 0x117d;

/// A `with` block start record ([`S_WITH32`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct With<'a> {
    /// The module stream offset of the enclosing scope record.
    pub parent: u32,
    /// The module stream offset of the `S_END` record of this block.
    pub end: u32,
    /// The length of the block code in bytes.
    pub code_length: u32,
    /// The start address.
    pub address: SectionOffset,
    /// The expression string.
    pub expression: &'a [u8],
}

impl<'a> With<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let parent = fields.u32()?;
        let end = fields.u32()?;
        let code_length = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let expression = fields.name()?;
        Ok(Self {
            parent,
            end,
            code_length,
            address,
            expression,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.parent)?;
        out.u32(self.end)?;
        out.u32(self.code_length)?;
        self.address.write(out)?;
        out.c_string(self.expression)
    }
}

flags! {
    /// The flags of separated code (`CV_SEPCODEFLAGS`).
    SeparatedCodeFlags(u32) {
        /// The record is also a lexical scope.
        LEXICAL_SCOPE = 1 << 0;
        /// The code returns to its parent.
        RETURNS_TO_PARENT = 1 << 1;
    }
}

/// A separated code block start record ([`S_SEPCODE`]).
///
/// The compiler moves a part of a procedure to a different place, for
/// example cold code after profile-guided optimization.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SeparatedCode {
    /// The module stream offset of the enclosing scope record.
    pub parent: u32,
    /// The module stream offset of the `S_END` record of this block.
    pub end: u32,
    /// The length of the separated code in bytes.
    pub code_length: u32,
    /// The flags.
    pub flags: SeparatedCodeFlags,
    /// The address of the separated code.
    pub address: SectionOffset,
    /// The address of the enclosing procedure, block or separated code.
    pub parent_address: SectionOffset,
}

impl SeparatedCode {
    /// Read the fields. Both offsets come before both section numbers.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let parent = fields.u32()?;
        let end = fields.u32()?;
        let code_length = fields.u32()?;
        let flags = SeparatedCodeFlags(fields.u32()?);
        let (address, parent_address) = read_address_pair(fields)?;
        Ok(Self {
            parent,
            end,
            code_length,
            flags,
            address,
            parent_address,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.parent)?;
        out.u32(self.end)?;
        out.u32(self.code_length)?;
        out.u32(self.flags.0)?;
        write_address_pair(out, self.address, self.parent_address)
    }
}

/// Read two offsets and then two section numbers.
fn read_address_pair(fields: &mut Fields<'_>) -> Result<(SectionOffset, SectionOffset)> {
    let offset = fields.u32()?;
    let parent_offset = fields.u32()?;
    let section = fields.u16()?;
    let parent_section = fields.u16()?;
    Ok((
        SectionOffset { section, offset },
        SectionOffset {
            section: parent_section,
            offset: parent_offset,
        },
    ))
}

/// Write two offsets and then two section numbers.
fn write_address_pair(
    out: &mut Output<'_>,
    first: SectionOffset,
    second: SectionOffset,
) -> Result<()> {
    out.u32(first.offset)?;
    out.u32(second.offset)?;
    out.u16(first.section)?;
    out.u16(second.section)
}

/// The location of a static local variable ([`S_STATICLOCAL`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StaticLocal {
    /// The address of the variable (`off` and `sect`).
    pub address: SectionOffset,
    /// The address of the enclosing scope (`offParent` and `sectParent`).
    pub parent_address: SectionOffset,
}

impl StaticLocal {
    /// Read the fields. Both offsets come before both section numbers.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let (address, parent_address) = read_address_pair(fields)?;
        Ok(Self {
            address,
            parent_address,
        })
    }

    /// Write the fields.
    pub(super) fn write(self, out: &mut Output<'_>) -> Result<()> {
        write_address_pair(out, self.address, self.parent_address)
    }
}

/// A virtual function table path record ([`S_VFTABLE32`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VirtualFunctionTable {
    /// The type index of the root of the path.
    pub root: TypeIndex,
    /// The type index of the path record.
    pub path: TypeIndex,
    /// The address of the table.
    pub address: SectionOffset,
}

impl VirtualFunctionTable {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let root = fields.type_index()?;
        let path = fields.type_index()?;
        let address = SectionOffset::read(fields)?;
        Ok(Self {
            root,
            path,
            address,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.root)?;
        out.type_index(self.path)?;
        self.address.write(out)
    }
}

/// A list of annotation strings at a code address ([`S_ANNOTATION`]).
///
/// MSVC writes the strings of the `__annotation` intrinsic in this record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AnnotationStrings<'a> {
    /// The code address.
    pub address: SectionOffset,
    /// The strings with their NUL terminators.
    strings: &'a [u8],
    /// The number of strings.
    count: u16,
}

impl<'a> AnnotationStrings<'a> {
    /// Create a value from strings with their NUL terminators.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `strings` does not end with a
    /// NUL byte. Returns [`ErrorKind::LimitExceeded`] for more than 65535
    /// strings.
    pub fn new(address: SectionOffset, strings: &'a [u8]) -> Result<Self> {
        if strings.last().is_some_and(|&byte| byte != 0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("annotation strings do not end with a NUL byte"));
        }
        // `strings` ends with a NUL byte, so the last part of the split is empty.
        let count = strings.split(|&byte| byte == 0).count() - 1;
        let count = u16::try_from(count).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("S_ANNOTATION has more than 65535 strings")
        })?;
        Ok(Self {
            address,
            strings,
            count,
        })
    }

    /// Read the address, the count and the strings.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let address = SectionOffset::read(fields)?;
        let count = fields.u16()?;
        let rest = fields.rest();
        let mut length = 0;
        for _ in 0..count {
            let end = rest[length..]
                .iter()
                .position(|&byte| byte == 0)
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidFormat, None)
                        .with_context("S_ANNOTATION string has no NUL terminator")
                })?;
            length += end + 1;
        }
        let strings = fields.bytes(length)?;
        Ok(Self {
            address,
            strings,
            count,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.address.write(out)?;
        out.u16(self.count)?;
        out.bytes(self.strings)
    }

    /// Return the number of strings.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count as usize
    }

    /// Return `true` if the record has no strings.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return an iterator over the strings, without their terminators.
    pub fn iter(&self) -> impl Iterator<Item = &'a [u8]> + 'a {
        let strings = self.strings;
        strings
            .strip_suffix(&[0])
            .unwrap_or(strings)
            .split(|&byte| byte == 0)
            .take(usize::from(self.count))
    }
}

/// A hot-patch function record ([`S_HOTPATCHFUNC`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HotPatchFunction<'a> {
    /// The ID index of the function in the IPI stream.
    pub function: IdIndex,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> HotPatchFunction<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let function = fields.id_index()?;
        let name = fields.name()?;
        Ok(Self { function, name })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.id_index(self.function)?;
        out.c_string(self.name)
    }
}

values! {
    /// The kind of an association (`CV_AssociationKind_e` in `cvconst.h`).
    AssociationKind(u16) {
        /// No associated symbol.
        NONE = 0;
        /// The associated symbol is the primary coroutine function.
        COROUTINE = 1;
    }
}

/// An association with another symbol ([`S_ASSOCIATION`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Association {
    /// The kind of the association.
    pub kind: AssociationKind,
    /// The address of the associated symbol.
    pub address: SectionOffset,
}

impl Association {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let kind = AssociationKind(fields.u16()?);
        let address = SectionOffset::read(fields)?;
        Ok(Self { kind, address })
    }

    /// Write the fields.
    pub(super) fn write(self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.kind.0)?;
        self.address.write(out)
    }
}

/// A code range of one architecture in a hybrid image ([`S_HYBRIDRANGE`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HybridRange {
    /// The length of the range in bytes.
    pub length: u32,
    /// The start address.
    pub address: SectionOffset,
    /// The architecture of the code (`arch`). No reference gives the values.
    pub architecture: u16,
}

impl HybridRange {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let length = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let architecture = fields.u16()?;
        Ok(Self {
            length,
            address,
            architecture,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.length)?;
        self.address.write(out)?;
        out.u16(self.architecture)
    }
}
