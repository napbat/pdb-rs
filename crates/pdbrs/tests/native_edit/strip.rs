//! Tests of `PdbEditor::strip_private` (#34).

use pdbrs::ErrorKind;
use pdbrs::native::Pdb;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::symbol::{
    FrameProcedure, FrameProcedureFlags, Procedure, ProcedureFlags, ProcedureReference,
    ProcedureType, Public, PublicFlags, S_END, S_FRAMEPROC, S_LPROC32, S_LPROCREF, SymbolData,
};
use pdbrs::native::dbi::OwnedModule;
use pdbrs::native::info::IdentityPolicy;
use pdbrs::native::write::PdbBuilder;

use super::{T_INT4, build, differing_streams, symbols, synthetic};

/// The name of the local procedure of the second module.
const LOCAL: &[u8] = b"local_helper";

/// Add a module with a local procedure and an `S_LPROCREF` record for it.
fn add_local_procedure(builder: &mut PdbBuilder<'static>) -> pdbrs::Result<()> {
    let module = builder.add_module(OwnedModule {
        unused1: 0,
        section_contribution: pdbrs::native::dbi::SectionContribution {
            section: 1,
            padding1: 0,
            offset: 0x40,
            size: 0x10,
            characteristics: 0x6000_0020,
            module: 1,
            padding2: 0,
            data_crc: Some(0x1234),
            reloc_crc: Some(0),
            coff_section: None,
        },
        flags: 0,
        stream: None,
        symbol_bytes: 0,
        c11_bytes: 0,
        c13_bytes: 0,
        source_file_count: 0,
        padding: 0,
        file_name_offset: 0,
        source_file_name_index: 0,
        pdb_file_path_index: 0,
        name: b"C:\\synthetic\\local.obj".to_vec(),
        object_file: b"C:\\synthetic\\local.obj".to_vec(),
        name_padding: [0; 3],
    })?;
    let procedure = module.push_symbol_fields(
        S_LPROC32,
        SymbolData::Procedure(Procedure {
            parent: 0,
            end: 0,
            next: 0,
            code_length: 0x10,
            debug_start: 0,
            debug_end: 0xf,
            signature: ProcedureType::Type(T_INT4),
            address: SectionOffset {
                section: 1,
                offset: 0x40,
            },
            flags: ProcedureFlags(0),
            extended_flags: None,
            name: LOCAL,
        }),
    )?;
    module.push_symbol_fields(
        S_FRAMEPROC,
        SymbolData::FrameProcedure(FrameProcedure {
            frame_size: 0,
            padding_size: 0,
            padding_offset: 0,
            callee_saved_size: 0,
            exception_handler: SectionOffset {
                section: 0,
                offset: 0,
            },
            flags: FrameProcedureFlags(0),
        }),
    )?;
    module.push_symbol_fields(S_END, SymbolData::ScopeEnd)?;
    let reference = builder.add_symbol_with(move |resolver, out| {
        SymbolData::ProcedureReference(ProcedureReference {
            name_checksum: 0,
            offset: resolver.module_symbol_offset(procedure)?,
            module: resolver.module_number(procedure.module())?,
            name: LOCAL,
        })
        .encode(S_LPROCREF, out)
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
        .map(|record| record.expect("framed").kind)
        .collect();
    assert_eq!(kinds, [S_LPROC32, S_END]);
    let first = stream.symbols().next().expect("record").expect("framed");
    let SymbolData::Procedure(procedure) = *first.symbol().expect("decoded").data() else {
        panic!("procedure");
    };
    assert_eq!(procedure.signature, ProcedureType::Type(TypeIndex::NONE));
    assert_eq!(
        (procedure.end, procedure.name),
        (
            8 + u32::try_from(first.data.len()).expect("record size"),
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
        .find(|record| record.kind == S_LPROCREF)
        .expect("S_LPROCREF");
    let SymbolData::ProcedureReference(reference) = *reference.symbol().expect("decoded").data()
    else {
        panic!("reference");
    };
    assert_eq!((reference.offset, reference.module), (4, 2));
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
        .add_public(&Public {
            flags: PublicFlags(0),
            address: SectionOffset {
                section: 2,
                offset: 0,
            },
            name: b"added",
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
