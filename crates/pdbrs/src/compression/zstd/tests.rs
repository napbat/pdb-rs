//! Tests for zstd decoding and encoding.
//!
//! The reference CLI `zstd` v1.5.7 wrote the frames with `zstd -q -19 -c`
//! and the options in the comment of each frame. Its input was a file, so
//! `--no-content-size` is necessary to omit the content size field.

use alloc::vec::Vec;

use super::{decode, encode};
use crate::ErrorKind;
use crate::test_support::hex;

/// A limit that does not restrict the test frames.
const LIMIT: usize = 1 << 20;

/// The input of the `TEXT_*` frames.
const TEXT: &[u8] = b"Zstandard frames hold blocks. Each block is raw, RLE or compressed. \
Zstandard frames hold blocks. The frame can hold a content checksum.";

/// `--check`: content size and checksum.
const TEXT_SIZE_CHECK: &str = "28b52ffd2488b502008285111290cf01904f3b8db6486c0b6bb3383db93d07e1\
    59da3aad7e85a9c2d867c8fdb4ca034a94a6295feccf7831c8c1e579cabe9ac5036d1ddd3972719da6c7531d4d\
    46902b350104003e07d9190607164e80cd276ca29e6d";

/// `--no-check`: content size, no checksum.
const TEXT_SIZE: &str = "28b52ffd2088b502008285111290cf01904f3b8db6486c0b6bb3383db93d07e1\
    59da3aad7e85a9c2d867c8fdb4ca034a94a6295feccf7831c8c1e579cabe9ac5036d1ddd3972719da6c7531d4d\
    46902b350104003e07d9190607164e80cd27";

/// `--check --no-content-size`: checksum, no content size.
const TEXT_CHECK: &str = "28b52ffd0400b502008285111290cf01904f3b8db6486c0b6bb3383db93d07e1\
    59da3aad7e85a9c2d867c8fdb4ca034a94a6295feccf7831c8c1e579cabe9ac5036d1ddd3972719da6c7531d4d\
    46902b350104003e07d9190607164e80cd276ca29e6d";

/// `--no-check --no-content-size`: no content size, no checksum.
const TEXT_PLAIN: &str = "28b52ffd0000b502008285111290cf01904f3b8db6486c0b6bb3383db93d07e1\
    59da3aad7e85a9c2d867c8fdb4ca034a94a6295feccf7831c8c1e579cabe9ac5036d1ddd3972719da6c7531d4d\
    46902b350104003e07d9190607164e80cd27";

/// Empty input, `--check`.
const EMPTY_SIZE_CHECK: &str = "28b52ffd240001000099e9d851";

/// Empty input, `--no-check --no-content-size`.
const EMPTY_PLAIN: &str = "28b52ffd0000010000";

/// 300 000 bytes of repeated "pdbrs zstd ", `--check`. The frame has three
/// blocks.
const LARGE_SIZE_CHECK: &str = "28b52ffda4e09304009c0000587064627273207a737464200100f2ffce8b1744\
    0000000100fdffce0b10450000000100dd131d000120758788";

/// 300 000 bytes of repeated "pdbrs zstd ", `--no-check --no-content-size`.
const LARGE_PLAIN: &str = "28b52ffd00489c0000587064627273207a737464200100f2ffce8b1744\
    0000000100fdffce0b10450000000100dd131d0001";

/// Return the input of the `LARGE_*` frames.
fn large() -> Vec<u8> {
    let pattern = b"pdbrs zstd ";
    (0..300_000)
        .map(|index| pattern[index % pattern.len()])
        .collect()
}

/// Return the declared size of `data`.
fn size(data: &[u8]) -> u64 {
    u64::try_from(data.len()).unwrap()
}

/// Decode `input`, expect an error and return its kind and context.
fn decode_error(input: &[u8], declared: u64, limit: usize) -> (ErrorKind, &'static str) {
    let error = decode(input, declared, limit).unwrap_err();
    (error.kind(), error.context().unwrap())
}

/// Return the expected error with `context` and [`ErrorKind::InvalidFormat`].
fn invalid(context: &'static str) -> (ErrorKind, &'static str) {
    (ErrorKind::InvalidFormat, context)
}

#[test]
fn decodes_reference_frames() {
    for frame in [TEXT_SIZE_CHECK, TEXT_SIZE, TEXT_CHECK, TEXT_PLAIN] {
        assert_eq!(decode(&hex(frame), size(TEXT), LIMIT).unwrap(), TEXT);
    }
    for frame in [EMPTY_SIZE_CHECK, EMPTY_PLAIN] {
        assert_eq!(decode(&hex(frame), 0, 0).unwrap(), b"");
    }
    let large = large();
    for frame in [LARGE_SIZE_CHECK, LARGE_PLAIN] {
        assert_eq!(decode(&hex(frame), size(&large), LIMIT).unwrap(), large);
    }
}

#[test]
fn decodes_with_the_limit_equal_to_the_declared_size() {
    // The frame has no content size, so its window is the 1 KiB minimum.
    let decoded = decode(&hex(TEXT_PLAIN), size(TEXT), TEXT.len()).unwrap();
    assert_eq!(decoded, TEXT);
}

#[test]
fn decodes_concatenated_and_skippable_frames() {
    let mut input = hex(TEXT_SIZE_CHECK);
    // A skippable frame: magic 0x184D2A50, length 3, 3 bytes of user data.
    input.extend_from_slice(&[0x50, 0x2A, 0x4D, 0x18, 3, 0, 0, 0, 1, 2, 3]);
    input.extend_from_slice(&hex(TEXT_PLAIN));
    let mut expected = TEXT.to_vec();
    expected.extend_from_slice(TEXT);
    assert_eq!(decode(&input, size(&expected), LIMIT).unwrap(), expected);
}

#[test]
fn round_trips_encoded_data() {
    let large = large();
    let inputs: [&[u8]; 4] = [b"", b"x", TEXT, &large];
    for input in inputs {
        let frame = encode(input).unwrap();
        assert_eq!(decode(&frame, size(input), LIMIT).unwrap(), input);
    }
}

#[test]
fn encoded_frames_have_a_checksum_and_compress_repeated_data() {
    let large = large();
    let frame = encode(&large).unwrap();
    assert!(frame.len() < large.len() / 10, "{} bytes", frame.len());
    // Bit 2 of the frame header descriptor is the content checksum flag.
    assert_eq!(frame[4] & 0x04, 0x04);
}

#[test]
fn rejects_empty_input() {
    assert_eq!(
        decode_error(b"", 0, LIMIT),
        invalid("zstd data holds no frame")
    );
}

#[test]
fn rejects_short_output() {
    for frame in [TEXT_SIZE_CHECK, TEXT_PLAIN] {
        assert_eq!(
            decode_error(&hex(frame), size(TEXT) + 1, LIMIT),
            invalid("zstd data produces fewer bytes than the declared size")
        );
    }
}

#[test]
fn rejects_extra_output() {
    let long = invalid("zstd data produces more bytes than the declared size");
    for frame in [TEXT_SIZE_CHECK, TEXT_PLAIN] {
        assert_eq!(decode_error(&hex(frame), size(TEXT) - 1, LIMIT), long);
        assert_eq!(decode_error(&hex(frame), 0, LIMIT), long);
    }
    assert_eq!(decode_error(&hex(LARGE_PLAIN), 1000, LIMIT), long);
}

#[test]
fn rejects_unused_input() {
    let trailing = invalid("bytes after a zstd frame do not form a frame");
    for extra in [&[0_u8][..], &[0; 8], b"garbage after the frame"] {
        let mut input = hex(TEXT_SIZE_CHECK);
        input.extend_from_slice(extra);
        assert_eq!(decode_error(&input, size(TEXT), LIMIT), trailing);
    }
}

#[test]
fn rejects_truncated_frame() {
    for frame in [TEXT_SIZE_CHECK, TEXT_PLAIN, LARGE_SIZE_CHECK] {
        let frame = hex(frame);
        for end in [4, 8, frame.len() / 2, frame.len() - 1] {
            let error = decode(&frame[..end], 136, LIMIT).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{end}");
        }
    }
    let frame = hex(TEXT_SIZE_CHECK);
    assert_eq!(
        decode_error(&frame[..frame.len() - 2], size(TEXT), LIMIT),
        invalid("zstd frame is corrupt or truncated")
    );
    assert_eq!(
        decode_error(&frame[..3], size(TEXT), LIMIT),
        invalid("zstd frame header is invalid")
    );
    let skippable = [0x50, 0x2A, 0x4D, 0x18, 9, 0, 0, 0, 1, 2, 3];
    assert_eq!(
        decode_error(&skippable, 0, LIMIT),
        invalid("zstd skippable frame is truncated")
    );
}

#[test]
fn rejects_corrupt_frame() {
    // A reserved block type (3) in the first block header.
    let mut frame = hex(TEXT_PLAIN);
    frame[6] |= 0x06;
    assert_eq!(
        decode_error(&frame, size(TEXT), LIMIT),
        invalid("zstd frame is corrupt or truncated")
    );
    // A wrong magic number.
    let mut frame = hex(TEXT_PLAIN);
    frame[0] ^= 1;
    assert_eq!(
        decode_error(&frame, size(TEXT), LIMIT),
        invalid("zstd frame header is invalid")
    );
}

#[test]
fn rejects_checksum_mismatch() {
    let mut frame = hex(TEXT_SIZE_CHECK);
    let last = frame.len() - 1;
    frame[last] ^= 1;
    assert_eq!(
        decode_error(&frame, size(TEXT), LIMIT),
        invalid("zstd frame checksum does not match")
    );
}

#[test]
fn rejects_content_size_mismatch() {
    let mismatch = invalid("zstd frame content size does not match its decoded size");
    // The single-segment frame stores the content size in 4 bytes at offset 5.
    let large = large();
    let mut frame = hex(LARGE_SIZE_CHECK);
    frame[5] += 1;
    assert_eq!(decode_error(&frame, size(&large), LIMIT), mismatch);
    // The single-segment frame stores the content size in 1 byte at offset 5.
    let mut frame = hex(TEXT_SIZE);
    frame[5] -= 1;
    assert_eq!(decode_error(&frame, size(TEXT), LIMIT), mismatch);
}

#[test]
fn rejects_declared_size_above_the_limit() {
    assert_eq!(
        decode_error(&hex(TEXT_PLAIN), size(TEXT), TEXT.len() - 1),
        (
            ErrorKind::LimitExceeded,
            "declared decoded size is larger than the decode limit"
        )
    );
    assert_eq!(
        decode_error(b"", u64::MAX, usize::MAX).0,
        ErrorKind::LimitExceeded
    );
}
