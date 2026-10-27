//! The cached navigation structures of the reader session.
//!
//! The first call of an accessor loads and validates its structure and stores it in a cell.
//! Later calls lend the stored value. [`super::ReadOptions::max_cached_bytes`] limits the
//! stream bytes that the cells hold.

use alloc::vec::Vec;

use super::cache::{Cache, Cell};
use super::{Pdb, require_stream};
use crate::io::ReadAt;
use crate::native::codeview::index::ModuleIndex;
use crate::native::dbi::{Dbi, Module, ModuleStream};
use crate::native::msf::StreamId;
use crate::native::names::{StringTable, StringTableLayout};
use crate::native::output::no_memory;
use crate::native::symbols::{GlobalSymbolIndex, Globals, PublicSymbolIndex, Publics};
use crate::native::types::{HEADER_SIZE, TypeOffsets, parse_header_prefix};
use crate::{Error, ErrorKind, Result};

/// The DBI stream and the position of each module record in the module info substream.
#[derive(Debug)]
struct CachedDbi {
    /// The validated DBI stream.
    dbi: Dbi,
    /// The position of each module record, in module index order.
    modules: Vec<usize>,
}

/// The `/names` stream and its validated layout.
#[derive(Debug)]
struct CachedNames {
    /// The stream bytes.
    bytes: Vec<u8>,
    /// The validated layout of `bytes`.
    layout: StringTableLayout,
}

/// The cells of the reader session. A cell holds `None` for an absent structure.
#[derive(Debug)]
pub(super) struct Session {
    /// The byte limit and the cached bytes.
    cache: Cache,
    /// The DBI stream.
    dbi: Cell<Option<CachedDbi>>,
    /// The `/names` stream.
    names: Cell<Option<CachedNames>>,
    /// The symbol record stream.
    symbol_records: Cell<Option<Vec<u8>>>,
    /// The global symbol index.
    globals: Cell<Option<GlobalSymbolIndex>>,
    /// The public symbol index.
    publics: Cell<Option<PublicSymbolIndex>>,
    /// The TPI header and index offsets.
    types: Cell<Option<TypeOffsets>>,
    /// The IPI header and index offsets.
    ids: Cell<Option<TypeOffsets>>,
}

impl Session {
    /// Make a session without cached structures and with a limit of `limit` bytes.
    pub(super) fn new(limit: usize) -> Self {
        Self {
            cache: Cache::new(limit),
            dbi: Cell::new(),
            names: Cell::new(),
            symbol_records: Cell::new(),
            globals: Cell::new(),
            publics: Cell::new(),
            types: Cell::new(),
            ids: Cell::new(),
        }
    }
}

/// Return the value and the size of a structure that the PDB does not have.
const fn absent<T>() -> (Option<T>, usize) {
    (None, 0)
}

impl<S: ReadAt> Pdb<S> {
    /// Return the DBI stream, if present. The session loads it once.
    ///
    /// The first call checks the DBI age and the referenced stream IDs. The DBI age must not
    /// be larger than the PDB Info age. Writers that change only named streams, for example
    /// `pdbstr -w`, increment the Info age and keep the DBI age. Image matching uses the DBI
    /// age.
    ///
    /// # Errors
    ///
    /// Returns errors for I/O, limits, malformed DBI, a DBI age that is larger than the Info
    /// age or invalid references. Module payloads are checked only when [`Pdb::module`] loads
    /// them. Returns [`ErrorKind::LimitExceeded`] if the stream does not fit in
    /// [`super::ReadOptions::max_cached_bytes`].
    pub fn debug_info(&self) -> Result<Option<&Dbi>> {
        Ok(self.cached_dbi()?.map(|cached| &cached.dbi))
    }

    /// Return the cached DBI stream and its module positions.
    fn cached_dbi(&self) -> Result<Option<&CachedDbi>> {
        self.session
            .cache
            .get_or_load(&self.session.dbi, || self.load_dbi())
            .map(Option::as_ref)
    }

    /// Load and check the DBI stream.
    fn load_dbi(&self) -> Result<(Option<CachedDbi>, usize)> {
        let Some(stream) = self.optional_stream(StreamId(3))? else {
            return Ok(absent());
        };
        let bytes = stream.read_all(self.options.max_stream_bytes)?;
        let size = bytes.len();
        let dbi = Dbi::parse(bytes)?;
        if dbi.header().age > self.info.identity().age() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("DBI age is larger than the PDB Info age")
                .with_stream(StreamId(3), 8));
        }
        let header = dbi.header();
        for reference in [
            header.global_stream,
            header.public_stream,
            header.symbol_stream,
        ]
        .into_iter()
        .flatten()
        {
            require_stream(&self.container, reference)?;
        }
        for reference in dbi.debug_streams().flatten() {
            require_stream(&self.container, reference)?;
        }
        let mut modules = Vec::new();
        let mut records = dbi.modules();
        loop {
            let position = records.position();
            let Some(module) = records.next() else {
                break;
            };
            if let Some(reference) = module?.1.stream {
                require_stream(&self.container, reference)?;
            }
            modules.try_reserve(1).map_err(|_| no_memory())?;
            modules.push(position);
        }
        Ok((Some(CachedDbi { dbi, modules }), size))
    }

    /// Return the `/names` string table, if the Info stream names it. The session loads it
    /// once.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::named_stream_bytes`] and [`StringTable::parse`], and
    /// [`ErrorKind::LimitExceeded`] if the stream does not fit in
    /// [`super::ReadOptions::max_cached_bytes`].
    pub fn names(&self) -> Result<Option<StringTable<'_>>> {
        let cached = self.session.cache.get_or_load(&self.session.names, || {
            let Some(bytes) = self.named_stream_bytes(b"/names")? else {
                return Ok(absent());
            };
            let layout = StringTableLayout::parse(&bytes)?;
            let size = bytes.len();
            Ok((Some(CachedNames { bytes, layout }), size))
        })?;
        Ok(cached.as_ref().map(|names| names.layout.view(&names.bytes)))
    }

    /// Return the symbol record stream that the DBI header names, if any. The session loads
    /// it once.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::debug_info`] and [`Pdb::stream_bytes`], and
    /// [`ErrorKind::LimitExceeded`] if the stream does not fit in
    /// [`super::ReadOptions::max_cached_bytes`].
    pub fn symbol_records(&self) -> Result<Option<&[u8]>> {
        let cached = self
            .session
            .cache
            .get_or_load(&self.session.symbol_records, || {
                let Some(id) = self
                    .debug_info()?
                    .and_then(|dbi| dbi.header().symbol_stream)
                else {
                    return Ok(absent());
                };
                let bytes = self.stream_bytes(id)?;
                let size = bytes.len();
                Ok((Some(bytes), size))
            })?;
        Ok(cached.as_deref())
    }

    /// Return the global symbol index that the DBI header names, with the symbol record
    /// stream. The session loads both once.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`], [`GlobalSymbolIndex::parse`] and
    /// [`Pdb::symbol_records`], and [`ErrorKind::LimitExceeded`] if the stream does not fit
    /// in [`super::ReadOptions::max_cached_bytes`].
    pub fn globals(&self) -> Result<Option<Globals<'_>>> {
        let index = self.session.cache.get_or_load(&self.session.globals, || {
            let Some(id) = self
                .debug_info()?
                .and_then(|dbi| dbi.header().global_stream)
            else {
                return Ok(absent());
            };
            let bytes = self.stream_bytes(id)?;
            let size = bytes.len();
            let index = GlobalSymbolIndex::parse(bytes, id, self.symbol_bucket_count())?;
            Ok((Some(index), size))
        })?;
        let Some(index) = index else {
            return Ok(None);
        };
        let records = self.symbol_records()?.unwrap_or_default();
        Ok(Some(Globals::new(index, records)))
    }

    /// Return the public symbol index that the DBI header names, with the symbol record
    /// stream. The session loads both once.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`], [`PublicSymbolIndex::parse`] and
    /// [`Pdb::symbol_records`], and [`ErrorKind::LimitExceeded`] if the stream does not fit
    /// in [`super::ReadOptions::max_cached_bytes`].
    pub fn publics(&self) -> Result<Option<Publics<'_>>> {
        let index = self.session.cache.get_or_load(&self.session.publics, || {
            let Some(id) = self
                .debug_info()?
                .and_then(|dbi| dbi.header().public_stream)
            else {
                return Ok(absent());
            };
            let bytes = self.stream_bytes(id)?;
            let size = bytes.len();
            let index = PublicSymbolIndex::parse(bytes, id, self.symbol_bucket_count())?;
            Ok((Some(index), size))
        })?;
        let Some(index) = index else {
            return Ok(None);
        };
        let records = self.symbol_records()?.unwrap_or_default();
        Ok(Some(Publics::new(index, records)))
    }

    /// Return the TPI header and the index offset pairs, if the TPI stream is present. The
    /// session loads them once.
    ///
    /// The function reads the header and the index offset buffer of the hash stream. It does
    /// not read the records; [`Pdb::types`] does that.
    ///
    /// # Errors
    ///
    /// Returns the header errors of [`crate::native::types::TypeStream::parse`], the index
    /// offset errors of [`crate::native::types::TypeHashes::parse`], reference and I/O
    /// errors, and [`ErrorKind::LimitExceeded`] if the parts do not fit in
    /// [`super::ReadOptions::max_cached_bytes`].
    pub fn type_offsets(&self) -> Result<Option<&TypeOffsets>> {
        self.session
            .cache
            .get_or_load(&self.session.types, || self.load_type_offsets(StreamId(2)))
            .map(Option::as_ref)
    }

    /// Return the IPI header and the index offset pairs, if the IPI stream is present. The
    /// session loads them once.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::type_offsets`].
    pub fn id_offsets(&self) -> Result<Option<&TypeOffsets>> {
        self.session
            .cache
            .get_or_load(&self.session.ids, || self.load_type_offsets(StreamId(4)))
            .map(Option::as_ref)
    }

    /// Read the header of type stream `id` and the index offset buffer of its hash stream.
    fn load_type_offsets(&self, id: StreamId) -> Result<(Option<TypeOffsets>, usize)> {
        let Some(stream) = self.optional_stream(id)? else {
            return Ok(absent());
        };
        let length = stream_length(&stream)?;
        let mut head = [0; HEADER_SIZE];
        let head = &mut head[..length.min(HEADER_SIZE)];
        stream.read_exact_at(0, head)?;
        let header = parse_header_prefix(head, length).map_err(|error| error.with_stream(id, 0))?;
        for reference in [header.hash_stream, header.hash_aux_stream]
            .into_iter()
            .flatten()
        {
            require_stream(&self.container, reference)?;
        }
        let mut buffer = Vec::new();
        if let Some(hash) = header.hash_stream
            && header.index_offsets.length != 0
        {
            let hash_stream = require_stream(&self.container, hash)?;
            let hash_length = hash_stream.len()?;
            // `parse_header_prefix` made sure that the offset is not negative.
            let start = u64::from(header.index_offsets.offset.unsigned_abs());
            let end = start + u64::from(header.index_offsets.length);
            if end > hash_length {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("type hash buffer is outside the hash stream")
                    .with_stream(hash, hash_length));
            }
            let size = usize::try_from(header.index_offsets.length).map_err(|_| no_memory())?;
            buffer.try_reserve_exact(size).map_err(|_| no_memory())?;
            buffer.resize(size, 0);
            hash_stream.read_exact_at(start, &mut buffer)?;
        }
        let size = HEADER_SIZE + buffer.len();
        Ok((Some(TypeOffsets::from_parts(header, &buffer)?), size))
    }

    /// Load the symbols and line-data regions of the module with `index`.
    ///
    /// The session keeps the position of each module record, so the function decodes only
    /// one module record. The module stream is not cached.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no DBI stream.
    /// - [`ErrorKind::InvalidReference`] if `index` is outside the DBI module list.
    /// - The errors of [`Pdb::debug_info`] and [`ModuleStream::parse`], exceeded limits and
    ///   failed I/O.
    pub fn module(&self, index: ModuleIndex) -> Result<Option<ModuleStream>> {
        let cached = self
            .cached_dbi()?
            .ok_or_else(|| Error::new(ErrorKind::MissingStream, None).with_context("DBI"))?;
        let outside = || {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("module index is outside the DBI module list")
        };
        let position = usize::try_from(index.0)
            .ok()
            .and_then(|position| cached.modules.get(position))
            .ok_or_else(outside)?;
        let (_, module) = cached
            .dbi
            .modules_at(*position, index.0)
            .next()
            .ok_or_else(outside)??;
        self.load_module(&module)
    }

    /// Load the module stream of `module`, a module record of the cached DBI stream.
    pub(crate) fn load_module(&self, module: &Module<'_>) -> Result<Option<ModuleStream>> {
        let Some(id) = module.stream else {
            return Ok(None);
        };
        let stream = require_stream(&self.container, id)?;
        ModuleStream::parse(stream.read_all(self.options.max_stream_bytes)?, module).map(Some)
    }
}

/// Return the length of `stream` as a `usize`.
fn stream_length(stream: &impl ReadAt) -> Result<usize> {
    usize::try_from(stream.len()?).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("type stream does not fit in memory")
    })
}
