//! Tests for raw DEFLATE decoding and encoding.
//!
//! The vectors come from two independent encoders:
//!
//! - Python 3.14.7 `zlib` (zlib-ng 1.3.1) with
//!   `zlib.compressobj(level, zlib.DEFLATED, -15, 9, strategy)`.
//! - .NET 10.0.11 `System.IO.Compression.DeflateStream`.
//!
//! The comment of each vector gives the block type, which is bits 1 and 2 of
//! the first byte.

use alloc::vec::Vec;

use super::{decode, encode};
use crate::ErrorKind;
use crate::test_support::hex;

/// A limit that does not restrict the test vectors.
const LIMIT: usize = 1 << 20;

/// An encoded vector and the data it decodes to.
struct Vector {
    /// The decoded data.
    data: &'static [u8],
    /// The raw DEFLATE stream as hexadecimal digits.
    stream: &'static str,
}

/// Python `zlib`, level 0: one stored block.
const PYTHON_STORED: Vector = Vector {
    data: b"pdbrs stored block",
    stream: "011200edff70646272732073746f72656420626c6f636b",
};

/// Python `zlib`, level 9, `Z_FIXED`: one fixed-Huffman block.
const PYTHON_FIXED: Vector = Vector {
    data: b"abcabcabcabc fixed fixed fixed",
    stream: "4b4c4a4e842185b4cc8ad414641200",
};

/// Python `zlib`, level 9: one dynamic-Huffman block.
const PYTHON_DYNAMIC: Vector = Vector {
    data: b"Dynamic Huffman blocks use code lengths that the encoder sends. \
            The decoder builds the tables from these code lengths. \
            zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz eeeeeeeeeeeeeeeeeeee \
            The decoder builds the tables from these code lengths.",
    stream: "9d8dbb0d8030104357f1044c41c1002c908f0388e422714901d3f3eb1005c285\
             253f5976bb8a49934357434846606376b3a22ae1b227226528a3a28ca61c4650\
             4ebe40295e1bf407f2bc91ad53f47ab58ab1918ab0e474e6c75a83eda3c017fd\
             3cdd01",
};

/// Python `zlib`, level 9, empty input: one fixed-Huffman block with only the
/// end-of-block code.
const PYTHON_EMPTY: Vector = Vector {
    data: b"",
    stream: "0300",
};

/// .NET `CompressionLevel.NoCompression`: one stored block.
const DOTNET_STORED: Vector = Vector {
    data: b"dotnet stored block",
    stream: "011300ecff646f746e65742073746f72656420626c6f636b",
};

/// .NET `CompressionLevel.Optimal`: one fixed-Huffman block.
const DOTNET_FIXED: Vector = Vector {
    data: b"dotnet dotnet dotnet fixed",
    stream: "4bc92fc94b2d514841a1d2322b525300",
};

/// .NET `CompressionLevel.SmallestSize`: one dynamic-Huffman block.
const DOTNET_DYNAMIC: Vector = Vector {
    data: b"DeflateStream writes raw DEFLATE data without a zlib header. \
            The Portable PDB embedded source blob uses raw DEFLATE data. \
            qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq xxxxxxxxxxxxxxxxxxxx \
            The Portable PDB embedded source blob uses raw DEFLATE data.",
    stream: "a58dc10dc23010045bd90ad203c8e1c5032969e00e2fb2254756ce6719513d29\
             200f24e635af99c05711e7e246d9302c3b1b4c06c27cbb5fd619515c30b2a7da\
             1d824fc98a4489b4096b221ed55cb41c12aee0a68c9111ad767b125aaaa2b793\
             e484fd47f03ee1aff517",
};

/// All vectors from the independent encoders.
const VECTORS: [Vector; 7] = [
    PYTHON_STORED,
    PYTHON_FIXED,
    PYTHON_DYNAMIC,
    PYTHON_EMPTY,
    DOTNET_STORED,
    DOTNET_FIXED,
    DOTNET_DYNAMIC,
];

/// Return the declared size of `data`.
fn size(data: &[u8]) -> u64 {
    u64::try_from(data.len()).unwrap()
}

/// Return the block type in the first block header of `stream`.
fn block_type(stream: &[u8]) -> u8 {
    (stream[0] >> 1) & 3
}

/// Decode `stream`, expect an error and return its kind and context.
fn decode_error(stream: &[u8], declared: u64, limit: usize) -> (ErrorKind, &'static str) {
    let error = decode(stream, declared, limit).unwrap_err();
    (error.kind(), error.context().unwrap())
}

#[test]
fn decodes_vectors_from_python_and_dotnet() {
    for vector in VECTORS {
        let stream = hex(vector.stream);
        let decoded = decode(&stream, size(vector.data), LIMIT).unwrap();
        assert_eq!(decoded, vector.data);
    }
}

#[test]
fn vectors_cover_all_block_types() {
    let stored = 0;
    let fixed = 1;
    let dynamic = 2;
    for (vector, kind) in [
        (PYTHON_STORED, stored),
        (PYTHON_FIXED, fixed),
        (PYTHON_DYNAMIC, dynamic),
        (PYTHON_EMPTY, fixed),
        (DOTNET_STORED, stored),
        (DOTNET_FIXED, fixed),
        (DOTNET_DYNAMIC, dynamic),
    ] {
        assert_eq!(block_type(&hex(vector.stream)), kind, "{:?}", vector.stream);
    }
}

#[test]
fn decodes_with_the_limit_equal_to_the_declared_size() {
    let stream = hex(PYTHON_DYNAMIC.stream);
    let decoded = decode(
        &stream,
        size(PYTHON_DYNAMIC.data),
        PYTHON_DYNAMIC.data.len(),
    )
    .unwrap();
    assert_eq!(decoded, PYTHON_DYNAMIC.data);
}

#[test]
fn round_trips_encoded_data() {
    let mut large = Vec::new();
    for index in 0_u32..20_000 {
        large.extend_from_slice(&(index % 300).to_le_bytes());
    }
    let inputs: [&[u8]; 4] = [b"", b"x", PYTHON_DYNAMIC.data, &large];
    for input in inputs {
        let stream = encode(input).unwrap();
        assert_eq!(decode(&stream, size(input), LIMIT).unwrap(), input);
    }
}

#[test]
fn encoded_data_is_smaller_for_repeated_data() {
    let input = [b'a'; 4096];
    let stream = encode(&input).unwrap();
    assert!(stream.len() < 64, "{} bytes", stream.len());
}

#[test]
fn decodes_zero_length_data() {
    assert_eq!(decode(&hex(PYTHON_EMPTY.stream), 0, 0).unwrap(), b"");
    assert_eq!(decode(&encode(b"").unwrap(), 0, LIMIT).unwrap(), b"");
}

#[test]
fn rejects_empty_input() {
    // .NET `DeflateStream` writes no bytes for empty input. That is not a
    // complete raw DEFLATE stream.
    assert_eq!(
        decode_error(b"", 0, LIMIT),
        (
            ErrorKind::UnexpectedEof,
            "raw DEFLATE stream ends before its final block"
        )
    );
}

#[test]
fn rejects_short_output() {
    let stream = hex(PYTHON_FIXED.stream);
    assert_eq!(
        decode_error(&stream, size(PYTHON_FIXED.data) + 1, LIMIT),
        (
            ErrorKind::InvalidFormat,
            "raw DEFLATE stream produces fewer bytes than the declared size"
        )
    );
}

#[test]
fn rejects_extra_output() {
    let long = (
        ErrorKind::InvalidFormat,
        "raw DEFLATE stream produces more bytes than the declared size",
    );
    for vector in [PYTHON_STORED, PYTHON_FIXED, PYTHON_DYNAMIC, DOTNET_DYNAMIC] {
        let stream = hex(vector.stream);
        assert_eq!(decode_error(&stream, size(vector.data) - 1, LIMIT), long);
        assert_eq!(decode_error(&stream, 0, LIMIT), long);
    }
}

#[test]
fn rejects_unused_input() {
    let mut stream = hex(DOTNET_FIXED.stream);
    stream.push(0);
    assert_eq!(
        decode_error(&stream, size(DOTNET_FIXED.data), LIMIT),
        (
            ErrorKind::InvalidFormat,
            "bytes follow the final raw DEFLATE block"
        )
    );
}

#[test]
fn rejects_truncated_stream() {
    let truncated = (
        ErrorKind::UnexpectedEof,
        "raw DEFLATE stream ends before its final block",
    );
    for vector in [PYTHON_STORED, PYTHON_DYNAMIC, DOTNET_FIXED] {
        let stream = hex(vector.stream);
        for end in [1, stream.len() / 2, stream.len() - 1] {
            assert_eq!(
                decode_error(&stream[..end], size(vector.data), LIMIT),
                truncated,
                "{end}"
            );
        }
    }
}

#[test]
fn rejects_corrupt_stream() {
    let corrupt = (ErrorKind::InvalidFormat, "raw DEFLATE stream is corrupt");
    // Block type 3 is reserved.
    assert_eq!(decode_error(&[0x07, 0x00], 0, LIMIT), corrupt);
    // The stored block length does not match its one's complement.
    let mut stream = hex(PYTHON_STORED.stream);
    stream[3] ^= 1;
    assert_eq!(
        decode_error(&stream, size(PYTHON_STORED.data), LIMIT),
        corrupt
    );
    // A fixed-Huffman block copies 3 bytes from distance 1, but no earlier
    // output exists. Bits, first bit first: BFINAL 1, BTYPE 01, length code 257
    // (0000001), distance code 0 (00000), end of block (0000000).
    assert_eq!(decode_error(&[0x03, 0x02, 0x00], 3, LIMIT), corrupt);
}

#[test]
fn rejects_declared_size_above_the_limit() {
    let stream = hex(PYTHON_DYNAMIC.stream);
    let declared = size(PYTHON_DYNAMIC.data);
    let limit = PYTHON_DYNAMIC.data.len() - 1;
    assert_eq!(
        decode_error(&stream, declared, limit),
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
