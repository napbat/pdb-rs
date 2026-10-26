//! Tests for compilation options and compilation metadata references.

use pdbrs::ErrorKind;
use pdbrs::portable::custom::{
    CompilationMetadataReferences, CompilationOption, CompilationOptions, CustomDebugKind,
    CustomDebugValue, MetadataReference, Utf8Bytes,
};

use super::{encoded, error, round_trip};

/// Two compilation options as Roslyn writes them.
const OPTIONS: &[u8] = b"language\0C#\0compiler-version\x004.14.0\0";

#[test]
fn decodes_compilation_options() {
    let CustomDebugValue::CompilationOptions(options) =
        round_trip(CustomDebugKind::COMPILATION_OPTIONS, OPTIONS)
    else {
        panic!("expected compilation options");
    };
    assert_eq!(options.len(), 2);
    assert_eq!(options.as_bytes(), OPTIONS);
    let pairs: Vec<_> = options
        .iter()
        .map(|option| (option.name.as_bytes(), option.value.as_bytes()))
        .collect();
    assert_eq!(
        pairs,
        [
            (&b"language"[..], &b"C#"[..]),
            (b"compiler-version", b"4.14.0")
        ]
    );
    assert_eq!(
        options.get(b"compiler-version").unwrap().to_str().unwrap(),
        "4.14.0"
    );
    assert_eq!(options.get(b"missing"), None);
    // An empty name or value is valid.
    assert_eq!(
        CompilationOptions::parse(b"\0\0").unwrap().get(b""),
        Some(Utf8Bytes::new(b""))
    );
    assert!(CompilationOptions::parse(&[]).unwrap().is_empty());
}

#[test]
fn rejects_malformed_compilation_options() {
    let kind = CustomDebugKind::COMPILATION_OPTIONS;
    // A name without a value, and strings without a NUL terminator.
    for blob in [&b"language\0"[..], b"language", b"language\0C#"] {
        assert_eq!(error(kind, blob), ErrorKind::UnexpectedEof, "{blob:?}");
    }
    assert_eq!(
        error(kind, b"language\0C#\0language\0VB\0"),
        ErrorKind::InvalidFormat
    );
    let option = |name: &'static [u8], value: &'static [u8]| {
        let mut option = CompilationOption::default();
        option.name = Utf8Bytes::new(name);
        option.value = Utf8Bytes::new(value);
        option
    };
    for options in [
        vec![option(b"a", b"1"), option(b"a", b"2")],
        vec![option(b"a\0", b"1")],
        vec![option(b"a", b"1\0")],
    ] {
        assert_eq!(
            encoded(|out| CompilationOptions::encode_parts(options, out))
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidFormat
        );
    }
}

/// Return the blob of one metadata reference.
fn reference_blob(file_name: &[u8], aliases: &[u8], flags: u8) -> Vec<u8> {
    let mut blob = Vec::new();
    for string in [file_name, aliases] {
        blob.extend_from_slice(string);
        blob.push(0);
    }
    blob.push(flags);
    blob.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
    blob.extend_from_slice(&0x0002_4000_u32.to_le_bytes());
    blob.extend(1..=16);
    blob
}

#[test]
fn decodes_compilation_metadata_references() {
    let mut blob = reference_blob(b"System.Runtime.dll", b"", MetadataReference::ASSEMBLY);
    blob.extend(reference_blob(
        b"Interop.dll",
        b"a,b",
        MetadataReference::EMBED_INTEROP_TYPES | 0x80,
    ));
    let CustomDebugValue::CompilationMetadataReferences(references) =
        round_trip(CustomDebugKind::COMPILATION_METADATA_REFERENCES, &blob)
    else {
        panic!("expected compilation metadata references");
    };
    assert_eq!(references.len(), 2);
    assert_eq!(references.as_bytes(), blob);
    let all: Vec<_> = references.iter().collect();
    assert_eq!(all[0].file_name.to_str().unwrap(), "System.Runtime.dll");
    assert!(all[0].aliases.is_empty());
    assert_eq!(all[0].flags, MetadataReference::ASSEMBLY);
    assert_eq!(all[0].time_stamp, 0x1234_5678);
    assert_eq!(all[0].file_size, 0x0002_4000);
    assert_eq!(
        all[0].mvid,
        core::array::from_fn(|index| u8::try_from(index + 1).unwrap())
    );
    assert_eq!(all[1].aliases.as_bytes(), b"a,b");
    // The codec keeps the reserved flag bits.
    assert_eq!(all[1].flags, 0x82);
    assert!(
        CompilationMetadataReferences::parse(&[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn rejects_malformed_compilation_metadata_references() {
    let kind = CustomDebugKind::COMPILATION_METADATA_REFERENCES;
    let blob = reference_blob(b"a.dll", b"x", 1);
    for length in 1..blob.len() {
        assert_eq!(
            error(kind, &blob[..length]),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
    let mut reference = MetadataReference::default();
    reference.file_name = Utf8Bytes::new(b"a\0.dll");
    reference.flags = 1;
    assert_eq!(
        encoded(|out| CompilationMetadataReferences::encode_parts([reference], out))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    reference.file_name = Utf8Bytes::new(b"a.dll");
    reference.aliases = Utf8Bytes::new(b"x\0");
    assert_eq!(
        encoded(|out| CompilationMetadataReferences::encode_parts([reference], out))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
}
