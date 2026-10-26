//! The write plan of a [`PdbBuilder`]: stream IDs, encoding and layout.

use alloc::vec::Vec;

use super::PdbBuilder;
use super::handle::Resolver;
use crate::io::{ReadAt, Write};
use crate::native::Identity;
use crate::native::Producer;
use crate::native::info::PeMatch;
use crate::native::msf::{MsfBuilder, StreamId, WriteOptions, WritePlan};
use crate::native::output::no_memory;
use crate::native::types::DEFAULT_HASH_BUCKETS;
use crate::{Error, ErrorKind, Result};

mod module_types;
mod streams;
mod symbols;
mod types;

/// The options of a write plan.
///
/// [`Default`] gives the conventions of Microsoft `link.exe`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BuildOptions {
    /// The container options: page size and limits.
    pub msf: WriteOptions,
    /// The producer whose layout of the TPI and IPI hash streams the writer
    /// follows (see [`crate::native::types::TypeStreamOptions::with_producer`]).
    pub hash_layout: Producer,
    /// The hash bucket count of the TPI and IPI streams.
    pub hash_buckets: u32,
    /// The producer whose conventions of the global and public symbol indexes
    /// the writer follows (see
    /// [`crate::native::symbols::PublicSymbolIndex::encode_parts`]).
    pub index_producer: Producer,
    /// The producer whose conventions of the named-stream map of the Info
    /// stream the writer follows (see
    /// [`crate::native::hash_table::OwnedHashTable`]).
    pub info_producer: Producer,
    /// `true` to write the TPI hash stream. Microsoft and LLVM linkers write it.
    pub type_hash_stream: bool,
    /// `true` to write the IPI hash stream.
    pub id_hash_stream: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            msf: WriteOptions::default(),
            hash_layout: Producer::Microsoft,
            hash_buckets: DEFAULT_HASH_BUCKETS,
            index_producer: Producer::Microsoft,
            info_producer: Producer::Microsoft,
            type_hash_stream: true,
            id_hash_stream: true,
        }
    }
}

/// A frozen native PDB: all stream bytes and the checked container layout.
///
/// [`PdbBuilder::plan`] makes the plan. The plan encoded all streams and
/// checked the MSF layout, so the write functions do only output work. The
/// plan borrows the stream bytes that the builder holds, so the builder
/// cannot change while the plan exists.
///
/// # Errors and partial output
///
/// [`PdbWritePlan::write_to`] writes in file order. An output error can
/// leave a partial output. Under `std`, `PdbWritePlan::write_file` writes
/// a temporary file and renames it; on error the destination does not change.
#[derive(Clone, Debug)]
pub struct PdbWritePlan<'b> {
    /// The streams in stream ID order. `None` is a nil stream.
    streams: MsfBuilder<StreamBytes<'b>>,
    /// The container options.
    options: WriteOptions,
    /// The values for the `RSDS` debug entry of the image.
    pe_match: PeMatch,
    /// The size of the output in bytes.
    file_size: u64,
}

impl PdbWritePlan<'_> {
    /// Return the values for the `RSDS` debug entry of the matching image.
    #[must_use]
    pub const fn pe_match(&self) -> PeMatch {
        self.pe_match
    }

    /// Return the number of streams.
    #[must_use]
    pub fn stream_count(&self) -> usize {
        self.streams.stream_count()
    }

    /// Return the size of the output in bytes.
    #[must_use]
    pub const fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Return the MSF plan of the streams.
    fn msf(&self) -> Result<WritePlan<'_>> {
        self.streams.plan(self.options)
    }

    /// Write the PDB to `output` in page order.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`WritePlan::write_to`].
    pub fn write_to<W: Write + ?Sized>(&self, output: &mut W) -> Result<()> {
        self.msf()?.write_to(output)
    }

    /// Write the PDB to a new vector.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`WritePlan::to_vec`].
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        self.msf()?.to_vec()
    }

    /// Write the PDB to the file `path` through a temporary sibling file.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`WritePlan::write_file`].
    #[cfg(feature = "std")]
    pub fn write_file(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        self.msf()?.write_file(path)
    }
}

/// The bytes of one planned stream: owned by the plan or borrowed from the builder.
#[derive(Clone, Debug)]
pub(super) enum StreamBytes<'b> {
    /// Bytes that the plan encoded.
    Owned(Vec<u8>),
    /// Bytes that the builder holds.
    Borrowed(&'b [u8]),
}

impl StreamBytes<'_> {
    /// Return the bytes.
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Borrowed(bytes) => bytes,
        }
    }
}

impl ReadAt for StreamBytes<'_> {
    fn len(&self) -> Result<u64> {
        ReadAt::len(self.bytes())
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        self.bytes().read_exact_at(offset, buffer)
    }
}

/// The stream IDs that the plan assigns.
#[derive(Clone, Debug)]
pub(super) struct StreamIds {
    /// The TPI hash stream, if the options select one.
    pub(super) tpi_hash: Option<StreamId>,
    /// The IPI hash stream, if the options select one.
    pub(super) ipi_hash: Option<StreamId>,
    /// The global symbol index.
    pub(super) globals: StreamId,
    /// The public symbol index.
    pub(super) publics: StreamId,
    /// The symbol record stream.
    pub(super) symbols: StreamId,
    /// The module stream of each module, or `None`.
    pub(super) modules: Vec<Option<StreamId>>,
    /// The stream of each optional debug stream, in builder order.
    pub(super) debug: Vec<StreamId>,
    /// The `/names` stream, if present.
    pub(super) names: Option<StreamId>,
    /// The stream of each named stream, in builder order.
    pub(super) named: Vec<StreamId>,
    /// The stream of each stream without a name.
    pub(super) unnamed: Vec<StreamId>,
    /// The number of streams.
    pub(super) count: u32,
}

/// A counter that gives the next stream ID.
struct Counter(u32);

impl Counter {
    /// Return the next stream ID.
    fn next(&mut self) -> Result<StreamId> {
        let id = StreamId(self.0);
        self.0 = self.0.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many streams")
        })?;
        Ok(id)
    }

    /// Return `count` next stream IDs.
    fn take(&mut self, count: usize) -> Result<Vec<StreamId>> {
        let mut ids = Vec::new();
        ids.try_reserve_exact(count).map_err(|_| no_memory())?;
        for _ in 0..count {
            ids.push(self.next()?);
        }
        Ok(ids)
    }
}

impl PdbBuilder<'_> {
    /// Assign the stream IDs in the order of the module documentation.
    fn assign_streams(&self, options: &BuildOptions) -> Result<StreamIds> {
        // Streams 0 to 4: previous directory, Info, TPI, DBI and IPI.
        let mut counter = Counter(5);
        let tpi_hash = options
            .type_hash_stream
            .then(|| counter.next())
            .transpose()?;
        let ipi_hash = options.id_hash_stream.then(|| counter.next()).transpose()?;
        let globals = counter.next()?;
        let publics = counter.next()?;
        let symbols = counter.next()?;
        let mut modules = Vec::new();
        modules
            .try_reserve_exact(self.modules.len())
            .map_err(|_| no_memory())?;
        for module in &self.modules {
            modules.push(if module.has_stream {
                Some(counter.next()?)
            } else {
                None
            });
        }
        let debug = counter.take(self.debug_streams.len())?;
        let names_stream = self.names.as_ref().map(|_| counter.next()).transpose()?;
        let named = counter.take(self.named.len())?;
        let unnamed = counter.take(self.streams.len())?;
        Ok(StreamIds {
            tpi_hash,
            ipi_hash,
            globals,
            publics,
            symbols,
            modules,
            debug,
            names: names_stream,
            named,
            unnamed,
            count: counter.0,
        })
    }

    /// Return a resolver for the TPI, IPI and stream handles.
    fn base_resolver<'p>(&self, ids: &'p StreamIds) -> Result<Resolver<'p>> {
        let count = |length: usize| {
            u32::try_from(length).map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None).with_context("builder item count")
            })
        };
        Ok(Resolver {
            type_count: count(self.types.records.len())?,
            id_count: count(self.ids.records.len())?,
            module_count: count(self.modules.len())?,
            streams: &ids.unnamed,
            module_offsets: None,
            symbol_offsets: None,
        })
    }

    /// Encode all streams, check all references and plan the container.
    ///
    /// See the [module documentation](super) for the steps.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] for a reserved handle without a
    ///   record, a handle that does not resolve, or a reference that the
    ///   checks reject.
    /// - [`ErrorKind::LimitExceeded`] if a value does not fit its field, a
    ///   stream is too large or an allocation fails.
    /// - [`ErrorKind::InvalidFormat`] if a record or a stream breaks a rule
    ///   of its encoder.
    /// - The errors of [`MsfBuilder::plan`].
    pub fn plan(&self, options: &BuildOptions) -> Result<PdbWritePlan<'_>> {
        let Identity::Modern { guid, .. } = self.info.identity() else {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("PDB Info stream has no GUID identity"));
        };
        let ids = self.assign_streams(options)?;
        let resolver = self.base_resolver(&ids)?;
        let tpi = self.type_stream(&resolver, options, ids.tpi_hash)?;
        let ipi = self.id_stream(&resolver, options, ids.ipi_hash, &tpi)?;
        let encoded = self.encode_symbols(&resolver, &ids, &tpi, &ipi, options)?;
        let mut records = Vec::new();
        let mut module_streams = Vec::new();
        records
            .try_reserve_exact(encoded.modules.len())
            .map_err(|_| no_memory())?;
        module_streams
            .try_reserve_exact(encoded.modules.len())
            .map_err(|_| no_memory())?;
        for module in encoded.modules {
            records.push(module.record);
            module_streams.extend(module.bytes);
        }
        let dbi = self.dbi_stream(&ids, records)?;
        let info = self.info_stream(&ids, options)?;
        let mut msf = MsfBuilder::new();
        // Stream 0 is the previous stream directory. A new PDB has none.
        for stream in [Vec::new(), info, tpi.stream, dbi, ipi.stream] {
            msf.push(StreamBytes::Owned(stream))?;
        }
        for stream in [tpi.hash, ipi.hash].into_iter().flatten() {
            msf.push(StreamBytes::Owned(stream))?;
        }
        for stream in [encoded.globals, encoded.publics, encoded.symbols] {
            msf.push(StreamBytes::Owned(stream))?;
        }
        for stream in module_streams {
            msf.push(StreamBytes::Owned(stream))?;
        }
        let full = Resolver {
            module_offsets: Some(&encoded.module_offsets),
            symbol_offsets: Some(&encoded.symbol_offsets),
            ..resolver
        };
        self.push_other_streams(&full, &mut msf)?;
        if msf.stream_count() != usize::try_from(ids.count).unwrap_or(usize::MAX) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("planned stream count differs from the assigned stream IDs"));
        }
        let file_size = msf.plan(options.msf)?.file_size();
        Ok(PdbWritePlan {
            streams: msf,
            options: options.msf,
            pe_match: PeMatch {
                guid,
                age: self.dbi_age,
            },
            file_size,
        })
    }
}
