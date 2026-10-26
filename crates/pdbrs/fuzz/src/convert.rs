//! The conversion round trip between MSF 7.0 and MSFZ containers.

use pdbrs::format::ContainerKind;
use pdbrs::native::container::Container;
use pdbrs::native::msf::{self, MsfLimits, OldDirectoryPolicy, StreamId};
use pdbrs::native::msfz::{self, Codec, MIN_CHUNK_SIZE};

use crate::msfz::limits;
use crate::{MAX_STREAM, ok};

/// The streams of a container: the bytes of each stream, or `None` for a nil stream.
type Streams = Vec<Option<Vec<u8>>>;

/// Read all streams of `container`, or `None` if one does not read.
fn streams(container: &Container<&[u8]>) -> Option<Streams> {
    let mut streams = Vec::new();
    for id in (0..container.stream_count()).map(StreamId) {
        streams.push(match container.stream(id).ok()? {
            Some(stream) => Some(stream.read_all(MAX_STREAM).ok()?),
            None => None,
        });
    }
    Some(streams)
}

/// Return the MSFZ options and the stream 0 policy of a selector byte.
///
/// - Bits 0 and 1: the maximum chunk size (4 KiB, 16 KiB, 64 KiB or 1 MiB).
/// - Bit 2: one stream for each chunk.
/// - Bit 3: raw DEFLATE chunks instead of zstd chunks.
/// - Bits 4 and 5: the stream directory codec (none, zstd, raw DEFLATE, none).
/// - Bit 6: padding to 16 KiB.
/// - Bit 7: [`OldDirectoryPolicy::Preserve`] instead of [`OldDirectoryPolicy::Empty`].
#[must_use]
pub fn options(selector: u8) -> (msfz::WriteOptions, OldDirectoryPolicy) {
    let chunk_size = MIN_CHUNK_SIZE << [0, 2, 4, 8][usize::from(selector & 0x03)];
    let codec = if selector & 0x08 == 0 {
        Codec::Zstd
    } else {
        Codec::Deflate
    };
    let directory = match (selector >> 4) & 0x03 {
        1 => Some(Codec::Zstd),
        2 => Some(Codec::Deflate),
        _ => None,
    };
    let options = msfz::WriteOptions::default()
        .with_max_chunk_size(chunk_size)
        .with_one_stream_per_chunk(selector & 0x04 != 0)
        .with_codec(codec)
        .with_directory_codec(directory)
        .with_pad_16k(selector & 0x40 != 0)
        .with_limits(limits());
    let policy = if selector & 0x80 == 0 {
        OldDirectoryPolicy::Empty
    } else {
        OldDirectoryPolicy::Preserve
    };
    (options, policy)
}

/// Convert an MSF 7.0 or MSFZ container to MSFZ and back to MSF 7.0.
///
/// The first input byte selects the options (see [`options`]). The rest is the container.
///
/// # Panics
///
/// Panics if a plan does not write, a second write gives other bytes, an output does not
/// read, a stream changes, or the MSF 7.0 output has allocated pages without a reference.
pub fn round_trip(data: &[u8]) {
    let Some((&selector, data)) = data.split_first() else {
        return;
    };
    let Some(source) = ok(Container::with_limits(data, MsfLimits::default(), limits())) else {
        return;
    };
    let Some(original) = streams(&source) else {
        return;
    };
    let (options, policy) = options(selector);
    let mut expected = original;
    if policy == OldDirectoryPolicy::Empty
        && let Some(first) = expected.first_mut()
    {
        *first = Some(Vec::new());
    }
    let Some(plan) = ok(source.to_msfz(options, policy)) else {
        return;
    };
    let bytes = plan.to_vec().expect("a planned MSFZ container writes");
    assert_eq!(u64::try_from(bytes.len()).ok(), Some(plan.file_size()));
    assert_eq!(
        plan.to_vec()
            .expect("a planned MSFZ container writes again"),
        bytes,
        "a second write gives other bytes"
    );
    let written = Container::with_limits(bytes.as_slice(), MsfLimits::default(), limits())
        .expect("a written MSFZ container opens");
    assert_eq!(written.kind(), ContainerKind::NativeMsfz);
    assert_eq!(
        streams(&written).expect("written MSFZ streams read"),
        expected,
        "the MSFZ streams changed"
    );

    let Some(plan) = ok(written.to_msf(msf::WriteOptions::default(), policy)) else {
        return;
    };
    let bytes = plan.to_vec().expect("a planned MSF container writes");
    let back = Container::open(bytes.as_slice()).expect("a written MSF container opens");
    assert_eq!(back.kind(), ContainerKind::NativeMsf7);
    assert_eq!(
        streams(&back).expect("written MSF streams read"),
        expected,
        "the MSF streams changed"
    );
    let report = back
        .msf()
        .expect("the output is MSF 7.0")
        .check_free_page_map()
        .expect("a written free page map is valid");
    assert_eq!(report.unreferenced_allocated_count(), 0);
}
