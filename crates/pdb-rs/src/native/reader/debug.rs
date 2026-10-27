//! The accessors of the streams that the optional debug header of the DBI stream names.
//!
//! Each accessor finds its slot in the cached DBI stream and loads the stream. The streams
//! are not cached. Without a DBI stream, each accessor returns `None`, as for an empty slot.

use super::Pdb;
use crate::io::ReadAt;
use crate::native::address::AddressTables;
use crate::native::dbi::DebugStream;
use crate::native::debug_stream::{
    DebugRecord, FixupStream, FunctionEntryStream, OmapStream, RecordStream, RvaVaBlob, TokenRidMap,
};
use crate::native::fpo::{FpoStream, FrameDataStream};
use crate::native::msf::StreamId;
use crate::native::section::SectionHeaderStream;
use crate::{Error, ErrorKind, Result};

impl<S: ReadAt> Pdb<S> {
    /// Return the stream in the `kind` slot of the optional debug header, if any.
    fn debug_slot(&self, kind: DebugStream) -> Result<Option<StreamId>> {
        Ok(self.debug_info()?.and_then(|dbi| dbi.debug_stream(kind)))
    }

    /// Load the FPO stream of the [`DebugStream::Fpo`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and
    /// [`FpoStream::parse`].
    pub fn fpo(&self) -> Result<Option<FpoStream>> {
        self.debug_slot(DebugStream::Fpo)?
            .map(|id| FpoStream::parse(self.stream_bytes(id)?, id))
            .transpose()
    }

    /// Load the new FPO stream of the [`DebugStream::NewFpo`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and
    /// [`FrameDataStream::parse`].
    pub fn frame_data(&self) -> Result<Option<FrameDataStream>> {
        self.debug_slot(DebugStream::NewFpo)?
            .map(|id| FrameDataStream::parse(self.stream_bytes(id)?, id))
            .transpose()
    }

    /// Load the exception data stream of the [`DebugStream::Exception`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and
    /// [`RecordStream::parse`].
    pub fn exception_data(&self) -> Result<Option<FunctionEntryStream>> {
        self.debug_records(DebugStream::Exception)
    }

    /// Load the fixup stream of the [`DebugStream::Fixup`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and
    /// [`RecordStream::parse`].
    pub fn fixups(&self) -> Result<Option<FixupStream>> {
        self.debug_records(DebugStream::Fixup)
    }

    /// Load the OMAP table of the [`DebugStream::OmapToSource`] or
    /// [`DebugStream::OmapFromSource`] slot, if any.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `kind` is not an OMAP slot.
    /// - The errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and [`OmapStream::parse`].
    pub fn omap(&self, kind: DebugStream) -> Result<Option<OmapStream>> {
        if !matches!(
            kind,
            DebugStream::OmapToSource | DebugStream::OmapFromSource
        ) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("the debug stream slot is not an OMAP slot"));
        }
        self.debug_slot(kind)?
            .map(|id| OmapStream::parse(self.stream_bytes(id)?, id))
            .transpose()
    }

    /// Load the section headers of the [`DebugStream::SectionHeader`] or
    /// [`DebugStream::OriginalSectionHeader`] slot, if any.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `kind` is not a section header slot.
    /// - The errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and
    ///   [`RecordStream::parse`].
    pub fn section_headers(&self, kind: DebugStream) -> Result<Option<SectionHeaderStream>> {
        if !matches!(
            kind,
            DebugStream::SectionHeader | DebugStream::OriginalSectionHeader
        ) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("the debug stream slot is not a section header slot"));
        }
        self.debug_records(kind)
    }

    /// Load the token to RID map of the [`DebugStream::TokenRidMap`] slot, if any.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and
    /// [`RecordStream::parse`].
    pub fn token_rid_map(&self) -> Result<Option<TokenRidMap>> {
        self.debug_records(DebugStream::TokenRidMap)
    }

    /// Load the blob of the [`DebugStream::Xdata`] or [`DebugStream::Pdata`] slot, if any.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `kind` is not a blob slot.
    /// - The errors of [`Pdb::debug_info`], [`Pdb::stream_bytes`] and [`RvaVaBlob::parse`].
    pub fn debug_blob(&self, kind: DebugStream) -> Result<Option<RvaVaBlob>> {
        if !matches!(kind, DebugStream::Xdata | DebugStream::Pdata) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("the debug stream slot is not a blob slot"));
        }
        self.debug_slot(kind)?
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
    pub fn address_tables(&self) -> Result<Option<AddressTables>> {
        let Some(sections) = self.section_headers(DebugStream::SectionHeader)? else {
            return Ok(None);
        };
        Ok(Some(AddressTables {
            sections,
            original: self.section_headers(DebugStream::OriginalSectionHeader)?,
            to_source: self.omap(DebugStream::OmapToSource)?,
            from_source: self.omap(DebugStream::OmapFromSource)?,
        }))
    }

    /// Load the record stream of the `kind` slot, if any.
    fn debug_records<R: DebugRecord>(&self, kind: DebugStream) -> Result<Option<RecordStream<R>>> {
        self.debug_slot(kind)?
            .map(|id| RecordStream::parse(self.stream_bytes(id)?, id))
            .transpose()
    }
}
