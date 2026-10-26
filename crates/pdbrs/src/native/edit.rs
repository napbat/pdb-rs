//! Semantic edits of an existing native PDB.
//!
//! [`Pdb::edit`] makes a [`PdbEditor`]. The editor borrows the source PDB and
//! owns only the changes. [`PdbEditor::plan`] checks all changes, encodes the
//! streams that the changes affect and returns a [`PdbEditPlan`]. The plan
//! writes the edited PDB.
//!
//! # Stream IDs and unchanged streams
//!
//! The output keeps the stream ID of each source stream:
//!
//! - The plan does not decode or encode a stream that no change affects. It
//!   reads the source bytes of that stream when it writes. Thus an exact
//!   stream comparison, for example `pdbtool compare`, shows the stream as
//!   identical.
//! - A removed stream becomes a nil stream, so the IDs of the other streams
//!   do not change.
//! - A new stream gets the next stream ID after the last source stream.
//! - Stream 0, the previous stream directory, is empty, as
//!   [`OldDirectoryPolicy::Empty`](crate::native::msf::OldDirectoryPolicy::Empty)
//!   writes it. The default page size is the page size of the source.
//!
//! # Supported edits
//!
//! | Edit | Functions | Streams that change |
//! | --- | --- | --- |
//! | Named streams | [`PdbEditor::set_named_stream`], [`PdbEditor::remove_named_stream`] | Info, the named stream |
//! | Public and global symbols | [`PdbEditor::add_public`], [`PdbEditor::add_global`], [`PdbEditor::rename_public`], [`PdbEditor::rename_global`] | Symbol records, global index, public index, module streams with moved global references |
//! | Types and IDs | [`PdbEditor::add_type`], [`PdbEditor::add_id`] | TPI or IPI and its hash stream |
//! | Identity | [`Pdb::edit`], [`PdbEditor::set_identity`] | Info, DBI if the DBI age changes |
//! | Strip private symbols | [`PdbEditor::strip_private`] | See below |
//!
//! A renamed record stays at its position: the plan encodes it again with
//! the new name. An added record goes to the end of the symbol record stream.
//! If the size of a renamed record changes, the offsets of the later records
//! move. The plan then writes the new offsets into the global and public
//! indexes and into the global references of the module streams. The global
//! and public indexes keep the bucket and the reference count of each source
//! hash record, and the thunk map and the section starts of the source.
//!
//! An added type or ID record gets the next index after the last source
//! record, so the indices of the source records do not change. The hash
//! stream keeps the source hash values, index offsets and hash adjusters.
//!
//! [`PdbEditor::strip_private`] writes the streams that Microsoft
//! `link /PDBSTRIPPED` writes (MSVC 14.51):
//!
//! - The TPI and IPI streams have no records. Their hash streams are empty.
//! - The global index and the symbol record stream keep only the `S_PUB32`
//!   records and the `S_LPROCREF` records of the global index. The public
//!   index keeps all public symbols.
//! - A module stream keeps the local procedures that an `S_LPROCREF` names,
//!   each as an `S_LPROC32` record with type 0 and an `S_END` record. Its
//!   global references are the offsets of these `S_LPROCREF` records. A
//!   module without such a procedure has no module stream.
//! - The DBI header has the stripped flag. The modules have no source files
//!   and no line data. The section contributions have zero CRCs.
//! - The plan removes `/names` and the `/src/` streams. `/names` stays if the
//!   PDB has a new FPO stream, because its frame programs refer to `/names`.
//!   `/TMCache` and `/UDTSRCLINEUNDONE` become empty. The plan removes the
//!   type map streams that `/TMCache` names.
//!
//! # Rejected edits
//!
//! The plan checks all changes before it writes a byte. A planning error
//! writes nothing, so `PdbEditPlan::write_file` makes no file. The plan
//! rejects these edits with [`ErrorKind::InvalidReference`]:
//!
//! - An edit that moves a symbol record, and a strip, if the source has
//!   opaque data that can refer to symbol offsets or type indices: a present
//!   stream that no other stream names, a named stream of an unknown kind, a
//!   symbol record of an unknown kind or a C13 subsection of an unknown kind.
//! - A record with a reference that does not resolve.
//! - A rename of a name that is not in the index or that is not unique.
//!
//! The plan rejects these transformations with
//! [`ErrorKind::UnsupportedVersion`]:
//!
//! - A change of `/names`, `/TMCache`, `/UDTSRCLINEUNDONE` or a `/src/`
//!   stream. Other records refer to their contents by offset, stream ID or
//!   name.
//! - A rename of a reference record such as `S_PROCREF`: the module record
//!   holds the same name.
//! - An added record of a kind that this crate does not decode.
//! - A strip together with a symbol, type or ID edit, and a strip of a
//!   procedure kind that the rules above do not cover.
//!
//! # Identity
//!
//! The caller gives an [`IdentityPolicy`]. The plan keeps the Info age and
//! the DBI age consistent. [`PdbEditPlan::pe_match`] returns the values for
//! the `RSDS` debug entry of the image.
//!
//! Caution: [`IdentityPolicy::Preserve`] after a semantic change keeps the
//! match with the old image. A debugger can then use changed symbols for
//! that image and misidentify them. For a semantic change, use
//! [`IdentityPolicy::IncrementAge`] or [`IdentityPolicy::Replace`], and write
//! the returned values to the image.

use alloc::vec::Vec;

use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::auxiliary::NamedStreamKind;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::index::{IdIndex, TypeIndex};
use crate::native::codeview::symbol::{
    Public, S_CONSTANT, S_GDATA32, S_GTHREAD32, S_LDATA32, S_LTHREAD32, S_PUB32, S_UDT, SymbolData,
};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::info::IdentityPolicy;
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

mod named;
mod opaque;
mod plan;
mod strip;
mod symbols;
mod types;

pub use plan::PdbEditPlan;

/// The stream ID of the TPI stream.
const TPI_STREAM: StreamId = StreamId(2);
/// The stream ID of the IPI stream.
const IPI_STREAM: StreamId = StreamId(4);
/// The offset of `index_end` in a TPI or IPI header.
const INDEX_END_OFFSET: u64 = 12;

/// The symbol index that names an added record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SymbolIndex {
    /// The global symbol index.
    Global,
    /// The public symbol index.
    Public,
}

/// A record that the editor adds to the symbol record stream.
#[derive(Clone, Debug)]
struct AddedSymbol {
    /// The complete record.
    record: Vec<u8>,
    /// The index that names the record.
    index: SymbolIndex,
}

/// A rename of a record of the symbol record stream.
#[derive(Clone, Debug)]
struct Rename {
    /// The index that names the record.
    index: SymbolIndex,
    /// The current name.
    old: Vec<u8>,
    /// The new name.
    new: Vec<u8>,
}

/// A change of a named stream: new bytes, or `None` to remove the stream.
#[derive(Clone, Debug)]
struct NamedChange {
    /// The name.
    name: Vec<u8>,
    /// The new bytes, or `None` to remove the stream.
    bytes: Option<Vec<u8>>,
}

/// The records that the editor appends to a TPI or IPI stream.
#[derive(Clone, Debug, Default)]
struct AddedRecords {
    /// The index of the first added record, read from the source header.
    first: Option<u32>,
    /// The complete records in index order.
    records: Vec<u8>,
    /// The number of records.
    count: u32,
}

/// An editor of a native PDB.
///
/// See the [module documentation](self) for the supported edits, the stream
/// rules and the rejected edits.
pub struct PdbEditor<'p, S> {
    /// The source PDB.
    source: &'p Pdb<S>,
    /// The identity policy.
    identity: IdentityPolicy,
    /// The named stream changes in call order.
    named: Vec<NamedChange>,
    /// The added TPI records.
    types: AddedRecords,
    /// The added IPI records.
    ids: AddedRecords,
    /// The added symbol records.
    symbols: Vec<AddedSymbol>,
    /// The renames.
    renames: Vec<Rename>,
    /// `true` to strip the private symbols.
    strip: bool,
}

impl<S> core::fmt::Debug for PdbEditor<'_, S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PdbEditor")
            .field("identity", &self.identity)
            .field("named", &self.named.len())
            .field("types", &self.types.count)
            .field("ids", &self.ids.count)
            .field("symbols", &self.symbols.len())
            .field("renames", &self.renames.len())
            .field("strip", &self.strip)
            .finish_non_exhaustive()
    }
}

/// Return an [`ErrorKind::UnsupportedVersion`] error for an edit that the
/// editor does not support.
fn unsupported(context: &'static str) -> Error {
    Error::new(ErrorKind::UnsupportedVersion, None).with_context(context)
}

/// Copy `bytes` into a new vector with a checked allocation.
fn copy(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len())
        .map_err(|_| no_memory())?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

/// Push a value with a checked allocation.
fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
    Ok(())
}

/// Return an error if `name` is not a valid record name: it has a NUL byte.
fn check_name(name: &[u8]) -> Result<()> {
    if name.contains(&0) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("symbol name contains a NUL byte"));
    }
    Ok(())
}

impl<S: ReadAt> Pdb<S> {
    /// Make an editor of this PDB with the identity policy `identity`.
    ///
    /// The editor borrows the PDB and reads no stream yet. See
    /// [`PdbEditor`] for the edits.
    ///
    /// Caution: [`IdentityPolicy::Preserve`] after a semantic change keeps
    /// the match with the old image (see the
    /// [module documentation](crate::native::edit#identity)).
    #[must_use]
    pub const fn edit(&self, identity: IdentityPolicy) -> PdbEditor<'_, S> {
        PdbEditor {
            source: self,
            identity,
            named: Vec::new(),
            types: AddedRecords {
                first: None,
                records: Vec::new(),
                count: 0,
            },
            ids: AddedRecords {
                first: None,
                records: Vec::new(),
                count: 0,
            },
            symbols: Vec::new(),
            renames: Vec::new(),
            strip: false,
        }
    }
}

impl<'p, S: ReadAt> PdbEditor<'p, S> {
    /// Return the source PDB.
    #[must_use]
    pub const fn source(&self) -> &'p Pdb<S> {
        self.source
    }

    /// Return the identity policy.
    #[must_use]
    pub const fn identity(&self) -> IdentityPolicy {
        self.identity
    }

    /// Replace the identity policy.
    pub const fn set_identity(&mut self, identity: IdentityPolicy) {
        self.identity = identity;
    }

    /// Add a named stream, or replace the bytes of a named stream.
    ///
    /// A replaced stream keeps its stream ID. A new stream gets the next
    /// stream ID after the last source stream. The plan decodes the bytes of
    /// a `srcsrv`, `sourcelink` or `/LinkInfo` stream with the decoder of its
    /// kind. It keeps the bytes of other names as they are.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the name is empty or contains a NUL
    ///   byte.
    /// - [`ErrorKind::UnsupportedVersion`] for `/names`, `/TMCache`,
    ///   `/UDTSRCLINEUNDONE` or a `/src/` name. Other records refer to the
    ///   contents of these streams.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn set_named_stream(&mut self, name: &[u8], bytes: Vec<u8>) -> Result<()> {
        self.push_named(name, Some(bytes))
    }

    /// Remove a named stream.
    ///
    /// The plan removes the name from the Info stream and writes a nil
    /// stream in its place. It returns [`ErrorKind::MissingStream`] if the
    /// source and the earlier changes have no stream with this name.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbEditor::set_named_stream`] for the name.
    pub fn remove_named_stream(&mut self, name: &[u8]) -> Result<()> {
        self.push_named(name, None)
    }

    /// Record one named stream change.
    fn push_named(&mut self, name: &[u8], bytes: Option<Vec<u8>>) -> Result<()> {
        if name.is_empty() || name.contains(&0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("named stream name is empty or contains a NUL byte"));
        }
        if matches!(
            NamedStreamKind::classify(name),
            NamedStreamKind::Names
                | NamedStreamKind::TmCache
                | NamedStreamKind::UdtSourceLineUndone
                | NamedStreamKind::SourceHeaderBlock
                | NamedStreamKind::SourceFile { .. }
        ) {
            return Err(unsupported(
                "other records refer to the contents of this named stream",
            ));
        }
        let name = copy(name)?;
        push(&mut self.named, NamedChange { name, bytes })
    }

    /// Add a public symbol (`S_PUB32`) to the symbol record stream and to
    /// the public index.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the name contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if the record is too long or an
    ///   allocation fails.
    pub fn add_public(&mut self, public: &Public<'_>) -> Result<()> {
        check_name(public.name)?;
        let mut record = Vec::new();
        SymbolData::Public(*public).encode(S_PUB32, &mut record)?;
        push(
            &mut self.symbols,
            AddedSymbol {
                record,
                index: SymbolIndex::Public,
            },
        )
    }

    /// Add a record of `kind` to the symbol record stream and to the global
    /// index, with a reference count of 1.
    ///
    /// The kinds are `S_GDATA32`, `S_LDATA32`, `S_GTHREAD32`, `S_LTHREAD32`,
    /// `S_CONSTANT` and `S_UDT`. The plan checks the type index of the record
    /// against the TPI stream with its added records.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] for another kind.
    /// - The errors of [`SymbolData::encode`].
    pub fn add_global(&mut self, kind: u16, data: &SymbolData<'_>) -> Result<()> {
        if !matches!(
            kind,
            S_GDATA32 | S_LDATA32 | S_GTHREAD32 | S_LTHREAD32 | S_CONSTANT | S_UDT
        ) {
            return Err(unsupported(
                "the editor adds only data, thread data, constant and UDT global records",
            ));
        }
        let mut record = Vec::new();
        data.encode(kind, &mut record)?;
        push(
            &mut self.symbols,
            AddedSymbol {
                record,
                index: SymbolIndex::Global,
            },
        )
    }

    /// Rename the public symbol `old` to `new`.
    ///
    /// The plan finds the `S_PUB32` record with the name `old` through the
    /// public index. The name must identify exactly one record.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a name contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn rename_public(&mut self, old: &[u8], new: &[u8]) -> Result<()> {
        self.push_rename(SymbolIndex::Public, old, new)
    }

    /// Rename the global symbol `old` to `new`.
    ///
    /// The plan finds the record with the name `old` through the global
    /// index. The name must identify exactly one record. The plan renames
    /// data, thread data, constant and UDT records. It rejects a reference
    /// record such as `S_PROCREF`, because the module record has the same
    /// name.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbEditor::rename_public`].
    pub fn rename_global(&mut self, old: &[u8], new: &[u8]) -> Result<()> {
        self.push_rename(SymbolIndex::Global, old, new)
    }

    /// Record one rename.
    fn push_rename(&mut self, index: SymbolIndex, old: &[u8], new: &[u8]) -> Result<()> {
        check_name(old)?;
        check_name(new)?;
        let rename = Rename {
            index,
            old: copy(old)?,
            new: copy(new)?,
        };
        push(&mut self.renames, rename)
    }

    /// Append a type record to the TPI stream and return its type index.
    ///
    /// The index is the next index after the last source record and the
    /// earlier added records. The plan checks the references of all records.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no TPI stream.
    /// - [`ErrorKind::UnsupportedVersion`] for [`TypeLeaf::Unsupported`].
    /// - [`ErrorKind::LimitExceeded`] if the index does not fit in 32 bits.
    /// - The errors of [`TypeLeaf::encode`] and of the stream reader.
    pub fn add_type(&mut self, leaf: &TypeLeaf<'_>) -> Result<TypeIndex> {
        if matches!(leaf, TypeLeaf::Unsupported(_)) {
            return Err(unsupported("added type record has no decoder"));
        }
        let source = self.source;
        let index = append_record(source, TPI_STREAM, &mut self.types, |out| leaf.encode(out))?;
        Ok(TypeIndex(index))
    }

    /// Append an ID record to the IPI stream and return its ID index.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbEditor::add_type`] for the IPI stream and
    /// [`IdRecord::encode`].
    pub fn add_id(&mut self, record: &IdRecord<'_>) -> Result<IdIndex> {
        if matches!(record, IdRecord::Unsupported(_)) {
            return Err(unsupported("added ID record has no decoder"));
        }
        let source = self.source;
        let index = append_record(source, IPI_STREAM, &mut self.ids, |out| record.encode(out))?;
        Ok(IdIndex(index))
    }

    /// Strip the private symbols as `link /PDBSTRIPPED` does.
    ///
    /// See the [module documentation](self) for the rules. The plan rejects
    /// a strip together with a symbol, type or ID edit.
    pub const fn strip_private(&mut self) {
        self.strip = true;
    }
}

/// Encode one record at the end of `added` and return its index.
fn append_record<S: ReadAt>(
    source: &Pdb<S>,
    stream: StreamId,
    added: &mut AddedRecords,
    encode: impl FnOnce(&mut Vec<u8>) -> Result<()>,
) -> Result<u32> {
    let first = if let Some(first) = added.first {
        first
    } else {
        let reader = source.msf().stream(stream)?.ok_or_else(|| {
            Error::new(ErrorKind::MissingStream, None)
                .with_context("PDB has no type stream for the added record")
        })?;
        let first = reader.read_u32_at(INDEX_END_OFFSET)?;
        added.first = Some(first);
        first
    };
    let index = first.checked_add(added.count).ok_or_else(|| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("type index does not fit in 32 bits")
    })?;
    let length = added.records.len();
    if let Err(error) = encode(&mut added.records) {
        added.records.truncate(length);
        return Err(error);
    }
    added.count = added.count.saturating_add(1);
    Ok(index)
}
