//! Each IPI record kind: exact re-encoding and encoder limits.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::id_record::{
    IdList, IdRecord, LF_BUILDINFO, LF_FUNC_ID, LF_MFUNC_ID, LF_STRING_ID, LF_SUBSTR_LIST,
    LF_UDT_MOD_SRC_LINE, LF_UDT_SRC_LINE,
};
use pdbrs::native::codeview::index::{IdIndex, ModuleIndex};
use pdbrs::native::codeview::{LeafKind, Record};

fn u32le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// Frame a payload; pad it to 4 bytes if `pad` is `true`.
fn record(kind: LeafKind, payload: &[u8], pad: bool) -> Vec<u8> {
    let mut body = kind.0.to_le_bytes().to_vec();
    body.extend_from_slice(payload);
    let count = if pad {
        (4 - (body.len() + 2) % 4) % 4
    } else {
        0
    };
    for left in (1..=count).rev() {
        body.push(0xf0 + u8::try_from(left).unwrap());
    }
    let mut full = u16::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    full.extend_from_slice(&body);
    full
}

fn round_trip(full: &[u8]) -> IdRecord<'_> {
    let decoded = IdRecord::decode(Record::new(
        0,
        u16::from_le_bytes([full[2], full[3]]),
        &full[4..],
    ))
    .unwrap();
    let mut out = Vec::new();
    decoded.encode(&mut out).unwrap();
    assert_eq!(out, full);
    assert_eq!(decoded.kind().0, u16::from_le_bytes([full[2], full[3]]));
    decoded
}

#[test]
fn every_id_kind_re_encodes() {
    for full in [
        record(
            LF_FUNC_ID,
            &[&u32le(0)[..], &u32le(0x1000), b"f\0"].concat(),
            true,
        ),
        record(
            LF_FUNC_ID,
            &[&u32le(0x1001)[..], &u32le(0x1000), b"g\0", &[1; 8]].concat(),
            true,
        ),
        record(
            LF_MFUNC_ID,
            &[&u32le(0x1002)[..], &u32le(0x1003), b"m\0"].concat(),
            true,
        ),
        record(
            LF_MFUNC_ID,
            &[&u32le(0x1002)[..], &u32le(0x1003), b"m\0", &[2; 8]].concat(),
            true,
        ),
        record(
            LF_BUILDINFO,
            &[&[2, 0][..], &u32le(0x1004), &u32le(0)].concat(),
            true,
        ),
        record(
            LF_SUBSTR_LIST,
            &[&u32le(2)[..], &u32le(0x1004), &u32le(0x1005)].concat(),
            true,
        ),
        record(LF_STRING_ID, &[&u32le(0)[..], b"text\0"].concat(), true),
        record(
            LF_STRING_ID,
            &[&u32le(0x1006)[..], b"tail\0"].concat(),
            true,
        ),
        record(
            LF_UDT_SRC_LINE,
            &[&u32le(0x1007)[..], &u32le(0x1008), &u32le(12)].concat(),
            true,
        ),
        record(LeafKind(0x1520), &[1, 2, 3], false),
    ] {
        round_trip(&full);
    }
}

#[test]
fn udt_module_source_line_keeps_its_padding_form() {
    let payload = [&u32le(0x1007)[..], &u32le(0x40), &u32le(12), &[3, 0]].concat();
    // Microsoft `link.exe` writes 18 bytes without padding.
    let unpadded = record(LF_UDT_MOD_SRC_LINE, &payload, false);
    assert_eq!(unpadded.len(), 18);
    let IdRecord::UdtModuleSourceLine(line) = round_trip(&unpadded) else {
        panic!("not a UDT source line");
    };
    assert!(!line.padded);
    assert_eq!(line.module, Some(ModuleIndex(2)));
    // LLVM pads its records to 4 bytes.
    let padded = record(LF_UDT_MOD_SRC_LINE, &payload, true);
    assert_eq!(padded.len(), 20);
    let IdRecord::UdtModuleSourceLine(line) = round_trip(&padded) else {
        panic!("not a UDT source line");
    };
    assert!(line.padded);
}

#[test]
fn id_lists_check_their_lengths() {
    assert_eq!(
        IdList::parse(&[0; 6]).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let list = IdList::parse(&[1, 0x10, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(
        list.iter().collect::<Vec<_>>(),
        [IdIndex(0x1001), IdIndex(0)]
    );
    assert_eq!(list.as_bytes().len(), 8);
    // A list that makes the record longer than its `u16` length is an error.
    let data = [&u32le(0x1_0000)[..], &vec![0; 4 * 0x1_0000]].concat();
    let decoded = IdRecord::decode(Record::new(0, LF_SUBSTR_LIST.0, &data)).unwrap();
    let IdRecord::SubstringList(list) = decoded else {
        panic!("not a list");
    };
    let mut out = Vec::new();
    let error = decoded.encode(&mut out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    assert_eq!(list.len(), 0x1_0000);
}
