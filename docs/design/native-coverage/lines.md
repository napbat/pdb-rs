# Native coverage: C13 line data and module streams

This page is part of the [native coverage](../native-coverage.md) accounting. Read the [column and value definitions](../native-coverage.md#columns) first.
The kind list comes from `cvinfo.h` (`DEBUG_S_SUBSECTION_TYPE`) and LLVM `CodeView.h` (`DebugSubsectionKind`).
`SubsectionKind::from_raw` classifies every kind below. `LineSubsection::decode` decodes `0xf2`, `0xf4`, `0xf6` and `0xfd` and keeps every other kind raw.

## Module stream regions

`ModuleStream::parse` splits a module stream with the region sizes of its DBI module record. The regions must fill the stream exactly.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Symbol region, signature `CV_SIGNATURE_C13` | `4` | complete ([code][dmod], [check][t:dbi], [check][e15]) | complete ([code][dmod], [code][sscope], [check][e5], [check][e10]) | complete ([code][sscope], [check][t:symbol_scopes], [check][e5]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][e10], [check][e13v]) | 28 fixture PDBs, 1509 module streams |
| Symbol region, signatures `CV_SIGNATURE_C7` and `CV_SIGNATURE_C11` | `1`, `2` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `ModuleStream::parse` returns `UnsupportedVersion` |
| Symbol region absent (size 0) | — | complete ([code][dmod], [check][t:dbi], [check][e15]) | complete ([code][dmod], [check][t:dbi]) | not applicable (no references) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture has a module stream without symbols) | 97 module records of the fixtures have no module stream; that is a different case |
| C11 line region | — | complete ([code][dmod], [check][t:dbi], [check][e15]) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | Kept raw; empty in all 17 fixture PDBs |
| C13 line region | — | complete ([code][dmod], [check][t:lines], [check][e15]) | complete ([code][dmod], [code][lmod], [check][e6], [check][e10]) | complete ([code][lmod], [check][e6], [check][t:lines]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][e6], [check][e13v]) | Subsection framing; the tables below give each kind |
| Global references region | — | complete ([code][dmod], [check][t:dbi], [check][e15]) | missing (#31: kept raw, not a typed offset array) | missing (#31: the offsets into the symbol record stream are not checked) | missing (#31) | missing (#31, #34) | partial (#31: non-empty in 22 fixture PDBs; not decoded) | 914 non-empty regions |

## Subsection kinds

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DEBUG_S_SYMBOLS` | `0xf1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31: the symbol codec exists, but the C13 classifier does not route to it) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |
| `DEBUG_S_LINES` | `0xf2` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lblk], [check][e6], [check][t:lines]) | complete ([code][lmod], [code][exs], [check][e6], [check][e10]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][e6], [check][e13s]) | 27 fixture PDBs, 10684 subsections |
| `DEBUG_S_STRINGTABLE` | `0xf3` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |
| `DEBUG_S_FILECHKSMS` | `0xf4` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][e6], [check][t:lines]) | complete ([code][exs], [check][e10]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][e6], [check][e13s]) | 27 fixture PDBs, 1191 subsections |
| `DEBUG_S_FRAMEDATA` | `0xf5` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |
| `DEBUG_S_INLINEELINES` | `0xf6` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][linl], [check][e6], [check][t:lines]) | complete ([code][lmod], [code][exs], [check][e6], [check][e10]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][e6], [check][e13s]) | 11 fixture PDBs, 77 subsections |
| `DEBUG_S_CROSSSCOPEIMPORTS` | `0xf7` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |
| `DEBUG_S_CROSSSCOPEEXPORTS` | `0xf8` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |
| `DEBUG_S_IL_LINES` | `0xf9` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture; C++/CLI with MSVC 14.51 (`msvc-x64-clr`) writes none (#27) |
| `DEBUG_S_FUNC_MDTOKEN_MAP` | `0xfa` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture; C++/CLI with MSVC 14.51 (`msvc-x64-clr`) writes none (#27) |
| `DEBUG_S_TYPE_MDTOKEN_MAP` | `0xfb` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture; C++/CLI with MSVC 14.51 (`msvc-x64-clr`) writes none (#27) |
| `DEBUG_S_MERGED_ASSEMBLYINPUT` | `0xfc` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |
| `DEBUG_S_COFF_SYMBOL_RVA` | `0xfd` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lrva], [check][e13l], [check][t:lines]) | complete ([code][exs], [check][e13s], [check][e13a]) | missing (#31) | missing (#31, #34) | partial (#31: LLVM does not print the values; they match `llvm-pdbutil bytes --chunks` and the object files (#13)) | 1 fixture PDB, 114 subsections |

## Subsection kinds after `cvinfo.h`

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DEBUG_S_INLINEE_MDTOKEN_MAP` | `0xfe` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture; Only the pdb-rs `line_data.md` names it; no pinned source defines it |
| `DEBUG_S_XFGHASH_TYPE` | `0xff` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture; LLVM `DebugSubsectionKind::XfgHashType`; `link.exe` drops it from `msvc-x64-xfg`, only the objects keep it (#27) |
| `DEBUG_S_XFGHASH_VIRTUAL` | `0x100` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31) | missing (#31) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture; LLVM `DebugSubsectionKind::XfgHashVirtual`; `link.exe` drops it from `msvc-x64-xfg`, only the objects keep it (#27) |
| `DEBUG_S_IGNORE` bit | `0x80000000` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | partial (#31: classified as `SubsectionKind::Ignored`; the `native` example counts it as undecoded) | not applicable (a reader ignores the contents) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | `cvinfo.h`: a reader ignores the contents of a subsection with this bit |
| Unknown kind | any other value | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | not applicable (no layout; `SubsectionKind::Unknown` keeps the bytes) | not applicable (no layout) | missing (#31: raw copy) | missing (#31, #34) | not applicable (no producer known) | Distinct from a framing error |

## Subsection variants

These rows split the layout variants inside the decoded kinds.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DEBUG_S_LINES` without columns | flags `0` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lblk], [check][e6], [check][t:lines]) | complete ([code][lmod], [check][e6]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][dpf]) | 26 fixture PDBs, 10650 items |
| `DEBUG_S_LINES` with columns (`CV_LINES_HAVE_COLUMNS`) | flags `1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lblk], [check][t:lines]) | complete ([code][lmod], [check][t:lines]) | missing (#31) | missing (#31, #34) | partial (#31: only in `dotnet-csharp-full`; no recorded consumer comparison) | 1 fixture PDB, 34 items |
| `DEBUG_S_FILECHKSMS` checksum kind `None` | `0` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines]) | complete ([code][exs], [check][e10]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][dpf]) | 4 fixture PDBs, 47 items |
| `DEBUG_S_FILECHKSMS` checksum kind `Md5` | `1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines]) | complete ([code][exs], [check][e10]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][dpf]) | 5 fixture PDBs, 116 items |
| `DEBUG_S_FILECHKSMS` checksum kind `Sha1` | `2` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines]) | complete ([code][exs], [check][e10]) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |
| `DEBUG_S_FILECHKSMS` checksum kind `Sha256` | `3` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines]) | complete ([code][exs], [check][e10]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][dpf]) | 26 fixture PDBs, 76077 items |
| `DEBUG_S_INLINEELINES` signature `CV_INLINEE_SOURCE_LINE_SIGNATURE` | `0` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][linl], [check][e6], [check][t:lines]) | complete ([code][lmod], [check][e6]) | missing (#31) | missing (#31, #34) | complete ([code][exs], [check][dpf]) | 11 fixture PDBs, 77 items |
| `DEBUG_S_INLINEELINES` signature with extra files | `1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][linl], [check][t:lines]) | complete ([code][lmod], [check][t:lines]) | missing (#31) | missing (#31, #34) | missing (#27, #31: no fixture) | No fixture |

[dmod]: ../../../crates/pdbrs/src/native/dbi/module.rs
[dpf]: ../pdb.md#producer-fixtures
[e10]: https://github.com/napbat/pdbrs/issues/10#issuecomment-6407894516
[e13a]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409665284
[e13l]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409581275
[e13s]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409780019
[e13v]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409862909
[e15]: https://github.com/napbat/pdbrs/issues/15#issuecomment-6410580545
[e5]: https://github.com/napbat/pdbrs/issues/5#issuecomment-6407599155
[e6]: https://github.com/napbat/pdbrs/issues/6#issuecomment-6407539677
[exs]: ../../../crates/pdbrs/examples/native/symbols.rs
[lblk]: ../../../crates/pdbrs/src/native/lines/blocks.rs
[lchk]: ../../../crates/pdbrs/src/native/lines/checksums.rs
[lines]: ../../../crates/pdbrs/src/native/lines.rs
[linl]: ../../../crates/pdbrs/src/native/lines/inlinee.rs
[lmod]: ../../../crates/pdbrs/src/native/lines/module.rs
[lrva]: ../../../crates/pdbrs/src/native/lines/coff_symbol_rva.rs
[sscope]: ../../../crates/pdbrs/src/native/codeview/symbol/scope.rs
[t:dbi]: ../../../crates/pdbrs/tests/dbi.rs
[t:lines]: ../../../crates/pdbrs/tests/lines.rs
[t:symbol_scopes]: ../../../crates/pdbrs/tests/symbol_scopes.rs
