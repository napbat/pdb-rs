//! Open-time validation: truncation, versions, codes, sizes, references, overlaps and limits.

use alloc::vec;
use alloc::vec::Vec;

use super::{
    CHUNK_COUNT_FIELD, CHUNK_TABLE_OFFSET_FIELD, CHUNK_TABLE_SIZE_FIELD, COMPRESSED, DEFLATE,
    DIRECTORY_COMPRESSION_FIELD, DIRECTORY_OFFSET_FIELD, DIRECTORY_SIZE_FIELD, Image, Part, STORED,
    Spec, ZSTD, expect_error, pattern, read,
};
use crate::ErrorKind;
use crate::format::ContainerKind;
use crate::native::msfz::{Msfz, MsfzLimits};

/// Return the streams of [`sample`].
fn sample_streams() -> Vec<Spec> {
    vec![
        Spec::Parts(Vec::new()),
        Spec::Parts(vec![Part::File(pattern(3, 40))]),
        Spec::Nil,
        Spec::Parts(vec![
            Part::Chunk {
                chunk: 0,
                offset: 100,
                size: 60,
            },
            Part::File(pattern(4, 12)),
        ]),
    ]
}

/// Build a container with the chunks of [`sample`] and `streams`.
fn with_sample_chunks(streams: &[Spec]) -> Image {
    let first = pattern(1, 120);
    let second = pattern(2, 80);
    Image::new(&[(ZSTD, &first), (DEFLATE, &second)], streams)
}

/// Return a valid container with two chunks, file fragments, a nil stream and an empty stream.
fn sample() -> Image {
    with_sample_chunks(&sample_streams())
}

/// Return [`sample`] with a fifth stream: one uncompressed fragment of `size` bytes at `location`.
///
/// The other structures have the same offsets for each `location` and `size`.
fn with_fragment(location: u64, size: u32) -> Image {
    let mut streams = sample_streams();
    let prefix = super::encode_directory(&streams, &[0, 0]).len();
    streams.push(Spec::Parts(vec![Part::Raw {
        size: 1,
        location: 0,
    }]));
    let mut image = with_sample_chunks(&streams);
    let record = image.directory + prefix;
    image.set_u32(record, size);
    image.set_u64(record + 4, location);
    image
}

#[test]
fn the_sample_opens() {
    let image = sample();
    let msfz = image.open().unwrap();
    assert_eq!(
        read(&msfz, 3).unwrap().unwrap(),
        [
            &pattern(1, 120)[100..],
            &pattern(2, 80)[..40],
            &pattern(4, 12)
        ]
        .concat()
    );
}

#[test]
fn every_truncation_fails_without_a_panic() {
    let image = sample();
    for length in 0..image.bytes.len() {
        let bytes = &image.bytes[..length];
        let error = Msfz::open(bytes).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                ErrorKind::UnexpectedEof | ErrorKind::InvalidSignature | ErrorKind::InvalidFormat
            ),
            "{length}: {error:?}"
        );
    }
    expect_error(
        Msfz::open(&image.bytes[..79]),
        ErrorKind::UnexpectedEof,
        "MSFZ header is incomplete",
    );
}

#[test]
fn bytes_after_the_last_structure_are_allowed() {
    // `pdbtool pdz-encode --pad16k` adds zero bytes up to 16 KiB.
    let mut image = sample();
    image.bytes.resize(16384, 0);
    let msfz = image.open().unwrap();
    assert_eq!(read(&msfz, 1).unwrap().unwrap(), pattern(3, 40));
}

#[test]
fn structures_past_the_end_of_the_file_fail() {
    let image = sample();
    let cases = [
        (
            DIRECTORY_OFFSET_FIELD,
            image.bytes.len() - 4,
            "MSFZ stream directory extends past the end of the file",
        ),
        (
            CHUNK_TABLE_OFFSET_FIELD,
            image.bytes.len() - 20,
            "MSFZ Chunk Table extends past the end of the file",
        ),
    ];
    for (field, offset, context) in cases {
        let mut broken = image.clone();
        broken.set_u64(field, u64::try_from(offset).unwrap());
        expect_error(broken.open(), ErrorKind::UnexpectedEof, context);
    }
    let mut broken = image.clone();
    let entry = broken.entry(1, 0);
    broken.set_u64(entry, u64::try_from(image.bytes.len()).unwrap());
    expect_error(
        broken.open(),
        ErrorKind::UnexpectedEof,
        "MSFZ chunk extends past the end of the file",
    );
    let mut broken = image.clone();
    broken.set_u64(DIRECTORY_OFFSET_FIELD, u64::MAX - 2);
    expect_error(
        broken.open(),
        ErrorKind::OffsetOverflow,
        "MSFZ stream directory extends past the end of the file",
    );
}

#[test]
fn unknown_versions_and_compression_codes_are_unsupported() {
    let image = sample();
    for version in [1, u64::MAX] {
        let mut broken = image.clone();
        broken.set_u64(32, version);
        let error = expect_error(
            broken.open(),
            ErrorKind::UnsupportedVersion,
            "MSFZ version is not 0",
        );
        assert_eq!(error.offset(), Some(32));
    }
    for code in [3, u32::MAX] {
        let mut broken = image.clone();
        broken.set_u32(DIRECTORY_COMPRESSION_FIELD, code);
        let error = expect_error(
            broken.open(),
            ErrorKind::UnsupportedVersion,
            "unknown MSFZ compression code",
        );
        assert_eq!(error.offset(), Some(60));
        let mut broken = image.clone();
        let entry = broken.entry(1, 8);
        broken.set_u32(entry, code);
        let error = expect_error(
            broken.open(),
            ErrorKind::UnsupportedVersion,
            "unknown MSFZ compression code",
        );
        assert_eq!(error.offset(), Some(u64::try_from(entry).unwrap()));
    }
    let mut other = image.bytes.clone();
    other[..32].copy_from_slice(ContainerKind::NativeMsf7.signature());
    let error = Msfz::open(other.as_slice()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidSignature);
}

#[test]
fn header_size_fields_must_agree() {
    let image = sample();
    let mut broken = image.clone();
    broken.set_u32(56, 0);
    expect_error(
        broken.open(),
        ErrorKind::InvalidFormat,
        "MSFZ stream count is 0",
    );
    let mut broken = image.clone();
    broken.set_u32(DIRECTORY_SIZE_FIELD, 4);
    expect_error(
        broken.open(),
        ErrorKind::InvalidFormat,
        "uncompressed MSFZ stream directory has two different sizes",
    );
    for (count, size) in [(2, 39), (2, 41), (3, 40), (1, 40)] {
        let mut broken = image.clone();
        broken.set_u32(CHUNK_COUNT_FIELD, count);
        broken.set_u32(CHUNK_TABLE_SIZE_FIELD, size);
        let error = expect_error(
            broken.open(),
            ErrorKind::InvalidFormat,
            "MSFZ Chunk Table size is not 20 bytes for each chunk",
        );
        assert_eq!(error.offset(), Some(76));
    }
}

#[test]
fn chunk_sizes_must_be_valid() {
    let image = Image::new(
        &[(STORED, &pattern(1, 30)), (ZSTD, &pattern(2, 30))],
        &[Spec::Parts(vec![Part::Chunk {
            chunk: 0,
            offset: 0,
            size: 60,
        }])],
    );
    assert_eq!(
        read(&image.open().unwrap(), 0).unwrap().unwrap(),
        [pattern(1, 30), pattern(2, 30)].concat()
    );
    for (chunk, field) in [(0, 12), (0, 16), (1, 12), (1, 16)] {
        let mut broken = image.clone();
        let entry = broken.entry(chunk, field);
        broken.set_u32(entry, 0);
        expect_error(
            broken.open(),
            ErrorKind::InvalidFormat,
            "MSFZ chunk has a size of 0",
        );
    }
    // A stored chunk has one size.
    let mut broken = image.clone();
    let entry = broken.entry(0, 16);
    broken.set_u32(entry, 31);
    expect_error(
        broken.open(),
        ErrorKind::InvalidFormat,
        "uncompressed MSFZ chunk has two different sizes",
    );
}

#[test]
fn compressed_directory_size_mismatches_fail() {
    let streams = [Spec::Parts(vec![Part::File(pattern(1, 8))]), Spec::Nil];
    for code in [ZSTD, DEFLATE] {
        let image = Image::with_directory(&[], &streams, code);
        image.open().unwrap();
        for delta in [-1_i64, 1] {
            let mut broken = image.clone();
            let size = u32::from_le_bytes(broken.bytes[68..72].try_into().unwrap());
            let size = u32::try_from(i64::from(size) + delta).unwrap();
            broken.set_u32(DIRECTORY_SIZE_FIELD, size);
            let error = broken.open().unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{error:?}");
        }
        let mut broken = image.clone();
        let start = broken.directory;
        broken.bytes[start + 1] ^= 0xff;
        broken.bytes[start + 5] ^= 0xff;
        let error = broken.open().unwrap_err();
        assert!(
            matches!(
                error.kind(),
                ErrorKind::InvalidFormat | ErrorKind::UnexpectedEof
            ),
            "{error:?}"
        );
    }
}

#[test]
fn stream_directory_records_must_be_complete() {
    let image = sample();
    // One stream more than the directory holds.
    let mut broken = image.clone();
    broken.set_u32(56, 5);
    expect_error(
        broken.open(),
        ErrorKind::InvalidFormat,
        "MSFZ stream directory ends inside a stream record",
    );
    // A stream count that needs more bytes than the directory has.
    let mut broken = image.clone();
    broken.set_u32(56, u32::MAX);
    expect_error(
        broken.open(),
        ErrorKind::InvalidFormat,
        "MSFZ stream directory is too short for the stream count",
    );
    // One stream less than the directory holds.
    let mut broken = image.clone();
    broken.set_u32(56, 3);
    let error = expect_error(
        broken.open(),
        ErrorKind::InvalidFormat,
        "bytes follow the last MSFZ stream record",
    );
    assert!(error.offset().unwrap() > u64::try_from(image.directory).unwrap());
    // A nil size after the first fragment.
    let broken = Image::new(
        &[],
        &[Spec::Parts(vec![
            Part::File(pattern(1, 4)),
            Part::Raw {
                size: u32::MAX,
                location: 0,
            },
        ])],
    );
    let error = expect_error(
        broken.open(),
        ErrorKind::InvalidFormat,
        "MSFZ fragment has the nil stream size",
    );
    assert_eq!(
        error.offset(),
        Some(u64::try_from(broken.directory + 12).unwrap())
    );
}

#[test]
fn reserved_location_bits_must_be_zero() {
    let image = sample();
    let location = image.files[0];
    for bit in 48..63 {
        let broken = Image::new(
            &[],
            &[Spec::Parts(vec![Part::Raw {
                size: 4,
                location: location | (1 << bit),
            }])],
        );
        expect_error(
            broken.open(),
            ErrorKind::InvalidFormat,
            "reserved bits of an MSFZ fragment location are not 0",
        );
    }
}

#[test]
fn fragment_references_must_be_inside_the_file_and_the_chunks() {
    let chunks = [(ZSTD, &pattern(1, 50)[..]), (STORED, &pattern(2, 50)[..])];
    let fragment = |chunk: u32, offset: u32, size: u32| {
        Image::new(
            &chunks,
            &[Spec::Parts(vec![Part::Chunk {
                chunk,
                offset,
                size,
            }])],
        )
    };
    let image = fragment(0, 49, 51);
    assert_eq!(
        read(&image.open().unwrap(), 0).unwrap().unwrap(),
        [&pattern(1, 50)[49..], &pattern(2, 50)[..]].concat()
    );
    expect_error(
        fragment(0, 49, 52).open(),
        ErrorKind::InvalidFormat,
        "compressed MSFZ fragment extends past the end of the last chunk",
    );
    expect_error(
        fragment(1, 0, u32::MAX - 1).open(),
        ErrorKind::InvalidFormat,
        "compressed MSFZ fragment extends past the end of the last chunk",
    );
    expect_error(
        fragment(0, 50, 1).open(),
        ErrorKind::InvalidReference,
        "MSFZ fragment starts after the end of its first chunk",
    );
    for chunk in [2, 0x7fff_ffff] {
        expect_error(
            fragment(chunk, 0, 1).open(),
            ErrorKind::InvalidReference,
            "MSFZ fragment names a chunk outside the Chunk Table",
        );
    }
    let length = u64::try_from(sample().bytes.len()).unwrap();
    let past = Image::new(
        &[],
        &[Spec::Parts(vec![Part::Raw {
            size: 8,
            location: length,
        }])],
    );
    expect_error(
        past.open(),
        ErrorKind::UnexpectedEof,
        "uncompressed MSFZ fragment extends past the end of the file",
    );
    let all_ones = Image::new(
        &chunks,
        &[Spec::Parts(vec![Part::Raw {
            size: 1,
            location: u64::MAX,
        }])],
    );
    assert_eq!(
        all_ones.open().unwrap_err().kind(),
        ErrorKind::InvalidReference
    );
    assert_eq!(COMPRESSED, 1 << 63);
}

/// Open `image` and expect an overlap error with `context`.
#[track_caller]
fn expect_overlap(image: &Image, context: &str) {
    expect_error(image.open(), ErrorKind::InvalidFormat, context);
}

#[test]
fn fixed_structures_must_not_overlap() {
    // A directory of one empty stream at offset 76: the bytes are `chunk_table_size`, 0.
    let mut image = Image::new(&[], &[Spec::Parts(Vec::new())]);
    image.open().unwrap();
    image.set_u64(DIRECTORY_OFFSET_FIELD, 76);
    expect_overlap(&image, "MSFZ stream directory overlaps the header");

    let image = sample();
    let mut broken = image.clone();
    broken.set_u64(CHUNK_TABLE_OFFSET_FIELD, 60);
    expect_overlap(&broken, "MSFZ Chunk Table overlaps the header");
    let mut broken = image.clone();
    let directory = u64::try_from(image.directory).unwrap();
    broken.set_u64(CHUNK_TABLE_OFFSET_FIELD, directory + 4);
    expect_overlap(&broken, "MSFZ Chunk Table overlaps the stream directory");
    // An empty Chunk Table has no range.
    let mut empty = Image::new(&[], &[Spec::Parts(vec![Part::File(pattern(1, 4))])]);
    empty.set_u64(CHUNK_TABLE_OFFSET_FIELD, 0);
    empty.open().unwrap();
    empty.set_u64(CHUNK_TABLE_OFFSET_FIELD, u64::MAX);
    empty.open().unwrap();
}

#[test]
fn chunks_must_not_overlap() {
    let image = sample();
    let stored = Image::new(
        &[(STORED, &pattern(1, 20)), (STORED, &pattern(2, 20))],
        &[Spec::Parts(vec![Part::File(pattern(3, 8))])],
    );
    let cases: [(Image, usize, u64, &str); 4] = [
        (stored.clone(), 0, 0, "MSFZ chunk overlaps the header"),
        (
            stored.clone(),
            1,
            stored.chunks[0] + 19,
            "two MSFZ chunks overlap",
        ),
        (
            stored.clone(),
            1,
            u64::try_from(stored.directory).unwrap(),
            "MSFZ chunk overlaps the stream directory",
        ),
        (
            stored.clone(),
            1,
            u64::try_from(stored.table).unwrap(),
            "MSFZ chunk overlaps the Chunk Table",
        ),
    ];
    for (mut broken, chunk, offset, context) in cases {
        let entry = broken.entry(chunk, 0);
        broken.set_u64(entry, offset);
        expect_overlap(&broken, context);
    }
    let mut broken = image.clone();
    let entry = broken.entry(1, 0);
    broken.set_u64(entry, image.chunks[0]);
    expect_overlap(&broken, "two MSFZ chunks overlap");
}

#[test]
fn uncompressed_fragments_must_not_overlap() {
    let base = with_fragment(0, 1);
    let directory = u64::try_from(base.directory).unwrap();
    let table = u64::try_from(base.table).unwrap();
    let cases = [
        (0, 8, "uncompressed MSFZ fragment overlaps the header"),
        (79, 8, "uncompressed MSFZ fragment overlaps the header"),
        (
            base.chunks[1] - 2,
            4,
            "uncompressed MSFZ fragment overlaps a chunk",
        ),
        (
            base.files[0] + 39,
            2,
            "two uncompressed MSFZ fragments overlap",
        ),
        (
            directory + 2,
            4,
            "uncompressed MSFZ fragment overlaps the stream directory",
        ),
        (
            table,
            4,
            "uncompressed MSFZ fragment overlaps the Chunk Table",
        ),
    ];
    for (location, size, context) in cases {
        expect_overlap(&with_fragment(location, size), context);
    }
    // A fragment in the gap before the stream directory is valid.
    let mut gap = with_fragment(0, 1);
    let start = gap.directory;
    gap.bytes.splice(start..start, [0; 8]);
    gap.directory += 8;
    gap.table += 8;
    gap.set_u64(DIRECTORY_OFFSET_FIELD, directory + 8);
    gap.set_u64(CHUNK_TABLE_OFFSET_FIELD, table + 8);
    let record = gap.directory + super::encode_directory(&sample_streams(), &[0, 0]).len();
    gap.set_u32(record, 8);
    gap.set_u64(record + 4, directory);
    let msfz = gap.open().unwrap();
    assert_eq!(read(&msfz, 4).unwrap().unwrap(), [0; 8]);
}

#[test]
fn limits_are_checked_before_allocation() {
    let image = sample();
    let directory_size = u32::from_le_bytes(image.bytes[68..72].try_into().unwrap());
    let error = expect_error(
        image.open_with(MsfzLimits::default().with_max_directory_bytes(directory_size - 1)),
        ErrorKind::LimitExceeded,
        "MSFZ stream directory is larger than the directory limit",
    );
    assert_eq!(error.offset(), Some(64));
    image
        .open_with(MsfzLimits::default().with_max_directory_bytes(directory_size))
        .unwrap();
    expect_error(
        image.open_with(MsfzLimits::default().with_max_chunk_table_bytes(39)),
        ErrorKind::LimitExceeded,
        "MSFZ Chunk Table is larger than the Chunk Table limit",
    );
    image
        .open_with(MsfzLimits::default().with_max_chunk_table_bytes(40))
        .unwrap();
    // A huge declared directory fails on the limit, not on the allocation.
    let mut huge = image.clone();
    huge.set_u32(DIRECTORY_COMPRESSION_FIELD, ZSTD);
    huge.set_u32(DIRECTORY_SIZE_FIELD, u32::MAX);
    expect_error(
        huge.open(),
        ErrorKind::LimitExceeded,
        "MSFZ stream directory is larger than the directory limit",
    );
    let limits = MsfzLimits::default();
    assert_eq!(limits.max_directory_bytes(), 64 << 20);
    assert_eq!(limits.max_chunk_table_bytes(), 64 << 20);
    assert_eq!(limits.max_chunk_bytes(), 256 << 20);
    assert_eq!(limits.max_cache_bytes(), 256 << 20);
    assert_eq!(image.open().unwrap().limits(), limits);
}
