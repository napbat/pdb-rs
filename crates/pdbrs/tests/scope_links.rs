//! Tests of the scope links and the reference targets (#42 stage 4).
//!
//! The scope records do not hold their parent and end links: `Symbol::links`
//! reads them from the source record. A reference record holds a
//! `ModuleSymbolTarget`.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::{ModuleIndex, ModuleSymbolOffset};
use pdbrs::native::codeview::symbol::{
    ModuleSymbolTarget, ModuleSymbolWriter, ProcedureReference, Public, S_BLOCK32, S_END,
    S_GPROC32, S_INLINESITE, S_PROCREF, S_PUB32, ScopeLinks, Symbol, SymbolData,
};
use pdbrs::native::codeview::{Records, SymbolKind};
use pdbrs::native::dbi::Module;
use pdbrs::native::{BuildOptions, PdbBuilder};

/// A scope record from a fixture with the links that `llvm-pdbutil` prints.
struct RealRecord {
    /// The fixture and the module stream offset of the record.
    source: &'static str,
    /// The record kind.
    kind: SymbolKind,
    /// The payload after the kind, as `llvm-pdbutil dump -sym-data` prints it.
    payload: &'static [u8],
    /// The parent link.
    parent: u32,
    /// The end link.
    end: u32,
}

/// Return scope records of real PDBs.
fn real_records() -> [RealRecord; 4] {
    [
        RealRecord {
            source: "msvc-x64-debug/demo.pdb, S_GPROC32 `main` at 204",
            kind: S_GPROC32,
            payload: &[
                0x00, 0x00, 0x00, 0x00, 0x70, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5c, 0x01,
                0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x4a, 0x01, 0x00, 0x00, 0x31, 0x10, 0x00, 0x00,
                0x10, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, b'm', b'a', b'i', b'n', 0x00,
            ],
            parent: 0,
            end: 368,
        },
        RealRecord {
            source: "msvc-x64-debug/demo.pdb, S_BLOCK32 at 1488",
            kind: S_BLOCK32,
            payload: &[
                0xe0, 0x04, 0x00, 0x00, 0x68, 0x06, 0x00, 0x00, 0x91, 0x00, 0x00, 0x00, 0x81, 0x05,
                0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            ],
            parent: 1248,
            end: 1640,
        },
        RealRecord {
            source: "msvc-x64-debug/demo.pdb, S_BLOCK32 at 1592",
            kind: S_BLOCK32,
            payload: &[
                0xd0, 0x05, 0x00, 0x00, 0x64, 0x06, 0x00, 0x00, 0x51, 0x00, 0x00, 0x00, 0xbf, 0x05,
                0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            ],
            parent: 1488,
            end: 1636,
        },
        RealRecord {
            source: "clang-lld-x64-release/demo.pdb, S_INLINESITE at 804",
            kind: S_INLINESITE,
            payload: &[
                0x48, 0x02, 0x00, 0x00, 0xbc, 0x03, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x06, 0x14,
                0x03, 0x40, 0x04, 0x30, 0x00, 0x00,
            ],
            parent: 584,
            end: 956,
        },
    ]
}

/// Frame one record.
fn framed(kind: SymbolKind, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len() + 2).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&kind.0.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

/// Decode the single record in `bytes`.
fn decode(bytes: &[u8]) -> Symbol<'_> {
    Symbol::decode(Records::new(bytes).next().unwrap().unwrap()).unwrap()
}

#[test]
fn real_scope_records_give_their_links() {
    for record in real_records() {
        let bytes = framed(record.kind, record.payload);
        let symbol = decode(&bytes);
        let links = symbol.links().expect(record.source);
        assert_eq!(
            (links.parent, links.end),
            (
                ModuleSymbolOffset(record.parent),
                ModuleSymbolOffset(record.end)
            ),
            "{}",
            record.source
        );
        // The decoded record encodes to its bytes with the same links.
        let mut out = Vec::new();
        symbol.encode(&mut out).unwrap();
        assert_eq!(out, bytes, "{}", record.source);
    }
}

#[test]
fn records_that_do_not_start_a_scope_have_no_links() {
    let mut out = Vec::new();
    SymbolData::ScopeEnd.encode(S_END, &mut out).unwrap();
    assert_eq!(decode(&out).links(), None);
    let reference = position_reference();
    out.clear();
    SymbolData::ProcedureReference(reference)
        .encode(S_PROCREF, &mut out)
        .unwrap();
    assert_eq!(decode(&out).links(), None);
}

#[test]
fn scope_records_encode_only_with_given_links() {
    for record in real_records() {
        let bytes = framed(record.kind, record.payload);
        let symbol = decode(&bytes);
        let mut out = Vec::new();
        let error = symbol
            .data()
            .encode(record.kind, &mut out)
            .expect_err(record.source);
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{}", record.source);
        out.clear();
        symbol
            .data()
            .encode_scope(record.kind, symbol.links().unwrap(), &mut out)
            .unwrap();
        assert_eq!(out, bytes, "{}", record.source);
        // Other links give other bytes: the encoder writes what it gets.
        out.clear();
        let links = ScopeLinks::new(ModuleSymbolOffset(4), ModuleSymbolOffset(8));
        symbol
            .data()
            .encode_scope(record.kind, links, &mut out)
            .unwrap();
        assert_eq!(decode(&out).links(), Some(links), "{}", record.source);
    }
    let mut out = Vec::new();
    let error = SymbolData::ScopeEnd
        .encode_scope(S_END, ScopeLinks::default(), &mut out)
        .expect_err("S_END does not start a scope");
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}

#[test]
fn module_writer_replaces_the_links_of_real_records() {
    let records = real_records();
    let procedure = &records[0];
    let block = &records[1];
    let mut writer = ModuleSymbolWriter::new(ModuleSymbolOffset(4));
    let procedure_bytes = framed(procedure.kind, procedure.payload);
    let block_bytes = framed(block.kind, block.payload);
    let outer = writer.push_symbol(&decode(&procedure_bytes)).unwrap();
    let inner = writer.push_symbol(&decode(&block_bytes)).unwrap();
    let inner_end = writer.push(S_END, &SymbolData::ScopeEnd).unwrap();
    let outer_end = writer.push(S_END, &SymbolData::ScopeEnd).unwrap();
    let out = writer.finish().unwrap();
    let links: Vec<_> = Records::new(&out)
        .map(|record| Symbol::decode(record.unwrap()).unwrap().links())
        .collect();
    assert_eq!(
        links,
        [
            Some(ScopeLinks::new(ModuleSymbolOffset(0), outer_end)),
            Some(ScopeLinks::new(outer, inner_end)),
            None,
            None
        ]
    );
    assert_eq!(inner, ModuleSymbolOffset(4 + 44));
}

/// Return an `S_PROCREF` record with a position target.
fn position_reference() -> ProcedureReference<'static> {
    let mut reference = ProcedureReference::default();
    reference.name_checksum = 0;
    reference.target = ModuleSymbolTarget::Position {
        module: Some(ModuleIndex(2)),
        offset: ModuleSymbolOffset(396),
    };
    reference.name = b"main".into();
    reference
}

#[test]
fn procedure_reference_with_a_position_target_round_trips() {
    // The layout of `REFSYM2`: checksum, offset, module number (index + 1), name.
    let mut payload = Vec::new();
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&396_u32.to_le_bytes());
    payload.extend_from_slice(&3_u16.to_le_bytes());
    payload.extend_from_slice(b"main\0\0");
    let bytes = framed(S_PROCREF, &payload);
    let data = SymbolData::ProcedureReference(position_reference());
    let mut out = Vec::new();
    data.encode(S_PROCREF, &mut out).unwrap();
    assert_eq!(out, bytes);
    let symbol = decode(&bytes);
    assert_eq!(*symbol.data(), data);
    out.clear();
    symbol.encode(&mut out).unwrap();
    assert_eq!(out, bytes);
}

#[test]
fn reference_with_an_unresolved_handle_does_not_encode() {
    let mut builder = PdbBuilder::new(1, [1; 16], 1).unwrap();
    let module = builder.add_module(Module::default()).unwrap();
    let handle = module
        .push_symbol_fields(S_PUB32, SymbolData::Public(Public::default()))
        .unwrap();
    assert_eq!(handle.module(), ModuleIndex(0));
    let mut reference = position_reference();
    reference.target = ModuleSymbolTarget::Handle(handle);
    let mut out = Vec::new();
    let error = SymbolData::ProcedureReference(reference.clone())
        .encode(S_PROCREF, &mut out)
        .expect_err("only a plan resolves a handle");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert!(
        error.to_string().contains("only a write plan resolves it"),
        "{error}"
    );
    // The module writer has no plan either.
    let mut writer = ModuleSymbolWriter::new(ModuleSymbolOffset(4));
    let error = writer
        .push(S_PROCREF, &SymbolData::ProcedureReference(reference))
        .expect_err("the module writer does not resolve a handle");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(writer.finish().unwrap(), Vec::<u8>::new());
}

#[test]
fn plan_rejects_a_handle_target_in_a_module_stream() {
    let mut builder = PdbBuilder::new(1, [1; 16], 1).unwrap();
    let module = builder.add_module(Module::default()).unwrap();
    let handle = module
        .push_symbol_fields(S_PUB32, SymbolData::Public(Public::default()))
        .unwrap();
    let mut reference = position_reference();
    reference.target = ModuleSymbolTarget::Handle(handle);
    module
        .push_symbol_fields(S_PROCREF, SymbolData::ProcedureReference(reference))
        .unwrap();
    let error = builder
        .plan(BuildOptions::default())
        .expect_err("module offsets are not known when the plan encodes a module stream");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn symbol_data_encode_keeps_non_scope_records() {
    let mut public = Public::default();
    public.name = b"f".into();
    let mut out = Vec::new();
    SymbolData::Public(public)
        .encode(S_PUB32, &mut out)
        .unwrap();
    assert_eq!(decode(&out).links(), None);
}
