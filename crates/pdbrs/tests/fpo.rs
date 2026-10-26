//! Tests for the FPO and new-FPO (frame data) streams.

use pdbrs::ErrorKind;
use pdbrs::native::fpo::{FPO_RECORD_SIZE, FRAME_DATA_SIZE, FpoStream, FrameDataStream, FrameKind};
use pdbrs::native::msf::StreamId;

/// The stream ID of the synthetic streams.
const STREAM: StreamId = StreamId(8);

/// Build one `FPO_DATA` record.
fn fpo(start: u32, size: u32, locals: u32, params: u16, attributes: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [start, size, locals] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&params.to_le_bytes());
    bytes.extend_from_slice(&attributes.to_le_bytes());
    bytes
}

/// Build one `FRAMEDATA` record.
fn frame(words: [u32; 6], prolog: u16, saved: u16, flags: u32) -> Vec<u8> {
    let mut bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    bytes.extend_from_slice(&prolog.to_le_bytes());
    bytes.extend_from_slice(&saved.to_le_bytes());
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes
}

fn assert_truncated(error: &pdbrs::Error, offset: u64) {
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof, "{error}");
    assert_eq!(error.offset(), None);
    assert_eq!(error.stream_location(), Some((STREAM.0, offset)));
}

#[test]
fn fpo_records_decode_fields_and_bit_fields() {
    let bytes = [
        // An SEH filter block as MSVC writes it: only `fHasSEH` (bit 11) is set.
        fpo(0x28ca, 6, 0, 0, 0x0800),
        // All bit fields at their maximum values.
        fpo(u32::MAX, u32::MAX, u32::MAX, u16::MAX, 0xffff),
        // `cbProlog` 0x80, `cbRegs` 5, `fUseBP`, frame kind 1.
        fpo(0x1000, 0x20, 3, 2, 0x4000 | 0x1000 | 0x0500 | 0x80),
        // Frame kind 2 and the reserved bit.
        fpo(0, 0, 0, 0, 0x8000 | 0x2000),
    ]
    .concat();
    let stream = FpoStream::parse(bytes, STREAM).unwrap();
    assert_eq!(stream.len(), 4);
    assert!(!stream.is_empty());
    let records: Vec<_> = stream.iter().collect();

    let filter = records[0];
    assert_eq!(
        (filter.start, filter.size, filter.locals, filter.params),
        (0x28ca, 6, 0, 0)
    );
    assert_eq!((filter.prolog_size, filter.saved_registers), (0, 0));
    assert!(filter.has_seh);
    assert!(!filter.uses_bp && !filter.reserved);
    assert_eq!(filter.frame, FrameKind::Fpo);

    let full = records[1];
    assert_eq!(
        (full.start, full.size, full.locals),
        (u32::MAX, u32::MAX, u32::MAX)
    );
    assert_eq!(full.params, u16::MAX);
    assert_eq!((full.prolog_size, full.saved_registers), (0xff, 7));
    assert!(full.has_seh && full.uses_bp && full.reserved);
    assert_eq!(full.frame, FrameKind::NonFpo);

    let trap = records[2];
    assert_eq!((trap.locals, trap.params), (3, 2));
    assert_eq!((trap.prolog_size, trap.saved_registers), (0x80, 5));
    assert!(!trap.has_seh && trap.uses_bp && !trap.reserved);
    assert_eq!(trap.frame, FrameKind::Trap);

    let tss = records[3];
    assert!(tss.reserved && !tss.has_seh && !tss.uses_bp);
    assert_eq!(tss.frame, FrameKind::Tss);

    assert_eq!(stream.iter().next_back(), Some(tss));
    assert_eq!(stream.iter().len(), 4);
    assert_eq!((&stream).into_iter().count(), 4);
}

#[test]
fn fpo_stream_accepts_empty_and_rejects_partial_records() {
    let empty = FpoStream::parse(Vec::new(), STREAM).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.iter().next(), None);

    let one = fpo(1, 2, 3, 4, 0);
    for extra in 1..FPO_RECORD_SIZE {
        let mut bytes = one.clone();
        bytes.extend(core::iter::repeat_n(0, extra));
        assert_truncated(&FpoStream::parse(bytes, STREAM).unwrap_err(), 16);
    }
    assert_truncated(&FpoStream::parse(vec![0; 15], STREAM).unwrap_err(), 0);
}

#[test]
fn frame_data_records_decode_fields_and_flags() {
    let bytes = [
        frame([0x1000, 290, 52, 0, 0, 0x20], 16, 0, 0b100),
        frame([u32::MAX; 6], u16::MAX, u16::MAX, u32::MAX),
        frame([0x13f0, 410, 152, 4, 8, 0x40], 41, 8, 0b011),
    ]
    .concat();
    let stream = FrameDataStream::parse(bytes, STREAM).unwrap();
    assert_eq!(stream.len(), 3);
    let records: Vec<_> = stream.iter().collect();

    let first = records[0];
    assert_eq!(
        (
            first.start,
            first.size,
            first.locals_size,
            first.params_size
        ),
        (0x1000, 290, 52, 0)
    );
    assert_eq!((first.max_stack_size, first.program), (0, 0x20));
    assert_eq!((first.prolog_size, first.saved_registers_size), (16, 0));
    assert!(first.is_function_start() && !first.has_seh() && !first.has_cpp_eh());

    let full = records[1];
    assert_eq!(full.max_stack_size, u32::MAX);
    assert_eq!(full.program, u32::MAX);
    assert_eq!(
        (full.prolog_size, full.saved_registers_size),
        (u16::MAX, u16::MAX)
    );
    assert_eq!(full.flags, u32::MAX);
    assert!(full.has_seh() && full.has_cpp_eh() && full.is_function_start());

    let third = records[2];
    assert_eq!((third.params_size, third.max_stack_size), (4, 8));
    assert!(third.has_seh() && third.has_cpp_eh() && !third.is_function_start());
    assert_eq!(stream.iter().next_back(), Some(third));
    assert_eq!((&stream).into_iter().len(), 3);
}

#[test]
fn frame_data_stream_has_no_header_and_rejects_partial_records() {
    let empty = FrameDataStream::parse(Vec::new(), STREAM).unwrap();
    assert!(empty.is_empty());

    // LLVM accepts a 4-byte relocation prefix only in object files; the PDB stream has none.
    let mut prefixed = vec![0; 4];
    prefixed.extend(frame([0; 6], 0, 0, 0));
    assert_truncated(&FrameDataStream::parse(prefixed, STREAM).unwrap_err(), 32);
    assert_truncated(
        &FrameDataStream::parse(vec![0; FRAME_DATA_SIZE - 1], STREAM).unwrap_err(),
        0,
    );
}
