//! Injected source files: the `/src/headerblock` stream and the `/src/files/*` streams.
//!
//! A linker can store source files, for example Natvis files, in a PDB. The named stream
//! `/src/headerblock` describes the files. The layout follows LLVM `RawTypes.h`
//! (`SrcHeaderBlockHeader`, `SrcHeaderBlockEntry`), LLVM `InjectedSourceStream.cpp` and
//! Microsoft `langapi/include/pdb.h` (`SrcHeaderBlock`, `SrcHeaderOut`):
//!
//! 1. A 64-byte header: version, stream size, file time, age and 44 zero padding bytes.
//! 2. A [`HashTable`] from the `/names` offset of the virtual file name to a 40-byte entry.
//!
//! The hash of a key is the key truncated to 16 bits (`HcNi` and `HASH` in Microsoft
//! `map.h`; LLVM `StringTableHashTraits`). [`SourceHeaderBlock::parse`] checks that a
//! linear-probing lookup finds each entry.
//!
//! The contents of each file are in the named stream `/src/files/<virtual file name>`. The
//! name uses the exact bytes of the virtual file name. Microsoft `link.exe` and LLVM store a
//! lowercase virtual file name with backslashes. The original file name keeps its case.
//!
//! This module does not decode the `/names` string table. The caller resolves the name offsets.
//!
//! The .NET symbol writers use the compression byte 101 ([`SourceCompression::DotNet`]). Their
//! streams hold a [`DotNetDocument`] record, not the file contents.

mod document;

pub use document::{
    CHECKSUM_SHA1, CHECKSUM_SHA256, ChecksumCheck, DOCUMENT_TYPE_TEXT, DotNetDocument,
    LANGUAGE_CSHARP, VENDOR_MICROSOFT,
};

use alloc::vec::Vec;

use super::hash::hash_buffer_v8;
use super::hash_table::{Cursor, HashTable, HashTableEntry, HashTableValue};
use super::msf::StreamId;
use super::output::Output;
use crate::{Error, ErrorKind, Result};

/// The name of the stream that describes the injected source files.
pub const HEADER_BLOCK_STREAM_NAME: &[u8] = b"/src/headerblock";

/// The name prefix of the streams that hold injected file contents.
pub const SOURCE_FILE_STREAM_PREFIX: &[u8] = b"/src/files/";

/// The only header and entry version (`srcverOne`, `PdbRaw_SrcHeaderBlockVer::SrcVerOne`).
pub const SOURCE_VERSION_ONE: u32 = 19_980_827;

/// The size of the header in bytes.
const HEADER_SIZE: usize = 64;

/// The number of padding bytes at the end of the header.
const HEADER_PADDING: usize = 44;

/// The size of one entry in bytes.
const ENTRY_SIZE: u32 = 40;

/// The compression of an injected source file (`SrcCompress` in Microsoft `pdb.h`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceCompression {
    /// The stream holds the file contents without compression (`srccompressNone`).
    None,
    /// Run-length encoding (`srccompressRLE`). This reader does not decode it.
    RunLength,
    /// Huffman encoding (`srccompressHuffman`). This reader does not decode it.
    Huffman,
    /// LZ compression (`srccompressLZ`). This reader does not decode it.
    Lz,
    /// A .NET document record (the value 101, LLVM `PDB_SourceCompression::DotNet`).
    ///
    /// It is not a compression method: the stream holds a [`DotNetDocument`].
    DotNet,
    /// A value that `pdb.h` and LLVM do not define.
    Unknown(u8),
}

impl SourceCompression {
    /// Decode the raw compression byte.
    #[must_use]
    pub const fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::None,
            1 => Self::RunLength,
            2 => Self::Huffman,
            3 => Self::Lz,
            101 => Self::DotNet,
            other => Self::Unknown(other),
        }
    }

    /// Return the raw compression byte.
    #[must_use]
    pub const fn to_raw(self) -> u8 {
        match self {
            Self::None => 0,
            Self::RunLength => 1,
            Self::Huffman => 2,
            Self::Lz => 3,
            Self::DotNet => 101,
            Self::Unknown(value) => value,
        }
    }
}

/// One entry of the `/src/headerblock` table (`SrcHeaderBlockEntry`).
///
/// The record size (40) and the version ([`SOURCE_VERSION_ONE`]) are checked and not stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InjectedSource {
    /// The CRC of the file contents: [`hash_buffer_v8`]. For
    /// [`SourceCompression::DotNet`], the CRC of the stream bytes.
    pub crc: u32,
    /// The size of the file contents after decompression. For
    /// [`SourceCompression::DotNet`], the size of the stream.
    pub file_size: u32,
    /// The `/names` offset of the original file name.
    pub file_name_offset: u32,
    /// The `/names` offset of the object file name.
    pub object_name_offset: u32,
    /// The `/names` offset of the virtual file name. The table key has the same value.
    pub virtual_file_name_offset: u32,
    /// The compression of the stream contents.
    pub compression: SourceCompression,
    /// The `fVirtual` flag: `true` for a file that the compiler injected.
    pub is_virtual: bool,
    /// The reserved bytes (`pvReserved1` in `SrcHeaderOut`), kept raw.
    pub reserved: [u8; 8],
}

impl HashTableValue<'_> for InjectedSource {
    const SIZE: usize = 40;

    fn decode(bytes: &[u8]) -> Result<Self> {
        let entry = <&[u8; 40]>::try_from(bytes)
            .map_err(|_| entry_error(ErrorKind::InvalidFormat, "injected source entry size"))?;
        let word = |at: usize| {
            u32::from_le_bytes([entry[at], entry[at + 1], entry[at + 2], entry[at + 3]])
        };
        let size = word(0);
        let version = word(4);
        let crc = word(8);
        let file_size = word(12);
        let file_name_offset = word(16);
        let object_name_offset = word(20);
        let virtual_file_name_offset = word(24);
        let compression = SourceCompression::from_raw(entry[28]);
        let flags = entry[29];
        let padding = u16::from_le_bytes([entry[30], entry[31]]);
        let mut reserved = [0; 8];
        reserved.copy_from_slice(&entry[32..]);
        if size != ENTRY_SIZE {
            return Err(entry_error(
                ErrorKind::InvalidFormat,
                "injected source entry size",
            ));
        }
        if version != SOURCE_VERSION_ONE {
            return Err(entry_error(
                ErrorKind::UnsupportedVersion,
                "injected source entry version",
            ));
        }
        if flags > 1 {
            return Err(entry_error(
                ErrorKind::InvalidFormat,
                "injected source flags have reserved bits",
            ));
        }
        if padding != 0 {
            return Err(entry_error(
                ErrorKind::InvalidFormat,
                "injected source entry padding is not zero",
            ));
        }
        Ok(Self {
            crc,
            file_size,
            file_name_offset,
            object_name_offset,
            virtual_file_name_offset,
            compression,
            is_virtual: flags == 1,
            reserved,
        })
    }

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut output = Output::new(out);
        output.reserve(Self::SIZE)?;
        for word in [
            ENTRY_SIZE,
            SOURCE_VERSION_ONE,
            self.crc,
            self.file_size,
            self.file_name_offset,
            self.object_name_offset,
            self.virtual_file_name_offset,
        ] {
            output.u32(word)?;
        }
        output.u8(self.compression.to_raw())?;
        output.u8(u8::from(self.is_virtual))?;
        output.u16(0)?;
        output.bytes(&self.reserved)
    }
}

impl InjectedSource {
    /// Check the contents of the file stream and return the file contents.
    ///
    /// `content` holds the full stream `stream`. The check needs no compression, a size equal
    /// to [`InjectedSource::file_size`] and a CRC equal to [`InjectedSource::crc`].
    ///
    /// # Errors
    ///
    /// Errors have the stream ID and logical offset 0.
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the entry uses compression or holds a
    ///   .NET document record.
    /// - [`ErrorKind::InvalidFormat`] if the size or the CRC differs.
    pub fn verify_content<'c>(&self, content: &'c [u8], stream: StreamId) -> Result<&'c [u8]> {
        if self.compression != SourceCompression::None {
            return Err(stream_error(
                ErrorKind::UnsupportedVersion,
                "injected source compression is not supported",
                stream,
            ));
        }
        self.verify_stream(content, stream)?;
        Ok(content)
    }

    /// Check the stream of a .NET document entry and decode its record.
    ///
    /// `content` holds the full stream `stream`. The entry must have the compression
    /// [`SourceCompression::DotNet`]. The entry file size and CRC apply to the stream bytes.
    /// Decode the embedded source with [`EmbeddedSource::decode`](crate::portable::embedded::EmbeddedSource::decode)
    /// and check it with [`DotNetDocument::verify_checksum`].
    ///
    /// # Errors
    ///
    /// Errors have the stream ID and logical offset 0.
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the entry has another compression.
    /// - [`ErrorKind::InvalidFormat`] if the size or the CRC differs.
    /// - The errors of [`DotNetDocument::parse`].
    pub fn verify_document<'c>(
        &self,
        content: &'c [u8],
        stream: StreamId,
    ) -> Result<DotNetDocument<'c>> {
        if self.compression != SourceCompression::DotNet {
            return Err(stream_error(
                ErrorKind::UnsupportedVersion,
                "injected source does not hold a .NET document record",
                stream,
            ));
        }
        self.verify_stream(content, stream)?;
        DotNetDocument::parse(content).map_err(|error| error.with_stream(stream.0, 0))
    }

    /// Check the size and the CRC of the stream bytes.
    fn verify_stream(&self, content: &[u8], stream: StreamId) -> Result<()> {
        if u64::try_from(content.len()).ok() != Some(u64::from(self.file_size)) {
            return Err(stream_error(
                ErrorKind::InvalidFormat,
                "injected source size differs from its entry",
                stream,
            ));
        }
        if hash_buffer_v8(content) != self.crc {
            return Err(stream_error(
                ErrorKind::InvalidFormat,
                "injected source CRC differs from its entry",
                stream,
            ));
        }
        Ok(())
    }
}

/// Return an error at logical offset 0 of `stream`.
fn stream_error(kind: ErrorKind, context: &'static str, stream: StreamId) -> Error {
    Error::new(kind, None)
        .with_context(context)
        .with_stream(stream.0, 0)
}

/// The header of the `/src/headerblock` stream (`SrcHeaderBlock`).
///
/// The version ([`SOURCE_VERSION_ONE`]) and the zero padding are checked and not stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceBlockHeader {
    /// The size of the stream in bytes.
    pub size: u32,
    /// The time stamp in Windows `FILETIME` format.
    pub file_time: u64,
    /// The age of the PDB at the last change of the table.
    pub age: u32,
}

/// A decoded `/src/headerblock` stream, borrowed from the stream bytes.
#[derive(Debug)]
pub struct SourceHeaderBlock<'a> {
    header: SourceBlockHeader,
    table: HashTable<'a, InjectedSource>,
}

impl<'a> SourceHeaderBlock<'a> {
    /// Decode the `/src/headerblock` stream `stream`.
    ///
    /// Returns `Ok(None)` for an empty stream. MSVC `link.exe` writes an empty stream when
    /// the PDB has no injected sources.
    ///
    /// # Errors
    ///
    /// Errors have the stream ID and a logical offset, and no physical offset.
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the header or an entry has another version.
    /// - [`ErrorKind::InvalidFormat`] if the header size differs from the stream size, the
    ///   padding is not zero, a table rule fails, a key differs from the virtual file name
    ///   offset of its entry, an entry is not in its probe sequence or bytes follow the table.
    /// - [`ErrorKind::UnexpectedEof`] if a non-empty stream ends early.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn parse(bytes: &'a [u8], stream: StreamId) -> Result<Option<Self>> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let mut cursor = Cursor::new(bytes, stream, "injected source header");
        let version = cursor.u32()?;
        if version != SOURCE_VERSION_ONE {
            return Err(cursor.fail(
                ErrorKind::UnsupportedVersion,
                0,
                "injected source header version",
            ));
        }
        let size = cursor.u32()?;
        let file_time = cursor.u64()?;
        let age = cursor.u32()?;
        let padding_at = cursor.position();
        if cursor.take(HEADER_PADDING)?.iter().any(|&byte| byte != 0) {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                padding_at,
                "injected source header padding is not zero",
            ));
        }
        debug_assert_eq!(cursor.position(), HEADER_SIZE);
        if u64::try_from(bytes.len()).ok() != Some(u64::from(size)) {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                4,
                "injected source header size differs from the stream size",
            ));
        }
        let table = HashTable::<InjectedSource>::read(&mut cursor)?;
        cursor.finish()?;
        if let Some(index) = table
            .entries()
            .iter()
            .position(|entry| entry.key != entry.value.virtual_file_name_offset)
        {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                table.key_offset(index),
                "injected source key differs from the virtual file name offset",
            ));
        }
        table.check_placement(|entry| Ok(entry.key & 0xffff))?;
        Ok(Some(Self {
            header: SourceBlockHeader {
                size,
                file_time,
                age,
            },
            table,
        }))
    }

    /// Return the header.
    #[must_use]
    pub const fn header(&self) -> SourceBlockHeader {
        self.header
    }

    /// Return the table entries in bucket order.
    #[must_use]
    pub fn entries(&self) -> &[HashTableEntry<InjectedSource>] {
        self.table.entries()
    }

    /// Return the hash table.
    #[must_use]
    pub const fn table(&self) -> &HashTable<'a, InjectedSource> {
        &self.table
    }

    /// Append the serialized stream to `out`.
    ///
    /// The output keeps the header and the hash table, so a decoded stream gives the same
    /// bytes again.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        encode_header(self.header, out)?;
        self.table.encode(out)
    }
}

/// Append the 64-byte `/src/headerblock` header.
fn encode_header(header: SourceBlockHeader, out: &mut Vec<u8>) -> Result<()> {
    let mut output = Output::new(out);
    output.reserve(HEADER_SIZE)?;
    output.u32(SOURCE_VERSION_ONE)?;
    output.u32(header.size)?;
    output.u64(header.file_time)?;
    output.u32(header.age)?;
    output.zeros(HEADER_PADDING)
}

/// Return the named-stream name of the contents of a file with `virtual_file_name`.
///
/// The name is [`SOURCE_FILE_STREAM_PREFIX`] followed by the exact name bytes.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
pub fn content_stream_name(virtual_file_name: &[u8]) -> Result<Vec<u8>> {
    let mut name = Vec::new();
    name.try_reserve_exact(SOURCE_FILE_STREAM_PREFIX.len() + virtual_file_name.len())
        .map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("injected source stream name allocation failed")
        })?;
    name.extend_from_slice(SOURCE_FILE_STREAM_PREFIX);
    name.extend_from_slice(virtual_file_name);
    Ok(name)
}

/// Return an entry error without a location. The table adds the stream and the offset.
fn entry_error(kind: ErrorKind, context: &'static str) -> Error {
    Error::new(kind, None).with_context(context)
}
