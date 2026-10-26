//! The `/UDTSRCLINEUNDONE` stream.
//!
//! The stream is an array of `u32` TPI type indices. Microsoft `link.exe` creates it. The
//! public Microsoft PDB sources and LLVM do not define it.
//!
//! The name suggests UDTs whose source line records the linker did not complete. The local
//! Rust samples agree: each index names an `LF_STRUCTURE` record, and no `LF_UDT_SRC_LINE` or
//! `LF_UDT_MOD_SRC_LINE` record in the IPI stream refers to it. This meaning is an observation,
//! not a documented contract. The reader does not check the indices against the TPI stream.

use alloc::vec::Vec;

use crate::native::codeview::index::TypeIndex;
use crate::native::msf::StreamId;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// A decoded `/UDTSRCLINEUNDONE` stream, borrowed from the stream bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdtSourceLineUndone<'a> {
    bytes: &'a [u8],
}

impl<'a> UdtSourceLineUndone<'a> {
    /// Decode the `/UDTSRCLINEUNDONE` stream `stream`. An empty stream has no indices.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] with the stream ID if the size is not a multiple
    /// of 4. The logical offset is the start of the partial index.
    pub fn parse(bytes: &'a [u8], stream: StreamId) -> Result<Self> {
        let partial = bytes.len() % 4;
        if partial != 0 {
            let offset = u64::try_from(bytes.len() - partial).unwrap_or(u64::MAX);
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("/UDTSRCLINEUNDONE size is not a multiple of 4")
                .with_stream(stream.0, offset));
        }
        Ok(Self { bytes })
    }

    /// Return the number of type indices.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / 4
    }

    /// Return `true` if the stream has no type indices.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Iterate the type indices in stream order.
    pub fn type_indices(&self) -> impl ExactSizeIterator<Item = TypeIndex> + 'a {
        self.bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| TypeIndex(u32::from_le_bytes(*word)))
    }

    /// Append the stream bytes to `out`. A decoded stream gives the same bytes again.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).bytes(self.bytes)
    }

    /// Append the stream bytes of `indices` to `out`, in iteration order.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode_indices(
        indices: impl IntoIterator<Item = TypeIndex>,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let mut output = Output::new(out);
        for index in indices {
            output.u32(index.0)?;
        }
        Ok(())
    }
}
