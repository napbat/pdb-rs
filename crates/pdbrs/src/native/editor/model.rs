//! The editor type and its accessors.

use alloc::vec;
use alloc::vec::Vec;

use super::rebuild::Rebuild;
use super::symbols::SharedSymbols;
use super::types::RecordList;
use super::{
    DebugInfo, DebugStreamList, GlobalSymbols, IdList, ModuleContents, NamedStreams, PublicSymbols,
    TypeList, UnnamedStreams, source,
};
use crate::io::ReadAt;
use crate::native::codeview::index::ModuleIndex;
use crate::native::dbi::Module;
use crate::native::info::Feature;
use crate::native::names::{HashVersion, StringTableBuilder};
use crate::native::output::no_memory;
use crate::native::{IdentityPolicy, Pdb};
use crate::{Error, ErrorKind, Result};

/// The identity and the feature codes of a model without a source Info
/// stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct InfoModel {
    /// The 32-bit signature.
    pub(super) signature: u32,
    /// The GUID.
    pub(super) guid: [u8; 16],
    /// The Info age.
    pub(super) age: u32,
    /// The DBI age.
    pub(super) dbi_age: u32,
    /// The feature codes.
    pub(super) features: Vec<Feature>,
}

/// A module of the model: still the source bytes, or decoded.
#[derive(Clone, Debug)]
pub(super) enum ModuleSlot {
    /// A source module that the editor did not decode.
    Source,
    /// A decoded or new module.
    Decoded(alloc::boxed::Box<ModuleContents>),
}

/// The `/names` stream of the model.
#[derive(Clone, Debug)]
pub(super) enum Names {
    /// The model has no `/names` stream.
    Absent,
    /// The string table.
    Table(StringTableBuilder),
}

/// An editor of a native PDB: one model for builds, edits and rebuilds.
///
/// [`PdbEditor::new`] starts without a source. [`Pdb::edit`] starts with a
/// source PDB: the editor borrows it and decodes a stream only on the first
/// `*_mut` call for that stream. [`PdbEditor::decode_all`] decodes every
/// stream. [`PdbEditor::plan`] encodes the streams and returns a
/// [`PdbWritePlan`](crate::native::PdbWritePlan) that writes the PDB.
///
/// # Usage
///
/// ```no_run
/// use pdbrs::native::codeview::index::ModuleIndex;
/// use pdbrs::native::codeview::symbol::S_LABEL32;
/// use pdbrs::native::codeview::type_record::{ArgumentList, TypeLeaf};
/// use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor};
///
/// # #[cfg(feature = "std")]
/// # fn main() -> pdbrs::Result<()> {
/// # let index = ModuleIndex(0);
/// # let leaf = TypeLeaf::ArgumentList(ArgumentList::parse(&[])?);
/// # let (signature, guid, age) = (1, [0; 16], 1);
/// // Read: a session that lends small structures from `&self`.
/// let pdb = Pdb::open("app.pdb")?;
/// if let Some(publics) = pdb.publics()? {
///     for public in publics.symbols() {
///         let public = public?; // borrowed from the session
///     }
/// }
/// let module = pdb.module(index)?; // bulk data: an owned load
///
/// // Edit: copy-on-write per stream.
/// let mut edit = pdb.edit(IdentityPolicy::IncrementAge);
/// let main = edit.module_mut(index)?.procedure_mut(b"main")?; // a scope of the tree
/// main.children.retain(|symbol| symbol.kind() != S_LABEL32); // no S_END to keep in step
/// let added = edit.types_mut()?.push(leaf)?; // decision 6
///
/// // Build: the same type without a source.
/// let mut build = PdbEditor::new(signature, guid, age)?;
///
/// let plan = edit.plan(EditOptions::default())?;
/// plan.write_file("app.edited.pdb")?;
/// // Write the new identity into the RSDS debug entry of the image.
/// let identity = plan.identity()?;
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "std"))]
/// # fn main() {}
/// ```
///
/// # Model
///
/// | Part | Accessor | Records |
/// | --- | --- | --- |
/// | Named streams | [`PdbEditor::named_streams_mut`] | [`NamedStreams`] |
/// | Public and global symbols | [`PdbEditor::publics_mut`], [`PdbEditor::globals_mut`] | [`SymbolRecord`](super::SymbolRecord) by [`SymbolHandle`](crate::native::codeview::index::SymbolHandle) |
/// | Modules | [`PdbEditor::module_mut`], [`PdbEditor::add_module`] | [`ModuleContents`]: the scope tree of [`ModuleSymbol`](super::ModuleSymbol), the C13 subsections, the source files |
/// | Types and IDs | [`PdbEditor::types_mut`], [`PdbEditor::ids_mut`] | [`TypeList`], [`IdList`] |
/// | `/names` | [`PdbEditor::names_mut`] | [`StringTableBuilder`] |
/// | DBI values | [`PdbEditor::debug_info_mut`] | [`DebugInfo`] |
/// | Optional debug streams | [`PdbEditor::debug_streams_mut`] | [`DebugStreamList`] |
/// | Streams without a name | [`PdbEditor::unnamed_streams_mut`] | [`UnnamedStreams`] |
/// | Feature codes | [`PdbEditor::set_features`] | [`Feature`] |
///
/// Each accessor decodes its part on the first call and keeps it decoded.
/// An accessor is atomic: an error leaves the model unchanged. The encoding
/// is deferred: the accessors change only the model, and the plan encodes
/// all streams. The output depends only on the final model, not on the
/// order of the calls. Two changes of the same record do not commute: the
/// last change wins.
///
/// # Tasks
///
/// The common edits are functions over the same model (#41). Each one checks
/// all its inputs first, so an error leaves the model unchanged:
///
/// - [`PdbEditor::set_source_link`]: the `sourcelink` named stream.
/// - [`PdbEditor::set_source_server`]: the `srcsrv` named stream.
/// - [`PdbEditor::remove_named_stream`]: a named stream and its Info entry.
/// - [`PdbEditor::rename_symbols`]: public and global symbols, and module
///   procedures with their reference records.
///
/// # Layouts
///
/// The plan writes one of two layouts:
///
/// - **Copy-on-write** (from [`Pdb::edit`]): each source stream keeps its
///   stream ID. The plan copies each stream that no change touches, and
///   reads its bytes from the source when it writes. A new stream gets the
///   next stream ID after the source streams, in this order: streams without
///   a name, named streams, module streams and debug streams. A removed
///   stream becomes a nil stream. Stream 0, the previous stream directory,
///   is empty.
/// - **New layout** (from [`PdbEditor::new`], or after
///   [`PdbEditor::decode_all`]): the plan assigns the stream IDs: 0 to 4 for
///   the previous stream directory, Info, TPI, DBI and IPI, then the TPI and
///   IPI hash streams, the global index, the public index, the symbol record
///   stream, the module streams in module order, the optional debug streams
///   in slot order, `/names`, the named streams and the streams without a
///   name. After `decode_all`, the plan drops each source stream that no
///   other stream names; the report gives it as removed.
///
/// In both layouts, type indices, ID indices and `/names` offsets keep their
/// values: the index spaces only grow (rule 5 of #42). A removal of type or
/// ID records is the exception: see below.
///
/// # Removal of type and ID records
///
/// Caution: the plan cannot see references in other files. An object file
/// that uses a type-server PDB, a precompiled-header object (`/Yc`) and the
/// object files of a `/DEBUG:FASTLINK` PDB keep the old indices. Remove
/// records only from a PDB that no other file refers to by index.
///
/// [`TypeList::remove`](super::TypeList::remove) and
/// [`IdList::remove`](super::IdList::remove) make a record a tombstone (#47).
/// In the model, the other records keep their indices, and a record that
/// [`TypeList::push`](super::TypeList::push) adds gets the next index. After
/// a removal, this index is a handle. The plan compacts each index space
/// with a removed record:
///
/// - The records that are not removed keep their order and get the next
///   output index. [`PdbWritePlan::type_index`](crate::native::PdbWritePlan::type_index)
///   and [`PdbWritePlan::id_index`](crate::native::PdbWritePlan::id_index)
///   give the output index of a model index.
/// - The plan writes the TPI and IPI records with the output indices and
///   builds their hash streams again: hash values, index offsets and hash
///   adjusters. A hash adjuster of a removed record goes away.
/// - The plan writes the output index into each reference of the symbol
///   record stream, of the module streams (symbol records and the C13
///   subsections), of the type map streams and of `/UDTSRCLINEUNDONE`. An
///   index field keeps its width, so no offset moves.
/// - A reference to a removed record fails the plan with
///   [`ErrorKind::InvalidReference`]. For a TPI or IPI record that refers,
///   [`Error::record`] gives its space and its model index. For another
///   record, [`Error::stream_location`] gives the output stream and the
///   offset of the record.
/// - The plan refuses the compaction with [`ErrorKind::UnsupportedEdit`] for
///   a PDB without a DBI stream (a type server), a `/DEBUG:FASTLINK` PDB
///   (feature `MINI`), a PDB without type merging (feature `NOTM`,
///   `S_MOD_TYPEREF`), a PDB whose modules use type servers, and for data
///   that can hold an index without a decoder (see below).
///
/// # Dependencies
///
/// The plan derives the structures that depend on a change:
///
/// | Changed part | The plan also writes |
/// | --- | --- |
/// | Named streams | the Info named-stream map |
/// | Public and global records | the symbol record stream, the global and public indexes, the global references of the modules |
/// | Module symbols or lines | the module stream (scope links and offsets from the tree), the DBI module record sizes, the DBI source info if the files change, the targets of the reference records of the symbol record stream |
/// | Types or IDs | the TPI or IPI stream and its hash stream; after a removal, also each stream with a reference to a moved index |
/// | `/names` | the string table and its hash table |
/// | Identity, features | the Info stream and the DBI age |
///
/// In the copy-on-write layout, the plan keeps the conventions of each
/// source stream: a source hash record keeps its bucket and reference count,
/// a TPI or IPI stream with appended records keeps its header and hash
/// buffers, and the DBI and Info streams keep their other values. A record
/// that moves gets its new offset in each decoded reference.
///
/// # Rejected edits
///
/// The plan checks all changes before it writes a byte. A planning error
/// writes nothing. The plan rejects these edits with
/// [`ErrorKind::UnsupportedEdit`]:
///
/// - An edit that moves symbol records, and a strip, if the source has
///   opaque data that can refer to symbol offsets or type indices: a present
///   stream that no other stream names and that does not decode as a type
///   map stream, a named stream of an unknown kind, a symbol record of an
///   unknown kind, a C13 subsection of an unknown kind or C11 line data. A
///   type map without an owner holds type indices, not symbol offsets, so it
///   does not block an edit. A module type stream belongs to the module
///   whose `S_MOD_TYPEREF` record names it. The error names the stream and
///   the index space ([`Error::index_space`]).
/// - A removal of type or ID records, if the output has data that can hold
///   an index without a decoder: a TPI or IPI record, a symbol record or a
///   C13 subsection of an unknown kind, a named stream of an unknown kind, a
///   present stream that no other stream names and that does not decode as a
///   type map stream, a type map of an object file (version 2: only the
///   object file tells TPI and IPI indices apart), a managed symbol whose
///   token field can be a type index, and an `S_DISCARDED` record with data.
///   A type map of a type server (version 1) gets the output indices.
/// - A change of `/TMCache`, `/UDTSRCLINEUNDONE` or a `/src/` stream in the
///   copy-on-write layout: other records refer to their contents.
/// - A new name of a reference record such as `S_PROCREF`, unless the module
///   record that it names gets the same name in the model:
///   [`PdbEditor::rename_symbols`] renames both.
/// - A symbol edit of a module whose symbols do not form a scope tree.
/// - A type or ID record that refers to itself or to a later record of its
///   index space (decision 7 of #42).
/// - A strip together with other edits (see below).
///
/// The plan rejects a reference that does not resolve with
/// [`ErrorKind::InvalidReference`], for example a reference record whose
/// target the model does not have any more. The error has the stream and
/// the offset of the reference record.
///
/// # Strip of private symbols
///
/// [`EditOptions::with_strip_private`](crate::native::EditOptions::with_strip_private) writes the streams that Microsoft
/// `link /PDBSTRIPPED` writes (MSVC 14.51):
///
/// - The TPI and IPI streams have no records. Their hash streams are empty.
/// - The global index and the symbol record stream keep only the `S_PUB32`
///   records and the `S_LPROCREF` records of the global index. The public
///   index keeps all public symbols.
/// - A module stream keeps the local procedures that an `S_LPROCREF` names,
///   each as an `S_LPROC32` record with type 0 and an `S_END` record. Its
///   global references are the offsets of these `S_LPROCREF` records. A
///   module without such a procedure has no module stream.
/// - The DBI header has the stripped flag. The modules have no source files,
///   no line data and no EC name indices. The EC name table is empty. The
///   section contributions have zero CRCs.
/// - The plan removes `/names` and the `/src/` streams. `/names` stays if the
///   PDB has a new FPO stream, because its frame programs refer to `/names`.
///   `/TMCache` and `/UDTSRCLINEUNDONE` become empty. The plan removes the
///   type map streams that `/TMCache` names and the type map streams that no
///   stream names (type maps without an owner).
/// - In a PDB without type merging (feature `NOTM`, `link /DEBUG:LAZY`), the
///   plan removes the module type streams that the `S_MOD_TYPEREF` records
///   name, and the Info stream loses the `NOTM` feature code.
///
/// The strip is a mode of the plan, not a change of the model (rule 13 of
/// #42). It works together with named stream changes and an identity
/// policy. The plan rejects it with [`ErrorKind::UnsupportedEdit`] if the
/// model has another decoded part, after [`PdbEditor::decode_all`], and
/// without a source.
///
/// # Identity
///
/// [`PdbEditor::new`] takes the signature, the GUID and the age. The DBI
/// header gets the same age. [`Pdb::edit`] takes an [`IdentityPolicy`]. The
/// plan keeps the Info age and the DBI age consistent.
/// [`PdbWritePlan::identity`](crate::native::PdbWritePlan::identity) returns the identity for the `RSDS` debug
/// entry of the image. Updating the image is the work of the caller.
///
/// Caution: [`IdentityPolicy::Preserve`] after a semantic change keeps the
/// match with the old image. A debugger can then use changed symbols for
/// that image and misidentify them. For a semantic change, use
/// [`IdentityPolicy::IncrementAge`] or [`IdentityPolicy::Replace`], and write
/// the returned identity to the image.
///
/// The same model and the same [`EditOptions`](crate::native::EditOptions) give the same bytes. The plan
/// does not read a clock or random values. This is deterministic
/// reconstruction, not exact copying: see [`crate::io::copy_exact`] for exact
/// copies.
pub struct PdbEditor<'p, S = &'static [u8]> {
    /// The source PDB, or `None` for a new PDB.
    pub(super) source: Option<&'p Pdb<S>>,
    /// The identity policy for the source identity.
    pub(super) policy: IdentityPolicy,
    /// The identity and features of the new layout, or `None` for the
    /// source Info stream.
    pub(super) info: Option<InfoModel>,
    /// The new feature codes of the copy-on-write layout.
    pub(super) features: Option<Vec<Feature>>,
    /// The named streams.
    pub(super) named: Option<NamedStreams>,
    /// The symbol record stream and its indexes.
    pub(super) symbols: Option<SharedSymbols>,
    /// The modules.
    pub(super) modules: Option<Vec<ModuleSlot>>,
    /// The TPI records.
    pub(super) types: Option<TypeList>,
    /// The IPI records.
    pub(super) ids: Option<IdList>,
    /// The `/names` stream.
    pub(super) names: Option<Names>,
    /// The DBI values.
    pub(super) dbi: Option<DebugInfo>,
    /// The optional debug streams.
    pub(super) debug: Option<DebugStreamList>,
    /// The streams without a name.
    pub(super) unnamed: Option<UnnamedStreams>,
    /// The rebuild state after [`PdbEditor::decode_all`].
    pub(super) rebuild: Option<Rebuild>,
}

impl<S> core::fmt::Debug for PdbEditor<'_, S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PdbEditor")
            .field("source", &self.source.is_some())
            .field("policy", &self.policy)
            .field(
                "new_layout",
                &(self.source.is_none() || self.rebuild.is_some()),
            )
            .field("named", &self.named.is_some())
            .field("symbols", &self.symbols.is_some())
            .field("types", &self.types.as_ref().map(TypeList::len))
            .field("ids", &self.ids.as_ref().map(IdList::len))
            .finish_non_exhaustive()
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Make an editor of this PDB with the identity policy `policy`.
    ///
    /// The editor borrows the PDB and reads no stream yet. See
    /// [`PdbEditor`] for the edits.
    ///
    /// Caution: [`IdentityPolicy::Preserve`] after a semantic change keeps
    /// the match with the old image (see the
    /// [`PdbEditor` documentation](PdbEditor#identity)).
    #[must_use]
    pub const fn edit(&self, policy: IdentityPolicy) -> PdbEditor<'_, S> {
        PdbEditor {
            source: Some(self),
            policy,
            info: None,
            features: None,
            named: None,
            symbols: None,
            modules: None,
            types: None,
            ids: None,
            names: None,
            dbi: None,
            debug: None,
            unnamed: None,
            rebuild: None,
        }
    }
}

impl PdbEditor<'static> {
    /// Create an editor of a new PDB with an explicit identity and no
    /// records.
    ///
    /// The Info stream and the DBI header get `age`. The model has a `/names`
    /// table with the [`HashVersion::V1`] hash, an empty EC name table and
    /// the feature code [`Feature::VC140`], as Microsoft `link.exe` writes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn new(signature: u32, guid: [u8; 16], age: u32) -> Result<Self> {
        Ok(Self {
            source: None,
            policy: IdentityPolicy::Preserve,
            info: Some(InfoModel {
                signature,
                guid,
                age,
                dbi_age: age,
                features: vec![Feature::VC140],
            }),
            features: None,
            named: Some(NamedStreams::default()),
            symbols: Some(SharedSymbols::default()),
            modules: Some(Vec::new()),
            types: Some(TypeList::from_list(RecordList::new())),
            ids: Some(IdList::from_list(RecordList::new())),
            names: Some(Names::Table(StringTableBuilder::new(HashVersion::V1)?)),
            dbi: Some(DebugInfo::new()?),
            debug: Some(DebugStreamList::default()),
            unnamed: Some(UnnamedStreams::new(5)),
            rebuild: None,
        })
    }
}

/// Return the error for a stream that an accessor needs.
pub(super) fn missing(context: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(context)
}

impl<'p, S: ReadAt> PdbEditor<'p, S> {
    /// Return the source PDB, or `None` for a new PDB.
    #[must_use]
    pub const fn source(&self) -> Option<&'p Pdb<S>> {
        self.source
    }

    /// Return the identity policy.
    #[must_use]
    pub const fn identity_policy(&self) -> IdentityPolicy {
        self.policy
    }

    /// Return `true` if the plan writes the new layout: the editor has no
    /// source, or [`PdbEditor::decode_all`] decoded every stream.
    #[must_use]
    pub const fn is_new_layout(&self) -> bool {
        self.source.is_none() || self.rebuild.is_some()
    }

    /// Return the source PDB of a copy-on-write accessor.
    pub(super) fn cow_source(&self) -> Result<&'p Pdb<S>> {
        self.source
            .ok_or_else(|| missing("the editor has no source for this part"))
    }

    /// Return the named streams other than `/names`, for a change.
    ///
    /// # Errors
    ///
    /// Returns the errors of the Info stream decoder.
    pub fn named_streams_mut(&mut self) -> Result<&mut NamedStreams> {
        if self.named.is_none() {
            let named = NamedStreams::from_info(self.cow_source()?.info())?;
            self.named = Some(named);
        }
        self.named.as_mut().ok_or_else(|| missing("named streams"))
    }

    /// Decode the symbol record stream and its indexes, if not yet decoded.
    fn shared(&mut self) -> Result<&mut SharedSymbols> {
        if self.symbols.is_none() {
            let decoded = source::shared_symbols(self.cow_source()?)?;
            self.symbols = Some(decoded);
        }
        self.symbols
            .as_mut()
            .ok_or_else(|| missing("symbol record stream"))
    }

    /// Return the public symbols for a change.
    ///
    /// The first call decodes the symbol record stream, the global index
    /// and the public index.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no DBI stream or no
    ///   symbol record stream.
    /// - The errors of the stream readers and the record decoders.
    pub fn publics_mut(&mut self) -> Result<PublicSymbols<'_>> {
        Ok(PublicSymbols {
            model: self.shared()?,
        })
    }

    /// Return the global symbols for a change.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbEditor::publics_mut`].
    pub fn globals_mut(&mut self) -> Result<GlobalSymbols<'_>> {
        Ok(GlobalSymbols {
            model: self.shared()?,
        })
    }

    /// Return the module list, with a slot for each source module.
    fn module_slots(&mut self) -> Result<&mut Vec<ModuleSlot>> {
        if self.modules.is_none() {
            let slots = source::module_slots(self.cow_source()?)?;
            self.modules = Some(slots);
        }
        self.modules.as_mut().ok_or_else(|| missing("modules"))
    }

    /// Return the module `index` for a change.
    ///
    /// The first call for a module decodes its DBI record, its source files
    /// and its module stream: the symbols into a scope tree and the C13
    /// subsections.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the PDB has no module `index`.
    /// - [`ErrorKind::MissingStream`] if the PDB has no DBI stream.
    /// - The errors of the stream readers and the record decoders.
    pub fn module_mut(&mut self, index: ModuleIndex) -> Result<&mut ModuleContents> {
        let source = self.source;
        let slots = self.module_slots()?;
        let slot = slots.get_mut(index.0 as usize).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("module index is outside the module list")
        })?;
        if let ModuleSlot::Source = slot {
            let pdb = source.ok_or_else(|| missing("the editor has no source for this module"))?;
            *slot = ModuleSlot::Decoded(alloc::boxed::Box::new(source::module(pdb, index)?));
        }
        match slot {
            ModuleSlot::Decoded(module) => Ok(module),
            ModuleSlot::Source => Err(missing("module")),
        }
    }

    /// Add a module with the module record `record` and return its index.
    ///
    /// The module has a module stream with an empty symbol region. The plan
    /// sets the stream and the region sizes of the record.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the module count does not fit in 32
    ///   bits or an allocation fails. The plan rejects more than 65535
    ///   modules.
    /// - The errors of [`PdbEditor::module_mut`] for the source module list.
    pub fn add_module(&mut self, record: Module<'_>) -> Result<ModuleIndex> {
        let slots = self.module_slots()?;
        let index = u32::try_from(slots.len())
            .ok()
            .filter(|&index| index != u32::MAX)
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many modules")
            })?;
        slots.try_reserve(1).map_err(|_| no_memory())?;
        slots.push(ModuleSlot::Decoded(alloc::boxed::Box::new(
            ModuleContents::new(ModuleIndex(index), record.into_owned()),
        )));
        Ok(ModuleIndex(index))
    }

    /// Return the number of modules.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbEditor::module_mut`] for the source module
    /// list.
    pub fn module_count(&mut self) -> Result<usize> {
        Ok(self.module_slots()?.len())
    }

    /// Return the type records of the TPI stream for a change.
    ///
    /// The first call decodes every record of the TPI stream and its hash
    /// adjusters.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no TPI stream.
    /// - The errors of the stream readers and the record decoders.
    pub fn types_mut(&mut self) -> Result<&mut TypeList> {
        if self.types.is_none() {
            let list = source::type_list(self.cow_source()?)?;
            self.types = Some(TypeList::from_list(list));
        }
        self.types.as_mut().ok_or_else(|| missing("TPI"))
    }

    /// Return the ID records of the IPI stream for a change.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbEditor::types_mut`] for the IPI stream.
    pub fn ids_mut(&mut self) -> Result<&mut IdList> {
        if self.ids.is_none() {
            let list = source::id_list(self.cow_source()?)?;
            self.ids = Some(IdList::from_list(list));
        }
        self.ids.as_mut().ok_or_else(|| missing("IPI"))
    }

    /// Return the `/names` string table for new names.
    ///
    /// File checksums, frame programs and other records refer to names by
    /// their offset. A new name goes to the end of the table, so the offsets
    /// of the other names stay (rule 5 of #42).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no `/names` stream.
    /// - The errors of the stream reader and of [`StringTableBuilder::from_table`].
    pub fn names_mut(&mut self) -> Result<&mut StringTableBuilder> {
        if self.names.is_none() {
            let table = source::names(self.cow_source()?)?;
            self.names = Some(table);
        }
        match &mut self.names {
            Some(Names::Table(table)) => Ok(table),
            _ => Err(missing("the PDB has no /names table")),
        }
    }

    /// Return the DBI values that the plan does not compute, for a change.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no DBI stream.
    /// - The errors of the DBI decoder.
    pub fn debug_info_mut(&mut self) -> Result<&mut DebugInfo> {
        if self.dbi.is_none() {
            let dbi = self
                .cow_source()?
                .debug_info()?
                .ok_or_else(|| missing("PDB has no DBI stream"))?;
            self.dbi = Some(DebugInfo::decode(dbi)?);
        }
        self.dbi.as_mut().ok_or_else(|| missing("DBI"))
    }

    /// Return the optional debug streams for a change.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbEditor::debug_info_mut`].
    pub fn debug_streams_mut(&mut self) -> Result<&mut DebugStreamList> {
        if self.debug.is_none() {
            let dbi = self
                .cow_source()?
                .debug_info()?
                .ok_or_else(|| missing("PDB has no DBI stream"))?;
            self.debug = Some(DebugStreamList::from_dbi(dbi)?);
        }
        self.debug.as_mut().ok_or_else(|| missing("debug streams"))
    }

    /// Return the streams without a name for a change.
    ///
    /// The model has only the new streams; the source streams without a name
    /// stay as they are.
    ///
    /// # Errors
    ///
    /// This function returns no error for an editor with a source; the
    /// `Result` keeps the shape of the other accessors.
    pub fn unnamed_streams_mut(&mut self) -> Result<&mut UnnamedStreams> {
        if self.unnamed.is_none() {
            let first = self.cow_source()?.container().stream_count();
            self.unnamed = Some(UnnamedStreams::new(first));
        }
        self.unnamed
            .as_mut()
            .ok_or_else(|| missing("unnamed streams"))
    }

    /// Replace the feature codes of the Info stream.
    ///
    /// [`Feature::MINIMAL_DEBUG_INFO`] selects the larger bucket count of the
    /// symbol indexes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn set_features(&mut self, features: &[Feature]) -> Result<()> {
        let mut list = Vec::new();
        list.try_reserve_exact(features.len())
            .map_err(|_| no_memory())?;
        list.extend_from_slice(features);
        match &mut self.info {
            Some(info) => info.features = list,
            None => self.features = Some(list),
        }
        Ok(())
    }

    /// Return the feature codes of the model.
    pub(super) fn feature_codes(&self) -> Vec<Feature> {
        if let Some(info) = &self.info {
            return info.features.clone();
        }
        if let Some(features) = &self.features {
            return features.clone();
        }
        self.source
            .map(|pdb| pdb.info().features().collect())
            .unwrap_or_default()
    }

    /// Return `true` if the model has a decoded part other than the named
    /// streams.
    pub(super) fn has_model_edits(&self) -> bool {
        self.symbols.is_some()
            || self.modules.is_some()
            || self.types.is_some()
            || self.ids.is_some()
            || self.names.is_some()
            || self.dbi.is_some()
            || self.debug.is_some()
            || self.unnamed.is_some()
            || self.features.is_some()
    }
}
