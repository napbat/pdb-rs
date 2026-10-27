//! Raw DEFLATE (RFC 1951) streams without a zlib or gzip wrapper.
//!
//! .NET document records and Portable PDB embedded sources use this format.

use alloc::vec::Vec;

use miniz_oxide::DataFormat;
use miniz_oxide::deflate::CompressionLevel;
use miniz_oxide::deflate::core::{CompressorOxide, TDEFLFlush, TDEFLStatus, compress_to_output};
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress};

use super::{invalid, limit_exceeded, output_buffer};
use crate::{Error, ErrorKind, Result};

/// Decode a complete raw DEFLATE stream into `declared` bytes.
///
/// The function checks `declared` against `limit` before it allocates the
/// output buffer. The stream must end exactly at the end of `input` and must
/// produce exactly `declared` bytes.
///
/// # Errors
///
/// - [`ErrorKind::LimitExceeded`] if `declared` is larger than `limit` or if
///   the allocation fails.
/// - [`ErrorKind::UnexpectedEof`] if `input` ends before the final block ends.
/// - [`ErrorKind::InvalidFormat`] if the stream is corrupt, if it produces fewer
///   or more bytes than `declared`, or if bytes follow the final block.
pub(crate) fn decode(input: &[u8], declared: u64, limit: usize) -> Result<Vec<u8>> {
    let mut output = output_buffer(declared, limit)?;
    let mut decompressor = DecompressorOxide::new();
    let (status, consumed, written) = decompress(
        &mut decompressor,
        input,
        &mut output,
        0,
        TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
    );
    match status {
        TINFLStatus::Done => {}
        TINFLStatus::HasMoreOutput => {
            return Err(invalid(
                "raw DEFLATE stream produces more bytes than the declared size",
            ));
        }
        TINFLStatus::FailedCannotMakeProgress => {
            return Err(Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("raw DEFLATE stream ends before its final block"));
        }
        _ => return Err(invalid("raw DEFLATE stream is corrupt")),
    }
    if written != output.len() {
        return Err(invalid(
            "raw DEFLATE stream produces fewer bytes than the declared size",
        ));
    }
    if consumed != input.len() {
        return Err(invalid("bytes follow the final raw DEFLATE block"));
    }
    Ok(output)
}

/// Encode `input` as one raw DEFLATE stream at the default compression level.
///
/// # Errors
///
/// - [`ErrorKind::LimitExceeded`] if the output allocation fails.
/// - [`ErrorKind::InvalidFormat`] if the encoder does not finish the stream.
pub(crate) fn encode(input: &[u8]) -> Result<Vec<u8>> {
    let mut compressor =
        CompressorOxide::with_format_and_level(DataFormat::Raw, CompressionLevel::DefaultLevel);
    let mut output = Vec::new();
    let mut allocation_failed = false;
    let (status, _) = compress_to_output(&mut compressor, input, TDEFLFlush::Finish, |bytes| {
        if output.try_reserve(bytes.len()).is_err() {
            allocation_failed = true;
            return false;
        }
        output.extend_from_slice(bytes);
        true
    });
    if allocation_failed {
        return Err(limit_exceeded("raw DEFLATE output allocation failed"));
    }
    if status != TDEFLStatus::Done {
        return Err(invalid("raw DEFLATE encoder does not finish the stream"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
