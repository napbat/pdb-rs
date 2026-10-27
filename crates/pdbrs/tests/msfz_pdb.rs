//! Tests of the native `Pdb` facade on MSFZ input (#37).
//!
//! The test converts the synthetic program of the `native_synthetic` example to MSFZ. It
//! stores the streams with odd IDs in stored chunks (compression code 0) of 1000 bytes, so
//! that their fragments cross chunk boundaries. It stores the other streams as uncompressed
//! fragments of at most 300 bytes. Stream 0 is empty, as `pdbtool pdz-encode` writes it.
//! The tests in `src/native/msfz/tests` cover zstd and raw DEFLATE chunks.

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdbrs::ContainerKind;
use pdbrs::io::ReadAt;
use pdbrs::native::container::Container;
use pdbrs::native::msf::{Msf, StreamId};
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb};
use pdbrs::{Format, Pdb as AnyPdb};

/// The decompressed size of each chunk.
const CHUNK_SIZE: usize = 1000;

/// The largest uncompressed fragment.
const FRAGMENT_SIZE: usize = 300;

/// Build the synthetic program as an MSF 7.00 PDB.
fn synthetic() -> Vec<u8> {
    program::build()
        .and_then(|editor| editor.plan(EditOptions::default())?.to_vec())
        .expect("the synthetic program builds")
}

/// Return the streams of an MSF 7.00 PDB. `None` is a nil stream.
fn streams(msf: &[u8]) -> Vec<Option<Vec<u8>>> {
    let msf = Msf::read(msf).expect("MSF 7.00");
    (0..msf.stream_count())
        .map(|id| {
            msf.stream(StreamId(id))
                .expect("stream")
                .map(|stream| stream.read_all(usize::MAX).expect("bytes"))
        })
        .collect()
}

/// Append a fragment record to `directory`.
fn fragment(directory: &mut Vec<u8>, size: usize, location: u64) {
    directory.extend_from_slice(&u32::try_from(size).unwrap().to_le_bytes());
    directory.extend_from_slice(&location.to_le_bytes());
}

/// Convert the streams of an MSF 7.00 PDB to an MSFZ container. Stream 0 becomes empty.
fn to_msfz(msf: &[u8]) -> Vec<u8> {
    let streams = streams(msf);
    let mut chunked = Vec::new();
    let mut directory = Vec::new();
    let mut file = vec![0_u8; 80];
    for (id, stream) in streams.iter().enumerate() {
        let Some(bytes) = stream else {
            directory.extend_from_slice(&u32::MAX.to_le_bytes());
            continue;
        };
        if id == 0 || bytes.is_empty() {
            directory.extend_from_slice(&0_u32.to_le_bytes());
            continue;
        }
        if id % 2 == 1 {
            // One fragment at its offset in the virtual byte array of the chunks.
            let start = chunked.len();
            let chunk = u64::try_from(start / CHUNK_SIZE).unwrap();
            let inner = u64::try_from(start % CHUNK_SIZE).unwrap();
            fragment(
                &mut directory,
                bytes.len(),
                (1 << 63) | (chunk << 32) | inner,
            );
            chunked.extend_from_slice(bytes);
        } else {
            for part in bytes.chunks(FRAGMENT_SIZE) {
                fragment(
                    &mut directory,
                    part.len(),
                    u64::try_from(file.len()).unwrap(),
                );
                file.extend_from_slice(part);
                // A gap between the fragments.
                file.push(0);
            }
        }
        directory.extend_from_slice(&0_u32.to_le_bytes());
    }
    let mut table = Vec::new();
    for chunk in chunked.chunks(CHUNK_SIZE) {
        let size = u32::try_from(chunk.len()).unwrap();
        table.extend_from_slice(&u64::try_from(file.len()).unwrap().to_le_bytes());
        table.extend_from_slice(&0_u32.to_le_bytes());
        table.extend_from_slice(&size.to_le_bytes());
        table.extend_from_slice(&size.to_le_bytes());
        file.extend_from_slice(chunk);
    }
    let directory_offset = u64::try_from(file.len()).unwrap();
    file.extend_from_slice(&directory);
    let table_offset = u64::try_from(file.len()).unwrap();
    file.extend_from_slice(&table);
    let directory_size = u32::try_from(directory.len()).unwrap().to_le_bytes();
    let header = [
        ContainerKind::NativeMsfz.signature(),
        &0_u64.to_le_bytes(),
        &directory_offset.to_le_bytes(),
        &table_offset.to_le_bytes(),
        &u32::try_from(streams.len()).unwrap().to_le_bytes(),
        &0_u32.to_le_bytes(),
        &directory_size,
        &directory_size,
        &u32::try_from(table.len() / 20).unwrap().to_le_bytes(),
        &u32::try_from(table.len()).unwrap().to_le_bytes(),
    ]
    .concat();
    file[..80].copy_from_slice(&header);
    file
}

#[test]
fn the_facade_reads_every_stream_of_an_msfz_pdb() {
    let msf = synthetic();
    let msfz = to_msfz(&msf);
    let detected = AnyPdb::parse(&msfz).expect("MSFZ PDB");
    assert_eq!(detected.format(), Format::Native);
    let pdb = Pdb::parse(&msfz).expect("MSFZ PDB");
    assert_eq!(pdb.container().kind(), ContainerKind::NativeMsfz);
    assert!(pdb.container().msf().is_none());
    let Container::Msfz(container) = pdb.container() else {
        panic!("not MSFZ");
    };
    assert!(container.chunk_count() > 1);
    let original = Pdb::parse(&msf).expect("MSF 7.00 PDB");
    let expected = streams(&msf);
    assert_eq!(
        pdb.container().stream_count(),
        original.container().stream_count()
    );
    for (id, bytes) in expected.iter().enumerate().skip(1) {
        let id = StreamId(u32::try_from(id).unwrap());
        let stream = pdb.container().stream(id).expect("stream");
        assert_eq!(
            stream.map(|stream| stream.read_all(usize::MAX).expect("bytes")),
            *bytes,
            "{id:?}"
        );
    }
    let empty = pdb.container().stream(StreamId(0)).unwrap().unwrap();
    assert_eq!(empty.len().unwrap(), 0);
    assert_eq!(
        format!("{:?}", pdb.info()),
        format!("{:?}", original.info())
    );
    assert_eq!(
        format!("{:?}", pdb.debug_info().unwrap()),
        format!("{:?}", original.debug_info().unwrap())
    );
    assert_eq!(
        format!("{:?}", pdb.types().unwrap()),
        format!("{:?}", original.types().unwrap())
    );
}

#[test]
fn msfz_input_rebuilds_and_edits_to_the_same_msf_output() {
    let msf = synthetic();
    let msfz = to_msfz(&msf);
    let original = Pdb::parse(&msf).expect("MSF 7.00 PDB");
    let pdb = Pdb::parse(&msfz).expect("MSFZ PDB");
    let rebuild = |pdb: &Pdb<&[u8]>| {
        let mut editor = pdb.edit(IdentityPolicy::Preserve);
        editor
            .decode_all()
            .and_then(|()| editor.plan(EditOptions::default())?.to_vec())
            .expect("rebuild")
    };
    assert_eq!(rebuild(&pdb), rebuild(&original));
    let srcsrv = b"SRCSRV: ini ----\r\nVERSION=2\r\nSRCSRV: end ----\r\n";
    let edit = |pdb: &Pdb<&[u8]>| {
        let mut editor = pdb.edit(IdentityPolicy::Preserve);
        editor
            .named_streams_mut()
            .and_then(|named| named.insert(b"srcsrv", srcsrv.to_vec()))
            .expect("named stream");
        editor
            .plan(EditOptions::default())
            .and_then(|plan| plan.to_vec())
            .expect("edit")
    };
    let edited = edit(&pdb);
    assert_eq!(edited, edit(&original));
    let edited = Pdb::parse(&edited).expect("edited PDB");
    assert_eq!(edited.container().kind(), ContainerKind::NativeMsf7);
    assert_eq!(
        edited.named_stream_bytes(b"srcsrv").unwrap().unwrap(),
        srcsrv
    );
}
