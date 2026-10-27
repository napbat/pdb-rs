//! Behavior tests for the conversion between MSF 7.00 and MSFZ containers.

use pdb_rs::ContainerKind;
use pdb_rs::ErrorKind;
use pdb_rs::native::container::Container;
use pdb_rs::native::msf::{self, Msf, MsfBuilder, OldDirectoryPolicy, StreamId};
use pdb_rs::native::msfz::{self, Codec, MIN_CHUNK_SIZE, MsfzBuilder};

/// Return `length` bytes that compress a little.
fn bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| (index ^ (index >> 8)).to_le_bytes()[0] ^ seed)
        .collect()
}

/// Streams with bytes in stream 0, a nil stream, empty streams and multi-chunk streams.
fn sample_streams() -> Vec<Option<Vec<u8>>> {
    vec![
        Some(bytes(48, 9)),
        Some(bytes(100, 1)),
        None,
        Some(Vec::new()),
        Some(bytes(9000, 3)),
        None,
        Some(bytes(5000, 5)),
        Some(Vec::new()),
    ]
}

/// Write `streams` as an MSF 7.00 container with 4096-byte pages.
fn msf_bytes(streams: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut builder = MsfBuilder::new();
    for stream in streams {
        match stream {
            Some(data) => builder.push(data.as_slice()).unwrap(),
            None => builder.push_nil().unwrap(),
        };
    }
    builder
        .plan(msf::MsfWriteOptions::default())
        .unwrap()
        .to_vec()
        .unwrap()
}

/// Read all streams of a container.
fn streams(container: &Container<&[u8]>) -> Vec<Option<Vec<u8>>> {
    (0..container.stream_count())
        .map(|id| {
            container
                .stream(StreamId(id))
                .unwrap()
                .map(|stream| stream.read_all(usize::MAX).unwrap())
        })
        .collect()
}

/// Return `streams` with the stream 0 that `policy` gives.
fn expected(streams: &[Option<Vec<u8>>], policy: OldDirectoryPolicy) -> Vec<Option<Vec<u8>>> {
    let mut expected = streams.to_vec();
    if policy == OldDirectoryPolicy::Empty {
        expected[0] = Some(Vec::new());
    }
    expected
}

fn small_chunks() -> msfz::MsfzWriteOptions {
    msfz::MsfzWriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE)
}

#[test]
fn msf_to_msfz_to_msf_keeps_ids_nil_empty_streams_and_bytes() {
    let original = sample_streams();
    let msf = msf_bytes(&original);
    let source = Container::read(msf.as_slice()).unwrap();
    for policy in [OldDirectoryPolicy::Empty, OldDirectoryPolicy::Preserve] {
        let expected = expected(&original, policy);
        let pdz = source
            .to_msfz(small_chunks(), policy)
            .unwrap()
            .to_vec()
            .unwrap();
        let converted = Container::read(pdz.as_slice()).unwrap();
        assert_eq!(converted.kind(), ContainerKind::NativeMsfz);
        assert_eq!(streams(&converted), expected);

        let back = converted
            .to_msf(msf::MsfWriteOptions::default(), policy)
            .unwrap()
            .to_vec()
            .unwrap();
        let back = Container::read(back.as_slice()).unwrap();
        assert_eq!(back.kind(), ContainerKind::NativeMsf7);
        assert_eq!(streams(&back), expected);
        let report = back.msf().unwrap().validate_free_page_map().unwrap();
        assert_eq!(report.unreferenced_allocated_count(), 0);

        // The second conversion gives the same MSFZ bytes.
        let again = back
            .to_msfz(small_chunks(), policy)
            .unwrap()
            .to_vec()
            .unwrap();
        assert_eq!(again, pdz);
    }
}

#[test]
fn msf_to_msf_is_a_repack() {
    let msf = msf_bytes(&sample_streams());
    let source = Container::read(msf.as_slice()).unwrap();
    let options = msf::MsfWriteOptions::default().with_page_size(1024);
    let converted = source
        .to_msf(options, OldDirectoryPolicy::Empty)
        .unwrap()
        .to_vec()
        .unwrap();
    let repacked = Msf::read(msf.as_slice())
        .unwrap()
        .repack(options, OldDirectoryPolicy::Empty)
        .unwrap()
        .to_vec()
        .unwrap();
    assert_eq!(converted, repacked);
}

#[test]
fn msfz_to_msfz_changes_only_the_layout() {
    let original = sample_streams();
    let msf = msf_bytes(&original);
    let pdz = Container::read(msf.as_slice())
        .unwrap()
        .to_msfz(
            msfz::MsfzWriteOptions::default(),
            OldDirectoryPolicy::Preserve,
        )
        .unwrap()
        .to_vec()
        .unwrap();
    let source = Container::read(pdz.as_slice()).unwrap();
    let options = small_chunks()
        .with_one_stream_per_chunk(true)
        .with_codec(Codec::Deflate)
        .with_directory_codec(Some(Codec::Zstd))
        .with_pad_16k(true);
    let recoded = source
        .to_msfz(options, OldDirectoryPolicy::Preserve)
        .unwrap()
        .to_vec()
        .unwrap();
    let recoded = Container::read(recoded.as_slice()).unwrap();
    assert_eq!(streams(&recoded), original);
    let msfz = recoded.msfz().unwrap();
    assert!(msfz.is_directory_compressed());
    // Streams of 48, 100, 9000 and 5000 bytes in 4 KiB chunks, one stream for each chunk.
    assert_eq!(msfz.chunk_count(), 1 + 1 + 3 + 2);
}

#[test]
fn an_msfz_stream_of_4_gib_does_not_convert_to_msf() {
    // Write a container with one chunk, then change it: stream 1 gets two fragments of 2 GiB
    // that name the same chunk, and the chunk declares 2 GiB. Opening does not decompress it.
    let mut builder = MsfzBuilder::new();
    builder.push(&[][..]).unwrap();
    builder.push(&[7_u8; 16][..]).unwrap();
    builder.push(&[8_u8; 16][..]).unwrap();
    let mut pdz = builder
        .plan(msfz::MsfzWriteOptions::default())
        .unwrap()
        .to_vec()
        .unwrap();
    let field = |bytes: &[u8], at: usize| {
        usize::try_from(u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())).unwrap()
    };
    let directory = field(&pdz, 40);
    let table = field(&pdz, 48);
    let half = 0x8000_0000_u32;
    let location = 1_u64 << 63;
    let mut records = Vec::new();
    records.extend_from_slice(&0_u32.to_le_bytes());
    for _ in 0..2 {
        records.extend_from_slice(&half.to_le_bytes());
        records.extend_from_slice(&location.to_le_bytes());
    }
    records.extend_from_slice(&0_u32.to_le_bytes());
    pdz[directory..directory + records.len()].copy_from_slice(&records);
    let size = u32::try_from(records.len()).unwrap().to_le_bytes();
    pdz[56..60].copy_from_slice(&2_u32.to_le_bytes());
    pdz[64..68].copy_from_slice(&size);
    pdz[68..72].copy_from_slice(&size);
    pdz[table + 16..table + 20].copy_from_slice(&half.to_le_bytes());

    let source = Container::read(pdz.as_slice()).unwrap();
    let stream = source.stream(StreamId(1)).unwrap().unwrap();
    assert_eq!(pdb_rs::io::ReadAt::len(&stream).unwrap(), 1 << 32);
    let error = source
        .to_msf(msf::MsfWriteOptions::default(), OldDirectoryPolicy::Empty)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    assert_eq!(error.stream_location(), Some((StreamId(1), 0)));
}
