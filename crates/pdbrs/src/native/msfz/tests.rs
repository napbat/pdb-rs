//! Tests of the MSFZ reader with synthetic containers.
//!
//! [`Image`] builds a container: the header, the stored chunks, the uncompressed fragments, the
//! stream directory and the Chunk Table, in this file order. A test can change any field after
//! the build to make a defect.

mod reads;
mod validation;

use alloc::vec;
use alloc::vec::Vec;

use super::{Msfz, MsfzLimits, StreamId};
use crate::ContainerKind;
use crate::compression::{deflate, zstd};
use crate::{Error, ErrorKind, Result};

/// Compression code 0: stored.
const STORED: u32 = 0;
/// Compression code 1: zstd.
const ZSTD: u32 = 1;
/// Compression code 2: raw DEFLATE.
const DEFLATE: u32 = 2;

/// The file offset of the `stream_dir_offset` header field.
const DIRECTORY_OFFSET_FIELD: usize = 40;
/// The file offset of the `chunk_table_offset` header field.
const CHUNK_TABLE_OFFSET_FIELD: usize = 48;
/// The file offset of the `stream_dir_compression` header field.
const DIRECTORY_COMPRESSION_FIELD: usize = 60;
/// The file offset of the `stream_dir_size_compressed` header field.
const DIRECTORY_STORED_FIELD: usize = 64;
/// The file offset of the `stream_dir_size_uncompressed` header field.
const DIRECTORY_SIZE_FIELD: usize = 68;
/// The file offset of the `num_chunks` header field.
const CHUNK_COUNT_FIELD: usize = 72;
/// The file offset of the `chunk_table_size` header field.
const CHUNK_TABLE_SIZE_FIELD: usize = 76;

/// Bit 63 of a fragment location.
const COMPRESSED: u64 = 1 << 63;

/// A fragment of a synthetic stream.
#[derive(Clone, Debug)]
enum Part {
    /// Uncompressed bytes. The builder writes them to the file.
    File(Vec<u8>),
    /// `size` bytes at `offset` of chunk `chunk`.
    Chunk { chunk: u32, offset: u32, size: u32 },
    /// A record with any size and location value. The builder writes no bytes.
    Raw { size: u32, location: u64 },
}

/// A synthetic stream.
#[derive(Clone, Debug)]
enum Spec {
    /// A nil stream.
    Nil,
    /// A stream of fragments. An empty list is an empty stream.
    Parts(Vec<Part>),
}

/// A synthetic container and the positions of its parts.
#[derive(Clone, Debug)]
struct Image {
    bytes: Vec<u8>,
    /// The file offset of each chunk.
    chunks: Vec<u64>,
    /// The file offset of each uncompressed fragment, in directory order.
    files: Vec<u64>,
    /// The file offset of the stream directory.
    directory: usize,
    /// The file offset of the Chunk Table.
    table: usize,
}

/// Return `count` bytes of a pattern that starts with `seed`.
fn pattern(seed: u8, count: usize) -> Vec<u8> {
    (0..count)
        .map(|index| seed.wrapping_add(u8::try_from(index % 251).unwrap()))
        .collect()
}

/// Compress `data` with the compression code `code`.
fn compress(code: u32, data: &[u8]) -> Vec<u8> {
    match code {
        STORED => data.to_vec(),
        ZSTD => zstd::encode(data).unwrap(),
        DEFLATE => deflate::encode(data).unwrap(),
        _ => panic!("unknown code {code}"),
    }
}

/// Encode the stream directory of `streams`.
///
/// `file_offsets` gives the file offset of each [`Part::File`] in order.
fn encode_directory(streams: &[Spec], file_offsets: &[u64]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut files = file_offsets.iter();
    for stream in streams {
        let Spec::Parts(parts) = stream else {
            bytes.extend_from_slice(&u32::MAX.to_le_bytes());
            continue;
        };
        for part in parts {
            let (size, location) = match part {
                Part::File(data) => (u32::try_from(data.len()).unwrap(), *files.next().unwrap()),
                Part::Chunk {
                    chunk,
                    offset,
                    size,
                } => (
                    *size,
                    COMPRESSED | (u64::from(*chunk) << 32) | u64::from(*offset),
                ),
                Part::Raw { size, location } => (*size, *location),
            };
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.extend_from_slice(&location.to_le_bytes());
        }
        bytes.extend_from_slice(&0_u32.to_le_bytes());
    }
    bytes
}

impl Image {
    /// Build a container with stored directory and the chunks `chunks` (code, data).
    fn new(chunks: &[(u32, &[u8])], streams: &[Spec]) -> Self {
        Self::with_directory(chunks, streams, STORED)
    }

    /// Build a container whose stream directory has the compression code `directory_code`.
    fn with_directory(chunks: &[(u32, &[u8])], streams: &[Spec], directory_code: u32) -> Self {
        let mut bytes = vec![0_u8; 80];
        bytes[..32].copy_from_slice(ContainerKind::NativeMsfz.signature());
        let mut table = Vec::new();
        let mut chunk_offsets = Vec::new();
        for (code, data) in chunks {
            let stored = compress(*code, data);
            let offset = u64::try_from(bytes.len()).unwrap();
            chunk_offsets.push(offset);
            table.extend_from_slice(&offset.to_le_bytes());
            table.extend_from_slice(&code.to_le_bytes());
            table.extend_from_slice(&u32::try_from(stored.len()).unwrap().to_le_bytes());
            table.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
            bytes.extend_from_slice(&stored);
        }
        let mut files = Vec::new();
        for stream in streams {
            if let Spec::Parts(parts) = stream {
                for part in parts {
                    if let Part::File(data) = part {
                        files.push(u64::try_from(bytes.len()).unwrap());
                        bytes.extend_from_slice(data);
                    }
                }
            }
        }
        let directory = encode_directory(streams, &files);
        let stored_directory = compress(directory_code, &directory);
        let directory_offset = bytes.len();
        bytes.extend_from_slice(&stored_directory);
        let table_offset = bytes.len();
        bytes.extend_from_slice(&table);
        let mut image = Self {
            bytes,
            chunks: chunk_offsets,
            files,
            directory: directory_offset,
            table: table_offset,
        };
        image.set_u64(
            DIRECTORY_OFFSET_FIELD,
            u64::try_from(directory_offset).unwrap(),
        );
        image.set_u64(
            CHUNK_TABLE_OFFSET_FIELD,
            u64::try_from(table_offset).unwrap(),
        );
        image.set_u32(56, u32::try_from(streams.len()).unwrap());
        image.set_u32(DIRECTORY_COMPRESSION_FIELD, directory_code);
        image.set_u32(
            DIRECTORY_STORED_FIELD,
            u32::try_from(stored_directory.len()).unwrap(),
        );
        image.set_u32(
            DIRECTORY_SIZE_FIELD,
            u32::try_from(directory.len()).unwrap(),
        );
        image.set_u32(CHUNK_COUNT_FIELD, u32::try_from(chunks.len()).unwrap());
        image.set_u32(CHUNK_TABLE_SIZE_FIELD, u32::try_from(table.len()).unwrap());
        image
    }

    /// Write a `u32` value at `offset`.
    fn set_u32(&mut self, offset: usize, value: u32) {
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Write a `u64` value at `offset`.
    fn set_u64(&mut self, offset: usize, value: u64) {
        self.bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// Return the file offset of field `field` (0 to 16) of Chunk Table entry `chunk`.
    fn entry(&self, chunk: usize, field: usize) -> usize {
        self.table + chunk * 20 + field
    }

    /// Open the image with the default limits.
    fn open(&self) -> Result<Msfz<&[u8]>> {
        Msfz::read(self.bytes.as_slice())
    }

    /// Open the image with `limits`.
    fn open_with(&self, limits: MsfzLimits) -> Result<Msfz<&[u8]>> {
        Msfz::with_limits(self.bytes.as_slice(), limits)
    }
}

/// Read all bytes of stream `id`, or `None` for a nil stream.
fn read(msfz: &Msfz<&[u8]>, id: u32) -> Result<Option<Vec<u8>>> {
    msfz.stream(StreamId(id))?
        .map(|stream| stream.read_all(usize::MAX))
        .transpose()
}

/// Check the kind and the context of an error.
#[track_caller]
fn expect_error<T: core::fmt::Debug>(result: Result<T>, kind: ErrorKind, context: &str) -> Error {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), kind, "{error:?}");
    assert_eq!(error.context(), Some(context), "{error:?}");
    error
}
