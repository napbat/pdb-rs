//! All C13 subsections of one module stream.

use alloc::vec::Vec;

use super::{FileChecksums, LineSubsection, SubsectionKind, invalid, too_large};
use crate::bytes::into_owned;
use crate::native::codeview::index::ChecksumOffset;
use crate::native::dbi::ModuleStream;
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// One decoded subsection of a module stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ModuleSubsection<'a> {
    /// The offset of the subsection header in the C13 line region.
    pub offset: u32,
    /// The subsection kind.
    pub kind: SubsectionKind,
    /// The decoded data.
    pub data: LineSubsection<'a>,
}

into_owned!(struct ModuleSubsection { data; offset, kind });

/// The decoded C13 line region of one module stream.
///
/// A module has at most one file checksum subsection. Line blocks and
/// inlinee entries refer to its entries by byte offset.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ModuleLines<'a> {
    /// The module stream index for errors.
    stream: u32,
    /// The offset of the C13 line region in the module stream.
    c13_start: usize,
    /// The subsections in stream order.
    subsections: Vec<ModuleSubsection<'a>>,
    /// The position of the file checksum subsection in `subsections`.
    checksums: Option<usize>,
}

into_owned!(struct ModuleLines { subsections; stream, c13_start, checksums });

impl<'a> ModuleLines<'a> {
    /// Frame and decode all subsections of the C13 line region of `module`.
    ///
    /// The function does not check the file checksum references. Call
    /// [`ModuleLines::check`] for that.
    ///
    /// # Errors
    ///
    /// All errors identify the module stream and a logical offset in it.
    ///
    /// - The framing errors of [`ModuleStream::c13_subsections`].
    /// - The errors of [`LineSubsection::decode`], at the subsection header.
    /// - [`ErrorKind::InvalidFormat`] if the region has more than one file
    ///   checksum subsection.
    /// - [`ErrorKind::LimitExceeded`] if the subsection list cannot be
    ///   allocated.
    pub fn parse(module: &'a ModuleStream) -> Result<Self> {
        let stream = module.stream().0;
        // The C13 region follows the symbol region and the C11 region.
        let c13_start =
            module.symbol_offset() + module.symbol_data().len() + module.c11_data().len();
        let mut lines = Self {
            stream,
            c13_start,
            subsections: Vec::new(),
            checksums: None,
        };
        let mut subsections = module.c13_subsections();
        while let Some(parts) = subsections.next_parts() {
            let (offset, kind, data) = parts?;
            let data =
                LineSubsection::decode(kind, data).map_err(|error| lines.locate(error, offset))?;
            if matches!(data, LineSubsection::FileChecksums(_)) {
                if lines.checksums.is_some() {
                    return Err(lines.locate(
                        invalid("module has more than one file checksum subsection"),
                        offset,
                    ));
                }
                lines.checksums = Some(lines.subsections.len());
            }
            lines.subsections.try_reserve(1).map_err(|_| {
                lines.locate(
                    too_large("module subsection list cannot be allocated"),
                    offset,
                )
            })?;
            lines
                .subsections
                .push(ModuleSubsection { offset, kind, data });
        }
        Ok(lines)
    }

    /// Return the decoded subsections in stream order.
    #[must_use]
    pub fn subsections(&self) -> &[ModuleSubsection<'a>] {
        &self.subsections
    }

    /// Return the file checksum table, if the module has one.
    #[must_use]
    pub fn checksums(&self) -> Option<&FileChecksums<'a>> {
        let position = self.checksums?;
        match &self.subsections.get(position)?.data {
            LineSubsection::FileChecksums(checksums) => Some(checksums),
            _ => None,
        }
    }

    /// Append the C13 line region to `out`: each subsection with its header
    /// and padding, in stream order.
    ///
    /// The function encodes the decoded values again with
    /// [`LineSubsection::encode_framed`]. For canonical input the output is
    /// equal to [`ModuleStream::c13_data`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`LineSubsection::encode_framed`]. The errors
    /// identify the module stream and the header of the subsection.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        for subsection in &self.subsections {
            subsection
                .data
                .encode_framed(out)
                .map_err(|error| self.locate(error, subsection.offset))?;
        }
        Ok(())
    }

    /// Make sure that each file reference identifies a file checksum entry.
    ///
    /// The function checks the file of each block of the line and IL line
    /// subsections, and the file and the extra files of each inlinee entry.
    /// A reference must be the offset of an entry start, not an offset
    /// inside an entry.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if a reference does not
    /// identify an entry, or if the module has references but no file
    /// checksum subsection. The error identifies the module stream and the
    /// header of the subsection with the reference.
    pub fn check(&self) -> Result<()> {
        let checksums = self.checksums();
        let check = |offset: ChecksumOffset, subsection: &ModuleSubsection<'a>| {
            if checksums.is_some_and(|table| table.contains(offset)) {
                Ok(())
            } else {
                Err(self.locate(
                    Error::new(ErrorKind::InvalidReference, None)
                        .with_context("file reference does not identify a file checksum entry"),
                    subsection.offset,
                ))
            }
        };
        for subsection in &self.subsections {
            match &subsection.data {
                LineSubsection::Lines(lines) | LineSubsection::IlLines(lines) => {
                    for block in lines.blocks() {
                        check(block.checksum_offset, subsection)?;
                    }
                }
                LineSubsection::InlineeLines(inlinees) => {
                    for entry in inlinees {
                        check(entry.checksum_offset, subsection)?;
                        for offset in &entry.extra_files {
                            check(offset, subsection)?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Add the module stream location of a subsection header to `error`.
    fn locate(&self, error: Error, offset: u32) -> Error {
        let offset = usize::try_from(offset)
            .ok()
            .and_then(|offset| offset.checked_add(self.c13_start))
            .and_then(|offset| u64::try_from(offset).ok())
            .unwrap_or(u64::MAX);
        error.with_stream(StreamId(self.stream), offset)
    }
}
