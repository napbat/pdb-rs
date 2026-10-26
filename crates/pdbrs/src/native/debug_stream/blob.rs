//! The `.xdata` (slot 7) and `.pdata` (slot 8) blobs.
//!
//! `link.exe /DEBUGTYPE:PDATA` copies the `.xdata` and `.pdata` section
//! contents of the image into two streams. Each stream starts with the
//! `DbgRvaVaBlob` header of `langapi/include/pdb.h`:
//!
//! | Offset | Field | Meaning |
//! | --- | --- | --- |
//! | 0 | `ver` | The blob version: 1 (`vdbOne`) |
//! | 4 | `cbHdr` | The header size in bytes: 32 or more |
//! | 8 | `cbData` | The data size in bytes |
//! | 12 | `rvaDataBase` | The RVA of the first data byte in the image |
//! | 16 | `vaImageBase` | The preferred image base (64 bits) |
//! | 24 | `ulReserved1`, `ulReserved2` | Reserved, 0 |
//!
//! The data follows the header. The decoder keeps header bytes after offset 32
//! as raw bytes, and the encoder writes them again.
//!
//! The `.pdata` data is an array of `RUNTIME_FUNCTION` entries. The entry
//! layout depends on the image machine of the DBI header:
//! [`RvaVaBlob::runtime_functions`] decodes x64 and ARM64 entries. The
//! `.xdata` data holds unwind information. A `.pdata` entry gives the RVA of
//! its unwind information; [`RvaVaBlob::data_at`] finds it in the `.xdata` blob.
//! This crate does not decode the unwind information.

use alloc::vec::Vec;

use super::{Codec, DebugRecord, RecordIter};
use crate::native::msf::StreamId;
use crate::native::output::{Output, to_u32};
use crate::native::words::word;
use crate::{Error, ErrorKind, Result};

/// The size of the `DbgRvaVaBlob` header without extra bytes.
pub const BLOB_HEADER_SIZE: usize = 32;
/// The blob version that this crate decodes (`vdbOne`).
pub const BLOB_VERSION: u32 = 1;
/// The `IMAGE_FILE_MACHINE_AMD64` value of the DBI header.
pub const MACHINE_AMD64: u16 = 0x8664;
/// The `IMAGE_FILE_MACHINE_ARM64` value of the DBI header.
pub const MACHINE_ARM64: u16 = 0xaa64;

/// The fixed fields of a `DbgRvaVaBlob` header.
///
/// The `cbHdr` and `cbData` fields come from the lengths of the extra header
/// bytes and of the data: see [`RvaVaBlob::header_size`] and [`RvaVaBlob::data_size`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BlobHeader {
    /// The blob version (`ver`).
    pub version: u32,
    /// The RVA of the first data byte in the image (`rvaDataBase`).
    pub data_rva: u32,
    /// The preferred image base (`vaImageBase`).
    pub image_base: u64,
    /// The reserved fields (`ulReserved1`, `ulReserved2`). Writers set them to 0.
    pub reserved: [u32; 2],
}

impl BlobHeader {
    /// Make a version 1 header with reserved fields of 0.
    #[must_use]
    pub const fn new(data_rva: u32, image_base: u64) -> Self {
        Self {
            version: BLOB_VERSION,
            data_rva,
            image_base,
            reserved: [0; 2],
        }
    }
}

/// A decoded `.xdata` or `.pdata` blob with owned serialized bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RvaVaBlob {
    /// The fixed header fields.
    header: BlobHeader,
    /// The complete stream bytes.
    bytes: Vec<u8>,
    /// The header size (`cbHdr`); the data starts at this offset.
    data_start: usize,
    /// The stream of the blob, if a stream gave it.
    stream: Option<StreamId>,
}

impl RvaVaBlob {
    /// Decode the blob stream `stream`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the stream is shorter than [`BLOB_HEADER_SIZE`].
    /// - [`ErrorKind::UnsupportedVersion`] if `ver` is not [`BLOB_VERSION`].
    /// - [`ErrorKind::InvalidFormat`] if `cbHdr` is less than [`BLOB_HEADER_SIZE`]
    ///   or greater than the stream length, or if `cbHdr + cbData` is not the
    ///   stream length.
    ///
    /// The error identifies `stream` and the offset of the bad field.
    pub fn parse(bytes: Vec<u8>, stream: StreamId) -> Result<Self> {
        let located = |kind: ErrorKind, context: &'static str, offset: u64| {
            Error::new(kind, None)
                .with_context(context)
                .with_stream(stream.0, offset)
        };
        let Some(fixed) = bytes.first_chunk::<BLOB_HEADER_SIZE>() else {
            return Err(located(
                ErrorKind::UnexpectedEof,
                "blob header is truncated",
                0,
            ));
        };
        let words = fixed.as_chunks::<4>().0;
        let version = word(words, 0);
        if version != BLOB_VERSION {
            return Err(located(
                ErrorKind::UnsupportedVersion,
                "blob version is not 1",
                0,
            ));
        }
        let header_size = usize::try_from(word(words, 1)).unwrap_or(usize::MAX);
        if header_size < BLOB_HEADER_SIZE || header_size > bytes.len() {
            return Err(located(
                ErrorKind::InvalidFormat,
                "blob header size does not fit the stream",
                4,
            ));
        }
        let data_size = usize::try_from(word(words, 2)).unwrap_or(usize::MAX);
        if data_size != bytes.len() - header_size {
            return Err(located(
                ErrorKind::InvalidFormat,
                "blob data size does not match the stream length",
                8,
            ));
        }
        let header = BlobHeader {
            version,
            data_rva: word(words, 3),
            image_base: u64::from(word(words, 4)) | (u64::from(word(words, 5)) << 32),
            reserved: [word(words, 6), word(words, 7)],
        };
        Ok(Self {
            header,
            bytes,
            data_start: header_size,
            stream: Some(stream),
        })
    }

    /// Make a blob from a header, extra header bytes and data.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the header version is not [`BLOB_VERSION`].
    /// - [`ErrorKind::LimitExceeded`] if a size does not fit in 32 bits or the
    ///   output cannot be allocated.
    pub fn new(header: BlobHeader, extra_header: &[u8], data: &[u8]) -> Result<Self> {
        if header.version != BLOB_VERSION {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("blob version is not 1"));
        }
        let data_start = BLOB_HEADER_SIZE.saturating_add(extra_header.len());
        to_u32(data_start, "blob header size exceeds 32 bits")?;
        to_u32(data.len(), "blob data size exceeds 32 bits")?;
        let mut blob = Self {
            header,
            bytes: Vec::new(),
            data_start,
            stream: None,
        };
        let mut bytes = Vec::new();
        blob.write(&mut bytes, extra_header, data)?;
        blob.bytes = bytes;
        Ok(blob)
    }

    /// Return the fixed header fields.
    #[must_use]
    pub const fn header(&self) -> BlobHeader {
        self.header
    }

    /// Return the header size in bytes (`cbHdr`).
    #[must_use]
    pub const fn header_size(&self) -> usize {
        self.data_start
    }

    /// Return the data size in bytes (`cbData`).
    #[must_use]
    pub const fn data_size(&self) -> usize {
        self.bytes.len() - self.data_start
    }

    /// Return the header bytes after the fixed fields. Version 1 writers write none.
    #[must_use]
    pub fn extra_header(&self) -> &[u8] {
        &self.bytes[BLOB_HEADER_SIZE..self.data_start]
    }

    /// Return the data: the section contents.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.bytes[self.data_start..]
    }

    /// Return the data from image address `rva` to the end of the data.
    ///
    /// Return `None` if `rva` is before [`BlobHeader::data_rva`] or after the data.
    #[must_use]
    pub fn data_at(&self, rva: u32) -> Option<&[u8]> {
        let offset = usize::try_from(rva.checked_sub(self.header.data_rva)?).ok()?;
        self.data().get(offset..)
    }

    /// Return the `RUNTIME_FUNCTION` entries of a `.pdata` blob.
    ///
    /// `machine` is [`DbiHeader::machine`]. The function decodes 12-byte x64
    /// entries for [`MACHINE_AMD64`] and 8-byte ARM64 entries for
    /// [`MACHINE_ARM64`]. Return `None` for other machines: their layouts
    /// are not decoded.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] if the data length is not a multiple
    /// of the entry size. The error has the stream location of the truncated entry.
    ///
    /// [`DbiHeader::machine`]: crate::native::dbi::DbiHeader::machine
    pub fn runtime_functions(&self, machine: u16) -> Result<Option<RuntimeFunctions<'_>>> {
        let entries = match machine {
            MACHINE_AMD64 => RuntimeFunctions::X64(self.entries()?),
            MACHINE_ARM64 => RuntimeFunctions::Arm64(self.entries()?),
            _ => return Ok(None),
        };
        Ok(Some(entries))
    }

    /// Check the data length and return an iterator over the entries.
    fn entries<R: DebugRecord>(&self) -> Result<RecordIter<'_, R>> {
        let data = self.data();
        let tail = data.len() % R::SIZE;
        if tail != 0 {
            let mut error = Error::new(ErrorKind::UnexpectedEof, None).with_context(R::TRUNCATED);
            if let Some(stream) = self.stream {
                let offset = self.data_start + data.len() - tail;
                error = error.with_stream(stream.0, u64::try_from(offset).unwrap_or(u64::MAX));
            }
            return Err(error);
        }
        Ok(RecordIter::new(data))
    }

    /// Append the blob to `out`.
    ///
    /// The encoder writes the header fields, the extra header bytes and the data.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a size does not fit in 32 bits
    /// or the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        self.write(out, self.extra_header(), self.data())
    }

    /// Return the encoded blob as a new vector.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`RvaVaBlob::encode`].
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.encode(&mut out)?;
        Ok(out)
    }

    /// Append the header of this blob, `extra_header` and `data` to `out`.
    fn write(&self, out: &mut Vec<u8>, extra_header: &[u8], data: &[u8]) -> Result<()> {
        let header_size = BLOB_HEADER_SIZE.saturating_add(extra_header.len());
        let mut out = Output::new(out);
        out.reserve(header_size.saturating_add(data.len()))?;
        out.u32(self.header.version)?;
        out.u32(to_u32(header_size, "blob header size exceeds 32 bits")?)?;
        out.u32(to_u32(data.len(), "blob data size exceeds 32 bits")?)?;
        out.u32(self.header.data_rva)?;
        out.u64(self.header.image_base)?;
        out.u32(self.header.reserved[0])?;
        out.u32(self.header.reserved[1])?;
        out.bytes(extra_header)?;
        out.bytes(data)
    }
}

/// The `RUNTIME_FUNCTION` entries of a `.pdata` blob.
#[derive(Clone, Debug)]
pub enum RuntimeFunctions<'a> {
    /// The 12-byte entries of an x64 image.
    X64(RecordIter<'a, X64RuntimeFunction>),
    /// The 8-byte entries of an ARM64 image.
    Arm64(RecordIter<'a, Arm64RuntimeFunction>),
}

/// One x64 `RUNTIME_FUNCTION` entry of `winnt.h` (`_IMAGE_RUNTIME_FUNCTION_ENTRY`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct X64RuntimeFunction {
    /// The RVA of the first byte of the function (`BeginAddress`).
    pub begin: u32,
    /// The RVA after the last byte of the function (`EndAddress`).
    pub end: u32,
    /// The RVA of the unwind information (`UnwindInfoAddress`).
    pub unwind_info: u32,
}

impl X64RuntimeFunction {
    /// Make an entry from its fields.
    #[must_use]
    pub const fn new(begin: u32, end: u32, unwind_info: u32) -> Self {
        Self {
            begin,
            end,
            unwind_info,
        }
    }
}

impl Codec for X64RuntimeFunction {
    const TRUNCATED: &'static str = "x64 runtime function entry is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
        Self {
            begin: word(record, 0),
            end: word(record, 1),
            unwind_info: word(record, 2),
        }
    }
}

impl DebugRecord for X64RuntimeFunction {
    const SIZE: usize = 12;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut out = Output::new(out);
        out.reserve(Self::SIZE)?;
        out.u32(self.begin)?;
        out.u32(self.end)?;
        out.u32(self.unwind_info)
    }
}

/// One ARM64 `RUNTIME_FUNCTION` entry of `winnt.h` (`_IMAGE_ARM64_RUNTIME_FUNCTION_ENTRY`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Arm64RuntimeFunction {
    /// The RVA of the first byte of the function (`BeginAddress`).
    pub begin: u32,
    /// The unwind word (`UnwindData`): an unwind information RVA or packed unwind data.
    pub unwind_data: u32,
}

impl Arm64RuntimeFunction {
    /// Make an entry from its fields.
    #[must_use]
    pub const fn new(begin: u32, unwind_data: u32) -> Self {
        Self { begin, unwind_data }
    }

    /// Return the `Flag` field: bits 0 and 1 of the unwind word.
    ///
    /// The value 0 means that the unwind word is the RVA of the unwind
    /// information. The values 1 and 2 mean packed unwind data.
    #[must_use]
    pub const fn flag(&self) -> u8 {
        self.unwind_data.to_le_bytes()[0] & 3
    }
}

impl Codec for Arm64RuntimeFunction {
    const TRUNCATED: &'static str = "ARM64 runtime function entry is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
        Self {
            begin: word(record, 0),
            unwind_data: word(record, 1),
        }
    }
}

impl DebugRecord for Arm64RuntimeFunction {
    const SIZE: usize = 8;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut out = Output::new(out);
        out.reserve(Self::SIZE)?;
        out.u32(self.begin)?;
        out.u32(self.unwind_data)
    }
}
