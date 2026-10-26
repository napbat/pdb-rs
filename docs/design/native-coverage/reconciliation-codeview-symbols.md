# Native coverage: reconciliation of the Microsoft pdb-rs CodeView symbol documents

This page is part of the [native coverage](../native-coverage.md) accounting. It covers `docs/reference/microsoft-pdb-rs/docs/codeview/symbols/`.
Each section names one imported document. Paths are relative to `docs/reference/`. The fields are:

- **Documents**: the format items that the document describes.
- **pdbrs**: `implemented` (pdbrs reads every item that the document describes), `partial`, `not implemented` or `not applicable` (no format content), with the source files.
- **Conflicts**: each difference between the document and pdbrs, or between the document and another source (`cvinfo.h`, LLVM source, `ms-pdb`, other documents). Errors inside the document are conflicts too.
- **Undocumented assumptions**: layouts and rules that pdbrs uses but the document does not state, with the issue that records the evidence.

The comparison used `main` at `b4d4cdc`, `cvinfo.h` at `805655a2`, LLVM at `b9f2235f` and the `ms-codeview` 0.1.7, `ms-pdb` 0.1.22, `ms-pdb-msf` 0.1.7 and `ms-pdb-msfz` 0.1.10 crates. "Inference, not tested" marks a statement that comes from reading source code only.

The pinned cvinfo.h does not define `THUNK_ORDINAL`, `CV_CFL_LANG` or `CV_CPU_TYPE_e`. Those enums are in DIA `cvconst.h`. For them I compared against LLVM only. The pinned references do not give the layouts of the symbol kinds above 0x1168. For these kinds, pdbrs uses the debug type records of `cvinfo.h` in the DIA SDK of Visual Studio 18 (`lib/amd64/diaguids.lib`) (#30).

Each decoder in `crates/pdbrs/src/native/codeview/symbol/` has an encoder next to it (`SymbolData::encode`, `Symbol::encode`, `codeview/symbol/encode.rs`). A decoded record encodes to its original bytes (#30). The sections below name the decoder only.

### `microsoft-pdb-rs/docs/codeview/symbols/symbols.md`
- Documents: The symbol record framing (u16 size, u16 kind, payload, 4-byte alignment, padding), global vs. module symbol streams, a summary table of symbol kinds, and the scope-opening kinds with the `S_END` stack rule.
- pdbrs: implemented — `codeview.rs` `Records` frames records with the same length rule (length counts the kind, minimum 2, `MIN_RECORD_LEN`). `codeview/symbol.rs` `decode_data` decodes 126 kinds (`S_SKIP` included) and gives `SymbolData::Unsupported` (raw payload) for others. Every kind of the summary table and of the nested-scope table decodes, also `S_ANNOTATION`, `S_DATAREF`, `S_ANNOTATIONREF`, `S_DEFRANGE`, `S_DEFRANGE_SUBFIELD`, `S_INLINESITE2`, `S_LPROC32_DPC_ID` and `S_SEPCODE` (#30). `codeview/symbol/scope.rs` `ModuleSymbols` keeps a scope stack and checks parent links, end links, end-record kind and a depth limit. It opens a scope for every decoded scope kind; no scope kind is rejected now. `codeview/symbol/validate.rs` checks the other rules (#30). `codeview/symbol/scope/write.rs` `ModuleSymbolWriter` and `symbols/records.rs` `SymbolRecordWriter` write module symbols and the symbol record stream; `native::PdbBuilder` (#33) and `native::PdbEditor` (#34) use them. The table values 0x1007/0x1008 that the table gives for `S_GDATA32`/`S_LDATA32` are `S_LDATA32_ST`/`S_GDATA32_ST`; pdbrs keeps those `_ST` kinds raw (see below).
- Conflicts:
  - Padding bytes: the document says the padding uses 0xF1, 0xF2, 0xF3. pdbrs requires zero bytes after the fields of a decoded symbol (`Symbol::decode` in `codeview/symbol.rs`). A nonzero byte gives `InvalidFormat` ("CodeView symbol record has invalid padding"). LLVM lld `fixRecordAlignment` (`lld/COFF/PDB.cpp`) writes zero bytes. The document's own `S_ANNOTATION` example (`s_annotation.md`) shows `00 00` padding. ms-codeview `syms/builder.rs` lines 67-69 writes F1/F2/F3, and its `test_parse_proc` uses `0xf1, 0xf2`. pdbrs rejects such records with `InvalidFormat`. Type records use F1/F2/F3 (`Fields::finish_type_padding`); symbol records do not.
  - Padding length: the document allows at most 3 padding bytes. The Windows SDK HLSL compiler `fxc` writes `S_LOCAL` records of a fixed length, so 4 or more zero bytes follow the name (`hlsl-fxc-sm50-debug`, #27). `ml64` writes an extra NUL after the `S_COMPILE3` version string. pdbrs accepts any number of zero bytes. `Symbol::padding` gives the count, and `Symbol::encode`, `ModuleSymbolWriter::push_symbol` and `SymbolRecordWriter::push_symbol` write the same count again. `SymbolData::encode` writes the shortest zero padding (#30).
  - Kind values: the table gives `S_GDATA32` = 0x1007 and `S_LDATA32` = 0x1008. cvinfo.h (lines 2866-2867), LLVM `CodeViewSymbols.def` and pdbrs (`symbol.rs` `S_LDATA32` = 0x110c, `S_GDATA32` = 0x110d) give 0x110d and 0x110c. In cvinfo.h, 0x1007 is `S_LDATA32_ST` and 0x1008 is `S_GDATA32_ST`.
  - Kind value: the table gives `S_INLINESITE2` = 0x115C. cvinfo.h line 2979, LLVM, ms-codeview `kind.rs` line 161 and pdbrs `procedure.rs` give 0x115d. 0x115c is `S_POGODATA`, which pdbrs decodes (`ProfileData`, `codeview/symbol/linker.rs`).
  - Scope ends: the document says that `S_END` ends all scopes. cvinfo.h and LLVM `symbolEndsScope` use `S_INLINESITE_END` (0x114e) for `S_INLINESITE` and `S_INLINESITE2`, and allow `S_PROC_ID_END` (0x114f). pdbrs `scope.rs` closes inline sites only with `S_INLINESITE_END` and other scopes with `S_END` or `S_PROC_ID_END`. A wrong end kind gives `InvalidFormat`. The validation also reports `S_PROC_ID_END` for a scope that is not an `_ID` procedure (`Rule::ProcedureIdEndKind`) and `S_END` for an `_ID` procedure (`Rule::ProcedureIdEndMissing`) (#30).
  - Alignment: the document requires each record start and size to be a multiple of 4. pdbrs `Records` does not check alignment. Decoded kinds must end with zero bytes only. Undecoded kinds and `S_SKIP` keep their payload without checks. `ModuleStreamParts::encode` requires that the symbol region ends at a 4-byte boundary (`Mod1::fCommit`, #31).
  - Nested-scope table: it lists `S_GPROC32_ID` but not `S_LPROC32_ID` (0x1146). LLVM `symbolOpensScope` and pdbrs open a scope for both. It omits `S_LPROC32_DPC` (0x1155), which LLVM lists and pdbrs decodes and opens (#30).
  - Internal: the text says "Each PDB contains one Global Symbol Stream" and then "Some PDBs do not contain a GSS". pdbrs treats the DBI symbol record stream as optional (`examples/native/symbols.rs` returns early if `symbol_stream` is absent).
  - Internal: the summary table omits `S_SEPCODE`, but the nested-scope table lists it. The table also lists 0x1137 before 0x1136.
  - Omissions: the table omits kinds that pdbrs decodes: `S_SKIP` 0x0007, `S_OEM` 0x0404, `S_VFTABLE32` 0x100c, `S_WITH32` 0x1104, `S_REGISTER` 0x1106, `S_BPREL32` 0x110b, `S_MANSLOT` 0x1120, `S_FRAMECOOKIE` 0x113a, `S_ENVBLOCK` 0x113d, `S_LPROC32_ID` 0x1146, `S_GPROC32_ID` 0x1147, `S_INLINESITE_END` 0x114e, `S_PROC_ID_END` 0x114f, `S_DEFRANGE_HLSL` 0x1150, `S_GDATA_HLSL` 0x1151, `S_LDATA_HLSL` 0x1152, `S_FILESTATIC` 0x1153, `S_LOCAL_DPC_GROUPSHARED` 0x1154, `S_LPROC32_DPC` 0x1155, `S_DEFRANGE_DPC_PTR_TAG` 0x1157, `S_DPC_SYM_TAG_MAP` 0x1158, `S_CALLEES` 0x115a, `S_CALLERS` 0x115b, `S_POGODATA` 0x115c, `S_HEAPALLOCSITE` 0x115e, `S_GDATA_HLSL32` to `S_LDATA_HLSL32_EX` 0x1162-0x1165, `S_FRAMEREG` 0x1166, `S_FASTLINK` 0x1167, `S_INLINEES` 0x1168, and the kinds above 0x1168, for example `S_HOTPATCHFUNC` 0x1169, `S_GPROC32EX` 0x1172, `S_STATICLOCAL` 0x1176, `S_DEFRANGE_REGISTER_REL_INDIR` 0x1177, `S_REGREL32_ENCTMP` 0x1179, `S_ASSOCIATION` 0x117c, `S_HYBRIDRANGE` 0x117d and `S_DEFRANGE_CONSTVAL_ON_ENTRY` 0x117f (#30).
  - Top-level column: the nested-scope table marks `S_BLOCK32`, `S_INLINESITE` and `S_INLINESITE2` as not top-level. The pdbrs validation agrees (`Rule::ScopeWithoutParent`, `NESTED_SCOPES` in `validate/module.rs`). `ModuleSymbols` alone accepts them at the top level (#30).
- Undocumented assumptions:
  - Scope links are module-stream offsets that include the 4-byte stream signature: record offset + base 4 (`scope.rs`, `ScopeLinks` in `symbol.rs`). Only C13 module streams (signature 4) are supported (#5).
  - The parent link of a scope opener must equal the offset of the enclosing scope, or 0 at top level. The end link must be after the start and must be the offset of the matching end record. The walk does not check `p_next`; the validation does (`Rule::NextLink`: 0 or a later procedure or thunk) (#5, #30).
  - `ModuleSymbolWriter` computes the parent and end links of each scope record and ignores the given values. It keeps the `next` link. The MSVC, LLVM and .NET producers of the fixtures write 0 there (#30).
  - The scope walk has a caller-set depth limit (`max_depth`) and returns `LimitExceeded` above it. The validation uses `DEFAULT_MAX_DEPTH` = 256.
  - The validation (`validate_module_symbols`, `validate_shared_symbols`) collects violations of `validate::Rule`: scope nesting and code ranges, type and ID index ranges and ID kinds, module references, def-range placement and gaps, def-ranges in inline sites (`Rule::DefRangeOutsideInlineSite`, `Rule::InlineSiteAnnotations`), subfield offsets against the type size (`Rule::SubfieldOutsideType`), local placement and flags, and register numbers for the module CPU (`CV_HREG_e` ranges from DIA `cvconst.h`) (#30).
  - In a PDB with the Info feature `NOTM` (`link /DEBUG:LAZY`, fixture `msvc-x64-lazy`), each module stream starts with `S_MOD_TYPEREF`, and the type and ID indices of the module symbols are indices of the object file, not of TPI and IPI (both have 0 records). The flags of `MODTYPEREF` give the records: `fNone` 0x01 (no types), `fOwnTMR` 0x08 (`word0` is one stream of the `.debug$T` records; types and IDs share one index space from 0x1000), `fOwnTM` 0x10 (`word0` is the type stream and `word1` the ID stream of the type server), `fRefTM` 0x20 (`word0` is the 0-based index of the owner module). `ModuleTypeReference::source` gives `ModuleTypeSource` (`symbol/linker.rs`); other flag values stay `ModuleTypeSource::Other`. The module symbols resolve and validate against these records (`TypeStream::from_records`, `ExternalDependencies::{no_type_merge, module_types}`) (#30, #32).
  - `S_SKIP` (0x0007) is a filler; LLD writes a zero-filled `S_SKIP` for a record that it cannot remap (`symbol.rs`; lld `replaceWithSkipRecord`).
  - Names are NUL-terminated byte strings. pdbrs does not decode them as text and does not demangle them (`symbol.rs` module doc).
  - Layouts of the kinds above 0x1168 (`PROCSYM32EX`, `BPRELSYM32INDIR`, `REGREL32INDIR`, `STATICLOCAL`, `HYBRIDRANGESYM`, `DEFRANGECONSTVAL_ON_ENTRY`, `DEFRANGEGLOBALSYM_ON_ENTRY`, `REFMINIPDB2`, the `_ENCTMP` typedefs and `SOURCELINKSYM`) come from the DIA SDK `diaguids.lib` debug type records (#30).
  - `S_ALTOBJNAME` (0x1181), `S_LOCAL_2005`, `S_DEFRANGE_2005`, `S_DEFRANGE2_2005` and `S_RESERVED1` to `S_RESERVED4` have no layout in any reference. The pre-VC70 `_ST` kinds (for example 0x1007, 0x1008, 0x100a) have length-prefixed names. pdbrs keeps all of them raw (`Unsupported`) and encodes them raw (#30).
  - Fixture-confirmed layouts for kinds without a reference layout: `S_FRAMEREG` (u16 register + 2 zero bytes), `S_FASTLINK` (`REFMINIPDB` without module index; flag bit 6 = namespace), `S_REGREL32_ENCTMP` (= `S_REGREL32` layout) (#13).
  - Managed kinds use packed cvinfo.h layouts (#14).
  - Producers of the kinds above `0x1168` (#39): MSVC 14.51 writes `S_ASSOCIATION` for C++20 coroutines and `S_REGREL32_INDIR` for structured bindings; x86 `/ZI` writes `S_BPREL32_ENCTMP` and `S_BPREL32_INDIR`; `link` keeps `S_STATICLOCAL` only with `/DEBUG:LAZY`; an ARM64EC link writes `S_HYBRIDRANGE`. The DIA SDK layouts decode all of them.

### `microsoft-pdb-rs/docs/codeview/symbols/s_annotation.md`
- Documents: The `S_ANNOTATION` (0x1019) record: u32 offset, u16 section, u16 string count, then `count` NUL-terminated strings, with an MSVC `__annotation` example.
- pdbrs: implemented — `AnnotationStrings::read` in `crates/pdbrs/src/native/codeview/symbol/code.rs` reads the address, the count and `count` NUL-terminated strings (`SymbolData::AnnotationStrings`). A string without a NUL gives `InvalidFormat`. `AnnotationStrings::iter` returns the strings. `AnnotationStrings::new` rejects strings without a final NUL and more than 65535 strings. pdbrs also decodes `S_ANNOTATIONREF` (0x1128), which points to this record (`global.rs`, #30).
- Conflicts:
  - None found for the layout: cvinfo.h `ANNOTATIONSYM` (off, seg, csz, rgsz), LLVM `AnnotationSym`, ms-codeview `AnnotationFixed` and pdbrs agree with the document.
  - Omission: the example source uses wide strings (`L"Hello!"`), but the record bytes hold single-byte strings. The document does not state the string encoding. pdbrs keeps the strings as bytes.
  - The example dump shows zero padding (`00 00` after `World!\0`). This contradicts the 0xF1/0xF2 padding rule in `symbols.md`. pdbrs requires zero padding, so it accepts the example bytes (Inference, not tested).
- Undocumented assumptions:
  - Fixture `msvc-x64-annotation` (MSVC 14.51, `__annotation`) has 3 `S_ANNOTATION` module records and 3 `S_ANNOTATIONREF` global records. pdbrs, `llvm-pdbutil`, DIA2Dump and `pdbtool` read them with the same result, and the records re-encode with 0 differences (#30).
  - The validation accepts an `S_ANNOTATIONREF` only if it identifies an `S_ANNOTATION` record (`Rule::ModuleRecordKind`, `validate.rs`) (#30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_armswitchtable.md`
- Documents: The `S_ARMSWITCHTABLE` (0x1159) record (base offset/section, switch type, branch offset, table offset, branch section, table section, entry count) and switch types 0-8.
- pdbrs: implemented — `JumpTable::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the same field order and gives `SymbolData::JumpTable`. `JumpTableEntry` names values 0-10.
- Conflicts:
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: the document lists values 0-8 only. cvinfo.h `CV_armswitchtype` also defines `CV_SWT_INT1SHL1` = 9 and `CV_SWT_INT2SHL1` = 10, and the aliases `CV_SWT_TBB` = 7 and `CV_SWT_TBH` = 8. LLVM `JumpTableEntrySize` and pdbrs `JumpTableEntry` (`INT8_SHIFT_LEFT` = 9, `INT16_SHIFT_LEFT` = 10) include 9 and 10.
  - ms-codeview vs document/cvinfo.h: the ms-codeview comment on `UINT2SHL1` says "shift left by 2". The name and the document say shift left by 1.
  - The claims "MSVC generates this symbol only when targeting ARM64" and "LLVM generates it for all targets" have no source in the document. I did not verify them.
- Undocumented assumptions: None. pdbrs does not enforce the "module streams only" rule.

### `microsoft-pdb-rs/docs/codeview/symbols/s_block.md`
- Documents: The `S_BLOCK32` (0x1103) record (p_parent, p_end, length, offset, segment, name) and its nesting rule.
- pdbrs: implemented — `Block::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the same layout. `scope.rs` opens a scope and checks the parent and end links. The validation (`validate/module.rs`) reports a top-level block (`Rule::ScopeWithoutParent`) and a code range outside the enclosing scope (`Rule::ScopeRange`) (#30).
- Conflicts: None found. The layout agrees with cvinfo.h `BLOCKSYM32` and LLVM `BlockSym`.
- Undocumented assumptions:
  - The link values are module-stream offsets that include the 4-byte signature (#5).
  - The nesting rule is a validation rule. `ModuleSymbols` alone accepts a top-level block with parent 0 (#30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_buildinfo.md`
- Documents: The `S_BUILDINFO` (0x114c) record: one `ItemId` that refers to an `LF_BUILDINFO` record in the IPI stream.
- pdbrs: implemented — `decode_data` in `crates/pdbrs/src/native/codeview/symbol.rs` reads an `IdIndex` (`SymbolData::BuildInfo`). `Symbol::id_index` returns it. The validation checks that the index is in the IPI stream (`Rule::IdIndexRange`) and names an `LF_BUILDINFO` record (`Rule::IdKind`, `validate.rs`) (#30).
- Conflicts: None found. The layout agrees with cvinfo.h `BUILDINFOSYM`.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_callsiteinfo.md`
- Documents: The `S_CALLSITEINFO` (0x1139) record (offset, section, reserved u16, type index) and the type shapes for indirect calls.
- pdbrs: implemented — `CallSiteInfo::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the layout and rejects a nonzero reserved field with `InvalidFormat`. The validation checks only that the type index is in the TPI stream or is a known primitive (`Rule::TypeIndexRange`, `Rule::PrimitiveType`). pdbrs does not check the shape of the referenced type or the nesting rule.
- Conflicts:
  - Document vs cvinfo.h: the document gives `offset` as `uint32_t`; cvinfo.h `CALLSITEINFO` gives `CV_off32_t` (signed). pdbrs reads it as u32 in `SectionOffset`. The width is the same.
  - Internal: line 35 and line 36 duplicate the same sentence fragment.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_coffgroup.md`
- Documents: The `S_COFFGROUP` (0x1137) record (size, characteristics, offset, segment, name) in the `* Linker *` module.
- pdbrs: implemented — `CoffGroup::read` in `crates/pdbrs/src/native/codeview/symbol/global.rs` reads the same layout. `Symbol::address` returns the start address.
- Conflicts: None found. The layout agrees with cvinfo.h `COFFGROUPSYM`, LLVM `CoffGroupSym` and ms-codeview `CoffGroupFixed`.
- Undocumented assumptions: None. pdbrs does not enforce the "`* Linker *` module only" rule.

### `microsoft-pdb-rs/docs/codeview/symbols/s_compile.md`
- Documents: The `S_COMPILE2` (0x1116) and `S_COMPILE3` (0x113c) records: flags (language in bits 0-7 and flag bits), machine, front-end and back-end versions, version string; and the language enumeration 0-22.
- pdbrs: implemented — `Compile2::read` and `Compile3::read` in `crates/pdbrs/src/native/codeview/symbol/compile.rs`. `CompileFlags` names bits 8-19. `SourceLanguage` names 0x00-0x16, `D` (0x44) and `OLD_SWIFT` (0x53). `machine` stays a raw u16 (`CV_CPU_TYPE_e`). `Compile2` also reads the optional extra string block (`StringList`).
- Conflicts:
  - Document vs cvinfo.h: the flag table gives `MSILModule` as bit 16 and `pad` as bits 16-31. The ranges overlap. cvinfo.h `COMPILESYM` gives `pad : 15` (bits 17-31).
  - Document vs cvinfo.h/LLVM/pdbrs: for `S_COMPILE3`, cvinfo.h `COMPILESYM3` defines `fSdl` (bit 17), `fPGO` (bit 18), `fExp` (bit 19) and `pad : 12` (bits 20-31). LLVM `CompileSym3Flags` and pdbrs `CompileFlags::{SDL, PGO, EXP}` agree. The document omits these bits.
  - Document vs LLVM/pdbrs: the document says values 23-255 are reserved. LLVM `CodeViewLanguages.def` defines `D` = 'D' (0x44, DMD) and `OldSwift` = 'S' (0x53). pdbrs decodes both.
  - Document vs cvinfo.h/LLVM/pdbrs: the `Compile2` struct ends with `strz name`. cvinfo.h `COMPILESYM` says the version string is "followed by an optional block of zero terminated strings terminated with a double zero". LLVM `Compile2Sym` has `ExtraStrings`. pdbrs reads them as `extra_strings`.
  - Omission: the document does not list `machine` values (`CV_CPU_TYPE_e`). pdbrs keeps the value raw and does not name it.
  - The rule "exactly one `S_COMPILE2`/`S_COMPILE3` per module" is not enforced by pdbrs.
- Undocumented assumptions:
  - `ml64` writes an extra NUL after the `S_COMPILE3` version string, so 4 or more zero bytes follow it. pdbrs accepts any number of zero bytes there and rejects nonzero bytes. cvdump and LLVM ignore the bytes (#5). `Compile3::trailing_zeros` keeps the count, so the encoder writes the same bytes (#30).
  - The `S_COMPILE2` string block can be absent or empty (only the final NUL). `StringList::is_terminated` tells the two forms apart, and the encoder keeps the stored form (#30). LLVM `mapStringZVectorZ` gives the same value for both.
  - The validation uses the CPU of the first compile record of a module to check register numbers (`Rule::RegisterUnknown`, `validate/register.rs`) (#30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_constant.md`
- Documents: The `S_CONSTANT` (0x1107) record (type index, numeric value, name) and the `S_MANCONSTANT` (0x112d) record (metadata token, numeric value, name).
- pdbrs: implemented — `Constant::read` in `crates/pdbrs/src/native/codeview/symbol/variable.rs` and `ManagedConstant::read` in `codeview/symbol/managed.rs` read the value as `Numeric` (`codeview/numeric.rs`). `Numeric` decodes every numeric leaf tag of cvinfo.h (integers, REAL*, COMPLEX*, `LF_VARSTRING`, `LF_DECIMAL`, `LF_DATE`, `LF_UTF8STRING`, `LF_REAL16`) and keeps the exact bytes, so a non-canonical integer width encodes to the same bytes (#29, #30). Only a tag that cvinfo.h does not define gives `UnsupportedVersion`.
- Conflicts:
  - Document vs cvinfo.h/pdbrs: the document names the `S_MANCONSTANT` field `metadata_token`. cvinfo.h `CONSTSYM` gives "Type index … or metadata token". pdbrs keeps it as `type_or_token`; Roslyn writes a `StandAloneSig` token (table 0x11) (#14).
- Undocumented assumptions:
  - The numeric leaf has no alignment inside the record; the name follows it directly (`numeric.rs`).
  - No native fixture has a non-integer `S_CONSTANT` value. Roslyn writes integer, `LF_VARSTRING`, real, decimal and date values in `S_MANCONSTANT` (`managed.rs`, #14, #29).

### `microsoft-pdb-rs/docs/codeview/symbols/s_data.md`
- Documents: The `S_GDATA32` and `S_LDATA32` records (type index, offset, segment, name) and where they appear.
- pdbrs: implemented — `Data::read` in `crates/pdbrs/src/native/codeview/symbol/variable.rs` reads the layout for `S_GDATA32` (0x110d) and `S_LDATA32` (0x110c) (`SymbolData::Data`). `S_GMANDATA`/`S_LMANDATA` use the same layout with a metadata token (`SymbolData::ManagedData`, #30).
- Conflicts:
  - Document vs cvinfo.h/LLVM/pdbrs: the title gives `S_GDATA32` = 0x1007 and `S_LDATA32` = 0x1008. The real values are 0x110d and 0x110c. In cvinfo.h, 0x1007 is `S_LDATA32_ST` and 0x1008 is `S_GDATA32_ST` (the order is also reversed). pdbrs keeps 0x1007 and 0x1008 raw (`Unsupported`).
  - Internal: typo "rercords".
- Undocumented assumptions:
  - pdbrs does not enforce the global/module location rules. lld `symbolGoesInModuleStream` agrees with the document (S_GDATA32 not in module streams).
  - An `S_DATAREF` in the global stream can identify these records. The validation accepts `S_LDATA32`, `S_GDATA32`, the thread kinds and the managed data kinds as targets (`validate.rs` `DATA`) (#30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_end.md`
- Documents: The `S_END` (0x0006) record, which has no payload and ends a scope that `S_LPROC32`, `S_GPROC32`, `S_THUNK32`, `S_INLINESITE` and others open.
- pdbrs: implemented — `decode_data` in `crates/pdbrs/src/native/codeview/symbol.rs` gives `SymbolData::ScopeEnd` for `S_END` and `S_PROC_ID_END` (0x114f). `scope.rs` closes the innermost non-inline scope and checks that the end link of the opener names this record. `ModuleSymbolWriter` writes the end link of the opener when the end record arrives (#30).
- Conflicts:
  - Document vs cvinfo.h/LLVM/pdbrs: the document says that `S_END` ends `S_INLINESITE`. `S_INLINESITE_END` (0x114e) ends it. pdbrs rejects `S_END` for an open inline site with `InvalidFormat` ("symbol scope end kind does not match its scope").
  - Omission: the document does not name `S_PROC_ID_END` (0x114f), which ends an `_ID` procedure. The validation reports `S_END` for an `_ID` procedure (`Rule::ProcedureIdEndMissing`) and `S_PROC_ID_END` for another scope (`Rule::ProcedureIdEndKind`) (#30).
- Undocumented assumptions: An `S_END` without an open scope gives `InvalidFormat`. A scope without an end record at the end of the stream gives `InvalidFormat` (#5).

### `microsoft-pdb-rs/docs/codeview/symbols/s_frameproc.md`
- Documents: The `S_FRAMEPROC` (0x1012) record (frame size, pad size, pad offset, saved-register size, exception handler offset and section, flags) and flag bits 0-8.
- pdbrs: implemented — `FrameProcedure::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the same packed layout. `FrameProcedureFlags` names bits 0-13 and 18-22. `local_frame_pointer()` reads bits 14-15 and `parameter_frame_pointer()` reads bits 16-17 (`FramePointer` 0-3).
- Conflicts:
  - Document vs cvinfo.h/LLVM/pdbrs: the document says bits 9-31 are padding and must be zero. cvinfo.h `FRAMEPROCSYM` and LLVM `FrameProcedureOptions` define `fAsyncEH` 9, `fGSNoStackOrdering` 10, `fWasInlined` 11, `fGSCheck` 12, `fSafeBuffers` 13, `encodedLocalBasePointer` 14-15, `encodedParamBasePointer` 16-17, `fPogoOn` 18, `fValidCounts` 19, `fOptSpeed` 20, `fGuardCF` 21, `fGuardCFW` 22, and `pad : 9` (bits 23-31). pdbrs follows cvinfo.h/LLVM.
  - ms-codeview vs document/cvinfo.h/LLVM/pdbrs: ms-codeview `FrameProc` (syms.rs line 606) has an extra `padding: U16` between `exception_handler_section` and `flags`. cvinfo.h is packed (`#pragma pack(1)`), LLVM maps `Flags` directly after `SectionIdOfExceptionHandler`, and pdbrs reads flags at offset 22 of the payload. With the ms-codeview layout, the flags value is read 2 bytes too late.
  - The rules "at most one `S_FRAMEPROC` per procedure" and "directly nested in the procedure" are not enforced by pdbrs. The validation has no rule for them.
- Undocumented assumptions:
  - The x64 meaning of the encoded frame pointer values (1 = RSP, 2 = RBP, 3 = R13) comes from LLVM `decodeFramePtrReg` (`procedure.rs`).
  - MSVC ARM64 writes `S_FRAMEREG` (0x1166: u16 register + 2 zero bytes) after `S_FRAMEPROC` in procedures with exception handling and in their funclets (fixture-confirmed, #13).

### `microsoft-pdb-rs/docs/codeview/symbols/s_inlinesite.md`
- Documents: The `S_INLINESITE` (0x114d) record (p_parent, p_end, inlinee) and the `S_INLINESITE2` record (p_parent, p_end, inlinee, binary annotations).
- pdbrs: implemented — `InlineSite::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads parent, end, inlinee (`IdIndex`), for `S_INLINESITE2` (0x115d) the u32 `invocations`, and the binary annotations (`SymbolData::InlineSite`, #30). `BinaryAnnotations` in `codeview/symbol/annotation.rs` validates and decodes opcodes 1-13, compressed unsigned integers (1, 2 or 4 bytes; first byte 0xE0-0xFF is invalid), signed operands (bit 0 = sign, `DecodeSignedInt32`), opcode 11 (code delta in bits 0-3, signed line delta above) and opcode 12 (length, then delta). Opcode 0 is accepted only as trailing padding: fewer than 4 bytes, all zero. `Annotation::encode` writes the opcodes again. `S_INLINESITE_END` (0x114e) closes both scope kinds. `codeview/symbol/annotation/code.rs` (`CodeState`, `separated_chunks`) gives the code ranges for the validation and for `native::locate`. `native::locate::SymbolLocator` gives the inline chain for an address, also in separated code (`InlineRange::chunk`, `SymbolLocator::code_chunks`) (#30).
- Conflicts:
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: the title gives `S_INLINESITE2` = 0x115c. The value is 0x115d. 0x115c is `S_POGODATA`.
  - Document vs cvinfo.h/LLVM/pdbrs: the `InlineSite` struct has no binary annotations. cvinfo.h `INLINESITESYM` and LLVM `InlineSiteSym` end with `binaryAnnotations`. pdbrs decodes them.
  - Document vs cvinfo.h/ms-codeview/pdbrs: the `InlineSite2` struct omits `uint32_t invocations` after `inlinee`. cvinfo.h `INLINESITESYM2`, ms-codeview `InlineSite2Fixed` and pdbrs (`InlineSite::invocations`) have it. The MSVC PGO fixture (`msvc-x64-pgo`, 202 records) confirms the layout (#27, #30).
  - Omission: the document does not state that `S_INLINESITE_END` (0x114e), not `S_END`, ends the scope.
  - Omission: the document does not describe the binary annotation opcodes or the compressed integer encoding (cvinfo.h `BinaryAnnotationOpcode`, `CVUncompressData`).
- Undocumented assumptions:
  - The `p_end` link must be the module-stream offset (with the 4-byte signature) of the `S_INLINESITE_END` record (#5).
  - The validation reports a top-level inline site (`Rule::ScopeWithoutParent`) and an inlinee that is not an `LF_FUNC_ID` or `LF_MFUNC_ID` record (`Rule::IdKind`). A parameter local can be directly in an inline site (`Rule::ParameterScope`) (#30).
  - An inline site counts as empty if it has code length annotations and all of them are 0. A def-range of length 0 is valid only in an empty scope (`Rule::DefRangeEmpty`, `inline_site_is_empty` in `validate/module.rs`) (#30).
  - `BA_OP_ChangeCodeOffsetBase` (opcode 2) selects the n-th separated code chunk (`cvinfo.h`: "nth separated code chunk (main code chunk == 0)") and sets the code offset to 0. Chunk 0 is the procedure; chunk n is the n-th `S_SEPCODE` record of the module whose parent address is the procedure start, in stream order. MSVC writes these `S_SEPCODE` records at the top level after the `S_END` of the procedure. A last range without a length ends at the end of its chunk. `msvc-x64-pgo` has 62 sites with this opcode; DIA agrees with `native::locate` at all 720 addresses of the separated code (#30).
  - The validation reports a def-range in an inline site that does not overlap or touch the code of its innermost or an enclosing inline site (`Rule::DefRangeOutsideInlineSite`). Gaps are not part of the range. Def-ranges of length 0 and def-ranges with the `CV_RANGEATTR` "may have no name" bit are not checked. "Touch" and enclosing sites are necessary because MSVC, Clang, rustc and fxc write def-ranges past the inlined code, and MSVC PGO writes def-ranges of an inner site in the separated code of an outer site. Annotations that give no valid code ranges (offset overflow, a range before an earlier range) give `Rule::InlineSiteAnnotations`, and the def-ranges of that site are not checked. One finding remains in the fixtures: a producer inconsistency of MSVC 19.51 PGO in `msvc-x64-pgo` (#30).
  - `ChangeFile` (opcode 5) gives an offset in the module file-checksum subsection (`annotation.rs`).
  - In a `NOTM` PDB, the inlinee of an inline site is an ID of the module records that `S_MOD_TYPEREF` names, not an IPI index; `SymbolLocator` resolves it there (`msvc-x64-amp-lazy`, `msvc-x64-modern-lazy`, #30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_label.md`
- Documents: The `S_LABEL32` (0x1105) record (offset, segment, u8 flags, name).
- pdbrs: implemented — `Label::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the layout. The flags use `ProcedureFlags` (`CV_PROCFLAGS`).
- Conflicts:
  - Omission: the document does not say that `flags` is `CV_PROCFLAGS`. cvinfo.h `LABELSYM32` gives `CV_PROCFLAGS flags`; bit 4 (`CV_PFLAG_NOTREACHED`, "label isn't fallen into") applies to labels.
- Undocumented assumptions: None. pdbrs does not enforce that labels nest inside procedures.

### `microsoft-pdb-rs/docs/codeview/symbols/s_local.md`
- Documents: `S_REGREL32` (0x1111), `S_LOCAL` (0x113e) and the def-range records `S_DEFRANGE` (0x113f), `S_DEFRANGE_SUBFIELD` (0x1140), `S_DEFRANGE_REGISTER` (0x1141), `S_DEFRANGE_FRAMEPOINTER_REL` (0x1142), `S_DEFRANGE_SUBFIELD_REGISTER` (0x1143), `S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE` (0x1144), `S_DEFRANGE_REGISTER_REL` (0x1145).
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/symbol/variable.rs` decodes `S_REGREL32` and `S_REGREL32_ENCTMP` (`RegisterRelative`), `S_LOCAL` (`Local`, `LocalFlags` bits 0-10), `S_DEFRANGE_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL`, `S_DEFRANGE_SUBFIELD_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE` (i32), `S_DEFRANGE_REGISTER_REL` and `S_DEFRANGE_REGISTER_REL_INDIR` (0x1177). `codeview/symbol/defrange.rs` decodes `S_DEFRANGE` and `S_DEFRANGE_SUBFIELD` (`DefRangeProgram`: u32 program, u32 parent offset for the subfield kind, range, gaps) (#30). Each range is `AddressRange` (u32 offset, u16 section, u16 length) followed by 4-byte `AddressGap` entries. The validation checks def-range placement, ranges and gaps (`validate/module.rs`, #30).
- Conflicts:
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: `struct Local` gives `uint32_t flags`. cvinfo.h `CV_LVARFLAGS`, LLVM `LocalSymFlags : uint16_t`, ms-codeview `LocalFixed.flags: U16` and pdbrs `LocalFlags(u16)` use 16 bits. With 32 bits, a decoder reads 2 name bytes as flags.
  - Document vs cvinfo.h/pdbrs: `struct DefRangeRegister` is `register, attr, gaps[]`. cvinfo.h `DEFRANGESYMREGISTER` has `CV_LVAR_ADDR_RANGE range` (8 bytes) between `attr` and `gaps`. pdbrs reads the range.
  - Document vs cvinfo.h/LLVM/pdbrs: the full-scope record gives `uint32 offset_to_frame_pointer`. cvinfo.h gives `CV_off32_t` (signed). pdbrs reads `i32`.
  - Internal: the document says that `S_REGREL32` is not paired with `S_LOCAL`, but the "Paired symbols" example pairs `S_LOCAL: x` with `S_REGREL32: rcx`.
  - Internal: "The list is not exhaustive and" is a cut sentence.
  - TODO sections: `S_DEFRANGE_FRAMEPOINTER_REL`, `S_DEFRANGE_SUBFIELD_REGISTER` and `S_DEFRANGE_REGISTER_REL` have no layout. cvinfo.h gives them (`DEFRANGESYMFRAMEPOINTERREL`: i32 offset, range, gaps; `DEFRANGESYMSUBFIELDREGISTER`: reg, attr, u32 with offParent in bits 0-11 and 20 bits padding, range, gaps; `DEFRANGESYMREGISTERREL`: baseReg, u16 with spilledUdtMember bit 0, padding bits 1-3 and offsetParent bits 4-15, i32 offset, range, gaps). pdbrs implements all three with these layouts.
  - Omission: `S_DEFRANGE` and `S_DEFRANGE_SUBFIELD` have no layout in the document. cvinfo.h gives `DEFRANGESYM` (u32 program, range, gaps) and `DEFRANGESYMSUBFIELD` (u32 program, u32 offParent, range, gaps). pdbrs uses these layouts (#30).
  - Omission: the document does not describe `CV_RANGEATTR` bit 0 (`maybe`) or `CV_LVARFLAGS` bits.
  - Omission: local-variable kinds that pdbrs decodes are absent: `S_BPREL32` 0x110b, `S_REGISTER` 0x1106, `S_FILESTATIC` 0x1153, `S_MANSLOT` 0x1120, `S_DEFRANGE_HLSL` 0x1150, `S_LOCAL_DPC_GROUPSHARED` 0x1154, `S_BPREL32_INDIR` 0x1170, `S_REGREL32_INDIR` 0x1171, `S_STATICLOCAL` 0x1176, `S_DEFRANGE_REGISTER_REL_INDIR` 0x1177, `S_BPREL32_ENCTMP` 0x1178, `S_REGREL32_ENCTMP` 0x1179, `S_BPREL32_INDIR_ENCTMP` 0x117a, `S_REGREL32_INDIR_ENCTMP` 0x117b, `S_DEFRANGE_CONSTVAL_ON_ENTRY` 0x117f, `S_DEFRANGE_GLOBALSYM_ON_ENTRY` 0x1180, and the attributed and managed register kinds (#30).
  - The document says `S_REGREL32` occurs only in `S_LPROC32`/`S_GPROC32` scopes. LLVM and pdbrs also have `S_LPROC32_ID`/`S_GPROC32_ID` procedures. pdbrs does not enforce `S_REGREL32` placement or `S_LOCAL` pairing. The validation reports a def-range that does not follow an `S_LOCAL` or `S_FILESTATIC` (`Rule::DefRangeWithoutLocal`) and a top-level `S_LOCAL` (`Rule::LocalOutsideScope`) (#30).
- Undocumented assumptions:
  - The gap array must be a multiple of 4 bytes. pdbrs returns all gaps. LLVM `CodeViewRecordIO::mapVectorTail` stops at a gap whose first byte is >= 0xF0 (it treats it as `LF_PAD`), so LLVM hides gaps in some records; the fixture bytes confirm the hidden gaps (#5).
  - `S_REGREL32` offset is u32 (LLVM `RegRelativeSym`); `S_DEFRANGE_REGISTER_REL_INDIR` `offset_in_udt` is i32 (LLVM `little32_t`) (#5).
  - `S_REGREL32_ENCTMP` (0x1179) uses the `S_REGREL32` layout (fixture-confirmed, #13). The other `_ENCTMP` and `_INDIR` kinds and the `_ON_ENTRY` def-ranges use layouts from the DIA SDK `diaguids.lib` debug type records (#30).
  - `fxc` writes `S_LOCAL` records of a fixed length: 4 or more zero bytes follow the NUL of the name. pdbrs accepts them and keeps the count (`Symbol::padding`) (#27, #30).
  - The validation rules for locals: gaps must be nonempty, inside the range and in order (`Rule::GapEmpty`, `GapOutsideRange`, `GapOrder`); a def-range must be inside its procedure (`Rule::DefRangeOutsideProcedure`) but not inside its block, because LLVM writes def-ranges that start before the block (#12); a def-range in the range of an `S_SEPCODE` whose parent address is the procedure start is valid, also when MSVC writes the `S_SEPCODE` after the procedure end (MSVC PGO fixture, #27); a def-range in an inline site must overlap or touch the code of that site or an enclosing site (`Rule::DefRangeOutsideInlineSite`, see `s_inlinesite.md`); a parameter must be directly in a procedure, an inline site or an `S_SEPCODE` (`Rule::ParameterScope`); flag bits 11-15 must be zero (`Rule::LocalFlagsReserved`); an optimized-out local must not have a def-range (`Rule::OptimizedOutWithRange`) (#30).
  - The parent offset of `S_DEFRANGE_SUBFIELD`, `S_DEFRANGE_SUBFIELD_REGISTER` and of `S_DEFRANGE_REGISTER_REL` with the spilled-member bit must be less than the size of the type of the local (`S_LOCAL`, `S_FILESTATIC` or the HLSL group-shared local) (`Rule::SubfieldOutsideType`). `ValidationContext::with_type_sizes` gives the size function; `types::type_size` computes it. A type without a known size is not checked. 243 subfield def-ranges of the fixtures have a known size: 0 findings (#30).
  - Register fields are raw `CV_HREG_e` values for the CPU in the compile record. The validation checks them against the ranges of DIA `cvconst.h`, which has more registers than LLVM `CodeViewRegisters.def` (for example `CV_REG_XMM6L` = 200, which MSVC writes for x64) (`validate/register.rs`, #30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_namespace.md`
- Documents: The `S_UNAMESPACE` (0x1124) record: one NUL-terminated namespace name.
- pdbrs: implemented — `decode_data` in `crates/pdbrs/src/native/codeview/symbol.rs` reads the name (`SymbolData::UsingNamespace`); `Symbol::name` returns it.
- Conflicts: None found. The layout agrees with cvinfo.h `UNAMESPACE`.
- Undocumented assumptions: None. pdbrs does not enforce the "procedures and blocks only" rule.

### `microsoft-pdb-rs/docs/codeview/symbols/s_objname.md`
- Documents: The `S_OBJNAME` (0x1101) record (u32 signature, object file path).
- pdbrs: implemented — `ObjectName::read` in `crates/pdbrs/src/native/codeview/symbol/compile.rs` reads the layout.
- Conflicts:
  - Document vs pdbrs: the document says the signature "should be at least a CRC32". The pdbrs field doc says that compilers usually write 0. pdbrs does not check the value.
- Undocumented assumptions: `S_ALTOBJNAME` (0x1181) has no layout in any reference; pdbrs keeps it raw (`Unsupported`) (#30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_procs.md`
- Documents: Procedure scopes, the procedure start kinds, the `S_LPROC32` (0x110f)/`S_GPROC32` (0x1110) layout and `CV_PROCFLAGS` bits, the `S_LMANPROC` (0x112b)/`S_GMANPROC` (0x112a) layout, and the nested kinds.
- pdbrs: partial — `Procedure::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` decodes `S_GPROC32`, `S_LPROC32`, `S_LPROC32_DPC` (0x1155) (TPI type) and `S_GPROC32_ID`, `S_LPROC32_ID`, `S_LPROC32_DPC_ID` (0x1156) (IPI ID, `ProcedureType`), and the `EX` kinds 0x1172-0x1175 with `extended_flags` (#30). `ProcedureFlags` names bits 0-7. `codeview/symbol/architecture.rs` decodes the MIPS and IA64 procedures. `ManagedProcedure::read` in `codeview/symbol/managed.rs` decodes the managed kinds with `return_register`. `scope.rs` opens scopes for all of them. `native::locate::SymbolLocator` finds the procedure for an address, also for an address in separated code through its `S_SEPCODE` record; `SymbolLocator::code_chunks` returns the chunks of a procedure (#30). Not implemented: `S_LPROC32_ST` (0x100a) and the other pre-VC70 `_ST` kinds; they give `Unsupported`.
- Conflicts:
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: bit 0 is `fpo` "true if function has frame pointer omitted". cvinfo.h `CV_PFLAG_NOFPO` says "frame pointer present"; LLVM `HasFP`, ms-codeview `NOFPO` ("Frame pointer present") and pdbrs `HAS_FRAME_POINTER` agree. The document inverts the meaning.
  - Document vs cvinfo.h/LLVM/pdbrs: bit 2 is "unused, must be zero". cvinfo.h gives `CV_PFLAG_FAR` (far return); pdbrs `HAS_FAR_RETURN`.
  - Document vs cvinfo.h/LLVM/pdbrs: bit 4 is "unused, must be zero". cvinfo.h gives `CV_PFLAG_NOTREACHED`; LLVM `IsUnreachable`; pdbrs `UNREACHABLE`.
  - Document vs pdbrs/LLVM lld: the document says `p_parent`, `p_end`, `p_next` are offsets "relative to the `S_LPROC32` record". pdbrs treats them as module-stream offsets that include the 4-byte signature (`ScopeLinks`), and checks `end` == offset of the end record. lld writes the absolute module-stream offset. The fixtures pass this check (#5). `ModuleSymbolWriter` writes absolute offsets too (#30).
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: the managed `Procedure` struct omits `uint16_t retReg` after `flags`. cvinfo.h `MANPROCSYM` has it; pdbrs reads `return_register` (fixture-confirmed, #14).
  - ms-codeview vs ms-codeview/cvinfo.h/pdbrs: ms-codeview has two managed layouts. `ManagedProcFixed` has no padding (agrees with packed cvinfo.h and pdbrs). `ManProcSymFixed` (line 938) inserts `padding: u8` before `ret_reg`, which shifts `ret_reg` and the name by 1 byte.
  - Document: the text says the managed record "is similar to `S_GDATA32`". The layout is the `S_GPROC32` layout with a metadata token in place of the type index.
  - Omission: the procedure start lists omit `S_GPROC32_ID` (0x1147) and `S_LPROC32_ID` (0x1146), which pdbrs and LLVM decode, and the `EX`, MIPS and IA64 procedure kinds, which pdbrs decodes (#30).
  - Internal: the bullet list is broken ("visible only within a single * `S_LPROC32_DPC`: …"). Typos: `s_parent` for `p_parent`, "i.e.g", `G_PROC32`.
  - The document says encoders set `p_next` to 0 and decoders ignore it. pdbrs keeps `next` raw. `ModuleSymbolWriter` keeps the given value. The validation accepts 0 or the offset of a later procedure or thunk (`Rule::NextLink`) (#30).
- Undocumented assumptions:
  - The top-level parent link must be 0; a nonzero value gives `InvalidReference` (`scope.rs`).
  - `S_END` ends managed procedure scopes. LLVM `symbolOpensScope` does not list `S_GMANPROC`/`S_LMANPROC`; pdbrs adds them because `MANPROCSYM` has links (#14). LLVM also does not list the MIPS, IA64 and `EX` procedures; their cvinfo.h structures have links, and `S_END` ends them (`scope.rs`, #30).
  - The `EX` kinds use the `PROCSYM32EX` layout of the DIA SDK `diaguids.lib`: the `PROCSYM32` layout with a 2-byte `CV_EXPROCFLAGS` in place of the 1-byte `CV_PROCFLAGS`. `ExtendedProcedureFlags` names the second byte from the DIA bit fields; no reference describes the bits (#30).
  - The `_DPC` kinds use the `S_LPROC32` and `S_LPROC32_ID` layouts (`symbol.rs`). MSVC 14.44 with `amp.h` writes them (`msvc-x64-amp`, and `S_LPROC32_DPC_ID` with `/DEBUG:LAZY` in `msvc-x64-amp-lazy`); `llvm-pdbutil` decodes `S_LPROC32_DPC` as pdbrs does (#39).
  - The managed `token` is usually a `MethodDef` token (table 0x06), and the name can be empty (#14).
  - MSVC funclets in `msvc-x64-lazy` are `S_LPROC32_ID` records with ID 0. The validation accepts ID 0 there (#30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_pub.md`
- Documents: The `S_PUB32` (0x110e) record (u32 flags, offset, segment, name), which appears only in the global symbol stream.
- pdbrs: implemented — `Public::read` in `crates/pdbrs/src/native/codeview/symbol/global.rs` reads the layout. `PublicFlags` names bits 0-3. `symbols/psi.rs` checks the PSI address map order against the `S_PUB32` addresses. `PublicSymbolIndex::build` (`symbols/psi/write.rs`) writes the PSI again, and `native::locate::SymbolLocator` gives the public symbol for an address (#30).
- Conflicts:
  - Omission: the document does not describe `flags`. cvinfo.h `CV_PUBSYMFLAGS_e` gives `cvpsfCode` 0x1, `cvpsfFunction` 0x2, `cvpsfManaged` 0x4, `cvpsfMSIL` 0x8. LLVM `PublicSymFlags` and pdbrs `PublicFlags` agree.
- Undocumented assumptions:
  - The PSI address map is sorted by section, then offset, then name (`cmpAddrMapByAddrAndName` in `PDB/dbi/gsi.cpp`, LLVM `computeAddrMap`). `IndexProducer::Microsoft` compares section and offset as signed values; `IndexProducer::Llvm` compares them as unsigned values. Records with the same address and name keep the order of their symbol offsets (#30).
  - In a hash bucket, a shorter name comes first; names of the same length compare without case if both are ASCII; equal names keep the order of their symbol offsets. The MSVC `LINK` 14.51 fixtures confirm the last rule (`symbols/table/write.rs`, #30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_refsyms.md`
- Documents: The `REFSYM2` layout (name checksum, symbol offset, 1-based module index, name) and the kinds `S_PROCREF` (0x1125), `S_DATAREF` (0x1126), `S_LPROCREF` (0x1127), `S_TOKENREF` (0x1129), `S_ANNOTATIONREF` (0x1128).
- pdbrs: implemented — `ProcedureReference::read` in `crates/pdbrs/src/native/codeview/symbol/global.rs` decodes `S_PROCREF`, `S_LPROCREF` and `S_TOKENREF` (`SymbolData::ProcedureReference`) and `S_DATAREF` and `S_ANNOTATIONREF` with the same layout (`SymbolData::SymbolReference`) (#30). `Symbol::reference` returns all five. `module` stays raw; `module_index()` returns `module - 1` and `None` for 0. `offset` includes the 4-byte module-stream signature. `validate_shared_symbols` (`validate.rs`) checks the module number (`Rule::ModuleIndex`), that the offset starts a record (`Rule::ModuleRecordMissing`) and the target kind (`Rule::ModuleRecordKind`) (#30).
- Conflicts:
  - Internal: the document says `name_checksum` "appears to be set to 0" and then says it "is a checksum computed over the name" with a TODO for the algorithm. cvinfo.h `REFSYM2` calls it `sumName` ("SUC of the name") and gives no algorithm. lld writes 0 (`ps.SumName = 0`). pdbrs keeps the value raw and does not check it.
  - TODO: `S_TOKENREF` targets are open in the document. cvinfo.h says "Reference to one of the many MANPROCSYM's". pdbrs treats it as a reference to `S_GMANPROC`/`S_LMANPROC`, and the validation accepts only these targets (#14, #30).
  - The document says `S_PROCREF` references `S_GPROC32`. lld also creates references from `S_GPROC32_ID`/`S_LPROC32_ID` after it converts them (`translateIdSymbols`, `symbolGoesInGlobalsStream`). (Inference, not tested: MSVC linker behavior for `_ID` kinds not checked.)
- Undocumented assumptions:
  - LLVM `ProcRefSym::modi()` and lld (`++ps.Module`, "MSVC seems to add one") confirm the 1-based module number. The validation reports module number 0 (#30).
  - `S_PROCREF` targets: the global procedure kinds of all architectures and `S_GMANPROC`. The .NET native PDB writer and the MSVC `/clr` linker write `S_PROCREF` for `S_GMANPROC` (fixtures of #12). `S_LPROCREF` targets: the module-local procedure kinds, the `_DPC` kinds and `S_LMANPROC` (`validate.rs`, #30).
  - `S_DATAREF` targets: `S_LDATA32`, `S_GDATA32`, `S_LTHREAD32`, `S_GTHREAD32`, `S_LMANDATA`, `S_GMANDATA`. `S_ANNOTATIONREF` targets: `S_ANNOTATION`. Fixture `msvc-x64-annotation` has 3 `S_ANNOTATIONREF` records (#30). No native fixture has `S_DATAREF`; it has synthetic tests only (external input, #30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_section.md`
- Documents: The `S_SECTION` (0x1136) record (u16 section number, u8 alignment exponent, u8 reserved, RVA, size, characteristics, name).
- pdbrs: implemented — `Section::read` in `crates/pdbrs/src/native/codeview/symbol/global.rs` reads the layout and rejects a nonzero reserved byte with `InvalidFormat`.
- Conflicts:
  - Internal: the "Usage" text says section symbols "are typically found in module symbol streams". The last line says they appear "only in the special `* Linker *` module". pdbrs does not enforce either rule.
- Undocumented assumptions: None. The layout agrees with cvinfo.h `SECTIONSYM`, LLVM `SectionSym` and ms-codeview `SectionFixed`.

### `microsoft-pdb-rs/docs/codeview/symbols/s_thread.md`
- Documents: The `S_LTHREAD32` (0x1112) and `S_GTHREAD32` (0x1113) records (type index, offset, segment, name).
- pdbrs: implemented — `Data::read` in `crates/pdbrs/src/native/codeview/symbol/variable.rs` reads the layout (`SymbolData::ThreadData`). The address is in the thread storage section.
- Conflicts:
  - Document vs LLVM lld: the document says both kinds can appear in module and global streams. lld `symbolGoesInModuleStream` puts `S_GTHREAD32` only in the globals stream; `S_LTHREAD32` goes to both. pdbrs does not enforce placement.
- Undocumented assumptions: `S_DATAREF` can identify both kinds (`validate.rs`, #30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_thunk.md`
- Documents: The `S_THUNK32` (0x1102) record (p_parent, p_end, p_next, offset, segment, u16 length, u8 ordinal, name, variant data).
- pdbrs: implemented — `Thunk::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the layout. `ThunkOrdinal` names 0-6. `scope.rs` opens a scope that `S_END` closes.
- Conflicts:
  - Omission: the document does not list the ordinal values. LLVM `ThunkOrdinal` gives `Standard` 0, `ThisAdjustor` 1, `Vcall` 2, `Pcode` 3, `UnknownLoad` 4, `TrampIncremental` 5, `BranchIsland` 6. pdbrs uses the same values. The pinned cvinfo.h does not define `THUNK_ORDINAL` (it is in `cvconst.h`).
  - Omission: the document does not give the variant data layout or length for any ordinal.
  - Document: it refers to `S_LPROC32` for the link meaning; see the offset conflict in `s_procs.md`.
- Undocumented assumptions:
  - A standard thunk (ordinal 0) has no variant data; pdbrs checks the zero padding after the name. For other ordinals, pdbrs keeps all bytes after the name raw, including the padding, because no reference gives the variant length. All 136 sample thunks are standard (#5). The encoder rejects a standard thunk with variant data (#30).
  - The walk does not check `p_next`. The validation accepts 0 or the offset of a later procedure or thunk (`Rule::NextLink`) (#30).

### `microsoft-pdb-rs/docs/codeview/symbols/s_trampoline.md`
- Documents: The `S_TRAMPOLINE` (0x112c) record (u16 kind, u16 thunk size, thunk offset, target offset, thunk section, target section) and kinds 0 (incremental) and 1 (branch island).
- pdbrs: implemented — `Trampoline::read` in `crates/pdbrs/src/native/codeview/symbol/link.rs` reads the layout. `TrampolineKind` names 0 and 1. `Symbol::address` returns the thunk address.
- Conflicts:
  - None found. The layout agrees with cvinfo.h `TRAMPOLINESYM`/`TRAMP_e` and LLVM `TrampolineSym`/`TrampolineType`.
  - Omission: the document does not describe `thunk_offset`, `target_offset`, `thunk_section` and `target_section`.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_udt.md`
- Documents: The `S_UDT` (0x1108) record (type index, name) and the expected target type kinds.
- pdbrs: implemented — `UserDefinedType::read` in `crates/pdbrs/src/native/codeview/symbol/variable.rs` reads the layout. `S_COBOLUDT` uses the same layout (#30). The validation checks that the type index is in the TPI stream or is a known primitive (`Rule::TypeIndexRange`, `Rule::PrimitiveType`, #30). pdbrs does not check the kind of the target type or that the names are equal.
- Conflicts:
  - TODO: "Can `S_UDT` point to primitives? What about typedefs?" stays open. cvinfo.h and LLVM do not restrict the target. pdbrs accepts any valid type index, primitives included.
  - Internal: the document says `S_UDT` is "usually found in the global symbol stream, but in rare cases … in a module symbol stream", then says it "can appear in the global symbol stream and in module symbol streams". lld puts function-local `S_UDT` in module streams (`symbolGoesInModuleStream`: scope depth > 0).
- Undocumented assumptions: None.
