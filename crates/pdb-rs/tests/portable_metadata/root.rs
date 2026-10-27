//! The metadata root, the stream headers, the `#Pdb` stream and the heap checks at open.

use pdb_rs::ErrorKind;
use pdb_rs::portable::metadata::{MetadataStream, MethodDefId, StreamHeader, TableId};
use pdb_rs::portable::{Pdb, ReadOptions};

use crate::builder::{Builder, Stream};
use crate::{expect, location, open_error};

#[test]
fn default_metadata_opens_with_its_headers() {
    let bytes = Builder::default().build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let metadata = pdb.metadata();
    assert_eq!(metadata.version(), (1, 1));
    assert_eq!(metadata.version_string(), b"PDB v1.0");
    let names: Vec<&[u8]> = metadata.streams().iter().map(StreamHeader::name).collect();
    assert_eq!(
        names,
        [&b"#Pdb"[..], b"#~", b"#Strings", b"#US", b"#GUID", b"#Blob"]
    );
    for stream in metadata.streams() {
        assert_eq!(stream.offset() % 4, 0);
        assert!(stream.offset() >= metadata.header_len());
    }
    let header = pdb.pdb_stream();
    assert_eq!(header.id()[0], 1);
    assert_eq!(header.entry_point(), Some(MethodDefId(1)));
    assert_eq!(header.entry_point_token().raw(), 0x0600_0001);
    let external: Vec<(TableId, u32)> = header.external_tables().collect();
    assert_eq!(external, [(TableId::MODULE, 1), (TableId::METHOD_DEF, 3)]);
    assert_eq!(pdb.identity().guid()[0], 1);
    assert_eq!(pdb.heap_size(MetadataStream::Strings), Some(8));
    assert_eq!(pdb.heap_size(MetadataStream::Guid), Some(32));
    assert_eq!(pdb.heap_size(MetadataStream::Tables), None);
}

#[test]
fn every_truncated_prefix_fails_without_a_panic() {
    let bytes = Builder::default().build();
    for length in 0..bytes.len() {
        let error = open_error(&bytes[..length]);
        assert!(
            matches!(
                error.kind(),
                ErrorKind::UnexpectedEof | ErrorKind::InvalidSignature
            ),
            "length {length}: {error}"
        );
    }
}

#[test]
fn signature_and_root_version_are_checked() {
    let mut bytes = Builder::default().build();
    bytes[3] = b'A';
    assert_eq!(open_error(&bytes).kind(), ErrorKind::InvalidSignature);

    let old = Builder {
        root_version: (1, 0),
        ..Builder::default()
    };
    expect(
        &open_error(&old.build()),
        ErrorKind::UnsupportedVersion,
        MetadataStream::Root,
    );
}

#[test]
fn version_string_needs_an_aligned_length_and_a_terminator() {
    let bytes = Builder::default().build();
    let mut unaligned = bytes.clone();
    unaligned[12] = 6;
    expect(
        &open_error(&unaligned),
        ErrorKind::InvalidFormat,
        MetadataStream::Root,
    );

    let mut unterminated = bytes;
    unterminated[16..28].fill(b'x');
    let error = open_error(&unterminated);
    expect(&error, ErrorKind::InvalidFormat, MetadataStream::Root);
    assert_eq!(
        error.context(),
        Some("metadata version string has no terminator")
    );
}

#[test]
fn unaligned_stream_offsets_and_sizes_are_rejected() {
    let builder = Builder::default();
    let mut streams = builder.streams();
    streams[2].offset = Some(0x102);
    let error = open_error(&builder.assemble(&streams));
    expect(&error, ErrorKind::InvalidFormat, MetadataStream::Strings);
    assert_eq!(
        error.context(),
        Some("metadata stream offset or size is not 4-byte aligned")
    );

    let mut streams = builder.streams();
    streams[5].size = Some(6);
    expect(
        &open_error(&builder.assemble(&streams)),
        ErrorKind::InvalidFormat,
        MetadataStream::Blob,
    );
}

#[test]
fn duplicate_and_overlapping_streams_are_rejected() {
    let builder = Builder::default();
    let mut streams = builder.streams();
    streams.push(Stream::new("#Strings", vec![0; 4]));
    let error = open_error(&builder.assemble(&streams));
    expect(&error, ErrorKind::InvalidFormat, MetadataStream::Strings);
    assert_eq!(error.context(), Some("duplicate metadata stream"));

    let first = Pdb::parse(&builder.build()).unwrap().metadata().streams()[0].offset();
    let mut streams = builder.streams();
    streams[1].offset = Some(first);
    let error = open_error(&builder.assemble(&streams));
    assert_eq!(error.context(), Some("metadata streams overlap"));

    let mut streams = builder.streams();
    streams[3].offset = Some(4);
    let error = open_error(&builder.assemble(&streams));
    assert_eq!(
        error.context(),
        Some("metadata stream overlaps the stream headers")
    );
}

#[test]
fn streams_after_the_input_and_long_names_are_rejected() {
    let builder = Builder::default();
    let mut streams = builder.streams();
    streams[5].size = Some(0x1000);
    expect(
        &open_error(&builder.assemble(&streams)),
        ErrorKind::UnexpectedEof,
        MetadataStream::Blob,
    );

    let mut streams = builder.streams();
    streams.push(Stream::new(&"#".repeat(32), Vec::new()));
    expect(
        &open_error(&builder.assemble(&streams)),
        ErrorKind::InvalidFormat,
        MetadataStream::Root,
    );
}

#[test]
fn the_stream_count_has_a_limit() {
    let bytes = Builder::default().build();
    let options = ReadOptions::default().with_max_streams(5);
    assert_eq!(options.max_streams(), 5);
    let error = Pdb::with_options(bytes.as_slice(), options).unwrap_err();
    expect(&error, ErrorKind::LimitExceeded, MetadataStream::Root);
}

#[test]
fn unknown_streams_stay_in_the_headers() {
    let builder = Builder::default();
    let mut streams = builder.streams();
    streams.push(Stream::new("#JTD", Vec::new()));
    streams.push(Stream::new("#Other", vec![1, 2, 3, 4]));
    let bytes = builder.assemble(&streams);
    let pdb = Pdb::parse(&bytes).unwrap();
    let other = &pdb.metadata().streams()[7];
    assert_eq!(other.name(), b"#Other");
    assert_eq!(other.stream(), None);
}

#[test]
fn metadata_without_pdb_stream_is_assembly_metadata() {
    let builder = Builder::default();
    let streams: Vec<Stream> = builder.streams().into_iter().skip(1).collect();
    let error = open_error(&builder.assemble(&streams));
    expect(&error, ErrorKind::MissingStream, MetadataStream::Pdb);
    assert!(
        error
            .context()
            .unwrap()
            .contains("ordinary assembly metadata")
    );
}

#[test]
fn table_stream_is_required_and_uncompressed_tables_are_unsupported() {
    let builder = Builder::default();
    let mut streams = builder.streams();
    streams.remove(1);
    expect(
        &open_error(&builder.assemble(&streams)),
        ErrorKind::MissingStream,
        MetadataStream::Tables,
    );

    let mut streams = builder.streams();
    streams[1].name = b"#-".to_vec();
    expect(
        &open_error(&builder.assemble(&streams)),
        ErrorKind::UnsupportedVersion,
        MetadataStream::UncompressedTables,
    );
}

/// Assemble the default streams with other `#Pdb` bytes.
fn with_pdb_stream(bytes: Vec<u8>) -> Vec<u8> {
    let builder = Builder::default();
    let mut streams = builder.streams();
    streams[0].bytes = bytes;
    builder.assemble(&streams)
}

#[test]
fn pdb_stream_size_and_tables_are_checked() {
    let valid = Builder::default().pdb_stream();
    for (bytes, case) in [
        (valid[..28].to_vec(), "shorter than 32 bytes"),
        ([valid.clone(), vec![0; 4]].concat(), "trailing row count"),
        (valid[..36].to_vec(), "missing row count"),
    ] {
        let error = open_error(&with_pdb_stream(bytes));
        expect(&error, ErrorKind::InvalidFormat, MetadataStream::Pdb);
        assert!(error.to_string().contains("stream #Pdb"), "{case}: {error}");
    }

    let debug_table = Builder {
        external: vec![(0x06, 3), (0x30, 1)],
        ..Builder::default()
    };
    expect(
        &open_error(&debug_table.build()),
        ErrorKind::InvalidFormat,
        MetadataStream::Pdb,
    );

    let too_many_rows = Builder {
        external: vec![(0x06, 0x0100_0000)],
        ..Builder::default()
    };
    expect(
        &open_error(&too_many_rows.build()),
        ErrorKind::InvalidFormat,
        MetadataStream::Pdb,
    );
}

#[test]
fn entry_point_is_nil_or_an_external_method() {
    let none = Builder {
        entry_point: 0,
        ..Builder::default()
    };
    let bytes = none.build();
    assert_eq!(Pdb::parse(&bytes).unwrap().pdb_stream().entry_point(), None);

    for entry_point in [0x0200_0001, 0x0600_0000, 0x0600_0004] {
        let builder = Builder {
            entry_point,
            ..Builder::default()
        };
        expect(
            &open_error(&builder.build()),
            ErrorKind::InvalidReference,
            MetadataStream::Pdb,
        );
    }
}

#[test]
fn heaps_start_with_an_empty_entry_and_guids_fill_16_bytes() {
    for (builder, stream) in [
        (
            Builder {
                strings: Some(b"name\0\0\0\0".to_vec()),
                ..Builder::default()
            },
            MetadataStream::Strings,
        ),
        (
            Builder {
                blobs: Some(vec![1, 0, 0, 0]),
                ..Builder::default()
            },
            MetadataStream::Blob,
        ),
        (
            Builder {
                user_strings: Some(vec![1, 0, 0, 0]),
                ..Builder::default()
            },
            MetadataStream::UserStrings,
        ),
        (
            Builder {
                guids: Some(vec![0; 20]),
                ..Builder::default()
            },
            MetadataStream::Guid,
        ),
    ] {
        let error = open_error(&builder.build());
        expect(&error, ErrorKind::InvalidFormat, stream);
        assert_eq!(location(&error).heap_offset(), Some(0));
    }
}

#[test]
fn absent_heaps_are_empty() {
    let builder = Builder {
        strings: None,
        user_strings: None,
        guids: None,
        blobs: None,
        ..Builder::default()
    };
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.heap_size(MetadataStream::Blob), Some(0));
    assert_eq!(pdb.stream_bytes(MetadataStream::Blob).unwrap(), None);
    assert_eq!(
        pdb.stream_bytes(MetadataStream::Pdb).unwrap(),
        Some(builder.pdb_stream())
    );
}
