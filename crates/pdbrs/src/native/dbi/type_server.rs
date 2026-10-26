//! The type server map substream of the DBI stream.
//!
//! The layout is `struct TSM` in Microsoft `PDB/dbi/dbi.h`.
//! `DBI1::AddTypeServer` in `PDB/dbi/dbi.cpp` appends one record for each
//! type server PDB that a `/Zi` object file refers to with `LF_TYPESERVER`.
//! Each record has these fields:
//!
//! 1. `reserved`: a `u32` that Microsoft writers use for a pointer.
//! 2. `tiBase`: the first type index of the server.
//! 3. `signature` and `age`: the identity of the type server PDB.
//! 4. The NUL-terminated name and the NUL-terminated path of the PDB.
//! 5. Zero padding to the next 4-byte boundary (`pbAlign`).
//!
//! The `iTSM` field in the flags of a module info record selects a record.
//! Linkers that merge types into the PDB write an empty map.

use alloc::vec::Vec;

use super::{align4, c_string, copy_bytes};
use crate::native::codeview::index::TypeIndex;
use crate::native::output::{Output, no_memory};
use crate::{Error, ErrorKind, Result};

/// The size of the fixed fields of a type server record.
const FIXED_SIZE: usize = 16;

/// One record of the type server map.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TypeServer {
    /// The `reserved` field. Microsoft writers store a stale pointer here.
    pub reserved: u32,
    /// The first type index of the type server (`tiBase`).
    pub type_index_base: TypeIndex,
    /// The signature of the type server PDB.
    pub signature: u32,
    /// The age of the type server PDB.
    pub age: u32,
    /// The name of the type server PDB, without its NUL.
    pub name: Vec<u8>,
    /// The path of the type server PDB, without its NUL.
    pub path: Vec<u8>,
    /// The bytes after the path up to the next 4-byte boundary.
    ///
    /// The record uses only the first `n` bytes, where `n` makes the record
    /// size a multiple of 4. The other bytes are zero.
    pub padding: [u8; 3],
}

impl TypeServer {
    /// Decode a non-empty type server map.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if a record, a name or the padding is
    ///   truncated.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    ///
    /// Errors do not have a stream location. The caller adds it.
    pub fn parse_map(bytes: &[u8]) -> Result<Vec<Self>> {
        let mut servers = Vec::new();
        let mut position = 0;
        while position < bytes.len() {
            let (server, next) = Self::read(bytes, position)?;
            servers.try_reserve(1).map_err(|_| no_memory())?;
            servers.push(server);
            position = next;
        }
        Ok(servers)
    }

    /// Decode the record at `start`; return it and the offset after it.
    fn read(bytes: &[u8], start: usize) -> Result<(Self, usize)> {
        let truncated = |context| Error::new(ErrorKind::UnexpectedEof, None).with_context(context);
        let rest = &bytes[start..];
        let fixed = rest
            .first_chunk::<FIXED_SIZE>()
            .ok_or_else(|| truncated("DBI type server record is truncated"))?;
        let value = |index: usize| {
            let mut word = [0; 4];
            word.copy_from_slice(&fixed[index * 4..index * 4 + 4]);
            u32::from_le_bytes(word)
        };
        let (name, after_name) = c_string(rest, FIXED_SIZE)
            .ok_or_else(|| truncated("DBI type server name has no terminating NUL"))?;
        let (path, after_path) = c_string(rest, after_name)
            .ok_or_else(|| truncated("DBI type server path has no terminating NUL"))?;
        // The record starts at a 4-byte boundary, so the record length aligns the end.
        let end = align4(after_path)
            .filter(|&end| end <= rest.len())
            .ok_or_else(|| truncated("DBI type server padding is truncated"))?;
        let mut padding = [0; 3];
        padding[..end - after_path].copy_from_slice(&rest[after_path..end]);
        let server = Self {
            reserved: value(0),
            type_index_base: TypeIndex(value(1)),
            signature: value(2),
            age: value(3),
            name: copy_bytes(name)?,
            path: copy_bytes(path)?,
            padding,
        };
        Ok((server, start + end))
    }

    /// Append the record and its padding to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the name or the path contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let end = FIXED_SIZE + self.name.len() + 1 + self.path.len() + 1;
        let padding = end.next_multiple_of(4) - end;
        let mut output = Output::new(out);
        output.reserve(end + padding)?;
        output.u32(self.reserved)?;
        output.u32(self.type_index_base.0)?;
        output.u32(self.signature)?;
        output.u32(self.age)?;
        output.c_string(&self.name)?;
        output.c_string(&self.path)?;
        output.bytes(&self.padding[..padding])
    }
}
