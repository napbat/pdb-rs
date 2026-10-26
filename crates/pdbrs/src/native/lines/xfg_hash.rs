//! `DEBUG_S_XFGHASH_TYPE` and `DEBUG_S_XFGHASH_VIRTUAL` subsections: the
//! eXtended Flow Guard (XFG) hashes of an object file.
//!
//! `cvinfo.h`, LLVM and `cvdump` name the kinds but do not define the
//! layouts. The layouts come from the object files of the `msvc-x64-xfg`
//! fixture (#27, MSVC 14.44 with `/guard:xfg`), and the decoders check the
//! sizes that they imply (#31):
//!
//! - `DEBUG_S_XFGHASH_TYPE`: entries of 12 bytes, a `u64` hash and the type
//!   index of a function type (`LF_PROCEDURE` or `LF_MFUNCTION`).
//! - `DEBUG_S_XFGHASH_VIRTUAL`: entries of 16 bytes, a `u64` hash, the type
//!   index of a class and a `u32` byte offset (0, 8 and 16 in the fixture).
//!
//! The type indexes are in the type scope of the object file. `link.exe`
//! does not copy these subsections to the PDB.

use alloc::vec::Vec;

use super::{entry_list, invalid};
use crate::Result;
use crate::native::codeview::index::TypeIndex;
use crate::native::output::Output;

/// The size of one `DEBUG_S_XFGHASH_TYPE` entry.
const TYPE_ENTRY_SIZE: usize = 12;

/// The size of one `DEBUG_S_XFGHASH_VIRTUAL` entry.
const VIRTUAL_ENTRY_SIZE: usize = 16;

/// One entry of a `DEBUG_S_XFGHASH_TYPE` subsection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct XfgTypeHash {
    /// The XFG hash.
    pub hash: u64,
    /// The function type.
    pub function_type: TypeIndex,
}

impl XfgTypeHash {
    /// Create an entry from its fields.
    #[must_use]
    pub const fn new(hash: u64, function_type: TypeIndex) -> Self {
        Self {
            hash,
            function_type,
        }
    }

    /// Decode a serialized entry.
    const fn parse(bytes: [u8; TYPE_ENTRY_SIZE]) -> Self {
        let [h0, h1, h2, h3, h4, h5, h6, h7, t0, t1, t2, t3] = bytes;
        Self {
            hash: u64::from_le_bytes([h0, h1, h2, h3, h4, h5, h6, h7]),
            function_type: TypeIndex(u32::from_le_bytes([t0, t1, t2, t3])),
        }
    }

    /// Append the 12-byte entry to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).bytes(&self.hash.to_le_bytes())?;
        Output::new(out).u32(self.function_type.0)
    }
}

/// One entry of a `DEBUG_S_XFGHASH_VIRTUAL` subsection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct XfgVirtualHash {
    /// The XFG hash.
    pub hash: u64,
    /// The class type.
    pub class: TypeIndex,
    /// A byte offset. The fixture has the values 0, 8 and 16.
    pub offset: u32,
}

impl XfgVirtualHash {
    /// Create an entry from its fields.
    #[must_use]
    pub const fn new(hash: u64, class: TypeIndex, offset: u32) -> Self {
        Self {
            hash,
            class,
            offset,
        }
    }

    /// Decode a serialized entry.
    const fn parse(bytes: [u8; VIRTUAL_ENTRY_SIZE]) -> Self {
        let [
            h0,
            h1,
            h2,
            h3,
            h4,
            h5,
            h6,
            h7,
            c0,
            c1,
            c2,
            c3,
            o0,
            o1,
            o2,
            o3,
        ] = bytes;
        Self {
            hash: u64::from_le_bytes([h0, h1, h2, h3, h4, h5, h6, h7]),
            class: TypeIndex(u32::from_le_bytes([c0, c1, c2, c3])),
            offset: u32::from_le_bytes([o0, o1, o2, o3]),
        }
    }

    /// Append the 16-byte entry to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).bytes(&self.hash.to_le_bytes())?;
        Output::new(out).u32(self.class.0)?;
        Output::new(out).u32(self.offset)
    }
}

entry_list!(
    /// A decoded `DEBUG_S_XFGHASH_TYPE` subsection.
    XfgTypeHashes,
    /// An iterator over [`XfgTypeHashes`].
    XfgTypeHashesIter,
    XfgTypeHash,
    TYPE_ENTRY_SIZE,
    XfgTypeHash::parse
);

entry_list!(
    /// A decoded `DEBUG_S_XFGHASH_VIRTUAL` subsection.
    XfgVirtualHashes,
    /// An iterator over [`XfgVirtualHashes`].
    XfgVirtualHashesIter,
    XfgVirtualHash,
    VIRTUAL_ENTRY_SIZE,
    XfgVirtualHash::parse
);

impl<'a> XfgTypeHashes<'a> {
    /// Decode the data of a `DEBUG_S_XFGHASH_TYPE` subsection.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat)
    /// if the size is not a multiple of 12. The error does not have a file
    /// offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        if !data.len().is_multiple_of(TYPE_ENTRY_SIZE) {
            return Err(invalid("XFG type hash size is not a multiple of 12"));
        }
        Ok(Self::new(data))
    }

    /// Append the subsection data to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        for entry in self {
            entry.encode(out)?;
        }
        Ok(())
    }
}

impl<'a> XfgVirtualHashes<'a> {
    /// Decode the data of a `DEBUG_S_XFGHASH_VIRTUAL` subsection.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat)
    /// if the size is not a multiple of 16. The error does not have a file
    /// offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        if !data.len().is_multiple_of(VIRTUAL_ENTRY_SIZE) {
            return Err(invalid("XFG virtual hash size is not a multiple of 16"));
        }
        Ok(Self::new(data))
    }

    /// Append the subsection data to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        for entry in self {
            entry.encode(out)?;
        }
        Ok(())
    }
}
