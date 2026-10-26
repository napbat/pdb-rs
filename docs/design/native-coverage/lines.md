# Native coverage: C13 line data and module streams

This page is part of the [native coverage](../native-coverage.md) accounting. Read the [column and value definitions](../native-coverage.md#columns) first.
The kind list comes from `cvinfo.h` (`DEBUG_S_SUBSECTION_TYPE`) and LLVM `CodeView.h` (`DebugSubsectionKind`).
`LineSubsection::decode` decodes every kind below except `0xfe`, and each value has an encoder. `ModuleStreamParts::encode` writes a module stream and gives the region sizes for the DBI module record.
Kinds `0xf1`, `0xf3`, `0xf5`, `0xff` and `0x100` occur in object files; `link.exe` does not copy them to the PDB (#31).

## Module stream regions

`ModuleStream::parse` splits a module stream with the region sizes of its DBI module record. The regions must fill the stream exactly.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Symbol region, signature `CV_SIGNATURE_C13` | `4` | complete ([code][dmod], [check][t:dbi], [check][e15]) | complete ([code][dmod], [code][sscope], [check][e5], [check][e30]) | complete ([code][symv], [check][t:symbol_validation], [check][e30]) | complete ([code][dmodw], [code][sscw], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][edit], [check][e31], [check][e34]) | complete ([code][exl], [check][e30], [check][e31]) | 36 fixture PDBs, 1630 module streams |
| Symbol region, signatures `CV_SIGNATURE_C7` and `CV_SIGNATURE_C11` | `1`, `2` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `ModuleStream::parse` returns `UnsupportedVersion` |
| Symbol region absent (size 0) | — | complete ([code][dmod], [check][t:dbi], [check][e15]) | complete ([code][dmod], [check][t:dbi]) | not applicable (no references) | complete ([code][dmodw], [check][t:lines_codec]) | complete ([code][dmodw], [check][t:lines_codec]) | missing (#31: external-input; no fixture has a module stream without symbols) | 97 module records of the fixtures have no module stream; that is a different case |
| C11 line region | — | complete ([code][dmod], [check][t:dbi], [check][e15]) | not applicable (legacy, #16) | not applicable (legacy, #16) | complete ([code][dmodw], [check][t:lines_codec]) | not applicable (legacy, #16) | not applicable (legacy, #16) | Kept raw and written again; empty in all 36 fixture PDBs |
| C13 line region | — | complete ([code][dmod], [check][t:lines], [check][e15]) | complete ([code][dmod], [code][lmod], [check][e6], [check][e31]) | complete ([code][lmod], [check][e6], [check][t:lines]) | complete ([code][lmod], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e6], [check][e31]) | Subsection framing; the tables below give each kind |
| Global references region | — | complete ([code][dmod], [check][t:dbi], [check][e15]) | complete ([code][gref], [check][t:lines_codec], [check][e31]) | complete ([code][gref], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][edsym], [check][e31], [check][e34]) | complete ([code][exl], [check][e31]) | 30 fixture PDBs, 1015 non-empty regions; `GlobalRefs::validate` checks each offset against the symbol record stream |

## Subsection kinds

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DEBUG_S_SYMBOLS` | `0xf1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lsym], [check][t:lines_codec], [check][e31]) | not applicable (object-file kind; its records refer to the type indices of the object file) | complete ([code][lsym], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | partial (#31: only in object files of #27 (7981 subsections re-encode); `link.exe` does not copy it to the PDB; external-input) | No fixture |
| `DEBUG_S_LINES` | `0xf2` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lblk], [check][e6], [check][e31]) | complete ([code][lmod], [code][exl], [check][e6], [check][e31]) | complete ([code][lblk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e6], [check][e31]) | 35 fixture PDBs, 11747 subsections |
| `DEBUG_S_STRINGTABLE` | `0xf3` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lstr], [check][t:lines_codec], [check][e31]) | not applicable (no references) | complete ([code][lstr], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | partial (#31: only in object files of #27 (97 subsections re-encode); `link.exe` does not copy it to the PDB; external-input) | No fixture |
| `DEBUG_S_FILECHKSMS` | `0xf4` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][e6], [check][e31]) | complete ([code][exl], [check][e10], [check][e31]) | complete ([code][lchk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e6], [check][e31]) | 35 fixture PDBs, 1292 subsections; one table for each module |
| `DEBUG_S_FRAMEDATA` | `0xf5` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lfd], [check][t:lines_codec], [check][e31]) | not applicable (object-file kind; frame programs refer to the string table of the object file) | complete ([code][lfd], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | partial (#31: only in object files of #27 (748 subsections re-encode); `link.exe` does not copy it to the PDB; external-input) | No fixture; the relocation value is optional, as `cvdump` and LLVM differ |
| `DEBUG_S_INLINEELINES` | `0xf6` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][linl], [check][e6], [check][e31]) | complete ([code][lmod], [code][exl], [check][e6], [check][e31]) | complete ([code][linl], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e6], [check][e31]) | 17 fixture PDBs, 83 subsections |
| `DEBUG_S_CROSSSCOPEIMPORTS` | `0xf7` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lxs], [check][t:lines_codec], [check][e31]) | partial (#31: external-input; framing and field checks only, because no fixture shows the referenced modules) | complete ([code][lxs], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: external-input; no local producer writes it) | No fixture |
| `DEBUG_S_CROSSSCOPEEXPORTS` | `0xf8` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lxs], [check][t:lines_codec], [check][e31]) | partial (#31: external-input; framing and field checks only, because no fixture shows the referenced modules) | complete ([code][lxs], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: external-input; no local producer writes it) | No fixture |
| `DEBUG_S_IL_LINES` | `0xf9` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lblk], [check][t:lines_codec], [check][e31]) | complete ([code][lmod], [check][t:lines_codec]) | complete ([code][lblk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: external-input; no local producer writes it) | No fixture; the `DEBUG_S_LINES` layout (`cvdump` `DumpModILLines`); C++/CLI with MSVC 14.51 writes none (#27) |
| `DEBUG_S_FUNC_MDTOKEN_MAP` | `0xfa` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][ltok], [check][t:lines_codec], [check][e31]) | not applicable (metadata tokens name the assembly outside the PDB) | complete ([code][ltok], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: external-input; no local producer writes it) | No fixture; C++/CLI with MSVC 14.51 writes none (#27) |
| `DEBUG_S_TYPE_MDTOKEN_MAP` | `0xfb` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][ltok], [check][t:lines_codec], [check][e31]) | not applicable (metadata tokens name the assembly outside the PDB) | complete ([code][ltok], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: external-input; no local producer writes it) | No fixture; C++/CLI with MSVC 14.51 writes none (#27) |
| `DEBUG_S_MERGED_ASSEMBLYINPUT` | `0xfc` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lasm], [check][t:lines_codec], [check][e31]) | not applicable (the record names an assembly outside the PDB) | complete ([code][lasm], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: external-input; no local producer writes it) | No fixture; layout from `cvdump` `DumpModMergedAssemblyInput` |
| `DEBUG_S_COFF_SYMBOL_RVA` | `0xfd` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lrva], [check][t:lines_codec], [check][e31]) | complete ([code][exl], [check][e13s], [check][e31]) | complete ([code][lrva], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e13s], [check][e31]) | 1 fixture PDB, 114 subsections (`msvc-x64-fastlink`); one RVA for each COFF symbol of the object file (#13) |

## Subsection kinds after `cvinfo.h`

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DEBUG_S_INLINEE_MDTOKEN_MAP` | `0xfe` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | missing (#31: external-input; no layout; `LineSubsection::Unsupported` keeps the bytes) | missing (#31: external-input; no layout) | partial (#31: the encoder copies the raw bytes; external-input) | partial (#31: the module stream writer keeps the raw bytes; external-input) | missing (#31: external-input; no local producer) | Only the pdb-rs `line_data.md` names it; no source gives a layout (#31) |
| `DEBUG_S_XFGHASH_TYPE` | `0xff` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lxfg], [check][t:lines_codec], [check][e31]) | not applicable (object-file kind; its type indices refer to the object file) | complete ([code][lxfg], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | partial (#31: only in object files of #27 (7 subsections re-encode); `link.exe` does not copy it to the PDB; external-input) | No fixture; LLVM `DebugSubsectionKind::XfgHashType`; 12-byte entries from the MSVC 14.44 objects of `msvc-x64-xfg` (#31); `link.exe` drops it (#27) |
| `DEBUG_S_XFGHASH_VIRTUAL` | `0x100` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lxfg], [check][t:lines_codec], [check][e31]) | not applicable (object-file kind; its type indices refer to the object file) | complete ([code][lxfg], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | partial (#31: only in object files of #27 (2 subsections re-encode); `link.exe` does not copy it to the PDB; external-input) | No fixture; LLVM `DebugSubsectionKind::XfgHashVirtual`; 16-byte entries from the same objects (#31); `link.exe` drops it (#27) |
| `DEBUG_S_IGNORE` bit | `0x80000000` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lines], [check][t:lines_codec]) | not applicable (a reader ignores the contents) | complete ([code][lines], [check][t:lines_codec]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: external-input; no local producer writes it) | `LineSubsection::Ignored` keeps the kind and the bytes; `cvinfo.h` tells readers to ignore the contents |
| Unknown kind | any other value | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | not applicable (no layout; `SubsectionKind::Unknown` keeps the bytes) | not applicable (no layout) | complete ([code][lines], [check][t:lines_codec]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | not applicable (no producer known) | Distinct from a framing error |

## Subsection variants

These rows split the layout variants inside the decoded kinds.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DEBUG_S_LINES` without columns | flags `0` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lblk], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][lblk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e31], [check][e33]) | 28 fixture PDBs, 11707 items |
| `DEBUG_S_LINES` with columns (`CV_LINES_HAVE_COLUMNS`) | flags `1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lblk], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][lblk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e31], [check][e33]) | 7 fixture PDBs, 40 items |
| `DEBUG_S_FILECHKSMS` checksum kind `None` | `0` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][lchk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e31], [check][e33]) | 4 fixture PDBs, 47 items |
| `DEBUG_S_FILECHKSMS` checksum kind `Md5` | `1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][lchk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e31], [check][e33]) | 11 fixture PDBs, 128 items |
| `DEBUG_S_FILECHKSMS` checksum kind `Sha1` | `2` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][lchk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | missing (#31: no fixture; `cl /ZH:SHA1` can make it) | No fixture |
| `DEBUG_S_FILECHKSMS` checksum kind `Sha256` | `3` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][lchk], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][lchk], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e31], [check][e33]) | 28 fixture PDBs, 82086 items |
| `DEBUG_S_INLINEELINES` signature `CV_INLINEE_SOURCE_LINE_SIGNATURE` | `0` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][linl], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][linl], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e31], [check][e33]) | 11 fixture PDBs, 77 items |
| `DEBUG_S_INLINEELINES` signature with extra files | `1` | complete ([code][dmod], [code][lines], [check][t:lines], [check][e15]) | complete ([code][linl], [check][t:lines_codec]) | complete ([code][lmod], [check][e31], [check][t:lines]) | complete ([code][linl], [check][t:lines_codec], [check][e31]) | complete ([code][dmodw], [code][lmod], [check][e31], [check][e33]) | complete ([code][exl], [check][e31], [check][e33]) | 6 fixture PDBs, 6 items |

[dmod]: ../../../crates/pdbrs/src/native/dbi/module.rs
[dmodw]: ../../../crates/pdbrs/src/native/dbi/module/write.rs
[e10]: https://github.com/napbat/pdbrs/issues/10#issuecomment-6407894516
[e13s]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409780019
[e15]: https://github.com/napbat/pdbrs/issues/15#issuecomment-6410580545
[e30]: https://github.com/napbat/pdbrs/issues/30#issuecomment-6413701533
[e31]: https://github.com/napbat/pdbrs/issues/31#issuecomment-6412958507
[e33]: https://github.com/napbat/pdbrs/issues/33#issuecomment-6415053819
[e34]: https://github.com/napbat/pdbrs/issues/34#issuecomment-6415135725
[e5]: https://github.com/napbat/pdbrs/issues/5#issuecomment-6407599155
[e6]: https://github.com/napbat/pdbrs/issues/6#issuecomment-6407539677
[edit]: ../../../crates/pdbrs/src/native/edit.rs
[edsym]: ../../../crates/pdbrs/src/native/edit/symbols.rs
[exl]: ../../../crates/pdbrs/examples/native/lines.rs
[gref]: ../../../crates/pdbrs/src/native/dbi/module/global_refs.rs
[lasm]: ../../../crates/pdbrs/src/native/lines/assembly_input.rs
[lblk]: ../../../crates/pdbrs/src/native/lines/blocks.rs
[lchk]: ../../../crates/pdbrs/src/native/lines/checksums.rs
[lfd]: ../../../crates/pdbrs/src/native/lines/frame_data.rs
[lines]: ../../../crates/pdbrs/src/native/lines.rs
[linl]: ../../../crates/pdbrs/src/native/lines/inlinee.rs
[lmod]: ../../../crates/pdbrs/src/native/lines/module.rs
[lrva]: ../../../crates/pdbrs/src/native/lines/coff_symbol_rva.rs
[lstr]: ../../../crates/pdbrs/src/native/lines/string_table.rs
[lsym]: ../../../crates/pdbrs/src/native/lines/symbol_records.rs
[ltok]: ../../../crates/pdbrs/src/native/lines/token_map.rs
[lxfg]: ../../../crates/pdbrs/src/native/lines/xfg_hash.rs
[lxs]: ../../../crates/pdbrs/src/native/lines/cross_scope.rs
[sscope]: ../../../crates/pdbrs/src/native/codeview/symbol/scope.rs
[sscw]: ../../../crates/pdbrs/src/native/codeview/symbol/scope/write.rs
[symv]: ../../../crates/pdbrs/src/native/codeview/symbol/validate.rs
[t:dbi]: ../../../crates/pdbrs/tests/dbi.rs
[t:lines]: ../../../crates/pdbrs/tests/lines.rs
[t:lines_codec]: ../../../crates/pdbrs/tests/lines_codec/main.rs
[t:symbol_validation]: ../../../crates/pdbrs/tests/symbol_validation.rs
