//! Native PDB stream discovery and bounded typed loading.

use alloc::vec::Vec;

use super::dbi::{Dbi, DebugStream, Module, ModuleStream};
use super::fpo::{FpoStream, FrameDataStream};
use super::info::Info;
use super::msf::{Msf, MsfLimits, Stream, StreamId};
use super::symbols::{BucketCount, GlobalSymbolIndex, PublicSymbolIndex};
use super::types::TypeStream;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// Resource limits for native container metadata and explicitly loaded streams.
#[derive(Clone, Copy, Debug)]
pub struct ReadOptions {
    /// Limits for the MSF directory and block metadata.
    pub msf: MsfLimits,
    /// Maximum bytes in any stream materialized by a typed accessor.
    pub max_stream_bytes: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            msf: MsfLimits::default(),
            max_stream_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A native PDB with a validated MSF directory and decoded Info stream.
///
/// Opening does not decode all streams. Accessors load and validate the requested
/// stream. Raw access remains available for data without a semantic decoder.
#[derive(Debug)]
pub struct Pdb<S> {
    msf: Msf<S>,
    info: Info,
    options: ReadOptions,
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
        let msf = Msf::with_limits(source, options.msf)?;
        if msf.stream_count() <= 1 {
            return Err(Error::new(ErrorKind::MissingStream, None).with_context("PDB Info"));
        }
        let stream = msf
            .stream(StreamId(1))?
            .ok_or_else(|| Error::new(ErrorKind::MissingStream, None).with_context("PDB Info"))?;
        let info = Info::parse(stream.read_all(options.max_stream_bytes)?)?;
        for named in info.named_streams() {
            require_stream(&msf, named.stream)?;
        }
        Ok(Self { msf, info, options })
    }

    /// Return the native container for raw stream access.
    #[must_use]
    pub const fn msf(&self) -> &Msf<S> {
        &self.msf
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
            .map_or(Ok(None), |id| self.msf.stream(id))
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
        require_stream(&self.msf, id)?.read_all(self.options.max_stream_bytes)
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

    /// Load the global symbol index that the DBI header names, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`] and [`GlobalSymbolIndex::parse`].
    pub fn global_symbols(&self, dbi: &Dbi) -> Result<Option<GlobalSymbolIndex>> {
        dbi.header()
            .global_stream
            .map(|id| {
                GlobalSymbolIndex::parse(self.stream_bytes(id)?, id, self.symbol_bucket_count())
            })
            .transpose()
    }

    /// Load the public symbol index that the DBI header names, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`] and [`PublicSymbolIndex::parse`].
    pub fn public_symbols(&self, dbi: &Dbi) -> Result<Option<PublicSymbolIndex>> {
        dbi.header()
            .public_stream
            .map(|id| {
                PublicSymbolIndex::parse(self.stream_bytes(id)?, id, self.symbol_bucket_count())
            })
            .transpose()
    }

    /// Load the FPO stream that the [`DebugStream::Fpo`] slot of `dbi` names, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`] and [`FpoStream::parse`].
    pub fn fpo(&self, dbi: &Dbi) -> Result<Option<FpoStream>> {
        dbi.debug_stream(DebugStream::Fpo)
            .map(|id| FpoStream::parse(self.stream_bytes(id)?, id))
            .transpose()
    }

    /// Load the new FPO stream that the [`DebugStream::NewFpo`] slot of `dbi` names, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`] and [`FrameDataStream::parse`].
    pub fn frame_data(&self, dbi: &Dbi) -> Result<Option<FrameDataStream>> {
        dbi.debug_stream(DebugStream::NewFpo)
            .map(|id| FrameDataStream::parse(self.stream_bytes(id)?, id))
            .transpose()
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
            .map_err(|error| error.with_stream(id.0, 0))?;
        for reference in [types.header().hash_stream, types.header().hash_aux_stream]
            .into_iter()
            .flatten()
        {
            require_stream(&self.msf, reference)?;
        }
        Ok(Some(types))
    }

    /// Load DBI metadata and check its age and referenced stream IDs.
    ///
    /// # Errors
    ///
    /// Returns errors for I/O, limits, malformed DBI, age mismatch or invalid
    /// references. Module payloads are checked only when explicitly loaded.
    pub fn debug_info(&self) -> Result<Option<Dbi>> {
        let Some(stream) = self.optional_stream(StreamId(3))? else {
            return Ok(None);
        };
        let dbi = Dbi::parse(stream.read_all(self.options.max_stream_bytes)?)?;
        if dbi.header().age != self.info.identity().age() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("PDB Info and DBI ages differ")
                .with_stream(3, 8));
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
            require_stream(&self.msf, reference)?;
        }
        for reference in dbi.debug_streams().flatten() {
            require_stream(&self.msf, reference)?;
        }
        for module in dbi.modules() {
            if let Some(reference) = module?.stream {
                require_stream(&self.msf, reference)?;
            }
        }
        Ok(Some(dbi))
    }

    /// Load the symbols and line-data regions of a DBI module.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing referenced stream, exceeded limits,
    /// malformed module regions or failed I/O.
    pub fn module_stream(&self, module: &Module<'_>) -> Result<Option<ModuleStream>> {
        let Some(id) = module.stream else {
            return Ok(None);
        };
        let stream = require_stream(&self.msf, id)?;
        ModuleStream::parse(stream.read_all(self.options.max_stream_bytes)?, module).map(Some)
    }

    fn optional_stream(&self, id: StreamId) -> Result<Option<Stream<'_, S>>> {
        if id.0 >= self.msf.stream_count() {
            return Ok(None);
        }
        self.msf.stream(id)
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
    /// # Errors
    ///
    /// Returns file I/O, container, Info-stream or reference errors.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::read(crate::io::FileReader::open(path)?)
    }
}

fn require_stream<S: ReadAt>(msf: &Msf<S>, id: StreamId) -> Result<Stream<'_, S>> {
    msf.stream(id)
        .map_err(|error| error.with_context("PDB stream reference"))?
        .ok_or_else(|| Error::new(ErrorKind::InvalidReference, None).with_stream(id.0, 0))
}
