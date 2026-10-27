//! Tests of `PdbEditor::strip_private` (#34).

use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::index::{ModuleIndex, ModuleSymbolOffset, TypeIndex};
use pdbrs::native::codeview::symbol::{
    FrameProcedure, FrameProcedureFlags, ModuleSymbolTarget, Procedure, ProcedureFlags,
    ProcedureReference, ProcedureType, Public, PublicFlags, S_END, S_FRAMEPROC, S_LPROC32,
    S_LPROCREF, Symbol, SymbolData,
};
use pdbrs::native::codeview::{Records, SymbolKind};
use pdbrs::native::dbi::Module;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbBuilder};
use pdbrs::{Bytes, ErrorKind};

use super::{T_INT4, build, differing_streams, symbols, synthetic};

/// The name of the local procedure of the second module.
const LOCAL: &[u8] = b"local_helper";

/// Add a module with a local procedure and an `S_LPROCREF` record for it.
fn add_local_procedure(builder: &mut PdbBuilder<'static>) -> pdbrs::Result<()> {
    let mut contribution = pdbrs::native::dbi::SectionContribution::default();
    contribution.section = 1;
    contribution.padding1 = 0;
    contribution.offset = 0x40;
    contribution.size = 0x10;
    contribution.characteristics = 0x6000_0020;
    contribution.module = ModuleIndex(1);
    contribution.padding2 = 0;
    contribution.data_crc = Some(0x1234);
    contribution.reloc_crc = Some(0);
    contribution.coff_section = None;
    let mut record = Module::default();
    record.unused1 = 0;
    record.section_contribution = contribution;
    record.flags = 0;
    record.stream = None;
    record.symbol_bytes = 0;
    record.c11_bytes = 0;
    record.c13_bytes = 0;
    record.source_file_count = 0;
    record.padding = 0;
    record.file_name_offset = 0;
    record.source_file_name_index = 0;
    record.pdb_file_path_index = 0;
    record.name = Bytes::new(b"C:\\synthetic\\local.obj");
    record.object_file = Bytes::new(b"C:\\synthetic\\local.obj");
    record.name_padding = [0; 3];
    let module = builder.add_module(record)?;
    let mut local = Procedure::new(ProcedureType::Type(T_INT4));
    local.next = ModuleSymbolOffset(0);
    local.code_length = 0x10;
    local.debug_start = 0;
    local.debug_end = 0xf;
    local.address = SectionOffset {
        section: 1,
        offset: 0x40,
    };
    local.flags = ProcedureFlags(0);
    local.extended_flags = None;
    local.name = Bytes::new(LOCAL);
    let procedure = module.push_symbol_fields(S_LPROC32, SymbolData::Procedure(local))?;
    let mut frame = FrameProcedure::default();
    frame.frame_size = 0;
    frame.padding_size = 0;
    frame.padding_offset = 0;
    frame.callee_saved_size = 0;
    frame.exception_handler = SectionOffset {
        section: 0,
        offset: 0,
    };
    frame.flags = FrameProcedureFlags(0);
    module.push_symbol_fields(S_FRAMEPROC, SymbolData::FrameProcedure(frame))?;
    module.push_symbol_fields(S_END, SymbolData::ScopeEnd)?;
    let mut reference = ProcedureReference::default();
    reference.name_checksum = 0;
    reference.target = ModuleSymbolTarget::Handle(procedure);
    reference.name = Bytes::new(LOCAL);
    let reference =
        builder.add_symbol_fields(S_LPROCREF, SymbolData::ProcedureReference(reference))?;
    builder.index_global(reference, 1)
}

#[test]
fn strip_keeps_publics_and_local_procedures() {
    let source = build(add_local_procedure);
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let source_identity = pdb.info().identity();
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    let plan = editor.plan(EditOptions::default()).expect("plan");
    let edited = plan.to_vec().expect("output");
    let changed: Vec<u32> = plan.changed_streams().iter().map(|id| id.0).collect();
    assert_eq!(differing_streams(&source, &edited), changed);
    let stripped = Pdb::parse(edited.as_slice()).expect("stripped");
    assert_eq!(stripped.info().identity(), source_identity);
    assert_eq!(stripped.info().named_stream(b"/names"), None);
    for types in [stripped.types(), stripped.ids()] {
        let types = types.expect("type stream").expect("present");
        assert_eq!(types.header().record_count(), 0);
    }
    let dbi = stripped.debug_info().expect("DBI").expect("DBI present");
    assert!(dbi.header().stripped());
    let modules: Vec<_> = dbi
        .modules()
        .map(|module| module.expect("module").1)
        .collect();
    // The first module has only global procedures: no module stream.
    assert_eq!(modules[0].stream, None);
    assert_eq!(modules[1].section_contribution.data_crc, Some(0));
    let stream = stripped
        .module_stream(&modules[1])
        .expect("module stream")
        .expect("present");
    let kinds: Vec<_> = stream
        .symbols()
        .map(|record| SymbolKind(record.expect("framed").kind))
        .collect();
    assert_eq!(kinds, [S_LPROC32, S_END]);
    let first = stream.symbols().next().expect("record").expect("framed");
    let symbol = Symbol::decode(first).expect("decoded");
    let SymbolData::Procedure(procedure) = symbol.data() else {
        panic!("procedure");
    };
    assert_eq!(procedure.signature, ProcedureType::Type(TypeIndex::NONE));
    assert_eq!(&procedure.name[..], LOCAL);
    assert_eq!(
        symbol.links().expect("links").end,
        ModuleSymbolOffset(8 + u32::try_from(first.data.len()).expect("record size"))
    );
    let symbols = symbols(&edited);
    assert_eq!(symbols.globals, [LOCAL.to_vec()]);
    assert_eq!(symbols.publics.len(), 3);
    // The S_LPROCREF record names the new offset of the procedure.
    let records = stripped
        .stream_bytes(dbi.header().symbol_stream.expect("symbols"))
        .expect("bytes");
    let reference = Records::new(&records)
        .map(|record| record.expect("framed"))
        .find(|record| SymbolKind(record.kind) == S_LPROCREF)
        .expect("S_LPROCREF");
    let SymbolData::ProcedureReference(reference) =
        Symbol::decode(reference).expect("decoded").data().clone()
    else {
        panic!("reference");
    };
    assert_eq!(
        reference.target,
        ModuleSymbolTarget::Position {
            module: Some(ModuleIndex(1)),
            offset: ModuleSymbolOffset(4),
        }
    );
    let refs: Vec<_> = stream.global_refs().expect("refs").iter().collect();
    assert_eq!(refs.len(), 1);
}

#[test]
fn strip_of_a_stripped_pdb_changes_nothing() {
    let source = build(add_local_procedure);
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    let once = editor
        .plan(EditOptions::default())
        .expect("plan")
        .to_vec()
        .expect("output");
    let pdb = Pdb::parse(once.as_slice()).expect("stripped");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(plan.changed_streams(), []);
}

#[test]
fn strip_with_another_edit_is_unsupported() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    editor
        .add_public(&{
            let mut public = Public::default();
            public.flags = PublicFlags(0);
            public.address = SectionOffset {
                section: 2,
                offset: 0,
            };
            public.name = Bytes::new(b"added");
            public
        })
        .expect("recorded");
    assert_eq!(
        editor
            .plan(EditOptions::default())
            .expect_err("combined edit")
            .kind(),
        ErrorKind::UnsupportedEdit
    );
}

#[test]
fn strip_with_an_opaque_stream_is_rejected() {
    let source = build(|builder| builder.add_stream(vec![0; 8]).map(|_| ()));
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    assert_eq!(
        editor
            .plan(EditOptions::default())
            .expect_err("opaque stream")
            .kind(),
        ErrorKind::UnsupportedEdit
    );
}

#[test]
fn strip_removes_an_unowned_type_map() {
    let source = build(|builder| {
        builder
            .add_stream(super::UNOWNED_TYPE_MAP.to_vec())
            .map(|_| ())
    });
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let last = pdb.container().stream_count() - 1;
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    let plan = editor.plan(EditOptions::default()).expect("type map");
    assert!(
        plan.changed_streams()
            .contains(&pdbrs::native::msf::StreamId(last))
    );
    let edited = plan.to_vec().expect("output");
    let edited = Pdb::parse(edited.as_slice()).expect("stripped");
    assert!(super::stream(&edited, last).is_none());
}
