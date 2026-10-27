//! A module whose symbols do not form a scope tree.
//!
//! A byte patch changes the `S_END` record that closes the `S_GPROC32` scope
//! of `sum` into an `S_INLINESITE_END` record: a closer of another kind.

use pdbrs::native::codeview::index::{ModuleIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{
    S_END, S_INLINESITE_END, S_UDT, SymbolData, UserDefinedType,
};
use pdbrs::native::editor::ModuleSymbol;
use pdbrs::native::lines::{DEBUG_S_LINES, LineBlock, LineEntry, LineSubsection, ModuleLines};
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, StreamChange};
use pdbrs::{Bytes, Error, ErrorKind};

use crate::layout::Layout;
use crate::{stream, synthetic};

/// The module of the synthetic program.
const MODULE: ModuleIndex = ModuleIndex(0);

/// Return the synthetic PDB with the closer patch, and the module stream.
pub(crate) fn opaque_module() -> (Vec<u8>, StreamId) {
    let mut bytes = synthetic();
    let (id, at) = {
        let pdb = Pdb::parse(bytes.as_slice()).expect("source");
        let module = pdb.module(MODULE).expect("module").expect("module stream");
        let end = module
            .symbols()
            .map(|record| record.expect("framed record"))
            .find(|record| record.kind == S_END.0)
            .expect("S_END record");
        // The kind field follows the 2-byte length field.
        let offset = module.symbol_offset() + end.offset as usize + 2;
        let id = module.stream();
        (id, Layout::parse(&bytes).file_offset(id.0, offset))
    };
    bytes[at..at + 2].copy_from_slice(&S_INLINESITE_END.0.to_le_bytes());
    (bytes, id)
}

/// Check that `error` rejects a symbol edit of the opaque module stream `id`.
fn assert_opaque(error: &Error, id: StreamId) {
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");
    assert_eq!(error.stream_location().map(|(stream, _)| stream), Some(id));
}

/// Return an `S_UDT` record of `T_INT4`.
fn udt() -> SymbolData<'static> {
    let mut udt = UserDefinedType::default();
    udt.type_index = TypeIndex(0x74);
    udt.name = Bytes::new(b"added");
    SymbolData::UserDefinedType(udt)
}

#[test]
fn symbol_edits_of_an_opaque_module_fail() {
    // Without the patch, the module decodes into a scope tree.
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let module = editor.module_mut(MODULE).expect("module");
    let scopes = module
        .symbols()
        .expect("scope tree")
        .iter()
        .filter(|symbol| matches!(symbol, ModuleSymbol::Scope(_)))
        .count();
    // The scopes of `sum` and `main`.
    assert_eq!(scopes, 2);

    let (source, id) = opaque_module();
    let pdb = Pdb::parse(source.as_slice()).expect("patched source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let module = editor.module_mut(MODULE).expect("opaque module decodes");
    assert_opaque(&module.symbols().expect_err("symbols"), id);
    assert_opaque(&module.symbols_mut().expect_err("symbols_mut"), id);
    assert_opaque(
        &module.procedure_mut(b"main").expect_err("procedure_mut"),
        id,
    );
    assert_opaque(&module.push(S_UDT, udt()).expect_err("push"), id);

    // A plan without a symbol edit copies the module stream.
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(
        plan.report().output(id).map(|entry| entry.change),
        Some(StreamChange::Copied)
    );
    assert_eq!(plan.report().changed().count(), 0);
    let output = plan.to_vec().expect("output");
    let edited = Pdb::parse(output.as_slice()).expect("output parses");
    assert_eq!(stream(&edited, id.0), stream(&pdb, id.0));
}

/// Return `lines`, a `DEBUG_S_LINES` subsection, with the first line number
/// plus 1, and that new line number.
fn next_line(lines: &LineSubsection<'_>) -> (LineSubsection<'static>, u32) {
    let LineSubsection::Lines(subsection) = lines else {
        panic!("not a line subsection");
    };
    let mut data = Vec::new();
    subsection.header().encode(&mut data).expect("header");
    let mut first = None;
    for block in subsection.blocks() {
        assert!(
            block.columns.is_none(),
            "the synthetic lines have no columns"
        );
        let mut entries: Vec<LineEntry> = block.lines.iter().collect();
        if first.is_none() {
            let entry = entries.first_mut().expect("line entry");
            entry.line_start += 1;
            first = Some(entry.line_start);
        }
        LineBlock::encode_parts(block.checksum_offset, &entries, None, &mut data).expect("block");
    }
    let changed = LineSubsection::decode(DEBUG_S_LINES, &data)
        .expect("decoded")
        .into_owned();
    (changed, first.expect("line block"))
}

/// Return the first line number of the first `DEBUG_S_LINES` subsection of
/// `lines`.
fn first_line(lines: &[LineSubsection<'_>]) -> u32 {
    lines
        .iter()
        .find_map(|subsection| match subsection {
            LineSubsection::Lines(lines) => lines.blocks().next(),
            _ => None,
        })
        .and_then(|block| block.lines.iter().next())
        .expect("line entry")
        .line_start
}

#[test]
fn lines_of_an_opaque_module_stay_editable() {
    let (source, id) = opaque_module();
    let pdb = Pdb::parse(source.as_slice()).expect("patched source");
    let source_module = pdb.module(MODULE).expect("module").expect("stream");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let lines = editor.module_mut(MODULE).expect("module").lines_mut();
    let position = lines
        .iter()
        .position(|subsection| matches!(subsection, LineSubsection::Lines(_)))
        .expect("line subsection");
    let (changed, line) = next_line(&lines[position]);
    lines[position] = changed;

    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(
        plan.report().output(id).map(|entry| entry.change),
        Some(StreamChange::Edited)
    );
    let output = plan.to_vec().expect("output");
    let edited = Pdb::parse(output.as_slice()).expect("output parses");
    let module = edited.module(MODULE).expect("module").expect("stream");
    assert_eq!(module.stream(), id);
    // The plan copies the symbol records of the opaque module.
    assert_eq!(module.symbol_data(), source_module.symbol_data());
    let lines = ModuleLines::parse(&module).expect("lines");
    let subsections: Vec<LineSubsection<'_>> = lines
        .subsections()
        .iter()
        .map(|subsection| subsection.data.clone())
        .collect();
    assert_eq!(first_line(&subsections), line);
}
