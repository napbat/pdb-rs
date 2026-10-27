//! Native PDB stream discovery, the reader session and bounded typed loading.
//!
//! [`Pdb`] reads streams through a [`Container`], so all accessors work on MSF 7.00 and MSFZ
//! input.
//!
//! The reader is a session. It keeps the small navigation structures in once-cells and lends
//! them from `&self`: the DBI stream, `/names`, the global and public symbol indexes, the
//! symbol record stream, and the TPI and IPI headers with their index offsets. Bulk data stays
//! an owned load that the caller controls: the module streams, the TPI and IPI records and the
//! other debug streams.

use alloc::vec::Vec;

use super::container::{Container, ContainerLimits, Stream};
use super::info::Info;
use super::msf::{Msf, MsfWriteOptions, StreamId};
use super::symbols::BucketCount;
use super::types::TypeStream;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

mod cache;
mod debug;
mod session;

use session::Session;

/// Resource limits for native container metadata, explicitly loaded streams and the session
/// cache.
///
/// The default uses [`ContainerLimits::default`], a stream limit of 64 MiB and a cache limit
/// of 256 MiB.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ReadOptions {
    limits: ContainerLimits,
    max_stream_bytes: usize,
    max_cached_bytes: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            limits: ContainerLimits::default(),
            max_stream_bytes: 64 * 1024 * 1024,
            max_cached_bytes: 256 * 1024 * 1024,
        }
    }
}

impl ReadOptions {
    /// Return the limits for the MSF 7.00 and MSFZ container metadata.
    #[must_use]
    pub const fn limits(self) -> ContainerLimits {
        self.limits
    }

    /// Set the limits for the MSF 7.00 and MSFZ container metadata.
    #[must_use]
    pub const fn with_limits(mut self, limits: ContainerLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Return the maximum bytes in any stream that a typed accessor loads.
    #[must_use]
    pub const fn max_stream_bytes(self) -> usize {
        self.max_stream_bytes
    }

    /// Set the maximum bytes in any stream that a typed accessor loads.
    #[must_use]
    pub const fn with_max_stream_bytes(mut self, max_stream_bytes: usize) -> Self {
        self.max_stream_bytes = max_stream_bytes;
        self
    }

    /// Return the maximum stream bytes that the session cache of a [`Pdb`] holds.
    ///
    /// The limit counts the bytes of the DBI, `/names`, global symbol index, public symbol
    /// index and symbol record streams, and the header and the index offset buffer of the TPI
    /// and IPI streams. An accessor that goes above the limit returns
    /// [`ErrorKind::LimitExceeded`] and caches nothing. The cache does not evict.
    #[must_use]
    pub const fn max_cached_bytes(self) -> usize {
        self.max_cached_bytes
    }

    /// Set the maximum stream bytes that the session cache of a [`Pdb`] holds.
    #[must_use]
    pub const fn with_max_cached_bytes(mut self, max_cached_bytes: usize) -> Self {
        self.max_cached_bytes = max_cached_bytes;
        self
    }
}

/// A native PDB with a validated stream directory, a decoded Info stream and a session cache.
///
/// The container is MSF 7.00 or MSFZ. Opening does not decode all streams. The first call of
/// a cached accessor ([`Pdb::debug_info`], [`Pdb::names`], [`Pdb::symbol_records`],
/// [`Pdb::globals`], [`Pdb::publics`], [`Pdb::type_offsets`], [`Pdb::id_offsets`]) loads and
/// validates its structure. Later calls lend the same value. The other accessors load and
/// validate the requested stream on each call. Raw access remains available for data without
/// a semantic decoder.
///
/// With the `std` feature, `Pdb<S>` is `Sync` if `S` is `Sync`: the cells are `OnceLock`
/// values. Without `std`, the cells are `OnceCell` values and the MSFZ chunk cache uses a
/// `RefCell`, so the value is not `Sync`.
#[derive(Debug)]
pub struct Pdb<S> {
    container: Container<S>,
    info: Info,
    options: ReadOptions,
    session: Session,
}

impl<S: ReadAt> Pdb<S> {
    /// Read a stable positional source with default resource limits.
    ///
    /// # Errors
    ///
    /// Returns container, Info-stream, reference, allocation or I/O errors.
    pub fn read(source: S) -> Result<Self> {
        Self::with_options(source, ReadOptions::default())
    }

    /// Read a source with explicit resource limits.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed or unsupported input, missing Info,
    /// invalid named-stream references, exceeded limits or failed I/O.
    pub fn with_options(source: S, options: ReadOptions) -> Result<Self> {
        let container = Container::with_limits(source, options.limits)?;
        if container.stream_count() <= 1 {
            return Err(Error::new(ErrorKind::MissingStream, None).with_context("PDB Info"));
        }
        let stream = container
            .stream(StreamId(1))?
            .ok_or_else(|| Error::new(ErrorKind::MissingStream, None).with_context("PDB Info"))?;
        let info = Info::parse(stream.read_all(options.max_stream_bytes)?)?;
        for named in info.named_streams() {
            require_stream(&container, named.stream)?;
        }
        Ok(Self {
            container,
            info,
            options,
            session: Session::new(options.max_cached_bytes),
        })
    }

    /// Return the stream container (MSF 7.00 or MSFZ) for raw stream access.
    #[must_use]
    pub const fn container(&self) -> &Container<S> {
        &self.container
    }

    /// Return the page size for MSF 7.00 output from this PDB: the page size of an MSF 7.00
    /// source, or the default page size of [`MsfWriteOptions`] for an MSFZ source.
    pub(crate) fn output_page_size(&self) -> u32 {
        self.container
            .msf()
            .map_or(MsfWriteOptions::default().page_size(), Msf::block_size)
    }

    /// Return decoded identity, named streams and feature codes.
    #[must_use]
    pub const fn info(&self) -> &Info {
        &self.info
    }

    /// Find a named stream using its exact name bytes.
    ///
    /// # Errors
    ///
    /// Returns a directory error if a referenced stream cannot be accessed.
    pub fn named_stream(&self, name: &[u8]) -> Result<Option<Stream<'_, S>>> {
        self.info
            .named_stream(name)
            .map_or(Ok(None), |id| self.container.stream(id))
    }

    /// Load all bytes of a present stream within the configured stream limit.
    ///
    /// Use this function for decoders that borrow their input, for example
    /// the `/names` string table or the section-header stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidStreamIndex`] for an index outside the
    /// directory, [`ErrorKind::InvalidReference`] for a nil stream, and
    /// limit, allocation or I/O errors.
    pub fn stream_bytes(&self, id: StreamId) -> Result<Vec<u8>> {
        require_stream(&self.container, id)?.read_all(self.options.max_stream_bytes)
    }

    /// Load all bytes of a named stream, if the Info stream names it.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`].
    pub fn named_stream_bytes(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        self.info
            .named_stream(name)
            .map(|id| self.stream_bytes(id))
            .transpose()
    }

    /// Return the bucket count of the global and public symbol indexes.
    ///
    /// The Info stream feature codes select it.
    #[must_use]
    pub fn symbol_bucket_count(&self) -> BucketCount {
        BucketCount::from_features(self.info.features())
    }

    /// Load the TPI stream, if present.
    ///
    /// # Errors
    ///
    /// Returns I/O, limit, unsupported-version, framing or reference errors.
    pub fn types(&self) -> Result<Option<TypeStream>> {
        self.type_stream(StreamId(2))
    }

    /// Load the IPI stream, if present.
    ///
    /// # Errors
    ///
    /// Returns I/O, limit, unsupported-version, framing or reference errors.
    pub fn ids(&self) -> Result<Option<TypeStream>> {
        self.type_stream(StreamId(4))
    }

    fn type_stream(&self, id: StreamId) -> Result<Option<TypeStream>> {
        let Some(stream) = self.optional_stream(id)? else {
            return Ok(None);
        };
        let types = TypeStream::parse(stream.read_all(self.options.max_stream_bytes)?)
            .map_err(|error| error.with_stream(id, 0))?;
        for reference in [types.header().hash_stream, types.header().hash_aux_stream]
            .into_iter()
            .flatten()
        {
            require_stream(&self.container, reference)?;
        }
        Ok(Some(types))
    }

    fn optional_stream(&self, id: StreamId) -> Result<Option<Stream<'_, S>>> {
        if id.0 >= self.container.stream_count() {
            return Ok(None);
        }
        self.container.stream(id)
    }
}

impl<'a> Pdb<&'a [u8]> {
    /// Parse a native PDB from borrowed bytes.
    ///
    /// # Errors
    ///
    /// Returns the same container, Info and reference errors as [`Pdb::read`].
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::read(bytes)
    }
}

#[cfg(all(feature = "std", any(unix, windows)))]
impl Pdb<crate::io::FileReader> {
    /// Open a native PDB without loading all file bytes into memory.
    ///
    /// Requires the `std` feature and Unix or Windows.
    ///
    /// # Errors
    ///
    /// Returns file I/O, container, Info-stream or reference errors.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::read(crate::io::FileReader::open(path)?)
    }
}

fn require_stream<S: ReadAt>(container: &Container<S>, id: StreamId) -> Result<Stream<'_, S>> {
    container
        .stream(id)
        .map_err(|error| error.with_context("PDB stream reference"))?
        .ok_or_else(|| Error::new(ErrorKind::InvalidReference, None).with_stream(id, 0))
}
