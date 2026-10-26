# Native PDB coverage

This document gives the coverage of modern native PDBs (MSF 7.0) for each record kind, stream version and container feature.
It is product documentation: it tells what the library reads, checks and writes today. GitHub issues hold the scope and the evidence.
The epic [#1](https://github.com/napbat/pdbrs/issues/1) owns the native scope, and [#28](https://github.com/napbat/pdbrs/issues/28) owns this accounting.

Legacy native PDBs ([#16](https://github.com/napbat/pdbrs/issues/16)), MSFZ ([#20](https://github.com/napbat/pdbrs/issues/20)) and Portable PDBs ([#17](https://github.com/napbat/pdbrs/issues/17)) have their own epics.
Legacy kinds and versions have rows here, marked `not applicable (legacy, #16)`, so that the inventory is complete. MSFZ and Portable PDB have no rows.

## Pages

| Page | Contents |
| --- | --- |
| [MSF container and PDB streams](native-coverage/streams.md) | Page sizes, free page maps, stream directory, stream 0; PDB Info versions and features; TPI and IPI versions and hash streams; DBI versions, flags, substreams and section contribution versions; optional debug streams; address translation; `/names` versions; GSI and PSI layouts; named and auxiliary streams |
| [Types, IDs and numeric leaves](native-coverage/types.md) | Type leaves, extensions after `cvinfo.h`, field-list members, ID leaves, numeric leaves, primitive kinds and pointer modes, padding, markers, aliases, legacy kinds |
| [Symbol records](native-coverage/symbols.md) | Symbol kinds, extensions after `cvinfo.h`, `S_OEM` payload forms, markers, aliases, legacy kinds |
| [C13 line data and module streams](native-coverage/lines.md) | Module stream regions, C13 subsection kinds, the ignore bit, subsection variants |
| [Reconciliation: LLVM](native-coverage/reconciliation-llvm.md) | The 12 files of `docs/reference/llvm/` |
| [Reconciliation: pdb-rs PDB documents](native-coverage/reconciliation-pdb.md) | `microsoft-pdb-rs/docs/pdb/`, the top-level pdb-rs files, `microsoft-pdb/` and `SOURCES.json` |
| [Reconciliation: pdb-rs CodeView types](native-coverage/reconciliation-codeview-types.md) | `microsoft-pdb-rs/docs/codeview/` top level, `types/` and `items/` |
| [Reconciliation: pdb-rs CodeView symbols](native-coverage/reconciliation-codeview-symbols.md) | `microsoft-pdb-rs/docs/codeview/symbols/` |

## Inventory sources

The rows come from the pinned implementation definitions, not from the prose documents alone:

- `cvinfo.h` at `microsoft/microsoft-pdb@805655a2`: `LEAF_ENUM_e`, `TYPE_ENUM_e`, `SYM_ENUM_e` and `DEBUG_S_SUBSECTION_TYPE`.
- LLVM at `b9f2235f`: `CodeViewTypes.def`, `CodeViewSymbols.def`, `CodeView.h` (`DebugSubsectionKind`), `TypeIndex.h` and `RawConstants.h`.
- `ms-codeview` 0.1.7: `types/kind.rs` and `syms/kind.rs`.
- The prose documents in [`docs/reference/`](../reference/README.md). The reconciliation pages compare them with the implementation.

A kind that only a later source defines (for example `S_FRAMEREG`, `LF_CLASS2` or `DEBUG_S_XFGHASH_TYPE`) is in a separate table of extensions after `cvinfo.h`.
A second name for the same value (for example `S_SLOT`) is in a separate alias row.

## Columns

Each row has six status columns:

| Column | Meaning |
| --- | --- |
| Raw preservation | The reader gives the exact bytes of the item, and exact copies and repacks keep them. Raw framing alone does not make semantic reading complete |
| Semantic reading | A typed decoder reads every field of the item |
| Reference validation | The library or the `native` example checks every reference that the item holds: type and ID indices, stream IDs, `/names` offsets, scope links, module indices and addresses |
| Writing | An encoder writes the item from its decoded form |
| Editing and index rebuilding | An editor changes the item and rebuilds the indexes that depend on it (hashes, offsets, address maps) |
| Independent fixture evidence | A PDB from an independent producer holds the item, and an independent consumer (`llvm-pdbutil` 22.1.6, DIA2Dump or `pdbtool`, #21) agrees with the decoded values |

## Values

| Value | Meaning |
| --- | --- |
| `complete` | The cell links the implementation (`code`) and a consumer-visible check (`check`): an issue evidence comment or a test file |
| `partial` | Some of the work exists. The cell names the open issue and the reason |
| `missing` | The work does not exist. The cell names the open issue |
| `not applicable` | The column does not apply. The cell gives the reason, for example `legacy, #16`, `no references` or `marker, not a record` |

A recognized version that pdbrs does not decode gives `UnsupportedVersion`. Corrupt data gives `InvalidFormat`, `InvalidReference` or `UnexpectedEof`. The two classes stay distinct.
A top-level record of a kind without a decoder keeps its kind and payload (`TypeLeaf::Unsupported`, `SymbolData::Unsupported`, `LineSubsection::Unsupported`).
A field-list member of such a kind stops the decoding of its list, because members have no length field.

## Fixture evidence

The fixture counts in the notes come from the native PDBs of the fixture manifest (`cargo run -p pdbrs-fixtures`, #12 and #27): 28 PDBs of 27 fixtures.
A read of every record with the pdbrs reader at `c14bed9` gave the counts. The `native` example does not print counts for each kind.
`msvc-x64-srcsrv/demo.srcsrv.pdb` is not in the counts: the reader stops at its age check (#32).

The fixtures are: `clang-link-x64-debug`, `clang-lld-arm64-debug`, `clang-lld-x64-debug`, `clang-lld-x64-release`, `clang-lld-x86-debug`, `dotnet-csharp-full`, `msvc-arm64-debug`, `msvc-x64-clr`, `msvc-x64-debug`, `msvc-x64-debugtype`, `msvc-x64-dll`, `msvc-x64-fastlink`, `msvc-x64-incremental`, `msvc-x64-natvis`, `msvc-x64-pch-z7`, `msvc-x64-pch-zi`, `msvc-x64-pgo`, `msvc-x64-release`, `msvc-x64-srcsrv`, `msvc-x64-stripped`, `msvc-x64-xfg`, `msvc-x64-z7`, `msvc-x86-debug`, `rust-link-x64-debug`, `rust-link-x64-release`, `rust-link-x86-debug`, `rust-lld-x64-debug`.

A fixture cell is `complete` only if an evidence comment names the kind and an independent consumer comparison for it.
A kind in a fixture without such a comparison is `partial`. Kinds that no consumer decodes (for example `S_FRAMEREG`) are `partial`, with the independent check that confirmed their layout.

## Summary

Each cell gives `complete / partial / missing / not applicable`.

| Page | Rows | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| [MSF container and PDB streams](native-coverage/streams.md) | 113 | 94 / 0 / 0 / 19 | 61 / 12 / 21 / 19 | 47 / 3 / 23 / 40 | 16 / 0 / 74 / 23 | 16 / 0 / 74 / 23 | 47 / 15 / 22 / 29 |
| [Types, IDs and numeric leaves](native-coverage/types.md) | 256 | 159 / 20 / 0 / 77 | 60 / 78 / 41 / 77 | 4 / 10 / 57 / 185 | 0 / 0 / 179 / 77 | 0 / 0 / 114 / 142 | 30 / 57 / 92 / 77 |
| [Symbol records](native-coverage/symbols.md) | 223 | 136 / 0 / 0 / 87 | 57 / 2 / 76 / 88 | 26 / 15 / 79 / 103 | 0 / 0 / 136 / 87 | 0 / 0 / 136 / 87 | 51 / 12 / 72 / 88 |
| [C13 line data and module streams](native-coverage/lines.md) | 32 | 31 / 0 / 0 / 1 | 15 / 1 / 13 / 3 | 14 / 0 / 13 / 5 | 0 / 0 / 30 / 2 | 0 / 0 / 30 / 2 | 10 / 3 / 16 / 3 |
| Total | 624 | 420 / 20 / 0 / 184 | 193 / 93 / 151 / 187 | 91 / 28 / 172 / 333 | 16 / 0 / 419 / 189 | 16 / 0 / 354 / 254 | 138 / 87 / 202 / 197 |

Writing is complete only for the MSF container ([#15](https://github.com/napbat/pdbrs/issues/15)). The native record and stream encoders are open in #29 to #32, and the builder and editor in #33 and #34.

## Reference reconciliation

Every imported reference document has a section on a reconciliation page. The table gives the result for each document.
"pdbrs" is `implemented`, `partial`, `not implemented` or `not applicable` (no format content). The counts are the listed conflicts and undocumented assumptions.

| Document | pdbrs | Conflicts | Undocumented assumptions |
| --- | --- | ---: | ---: |
| [`llvm/llvm/docs/PDB/index.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbindexmd) | implemented | 5 | 3 |
| [`llvm/llvm/docs/PDB/MsfFile.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmsffilemd) | implemented | 7 | 6 |
| [`llvm/llvm/docs/PDB/PdbStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbpdbstreammd) | implemented | 4 | 3 |
| [`llvm/llvm/docs/PDB/TpiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbtpistreammd) | implemented | 8 | 3 |
| [`llvm/llvm/docs/PDB/DbiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbdbistreammd) | partial | 9 | 5 |
| [`llvm/llvm/docs/PDB/ModiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmodistreammd) | partial | 3 | 3 |
| [`llvm/llvm/docs/PDB/PublicStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbpublicstreammd) | implemented | 4 | 4 |
| [`llvm/llvm/docs/PDB/GlobalStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbglobalstreammd) | implemented | 3 | 4 |
| [`llvm/llvm/docs/PDB/HashTable.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbhashtablemd) | implemented | 3 | 2 |
| [`llvm/llvm/docs/PDB/CodeViewTypes.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbcodeviewtypesmd) | partial | 5 | 4 |
| [`llvm/llvm/docs/PDB/CodeViewSymbols.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbcodeviewsymbolsmd) | partial | 9 | 7 |
| [`llvm/llvm/LICENSE.TXT`](native-coverage/reconciliation-llvm.md#llvmllvmlicensetxt) | not applicable | 0 | 0 |
| [`microsoft-pdb-rs/docs/pdb/tpi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbtpi_streammd) | implemented | 11 | 5 |
| [`microsoft-pdb-rs/docs/pdb/relationships.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbrelationshipsmd) | partial | 4 | 2 |
| [`microsoft-pdb-rs/docs/pdb/pdbi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbpdbi_streammd) | implemented | 9 | 6 |
| [`microsoft-pdb-rs/docs/pdb/pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbpdbmd) | partial | 3 | 4 |
| [`microsoft-pdb-rs/docs/pdb/names_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbnames_streammd) | partial | 7 | 2 |
| [`microsoft-pdb-rs/docs/pdb/msf.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfmd) | implemented | 9 | 6 |
| [`microsoft-pdb-rs/docs/pdb/msfz.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfzmd) | not implemented | 11 | 0 |
| [`microsoft-pdb-rs/docs/pdb/mini_pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmini_pdbmd) | partial | 5 | 3 |
| [`microsoft-pdb-rs/docs/pdb/module_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmodule_streammd) | partial | 3 | 3 |
| [`microsoft-pdb-rs/docs/pdb/ipi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbipi_streammd) | implemented | 3 | 3 |
| [`microsoft-pdb-rs/docs/pdb/hashing.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbhashingmd) | partial | 6 | 1 |
| [`microsoft-pdb-rs/docs/pdb/globals.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbglobalsmd) | implemented | 10 | 5 |
| [`microsoft-pdb-rs/docs/pdb/dbi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_streammd) | implemented | 6 | 2 |
| [`microsoft-pdb-rs/docs/pdb/dbi_sources.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_sourcesmd) | implemented | 4 | 2 |
| [`microsoft-pdb-rs/docs/pdb/dbi_sections.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_sectionsmd) | implemented | 3 | 3 |
| [`microsoft-pdb-rs/docs/pdb/dbi_opt_debug.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_opt_debugmd) | partial | 4 | 3 |
| [`microsoft-pdb-rs/docs/pdb/dbi_modules.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_modulesmd) | partial | 3 | 2 |
| [`microsoft-pdb-rs/docs/pdb/dbi_fixups.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_fixupsmd) | not implemented | 2 | 0 |
| [`microsoft-pdb-rs/docs/terminology.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsterminologymd) | not applicable | 2 | 0 |
| [`microsoft-pdb-rs/docs/references.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsreferencesmd) | not applicable | 4 | 0 |
| [`microsoft-pdb-rs/docs/intro.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsintromd) | not applicable | 1 | 0 |
| [`microsoft-pdb-rs/docs/determinism.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsdeterminismmd) | partial | 2 | 1 |
| [`microsoft-pdb-rs/docs/index.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsindexmd) | not applicable | 0 | 0 |
| [`microsoft-pdb-rs/docs/data_types.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsdata_typesmd) | partial | 2 | 0 |
| [`microsoft-pdb-rs/LICENSE`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rslicense) | not applicable | 0 | 0 |
| [`microsoft-pdb/README.md`](native-coverage/reconciliation-pdb.md#microsoft-pdbreadmemd) | not applicable | 3 | 0 |
| [`microsoft-pdb/docs/ExternalResources.md`](native-coverage/reconciliation-pdb.md#microsoft-pdbdocsexternalresourcesmd) | not applicable | 1 | 0 |
| [`microsoft-pdb/LICENSE`](native-coverage/reconciliation-pdb.md#microsoft-pdblicense) | not applicable | 0 | 0 |
| [`SOURCES.json`](native-coverage/reconciliation-pdb.md#sourcesjson) | not applicable | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/primitive_types.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewprimitive_typesmd) | implemented | 7 | 2 |
| [`microsoft-pdb-rs/docs/codeview/number.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewnumbermd) | partial | 5 | 2 |
| [`microsoft-pdb-rs/docs/codeview/line_data.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewline_datamd) | partial | 8 | 5 |
| [`microsoft-pdb-rs/docs/codeview/codeview.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewcodeviewmd) | partial | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/calling_convention.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewcalling_conventionmd) | implemented | 4 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/types.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypestypesmd) | partial | 4 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_union.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_unionmd) | implemented | 1 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_vtshape.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_vtshapemd) | implemented | 3 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_skip.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_skipmd) | not implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_procedure.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_proceduremd) | implemented | 2 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_precomp.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_precompmd) | not implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_modifier.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_modifiermd) | implemented | 2 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_pointer.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_pointermd) | partial | 5 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_mfunction.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_mfunctionmd) | implemented | 4 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_methodlist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_methodlistmd) | implemented | 3 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_fieldlist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_fieldlistmd) | partial | 7 | 3 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_enum.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_enummd) | implemented | 1 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_endprecomp.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_endprecompmd) | not implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_class.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_classmd) | implemented | 3 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_bitfield.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_bitfieldmd) | implemented | 0 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_array.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_arraymd) | implemented | 3 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_arglist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_arglistmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/items/items.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemsitemsmd) | implemented | 2 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_buildinfo.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_buildinfomd) | implemented | 4 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_func_id.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_func_idmd) | implemented | 5 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_mfunc_id.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_mfunc_idmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_string_id.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_string_idmd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_substr_list.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_substr_listmd) | implemented | 2 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_udt_src_line.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_udt_src_linemd) | implemented | 1 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_udt_mod_src_line.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_udt_mod_src_linemd) | implemented | 2 | 1 |
| [`microsoft-pdb-rs/docs/codeview/symbols/symbols.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolssymbolsmd) | partial | 9 | 7 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_annotation.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_annotationmd) | not implemented | 3 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_armswitchtable.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_armswitchtablemd) | implemented | 3 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_block.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_blockmd) | implemented | 0 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_buildinfo.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_buildinfomd) | implemented | 0 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_callsiteinfo.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_callsiteinfomd) | implemented | 2 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_coffgroup.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_coffgroupmd) | implemented | 0 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_compile.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_compilemd) | implemented | 6 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_constant.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_constantmd) | partial | 2 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_data.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_datamd) | implemented | 2 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_end.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_endmd) | implemented | 2 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_frameproc.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_frameprocmd) | implemented | 3 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_inlinesite.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_inlinesitemd) | partial | 5 | 3 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_label.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_labelmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_local.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_localmd) | partial | 10 | 4 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_namespace.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_namespacemd) | implemented | 0 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_objname.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_objnamemd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_procs.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_procsmd) | partial | 10 | 3 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_pub.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_pubmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_refsyms.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_refsymsmd) | partial | 4 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_section.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_sectionmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_thread.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_threadmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_thunk.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_thunkmd) | implemented | 3 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_trampoline.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_trampolinemd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_udt.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_udtmd) | implemented | 2 | 0 |

`docs/reference/README.md` and `docs/reference/.gitattributes` are local files, not imported documents.

### Main conflicts

These conflicts affect the implementation directly. The reconciliation pages give all of them.

1. FPO `fHasSEH`: LLVM `COFF.h` reads bit 9 (and `fUseBP` bit 10, `cbProlog` 4 bits). `winnt.h` and pdbrs use bit 11 (bit 12, 8 bits). 7 of 8 x86 fixture records have the word `0x0800` (#13). See [`DbiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbdbistreammd) and [`dbi_opt_debug.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_opt_debugmd).
2. Page sizes: LLVM `MsfFile.md` allows 512 to 4096. LLVM code accepts 512 to 32768, and pdb-rs `msf.md` and `ms-pdb-msf` accept 512 to 65536. The pdbrs reader accepts 512 to 65536. The writer rejects 65536 because DIA, LLVM and `link.exe` reject it (#15). See [`MsfFile.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmsffilemd) and [`msf.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfmd).
3. Page-map pages: pdbrs reads and writes more than one page-map page. LLVM rejects such a file ("Too many directory blocks").
4. MSFZ: `msfz.md` and `ms-pdb-msfz` 0.1.10 differ on the directory size check, the compression codes (code 2, raw DEFLATE) and the reserved bits. #20 follows the reference implementation. See [`msfz.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfzmd).
5. FASTLINK buckets: Microsoft `GSI1::hashSz` truncates the bucket to 16 bits. `ms-pdb` 0.1.22 does not, and LLVM always uses 4096 buckets. pdbrs follows Microsoft and finds all 836 public records of `msvc-x64-fastlink` (#13). See [`mini_pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmini_pdbmd).
6. Named-stream map: LLVM `PdbStream.md` and pdb-rs `pdbi_stream.md` omit the single `niMac` word after the map. An early pdbrs build read it as feature code 0 (#9).
7. TPI hash adjusters: LLVM `TpiStream.md` calls the keys hash values. Microsoft `tpi.cpp`, LLVM code and pdbrs read them as `/names` offsets (#3).
8. Section contributions: Microsoft `dbi.cpp` and pdbrs read a substream without the V60 or V2 version as headerless V40 records. LLVM rejects it, and no document describes it.
9. DBI version: pdb-rs `dbi_stream.md` calls V110 current. Microsoft `dbi.h` and every fixture use V70, the only version that pdbrs accepts.
10. Symbol padding: pdb-rs `symbols.md` and the `ms-codeview` builder use `f3 f2 f1`. LLD and the fixtures use zero bytes, and pdbrs requires zero bytes. See [`symbols.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolssymbolsmd).
11. Kind values in pdb-rs documents: `S_GDATA32`/`S_LDATA32` given as `0x1007`/`0x1008` (the `_ST` values), `S_INLINESITE2` as `0x115c` (`S_POGODATA`), `LF_ARRAY` as `0x1502` (`LF_ENUMERATE`), `LF_FRIENDCLS` as `0x040b` (the 16-bit value).
12. Marker values that later sources reuse: `cvinfo.h` `S_RECTYPE_MAX` is `0x1166`, which LLVM uses for `S_FRAMEREG`; `LF_TYPE_LAST` `0x151e` is LLVM `LF_TAGGED_UNION`; `LF_ID_LAST` `0x1608` is `LF_CLASS2`.
13. Primitive kind `0xf0` (`T_NCVPTR`): `cvinfo.h` defines it with pointer modes. pdbrs returns `InvalidFormat` for it (#29).
14. Ages: pdb-rs `dbi_stream.md` tells readers to accept a DBI age that differs from the Info age. pdbrs returns `InvalidFormat`, and `pdbstr` writes such files (`msvc-x64-srcsrv`, #27, #32).
15. Evidence record: one #13 comment says that pdbrs truncates the FASTLINK bucket "as `ms-pdb` does". The `ms-pdb` 0.1.22 source does not truncate; the #13 decision comment is correct.

### Undocumented assumptions

pdbrs uses these layouts and rules, and no imported document states them:

- `S_FRAMEREG` (`0x1166`): `u16` register, then 2 zero bytes. Confirmed from `msvc-arm64-debug` bytes (#13).
- `S_FASTLINK` (`0x1167`): `u32` value, `u16` flags, name. `REFMINIPDB` without the module field; flag bit 6 marks namespaces. Confirmed from `msvc-x64-fastlink` and its object files (#13).
- `S_REGREL32_ENCTMP` (`0x1179`): the `S_REGREL32` layout. Confirmed from `msvc-x64-incremental` (#13).
- `DEBUG_S_COFF_SYMBOL_RVA` (`0xfd`): an array of `u32` image RVAs, one for each COFF symbol record; `0` means no address and `0xffffffff` an absolute symbol (#13).
- `/TMCache` version `0x20200229` and its type map streams (versions 1 and 2): from `DBI1::DumpTMCache` of `mspdbcore.dll` 14.51 (#13).
- `/UDTSRCLINEUNDONE`: a list of `u32` type indices (#9).
- The FASTLINK bucket rule `(hash % 0x3ffff) & 0xffff` (#13).
- MSVC writes 12 optional debug header slots; slot 11 has no name and is nil in every fixture.
- The trampoline join: thunk index `(thunk offset - thunk_table_offset) / thunk_size`, and the thunk map entry is the section start of the target plus the target offset (#13).
- Scope links are module stream offsets that include the 4-byte signature (#5).
- The `S_OEM` attributes of the managed OEM ID `c6ea3fc9-59b3-49d6-bc25-0902bbabb460` (`MD2`, `asyncMethodInfo`) follow Roslyn and Mono.Cecil, not a PDB document (#14).

## External dependencies

pdbrs does not load external files. These items name data outside the PDB, and pdbrs does not resolve them:

| Item | Target | State |
| --- | --- | --- |
| Non-empty DBI type server map, `LF_TYPESERVER2` | A type server PDB | `UnsupportedVersion` for the map; the leaf stays raw (#32, #29) |
| `LF_PRECOMP`, `LF_ENDPRECOMP` | Precompiled header types in an object file | Raw; the fixtures keep them in the objects only (#27) |
| `S_FASTLINK`, `S_REF_MINIPDB`, `DEBUG_S_COFF_SYMBOL_RVA` | Object files of a FASTLINK link | Values decoded; targets not resolved (#32) |
| `/TMCache` checksums | Type sections of object files | Kept; cannot be checked |
| `S_MANSLOT`, `S_MANCONSTANT`, `S_TOKENREF`, `S_OEM` tokens | Metadata of the managed assembly | Kept |
| `sourcelink`, `srcsrv` | Source servers and URLs | Bytes kept; nothing is fetched |

#32 adds an API that lists these dependencies.

## Scope decisions

| Format | Decision |
| --- | --- |
| MSFZ containers | Required; separate codec (#20) |
| Small MSF and other legacy versions and kinds | Required, deferred (#16) |
| 65536-byte pages | Read; not written, because consumers reject them (#15) |
| Edits in place and incremental MSF commits | Not planned ([design](pdb.md#implemented-native-reading)) |
| OMAP translation | Required (#32); no local producer, so the evidence needs `external-input` |
| Compressed injected sources (run-length, Huffman, LZ) | Required (#32); no producer known |
| Natvis XML, Source Link JSON and `srcsrv` text contents | Decision required (#32): pdbrs keeps the bytes; interpretation of the text formats is not part of the PDB format |
| Producer extensions without a layout (LLVM-only kinds, `S_OEM` of other OEM IDs) | Keep raw; decode when a source or fixture gives the layout (#29, #30) |
