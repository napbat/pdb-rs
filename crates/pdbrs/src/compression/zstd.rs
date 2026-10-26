//! zstd frames (RFC 8878) for MSFZ chunks.
//!
//! The decoder accepts one or more frames, which can include skippable frames.
//! It does not support dictionaries.

use alloc::vec::Vec;

use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};
use ruzstd::encoding::{CompressionLevel, FrameCompressor};
use ruzstd::io::{Error as IoError, Read as _, Write};

use super::{invalid, limit_exceeded, output_buffer};
use crate::Result;

/// The magic number at the start of a zstd frame, in file byte order.
const FRAME_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// The smallest window size limit, in bytes.
///
/// A frame without a content size can declare a window larger than its
/// content. The reference encoder uses windows up to 8 MiB at level 19.
const MIN_WINDOW_LIMIT: u64 = 8 << 20;

/// Decode complete zstd data into `declared` bytes.
///
/// The function checks `declared` against `limit` before it allocates the
/// output buffer. It verifies the content size and the checksum of each frame
/// that has them. The frames must end exactly at the end of `input` and must
/// produce exactly `declared` bytes in total.
///
/// The decoder keeps a window of earlier output. It rejects a frame with a
/// window larger than `limit` or 8 MiB, whichever is larger.
///
/// # Errors
///
/// - [`crate::ErrorKind::LimitExceeded`] if `declared` is larger than `limit`
///   or if the allocation fails.
/// - [`crate::ErrorKind::InvalidFormat`] if `input` holds no frame, if a frame
///   is corrupt or truncated, if a content size or a checksum does not match,
///   if the frames produce fewer or more bytes than `declared`, or if bytes
///   after a frame do not form a frame.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "first user: #20 MSFZ chunk decoding")
)]
pub(crate) fn decode(input: &[u8], declared: u64, limit: usize) -> Result<Vec<u8>> {
    let mut output = output_buffer(declared, limit)?;
    if input.is_empty() {
        return Err(invalid("zstd data holds no frame"));
    }
    let mut decoder = FrameDecoder::new();
    let window_limit = u64::try_from(limit)
        .unwrap_or(u64::MAX)
        .max(MIN_WINDOW_LIMIT);
    decoder.set_max_window_size(window_limit);
    let mut rest = input;
    let mut written = 0;
    let mut first = true;
    while !rest.is_empty() {
        let has_content_size = rest.starts_with(&FRAME_MAGIC)
            && rest
                .get(FRAME_MAGIC.len())
                .is_some_and(|&descriptor| descriptor & 0xE0 != 0);
        match decoder.reset(&mut rest) {
            Ok(()) => {}
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                length,
                ..
            })) => {
                rest = usize::try_from(length)
                    .ok()
                    .and_then(|length| rest.get(length..))
                    .ok_or_else(|| invalid("zstd skippable frame is truncated"))?;
                first = false;
                continue;
            }
            Err(_) if first => return Err(invalid("zstd frame header is invalid")),
            Err(_) => return Err(invalid("bytes after a zstd frame do not form a frame")),
        }
        first = false;
        let start = written;
        written += decode_frame(&mut decoder, &mut rest, &mut output[written..])?;
        if has_content_size && u64::try_from(written - start).ok() != Some(decoder.content_size()) {
            return Err(invalid(
                "zstd frame content size does not match its decoded size",
            ));
        }
        if let Some(stored) = decoder.get_checksum_from_data()
            && decoder.get_calculated_checksum() != Some(stored)
        {
            return Err(invalid("zstd frame checksum does not match"));
        }
    }
    if written != output.len() {
        return Err(invalid(
            "zstd data produces fewer bytes than the declared size",
        ));
    }
    Ok(output)
}

/// Decode the blocks of the current frame into `output`.
///
/// Returns the number of bytes written.
fn decode_frame(decoder: &mut FrameDecoder, input: &mut &[u8], output: &mut [u8]) -> Result<usize> {
    let mut written = 0;
    loop {
        decoder
            .decode_blocks(&mut *input, BlockDecodingStrategy::UptoBlocks(1))
            .map_err(|_| invalid("zstd frame is corrupt or truncated"))?;
        written += decoder
            .read(&mut output[written..])
            .map_err(|_| invalid("zstd frame is corrupt or truncated"))?;
        if decoder.can_collect() != 0 {
            return Err(invalid(
                "zstd data produces more bytes than the declared size",
            ));
        }
        if decoder.is_finished() {
            return Ok(written);
        }
    }
}

/// Encode `input` as one zstd frame with a content checksum.
///
/// The encoder uses the "Fastest" level. The frame does not store the content
/// size.
///
/// # Errors
///
/// Returns [`crate::ErrorKind::LimitExceeded`] if the output allocation fails.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "first user: #20 MSFZ chunk encoding")
)]
pub(crate) fn encode(input: &[u8]) -> Result<Vec<u8>> {
    let mut drain = Drain::default();
    let mut compressor = FrameCompressor::new(CompressionLevel::Fastest);
    compressor.set_source(input);
    compressor.set_drain(&mut drain);
    compressor.compress();
    drop(compressor);
    if drain.allocation_failed {
        return Err(limit_exceeded("zstd output allocation failed"));
    }
    Ok(drain.bytes)
}

/// The encoder output, which grows with fallible allocation.
///
/// The encoder panics if a write fails. Thus a failed allocation sets a flag
/// and discards the bytes, and the caller checks the flag.
#[derive(Default)]
struct Drain {
    /// The encoded bytes.
    bytes: Vec<u8>,
    /// Whether an allocation failed.
    allocation_failed: bool,
}

impl Write for Drain {
    fn write(&mut self, buf: &[u8]) -> Result<usize, IoError> {
        if !self.allocation_failed {
            if self.bytes.try_reserve(buf.len()).is_ok() {
                self.bytes.extend_from_slice(buf);
            } else {
                self.allocation_failed = true;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), IoError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
