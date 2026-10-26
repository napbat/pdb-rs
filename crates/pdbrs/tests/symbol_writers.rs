//! Tests for the module symbol writer and the symbol record stream writer.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::SymbolKind;
use pdbrs::native::codeview::index::ModuleSymbolOffset;
use pdbrs::native::codeview::symbol::{
    ModuleSymbolWriter, ModuleSymbols, S_BLOCK32, S_END, S_GPROC32, S_INLINESITE, S_INLINESITE_END,
    S_PUB32, Symbol, SymbolData,
};
use pdbrs::native::msf::StreamId;
use pdbrs::native::symbols::{SymbolOffset, SymbolRecordWriter};

/// Frame one record with zero padding.
fn record(kind: SymbolKind, payload: &[u8]) -> Vec<u8> {
    let mut payload = payload.to_vec();
    while !(payload.len() + 4).is_multiple_of(4) {
        payload.push(0);
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_le_bytes());
    bytes.extend_from_slice(&kind.0.to_le_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

/// A scope start payload with wrong links: parent 0x55 and end 0x66.
fn scope_payload(kind: SymbolKind) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0x55_u32.to_le_bytes());
    payload.extend_from_slice(&0x66_u32.to_le_bytes());
    match kind {
        S_INLINESITE => payload.extend_from_slice(&0x1001_u32.to_le_bytes()),
        S_BLOCK32 => payload.extend_from_slice(&[0; 10]),
        _ => payload.extend_from_slice(&[0; 27]),
    }
    if kind != S_INLINESITE {
        payload.extend_from_slice(b"f\0");
    }
    payload
}

/// Decode the single record in `bytes`.
fn decode(bytes: &[u8]) -> Symbol<'_> {
    Symbol::decode(Records::new(bytes).next().unwrap().unwrap()).unwrap()
}

#[test]
fn module_writer_sets_scope_links() {
    let procedure = record(S_GPROC32, &scope_payload(S_GPROC32));
    let block = record(S_BLOCK32, &scope_payload(S_BLOCK32));
    let site = record(S_INLINESITE, &scope_payload(S_INLINESITE));
    let end = record(S_END, &[]);
    let site_end = record(S_INLINESITE_END, &[]);
    let mut writer = ModuleSymbolWriter::new(ModuleSymbolOffset(4));
    let mut offsets = Vec::new();
    for bytes in [&procedure, &block, &site, &site_end, &end, &end] {
        let symbol = decode(bytes);
        offsets.push(writer.push(symbol.kind(), symbol.data()).unwrap());
    }
    let out = writer.finish().unwrap();
    // The walker checks every parent and end link.
    let walked: Vec<_> = ModuleSymbols::new(&out, StreamId(9), ModuleSymbolOffset(4), 8)
        .map(Result::unwrap)
        .collect();
    assert_eq!(walked.len(), 6);
    assert_eq!(walked[0].offset, ModuleSymbolOffset(4));
    let links = walked[0].symbol.scope_links().unwrap();
    assert_eq!(
        (links.parent, links.end),
        (ModuleSymbolOffset(0), offsets[5])
    );
    let links = walked[1].symbol.scope_links().unwrap();
    assert_eq!((links.parent, links.end), (offsets[0], offsets[4]));
    let links = walked[2].symbol.scope_links().unwrap();
    assert_eq!((links.parent, links.end), (offsets[1], offsets[3]));
}

#[test]
fn module_writer_rejects_unbalanced_scopes() {
    let end = record(S_END, &[]);
    let site_end = record(S_INLINESITE_END, &[]);
    let procedure = record(S_GPROC32, &scope_payload(S_GPROC32));
    // An end without a scope.
    let mut writer = ModuleSymbolWriter::new(ModuleSymbolOffset(4));
    let symbol = decode(&end);
    let error = writer.push(symbol.kind(), symbol.data()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(writer.next_offset().unwrap(), ModuleSymbolOffset(4));
    // An inline site end for a procedure.
    let symbol = decode(&procedure);
    writer.push(symbol.kind(), symbol.data()).unwrap();
    let symbol = decode(&site_end);
    let error = writer.push(symbol.kind(), symbol.data()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    // A scope without an end.
    assert_eq!(
        writer.finish().unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn module_writer_checks_the_offset_width() {
    let procedure = record(S_GPROC32, &scope_payload(S_GPROC32));
    let symbol = decode(&procedure);
    let mut writer = ModuleSymbolWriter::new(ModuleSymbolOffset(u32::MAX - 8));
    writer.push(symbol.kind(), symbol.data()).unwrap();
    assert_eq!(
        writer
            .push(symbol.kind(), symbol.data())
            .unwrap_err()
            .kind(),
        ErrorKind::OffsetOverflow
    );
}

#[test]
fn record_writer_returns_the_record_offsets() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&2_u32.to_le_bytes());
    payload.extend_from_slice(&0x10_u32.to_le_bytes());
    payload.extend_from_slice(&1_u16.to_le_bytes());
    payload.extend_from_slice(b"main\0");
    let public = record(S_PUB32, &payload);
    let symbol = decode(&public);
    let mut writer = SymbolRecordWriter::new();
    assert_eq!(
        writer.push(symbol.kind(), symbol.data()).unwrap(),
        SymbolOffset(0)
    );
    let second = writer.push(symbol.kind(), symbol.data()).unwrap();
    assert_eq!(second.0 as usize, public.len());
    // A wrong kind leaves the writer unchanged.
    let error = writer.push(S_END, symbol.data()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let stream = writer.finish();
    assert_eq!(stream.offsets, [SymbolOffset(0), second]);
    assert_eq!(stream.bytes, [public.clone(), public].concat());
    let SymbolData::Public(value) = *Symbol::decode(second.record(&stream.bytes).unwrap())
        .unwrap()
        .data()
    else {
        panic!("expected a public");
    };
    assert_eq!(value.name, b"main");
}

/// The `S_LOCAL` record at offset 172 of module stream 10 of the fxc
/// shader model 5.0 vertex shader fixture of #27: the name `input` and 32
/// zero bytes.
fn fxc_local() -> Vec<u8> {
    let mut bytes = vec![
        0x2e, 0x00, 0x3e, 0x11, 0x03, 0x10, 0x00, 0x00, 0x09, 0x00, b'i', b'n', b'p', b'u', b't',
        0x00,
    ];
    bytes.extend_from_slice(&[0; 32]);
    bytes
}

#[test]
fn long_zero_padding_of_fxc_is_kept() {
    let bytes = fxc_local();
    let symbol = decode(&bytes);
    assert_eq!(symbol.name(), Some(&b"input"[..]));
    assert_eq!(symbol.trailing_zeros(), 32);
    let mut out = Vec::new();
    symbol.encode(&mut out).unwrap();
    assert_eq!(out, bytes);
    // The fields alone encode with the shortest padding.
    out.clear();
    symbol.data().encode(symbol.kind(), &mut out).unwrap();
    assert_eq!(out.len(), 16);
    // Both writers keep the producer form.
    let mut module = ModuleSymbolWriter::new(ModuleSymbolOffset(4));
    module.push_symbol(&symbol).unwrap();
    assert_eq!(module.finish().unwrap(), bytes);
    let mut records = SymbolRecordWriter::new();
    records.push_symbol(&symbol).unwrap();
    assert_eq!(records.finish().bytes, bytes);
}

#[test]
fn padding_must_be_zero() {
    let mut bytes = fxc_local();
    bytes[40] = 1;
    let error = Symbol::decode(Records::new(&bytes).next().unwrap().unwrap());
    assert_eq!(error.unwrap_err().kind(), ErrorKind::InvalidFormat);
}
