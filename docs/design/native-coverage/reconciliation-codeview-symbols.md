# Native coverage: reconciliation of the Microsoft pdb-rs CodeView symbol documents

This page is part of the [native coverage](../native-coverage.md) accounting. It covers `docs/reference/microsoft-pdb-rs/docs/codeview/symbols/`.
Each section names one imported document. Paths are relative to `docs/reference/`. The fields are:

- **Documents**: the format items that the document describes.
- **pdbrs**: `implemented` (pdbrs reads every item that the document describes), `partial`, `not implemented` or `not applicable` (no format content), with the source files.
- **Conflicts**: each difference between the document and pdbrs, or between the document and another source (`cvinfo.h`, LLVM source, `ms-pdb`, other documents). Errors inside the document are conflicts too.
- **Undocumented assumptions**: layouts and rules that pdbrs uses but the document does not state, with the issue that records the evidence.

The comparison used `main` at `8b77fb2`, `cvinfo.h` at `805655a2`, LLVM at `b9f2235f` and the `ms-codeview` 0.1.7, `ms-pdb` 0.1.22, `ms-pdb-msf` 0.1.7 and `ms-pdb-msfz` 0.1.10 crates. "Inference, not tested" marks a statement that comes from reading source code only.

The pinned cvinfo.h does not define `THUNK_ORDINAL`, `CV_CFL_LANG` or `CV_CPU_TYPE_e`. Those enums are in DIA `cvconst.h`. For them I compared against LLVM only.

### `microsoft-pdb-rs/docs/codeview/symbols/symbols.md`
- Documents: The symbol record framing (u16 size, u16 kind, payload, 4-byte alignment, padding), global vs. module symbol streams, a summary table of symbol kinds, and the scope-opening kinds with the `S_END` stack rule.
- pdbrs: partial — `codeview.rs` `Records` frames records with the same length rule (length counts the kind, minimum 2, `MIN_RECORD_LEN`). `codeview/symbol.rs` `decode_data` decodes 57 kinds and gives `SymbolData::Unsupported` (raw payload) for others. `codeview/symbol/scope.rs` `ModuleSymbols` keeps a scope stack and checks parent links, end links, end-record kind and a depth limit. The summary-table kinds that pdbrs does not decode: `S_ANNOTATION` (0x1019), `S_DATAREF` (0x1126), `S_ANNOTATIONREF` (0x1128), `S_DEFRANGE` (0x113f), `S_DEFRANGE_SUBFIELD` (0x1140), `S_INLINESITE2` (real value 0x115d). The nested-scope table kinds that pdbrs does not decode: `S_LPROC32_DPC_ID` (0x1156) and `S_SEPCODE` (0x1132). `ModuleSymbols` rejects these scope openers with `UnsupportedVersion` (`UNDECODED_SCOPE_KINDS` = 0x1132, 0x1155, 0x1156, 0x115d). pdbrs also does not decode the table values 0x1007/0x1008 that the table gives for `S_GDATA32`/`S_LDATA32`, because those values are `S_LDATA32_ST`/`S_GDATA32_ST` (see below).
- Conflicts:
  - Padding bytes: the document says the padding uses 0xF1, 0xF2, 0xF3. pdbrs requires zero padding: each decoded symbol must end with fewer than 4 bytes, all zero (`Fields::finish_zero_padding` in `codeview.rs`, called from `Symbol::decode`). LLVM lld `fixRecordAlignment` (`lld/COFF/PDB.cpp`) writes zero bytes. The document's own `S_ANNOTATION` example (`s_annotation.md`) shows `00 00` padding. ms-codeview `syms/builder.rs` lines 67-69 writes F1/F2/F3, and its `test_parse_proc` uses `0xf1, 0xf2`. pdbrs rejects such records with `InvalidFormat`.
  - Kind values: the table gives `S_GDATA32` = 0x1007 and `S_LDATA32` = 0x1008. cvinfo.h (lines 2866-2867), LLVM `CodeViewSymbols.def` and pdbrs (`symbol.rs` `S_LDATA32` = 0x110c, `S_GDATA32` = 0x110d) give 0x110d and 0x110c. In cvinfo.h, 0x1007 is `S_LDATA32_ST` and 0x1008 is `S_GDATA32_ST`.
  - Kind value: the table gives `S_INLINESITE2` = 0x115C. cvinfo.h line 2979, LLVM, ms-codeview `kind.rs` line 161 and pdbrs `scope.rs` give 0x115d. 0x115c is `S_POGODATA`.
  - Scope ends: the document says that `S_END` ends all scopes. cvinfo.h and LLVM `symbolEndsScope` use `S_INLINESITE_END` (0x114e) for `S_INLINESITE` and allow `S_PROC_ID_END` (0x114f). pdbrs `scope.rs` closes inline sites only with `S_INLINESITE_END` and other scopes with `S_END` or `S_PROC_ID_END`. A wrong end kind gives `InvalidFormat`.
  - Alignment: the document requires each record start and size to be a multiple of 4. pdbrs `Records` does not check alignment. Only decoded kinds must have fewer than 4 trailing zero bytes. Undecoded kinds and `S_SKIP` keep their payload without checks.
  - Nested-scope table: it lists `S_GPROC32_ID` but not `S_LPROC32_ID` (0x1146). LLVM `symbolOpensScope` and pdbrs open a scope for both. It omits `S_LPROC32_DPC` (0x1155), which LLVM and pdbrs `UNDECODED_SCOPE_KINDS` list.
  - Internal: the text says "Each PDB contains one Global Symbol Stream" and then "Some PDBs do not contain a GSS". pdbrs treats the DBI symbol record stream as optional (`examples/native/symbols.rs` returns early if `symbol_stream` is absent).
  - Internal: the summary table omits `S_SEPCODE`, but the nested-scope table lists it. The table also lists 0x1137 before 0x1136.
  - Omissions: the table omits kinds that pdbrs decodes: `S_SKIP` 0x0007, `S_OEM` 0x0404, `S_REGISTER` 0x1106, `S_BPREL32` 0x110b, `S_MANSLOT` 0x1120, `S_FRAMECOOKIE` 0x113a, `S_ENVBLOCK` 0x113d, `S_LPROC32_ID` 0x1146, `S_GPROC32_ID` 0x1147, `S_INLINESITE_END` 0x114e, `S_PROC_ID_END` 0x114f, `S_FILESTATIC` 0x1153, `S_CALLEES` 0x115a, `S_HEAPALLOCSITE` 0x115e, `S_FRAMEREG` 0x1166, `S_FASTLINK` 0x1167, `S_INLINEES` 0x1168, `S_DEFRANGE_REGISTER_REL_INDIR` 0x1177, `S_REGREL32_ENCTMP` 0x1179.
- Undocumented assumptions:
  - Scope links are module-stream offsets that include the 4-byte stream signature: record offset + base 4 (`scope.rs`, `ScopeLinks` in `symbol.rs`). Only C13 module streams (signature 4) are supported (#5).
  - The parent link of a scope opener must equal the offset of the enclosing scope, or 0 at top level. The end link must be after the start and must be the offset of the matching end record. The walk does not check `p_next` (#5).
  - The scope walk has a caller-set depth limit (`max_depth`) and returns `LimitExceeded` above it.
  - `S_SKIP` (0x0007) is a filler; LLD writes a zero-filled `S_SKIP` for a record that it cannot remap (`symbol.rs`; lld `replaceWithSkipRecord`).
  - Names are NUL-terminated byte strings. pdbrs does not decode them as text and does not demangle them (`symbol.rs` module doc).
  - Fixture-confirmed layouts for kinds without a reference layout: `S_FRAMEREG` (u16 register + 2 zero bytes), `S_FASTLINK` (`REFMINIPDB` without module index; flag bit 6 = namespace), `S_REGREL32_ENCTMP` (= `S_REGREL32` layout) (#13).
  - Managed kinds use packed cvinfo.h layouts (#14).

### `microsoft-pdb-rs/docs/codeview/symbols/s_annotation.md`
- Documents: The `S_ANNOTATION` (0x1019) record: u32 offset, u16 section, u16 string count, then `count` NUL-terminated strings, with an MSVC `__annotation` example.
- pdbrs: not implemented — `decode_data` in `crates/pdbrs/src/native/codeview/symbol.rs` has no arm for 0x1019, so the record gives `SymbolData::Unsupported` with the raw payload and padding. `ModuleSymbols` passes it through. The full-read example reports it as undecoded. pdbrs also does not decode `S_ANNOTATIONREF` (0x1128), which points to this record.
- Conflicts:
  - None found for the layout: cvinfo.h `ANNOTATIONSYM` (off, seg, csz, rgsz), LLVM `AnnotationSym` and ms-codeview `AnnotationFixed` agree with the document.
  - pdbrs vs LLVM/ms-codeview: LLVM (`SYMBOL_RECORD(S_ANNOTATION, 0x1019, AnnotationSym)`) and ms-codeview (`Annotation`) decode the record; pdbrs does not.
  - Omission: the example source uses wide strings (`L"Hello!"`), but the record bytes hold single-byte strings. The document does not state the string encoding.
  - The example dump shows zero padding (`00 00` after `World!\0`). This contradicts the 0xF1/0xF2 padding rule in `symbols.md`.
- Undocumented assumptions: None.

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
- pdbrs: implemented — `Block::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the same layout. `scope.rs` opens a scope and checks the parent and end links.
- Conflicts: None found. The layout agrees with cvinfo.h `BLOCKSYM32` and LLVM `BlockSym`.
- Undocumented assumptions:
  - The link values are module-stream offsets that include the 4-byte signature (#5).
  - pdbrs does not enforce that `S_BLOCK32` nests inside a procedure. A top-level block with parent 0 passes the scope walk.

### `microsoft-pdb-rs/docs/codeview/symbols/s_buildinfo.md`
- Documents: The `S_BUILDINFO` (0x114c) record: one `ItemId` that refers to an `LF_BUILDINFO` record in the IPI stream.
- pdbrs: implemented — `decode_data` in `crates/pdbrs/src/native/codeview/symbol.rs` reads an `IdIndex` (`SymbolData::BuildInfo`). `Symbol::id_index` returns it. The symbol decoder does not resolve the IPI record.
- Conflicts: None found. The layout agrees with cvinfo.h `BUILDINFOSYM`.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_callsiteinfo.md`
- Documents: The `S_CALLSITEINFO` (0x1139) record (offset, section, reserved u16, type index) and the type shapes for indirect calls.
- pdbrs: implemented — `CallSiteInfo::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the layout and rejects a nonzero reserved field with `InvalidFormat`. pdbrs does not check the shape of the referenced type or the nesting rule.
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
  - `ml64` writes an extra NUL after the `S_COMPILE3` version string, so 4 or more zero bytes follow it. pdbrs accepts any number of zero bytes there and rejects nonzero bytes. cvdump and LLVM ignore the bytes (#5).
  - An absent `S_COMPILE2` string block and an empty block give the same value (LLVM `mapStringZVectorZ`).

### `microsoft-pdb-rs/docs/codeview/symbols/s_constant.md`
- Documents: The `S_CONSTANT` (0x1107) record (type index, numeric value, name) and the `S_MANCONSTANT` (0x112d) record (metadata token, numeric value, name).
- pdbrs: partial — `Constant::read` in `crates/pdbrs/src/native/codeview/symbol/variable.rs` accepts only integer numeric leaves (immediate, `LF_CHAR` … `LF_UOCTWORD`). A real, complex, string or other leaf makes `Symbol::decode` fail with `UnsupportedVersion`. `ManagedConstant::read` in `codeview/symbol/managed.rs` accepts integers, `LF_VARSTRING` (0x8010) and fixed-size leaves (REAL*, COMPLEX*, `LF_DECIMAL`, `LF_DATE`) as raw bytes. Other leaves, for example `LF_UTF8STRING` (0x801b), give `UnsupportedVersion` (#14).
- Conflicts:
  - Document vs pdbrs: the document allows any `Number`. pdbrs rejects non-integer `S_CONSTANT` values.
  - Document vs cvinfo.h/pdbrs: the document names the `S_MANCONSTANT` field `metadata_token`. cvinfo.h `CONSTSYM` gives "Type index … or metadata token". pdbrs keeps it as `type_or_token`; Roslyn writes a `StandAloneSig` token (table 0x11) (#14).
- Undocumented assumptions: The numeric leaf has no alignment inside the record; the name follows it directly (`numeric.rs`).

### `microsoft-pdb-rs/docs/codeview/symbols/s_data.md`
- Documents: The `S_GDATA32` and `S_LDATA32` records (type index, offset, segment, name) and where they appear.
- pdbrs: implemented — `Data::read` in `crates/pdbrs/src/native/codeview/symbol/variable.rs` reads the layout for `S_GDATA32` (0x110d) and `S_LDATA32` (0x110c) (`SymbolData::Data`).
- Conflicts:
  - Document vs cvinfo.h/LLVM/pdbrs: the title gives `S_GDATA32` = 0x1007 and `S_LDATA32` = 0x1008. The real values are 0x110d and 0x110c. In cvinfo.h, 0x1007 is `S_LDATA32_ST` and 0x1008 is `S_GDATA32_ST` (the order is also reversed). pdbrs does not decode 0x1007 or 0x1008.
  - Internal: typo "rercords".
- Undocumented assumptions: None. pdbrs does not enforce the global/module location rules. lld `symbolGoesInModuleStream` agrees with the document (S_GDATA32 not in module streams).

### `microsoft-pdb-rs/docs/codeview/symbols/s_end.md`
- Documents: The `S_END` (0x0006) record, which has no payload and ends a scope that `S_LPROC32`, `S_GPROC32`, `S_THUNK32`, `S_INLINESITE` and others open.
- pdbrs: implemented — `decode_data` in `crates/pdbrs/src/native/codeview/symbol.rs` gives `SymbolData::ScopeEnd` for `S_END` and `S_PROC_ID_END` (0x114f). `scope.rs` closes the innermost non-inline scope and checks that the end link of the opener names this record.
- Conflicts:
  - Document vs cvinfo.h/LLVM/pdbrs: the document says that `S_END` ends `S_INLINESITE`. `S_INLINESITE_END` (0x114e) ends it. pdbrs rejects `S_END` for an open inline site with `InvalidFormat` ("symbol scope end kind does not match its scope").
  - Omission: the document does not name `S_PROC_ID_END` (0x114f), which can end an `_ID` procedure.
- Undocumented assumptions: An `S_END` without an open scope gives `InvalidFormat`. A scope without an end record at the end of the stream gives `InvalidFormat` (#5).

### `microsoft-pdb-rs/docs/codeview/symbols/s_frameproc.md`
- Documents: The `S_FRAMEPROC` (0x1012) record (frame size, pad size, pad offset, saved-register size, exception handler offset and section, flags) and flag bits 0-8.
- pdbrs: implemented — `FrameProcedure::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the same packed layout. `FrameProcedureFlags` names bits 0-13 and 18-22. `local_frame_pointer()` reads bits 14-15 and `parameter_frame_pointer()` reads bits 16-17 (`FramePointer` 0-3).
- Conflicts:
  - Document vs cvinfo.h/LLVM/pdbrs: the document says bits 9-31 are padding and must be zero. cvinfo.h `FRAMEPROCSYM` and LLVM `FrameProcedureOptions` define `fAsyncEH` 9, `fGSNoStackOrdering` 10, `fWasInlined` 11, `fGSCheck` 12, `fSafeBuffers` 13, `encodedLocalBasePointer` 14-15, `encodedParamBasePointer` 16-17, `fPogoOn` 18, `fValidCounts` 19, `fOptSpeed` 20, `fGuardCF` 21, `fGuardCFW` 22, and `pad : 9` (bits 23-31). pdbrs follows cvinfo.h/LLVM.
  - ms-codeview vs document/cvinfo.h/LLVM/pdbrs: ms-codeview `FrameProc` (syms.rs line 606) has an extra `padding: U16` between `exception_handler_section` and `flags`. cvinfo.h is packed (`#pragma pack(1)`), LLVM maps `Flags` directly after `SectionIdOfExceptionHandler`, and pdbrs reads flags at offset 22 of the payload. With the ms-codeview layout, the flags value is read 2 bytes too late.
  - The rules "at most one `S_FRAMEPROC` per procedure" and "directly nested in the procedure" are not enforced by pdbrs.
- Undocumented assumptions:
  - The x64 meaning of the encoded frame pointer values (1 = RSP, 2 = RBP, 3 = R13) comes from LLVM `decodeFramePtrReg` (`procedure.rs`).
  - MSVC ARM64 writes `S_FRAMEREG` (0x1166: u16 register + 2 zero bytes) after `S_FRAMEPROC` in procedures with exception handling and in their funclets (fixture-confirmed, #13).

### `microsoft-pdb-rs/docs/codeview/symbols/s_inlinesite.md`
- Documents: The `S_INLINESITE` (0x114d) record (p_parent, p_end, inlinee) and the `S_INLINESITE2` record (p_parent, p_end, inlinee, binary annotations).
- pdbrs: partial — `InlineSite::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads parent, end, inlinee (`IdIndex`) and the binary annotations. `BinaryAnnotations` in `codeview/symbol/annotation.rs` validates and decodes opcodes 1-13, compressed unsigned integers (1, 2 or 4 bytes; first byte 0xE0-0xFF is invalid), signed operands (bit 0 = sign, `DecodeSignedInt32`), opcode 11 (code delta in bits 0-3, signed line delta above) and opcode 12 (length, then delta). Opcode 0 is accepted only as trailing padding: fewer than 4 bytes, all zero. `S_INLINESITE_END` (0x114e) closes the scope. `S_INLINESITE2` (0x115d) is not implemented: it gives `Unsupported`, and `ModuleSymbols` rejects it as an undecoded scope opener (`UnsupportedVersion`).
- Conflicts:
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: the title gives `S_INLINESITE2` = 0x115c. The value is 0x115d. 0x115c is `S_POGODATA`.
  - Document vs cvinfo.h/LLVM/pdbrs: the `InlineSite` struct has no binary annotations. cvinfo.h `INLINESITESYM` and LLVM `InlineSiteSym` end with `binaryAnnotations`. pdbrs decodes them.
  - Document vs cvinfo.h/ms-codeview: the `InlineSite2` struct omits `uint32_t invocations` after `inlinee`. cvinfo.h `INLINESITESYM2` and ms-codeview `InlineSite2Fixed` have it.
  - Omission: the document does not state that `S_INLINESITE_END` (0x114e), not `S_END`, ends the scope.
  - Omission: the document does not describe the binary annotation opcodes or the compressed integer encoding (cvinfo.h `BinaryAnnotationOpcode`, `CVUncompressData`).
- Undocumented assumptions:
  - The `p_end` link must be the module-stream offset (with the 4-byte signature) of the `S_INLINESITE_END` record (#5).
  - pdbrs does not enforce that an inline site nests inside a procedure.
  - `ChangeFile` (opcode 5) gives an offset in the module file-checksum subsection (`annotation.rs`).

### `microsoft-pdb-rs/docs/codeview/symbols/s_label.md`
- Documents: The `S_LABEL32` (0x1105) record (offset, segment, u8 flags, name).
- pdbrs: implemented — `Label::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the layout. The flags use `ProcedureFlags` (`CV_PROCFLAGS`).
- Conflicts:
  - Omission: the document does not say that `flags` is `CV_PROCFLAGS`. cvinfo.h `LABELSYM32` gives `CV_PROCFLAGS flags`; bit 4 (`CV_PFLAG_NOTREACHED`, "label isn't fallen into") applies to labels.
- Undocumented assumptions: None. pdbrs does not enforce that labels nest inside procedures.

### `microsoft-pdb-rs/docs/codeview/symbols/s_local.md`
- Documents: `S_REGREL32` (0x1111), `S_LOCAL` (0x113e) and the def-range records `S_DEFRANGE` (0x113f), `S_DEFRANGE_SUBFIELD` (0x1140), `S_DEFRANGE_REGISTER` (0x1141), `S_DEFRANGE_FRAMEPOINTER_REL` (0x1142), `S_DEFRANGE_SUBFIELD_REGISTER` (0x1143), `S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE` (0x1144), `S_DEFRANGE_REGISTER_REL` (0x1145).
- pdbrs: partial — `crates/pdbrs/src/native/codeview/symbol/variable.rs` decodes `S_REGREL32` and `S_REGREL32_ENCTMP` (`RegisterRelative`), `S_LOCAL` (`Local`, `LocalFlags` bits 0-10), `S_DEFRANGE_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL`, `S_DEFRANGE_SUBFIELD_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE` (i32), `S_DEFRANGE_REGISTER_REL` and `S_DEFRANGE_REGISTER_REL_INDIR` (0x1177). Each range is `AddressRange` (u32 offset, u16 section, u16 length) followed by 4-byte `AddressGap` entries. Not implemented: `S_DEFRANGE` (0x113f) and `S_DEFRANGE_SUBFIELD` (0x1140); they give `Unsupported`.
- Conflicts:
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: `struct Local` gives `uint32_t flags`. cvinfo.h `CV_LVARFLAGS`, LLVM `LocalSymFlags : uint16_t`, ms-codeview `LocalFixed.flags: U16` and pdbrs `LocalFlags(u16)` use 16 bits. With 32 bits, a decoder reads 2 name bytes as flags.
  - Document vs cvinfo.h/pdbrs: `struct DefRangeRegister` is `register, attr, gaps[]`. cvinfo.h `DEFRANGESYMREGISTER` has `CV_LVAR_ADDR_RANGE range` (8 bytes) between `attr` and `gaps`. pdbrs reads the range.
  - Document vs cvinfo.h/LLVM/pdbrs: the full-scope record gives `uint32 offset_to_frame_pointer`. cvinfo.h gives `CV_off32_t` (signed). pdbrs reads `i32`.
  - Internal: the document says that `S_REGREL32` is not paired with `S_LOCAL`, but the "Paired symbols" example pairs `S_LOCAL: x` with `S_REGREL32: rcx`.
  - Internal: "The list is not exhaustive and" is a cut sentence.
  - TODO sections: `S_DEFRANGE_FRAMEPOINTER_REL`, `S_DEFRANGE_SUBFIELD_REGISTER` and `S_DEFRANGE_REGISTER_REL` have no layout. cvinfo.h gives them (`DEFRANGESYMFRAMEPOINTERREL`: i32 offset, range, gaps; `DEFRANGESYMSUBFIELDREGISTER`: reg, attr, u32 with offParent in bits 0-11 and 20 bits padding, range, gaps; `DEFRANGESYMREGISTERREL`: baseReg, u16 with spilledUdtMember bit 0, padding bits 1-3 and offsetParent bits 4-15, i32 offset, range, gaps). pdbrs implements all three with these layouts.
  - Omission: `S_DEFRANGE` and `S_DEFRANGE_SUBFIELD` have no layout in the document. cvinfo.h gives `DEFRANGESYM` (u32 program, range, gaps) and `DEFRANGESYMSUBFIELD` (u32 program, u32 offParent, range, gaps).
  - Omission: the document does not describe `CV_RANGEATTR` bit 0 (`maybe`) or `CV_LVARFLAGS` bits.
  - Omission: local-variable kinds that pdbrs decodes are absent: `S_BPREL32` 0x110b, `S_REGISTER` 0x1106, `S_FILESTATIC` 0x1153, `S_DEFRANGE_REGISTER_REL_INDIR` 0x1177, `S_REGREL32_ENCTMP` 0x1179, `S_MANSLOT` 0x1120.
  - The document says `S_REGREL32` occurs only in `S_LPROC32`/`S_GPROC32` scopes. LLVM and pdbrs also have `S_LPROC32_ID`/`S_GPROC32_ID` procedures. pdbrs does not enforce placement or `S_LOCAL` pairing.
- Undocumented assumptions:
  - The gap array must be a multiple of 4 bytes. pdbrs returns all gaps. LLVM `CodeViewRecordIO::mapVectorTail` stops at a gap whose first byte is >= 0xF0 (it treats it as `LF_PAD`), so LLVM hides gaps in some records; the fixture bytes confirm the hidden gaps (#5).
  - `S_REGREL32` offset is u32 (LLVM `RegRelativeSym`); `S_DEFRANGE_REGISTER_REL_INDIR` `offset_in_udt` is i32 (LLVM `little32_t`) (#5).
  - `S_REGREL32_ENCTMP` (0x1179) uses the `S_REGREL32` layout (fixture-confirmed, #13).
  - Register fields are raw `CV_HREG_e` values for the CPU in the compile record (`variable.rs`).

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
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_procs.md`
- Documents: Procedure scopes, the procedure start kinds, the `S_LPROC32` (0x110f)/`S_GPROC32` (0x1110) layout and `CV_PROCFLAGS` bits, the `S_LMANPROC` (0x112b)/`S_GMANPROC` (0x112a) layout, and the nested kinds.
- pdbrs: partial — `Procedure::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` decodes `S_GPROC32`, `S_LPROC32` (TPI type) and `S_GPROC32_ID`, `S_LPROC32_ID` (IPI ID, `ProcedureType`). `ProcedureFlags` names bits 0-7. `ManagedProcedure::read` in `codeview/symbol/managed.rs` decodes the managed kinds with `return_register`. `scope.rs` opens scopes for all of them. Not implemented: `S_LPROC32_DPC` (0x1155), `S_LPROC32_DPC_ID` (0x1156), `S_LPROC32_ST` (0x100a). The scope walk rejects the DPC kinds with `UnsupportedVersion`.
- Conflicts:
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: bit 0 is `fpo` "true if function has frame pointer omitted". cvinfo.h `CV_PFLAG_NOFPO` says "frame pointer present"; LLVM `HasFP`, ms-codeview `NOFPO` ("Frame pointer present") and pdbrs `HAS_FRAME_POINTER` agree. The document inverts the meaning.
  - Document vs cvinfo.h/LLVM/pdbrs: bit 2 is "unused, must be zero". cvinfo.h gives `CV_PFLAG_FAR` (far return); pdbrs `HAS_FAR_RETURN`.
  - Document vs cvinfo.h/LLVM/pdbrs: bit 4 is "unused, must be zero". cvinfo.h gives `CV_PFLAG_NOTREACHED`; LLVM `IsUnreachable`; pdbrs `UNREACHABLE`.
  - Document vs pdbrs/LLVM lld: the document says `p_parent`, `p_end`, `p_next` are offsets "relative to the `S_LPROC32` record". pdbrs treats them as module-stream offsets that include the 4-byte signature (`ScopeLinks`), and checks `end` == offset of the end record. lld writes the absolute module-stream offset. The fixtures pass this check (#5).
  - Document vs cvinfo.h/LLVM/ms-codeview/pdbrs: the managed `Procedure` struct omits `uint16_t retReg` after `flags`. cvinfo.h `MANPROCSYM` has it; pdbrs reads `return_register` (fixture-confirmed, #14).
  - ms-codeview vs ms-codeview/cvinfo.h/pdbrs: ms-codeview has two managed layouts. `ManagedProcFixed` has no padding (agrees with packed cvinfo.h and pdbrs). `ManProcSymFixed` (line 938) inserts `padding: u8` before `ret_reg`, which shifts `ret_reg` and the name by 1 byte.
  - Document: the text says the managed record "is similar to `S_GDATA32`". The layout is the `S_GPROC32` layout with a metadata token in place of the type index.
  - Omission: the procedure start lists omit `S_GPROC32_ID` (0x1147) and `S_LPROC32_ID` (0x1146), which pdbrs and LLVM decode.
  - Internal: the bullet list is broken ("visible only within a single * `S_LPROC32_DPC`: …"). Typos: `s_parent` for `p_parent`, "i.e.g", `G_PROC32`.
  - The document says encoders set `p_next` to 0 and decoders ignore it. pdbrs keeps `next` raw and does not check it.
- Undocumented assumptions:
  - The top-level parent link must be 0; a nonzero value gives `InvalidReference` (`scope.rs`).
  - `S_END` ends managed procedure scopes. LLVM `symbolOpensScope` does not list `S_GMANPROC`/`S_LMANPROC`; pdbrs adds them because `MANPROCSYM` has links (#14).
  - The managed `token` is usually a `MethodDef` token (table 0x06), and the name can be empty (#14).

### `microsoft-pdb-rs/docs/codeview/symbols/s_pub.md`
- Documents: The `S_PUB32` (0x110e) record (u32 flags, offset, segment, name), which appears only in the global symbol stream.
- pdbrs: implemented — `Public::read` in `crates/pdbrs/src/native/codeview/symbol/global.rs` reads the layout. `PublicFlags` names bits 0-3. `symbols/psi.rs` checks the PSI address map order against the `S_PUB32` addresses.
- Conflicts:
  - Omission: the document does not describe `flags`. cvinfo.h `CV_PUBSYMFLAGS_e` gives `cvpsfCode` 0x1, `cvpsfFunction` 0x2, `cvpsfManaged` 0x4, `cvpsfMSIL` 0x8. LLVM `PublicSymFlags` and pdbrs `PublicFlags` agree.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_refsyms.md`
- Documents: The `REFSYM2` layout (name checksum, symbol offset, 1-based module index, name) and the kinds `S_PROCREF` (0x1125), `S_DATAREF` (0x1126), `S_LPROCREF` (0x1127), `S_TOKENREF` (0x1129), `S_ANNOTATIONREF` (0x1128).
- pdbrs: partial — `ProcedureReference::read` in `crates/pdbrs/src/native/codeview/symbol/global.rs` decodes `S_PROCREF`, `S_LPROCREF` and `S_TOKENREF` (`SymbolData::ProcedureReference`). `module` stays raw; `module_index()` returns `module - 1` and `None` for 0. `offset` includes the 4-byte module-stream signature. Not implemented: `S_DATAREF` and `S_ANNOTATIONREF`; they give `Unsupported`. The library does not resolve references. The full-read example (`examples/native/symbols.rs`) checks that each reference names a procedure record start in a module.
- Conflicts:
  - Internal: the document says `name_checksum` "appears to be set to 0" and then says it "is a checksum computed over the name" with a TODO for the algorithm. cvinfo.h `REFSYM2` calls it `sumName` ("SUC of the name") and gives no algorithm. lld writes 0 (`ps.SumName = 0`). pdbrs keeps the value raw and does not check it.
  - TODO: `S_TOKENREF` targets are open in the document. cvinfo.h says "Reference to one of the many MANPROCSYM's". pdbrs treats it as a reference to `S_GMANPROC`/`S_LMANPROC` (#14).
  - The document says `S_PROCREF` references `S_GPROC32`. lld also creates references from `S_GPROC32_ID`/`S_LPROC32_ID` after it converts them (`translateIdSymbols`, `symbolGoesInGlobalsStream`). (Inference, not tested: MSVC linker behavior for `_ID` kinds not checked..)
  - pdbrs does not check the invariant "module_index ≤ num_modules" in the library; only the example checks it.
- Undocumented assumptions: LLVM `ProcRefSym::modi()` and lld (`++ps.Module`, "MSVC seems to add one") confirm the 1-based module number.

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
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_thunk.md`
- Documents: The `S_THUNK32` (0x1102) record (p_parent, p_end, p_next, offset, segment, u16 length, u8 ordinal, name, variant data).
- pdbrs: implemented — `Thunk::read` in `crates/pdbrs/src/native/codeview/symbol/procedure.rs` reads the layout. `ThunkOrdinal` names 0-6. `scope.rs` opens a scope that `S_END` closes.
- Conflicts:
  - Omission: the document does not list the ordinal values. LLVM `ThunkOrdinal` gives `Standard` 0, `ThisAdjustor` 1, `Vcall` 2, `Pcode` 3, `UnknownLoad` 4, `TrampIncremental` 5, `BranchIsland` 6. pdbrs uses the same values. The pinned cvinfo.h does not define `THUNK_ORDINAL` (it is in `cvconst.h`).
  - Omission: the document does not give the variant data layout or length for any ordinal.
  - Document: it refers to `S_LPROC32` for the link meaning; see the offset conflict in `s_procs.md`.
- Undocumented assumptions:
  - A standard thunk (ordinal 0) has no variant data; pdbrs checks the zero padding after the name. For other ordinals, pdbrs keeps all bytes after the name raw, including the padding, because no reference gives the variant length. All 136 sample thunks are standard (#5).
  - pdbrs does not check `p_next`.

### `microsoft-pdb-rs/docs/codeview/symbols/s_trampoline.md`
- Documents: The `S_TRAMPOLINE` (0x112c) record (u16 kind, u16 thunk size, thunk offset, target offset, thunk section, target section) and kinds 0 (incremental) and 1 (branch island).
- pdbrs: implemented — `Trampoline::read` in `crates/pdbrs/src/native/codeview/symbol/link.rs` reads the layout. `TrampolineKind` names 0 and 1. `Symbol::address` returns the thunk address.
- Conflicts:
  - None found. The layout agrees with cvinfo.h `TRAMPOLINESYM`/`TRAMP_e` and LLVM `TrampolineSym`/`TrampolineType`.
  - Omission: the document does not describe `thunk_offset`, `target_offset`, `thunk_section` and `target_section`.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/symbols/s_udt.md`
- Documents: The `S_UDT` (0x1108) record (type index, name) and the expected target type kinds.
- pdbrs: implemented — `UserDefinedType::read` in `crates/pdbrs/src/native/codeview/symbol/variable.rs` reads the layout. pdbrs does not check the kind of the target type or that the names are equal.
- Conflicts:
  - TODO: "Can `S_UDT` point to primitives? What about typedefs?" stays open. cvinfo.h and LLVM do not restrict the target. pdbrs accepts any type index.
  - Internal: the document says `S_UDT` is "usually found in the global symbol stream, but in rare cases … in a module symbol stream", then says it "can appear in the global symbol stream and in module symbol streams". lld puts function-local `S_UDT` in module streams (`symbolGoesInModuleStream`: scope depth > 0).
- Undocumented assumptions: None.
