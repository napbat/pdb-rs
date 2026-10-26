//! The MSF 7.0 container: reading, repacking and building.

use pdbrs::io::ReadAt;
use pdbrs::native::msf::{
    Msf, MsfBuilder, MsfLimits, OldDirectory, OldDirectoryPolicy, StreamId, WriteOptions,
};

use crate::input::Input;
use crate::{MAX_STREAM, ok};

/// Read all streams, the free page map and the old directory of a container.
///
/// # Panics
///
/// Panics if a container opens with other limits and gives another stream count.
pub fn read(data: &[u8]) {
    let _ = pdbrs::format::probe(data);
    let Some(msf) = ok(Msf::open(data)) else {
        return;
    };
    walk(&msf);
    let limits = MsfLimits::default().with_max_directory_bytes(4096);
    if let Some(small) = ok(Msf::with_limits(data, limits)) {
        assert_eq!(small.stream_count(), msf.stream_count());
    }
}

/// Read each stream of `msf` and check the reported sizes.
fn walk<S: ReadAt>(msf: &Msf<S>) {
    let _ = msf.check_free_page_map();
    for id in (0..msf.stream_count()).map(StreamId) {
        let Some(Some(stream)) = ok(msf.stream(id)) else {
            continue;
        };
        assert_eq!(stream.id(), id);
        let Some(bytes) = ok(stream.read_all(MAX_STREAM)) else {
            continue;
        };
        if id.0 == 0
            && let Some(Some(old)) = ok(OldDirectory::parse(&bytes, msf.block_size()))
        {
            for stream in old.streams() {
                let _ = stream.size();
                let _ = stream.pages().count();
            }
            let _ = old.page_count();
        }
    }
}

/// Read the streams of `msf` with the stream size limit, or `None` if one fails.
fn streams<S: ReadAt>(msf: &Msf<S>) -> Option<Vec<Option<Vec<u8>>>> {
    let mut streams = Vec::new();
    for id in (0..msf.stream_count()).map(StreamId) {
        streams.push(match msf.stream(id).ok()? {
            Some(stream) => Some(stream.read_all(MAX_STREAM).ok()?),
            None => None,
        });
    }
    Some(streams)
}

/// The page sizes that the round trips select from.
const PAGE_SIZES: [u32; 5] = [512, 1024, 2048, 4096, 8192];

/// Repack a container and check that the streams do not change.
///
/// The first input byte selects the page size and the stream 0 policy.
///
/// # Panics
///
/// Panics if the plan does not write, the output does not read, or a stream changes.
pub fn repack(data: &[u8]) {
    let Some((&selector, data)) = data.split_first() else {
        return;
    };
    let Some(msf) = ok(Msf::open(data)) else {
        return;
    };
    let Some(original) = streams(&msf) else {
        return;
    };
    let page_size = if selector & 0x08 == 0 {
        PAGE_SIZES[usize::from(selector & 0x07) % PAGE_SIZES.len()]
    } else {
        msf.block_size()
    };
    let policy = if selector & 0x10 == 0 {
        OldDirectoryPolicy::Empty
    } else {
        OldDirectoryPolicy::Preserve
    };
    let options = WriteOptions::default().with_page_size(page_size);
    let Some(plan) = ok(msf.repack(options, policy)) else {
        return;
    };
    let bytes = plan.to_vec().expect("a planned container writes");
    assert_eq!(u64::try_from(bytes.len()).ok(), Some(plan.file_size()));
    let written = Msf::open(bytes.as_slice()).expect("a written container opens");
    assert_eq!(written.block_size(), page_size);
    let report = written
        .check_free_page_map()
        .expect("a written free page map is valid");
    assert_eq!(report.unreferenced_allocated_count(), 0);
    let copy = streams(&written).expect("written streams read");
    assert_eq!(copy.len(), original.len());
    for (index, (left, right)) in original.iter().zip(&copy).enumerate() {
        if index == 0 && policy == OldDirectoryPolicy::Empty {
            continue;
        }
        assert_eq!(left, right, "stream {index} changed");
    }
}

/// Build a container from input streams and check that it reads back the same streams.
///
/// # Panics
///
/// Panics if the plan does not write, the output does not read, or a stream changes.
pub fn build(data: &[u8]) {
    let mut input = Input::new(data);
    let page_size = PAGE_SIZES[usize::from(input.u8()) % PAGE_SIZES.len()];
    let mut expected: Vec<Option<&[u8]>> = Vec::new();
    let mut builder = MsfBuilder::new();
    while !input.is_empty() && expected.len() < 64 {
        let tag = input.u8();
        if tag & 0x80 == 0 {
            let length = usize::from(input.u16());
            let stream = input.bytes(length);
            builder.push(stream).expect("a stream fits");
            expected.push(Some(stream));
        } else {
            builder.push_nil().expect("a nil stream fits");
            expected.push(None);
        }
    }
    assert_eq!(builder.stream_count(), expected.len());
    let options = WriteOptions::default().with_page_size(page_size);
    let Some(plan) = ok(builder.plan(options)) else {
        return;
    };
    let bytes = plan.to_vec().expect("a planned container writes");
    let msf = Msf::open(bytes.as_slice()).expect("a built container opens");
    let actual = streams(&msf).expect("built streams read");
    let expected: Vec<Option<Vec<u8>>> = expected.iter().map(|s| s.map(<[u8]>::to_vec)).collect();
    assert_eq!(actual, expected);
}
