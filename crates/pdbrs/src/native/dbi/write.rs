//! The encoder of a decoded DBI stream.
//!
//! [`Dbi::encode`] decodes each substream and encodes it again with the
//! encoder of its records. The output of a decoded stream is the same as the
//! input, because the decoders keep all raw values and padding bytes.

use alloc::vec::Vec;

use super::debug_header::encode_debug_streams;
use super::{Dbi, DbiHeader, FIXED_SIZE, SUBSTREAM_COUNT, Substream};
use crate::Result;
use crate::native::output::{Output, no_memory};

impl Dbi {
    /// Append the complete DBI stream to `out`.
    ///
    /// The function writes the header and the substreams in disk order. It
    /// computes the substream sizes in the header from the encoded substreams.
    /// If it returns an error, it removes the bytes that it appended.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Dbi::encode_substream`] and of [`DbiHeader::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        encode_stream(&self.header, out, |kind, out| {
            self.encode_substream(kind, out)
        })
    }

    /// Decode one substream and append its encoded form to `out`.
    ///
    /// An empty substream gives no bytes.
    ///
    /// # Errors
    ///
    /// Returns the errors of the decoder of the substream and the errors of
    /// the encoders of its records.
    pub fn encode_substream(&self, kind: Substream, out: &mut Vec<u8>) -> Result<()> {
        match kind {
            Substream::ModuleInfo => {
                for module in self.modules() {
                    module?.1.encode(out)?;
                }
            }
            Substream::SectionContributions => {
                if let Some(contributions) = self.section_contributions()? {
                    contributions.encode(out)?;
                }
            }
            Substream::SectionMap => {
                if let Some(map) = self.section_map()? {
                    map.encode(out)?;
                }
            }
            Substream::SourceInfo => {
                if let Some(info) = self.source_info()? {
                    info.encode(out)?;
                }
            }
            Substream::TypeServerMap => {
                for server in self.type_server_map()? {
                    server.encode(out)?;
                }
            }
            Substream::Ec => {
                if let Some(names) = self.ec_names()? {
                    names.encode(out)?;
                }
            }
            Substream::OptionalDebugHeader => encode_debug_streams(self.debug_streams(), out)?,
        }
        Ok(())
    }
}

/// Append a header and the substreams that `substream` encodes to `out`.
///
/// The function calls `substream` once for each kind, in disk order. If an
/// error occurs, it removes the bytes that it appended.
pub(super) fn encode_stream(
    header: &DbiHeader,
    out: &mut Vec<u8>,
    mut substream: impl FnMut(Substream, &mut Vec<u8>) -> Result<()>,
) -> Result<()> {
    let start = out.len();
    let result = (|| {
        Output::new(out).zeros(FIXED_SIZE)?;
        let mut sizes = [0; SUBSTREAM_COUNT];
        for (size, kind) in sizes.iter_mut().zip(Substream::ALL) {
            let before = out.len();
            substream(kind, out)?;
            *size = out.len() - before;
        }
        let mut fixed = Vec::new();
        fixed
            .try_reserve_exact(FIXED_SIZE)
            .map_err(|_| no_memory())?;
        header.encode(&sizes, &mut fixed)?;
        out[start..start + FIXED_SIZE].copy_from_slice(&fixed);
        Ok(())
    })();
    if result.is_err() {
        out.truncate(start);
    }
    result
}
