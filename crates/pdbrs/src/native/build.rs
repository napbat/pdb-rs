//! Construction of new native PDBs from semantic records.
//!
//! [`PdbBuilder`] collects the records, [`PdbBuilder::plan`] makes a
//! [`PdbWritePlan`] and [`PdbContents`] reads a PDB for a semantic rebuild.
//! The public handles, the [`Resolver`] and the [`ModuleBuilder`] are in
//! [`crate::native::write`].

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::native::Identity;
use crate::native::IdentityPolicy;
use crate::native::Producer;
use crate::native::codeview::SymbolKind;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::index::NameOffset;
use crate::native::codeview::symbol::{Symbol, SymbolData};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::dbi::{
    DbiHeader, DebugStream, Module, SectionContribution, SectionContributionVersion, SectionMap,
    TypeServer,
};
use crate::native::info::{Feature, InfoBuilder};
use crate::native::names::{HashVersion, StringTableBuilder};
use crate::native::output::no_memory;
use crate::native::symbols::{SectionStart, ThunkTable};
use crate::{Error, ErrorKind, Result};

mod entry;
mod handle;
mod module;
mod options;
mod plan;
mod rebuild;

pub use handle::{
    IdHandle, ModuleHandle, ModuleSymbolHandle, Resolver, StreamHandle, SymbolHandle, TypeHandle,
};
pub use module::ModuleBuilder;
pub use options::BuildOptions;
pub use plan::PdbWritePlan;
pub use rebuild::PdbContents;

use entry::{Entry, SymbolEntry};
use module::position;

/// The records of one type stream (TPI or IPI) in handle order.
#[derive(Debug)]
struct RecordTable<'a, T> {
    /// The record of each handle, or `None` for a reserved handle.
    records: Vec<Option<Entry<'a, T>>>,
    /// The hash adjusters: a record position and a `/names` offset.
    adjusters: Vec<(u32, NameOffset)>,
}

impl<'a, T> RecordTable<'a, T> {
    /// Create an empty table.
    const fn new() -> Self {
        Self {
            records: Vec::new(),
            adjusters: Vec::new(),
        }
    }

    /// Add a slot and return its position.
    fn reserve(&mut self) -> Result<u32> {
        let position = position(self.records.len(), "type stream has too many records")?;
        handle::record_index(position, "record index does not fit in 32 bits")?;
        self.records.try_reserve(1).map_err(|_| no_memory())?;
        self.records.push(None);
        Ok(position)
    }

    /// Set the record of the slot at `position`.
    fn define(&mut self, position: u32, entry: Entry<'a, T>) -> Result<()> {
        let slot = usize::try_from(position)
            .ok()
            .and_then(|position| self.records.get_mut(position))
            .ok_or_else(|| handle::unresolved("handle has no slot in the builder"))?;
        if slot.is_some() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("handle already has a record"));
        }
        *slot = Some(entry);
        Ok(())
    }

    /// Add a record and return its position.
    fn add(&mut self, entry: Entry<'a, T>) -> Result<u32> {
        let position = self.reserve()?;
        self.define(position, entry)?;
        Ok(position)
    }

    /// Add a hash adjuster.
    fn adjust(&mut self, position: u32, name_offset: NameOffset) -> Result<()> {
        if usize::try_from(position).map_or(true, |position| position >= self.records.len()) {
            return Err(handle::unresolved("hash adjuster handle has no record"));
        }
        self.adjusters.try_reserve(1).map_err(|_| no_memory())?;
        self.adjusters.push((position, name_offset));
        Ok(())
    }
}

/// A shared symbol record and the indexes that name it.
#[derive(Debug)]
struct SharedSymbol<'a> {
    /// The record.
    entry: SymbolEntry<'a>,
    /// The reference count in the global index, or `None` if the global
    /// index does not name the record.
    global: Option<u32>,
    /// `true` if the public index names the record.
    public: bool,
}

/// The DBI values that the plan does not compute.
#[derive(Debug)]
struct DbiParts {
    /// The header. The plan sets the age and the stream fields.
    header: DbiHeader,
    /// The layout of the section contribution substream.
    contribution_version: SectionContributionVersion,
    /// The section contributions.
    contributions: Vec<SectionContribution>,
    /// The section map.
    section_map: Option<SectionMap>,
    /// The type server map.
    type_servers: Vec<TypeServer>,
    /// The EC names.
    ec_names: Option<StringTableBuilder>,
    /// The number of slots of the optional debug header.
    debug_slots: usize,
}

/// A builder of a new native PDB.
///
/// The builder collects the records of a new PDB: the identity, the TPI and
/// IPI records, the modules with their symbols and C13 subsections, the
/// symbol record stream with the global and public indexes, the DBI
/// substreams, the optional debug streams, `/names` and other named streams.
/// [`PdbBuilder::plan`] encodes all streams with the encoders of each record
/// family and returns a [`PdbWritePlan`]. The plan writes the container.
///
/// # Handles
///
/// The builder gives a handle for each item: [`TypeHandle`], [`IdHandle`],
/// [`ModuleHandle`], [`ModuleSymbolHandle`], [`SymbolHandle`] and
/// [`StreamHandle`]. See [`crate::native::write`] for the two ways in which
/// a record refers to other items. [`PdbBuilder::reserve_type`] and
/// [`PdbBuilder::reserve_id`] give a handle before its record exists, for
/// example for a record that refers to a later record.
///
/// # Plan
///
/// [`PdbBuilder::plan`] does all work before output:
///
/// 1. It checks that each reserved handle has a record.
/// 2. It assigns the stream IDs: 0 to 4 for the previous stream directory,
///    Info, TPI, DBI and IPI. Then the TPI and IPI hash streams, the global
///    index, the public index, the symbol record stream, the module streams
///    in module order, the optional debug streams in slot order, the named
///    streams in the order of their addition and the streams without a name.
/// 3. It encodes the TPI and IPI records and checks their references with
///    [`validate_type_references`](crate::native::types::validate_type_references)
///    and [`IdTable::check`](crate::native::codeview::id_record::IdTable::check).
/// 4. It encodes the module symbols with scope links, then the symbol record
///    stream. A shared record can resolve the offset of a module record. It
///    checks the symbol references with
///    [`validate_module_symbols`](crate::native::codeview::symbol::validate::validate_module_symbols)
///    and
///    [`validate_shared_symbols`](crate::native::codeview::symbol::validate::validate_shared_symbols).
/// 5. It joins each module stream with its global references and checks the
///    file references of its C13 subsections.
/// 6. It builds the global and public indexes, the DBI stream, `/names`, the
///    named streams and the Info stream.
/// 7. It plans the MSF layout.
///
/// An unresolved handle gives [`ErrorKind::InvalidReference`]. A value that
/// does not fit its field gives [`ErrorKind::LimitExceeded`]. A planning
/// error writes nothing.
///
/// The same builder and the same [`BuildOptions`] give the same bytes. The
/// plan does not read a clock or random values: the caller gives the
/// identity. This is deterministic reconstruction, not exact copying: see
/// [`crate::io::copy_exact`] for exact copies.
///
/// # Identity
///
/// [`PdbBuilder::new`] takes the signature, the GUID and the age. The DBI
/// header gets the same age. [`PdbWritePlan::identity`] returns the identity
/// for the `RSDS` debug entry of the image. Updating the image is the work
/// of the caller.
///
/// # Semantic rebuild
///
/// [`PdbContents`] reads all streams of a native PDB.
/// [`PdbContents::to_builder`] decodes every record and gives the decoded
/// values to a new builder.
pub struct PdbBuilder<'a> {
    /// The Info stream without named streams: identity and features.
    info: InfoBuilder,
    /// The DBI age.
    dbi_age: u32,
    /// The TPI records.
    types: RecordTable<'a, TypeLeaf<'a>>,
    /// The IPI records.
    ids: RecordTable<'a, IdRecord<'a>>,
    /// The `/names` string table, or `None` for a PDB without `/names`.
    names: Option<StringTableBuilder>,
    /// The DBI values.
    dbi: DbiParts,
    /// The modules.
    modules: Vec<ModuleBuilder<'a>>,
    /// The records of the symbol record stream.
    symbols: Vec<SharedSymbol<'a>>,
    /// The thunk table of the public index.
    thunks: ThunkTable<'a>,
    /// The section starts of the public index.
    section_starts: Vec<SectionStart>,
    /// The optional debug streams by slot.
    debug_streams: Vec<(usize, Entry<'a, Vec<u8>>)>,
    /// The named streams other than `/names`.
    named: Vec<(&'a [u8], Entry<'a, Vec<u8>>)>,
    /// The streams without a name.
    streams: Vec<Entry<'a, Vec<u8>>>,
}

impl core::fmt::Debug for PdbBuilder<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PdbBuilder")
            .field("identity", &self.info.identity())
            .field("dbi_age", &self.dbi_age)
            .field("types", &self.types.records.len())
            .field("ids", &self.ids.records.len())
            .field("modules", &self.modules.len())
            .field("symbols", &self.symbols.len())
            .field("named_streams", &self.named.len())
            .finish_non_exhaustive()
    }
}

impl<'a> PdbBuilder<'a> {
    /// Create a builder with an explicit identity and no records.
    ///
    /// The Info stream and the DBI header get `age`. The builder has a
    /// `/names` table with the [`HashVersion::V1`] hash and the feature code
    /// [`Feature::VC140`], as Microsoft `link.exe` writes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn new(signature: u32, guid: [u8; 16], age: u32) -> Result<Self> {
        let mut info = InfoBuilder::new(Producer::Microsoft, signature, guid, age)?;
        info.set_features(&[Feature::VC140])?;
        Ok(Self {
            info,
            dbi_age: age,
            types: RecordTable::new(),
            ids: RecordTable::new(),
            names: Some(StringTableBuilder::new(HashVersion::V1)?),
            dbi: DbiParts {
                header: DbiHeader::default(),
                contribution_version: SectionContributionVersion::V60,
                contributions: Vec::new(),
                section_map: None,
                type_servers: Vec::new(),
                // Microsoft `link.exe` writes an empty EC name table, and LLVM
                // reads it for the name indices of the module records.
                ec_names: Some(StringTableBuilder::new(HashVersion::V1)?),
                debug_slots: 0,
            },
            modules: Vec::new(),
            symbols: Vec::new(),
            thunks: ThunkTable::default(),
            section_starts: Vec::new(),
            debug_streams: Vec::new(),
            named: Vec::new(),
            streams: Vec::new(),
        })
    }

    /// Return the Info stream values: the identity and the features.
    #[must_use]
    pub const fn info(&self) -> &InfoBuilder {
        &self.info
    }

    /// Return the DBI age.
    #[must_use]
    pub const fn dbi_age(&self) -> u32 {
        self.dbi_age
    }

    /// Set the DBI age.
    ///
    /// The age of a PDB and its image must match the DBI age. Use
    /// [`PdbBuilder::apply_identity`] to keep the Info age and the DBI age
    /// consistent.
    pub const fn set_dbi_age(&mut self, age: u32) {
        self.dbi_age = age;
    }

    /// Apply an identity policy to the Info stream and the DBI age.
    ///
    /// Return the identity for the `RSDS` debug entry of the image: the GUID
    /// and the DBI age.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`InfoBuilder::apply_identity`].
    pub fn apply_identity(&mut self, policy: IdentityPolicy) -> Result<Identity> {
        self.info.apply_identity(policy, &mut self.dbi_age)
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
        self.info.set_features(features)
    }

    /// Reserve a type handle. Set its record with [`PdbBuilder::define_type`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the type index does not fit
    /// in 32 bits or an allocation fails.
    pub fn reserve_type(&mut self) -> Result<TypeHandle> {
        self.types.reserve().map(TypeHandle)
    }

    /// Set the record of a reserved type handle.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the builder did not make `handle`.
    /// - [`ErrorKind::InvalidFormat`] if `handle` has a record already.
    pub fn define_type(&mut self, handle: TypeHandle, leaf: TypeLeaf<'a>) -> Result<()> {
        self.types.define(handle.0, Entry::Value(leaf))
    }

    /// Set the record of a reserved type handle to a deferred record.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::define_type`].
    pub fn define_type_with(
        &mut self,
        handle: TypeHandle,
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<()> {
        self.types
            .define(handle.0, Entry::Deferred(Box::new(encode)))
    }

    /// Add a type record with final type indices.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::reserve_type`].
    pub fn add_type(&mut self, leaf: TypeLeaf<'a>) -> Result<TypeHandle> {
        self.types.add(Entry::Value(leaf)).map(TypeHandle)
    }

    /// Add a type record that `encode` appends when the plan resolves handles.
    ///
    /// The resolver of a type record gives type indices and ID indices.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::reserve_type`].
    pub fn add_type_with(
        &mut self,
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<TypeHandle> {
        self.types
            .add(Entry::Deferred(Box::new(encode)))
            .map(TypeHandle)
    }

    /// Add a hash adjuster to the TPI hash stream: the UDT record `handle`
    /// is at the head of the hash chain of the name at `name_offset` in
    /// `/names`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the builder did not make `handle`.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn add_type_hash_adjuster(
        &mut self,
        handle: TypeHandle,
        name_offset: NameOffset,
    ) -> Result<()> {
        self.types.adjust(handle.0, name_offset)
    }

    /// Reserve an ID handle. Set its record with [`PdbBuilder::define_id`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the ID index does not fit in
    /// 32 bits or an allocation fails.
    pub fn reserve_id(&mut self) -> Result<IdHandle> {
        self.ids.reserve().map(IdHandle)
    }

    /// Set the record of a reserved ID handle.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the builder did not make `handle`.
    /// - [`ErrorKind::InvalidFormat`] if `handle` has a record already.
    pub fn define_id(&mut self, handle: IdHandle, record: IdRecord<'a>) -> Result<()> {
        self.ids.define(handle.0, Entry::Value(record))
    }

    /// Set the record of a reserved ID handle to a deferred record.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::define_id`].
    pub fn define_id_with(
        &mut self,
        handle: IdHandle,
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<()> {
        self.ids.define(handle.0, Entry::Deferred(Box::new(encode)))
    }

    /// Add an ID record with final indices.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::reserve_id`].
    pub fn add_id(&mut self, record: IdRecord<'a>) -> Result<IdHandle> {
        self.ids.add(Entry::Value(record)).map(IdHandle)
    }

    /// Add an ID record that `encode` appends when the plan resolves handles.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::reserve_id`].
    pub fn add_id_with(
        &mut self,
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<IdHandle> {
        self.ids
            .add(Entry::Deferred(Box::new(encode)))
            .map(IdHandle)
    }

    /// Add a hash adjuster to the IPI hash stream.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::add_type_hash_adjuster`].
    pub fn add_id_hash_adjuster(
        &mut self,
        handle: IdHandle,
        name_offset: NameOffset,
    ) -> Result<()> {
        self.ids.adjust(handle.0, name_offset)
    }

    /// Return the `/names` table, or `None` if the PDB has no `/names` stream.
    #[must_use]
    pub const fn names(&self) -> Option<&StringTableBuilder> {
        self.names.as_ref()
    }

    /// Add a name to `/names` and return its offset.
    ///
    /// File checksums, frame programs and other records refer to names by
    /// this offset. The offset of a name does not change when other names
    /// follow, so the caller can use it at once.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the builder has no `/names` table.
    /// - The errors of [`StringTableBuilder::insert`].
    pub fn add_name(&mut self, name: &[u8]) -> Result<NameOffset> {
        self.names
            .as_mut()
            .ok_or_else(|| {
                Error::new(ErrorKind::MissingStream, None)
                    .with_context("PDB builder has no /names table")
            })?
            .insert(name)
    }

    /// Replace the `/names` table. `None` writes no `/names` stream.
    pub fn set_names(&mut self, names: Option<StringTableBuilder>) {
        self.names = names;
    }

    /// Set the DBI header values.
    ///
    /// The plan replaces the age and the global, public and symbol record
    /// stream fields.
    pub const fn set_dbi_header(&mut self, header: DbiHeader) {
        self.dbi.header = header;
    }

    /// Set the section contributions and the layout of their substream.
    pub fn set_section_contributions(
        &mut self,
        version: SectionContributionVersion,
        contributions: Vec<SectionContribution>,
    ) {
        self.dbi.contribution_version = version;
        self.dbi.contributions = contributions;
    }

    /// Set the section map. `None` writes an empty substream.
    pub fn set_section_map(&mut self, map: Option<SectionMap>) {
        self.dbi.section_map = map;
    }

    /// Set the type server map.
    pub fn set_type_servers(&mut self, servers: Vec<TypeServer>) {
        self.dbi.type_servers = servers;
    }

    /// Set the EC name table. `None` writes an empty substream.
    pub fn set_ec_names(&mut self, names: Option<StringTableBuilder>) {
        self.dbi.ec_names = names;
    }

    /// Set the smallest number of slots in the DBI optional debug header.
    ///
    /// The header has a slot for each [`DebugStream`] with a stream, and
    /// at least `count` slots. Microsoft and LLVM writers write 11 slots.
    pub const fn set_debug_slot_count(&mut self, count: usize) {
        self.dbi.debug_slots = count;
    }

    /// Set the bytes of an optional debug stream.
    ///
    /// Use the encoder of the stream family, for example
    /// [`RecordStream::to_bytes`](crate::native::debug_stream::RecordStream::to_bytes).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn set_debug_stream(&mut self, kind: DebugStream, bytes: Vec<u8>) -> Result<()> {
        let slot = kind.index();
        self.debug_streams.retain(|(existing, _)| *existing != slot);
        self.debug_streams.try_reserve(1).map_err(|_| no_memory())?;
        self.debug_streams.push((slot, Entry::Value(bytes)));
        Ok(())
    }

    /// Add a module with the module record `record` and return it.
    ///
    /// The module has a module stream with an empty symbol region.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the module count does not fit
    /// in 32 bits or an allocation fails. The plan rejects more than 65535
    /// modules.
    pub fn add_module(&mut self, record: Module<'static>) -> Result<&mut ModuleBuilder<'a>> {
        let index = position(self.modules.len(), "PDB has too many modules")?;
        self.modules.try_reserve(1).map_err(|_| no_memory())?;
        self.modules.push(ModuleBuilder::new(index, record));
        Ok(self.modules.last_mut().unwrap_or_else(|| unreachable!()))
    }

    /// Return a module of the builder.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder did not make `handle`.
    pub fn module_mut(&mut self, handle: ModuleHandle) -> Result<&mut ModuleBuilder<'a>> {
        usize::try_from(handle.0)
            .ok()
            .and_then(|index| self.modules.get_mut(index))
            .ok_or_else(|| handle::unresolved("module handle has no module in the builder"))
    }

    /// Add a decoded record to the symbol record stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the record count does not fit
    /// in 32 bits or an allocation fails.
    pub fn add_symbol(&mut self, symbol: Symbol<'a>) -> Result<SymbolHandle> {
        self.push_symbol(SymbolEntry::Decoded(symbol))
    }

    /// Add a record of `kind` with `data` to the symbol record stream.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::add_symbol`].
    pub fn add_symbol_fields(
        &mut self,
        kind: SymbolKind,
        data: SymbolData<'a>,
    ) -> Result<SymbolHandle> {
        self.push_symbol(SymbolEntry::Fields(kind, data))
    }

    /// Add a record to the symbol record stream that `encode` appends when
    /// the plan resolves handles.
    ///
    /// The resolver of a shared record also gives module symbol offsets,
    /// for example for `S_PROCREF`. It does not give symbol offsets.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::add_symbol`].
    pub fn add_symbol_with(
        &mut self,
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<SymbolHandle> {
        self.push_symbol(SymbolEntry::Deferred(Box::new(encode)))
    }

    /// Add one shared symbol record.
    fn push_symbol(&mut self, entry: SymbolEntry<'a>) -> Result<SymbolHandle> {
        let index = position(
            self.symbols.len(),
            "symbol record stream has too many records",
        )?;
        self.symbols.try_reserve(1).map_err(|_| no_memory())?;
        self.symbols.push(SharedSymbol {
            entry,
            global: None,
            public: false,
        });
        Ok(SymbolHandle(index))
    }

    /// Return the shared record of `handle`.
    fn shared(&mut self, handle: SymbolHandle) -> Result<&mut SharedSymbol<'a>> {
        usize::try_from(handle.0)
            .ok()
            .and_then(|index| self.symbols.get_mut(index))
            .ok_or_else(|| handle::unresolved("symbol handle has no record in the builder"))
    }

    /// Put a shared record in the global symbol index.
    ///
    /// `reference_count` is the number of modules that use the record. LLVM
    /// writes 1.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder did not make `handle`.
    pub fn index_global(&mut self, handle: SymbolHandle, reference_count: u32) -> Result<()> {
        self.shared(handle)?.global = Some(reference_count);
        Ok(())
    }

    /// Put an `S_PUB32` record in the public symbol index.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the builder did not make `handle`.
    pub fn index_public(&mut self, handle: SymbolHandle) -> Result<()> {
        self.shared(handle)?.public = true;
        Ok(())
    }

    /// Set the incremental thunk table of the public symbol index.
    pub const fn set_public_thunks(&mut self, thunks: ThunkTable<'a>) {
        self.thunks = thunks;
    }

    /// Set the section start table of the public symbol index.
    pub fn set_section_starts(&mut self, starts: Vec<SectionStart>) {
        self.section_starts = starts;
    }

    /// Add a named stream with `bytes`.
    ///
    /// The plan adds `/names` itself. Use the encoder of the stream family,
    /// for example [`SourceLink::encode`](crate::native::auxiliary::SourceLink::encode).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the name is `/names`, empty or
    ///   already in the builder.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn add_named_stream(&mut self, name: &'a [u8], bytes: Vec<u8>) -> Result<()> {
        self.push_named(name, Entry::Value(bytes))
    }

    /// Add a named stream that `encode` appends when the plan resolves handles.
    ///
    /// The resolver gives all values, for example the stream IDs that a
    /// `/TMCache` stream names.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PdbBuilder::add_named_stream`].
    pub fn add_named_stream_with(
        &mut self,
        name: &'a [u8],
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<()> {
        self.push_named(name, Entry::Deferred(Box::new(encode)))
    }

    /// Add one named stream.
    fn push_named(&mut self, name: &'a [u8], entry: Entry<'a, Vec<u8>>) -> Result<()> {
        if name.is_empty() || name == b"/names" || self.named.iter().any(|(n, _)| *n == name) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("named stream name is empty, /names or already present"));
        }
        self.named.try_reserve(1).map_err(|_| no_memory())?;
        self.named.push((name, entry));
        Ok(())
    }

    /// Add a stream without a name and return its handle.
    ///
    /// Another stream refers to it by its stream ID, for example a type map
    /// stream that `/TMCache` names.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the stream count does not
    /// fit in 32 bits or an allocation fails.
    pub fn add_stream(&mut self, bytes: Vec<u8>) -> Result<StreamHandle> {
        let index = position(self.streams.len(), "PDB has too many streams")?;
        self.streams.try_reserve(1).map_err(|_| no_memory())?;
        self.streams.push(Entry::Value(bytes));
        Ok(StreamHandle(index))
    }
}
