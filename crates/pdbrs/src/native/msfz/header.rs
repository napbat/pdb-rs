//! The 80-byte MSFZ file header and the compression codes.

use crate::format::ContainerKind;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The size of the file header in bytes.
pub(super) const HEADER_SIZE: u64 = 80;

/// The file offset of the `version` field.
const VERSION_OFFSET: u64 = 32;

/// The file offset of the `stream_dir_compression` field.
pub(super) const DIRECTORY_COMPRESSION_OFFSET: u64 = 60;

/// The file offset of the `chunk_table_size` field.
const CHUNK_TABLE_SIZE_OFFSET: u64 = 76;

/// The only supported value of the `version` field.
const VERSION_0: u64 = 0;

/// A compression code of the stream directory or of a chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Compression {
    /// Code 0: the bytes are stored without compression.
    None,
    /// Code 1: zstd frames.
    Zstd,
    /// Code 2: a raw DEFLATE stream.
    Deflate,
}

impl Compression {
    /// Decode a compression code.
    ///
    /// Returns [`ErrorKind::UnsupportedVersion`] at `offset` for an unknown code.
    pub(super) fn from_code(code: u32, offset: Option<u64>) -> Result<Self> {
        match code {
            0 => Ok(Self::None),
            1 => Ok(Self::Zstd),
            2 => Ok(Self::Deflate),
            _ => Err(Error::new(ErrorKind::UnsupportedVersion, offset)
                .with_context("unknown MSFZ compression code")),
        }
    }
}

/// The fields of the file header.
#[derive(Clone, Copy, Debug)]
pub(super) struct Header {
    /// The file offset of the stream directory.
    pub(super) directory_offset: u64,
    /// The file offset of the Chunk Table.
    pub(super) chunk_table_offset: u64,
    /// The number of streams, at least 1.
    pub(super) stream_count: u32,
    /// The compression of the stream directory.
    pub(super) directory_compression: Compression,
    /// The size of the stream directory in the file.
    pub(super) directory_size_compressed: u32,
    /// The size of the decompressed stream directory.
    pub(super) directory_size: u32,
    /// The number of chunks.
    pub(super) chunk_count: u32,
    /// The size of the Chunk Table in bytes.
    pub(super) chunk_table_size: u32,
}

impl Header {
    /// Read and check the header of `source`.
    ///
    /// The caller has checked the signature.
    pub(super) fn read<S: ReadAt + ?Sized>(source: &S) -> Result<Self> {
        let length = source.len()?;
        if length < HEADER_SIZE {
            return Err(Error::new(ErrorKind::UnexpectedEof, Some(length))
                .with_context("MSFZ header is incomplete"));
        }
        let mut bytes = [0_u8; 80];
        source.read_exact_at(0, &mut bytes)?;
        debug_assert!(bytes.starts_with(ContainerKind::NativeMsfz.signature()));
        let u32_at = |offset: usize| {
            let mut field = [0; 4];
            field.copy_from_slice(&bytes[offset..offset + 4]);
            u32::from_le_bytes(field)
        };
        let u64_at = |offset: usize| {
            let mut field = [0; 8];
            field.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_le_bytes(field)
        };
        if u64_at(32) != VERSION_0 {
            return Err(
                Error::new(ErrorKind::UnsupportedVersion, Some(VERSION_OFFSET))
                    .with_context("MSFZ version is not 0"),
            );
        }
        let header = Self {
            directory_offset: u64_at(40),
            chunk_table_offset: u64_at(48),
            stream_count: u32_at(56),
            directory_compression: Compression::from_code(
                u32_at(60),
                Some(DIRECTORY_COMPRESSION_OFFSET),
            )?,
            directory_size_compressed: u32_at(64),
            directory_size: u32_at(68),
            chunk_count: u32_at(72),
            chunk_table_size: u32_at(76),
        };
        if header.stream_count == 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, Some(56))
                .with_context("MSFZ stream count is 0"));
        }
        if header.directory_compression == Compression::None
            && header.directory_size_compressed != header.directory_size
        {
            return Err(Error::new(ErrorKind::InvalidFormat, Some(64))
                .with_context("uncompressed MSFZ stream directory has two different sizes"));
        }
        if u64::from(header.chunk_count) * super::CHUNK_ENTRY_SIZE
            != u64::from(header.chunk_table_size)
        {
            return Err(
                Error::new(ErrorKind::InvalidFormat, Some(CHUNK_TABLE_SIZE_OFFSET))
                    .with_context("MSFZ Chunk Table size is not 20 bytes for each chunk"),
            );
        }
        Ok(header)
    }
}
