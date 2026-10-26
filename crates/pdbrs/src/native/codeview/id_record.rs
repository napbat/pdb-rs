//! ID records of the IPI stream.
//!
//! The IPI stream holds ID records. They use the same framing as type
//! records, but their leaf kinds and their index space are different.
//! [`IdRecord::decode`] decodes one framed [`Record`]. It keeps references
//! to IPI records as [`IdIndex`] values and references to TPI records as
//! [`TypeIndex`] values. It keeps `/names` byte offsets as plain `u32`
//! values. It does not resolve any reference. [`IdTable`] resolves string
//! IDs and validates references.
//!
//! The layouts come from these sources:
//!
//! - `lfFuncId`, `lfMFuncId`, `lfStringId`, `lfUdtSrcLine`,
//!   `lfUdtModSrcLine` and `lfBuildInfo` in `cvinfo.h` of
//!   `microsoft/microsoft-pdb`.
//! - `CodeViewTypes.def` and `TypeRecordMapping.cpp` of LLVM for the leaf
//!   kinds and the count fields.
//! - `codeview/src/types/records.rs` of `microsoft/pdb-rs` for the optional
//!   decorated name hash after the name of `LF_FUNC_ID` and `LF_MFUNC_ID`.
//!
//! Each record ends with at most three `LF_PAD` bytes. The decoder rejects
//! all other bytes after the last field.

use core::iter::FusedIterator;

use super::index::{IdIndex, TypeIndex};
use super::{Fields, Record};
use crate::{Error, ErrorKind, Result};

mod encode;
mod table;

pub use table::{IdTable, MAX_STRING_DEPTH};

/// The leaf kind of a function ID (`LF_FUNC_ID`).
pub const LF_FUNC_ID: u16 = 0x1601;
/// The leaf kind of a member function ID (`LF_MFUNC_ID`).
pub const LF_MFUNC_ID: u16 = 0x1602;
/// The leaf kind of a build information record (`LF_BUILDINFO`).
pub const LF_BUILDINFO: u16 = 0x1603;
/// The leaf kind of a substring list (`LF_SUBSTR_LIST`).
pub const LF_SUBSTR_LIST: u16 = 0x1604;
/// The leaf kind of a string ID (`LF_STRING_ID`).
pub const LF_STRING_ID: u16 = 0x1605;
/// The leaf kind of a UDT source line (`LF_UDT_SRC_LINE`).
pub const LF_UDT_SRC_LINE: u16 = 0x1606;
/// The leaf kind of a UDT source line with a module (`LF_UDT_MOD_SRC_LINE`).
pub const LF_UDT_MOD_SRC_LINE: u16 = 0x1607;

/// The size of the optional decorated name hash.
const NAME_HASH_SIZE: usize = 8;

/// The size of one serialized ID index.
const ID_SIZE: usize = 4;

/// The error context for a payload that ends before its last field.
const SHORT_PAYLOAD: &str = "IPI record payload is too short for its fields";

/// One decoded IPI record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum IdRecord<'a> {
    /// An `LF_FUNC_ID` record.
    Function(FunctionId<'a>),
    /// An `LF_MFUNC_ID` record.
    MemberFunction(MemberFunctionId<'a>),
    /// An `LF_BUILDINFO` record.
    BuildInfo(BuildInfo<'a>),
    /// An `LF_SUBSTR_LIST` record: a list of string IDs.
    SubstringList(IdList<'a>),
    /// An `LF_STRING_ID` record.
    String(StringId<'a>),
    /// An `LF_UDT_SRC_LINE` record.
    UdtSourceLine(UdtSourceLine),
    /// An `LF_UDT_MOD_SRC_LINE` record.
    UdtModuleSourceLine(UdtModuleSourceLine),
    /// A record of a kind that this crate does not decode.
    ///
    /// The record stays raw. This includes type leaf kinds, because the IPI
    /// stream must not contain them.
    Unsupported(Record<'a>),
}

impl<'a> IdRecord<'a> {
    /// Decode a framed IPI record.
    ///
    /// Return [`IdRecord::Unsupported`] for a kind that is not an ID kind.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the payload is too short for
    /// its fields, if a name has no NUL terminator, or if the bytes after the
    /// last field are not valid `LF_PAD` padding. Returns
    /// [`ErrorKind::LimitExceeded`] if a list count does not fit in memory.
    /// The error does not have a file offset.
    pub fn decode(record: Record<'a>) -> Result<Self> {
        let mut fields = Fields::new(record.data, ErrorKind::InvalidFormat, SHORT_PAYLOAD);
        let decoded = match record.kind {
            LF_FUNC_ID => Self::Function(FunctionId {
                scope: optional_id(fields.id_index()?),
                function_type: fields.type_index()?,
                name: fields.name()?,
                decorated_name_hash: name_hash(&mut fields)?,
            }),
            LF_MFUNC_ID => Self::MemberFunction(MemberFunctionId {
                parent_type: fields.type_index()?,
                function_type: fields.type_index()?,
                name: fields.name()?,
                decorated_name_hash: name_hash(&mut fields)?,
            }),
            LF_BUILDINFO => {
                let count = usize::from(fields.u16()?);
                Self::BuildInfo(BuildInfo {
                    arguments: id_list(&mut fields, count)?,
                })
            }
            LF_SUBSTR_LIST => {
                let count = usize::try_from(fields.u32()?).map_err(|_| too_many())?;
                Self::SubstringList(id_list(&mut fields, count)?)
            }
            LF_STRING_ID => Self::String(StringId {
                substrings: optional_id(fields.id_index()?),
                string: fields.name()?,
            }),
            LF_UDT_SRC_LINE => Self::UdtSourceLine(UdtSourceLine {
                udt: fields.type_index()?,
                source_file: fields.id_index()?,
                line: fields.u32()?,
            }),
            LF_UDT_MOD_SRC_LINE => Self::UdtModuleSourceLine(UdtModuleSourceLine {
                udt: fields.type_index()?,
                name_offset: fields.u32()?,
                line: fields.u32()?,
                module: fields.u16()?,
                padded: !fields.rest().is_empty(),
            }),
            _ => return Ok(Self::Unsupported(record)),
        };
        fields.finish_type_padding()?;
        Ok(decoded)
    }
}

/// An `LF_FUNC_ID` record: a function that is not a member function.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FunctionId<'a> {
    /// The parent scope, or `None` for the global scope.
    ///
    /// The scope refers to an `LF_STRING_ID` record, for example a namespace
    /// path. The serialized value 0 gives `None`.
    pub scope: Option<IdIndex>,
    /// The function type in the TPI stream.
    pub function_type: TypeIndex,
    /// The undecorated function name, without its NUL terminator.
    pub name: &'a [u8],
    /// The hash of the decorated name, if the record contains it.
    ///
    /// The hash is present if at least 8 bytes follow the name.
    pub decorated_name_hash: Option<u64>,
}

/// An `LF_MFUNC_ID` record: a member function.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct MemberFunctionId<'a> {
    /// The type of the parent class, structure, union or enum in the TPI
    /// stream.
    pub parent_type: TypeIndex,
    /// The member function type in the TPI stream.
    pub function_type: TypeIndex,
    /// The undecorated function name, without its NUL terminator.
    pub name: &'a [u8],
    /// The hash of the decorated name, if the record contains it.
    ///
    /// The hash is present if at least 8 bytes follow the name.
    pub decorated_name_hash: Option<u64>,
}

/// The position of an argument in an `LF_BUILDINFO` record
/// (`CV_BuildInfo_e`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum BuildInfoArgument {
    /// The current directory of the tool.
    CurrentDirectory,
    /// The path of the tool, for example the compiler.
    BuildTool,
    /// The main source file.
    SourceFile,
    /// The program database file of the compiler.
    ProgramDatabase,
    /// The command-line arguments.
    CommandLine,
}

impl BuildInfoArgument {
    /// Return the position of the argument in the record.
    #[must_use]
    pub const fn position(self) -> usize {
        match self {
            Self::CurrentDirectory => 0,
            Self::BuildTool => 1,
            Self::SourceFile => 2,
            Self::ProgramDatabase => 3,
            Self::CommandLine => 4,
        }
    }
}

/// An `LF_BUILDINFO` record.
///
/// A record can have fewer or more arguments than [`BuildInfoArgument`]
/// names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BuildInfo<'a> {
    /// The arguments in their serialized order.
    ///
    /// Each argument refers to an `LF_STRING_ID` record. The value 0 means
    /// that the argument has no value.
    pub arguments: IdList<'a>,
}

impl BuildInfo<'_> {
    /// Return one argument, or `None` if the record does not contain it.
    #[must_use]
    pub fn argument(&self, argument: BuildInfoArgument) -> Option<IdIndex> {
        self.arguments.get(argument.position())
    }
}

/// An `LF_STRING_ID` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct StringId<'a> {
    /// The `LF_SUBSTR_LIST` record with the leading parts of the string, if
    /// any.
    ///
    /// The serialized value 0 gives `None`. [`IdTable::resolve_string`]
    /// puts the substrings before [`StringId::string`].
    pub substrings: Option<IdIndex>,
    /// The string bytes, without the NUL terminator.
    pub string: &'a [u8],
}

/// An `LF_UDT_SRC_LINE` record: the source location of a UDT.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct UdtSourceLine {
    /// The UDT in the TPI stream.
    pub udt: TypeIndex,
    /// The `LF_STRING_ID` record with the source file name.
    pub source_file: IdIndex,
    /// The line number.
    pub line: u32,
}

/// An `LF_UDT_MOD_SRC_LINE` record: the source location of a UDT in a module.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct UdtModuleSourceLine {
    /// The UDT in the TPI stream.
    pub udt: TypeIndex,
    /// The byte offset of the source file name in the `/names` stream.
    pub name_offset: u32,
    /// The line number.
    pub line: u32,
    /// The raw module value that contributes the UDT definition.
    pub module: u16,
    /// `true` if `LF_PAD` padding follows the module field.
    ///
    /// Microsoft `link.exe` writes this record without padding: its 18 bytes
    /// are not a multiple of 4. [`IdRecord::encode`] keeps the form.
    pub padded: bool,
}

/// A borrowed list of serialized ID indices.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IdList<'a> {
    /// The serialized indices.
    entries: &'a [[u8; ID_SIZE]],
}

impl<'a> IdList<'a> {
    /// Return the number of indices.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` if the list has no indices.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the index at `position`, or `None` past the end.
    #[must_use]
    pub fn get(&self, position: usize) -> Option<IdIndex> {
        self.entries.get(position).copied().map(index_from_bytes)
    }

    /// Return an iterator over the indices.
    #[must_use]
    pub fn iter(&self) -> IdListIter<'a> {
        IdListIter {
            entries: self.entries.iter(),
        }
    }
}

impl<'a> IntoIterator for IdList<'a> {
    type Item = IdIndex;
    type IntoIter = IdListIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &IdList<'a> {
    type Item = IdIndex;
    type IntoIter = IdListIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the indices of an [`IdList`].
#[derive(Clone, Debug)]
pub struct IdListIter<'a> {
    /// The remaining serialized indices.
    entries: core::slice::Iter<'a, [u8; ID_SIZE]>,
}

impl Iterator for IdListIter<'_> {
    type Item = IdIndex;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next().copied().map(index_from_bytes)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl DoubleEndedIterator for IdListIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.next_back().copied().map(index_from_bytes)
    }
}

impl ExactSizeIterator for IdListIter<'_> {}

impl FusedIterator for IdListIter<'_> {}

/// Convert a serialized ID index.
const fn index_from_bytes(bytes: [u8; ID_SIZE]) -> IdIndex {
    IdIndex(u32::from_le_bytes(bytes))
}

/// Convert the serialized "no ID" value to `None`.
fn optional_id(index: IdIndex) -> Option<IdIndex> {
    (index != IdIndex::NONE).then_some(index)
}

/// Read the optional decorated name hash after a name.
fn name_hash(fields: &mut Fields<'_>) -> Result<Option<u64>> {
    if fields.rest().len() >= NAME_HASH_SIZE {
        fields.u64().map(Some)
    } else {
        Ok(None)
    }
}

/// Read `count` ID indices.
fn id_list<'a>(fields: &mut Fields<'a>, count: usize) -> Result<IdList<'a>> {
    let size = count.checked_mul(ID_SIZE).ok_or_else(too_many)?;
    let (entries, _) = fields.bytes(size)?.as_chunks::<ID_SIZE>();
    Ok(IdList { entries })
}

/// Return the error for a list count that does not fit in memory.
fn too_many() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("IPI record list count does not fit in memory")
}
