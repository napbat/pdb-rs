//! The format facade `pdbrs::Pdb` with synthetic inputs.

use pdbrs::portable::embedded::{decode_embedded, encode_embedded};
use pdbrs::portable::metadata::MetadataStream;
use pdbrs::{ErrorKind, Format, Pdb};

use crate::builder::{Builder, Stream};
use crate::location;

#[test]
fn portable_metadata_opens_with_the_portable_backend() {
    let bytes = Builder::default().build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.format(), Format::Portable);
    let Pdb::Portable(portable) = pdb else {
        panic!("expected the portable backend");
    };
    assert_eq!(portable.pdb_stream().id()[19], 20);
}

#[test]
fn assembly_metadata_is_rejected_and_not_misclassified() {
    // Assembly metadata has a `#~` stream with type-system tables (here one Module row) and
    // no `#Pdb` stream.
    let builder = Builder::default();
    let mut tables = vec![0, 0, 0, 0, 2, 0, 0, 1];
    tables.extend_from_slice(&1_u64.to_le_bytes());
    tables.extend_from_slice(&0_u64.to_le_bytes());
    tables.extend_from_slice(&1_u32.to_le_bytes());
    tables.extend_from_slice(&[0; 10]);
    let streams = vec![
        Stream::new("#~", tables),
        Stream::new("#Strings", vec![0; 4]),
        Stream::new("#GUID", vec![0; 16]),
        Stream::new("#Blob", vec![0; 4]),
    ];
    let assembly = Builder {
        version: b"v4.0.30319".to_vec(),
        ..builder
    }
    .assemble(&streams);
    let error = Pdb::parse(&assembly).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingStream);
    assert_eq!(location(&error).stream(), Some(MetadataStream::Pdb));
    assert!(error.to_string().contains("not a Portable PDB"), "{error}");
}

#[test]
fn embedded_payloads_need_the_embedded_adapter() {
    let bytes = Builder::default().build();
    let payload = encode_embedded(&bytes).unwrap();
    let error = Pdb::parse(&payload).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert!(error.context().unwrap().contains("decode_embedded"));

    let decoded = decode_embedded(&payload, 1 << 20).unwrap();
    let pdb = Pdb::parse(&decoded).unwrap();
    assert_eq!(pdb.format(), Format::Portable);
}

#[test]
fn other_signatures_are_not_portable() {
    let legacy = b"Microsoft C/C++ program database 2.00\r\n\x1aJG\0\0";
    let error = Pdb::parse(legacy).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);

    let native = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";
    let error = Pdb::parse(native).unwrap_err();
    assert_eq!(error.portable_location(), None, "{error}");

    let error = Pdb::parse(b"MZ\x90\0").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidSignature);
}

#[test]
fn uncompressed_tables_are_unsupported_through_the_facade() {
    let builder = Builder::default();
    let mut streams = builder.streams();
    streams[1].name = b"#-".to_vec();
    let error = Pdb::parse(&builder.assemble(&streams)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
}
