//! Tests for the C13 subsection decoders and encoders, the global
//! references and the module stream writer.

mod kinds;
mod lines;
mod module_stream;

use pdbrs::ErrorKind;
use pdbrs::native::lines::LineSubsection;

/// Serialize `u32` values in little-endian order.
fn u32s(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Return the error kind of a failed result.
fn error_kind<T: core::fmt::Debug>(result: pdbrs::Result<T>) -> ErrorKind {
    result.unwrap_err().kind()
}

/// Decode `data` as a subsection of `kind`, encode it again and compare.
///
/// Also check the framed form: the header, the data and the zero padding.
fn round_trip(kind: u32, data: &[u8]) -> LineSubsection<'_> {
    let decoded = LineSubsection::decode(kind, data).unwrap();
    assert_eq!(decoded.kind(), kind);
    let mut out = Vec::new();
    decoded.encode(&mut out).unwrap();
    assert_eq!(out, data, "kind {kind:#x}");
    let mut framed = Vec::new();
    decoded.encode_framed(&mut framed).unwrap();
    let mut expected = u32s(&[kind, u32::try_from(data.len()).unwrap()]);
    expected.extend_from_slice(data);
    while !expected.len().is_multiple_of(4) {
        expected.push(0);
    }
    assert_eq!(framed, expected, "kind {kind:#x}");
    decoded
}
