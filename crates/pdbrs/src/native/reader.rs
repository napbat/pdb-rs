//! Native PDB stream discovery and bounded typed loading.
//!
//! [`Pdb`] reads streams through a [`Container`], so all accessors work on MSF 7.00 and MSFZ
//! input.

use alloc::vec::Vec;

use super::address::AddressTables;
use super::container::{Container, ContainerLimits, Stream};
use super::dbi::{Dbi, DebugStream, Module, ModuleStream};
use super::debug_stream::{
    DebugRecord, FixupStream, FunctionEntryStream, OmapStream, RecordStream, RvaVaBlob, TokenRidMap,
};
use super::fpo::{FpoStream, FrameDataStream};
use super::info::Info;
use super::msf::{Msf, MsfWriteOptions, StreamId};
use super::section::SectionHeaderStream;
use super::symbols::{BucketCount, GlobalSymbolIndex, PublicSymbolIndex};
use super::types::TypeStream;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// Resource limits for native container metadata and explicitly loaded streams.
///
/// The default uses [`ContainerLimits::default`] and a stream limit of 64 MiB.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ReadOptions {
    limits: ContainerLimits,
    max_stream_bytes: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            limits: ContainerLimits::default(),
            max_stream_bytes: 64 * 1024 * 1024,
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
}

/// A native PDB with a validated stream directory and decoded Info stream.
///
/// The container is MSF 7.00 or MSFZ. Opening does not decode all streams. Accessors load and
/// validate the requested stream. Raw access remains available for data without a semantic
/// decoder.
///
/// With the `std` feature, `Pdb<S>` is `Sync` if `S` is `Sync`. Without `std`, the MSFZ chunk
/// cache uses a `RefCell`, so the value is not `Sync`.
#[derive(Debug)]
pub struct Pdb<S> {
    container: Container<S>,
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
            .map_err(|error| error.with_stream(id, 0))?;
        for reference in [types.header().hash_stream, types.header().hash_aux_stream]
            .into_iter()
            .flatten()
        {
            require_stream(&self.container, reference)?;
        }
        Ok(Some(types))
    }

    /// Load DBI metadata and check its age and referenced stream IDs.
    ///
    /// The DBI age must not be larger than the PDB Info age. Writers that change only
    /// named streams, for example `pdbstr -w`, increment the Info age and keep the DBI
    /// age. Image matching uses the DBI age.
    ///
    /// # Errors
    ///
    /// Returns errors for I/O, limits, malformed DBI, a DBI age that is larger
    /// than the Info age or invalid references. Module payloads are checked
    /// only when explicitly loaded.
    pub fn debug_info(&self) -> Result<Option<Dbi>> {
        let Some(stream) = self.optional_stream(StreamId(3))? else {
            return Ok(None);
        };
        let dbi = Dbi::parse(stream.read_all(self.options.max_stream_bytes)?)?;
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
        for module in dbi.modules() {
            if let Some(reference) = module?.1.stream {
                require_stream(&self.container, reference)?;
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
        let stream = require_stream(&self.container, id)?;
        ModuleStream::parse(stream.read_all(self.options.max_stream_bytes)?, module).map(Some)
    }

    fn optional_stream(&self, id: StreamId) -> Result<Option<Stream<'_, S>>> {
        if id.0 >= self.container.stream_count() {
            return Ok(None);
        }
        self.container.stream(id)
    }

    /// Load the exception data stream of the [`DebugStream::Exception`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`] and [`RecordStream::parse`].
    pub fn exception_data(&self, dbi: &Dbi) -> Result<Option<FunctionEntryStream>> {
        self.debug_records(dbi, DebugStream::Exception)
    }

    /// Load the fixup stream of the [`DebugStream::Fixup`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`] and [`RecordStream::parse`].
    pub fn fixups(&self, dbi: &Dbi) -> Result<Option<FixupStream>> {
        self.debug_records(dbi, DebugStream::Fixup)
    }

    /// Load the OMAP table of the [`DebugStream::OmapToSource`] or
    /// [`DebugStream::OmapFromSource`] slot, if any.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `kind` is not an OMAP slot.
    /// - The errors of [`Pdb::stream_bytes`] and [`OmapStream::parse`].
    pub fn omap(&self, dbi: &Dbi, kind: DebugStream) -> Result<Option<OmapStream>> {
        if !matches!(
            kind,
            DebugStream::OmapToSource | DebugStream::OmapFromSource
        ) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("the debug stream slot is not an OMAP slot"));
        }
        dbi.debug_stream(kind)
            .map(|id| OmapStream::parse(self.stream_bytes(id)?, id))
            .transpose()
    }

    /// Load the section headers of the [`DebugStream::SectionHeader`] or
    /// [`DebugStream::OriginalSectionHeader`] slot, if any.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `kind` is not a section header slot.
    /// - The errors of [`Pdb::stream_bytes`] and [`RecordStream::parse`].
    pub fn section_headers(
        &self,
        dbi: &Dbi,
        kind: DebugStream,
    ) -> Result<Option<SectionHeaderStream>> {
        if !matches!(
            kind,
            DebugStream::SectionHeader | DebugStream::OriginalSectionHeader
        ) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("the debug stream slot is not a section header slot"));
        }
        self.debug_records(dbi, kind)
    }

    /// Load the token to RID map of the [`DebugStream::TokenRidMap`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::stream_bytes`] and [`RecordStream::parse`].
    pub fn token_rid_map(&self, dbi: &Dbi) -> Result<Option<TokenRidMap>> {
        self.debug_records(dbi, DebugStream::TokenRidMap)
    }

    /// Load the blob of the [`DebugStream::Xdata`] or [`DebugStream::Pdata`] slot, if any.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `kind` is not a blob slot.
    /// - The errors of [`Pdb::stream_bytes`] and [`RvaVaBlob::parse`].
    pub fn debug_blob(&self, dbi: &Dbi, kind: DebugStream) -> Result<Option<RvaVaBlob>> {
        if !matches!(kind, DebugStream::Xdata | DebugStream::Pdata) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("the debug stream slot is not a blob slot"));
        }
        dbi.debug_stream(kind)
            .map(|id| RvaVaBlob::parse(self.stream_bytes(id)?, id))
            .transpose()
    }

    /// Load the section headers and OMAP tables for address translation.
    ///
    /// Return `None` if the DBI stream names no [`DebugStream::SectionHeader`]
    /// stream. Use [`AddressTables::map`] for the translation.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::section_headers`] and [`Pdb::omap`].
    pub fn address_tables(&self, dbi: &Dbi) -> Result<Option<AddressTables>> {
        let Some(sections) = self.section_headers(dbi, DebugStream::SectionHeader)? else {
            return Ok(None);
        };
        Ok(Some(AddressTables {
            sections,
            original: self.section_headers(dbi, DebugStream::OriginalSectionHeader)?,
            to_source: self.omap(dbi, DebugStream::OmapToSource)?,
            from_source: self.omap(dbi, DebugStream::OmapFromSource)?,
        }))
    }

    /// Load the record stream of the `kind` slot of `dbi`, if any.
    fn debug_records<R: DebugRecord>(
        &self,
        dbi: &Dbi,
        kind: DebugStream,
    ) -> Result<Option<RecordStream<R>>> {
        dbi.debug_stream(kind)
            .map(|id| RecordStream::parse(self.stream_bytes(id)?, id))
            .transpose()
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
