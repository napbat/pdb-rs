# Native PDB coverage

This document gives the coverage of modern native PDBs (MSF 7.0) for each record kind, stream version and container feature.
It is product documentation: it tells what the library reads, checks, writes and edits today. GitHub issues hold the scope and the evidence.
The epic [#1](https://github.com/napbat/pdbrs/issues/1) owns the native scope, and [#28](https://github.com/napbat/pdbrs/issues/28) owns this accounting.

Legacy native PDBs ([#16](https://github.com/napbat/pdbrs/issues/16)), MSFZ ([#20](https://github.com/napbat/pdbrs/issues/20)) and Portable PDBs ([#17](https://github.com/napbat/pdbrs/issues/17)) have their own epics.
Legacy kinds and versions have rows here, marked `not applicable (legacy, #16)`, so that the inventory is complete. MSFZ and Portable PDB have no rows.

## Pages

| Page | Contents |
| --- | --- |
| [MSF container and PDB streams](native-coverage/streams.md) | Page sizes, free page maps, stream directory, stream 0; PDB Info versions, identity and features; TPI and IPI versions and hash streams; DBI versions, flags, substreams and section contribution versions; optional debug streams; address translation and OMAP; `/names` versions; GSI and PSI layouts; named and auxiliary streams |
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
- The debug type records of `cvinfo.h` in the DIA SDK `diaguids.lib` (Visual Studio 18) for the symbol kinds above `0x1168`, for example `S_HYBRIDRANGE` (#30).
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
| Writing | An encoder writes the item from its decoded form, to the same bytes for a decoded item |
| Editing and index rebuilding | `PdbBuilder` (#33) or `PdbEditor` (#34) writes or changes the item and rebuilds the streams and indexes that depend on it (hashes, offsets, address maps) |
| Independent fixture evidence | A PDB from an independent producer holds the item, and an independent consumer (`llvm-pdbutil` 22.1.6, DIA2Dump, `pdbtool` or `llvm-symbolizer`, #21) agrees with pdbrs |

## Values

| Value | Meaning |
| --- | --- |
| `complete` | The cell links the implementation (`code`) and a consumer-visible check (`check`): an issue evidence comment or a test file |
| `partial` | Some of the work exists. The cell names the open issue and the reason |
| `missing` | The work does not exist. The cell names the open issue and the reason |
| `not applicable` | The column does not apply. The cell gives the reason, for example `legacy, #16`, `no references` or `marker, not a record` |

`external-input` in a cell means that the work needs input that is not available here: a producer that writes the item, or a definition of its layout. The issue carries the `external-input` label.

A recognized version that pdbrs does not decode gives `UnsupportedVersion`. Corrupt data gives `InvalidFormat`, `InvalidReference` or `UnexpectedEof`. The two classes stay distinct.
A top-level record of a kind without a decoder keeps its kind and payload (`TypeLeaf::Unsupported`, `SymbolData::Unsupported`, `LineSubsection::Unsupported`), and its encoder writes the bytes again.

## Fixture evidence

The fixture counts in the notes come from the native PDBs of the fixture manifest (`cargo run -p pdbrs-fixtures`, #12 and #27): 36 PDBs of 31 fixtures.
A read of every record with the pdbrs reader at `8dc001c` gave the counts. The `native` example does not print counts for each kind.
The 3 PDBs of `hlsl-dxc-debug` are not in the counts: their MSF header declares fewer pages than the streams use, and `pdbtool`, DIA and pdbrs reject them (#27).

The fixtures are: `clang-link-x64-debug`, `clang-lld-arm64-debug`, `clang-lld-x64-debug`, `clang-lld-x64-release`, `clang-lld-x86-debug`, `dotnet-csharp-full`, `hlsl-dxc-debug`, `hlsl-fxc-sm50-debug`, `hlsl-fxc-sm51-debug`, `msvc-arm64-debug`, `msvc-arm64-debugtype`, `msvc-x64-clr`, `msvc-x64-debug`, `msvc-x64-debugtype`, `msvc-x64-dll`, `msvc-x64-fastlink`, `msvc-x64-incremental`, `msvc-x64-natvis`, `msvc-x64-pch-z7`, `msvc-x64-pch-zi`, `msvc-x64-pgo`, `msvc-x64-release`, `msvc-x64-srcsrv`, `msvc-x64-stripped`, `msvc-x64-xfg`, `msvc-x64-z7`, `msvc-x86-debug`, `rust-link-x64-debug`, `rust-link-x64-release`, `rust-link-x86-debug`, `rust-lld-x64-debug`.

A fixture cell is `complete` if a fixture PDB holds the item and an evidence comment records an independent consumer check of it:

- a comparison of decoded values with `llvm-pdbutil` or DIA2Dump (#2 to #14), or
- the round trip of #29 to #32: pdbrs decodes and encodes every record and stream of every fixture to the same bytes, and copies with the rebuilt streams give the same `llvm-pdbutil` and DIA2Dump output and pass `pdbtool check` and `pdbtool compare`, or
- the semantic rebuild of #33 (`cargo run -p pdbrs-fixtures --release -- verify-native-build`: 36 of 36 PDBs) and the edits of #34 (`verify-native-edit`: 216 edits, 0 problems without an explanation).

A kind that only object files hold (for example `LF_PRECOMP` or `DEBUG_S_FRAMEDATA`) is `partial`: pdbrs decodes and encodes the object file data, but no PDB fixture holds it.
A kind in no fixture is `missing` with `external-input` and the producer that can make it, where one is known.

## Summary

Each cell gives `complete / partial / missing / not applicable`.

| Page | Rows | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| [MSF container and PDB streams](native-coverage/streams.md) | 114 | 94 / 0 / 0 / 20 | 87 / 0 / 8 / 19 | 55 / 2 / 8 / 49 | 83 / 4 / 4 / 23 | 83 / 4 / 4 / 23 | 59 / 8 / 22 / 25 |
| [Types, IDs and numeric leaves](native-coverage/types.md) | 256 | 179 / 0 / 0 / 77 | 174 / 0 / 0 / 82 | 62 / 0 / 0 / 194 | 166 / 0 / 0 / 90 | 96 / 0 / 0 / 160 | 80 / 3 / 78 / 95 |
| [Symbol records](native-coverage/symbols.md) | 224 | 137 / 0 / 0 / 87 | 128 / 0 / 4 / 92 | 96 / 12 / 4 / 112 | 129 / 4 / 0 / 91 | 129 / 4 / 0 / 91 | 68 / 0 / 64 / 92 |
| [C13 line data and module streams](native-coverage/lines.md) | 32 | 31 / 0 / 0 / 1 | 28 / 0 / 1 / 3 | 16 / 2 / 1 / 13 | 30 / 1 / 0 / 1 | 29 / 1 / 0 / 2 | 14 / 5 / 10 / 3 |
| Total | 626 | 441 / 0 / 0 / 185 | 417 / 0 / 13 / 196 | 229 / 16 / 13 / 368 | 408 / 9 / 4 / 205 | 337 / 9 / 4 / 276 | 221 / 16 / 174 / 215 |

## Remaining open cells

Each `missing` or `partial` cell of a modern or extension row names its issue. The table groups them by issue and column.
Most cells are fixture evidence for kinds that no local producer writes (`external-input`).

| Issue | Column | Cells | Rows |
| --- | --- | ---: | --- |
| #1 | Independent fixture evidence | 4 | Page size 8192, Page size 16384, Page size 32768, Free page map intervals after the first |
| #29 | Semantic reading | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Reference validation | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Writing | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Editing and index rebuilding | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Independent fixture evidence | 83 | Hash stream: hash adjusters, Auxiliary hash stream (`hash_aux_stream`), `LF_ENDPRECOMP`, `LF_COBOL0`, `LF_BARRAY`, `LF_VFTPATH`, `LF_OEM`, `LF_OEM2`, `LF_SKIP`, `LF_DERIVED`, `LF_DIMCONU`, `LF_DIMCONLU`, `LF_DIMVARU`, `LF_DIMVARLU`, `LF_TYPESERVER`, `LF_DIMARRAY`, `LF_PRECOMP`, `LF_ALIAS`, `LF_DEFARG`, `LF_MANAGED`, `LF_TYPESERVER2`, `LF_INTERFACE`, `LF_CLASS2`, `LF_STRUCTURE2`, `LF_INTERFACE2`, `LF_UNION2`, `LF_FRIENDCLS`, `LF_VFUNCOFF`, `LF_FRIENDFCN`, `LF_NESTTYPEEX`, `LF_MEMBERMODIFY`, `LF_BINTERFACE`, `LF_REAL32`, `LF_REAL80`, `LF_REAL128`, `LF_REAL48`, `LF_COMPLEX32`, `LF_COMPLEX64`, `LF_COMPLEX80`, `LF_COMPLEX128` and 43 more |
| #30 | Semantic reading | 4 | `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_ALTOBJNAME` |
| #30 | Reference validation | 16 | `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_DEFRANGE`, `S_DEFRANGE_SUBFIELD`, `S_DEFRANGE_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL`, `S_DEFRANGE_SUBFIELD_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE`, `S_DEFRANGE_REGISTER_REL`, `S_DEFRANGE_HLSL`, `S_DEFRANGE_DPC_PTR_TAG`, `S_DEFRANGE_REGISTER_REL_INDIR`, `S_DEFRANGE_CONSTVAL_ON_ENTRY`, `S_DEFRANGE_GLOBALSYM_ON_ENTRY`, `S_ALTOBJNAME` |
| #30 | Writing | 4 | `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_ALTOBJNAME` |
| #30 | Editing and index rebuilding | 4 | `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_ALTOBJNAME` |
| #30 | Independent fixture evidence | 66 | Feature `PdbFeatureMinimalDebugInfo` (`MINI`), Minimal bucket count (`MINI` feature), `S_VFTABLE32`, `S_ANNOTATION`, `S_MANTYPREF`, `S_WITH32`, `S_COBOLUDT`, `S_MANYREG`, `S_LPROCMIPS`, `S_GPROCMIPS`, `S_MANYREG2`, `S_LPROCIA64`, `S_GPROCIA64`, `S_LOCALSLOT`, `S_PARAMSLOT`, `S_LMANDATA`, `S_MANFRAMEREL`, `S_MANREGISTER`, `S_MANMANYREG`, `S_MANREGREL`, `S_MANMANYREG2`, `S_DATAREF`, `S_ANNOTATIONREF`, `S_LMANPROC`, `S_ATTR_FRAMEREL`, `S_ATTR_REGISTER`, `S_ATTR_REGREL`, `S_ATTR_MANYREG`, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_DISCARDED`, `S_DEFRANGE`, `S_DEFRANGE_SUBFIELD`, `S_LPROC32_ID`, `S_GPROC32_ID`, `S_LPROCMIPS_ID`, `S_GPROCMIPS_ID`, `S_LPROCIA64_ID`, `S_GPROCIA64_ID` and 26 more |
| #31 | Semantic reading | 1 | `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Reference validation | 3 | `DEBUG_S_CROSSSCOPEIMPORTS`, `DEBUG_S_CROSSSCOPEEXPORTS`, `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Writing | 1 | `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Editing and index rebuilding | 1 | `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Independent fixture evidence | 15 | Symbol region absent (size 0), `DEBUG_S_SYMBOLS`, `DEBUG_S_STRINGTABLE`, `DEBUG_S_FRAMEDATA`, `DEBUG_S_CROSSSCOPEIMPORTS`, `DEBUG_S_CROSSSCOPEEXPORTS`, `DEBUG_S_IL_LINES`, `DEBUG_S_FUNC_MDTOKEN_MAP`, `DEBUG_S_TYPE_MDTOKEN_MAP`, `DEBUG_S_MERGED_ASSEMBLYINPUT`, `DEBUG_S_INLINEE_MDTOKEN_MAP`, `DEBUG_S_XFGHASH_TYPE`, `DEBUG_S_XFGHASH_VIRTUAL`, `DEBUG_S_IGNORE` bit, `DEBUG_S_FILECHKSMS` checksum kind `Sha1` |
| #32 | Semantic reading | 7 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Reference validation | 9 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, Type map stream version 1 (type server map), Type map stream version 2 (object map), `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Writing | 7 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Editing and index rebuilding | 7 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Independent fixture evidence | 22 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, Feature `PdbImplVC110`, Feature `PdbFeatureNoTypeMerge` (`NOTM`), `PdbDbiV110`, Header flag `fCTypes`, Section contributions `DBISCImpv` V40 (headerless, 20-byte records), Type server map, not empty, Slot 1: Exception (`IMAGE_FUNCTION_ENTRY`), Slot 3: OMAP to source, Slot 4: OMAP from source, Slot 6: Token to RID map, Slot 10: Original section headers, OMAP translation (both directions), Hash version 2 (`LHashPbCbV2`), `/LinkInfo` version 1, `/LinkInfo` version 2, `/UDTSRCLINEUNDONE`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |

## Reference reconciliation

Every imported reference document has a section on a reconciliation page. The table gives the result for each document.
"pdbrs" is `implemented`, `partial`, `not implemented` or `not applicable` (no format content). The counts are the listed conflicts and undocumented assumptions.
Totals: 96 documents; 78 implemented, 8 partial, 0 not implemented, 10 not applicable; 353 conflicts and 274 undocumented assumptions.

| Document | pdbrs | Conflicts | Undocumented assumptions |
| --- | --- | ---: | ---: |
| [`llvm/llvm/docs/PDB/index.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbindexmd) | implemented | 7 | 6 |
| [`llvm/llvm/docs/PDB/MsfFile.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmsffilemd) | implemented | 7 | 7 |
| [`llvm/llvm/docs/PDB/PdbStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbpdbstreammd) | implemented | 5 | 5 |
| [`llvm/llvm/docs/PDB/TpiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbtpistreammd) | implemented | 8 | 5 |
| [`llvm/llvm/docs/PDB/DbiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbdbistreammd) | implemented | 16 | 7 |
| [`llvm/llvm/docs/PDB/ModiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmodistreammd) | partial | 4 | 7 |
| [`llvm/llvm/docs/PDB/PublicStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbpublicstreammd) | implemented | 4 | 5 |
| [`llvm/llvm/docs/PDB/GlobalStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbglobalstreammd) | implemented | 3 | 6 |
| [`llvm/llvm/docs/PDB/HashTable.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbhashtablemd) | implemented | 3 | 5 |
| [`llvm/llvm/docs/PDB/CodeViewTypes.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbcodeviewtypesmd) | implemented | 5 | 9 |
| [`llvm/llvm/docs/PDB/CodeViewSymbols.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbcodeviewsymbolsmd) | implemented | 9 | 9 |
| [`llvm/llvm/LICENSE.TXT`](native-coverage/reconciliation-llvm.md#llvmllvmlicensetxt) | not applicable | 0 | 0 |
| [`microsoft-pdb-rs/docs/pdb/tpi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbtpi_streammd) | implemented | 11 | 10 |
| [`microsoft-pdb-rs/docs/pdb/relationships.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbrelationshipsmd) | partial | 4 | 4 |
| [`microsoft-pdb-rs/docs/pdb/pdbi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbpdbi_streammd) | implemented | 10 | 8 |
| [`microsoft-pdb-rs/docs/pdb/pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbpdbmd) | partial | 3 | 6 |
| [`microsoft-pdb-rs/docs/pdb/names_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbnames_streammd) | implemented | 9 | 3 |
| [`microsoft-pdb-rs/docs/pdb/msf.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfmd) | implemented | 9 | 7 |
| [`microsoft-pdb-rs/docs/pdb/msfz.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfzmd) | implemented | 11 | 4 |
| [`microsoft-pdb-rs/docs/pdb/mini_pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmini_pdbmd) | implemented | 5 | 4 |
| [`microsoft-pdb-rs/docs/pdb/module_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmodule_streammd) | partial | 3 | 10 |
| [`microsoft-pdb-rs/docs/pdb/ipi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbipi_streammd) | implemented | 3 | 4 |
| [`microsoft-pdb-rs/docs/pdb/hashing.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbhashingmd) | implemented | 6 | 1 |
| [`microsoft-pdb-rs/docs/pdb/globals.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbglobalsmd) | implemented | 12 | 6 |
| [`microsoft-pdb-rs/docs/pdb/dbi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_streammd) | implemented | 6 | 3 |
| [`microsoft-pdb-rs/docs/pdb/dbi_sources.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_sourcesmd) | implemented | 5 | 3 |
| [`microsoft-pdb-rs/docs/pdb/dbi_sections.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_sectionsmd) | implemented | 3 | 4 |
| [`microsoft-pdb-rs/docs/pdb/dbi_opt_debug.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_opt_debugmd) | implemented | 4 | 5 |
| [`microsoft-pdb-rs/docs/pdb/dbi_modules.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_modulesmd) | implemented | 5 | 4 |
| [`microsoft-pdb-rs/docs/pdb/dbi_fixups.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_fixupsmd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/terminology.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsterminologymd) | not applicable | 2 | 0 |
| [`microsoft-pdb-rs/docs/references.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsreferencesmd) | not applicable | 4 | 0 |
| [`microsoft-pdb-rs/docs/intro.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsintromd) | not applicable | 1 | 0 |
| [`microsoft-pdb-rs/docs/determinism.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsdeterminismmd) | partial | 3 | 4 |
| [`microsoft-pdb-rs/docs/index.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsindexmd) | not applicable | 0 | 0 |
| [`microsoft-pdb-rs/docs/data_types.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocsdata_typesmd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/LICENSE`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rslicense) | not applicable | 0 | 0 |
| [`microsoft-pdb/README.md`](native-coverage/reconciliation-pdb.md#microsoft-pdbreadmemd) | not applicable | 3 | 0 |
| [`microsoft-pdb/docs/ExternalResources.md`](native-coverage/reconciliation-pdb.md#microsoft-pdbdocsexternalresourcesmd) | not applicable | 1 | 0 |
| [`microsoft-pdb/LICENSE`](native-coverage/reconciliation-pdb.md#microsoft-pdblicense) | not applicable | 0 | 0 |
| [`SOURCES.json`](native-coverage/reconciliation-pdb.md#sourcesjson) | not applicable | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/primitive_types.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewprimitive_typesmd) | implemented | 8 | 3 |
| [`microsoft-pdb-rs/docs/codeview/number.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewnumbermd) | implemented | 4 | 4 |
| [`microsoft-pdb-rs/docs/codeview/line_data.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewline_datamd) | partial | 8 | 14 |
| [`microsoft-pdb-rs/docs/codeview/codeview.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewcodeviewmd) | partial | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/calling_convention.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewcalling_conventionmd) | implemented | 4 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/types.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypestypesmd) | implemented | 4 | 6 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_union.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_unionmd) | implemented | 0 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_vtshape.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_vtshapemd) | implemented | 3 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_skip.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_skipmd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_procedure.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_proceduremd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_precomp.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_precompmd) | implemented | 1 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_modifier.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_modifiermd) | implemented | 3 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_pointer.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_pointermd) | implemented | 5 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_mfunction.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_mfunctionmd) | implemented | 4 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_methodlist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_methodlistmd) | implemented | 3 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_fieldlist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_fieldlistmd) | implemented | 7 | 5 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_enum.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_enummd) | implemented | 1 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_endprecomp.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_endprecompmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_class.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_classmd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_bitfield.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_bitfieldmd) | implemented | 0 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_array.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_arraymd) | implemented | 2 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_arglist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_arglistmd) | implemented | 1 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/items.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemsitemsmd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_buildinfo.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_buildinfomd) | implemented | 4 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_func_id.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_func_idmd) | implemented | 5 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_mfunc_id.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_mfunc_idmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_string_id.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_string_idmd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_substr_list.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_substr_listmd) | implemented | 2 | 1 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_udt_src_line.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_udt_src_linemd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/items/lf_udt_mod_src_line.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewitemslf_udt_mod_src_linemd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/symbols.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolssymbolsmd) | implemented | 11 | 11 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_annotation.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_annotationmd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_armswitchtable.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_armswitchtablemd) | implemented | 3 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_block.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_blockmd) | implemented | 0 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_buildinfo.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_buildinfomd) | implemented | 0 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_callsiteinfo.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_callsiteinfomd) | implemented | 2 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_coffgroup.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_coffgroupmd) | implemented | 0 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_compile.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_compilemd) | implemented | 6 | 3 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_constant.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_constantmd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_data.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_datamd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_end.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_endmd) | implemented | 2 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_frameproc.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_frameprocmd) | implemented | 3 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_inlinesite.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_inlinesitemd) | implemented | 5 | 4 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_label.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_labelmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_local.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_localmd) | implemented | 10 | 6 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_namespace.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_namespacemd) | implemented | 0 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_objname.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_objnamemd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_procs.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_procsmd) | partial | 10 | 5 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_pub.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_pubmd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_refsyms.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_refsymsmd) | implemented | 3 | 3 |
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
5. FASTLINK buckets: Microsoft `GSI1::hashSz` truncates the bucket to 16 bits. `ms-pdb` 0.1.22 does not, and LLVM always uses 4096 buckets. pdbrs follows Microsoft, finds all 836 public records of `msvc-x64-fastlink` and rebuilds the index to the same bytes (#13, #30). See [`mini_pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmini_pdbmd).
6. Named-stream map: LLVM `PdbStream.md` and pdb-rs `pdbi_stream.md` omit the single `niMac` word after the map. An early pdbrs build read it as feature code 0 (#9).
7. TPI hash adjusters: LLVM `TpiStream.md` calls the keys hash values. Microsoft `tpi.cpp`, LLVM code and pdbrs read them as `/names` offsets (#3).
8. Section contributions: Microsoft `dbi.cpp` and pdbrs read a substream without the V60 or V2 version as headerless V40 records. LLVM rejects it, and no document describes it.
9. DBI version: pdb-rs `dbi_stream.md` calls V110 current. Microsoft `dbi.h` and every fixture use V70, the only version that pdbrs accepts.
10. Symbol padding: pdb-rs `symbols.md` and the `ms-codeview` builder use `f3 f2 f1`. LLD, MSVC and fxc write zero bytes (fxc writes 4 or more), and pdbrs accepts only zero bytes and keeps their count (#30). See [`symbols.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolssymbolsmd).
11. Kind values in pdb-rs documents: `S_GDATA32`/`S_LDATA32` given as `0x1007`/`0x1008` (the `_ST` values), `S_INLINESITE2` as `0x115c` (`S_POGODATA`), `LF_ARRAY` as `0x1502` (`LF_ENUMERATE`), `LF_FRIENDCLS` as `0x040b` (the 16-bit value).
12. Marker values that later sources reuse: `cvinfo.h` `S_RECTYPE_MAX` is `0x1166`, which LLVM uses for `S_FRAMEREG`; `LF_TYPE_LAST` `0x151e` is LLVM `LF_TAGGED_UNION`; `LF_ID_LAST` `0x1608` is `LF_CLASS2`.
13. Ages: pdb-rs `dbi_stream.md` tells readers to accept any age difference. `pdbstr -w` increments only the Info age (#27). pdbrs accepts an Info age larger than the DBI age and rejects a larger DBI age (#32).
14. `LF_HLSL` property bits: `cvinfo.h` says that bits 4 to 15 are zero. fxc writes other values, and LLVM, DIA and `pdbtool` accept them. pdbrs keeps the raw word (#29).
15. `/names` bucket counts: Microsoft `NMT` and LLVM grow the bucket array differently. `StringTableBuilder` follows Microsoft and gives the same bytes for the 23 MSVC tables (#32).

`T_NCVPTR` (`0xf0`), which #28 found rejected, decodes since #29.

### Undocumented assumptions

pdbrs uses these layouts and rules, and no imported document states them:

- `S_FRAMEREG` (`0x1166`): `u16` register, then 2 zero bytes. Confirmed from `msvc-arm64-debug` bytes (#13).
- `S_FASTLINK` (`0x1167`): `u32` value, `u16` flags, name. `REFMINIPDB` without the module field; flag bit 6 marks namespaces. Confirmed from `msvc-x64-fastlink` and its object files (#13) and the DIA SDK `REFMINIPDB2` (#30).
- The kinds above `0x1168` (`S_HOTPATCHFUNC`, the `_INDIR` and `_ENCTMP` variables, `S_GPROC32EX`, `S_STATICLOCAL`, `S_ASSOCIATION`, `S_HYBRIDRANGE`, `S_SOURCELINK`, the `_ON_ENTRY` def-ranges): layouts from the debug type records in the DIA SDK `diaguids.lib` (#30).
- `DEBUG_S_COFF_SYMBOL_RVA` (`0xfd`): an array of `u32` image RVAs, one for each COFF symbol record; `0` means no address and `0xffffffff` an absolute symbol (#13).
- `DEBUG_S_XFGHASH_TYPE` (`0xff`) and `DEBUG_S_XFGHASH_VIRTUAL` (`0x100`): 12-byte and 16-byte entries, from the MSVC 14.44 objects of `msvc-x64-xfg` (#31).
- `DEBUG_S_FRAMEDATA`: an optional `u32` relocation value before the records, because `cvdump` and LLVM differ (#31).
- `/TMCache` version `0x20200229` and its type map streams (versions 1 and 2): from `DBI1::DumpTMCache` of `mspdbcore.dll` 14.51 (#13).
- `/UDTSRCLINEUNDONE`: a list of `u32` type indices of UDTs (#9).
- The FASTLINK bucket rule `(hash % 0x3ffff) & 0xffff` (#13).
- MSVC writes 12 optional debug header slots; slot 11 has no name and is nil in every fixture.
- The trampoline join: thunk index `(thunk offset - thunk_table_offset) / thunk_size`, and the thunk map entry is the section start of the target plus the target offset (#13).
- Scope links are module stream offsets that include the 4-byte signature (#5).
- `link.exe` writes `LF_UDT_MOD_SRC_LINE` without padding, and the MSVC fixtures split field lists at `0xfb00` bytes (LLVM uses `0xff00`) (#29).
- The hash table growth rules of Microsoft `map.h` and LLVM `HashTable.h` (#32), and the hash stream forms of Microsoft and LLVM (`HashLayout`, #29).
- `link /PDBSTRIPPED` output: the rules of `strip_private` come from `msvc-x64-stripped/demo.stripped.pdb` (#34).
- The `S_OEM` attributes of the managed OEM ID `c6ea3fc9-59b3-49d6-bc25-0902bbabb460` (`MD2`, `asyncMethodInfo`) follow Roslyn and Mono.Cecil, not a PDB document (#14).

## External dependencies

pdbrs does not load external files. `Pdb::external_dependencies` lists the type servers with their modules, the precompiled-header groups and the FASTLINK objects (#32). It opens no file.

| Item | Target | State |
| --- | --- | --- |
| Non-empty DBI type server map, `LF_TYPESERVER`, `LF_TYPESERVER2` | A type server PDB | Decoded; `Pdb::external_dependencies` lists the type servers of the DBI map; not loaded |
| `LF_PRECOMP`, `LF_ENDPRECOMP` | Precompiled header types in an object file | Decoded; the precompiled-header groups are listed; the fixtures keep them in the objects only (#27) |
| `S_FASTLINK`, `S_REF_MINIPDB`, `S_PDBMAP`, `DEBUG_S_COFF_SYMBOL_RVA` | Object files and PDBs of a FASTLINK link | Decoded and listed; targets not resolved, so FASTLINK addresses stay unresolved (#33) |
| `/TMCache` checksums | Type sections of object files | Kept; cannot be checked |
| `S_MANSLOT`, `S_MANCONSTANT`, `S_TOKENREF`, `S_OEM` tokens, token maps | Metadata of the managed assembly | Kept |
| `sourcelink`, `srcsrv` | Source servers and URLs | Bytes kept and checked; nothing is fetched or run |

## Scope decisions

| Format | Decision |
| --- | --- |
| MSFZ containers | Required; separate codec (#20) |
| Small MSF and other legacy versions and kinds | Required, deferred (#16) |
| 65536-byte pages | Read; not written, because consumers reject them (#15) |
| Edits in place and incremental MSF commits | Not planned ([design](pdb.md#implemented-native-reading)); the editor writes a new file (#34) |
| OMAP translation | Implemented (#32); the real-producer fixture needs `external-input` |
| Compressed injected sources (run-length, Huffman, LZ) | Required (#32); no producer and no algorithm description (`external-input`) |
| Natvis XML, Source Link JSON and `srcsrv` text | Decided in #32: pdbrs keeps and checks the bytes and does not interpret them ([decision](https://github.com/napbat/pdbrs/issues/32#issuecomment-6412876796)) |
| `hlsl-dxc-debug` PDBs (declared page count too small) | Rejected, as `pdbtool` and DIA reject them (#27) |
| Producer extensions without a layout (LLVM-only type kinds, `S_ALTOBJNAME`, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `0xfe`, `S_OEM` of other OEM IDs) | Keep raw and write raw; decode when a source or fixture gives the layout (#29, #30, #31) |
