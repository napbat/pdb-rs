//! An owned `/src/headerblock` stream that grows with the rules of a producer.
//!
//! The table maps the `/names` offset of a virtual file name to an [`InjectedSource`] entry.
//! The hash of a key is the key truncated to 16 bits. [`OwnedHashTable`] applies the insertion
//! rules of [`Producer::Microsoft`] (`link.exe`) or [`Producer::Llvm`] (`lld-link`). The first
//! capacity of the table differs from the default of the producer:
//!
//! - Microsoft `MapNiSrcHdr` (`ms_srcimpl.h`) uses the default `Map()`: 1 bucket.
//! - LLVM `PDBFileBuilder` makes `InjectedSourceTable(2)`: 2 buckets.
//!
//! LLVM writes the file time 0, the age 0 and the object file name offset 1 in each entry.
//! The producers insert the files in the order of the linker options. The virtual file names
//! then have increasing `/names` offsets.
//!
//! The contents of each file go into the named stream
//! [`content_stream_name`](super::content_stream_name) of its virtual file name.

use alloc::vec::Vec;

use super::{
    HEADER_SIZE, InjectedSource, SourceBlockHeader, SourceCompression, SourceHeaderBlock,
    encode_header,
};
use crate::native::Producer;
use crate::native::codeview::index::NameOffset;
use crate::native::hash::hash_buffer_v8;
use crate::native::hash_table::{HashTableKeys, OwnedHashTable};
use crate::native::names::StringTableBuilder;
use crate::native::output::to_u32;
use crate::{Error, ErrorKind, Result};

/// The hash of a `/src/headerblock` key: the key truncated to 16 bits.
#[derive(Clone, Copy, Debug)]
struct SourceKeys;

impl HashTableKeys for SourceKeys {
    fn hash(&self, key: u32) -> Result<u32> {
        Ok(key & 0xffff)
    }
}

/// The names and the stream bytes of a file to inject.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InjectedFile<'a> {
    /// The original file name.
    pub file_name: &'a [u8],
    /// The object file name.
    pub object_name: &'a [u8],
    /// The virtual file name. `link.exe` and `lld-link` use the lowercase file name.
    pub virtual_file_name: &'a [u8],
    /// [`SourceCompression::NONE`] for file contents, or [`SourceCompression::DOT_NET`] for a
    /// [`DotNetDocument`](super::DotNetDocument) record.
    pub compression: SourceCompression,
    /// The `fVirtual` flag.
    pub is_virtual: bool,
    /// The bytes of the content stream.
    pub stream: &'a [u8],
}

/// An owned `/src/headerblock` stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceHeaderBlockBuilder {
    /// The file time of the header.
    file_time: u64,
    /// The age of the header.
    age: u32,
    /// The table from the virtual file name offset to the entry.
    table: OwnedHashTable<InjectedSource>,
}

impl SourceHeaderBlockBuilder {
    /// Create an empty table with the first capacity that `producer` uses for this table.
    ///
    /// `file_time` is a Windows `FILETIME` value. `age` is the PDB age.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn new(producer: Producer, file_time: u64, age: u32) -> Result<Self> {
        let capacity = match producer {
            Producer::Microsoft => 1,
            Producer::Llvm => 2,
        };
        Ok(Self {
            file_time,
            age,
            table: OwnedHashTable::with_capacity(producer, capacity)?,
        })
    }

    /// Copy a decoded stream. The copy keeps the bucket of each entry and the bit vectors.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn from_block(block: &SourceHeaderBlock<'_>, producer: Producer) -> Result<Self> {
        let header = block.header();
        Ok(Self {
            file_time: header.file_time,
            age: header.age,
            table: OwnedHashTable::from_table(block.table(), producer)?,
        })
    }

    /// Set the file time and the age of the header.
    pub const fn set_header(&mut self, file_time: u64, age: u32) {
        self.file_time = file_time;
        self.age = age;
    }

    /// Return the table.
    #[must_use]
    pub const fn table(&self) -> &OwnedHashTable<InjectedSource> {
        &self.table
    }

    /// Insert an entry with the `/names` offsets of the caller.
    ///
    /// The key is [`InjectedSource::virtual_file_name_offset`]. Return the previous entry of
    /// the key.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the table cannot grow.
    pub fn insert(&mut self, source: InjectedSource) -> Result<Option<InjectedSource>> {
        self.table
            .insert(source.virtual_file_name_offset.0, source, &SourceKeys)
    }

    /// Remove the entry of the virtual file name offset `key` and return it.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn remove(&mut self, key: NameOffset) -> Result<Option<InjectedSource>> {
        self.table.remove(key.0, &SourceKeys)
    }

    /// Add the names of `file` to `names`, then insert its entry and return the entry.
    ///
    /// The function adds the names to `names` in this order: the original file name, the
    /// virtual file name and the object file name. LLVM `addInjectedSource` uses the same
    /// order for the first two names. The CRC is [`hash_buffer_v8`] of the stream bytes, and
    /// the file size is the stream size. Put the stream bytes into the named stream
    /// [`content_stream_name`](super::content_stream_name) of the virtual file name.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the compression is not
    ///   [`SourceCompression::NONE`] or [`SourceCompression::DOT_NET`], or the virtual file name
    ///   is empty.
    /// - [`ErrorKind::LimitExceeded`] if the stream size does not fit in `u32`, or a table
    ///   cannot grow.
    /// - The errors of [`StringTableBuilder::insert`].
    pub fn add_file(
        &mut self,
        names: &mut StringTableBuilder,
        file: &InjectedFile<'_>,
    ) -> Result<InjectedSource> {
        if file.compression != SourceCompression::NONE
            && file.compression != SourceCompression::DOT_NET
        {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("injected source compression is not None or DotNet"));
        }
        if file.virtual_file_name.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("injected source virtual file name is empty"));
        }
        let file_size = to_u32(file.stream.len(), "injected source size")?;
        let file_name_offset = names.insert(file.file_name)?;
        let virtual_file_name_offset = names.insert(file.virtual_file_name)?;
        let object_name_offset = names.insert(file.object_name)?;
        let source = InjectedSource {
            crc: hash_buffer_v8(file.stream),
            file_size,
            file_name_offset,
            object_name_offset,
            virtual_file_name_offset,
            compression: file.compression,
            is_virtual: file.is_virtual,
            reserved: [0; 8],
        };
        self.insert(source)?;
        Ok(source)
    }

    /// Append the stream bytes to `out`.
    ///
    /// The header size field gets the size of the stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the stream size does not fit in `u32` or the
    /// output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut table = Vec::new();
        self.table.encode(&mut table)?;
        let size = to_u32(
            table.len().saturating_add(HEADER_SIZE),
            "injected source stream size",
        )?;
        let header = SourceBlockHeader {
            size,
            file_time: self.file_time,
            age: self.age,
        };
        encode_header(header, out)?;
        out.try_reserve(table.len())
            .map_err(|_| crate::native::output::no_memory())?;
        out.extend_from_slice(&table);
        Ok(())
    }
}
