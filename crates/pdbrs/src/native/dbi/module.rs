//! Module streams, their C13 subsections and their global references.

use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::ops::Range;

use super::{Module, align4, logical, read_u32};
use crate::bytes::into_owned;
use crate::native::codeview::Records;
use crate::native::lines::SubsectionKind;
use crate::native::msf::StreamId;
use crate::{Bytes, Error, ErrorKind, Result};

mod global_refs;
mod write;

pub use global_refs::{GlobalRefs, GlobalRefsIter};
pub use write::ModuleStreamParts;

/// The `CV_SIGNATURE_C13` value at the start of a symbol region.
const SIGNATURE_C13: u32 = 4;
/// The size of the signature at the start of a symbol region.
const SIGNATURE_SIZE: usize = 4;
/// The size of a C13 subsection header: a kind and a length.
const SUBSECTION_HEADER_SIZE: usize = 8;

/// The region sizes of a module stream, as the DBI module record keeps them.
///
/// [`ModuleStreamParts::encode`] returns the sizes of the stream that it
/// writes. Copy them into the module record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ModuleRegionSizes {
    /// The size of the symbol region, with the 4-byte signature. The value
    /// is 0 if the stream has no symbol region.
    pub symbol_bytes: u32,
    /// The size of the C11 line region.
    pub c11_bytes: u32,
    /// The size of the C13 line region.
    pub c13_bytes: u32,
}

/// A module stream split into its regions, with owned serialized bytes.
///
/// The regions come in this order: symbols, C11 lines, C13 lines, the global
/// reference size and the global references. The module info record gives
/// the sizes of the first three regions. A symbol size of 0 means that the
/// symbol region and its signature are absent.
///
/// This type does not decode line data. [`ModuleStream::c13_subsections`]
/// frames the C13 region, and the C11 region stays raw.
#[derive(Clone, Debug)]
pub struct ModuleStream {
    bytes: Vec<u8>,
    stream: StreamId,
    symbols: Range<usize>,
    c11: Range<usize>,
    c13: Range<usize>,
    global_refs: Range<usize>,
}

impl ModuleStream {
    /// Split the module stream `bytes` with the region sizes of `module`.
    ///
    /// The regions must fill the stream exactly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if `module` has no module stream.
    /// - [`ErrorKind::InvalidFormat`] if the symbol size is 1 to 3 bytes, or
    ///   if bytes follow the global references.
    /// - [`ErrorKind::UnsupportedVersion`] if the symbol signature is not C13.
    /// - [`ErrorKind::UnexpectedEof`] if a region or the global reference size is truncated.
    /// - [`ErrorKind::LimitExceeded`] or [`ErrorKind::OffsetOverflow`] if a
    ///   region end cannot be represented.
    ///
    /// Errors other than [`ErrorKind::MissingStream`] identify the module
    /// stream and a logical offset in it.
    pub fn parse(bytes: Vec<u8>, module: &Module<'_>) -> Result<Self> {
        let stream = module.stream.ok_or_else(|| {
            Error::new(ErrorKind::MissingStream, None).with_context("module has no module stream")
        })?;
        let id = stream.0;
        let length = bytes.len();
        let symbol_region = region(
            id,
            0,
            module.symbol_bytes,
            length,
            "module symbol region is truncated",
        )?;
        let symbols = if symbol_region.is_empty() {
            0..0
        } else {
            if symbol_region.len() < SIGNATURE_SIZE {
                return Err(module_error(
                    id,
                    ErrorKind::InvalidFormat,
                    0,
                    "module symbol region is smaller than its signature",
                ));
            }
            if read_u32(&bytes, 0) != Some(SIGNATURE_C13) {
                return Err(module_error(
                    id,
                    ErrorKind::UnsupportedVersion,
                    0,
                    "module symbol signature is not C13",
                ));
            }
            SIGNATURE_SIZE..symbol_region.end
        };
        let c11 = region(
            id,
            symbol_region.end,
            module.c11_bytes,
            length,
            "module C11 line region is truncated",
        )?;
        let c13 = region(
            id,
            c11.end,
            module.c13_bytes,
            length,
            "module C13 line region is truncated",
        )?;
        let refs_size = read_u32(&bytes, c13.end).ok_or_else(|| {
            module_error(
                id,
                ErrorKind::UnexpectedEof,
                length,
                "module global reference size is truncated",
            )
        })?;
        // `read_u32` succeeded, so the four bytes lie inside the stream.
        let global_refs = region(
            id,
            c13.end + 4,
            refs_size,
            length,
            "module global references are truncated",
        )?;
        if global_refs.end != length {
            return Err(module_error(
                id,
                ErrorKind::InvalidFormat,
                global_refs.end,
                "module stream has bytes after the global references",
            ));
        }
        Ok(Self {
            bytes,
            stream,
            symbols,
            c11,
            c13,
            global_refs,
        })
    }

    /// Return the module stream index.
    #[must_use]
    pub const fn stream(&self) -> StreamId {
        self.stream
    }

    /// Return the module stream bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Return the symbol records without the signature.
    ///
    /// A record offset is relative to [`ModuleStream::symbol_offset`]. Add
    /// the two values to get the offset in the module stream, which other
    /// records use as a reference.
    #[must_use]
    pub fn symbols(&self) -> Records<'_> {
        Records::new(self.symbol_data())
    }

    /// Return the raw symbol record bytes without the signature.
    #[must_use]
    pub fn symbol_data(&self) -> &[u8] {
        &self.bytes[self.symbols.clone()]
    }

    /// Return the module stream offset of the first symbol record.
    ///
    /// The value is 4 if the symbol region is present and 0 if it is absent.
    #[must_use]
    pub const fn symbol_offset(&self) -> usize {
        self.symbols.start
    }

    /// Return the raw C11 line region.
    #[must_use]
    pub fn c11_data(&self) -> &[u8] {
        &self.bytes[self.c11.clone()]
    }

    /// Return the raw C13 line region.
    #[must_use]
    pub fn c13_data(&self) -> &[u8] {
        &self.bytes[self.c13.clone()]
    }

    /// Return an iterator that frames the C13 line region into subsections.
    #[must_use]
    pub fn c13_subsections(&self) -> Subsections<'_> {
        Subsections {
            bytes: self.c13_data(),
            base: self.c13.start,
            stream: self.stream.0,
            position: 0,
        }
    }

    /// Return the raw global reference bytes, without their size field.
    #[must_use]
    pub fn global_ref_data(&self) -> &[u8] {
        &self.bytes[self.global_refs.clone()]
    }

    /// Decode the global references: offsets into the symbol record stream.
    ///
    /// Call [`GlobalRefs::check`] to check the offsets against the
    /// symbol record stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the size of the global
    /// references is not a multiple of 4. The error identifies the module
    /// stream and the offset of the global references.
    pub fn global_refs(&self) -> Result<GlobalRefs<'_>> {
        GlobalRefs::parse(
            self.global_ref_data(),
            self.stream.0,
            self.global_refs.start,
        )
    }

    /// Return the region sizes of the stream, as the module record keeps
    /// them.
    #[must_use]
    pub fn region_sizes(&self) -> ModuleRegionSizes {
        // `parse` read each size from a `u32` field of the module record.
        let size = |range: &Range<usize>| u32::try_from(range.len()).unwrap_or(u32::MAX);
        let symbol_bytes = if self.symbols.start == SIGNATURE_SIZE {
            size(&(0..self.symbols.end))
        } else {
            0
        };
        ModuleRegionSizes {
            symbol_bytes,
            c11_bytes: size(&self.c11),
            c13_bytes: size(&self.c13),
        }
    }
}

/// One framed C13 subsection.
///
/// The kind keeps unknown values and the ignore bit.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Subsection<'a> {
    /// The offset of the subsection header in the C13 line region.
    pub offset: u32,
    /// The `DEBUG_S_SUBSECTION_TYPE` value.
    pub kind: SubsectionKind,
    /// The subsection contents, without the header and the alignment padding.
    pub data: Bytes<'a>,
}

into_owned!(struct Subsection { data; offset, kind });

/// The offset, the kind and the data of one framed subsection.
type SubsectionParts<'a> = (u32, SubsectionKind, &'a [u8]);

/// An iterator that frames a C13 line region into subsections.
///
/// Each subsection has an 8-byte header with a kind and a data length. The
/// data is padded to a 4-byte boundary. The iterator returns one error for
/// a malformed subsection and then stops.
#[derive(Clone, Debug)]
pub struct Subsections<'a> {
    /// The C13 line region.
    bytes: &'a [u8],
    /// The offset of the C13 line region in the module stream.
    base: usize,
    /// The module stream index for errors.
    stream: u32,
    /// The offset of the next subsection in `bytes`.
    position: usize,
}

impl<'a> Subsections<'a> {
    /// Frame the next subsection and return its offset, its kind and its data.
    ///
    /// The function moves the iterator like [`Iterator::next`]. The data
    /// borrows the C13 line region for `'a`, so a decoder can keep it
    /// after the iterator moves on.
    pub fn next_parts(&mut self) -> Option<Result<(u32, SubsectionKind, &'a [u8])>> {
        if self.position >= self.bytes.len() {
            return None;
        }
        match self.read() {
            Ok((parts, next)) => {
                self.position = next;
                Some(Ok(parts))
            }
            Err(error) => {
                self.position = self.bytes.len();
                Some(Err(error))
            }
        }
    }

    /// Frame the subsection at the current position; return its parts and the next position.
    fn read(&self) -> Result<(SubsectionParts<'a>, usize)> {
        let start = self.position;
        let end = self.base.saturating_add(self.bytes.len());
        let error = |kind, context| module_error(self.stream, kind, end, context);
        let rest = &self.bytes[start..];
        let header = rest
            .first_chunk::<SUBSECTION_HEADER_SIZE>()
            .ok_or_else(|| {
                error(
                    ErrorKind::UnexpectedEof,
                    "C13 subsection header is truncated",
                )
            })?;
        let [k0, k1, k2, k3, l0, l1, l2, l3] = *header;
        let size = usize::try_from(u32::from_le_bytes([l0, l1, l2, l3])).map_err(|_| {
            error(
                ErrorKind::LimitExceeded,
                "C13 subsection size does not fit in memory",
            )
        })?;
        let data_end = SUBSECTION_HEADER_SIZE
            .checked_add(size)
            .ok_or_else(|| error(ErrorKind::OffsetOverflow, "C13 subsection end overflows"))?;
        let data = rest
            .get(SUBSECTION_HEADER_SIZE..data_end)
            .ok_or_else(|| error(ErrorKind::UnexpectedEof, "C13 subsection data is truncated"))?;
        // The subsection starts at a 4-byte boundary, so its length aligns the end.
        let padded = align4(data_end)
            .filter(|&padded| padded <= rest.len())
            .ok_or_else(|| {
                error(
                    ErrorKind::UnexpectedEof,
                    "C13 subsection padding is truncated",
                )
            })?;
        let offset = u32::try_from(start).map_err(|_| {
            error(
                ErrorKind::OffsetOverflow,
                "C13 subsection offset does not fit in 32 bits",
            )
        })?;
        let kind = SubsectionKind(u32::from_le_bytes([k0, k1, k2, k3]));
        Ok(((offset, kind, data), start + padded))
    }
}

impl<'a> Iterator for Subsections<'a> {
    type Item = Result<Subsection<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.next_parts()?.map(|(offset, kind, data)| Subsection {
            offset,
            kind,
            data: Bytes::new(data),
        }))
    }
}

impl FusedIterator for Subsections<'_> {}

/// Check a region of `size` bytes at `start` in a stream of `length` bytes.
fn region(
    stream: u32,
    start: usize,
    size: u32,
    length: usize,
    context: &'static str,
) -> Result<Range<usize>> {
    let size = usize::try_from(size).map_err(|_| {
        module_error(
            stream,
            ErrorKind::LimitExceeded,
            start,
            "module region size does not fit in memory",
        )
    })?;
    let end = start.checked_add(size).ok_or_else(|| {
        module_error(
            stream,
            ErrorKind::OffsetOverflow,
            start,
            "module region end overflows",
        )
    })?;
    if end > length {
        return Err(module_error(
            stream,
            ErrorKind::UnexpectedEof,
            length,
            context,
        ));
    }
    Ok(start..end)
}

/// Create an error at a logical offset in a module stream.
fn module_error(stream: u32, kind: ErrorKind, offset: usize, context: &'static str) -> Error {
    Error::new(kind, None)
        .with_context(context)
        .with_stream(StreamId(stream), logical(offset))
}
