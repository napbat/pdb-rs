# Native PDB coverage

This document gives the coverage of modern native PDBs (MSF 7.0 and MSFZ containers) for each record kind, stream version and container feature.
It is product documentation: it tells what the library reads, checks, writes and edits today. GitHub issues hold the scope and the evidence.
The epics [#1](https://github.com/napbat/pdbrs/issues/1) (MSF 7.0) and [#20](https://github.com/napbat/pdbrs/issues/20) (MSFZ) own the native scope, and [#28](https://github.com/napbat/pdbrs/issues/28) owns this accounting.

Legacy native PDBs ([#16](https://github.com/napbat/pdbrs/issues/16)) and Portable PDBs ([#17](https://github.com/napbat/pdbrs/issues/17)) have their own epics.
Legacy kinds and versions have rows here, marked `not applicable (legacy, #16)`, so that the inventory is complete. Portable PDB has no rows.

## Pages

| Page | Contents |
| --- | --- |
| [Containers and PDB streams](native-coverage/streams.md) | MSF 7.0: page sizes, free page maps, stream directory, stream 0. MSFZ: header, stream directory, Chunk Table, chunk codecs, fragments, options and conversions. PDB Info versions, identity and features; TPI and IPI versions and hash streams; module type streams of `NOTM` PDBs; DBI versions, flags, substreams and section contribution versions; optional debug streams; address translation and OMAP; `/names` versions; GSI and PSI layouts; named and auxiliary streams |
| [Types, IDs and numeric leaves](native-coverage/types.md) | Type leaves, extensions after `cvinfo.h`, producer forms, field-list members, ID leaves, numeric leaves, primitive kinds and pointer modes, padding, markers, aliases, legacy kinds |
| [Symbol records](native-coverage/symbols.md) | Symbol kinds, extensions after `cvinfo.h`, `S_OEM` payload forms, markers, aliases, legacy kinds |
| [C13 line data and module streams](native-coverage/lines.md) | Module stream regions, C13 subsection kinds, the ignore bit, subsection variants |
| [Reconciliation: LLVM](native-coverage/reconciliation-llvm.md) | The 12 files of `docs/reference/llvm/` |
| [Reconciliation: pdb-rs PDB documents](native-coverage/reconciliation-pdb.md) | `microsoft-pdb-rs/docs/pdb/` (with `msfz.md`), the top-level pdb-rs files, `microsoft-pdb/` and `SOURCES.json` |
| [Reconciliation: pdb-rs CodeView types](native-coverage/reconciliation-codeview-types.md) | `microsoft-pdb-rs/docs/codeview/` top level, `types/` and `items/` |
| [Reconciliation: pdb-rs CodeView symbols](native-coverage/reconciliation-codeview-symbols.md) | `microsoft-pdb-rs/docs/codeview/symbols/` |

## Inventory sources

The rows come from the pinned implementation definitions, not from the prose documents alone:

- `cvinfo.h` at `microsoft/microsoft-pdb@805655a2`: `LEAF_ENUM_e`, `TYPE_ENUM_e`, `SYM_ENUM_e` and `DEBUG_S_SUBSECTION_TYPE`.
- LLVM at `b9f2235f`: `CodeViewTypes.def`, `CodeViewSymbols.def`, `CodeView.h` (`DebugSubsectionKind`), `TypeIndex.h` and `RawConstants.h`.
- `ms-codeview` 0.1.7: `types/kind.rs` and `syms/kind.rs`. `ms-pdb-msfz` 0.1.10 and the MSFZ specification `msfz.md` for the MSFZ rows.
- The debug type records of `cvinfo.h` in the DIA SDK `diaguids.lib` (Visual Studio 18) for the symbol kinds above `0x1168`, for example `S_HYBRIDRANGE` (#30).
- The prose documents in [`docs/reference/`](../reference/README.md). The reconciliation pages compare them with the implementation.

A kind that only a later source defines (for example `S_FRAMEREG`, `LF_CLASS2` or `DEBUG_S_XFGHASH_TYPE`) is in a separate table of extensions after `cvinfo.h`.
A second name for the same value (for example `S_SLOT`) is in a separate alias row.

## Columns

Each row has six status columns:

| Column | Meaning |
| --- | --- |
| Raw preservation | The reader gives the exact bytes of the item, and exact copies, repacks and container conversions keep them. Raw framing alone does not make semantic reading complete |
| Semantic reading | A typed decoder reads every field of the item |
| Reference validation | The library or the `native` example checks every reference that the item holds: type and ID indices, stream IDs, `/names` offsets, scope links, module indices, addresses and container ranges |
| Writing | An encoder writes the item from its decoded form, to the same bytes for a decoded item |
| Editing and index rebuilding | `PdbBuilder` (#33), `PdbEditor` (#34) or a container conversion (#38) writes or changes the item and rebuilds the streams and indexes that depend on it (hashes, offsets, address maps) |
| Independent fixture evidence | A PDB from an independent producer holds the item, and an independent consumer (`llvm-pdbutil` 22.1.6, DIA2Dump, `pdbtool` or `llvm-symbolizer`, #21) agrees with pdbrs |

## Values

| Value | Meaning |
| --- | --- |
| `complete` | The cell links the implementation (`code`) and a consumer-visible check (`check`): an issue evidence comment, a test file or a design evidence section |
| `partial` | Some of the work exists. The cell names the open issue and the reason |
| `missing` | The work does not exist. The cell names the open issue and the reason |
| `not applicable` | The column does not apply. The cell gives the reason, for example `legacy, #16`, `no references` or `marker, not a record` |

`external-input` in a cell means that the work needs input that is not available here: a producer that writes the item, or a definition of its layout. The issue carries the `external-input` label.

A recognized version that pdbrs does not decode gives `UnsupportedVersion`. Corrupt data gives `InvalidFormat`, `InvalidReference` or `UnexpectedEof`. The two classes stay distinct.
A top-level record of a kind without a decoder keeps its kind and payload (`TypeLeaf::Unsupported`, `SymbolData::Unsupported`, `LineSubsection::Unsupported`), and its encoder writes the bytes again.

## Fixture evidence

The fixture counts in the notes come from the native PDBs of the fixture manifest (`cargo run -p pdbrs-fixtures`, #12, #27 and #1): 48 PDBs of 43 fixtures, and 63 PDZ files that `pdbtool pdz-encode` wrote from them (#37).
A read of every record with the pdbrs reader at `b4d4cdc` gave the counts. The `native` example does not print counts for each kind.
The 3 PDBs of `hlsl-dxc-debug` are not in the counts: their MSF header declares fewer pages than the streams use, and `pdbtool`, DIA and pdbrs reject them (#27).

The fixtures are: `clang-link-x64-debug`, `clang-lld-arm64-debug`, `clang-lld-x64-debug`, `clang-lld-x64-page16384`, `clang-lld-x64-page32768`, `clang-lld-x64-page8192`, `clang-lld-x64-release`, `clang-lld-x64-static`, `clang-lld-x86-debug`, `dotnet-csharp-full`, `hlsl-dxc-debug`, `hlsl-fxc-sm50-debug`, `hlsl-fxc-sm51-debug`, `msvc-arm64-debug`, `msvc-arm64-debugtype`, `msvc-x64-annotation`, `msvc-x64-clr`, `msvc-x64-debug`, `msvc-x64-debugtype`, `msvc-x64-dll`, `msvc-x64-fastlink`, `msvc-x64-incremental`, `msvc-x64-large`, `msvc-x64-lazy`, `msvc-x64-natvis`, `msvc-x64-page16384`, `msvc-x64-page32768`, `msvc-x64-page8192`, `msvc-x64-pch-z7`, `msvc-x64-pch-zi`, `msvc-x64-pgo`, `msvc-x64-release`, `msvc-x64-sha1`, `msvc-x64-srcsrv`, `msvc-x64-stripped`, `msvc-x64-vc120`, `msvc-x64-xfg`, `msvc-x64-z7`, `msvc-x86-debug`, `rust-link-x64-debug`, `rust-link-x64-release`, `rust-link-x86-debug`, `rust-lld-x64-debug`.

A fixture cell is `complete` if a fixture PDB holds the item and an evidence comment records an independent consumer check of it:

- a comparison of decoded values with `llvm-pdbutil` or DIA2Dump (#2 to #14, #1 for the `types` example), or
- the round trip of #29 to #32: pdbrs decodes and encodes every record and stream of every fixture to the same bytes, and copies with the rebuilt streams give the same `llvm-pdbutil` and DIA2Dump output and pass `pdbtool check` and `pdbtool compare`, or
- the semantic rebuild of #33 (`cargo run -p pdbrs-fixtures --release -- verify-native-build`: 48 of 48 PDBs) and the edits of #34 (`verify-native-edit`: 288 edits, 0 problems without an explanation), or
- for MSFZ, the stream comparison of each PDZ fixture with its MSF 7.0 source (#37) and `verify-msfz` (#38).

A kind that only object files hold (for example `LF_PRECOMP` or `DEBUG_S_FRAMEDATA`) is `partial`: pdbrs decodes and encodes the object file data, but no PDB fixture holds it.
A kind in no fixture is `missing` with `external-input` and the producer that can make it, where one is known.

## Summary

Each cell gives `complete / partial / missing / not applicable`.

| Page | Rows | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| [Containers and PDB streams](native-coverage/streams.md) | 141 | 119 / 0 / 0 / 22 | 109 / 0 / 9 / 23 | 76 / 1 / 8 / 56 | 103 / 4 / 5 / 29 | 106 / 4 / 5 / 26 | 84 / 9 / 21 / 27 |
| [Types, IDs and numeric leaves](native-coverage/types.md) | 257 | 180 / 0 / 0 / 77 | 175 / 0 / 0 / 82 | 63 / 0 / 0 / 194 | 167 / 0 / 0 / 90 | 97 / 0 / 0 / 160 | 81 / 3 / 78 / 95 |
| [Symbol records](native-coverage/symbols.md) | 224 | 137 / 0 / 0 / 87 | 128 / 0 / 4 / 92 | 108 / 0 / 4 / 112 | 129 / 4 / 0 / 91 | 129 / 4 / 0 / 91 | 74 / 0 / 58 / 92 |
| [C13 line data and module streams](native-coverage/lines.md) | 32 | 31 / 0 / 0 / 1 | 28 / 0 / 1 / 3 | 18 / 0 / 1 / 13 | 30 / 1 / 0 / 1 | 29 / 1 / 0 / 2 | 15 / 5 / 9 / 3 |
| Total | 654 | 467 / 0 / 0 / 187 | 440 / 0 / 14 / 200 | 265 / 1 / 13 / 375 | 429 / 9 / 5 / 211 | 361 / 9 / 5 / 279 | 254 / 17 / 166 / 217 |

## Remaining open cells

Each `missing` or `partial` cell of a modern or extension row names its open issue and the reason. The table groups them by issue and column.
All of them need `external-input` (a producer or a layout definition that is not available here), except the MSFZ stream larger than 4 GiB, which needs a very large input.

| Issue | Column | Cells | Rows |
| --- | --- | ---: | --- |
| #20 | Independent fixture evidence | 5 | Stream directory, raw DEFLATE, Chunk, stored, Chunk, raw DEFLATE, Compressed fragment across chunk boundaries, Stream larger than 4 GiB |
| #29 | Semantic reading | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Reference validation | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Writing | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Editing and index rebuilding | 1 | Auxiliary hash stream (`hash_aux_stream`) |
| #29 | Independent fixture evidence | 83 | Hash stream: hash adjusters, Auxiliary hash stream (`hash_aux_stream`), `LF_ENDPRECOMP`, `LF_COBOL0`, `LF_BARRAY`, `LF_VFTPATH`, `LF_OEM`, `LF_OEM2`, `LF_SKIP`, `LF_DERIVED`, `LF_DIMCONU`, `LF_DIMCONLU`, `LF_DIMVARU`, `LF_DIMVARLU`, `LF_TYPESERVER`, `LF_DIMARRAY`, `LF_PRECOMP`, `LF_ALIAS`, `LF_DEFARG`, `LF_MANAGED`, `LF_TYPESERVER2`, `LF_INTERFACE`, `LF_CLASS2`, `LF_STRUCTURE2`, `LF_INTERFACE2`, `LF_UNION2`, `LF_FRIENDCLS`, `LF_VFUNCOFF`, `LF_FRIENDFCN`, `LF_NESTTYPEEX`, `LF_MEMBERMODIFY`, `LF_BINTERFACE`, `LF_REAL32`, `LF_REAL80`, `LF_REAL128`, `LF_REAL48`, `LF_COMPLEX32`, `LF_COMPLEX64`, `LF_COMPLEX80`, `LF_COMPLEX128`, `LF_OCTWORD`, `LF_UOCTWORD`, `LF_DATE`, `LF_UTF8STRING`, `LF_REAL16`, `T_ABS`, `T_SEGMENT`, `T_CURRENCY`, `T_NBASICSTR`, `T_FBASICSTR`, `T_NOTTRANS`, `T_OCT`, `T_UOCT`, `T_BOOL16`, `T_BOOL32`, `T_BOOL64`, `Boolean128` (LLVM), `T_REAL80`, `T_REAL128`, `T_REAL48`, `T_REAL16`, `T_CPLX32`, `T_CPLX64`, `T_CPLX80`, `T_CPLX128`, `Complex48` (LLVM), `Complex32PartialPrecision` (LLVM), `Complex16` (LLVM), `T_BIT`, `T_PASCHAR`, `T_BOOL32FF`, `T_INT1`, `T_UINT1`, `T_INT2`, `T_UINT2`, `T_INT8`, `T_INT16`, `T_UINT16`, `T_NCVPTR`, `CV_TM_FPTR`, `CV_TM_HPTR`, `CV_TM_FPTR32`, `CV_TM_NPTR128` |
| #30 | Semantic reading | 5 | `S_MOD_TYPEREF` `fRefTMPCT`, `fOwnTMPCT` and other flags, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_ALTOBJNAME` |
| #30 | Reference validation | 5 | `S_MOD_TYPEREF` `fRefTMPCT`, `fOwnTMPCT` and other flags, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_ALTOBJNAME` |
| #30 | Writing | 5 | `S_MOD_TYPEREF` `fRefTMPCT`, `fOwnTMPCT` and other flags, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_ALTOBJNAME` |
| #30 | Editing and index rebuilding | 5 | `S_MOD_TYPEREF` `fRefTMPCT`, `fOwnTMPCT` and other flags, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_ALTOBJNAME` |
| #30 | Independent fixture evidence | 61 | Feature `PdbFeatureMinimalDebugInfo` (`MINI`), `S_MOD_TYPEREF` `fRefTMPCT`, `fOwnTMPCT` and other flags, Minimal bucket count (`MINI` feature), `S_VFTABLE32`, `S_MANTYPREF`, `S_WITH32`, `S_COBOLUDT`, `S_MANYREG`, `S_LPROCMIPS`, `S_GPROCMIPS`, `S_MANYREG2`, `S_LPROCIA64`, `S_GPROCIA64`, `S_LOCALSLOT`, `S_PARAMSLOT`, `S_LMANDATA`, `S_MANFRAMEREL`, `S_MANREGISTER`, `S_MANMANYREG`, `S_MANREGREL`, `S_MANMANYREG2`, `S_DATAREF`, `S_LMANPROC`, `S_ATTR_FRAMEREL`, `S_ATTR_REGISTER`, `S_ATTR_REGREL`, `S_ATTR_MANYREG`, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `S_DISCARDED`, `S_DEFRANGE`, `S_DEFRANGE_SUBFIELD`, `S_LPROCMIPS_ID`, `S_GPROCMIPS_ID`, `S_LPROCIA64_ID`, `S_GPROCIA64_ID`, `S_LOCAL_DPC_GROUPSHARED`, `S_LPROC32_DPC`, `S_LPROC32_DPC_ID`, `S_DEFRANGE_DPC_PTR_TAG`, `S_DPC_SYM_TAG_MAP`, `S_REF_MINIPDB`, `S_PDBMAP`, `S_GDATA_HLSL32`, `S_LDATA_HLSL32`, `S_HOTPATCHFUNC`, `S_BPREL32_INDIR`, `S_REGREL32_INDIR`, `S_GPROC32EX`, `S_LPROC32EX`, `S_GPROC32EX_ID`, `S_LPROC32EX_ID`, `S_STATICLOCAL`, `S_BPREL32_ENCTMP`, `S_BPREL32_INDIR_ENCTMP`, `S_REGREL32_INDIR_ENCTMP`, `S_ASSOCIATION`, `S_HYBRIDRANGE`, `S_SOURCELINK`, `S_ALTOBJNAME` |
| #31 | Semantic reading | 1 | `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Reference validation | 1 | `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Writing | 1 | `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Editing and index rebuilding | 1 | `DEBUG_S_INLINEE_MDTOKEN_MAP` |
| #31 | Independent fixture evidence | 14 | Symbol region absent (size 0), `DEBUG_S_SYMBOLS`, `DEBUG_S_STRINGTABLE`, `DEBUG_S_FRAMEDATA`, `DEBUG_S_CROSSSCOPEIMPORTS`, `DEBUG_S_CROSSSCOPEEXPORTS`, `DEBUG_S_IL_LINES`, `DEBUG_S_FUNC_MDTOKEN_MAP`, `DEBUG_S_TYPE_MDTOKEN_MAP`, `DEBUG_S_MERGED_ASSEMBLYINPUT`, `DEBUG_S_INLINEE_MDTOKEN_MAP`, `DEBUG_S_XFGHASH_TYPE`, `DEBUG_S_XFGHASH_VIRTUAL`, `DEBUG_S_IGNORE` bit |
| #32 | Semantic reading | 7 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Reference validation | 7 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Writing | 7 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Editing and index rebuilding | 7 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |
| #32 | Independent fixture evidence | 20 | `PdbImpvVC80`, `PdbImpvVC110`, `PdbImpvVC140`, `PdbDbiV110`, Header flag `fCTypes`, Section contributions `DBISCImpv` V40 (headerless, 20-byte records), Type server map, not empty, Slot 1: Exception (`IMAGE_FUNCTION_ENTRY`), Slot 3: OMAP to source, Slot 4: OMAP from source, Slot 6: Token to RID map, Slot 10: Original section headers, OMAP translation (both directions), Hash version 2 (`LHashPbCbV2`), `/LinkInfo` version 1, `/LinkInfo` version 2, `/UDTSRCLINEUNDONE`, `/src/files/*`, compression run-length encoding, `/src/files/*`, compression Huffman, `/src/files/*`, compression LZ |

## Reference reconciliation

Every imported reference document has a section on a reconciliation page. The table gives the result for each document.
"pdbrs" is `implemented`, `partial`, `not implemented` or `not applicable` (no format content). The counts are the listed conflicts and undocumented assumptions.
Totals: 96 documents; 78 implemented, 8 partial, 0 not implemented, 10 not applicable; 357 conflicts and 306 undocumented assumptions.

| Document | pdbrs | Conflicts | Undocumented assumptions |
| --- | --- | ---: | ---: |
| [`llvm/llvm/docs/PDB/index.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbindexmd) | implemented | 7 | 6 |
| [`llvm/llvm/docs/PDB/MsfFile.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmsffilemd) | implemented | 7 | 7 |
| [`llvm/llvm/docs/PDB/PdbStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbpdbstreammd) | implemented | 5 | 7 |
| [`llvm/llvm/docs/PDB/TpiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbtpistreammd) | implemented | 8 | 6 |
| [`llvm/llvm/docs/PDB/DbiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbdbistreammd) | implemented | 16 | 7 |
| [`llvm/llvm/docs/PDB/ModiStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmodistreammd) | partial | 4 | 9 |
| [`llvm/llvm/docs/PDB/PublicStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbpublicstreammd) | implemented | 4 | 5 |
| [`llvm/llvm/docs/PDB/GlobalStream.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbglobalstreammd) | implemented | 3 | 6 |
| [`llvm/llvm/docs/PDB/HashTable.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbhashtablemd) | implemented | 3 | 5 |
| [`llvm/llvm/docs/PDB/CodeViewTypes.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbcodeviewtypesmd) | implemented | 6 | 9 |
| [`llvm/llvm/docs/PDB/CodeViewSymbols.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbcodeviewsymbolsmd) | implemented | 9 | 13 |
| [`llvm/llvm/LICENSE.TXT`](native-coverage/reconciliation-llvm.md#llvmllvmlicensetxt) | not applicable | 0 | 0 |
| [`microsoft-pdb-rs/docs/pdb/tpi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbtpi_streammd) | implemented | 12 | 11 |
| [`microsoft-pdb-rs/docs/pdb/relationships.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbrelationshipsmd) | partial | 4 | 5 |
| [`microsoft-pdb-rs/docs/pdb/pdbi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbpdbi_streammd) | implemented | 10 | 9 |
| [`microsoft-pdb-rs/docs/pdb/pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbpdbmd) | partial | 3 | 7 |
| [`microsoft-pdb-rs/docs/pdb/names_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbnames_streammd) | implemented | 9 | 3 |
| [`microsoft-pdb-rs/docs/pdb/msf.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfmd) | implemented | 9 | 7 |
| [`microsoft-pdb-rs/docs/pdb/msfz.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfzmd) | implemented | 13 | 5 |
| [`microsoft-pdb-rs/docs/pdb/mini_pdb.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmini_pdbmd) | implemented | 5 | 5 |
| [`microsoft-pdb-rs/docs/pdb/module_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmodule_streammd) | partial | 3 | 14 |
| [`microsoft-pdb-rs/docs/pdb/ipi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbipi_streammd) | implemented | 3 | 6 |
| [`microsoft-pdb-rs/docs/pdb/hashing.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbhashingmd) | implemented | 6 | 1 |
| [`microsoft-pdb-rs/docs/pdb/globals.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbglobalsmd) | implemented | 12 | 6 |
| [`microsoft-pdb-rs/docs/pdb/dbi_stream.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_streammd) | implemented | 6 | 3 |
| [`microsoft-pdb-rs/docs/pdb/dbi_sources.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_sourcesmd) | implemented | 5 | 3 |
| [`microsoft-pdb-rs/docs/pdb/dbi_sections.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_sectionsmd) | implemented | 3 | 4 |
| [`microsoft-pdb-rs/docs/pdb/dbi_opt_debug.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbdbi_opt_debugmd) | implemented | 4 | 6 |
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
| [`microsoft-pdb-rs/docs/codeview/types/types.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypestypesmd) | implemented | 4 | 8 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_union.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_unionmd) | implemented | 0 | 3 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_vtshape.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_vtshapemd) | implemented | 3 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_skip.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_skipmd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_procedure.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_proceduremd) | implemented | 2 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_precomp.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_precompmd) | implemented | 1 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_modifier.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_modifiermd) | implemented | 3 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_pointer.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_pointermd) | implemented | 5 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_mfunction.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_mfunctionmd) | implemented | 4 | 1 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_methodlist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_methodlistmd) | implemented | 3 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_fieldlist.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_fieldlistmd) | implemented | 7 | 5 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_enum.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_enummd) | implemented | 1 | 2 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_endprecomp.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_endprecompmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/types/lf_class.md`](native-coverage/reconciliation-codeview-types.md#microsoft-pdb-rsdocscodeviewtypeslf_classmd) | implemented | 2 | 3 |
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
| [`microsoft-pdb-rs/docs/codeview/symbols/symbols.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolssymbolsmd) | implemented | 11 | 12 |
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
| [`microsoft-pdb-rs/docs/codeview/symbols/s_inlinesite.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_inlinesitemd) | implemented | 5 | 6 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_label.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_labelmd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_local.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_localmd) | implemented | 10 | 7 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_namespace.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_namespacemd) | implemented | 0 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_objname.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_objnamemd) | implemented | 1 | 0 |
| [`microsoft-pdb-rs/docs/codeview/symbols/s_procs.md`](native-coverage/reconciliation-codeview-symbols.md#microsoft-pdb-rsdocscodeviewsymbolss_procsmd) | partial | 10 | 6 |
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
2. Page sizes: LLVM `MsfFile.md` allows 512 to 4096. LLVM code accepts 512 to 32768, and pdb-rs `msf.md` and `ms-pdb-msf` accept 512 to 65536. `link.exe` and `lld-link` write 4096 to 32768 (#1). The pdbrs reader accepts 512 to 65536. The writer rejects 65536 because DIA, LLVM and `link.exe` reject it (#15). See [`MsfFile.md`](native-coverage/reconciliation-llvm.md#llvmllvmdocspdbmsffilemd) and [`msf.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfmd).
3. Page-map pages: pdbrs reads and writes more than one page-map page. LLVM rejects such a file ("Too many directory blocks").
4. MSFZ: `msfz.md` and `ms-pdb-msfz` 0.1.10 differ. pdbrs follows the reference for the directory layout and the compression codes (code 2 is raw DEFLATE). It follows the specification for reserved location bits (an error), chunk sizes, overlaps and fragments across chunks (read, as the specification allows; never written, because `pdbtool` rejects them) (#37, #38). See [`msfz.md`](native-coverage/reconciliation-pdb.md#microsoft-pdb-rsdocspdbmsfzmd).
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
15. `/names` bucket counts: Microsoft `NMT` and LLVM grow the bucket array differently. `StringTableBuilder` follows Microsoft and gives the same bytes for the MSVC tables (#32).
16. `BA_OP_ChangeCodeOffsetBase`: `cvinfo.h` says that the value selects the n-th separated code chunk. pdbrs uses this rule (the n-th `S_SEPCODE` of the procedure) in the validator and in `SymbolLocator`; DIA agrees at all 720 separated-code addresses of `msvc-x64-pgo`, and `llvm-symbolizer` gives no function there (#30).

### Undocumented assumptions

pdbrs uses these layouts and rules, and no imported document states them:

- `S_FRAMEREG` (`0x1166`): `u16` register, then 2 zero bytes. Confirmed from `msvc-arm64-debug` bytes (#13).
- `S_FASTLINK` (`0x1167`): `u32` value, `u16` flags, name. `REFMINIPDB` without the module field; flag bit 6 marks namespaces. Confirmed from `msvc-x64-fastlink` and its object files (#13) and the DIA SDK `REFMINIPDB2` (#30).
- The kinds above `0x1168` (`S_HOTPATCHFUNC`, the `_INDIR` and `_ENCTMP` variables, `S_GPROC32EX`, `S_STATICLOCAL`, `S_ASSOCIATION`, `S_HYBRIDRANGE`, `S_SOURCELINK`, the `_ON_ENTRY` def-ranges): layouts from the debug type records in the DIA SDK `diaguids.lib` (#30).
- `S_MOD_TYPEREF` in `NOTM` PDBs (`link /DEBUG:LAZY`): `fOwnTMR` names one stream with the `.debug$T` records of a `/Z7` object, `fOwnTM` names a type stream and an ID stream, `fRefTM` names the owner module; no directory entry or name gives these streams (#30, #32).
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
- MASM (`ml64` 14.50) ends some `LF_STRUCTURE` records with `f1` and two stale bytes instead of `f3 f2 f1`; the static runtime libraries contain them (`clang-lld-x64-static`, #29).
- The hash table growth rules of Microsoft `map.h` and LLVM `HashTable.h` (#32), and the hash stream forms of Microsoft and LLVM (`HashLayout`, #29).
- `link /PDBSTRIPPED` output: the rules of `strip_private` come from `msvc-x64-stripped/demo.stripped.pdb`, and for `NOTM` PDBs from a `link /DEBUG:LAZY /PDBSTRIPPED` scratch link (#34, #32).
- `pdbtool pdz-encode` writes stream 0 as an empty stream and the Info stream and the DBI header as uncompressed fragments (#37, #38).
- The `S_OEM` attributes of the managed OEM ID `c6ea3fc9-59b3-49d6-bc25-0902bbabb460` (`MD2`, `asyncMethodInfo`) follow Roslyn and Mono.Cecil, not a PDB document (#14).

## External dependencies

pdbrs does not load external files. `Pdb::external_dependencies` lists the type servers with their modules, the precompiled-header groups, the FASTLINK objects, the `NOTM` feature and the module type sources (#32, #30). It opens no file.

| Item | Target | State |
| --- | --- | --- |
| Non-empty DBI type server map, `LF_TYPESERVER`, `LF_TYPESERVER2` | A type server PDB | Decoded; `Pdb::external_dependencies` lists the type servers of the DBI map; not loaded |
| `LF_PRECOMP`, `LF_ENDPRECOMP` | Precompiled header types in an object file | Decoded; the precompiled-header groups are listed; the fixtures keep them in the objects only (#27) |
| `S_FASTLINK`, `S_REF_MINIPDB`, `S_PDBMAP`, `DEBUG_S_COFF_SYMBOL_RVA` | Object files and PDBs of a FASTLINK link | Decoded and listed; targets not resolved, so FASTLINK addresses stay unresolved (#33) |
| `S_MOD_TYPEREF` (`NOTM`) | Module type streams inside the PDB | Read, validated, rebuilt and edited; listed as module type sources (#30, #32) |
| `/TMCache` checksums | Type sections of object files | Kept; cannot be checked |
| `S_MANSLOT`, `S_MANCONSTANT`, `S_TOKENREF`, `S_OEM` tokens, token maps | Metadata of the managed assembly | Kept |
| `sourcelink`, `srcsrv` | Source servers and URLs | Bytes kept and checked; nothing is fetched or run |

## Scope decisions

| Format | Decision |
| --- | --- |
| MSFZ containers | Implemented: reading (#37), writing and conversion in both directions (#38); stored chunks, DEFLATE directories and fragments across chunks have no producer |
| Small MSF and other legacy versions and kinds | Required, deferred (#16) |
| 65536-byte pages | Read; not written, because consumers reject them (#15) |
| Edits in place and incremental MSF commits | Not planned ([design](pdb.md#implemented-native-reading)); the editor writes a new file (#34) |
| OMAP translation | Implemented (#32); the real-producer fixture needs `external-input` |
| Compressed injected sources (run-length, Huffman, LZ) | Required (#32); no producer and no algorithm description (`external-input`) |
| Natvis XML, Source Link JSON and `srcsrv` text | Decided in #32: pdbrs keeps and checks the bytes and does not interpret them ([decision](https://github.com/napbat/pdbrs/issues/32#issuecomment-6412876796)) |
| `hlsl-dxc-debug` PDBs (declared page count too small) | Rejected, as `pdbtool` and DIA reject them (#27) |
| Producer extensions without a layout (LLVM-only type kinds, `S_ALTOBJNAME`, `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005`, `0xfe`, `S_OEM` of other OEM IDs, precompiled-header `S_MOD_TYPEREF` flags) | Keep raw and write raw; decode when a source or fixture gives the layout (#29, #30, #31) |
