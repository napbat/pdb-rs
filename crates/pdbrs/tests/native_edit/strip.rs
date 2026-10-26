//! Tests of `PdbEditor::strip_private` (#34).

use pdbrs::ErrorKind;
use pdbrs::native::IdentityPolicy;
use pdbrs::native::Pdb;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::index::{ModuleIndex, ModuleSymbolOffset, TypeIndex};
use pdbrs::native::codeview::symbol::{
    FrameProcedure, FrameProcedureFlags, Procedure, ProcedureFlags, ProcedureReference,
    ProcedureType, Public, PublicFlags, S_END, S_FRAMEPROC, S_LPROC32, S_LPROCREF, Symbol,
    SymbolData,
};
use pdbrs::native::codeview::{Records, SymbolKind};
use pdbrs::native::dbi::OwnedModule;
use pdbrs::native::write::PdbBuilder;

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
    let mut record = OwnedModule::default();
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
    record.name = b"C:\\synthetic\\local.obj".to_vec();
    record.object_file = b"C:\\synthetic\\local.obj".to_vec();
    record.name_padding = [0; 3];
    let module = builder.add_module(record)?;
    let mut local = Procedure::new(ProcedureType::Type(T_INT4));
    local.parent = ModuleSymbolOffset(0);
    local.end = ModuleSymbolOffset(0);
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
    local.name = LOCAL;
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
    let reference = builder.add_symbol_with(move |resolver, out| {
        let number = resolver.module_number(procedure.module())?;
        let mut reference = ProcedureReference::default();
        reference.name_checksum = 0;
        reference.offset = ModuleSymbolOffset(resolver.module_symbol_offset(procedure)?);
        // The module number is the module index plus 1.
        reference.module = Some(ModuleIndex(u32::from(number) - 1));
        reference.name = LOCAL;
        SymbolData::ProcedureReference(reference).encode(S_LPROCREF, out)
    })?;
    builder.index_global(reference, 1)
}

#[test]
fn strip_keeps_publics_and_local_procedures() {
    let source = build(add_local_procedure);
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let source_identity = pdb.info().identity();
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    let plan = editor.plan().expect("plan");
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
        .map(|module| module.expect("module"))
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
    let SymbolData::Procedure(procedure) = *Symbol::decode(first).expect("decoded").data() else {
        panic!("procedure");
    };
    assert_eq!(procedure.signature, ProcedureType::Type(TypeIndex::NONE));
    assert_eq!(
        (procedure.end, procedure.name),
        (
            ModuleSymbolOffset(8 + u32::try_from(first.data.len()).expect("record size")),
            LOCAL
        )
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
        *Symbol::decode(reference).expect("decoded").data()
    else {
        panic!("reference");
    };
    assert_eq!(
        (reference.offset, reference.module),
        (ModuleSymbolOffset(4), Some(ModuleIndex(1)))
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
    let once = editor.plan().expect("plan").to_vec().expect("output");
    let pdb = Pdb::parse(once.as_slice()).expect("stripped");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    let plan = editor.plan().expect("plan");
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
            public.name = b"added";
            public
        })
        .expect("recorded");
    assert_eq!(
        editor.plan().expect_err("combined edit").kind(),
        ErrorKind::UnsupportedVersion
    );
}

#[test]
fn strip_with_an_opaque_stream_is_rejected() {
    let source = build(|builder| builder.add_stream(vec![0; 8]).map(|_| ()));
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    assert_eq!(
        editor.plan().expect_err("opaque stream").kind(),
        ErrorKind::InvalidReference
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
    let plan = editor.plan().expect("type map");
    assert!(
        plan.changed_streams()
            .contains(&pdbrs::native::msf::StreamId(last))
    );
    let edited = plan.to_vec().expect("output");
    let edited = Pdb::parse(edited.as_slice()).expect("stripped");
    assert!(super::stream(&edited, last).is_none());
}
