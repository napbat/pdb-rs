//! The `/TMCache` stream.
//!
//! Microsoft `link.exe` creates this stream. The public Microsoft PDB sources and LLVM do not
//! define its layout, and LLVM does not create the stream. A reverse-engineered description
//! (`PascalBeyer/PDB-Documentation`) links it to the type map cache of incremental links, but
//! marks fields as uncertain. This reader accepts an empty stream and reports other contents as
//! unsupported.

use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// A decoded `/TMCache` stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TmCache {
    /// The stream is empty.
    Empty,
}

impl TmCache {
    /// Decode the `/TMCache` stream `stream`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnsupportedVersion`] with the stream ID and logical offset 0 if the
    /// stream is not empty.
    pub fn parse(bytes: &[u8], stream: StreamId) -> Result<Self> {
        if bytes.is_empty() {
            Ok(Self::Empty)
        } else {
            Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("/TMCache contents have no documented layout")
                .with_stream(stream.0, 0))
        }
    }
}
