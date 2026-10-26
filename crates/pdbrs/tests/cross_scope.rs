//! Tests for the reference validation of cross-scope subsections (#31).

use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::dbi::{Dbi, ModuleRegionSizes, ModuleStream, ModuleStreamParts};
use pdbrs::native::lines::{
    CrossScopeContext, CrossScopeExport, CrossScopeImport, CrossScopeModules, CrossScopeRule,
    DEBUG_S_CROSSSCOPEEXPORTS, DEBUG_S_CROSSSCOPEIMPORTS, LineSubsection, ModuleLines,
    validate_cross_scope,
};
use pdbrs::native::names::{HashVersion, StringTable, StringTableBuilder};

/// The module stream index in the synthetic module record.
const MODULE_STREAM: u16 = 5;

/// Build a DBI stream with one module record `m.o` with the region `sizes`.
fn module_dbi(sizes: ModuleRegionSizes) -> Dbi {
    let mut record = vec![0; 64];
    record[34..36].copy_from_slice(&MODULE_STREAM.to_le_bytes());
    record[36..40].copy_from_slice(&sizes.symbol_bytes.to_le_bytes());
    record[40..44].copy_from_slice(&sizes.c11_bytes.to_le_bytes());
    record[44..48].copy_from_slice(&sizes.c13_bytes.to_le_bytes());
    record.extend_from_slice(b"m.o\0m.o\0");
    let mut bytes = vec![0; 64];
    bytes[0..4].copy_from_slice(&(-1_i32).to_le_bytes());
    bytes[4..8].copy_from_slice(&19_990_903_u32.to_le_bytes());
    bytes[16..18].copy_from_slice(&0xffff_u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&u32::try_from(record.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&record);
    Dbi::parse(bytes).unwrap()
}

/// Build a module stream with the framed subsections `(kind, data)`.
fn module_stream(subsections: &[(u32, Vec<u8>)]) -> ModuleStream {
    let mut c13 = Vec::new();
    for (kind, data) in subsections {
        LineSubsection::decode(*kind, data)
            .unwrap()
            .encode_framed(&mut c13)
            .unwrap();
    }
    let parts = ModuleStreamParts {
        symbols: None,
        c11: &[],
        c13: &c13,
        global_refs: &[],
    };
    let encoded = parts.encode().unwrap();
    let dbi = module_dbi(encoded.sizes);
    let module = dbi.modules().next().unwrap().unwrap();
    ModuleStream::parse(encoded.bytes, &module).unwrap()
}

/// Return the data of an export subsection with `(local, global)` pairs.
fn exports(pairs: &[(u32, u32)]) -> Vec<u8> {
    let mut data = Vec::new();
    for &(local, global) in pairs {
        CrossScopeExport::new(local, global)
            .encode(&mut data)
            .unwrap();
    }
    data
}

/// Return the data of an import subsection with `(name offset, IDs)` entries.
fn imports(entries: &[(u32, &[u32])]) -> Vec<u8> {
    let mut data = Vec::new();
    for &(offset, ids) in entries {
        CrossScopeImport::encode_parts(offset, ids, &mut data).unwrap();
    }
    data
}

/// Return the rules, entries and values of the violations.
fn findings(
    stream: &ModuleStream,
    context: &CrossScopeContext<'_>,
) -> Vec<(CrossScopeRule, usize, u32)> {
    let lines = ModuleLines::parse(stream).unwrap();
    validate_cross_scope(&lines, context)
        .unwrap()
        .iter()
        .map(|violation| (violation.rule, violation.entry, violation.value))
        .collect()
}

/// The TPI end 0x1010 and the IPI end 0x1004 of the tests.
fn ends() -> CrossScopeContext<'static> {
    CrossScopeContext::new(TypeIndex(0x1010), IdIndex(0x1004))
}

#[test]
fn checks_export_global_ids_by_stream() {
    let stream = module_stream(&[(
        DEBUG_S_CROSSSCOPEEXPORTS,
        exports(&[
            (0x0000_0001, 0x1000),
            (0x0000_0002, 0x0074),
            (0x8000_0003, 0x1003),
            (0x0000_0004, 0x100f),
            (0x0000_0005, 0x1010),
            (0x8000_0006, 0x1004),
            (0x8000_0007, 0x0074),
            (0x0000_0008, 0x00ff),
            (0x0000_0001, 0x1001),
        ]),
    )]);
    assert_eq!(
        findings(&stream, &ends()),
        [
            (CrossScopeRule::ExportGlobalId, 4, 0x1010),
            (CrossScopeRule::ExportGlobalId, 5, 0x1004),
            (CrossScopeRule::ExportGlobalId, 6, 0x0074),
            (CrossScopeRule::ExportGlobalId, 7, 0x00ff),
            (CrossScopeRule::ExportDuplicate, 8, 1),
        ]
    );
    // A `NOTM` or `MINI` PDB has unmapped global IDs.
    assert_eq!(
        findings(&stream, &ends().without_export_mapping()),
        [(CrossScopeRule::ExportDuplicate, 8, 1)]
    );
}

#[test]
fn checks_imports_against_names_and_module_exports() {
    let mut builder = StringTableBuilder::new(HashVersion::V1).unwrap();
    let exporter = builder.insert(b"C:\\OBJ\\A.OBJ").unwrap();
    let silent = builder.insert(b"b.obj").unwrap();
    let missing = builder.insert(b"c.obj").unwrap();
    let mut names_bytes = Vec::new();
    builder.encode(&mut names_bytes).unwrap();
    let names = StringTable::parse(&names_bytes).unwrap();

    let export_data = exports(&[(0x8000_0002, 0x1001), (7, 0x1002)]);
    let export_stream = module_stream(&[(DEBUG_S_CROSSSCOPEEXPORTS, export_data)]);
    let export_lines = ModuleLines::parse(&export_stream).unwrap();
    let Some(LineSubsection::CrossScopeExports(list)) = export_lines
        .subsections()
        .first()
        .map(|subsection| &subsection.data)
    else {
        panic!("expected exports");
    };
    let mut modules = CrossScopeModules::new();
    modules
        .add(b"a.obj", b"c:\\obj\\a.obj", Some(list))
        .unwrap();
    modules.add(b"b.obj", b"lib.lib", None).unwrap();
    assert_eq!(modules.len(), 2);

    let stream = module_stream(&[(
        DEBUG_S_CROSSSCOPEIMPORTS,
        imports(&[
            (exporter, &[0x8000_0002, 7, 8]),
            (silent, &[1, 2]),
            (missing, &[1]),
            (0x7fff, &[1]),
        ]),
    )]);
    let context = ends().with_names(&names).with_modules(&modules);
    assert_eq!(
        findings(&stream, &context),
        [
            (CrossScopeRule::ImportNotExported, 0, 8),
            (CrossScopeRule::ImportModuleUnknown, 2, missing),
            (CrossScopeRule::ImportModuleName, 3, 0x7fff),
        ]
    );
    // Without the module list, only the name references are checked.
    assert_eq!(
        findings(&stream, &ends().with_names(&names)),
        [(CrossScopeRule::ImportModuleName, 3, 0x7fff)]
    );
    // Without `/names`, the imports are not checked.
    assert_eq!(findings(&stream, &ends()), []);
}
