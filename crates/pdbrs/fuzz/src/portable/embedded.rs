//! The `MPDB` envelope and the embedded-source blob codec.

use pdbrs::portable::embedded::{EmbeddedFormat, EmbeddedSource, decode_embedded, encode_embedded};

use crate::{MAX_STREAM, encoded, ok};

/// The decode limit of the round trips. It is small, so that a run stays fast.
const LIMIT: usize = 1 << 20;

/// Decode an `MPDB` payload, and round-trip the result and the input through the encoder.
///
/// # Panics
///
/// Panics if an encoded payload does not decode to the same bytes.
pub fn mpdb(data: &[u8]) {
    if let Some(decoded) = ok(decode_embedded(data, LIMIT)) {
        assert!(decoded.len() <= LIMIT);
        if let Some(encoded) = ok(encode_embedded(&decoded)) {
            let again = decode_embedded(&encoded, LIMIT).expect("an encoded payload decodes");
            assert_eq!(again, decoded);
        }
        let _ = pdbrs::portable::Pdb::parse(&decoded);
    }
    if let Some(encoded) = ok(encode_embedded(data)) {
        let again = decode_embedded(&encoded, data.len()).expect("an encoded payload decodes");
        assert_eq!(again, data);
        if let Some(short) = data.len().checked_sub(1) {
            assert!(
                decode_embedded(&encoded, short).is_err(),
                "the limit applies"
            );
        }
    }
}

/// Decode an embedded-source blob, and round-trip the result and the input in both formats.
///
/// # Panics
///
/// Panics if a decoded size differs from the declared size, or if an encoded blob does not
/// decode to the same bytes.
pub fn source(data: &[u8]) {
    if let Some(source) = ok(EmbeddedSource::parse(data)) {
        let _ = (source.format(), source.decoded_size());
        if let Some(decoded) = ok(source.decode(LIMIT)) {
            assert_eq!(
                u64::try_from(decoded.len()).ok(),
                Some(source.decoded_size())
            );
            round_trip(&decoded);
        }
        let _ = source.decode(MAX_STREAM);
    }
    round_trip(data);
}

/// Encode `content` in both formats and check that each blob decodes to `content`.
///
/// An empty source always gives the raw form.
fn round_trip(content: &[u8]) {
    for format in [EmbeddedFormat::Raw, EmbeddedFormat::Deflate] {
        let Some(blob) = ok(encoded(|out| {
            EmbeddedSource::encode_parts(content, format, out)
        })) else {
            continue;
        };
        let parsed = EmbeddedSource::parse(&blob).expect("an encoded blob parses");
        let expected = if content.is_empty() {
            EmbeddedFormat::Raw
        } else {
            format
        };
        assert_eq!(parsed.format(), expected);
        let decoded = parsed
            .decode(content.len())
            .expect("an encoded blob decodes");
        assert_eq!(&*decoded, content);
    }
}
