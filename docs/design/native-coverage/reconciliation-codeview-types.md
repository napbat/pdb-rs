# Native coverage: reconciliation of the Microsoft pdb-rs CodeView type, item and line documents

This page is part of the [native coverage](../native-coverage.md) accounting. It covers `docs/reference/microsoft-pdb-rs/docs/codeview/` (top level, `types/` and `items/`).
Each section names one imported document. Paths are relative to `docs/reference/`. The fields are:

- **Documents**: the format items that the document describes.
- **pdbrs**: `implemented` (pdbrs reads every item that the document describes), `partial`, `not implemented` or `not applicable` (no format content), with the source files.
- **Conflicts**: each difference between the document and pdbrs, or between the document and another source (`cvinfo.h`, LLVM source, `ms-pdb`, other documents). Errors inside the document are conflicts too.
- **Undocumented assumptions**: layouts and rules that pdbrs uses but the document does not state, with the issue that records the evidence.

The comparison used `main` at `b4d4cdc`, `cvinfo.h` at `805655a2`, LLVM at `b9f2235f` and the `ms-codeview` 0.1.7, `ms-pdb` 0.1.22, `ms-pdb-msf` 0.1.7 and `ms-pdb-msfz` 0.1.10 crates. "Inference, not tested" marks a statement that comes from reading source code only.

Common pdbrs facts used below:
- Record framing: `crates/pdbrs/src/native/codeview.rs` (`Records`). The `u16` length counts the kind and the payload. Length < 2 is `InvalidFormat`. pdbrs does not require an even length or an aligned record start.
- End padding of TPI and IPI records: `Fields::finish_type_padding` (`codeview.rs:282-296`) accepts 0 to 3 bytes `f3 f2 f1` only. Other trailing bytes are `InvalidFormat`. The one exception is the MASM form at the end of a UDT record (see `lf_class.md`, #29).
- Encoders: `TypeLeaf::encode` (`codeview/type_record.rs:357-443`), `Member::encode` and `encode_field_list` (`codeview/field_list/encode.rs`) and `IdRecord::encode` (`codeview/id_record/encode.rs`) write each decoded record to the same bytes (#29). They pad to a multiple of 4 bytes with `LF_PAD` bytes. `types::write_type_stream` (`native/types/write.rs`) writes the TPI and IPI streams and their hash streams. `native::PdbBuilder` (#33) and `native::PdbEditor` (#34) use these encoders.
- Names: NUL-terminated bytes (`Fields::name`), not decoded as text.
- Numeric leaves in type records, field-list members, `S_CONSTANT` and `S_MANCONSTANT`: `Numeric::parse_prefix` (`codeview/numeric.rs:337-415`) decodes every leaf tag of `cvinfo.h` and keeps the tag and the exact value bytes (#29). An unknown tag is `UnsupportedVersion`.
- Reference checks: `types::validate_type_references` (`native/types/validate.rs`) checks each TPI type index by its role (`codeview/type_record/references.rs`). `IdTable::validate` (`codeview/id_record/table.rs:162-258`) checks the IPI references.


## Top level

### `microsoft-pdb-rs/docs/codeview/primitive_types.md`
- Documents: The bit layout of primitive type indices (size, type, mode), the `TY_*` type families, the `MODE_*` pointer modes and tables of `T_*` values.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/index.rs` decodes an index below `FIRST_RECORD_INDEX` (0x1000) as `PrimitiveKind` (whole low byte, bits 0-7, 58 values including `CvPointer` 0xf0) and `PrimitiveMode` (byte 1, bits 8-11, values 0-7) (`index.rs:28-54`, `95-307`). An unknown kind or mode is `InvalidFormat`. pdbrs does not split the kind into type and size nibbles. `PrimitiveKind::size`, `PrimitiveMode::pointer_size` and `Primitive::size` give the byte size; `PrimitiveKind::is_signed_integer` gives the signedness (`index.rs:75-93`, `309-411`, #29). `native/types.rs:378-384` rejects a TPI/IPI `index_begin` below 0x1000.
- Conflicts:
  - Document omission vs `cvinfo.h`: `cvinfo.h` defines kind `0xf0` with pointer modes only (for example `T_NCVPTR` `0x01f0`, `T_32NCVPTR` `0x04f0`, `T_64NCVPTR` `0x06f0`). The document does not list these indices. pdbrs decodes kind 0xf0 with modes 1-6 as `PrimitiveKind::CvPointer` (#29 fixed the `InvalidFormat` result that #28 found).
  - Document vs `cvinfo.h`: the document says bit 3 is reserved and "always zero" and `size` is bits 0-2. `cvinfo.h` defines `CV_SMASK 0x00f` (subtype is bits 0-3). Real values set bit 3: `T_INT1` 0x68, `T_UINT1` 0x69, `T_HRESULT` 0x08, `T_INT16` 0x78, `T_UINT16` 0x79, `T_CHAR16` 0x7a, `T_CHAR32` 0x7b. pdbrs decodes all of them.
  - Document vs `cvinfo.h` (`CV_prmode_e`): the mode table stops at 6. `cvinfo.h` defines `CV_TM_NPTR128 = 7`. pdbrs decodes 7 as `NearPointer128` (16-byte pointer).
  - Document vs `cvinfo.h` (`CV_integral_e`, `CV_real_e`, `CV_special_e`, `CV_special2_e`): the size table marks these values "reserved" or "obsolete", but `cvinfo.h` defines them: `CV_IN_16BYTE = 4` (`T_OCT` 0x14, `T_UOCT` 0x24); `CV_RC_REAL32PP = 5`, `CV_RC_REAL16 = 6` (0x45, 0x46); `CV_SP_CURRENCY`..`CV_SP_NOTTRANS` = 4-7 and `CV_SP_HRESULT = 8`; `CV_S2_BOOL32FF = 2` (0x62); `CV_INT` subtypes 8-0xb. pdbrs decodes all of these.
  - Document vs LLVM `TypeIndex.h`: the tables omit kinds that LLVM and pdbrs decode and the pinned `cvinfo.h` does not define: `Boolean128` 0x34, `Complex48` 0x54, `Complex32PartialPrecision` 0x55, `Complex16` 0x56, `Character8` (`char8_t`) 0x7c.
  - Document vs LLVM `CodeViewDebug.cpp:1880-1930`: the table maps C/C++ `char` to `T_CHAR` (0x10) and gives no index for `int`. LLVM writes plain `char` as `NarrowCharacter` (`T_RCHAR` 0x70) and 4-byte `int` as `Int32` (`T_INT4` 0x74); `T_CHAR` is `signed char`. The `T_RCHAR` row has no size and asks "is this `__int8`?"; `__int8` is `T_INT1` 0x68, which the document omits.
  - Document vs pdbrs: the document says every index below `type_index_begin` is primitive. pdbrs `TypeIndex::is_primitive` uses the fixed bound 0x1000, not the stream header value.
  - Document errors: a `TODO` remains ("actually delete obsolete variants"). The 48-bit to 80-bit real and complex sections put the description in the "C/C++ Type" column. The text "thse" and "can taken" are typing errors.
- Undocumented assumptions:
  - pdbrs accepts mode bits for every kind except 0xf0, for example 0x0600 (`T_NOTYPE` with mode 6). `cvinfo.h` defines no such value. Kind 0xf0 with mode 0 or 7 is `InvalidFormat`, because `cvinfo.h` defines it only as a pointer (`index.rs:44-51`, #29).
  - pdbrs rejects bit 11 set (mode byte 8-15) as `InvalidFormat`. This agrees with the document's reserved bit 11 but the document does not say that a reader must reject it.
  - Sizes that the document does not give: `Complex48` 12 bytes, `Complex16` 4 bytes, `Complex32PartialPrecision` 8 bytes, `Boolean128` 16 bytes, `HResult` 4 bytes, `Currency` 8 bytes. No size for no type, absolute, segment, `void`, BASIC strings, not translated, bit, Pascal `CHAR` and `CvPointer` in direct mode (`index.rs:309-376`, #29). pdbrs gives `NarrowCharacter` (`T_RCHAR`) and `WideCharacter` (`T_WCHAR`) a signedness (signed and unsigned); `cvinfo.h` does not state it.
  - Producer fixtures of #39 hold the rare kinds: `ml64` and `ml` write `T_REAL48`, `T_INT8`, `T_REAL80` and `T_UINT16`; clang-cl and flang write `T_OCT`, `T_UOCT`, `T_REAL16`, `T_REAL80`, `T_REAL128`, `T_CPLX32` to `T_CPLX128`, LLVM `Complex16` (`0x56`) and `T_BOOL16` to `T_BOOL64`; MSVC ARM64EC writes `T_UINT2`. `llvm-pdbutil` names all of them.

### `microsoft-pdb-rs/docs/codeview/number.md`
- Documents: The numeric leaf (`Number`): immediate values below 0x8000 and leaf tags 0x8000-0x801b with their payload sizes, `LF_VARSTRING`, and examples.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/numeric.rs` (`Numeric`, `numeric.rs:293-415`) decodes immediates and every tag of the document and of `cvinfo.h`: the integer leaves into `IntegerLeaf` (value and encoding width), `LF_REAL16`/`32`/`48`/`64`/`80`/`128`, `LF_COMPLEX32`/`64`/`80`/`128`, `LF_DECIMAL`, `LF_DATE` as exact value bytes, `LF_VARSTRING` (u16 length + bytes) and `LF_UTF8STRING` (NUL-terminated) (#29). Typed accessors: `integer`, `real16_bits`, `real32`, `real64`, `complex32`, `complex64`, `decimal`, `date`, `string`. 48-, 80- and 128-bit reals and 80- and 128-bit complex values stay bytes (no Rust type). `Numeric::encode` writes the same bytes again; `Numeric::from_integer` gives the smallest form (LLVM rules, extended to 128 bits). Type records, field-list members, `S_CONSTANT` (`codeview/symbol/variable.rs`) and `S_MANCONSTANT` (`codeview/symbol/managed.rs`) use `Numeric`. `Integer::parse_prefix` still accepts only the integer leaves.
- Conflicts:
  - Document internal error: example row `04 80 fe ff ff ff | LF_LONG | -2` uses leaf 0x8004, which is `LF_ULONG`. `LF_LONG` -2 is `03 80 fe ff ff ff`.
  - Document internal error: example `10 80 04 00 44 45 46` declares length 4 but has 3 string bytes for "DEF". The length must be `03 00`.
  - Document vs `cvinfo.h`, LLVM `CodeViewTypes.def` and `ms-codeview` `number.rs`: the table omits `LF_REAL16` = 0x801c (2 bytes). pdbrs decodes it in all contexts (#29).
  - Document vs pdbrs: an unknown tag above 0x8000 is not a valid `Number`. The document does not say what a reader must do. pdbrs returns `UnsupportedVersion`, so the record fails to decode.
  - `LF_DATE` (`cvinfo.h` vs Roslyn): `cvinfo.h` names an OLE automation date (an `f64` of days since 30 December 1899). Roslyn writes the .NET `DateTime.Ticks` value as a little-endian `i64` in the `S_MANCONSTANT` records of VB `Date` constants (`dotnet-vb-full`), and DIA returns the same 8 bytes as `VT_DATE`. `Numeric::date` keeps the `cvinfo.h` meaning; `Numeric::date_ticks` gives the tick count (#29).
- Undocumented assumptions:
  - pdbrs reports immediate values as `Unsigned` and keeps signedness and width for each leaf (`IntegerLeaf`, `numeric.rs:144-276`), so a non-canonical width (for example `LF_ULONG` 5) encodes again to the same bytes (#29).
  - Value sizes of the non-integer leaves come from the `cvinfo.h` `lfReal*`/`lfCmplx*`/`lfDecimal`/`lfDate` structures: `LF_REAL16`=2, `LF_REAL32`=4, `LF_REAL48`=6, `LF_REAL64`/`LF_COMPLEX32`/`LF_DATE`=8, `LF_REAL80`=10, `LF_REAL128`/`LF_COMPLEX64`/`LF_DECIMAL`=16, `LF_COMPLEX80`=20, `LF_COMPLEX128`=32 (#14, #29). `LF_UTF8STRING` without a NUL terminator is `InvalidFormat`.
  - `Numeric::from_integer` picks an immediate, `LF_USHORT`, `LF_ULONG`, `LF_UQUADWORD` or `LF_UOCTWORD` for a value that is not negative, and `LF_CHAR`, `LF_SHORT`, `LF_LONG`, `LF_QUADWORD` or `LF_OCTWORD` for a negative value (LLVM `writeEncodedSignedInteger`/`writeEncodedUnsignedInteger`, `numeric.rs:417-455`).
  - No fixture has a non-integer numeric leaf; only synthetic tests (`tests/numeric_leaves.rs`) cover them (#29).
  - Roslyn writes one fixed numeric leaf for each constant type (for example `LF_LONG` for `int` 0 and `LF_SHORT` for `short` 32767), and `LF_VARSTRING` with UTF-8 bytes for all strings, also non-ASCII text and NUL. It does not write `LF_UTF8STRING` (#39).

### `microsoft-pdb-rs/docs/codeview/line_data.md`
- Documents: The C13 line region of a module stream: subsection framing and alignment, the `DEBUG_S_*` kind list 0xf1-0x100 and the ignore bit, the `DEBUG_S_LINES` header, blocks, line and column entries, and the `DEBUG_S_FILECHKSMS` entries with checksum kinds.
- pdbrs: partial — framing in `crates/pdbrs/src/native/dbi/module.rs` (`ModuleStream::c13_subsections`: 8-byte header, data, padding to 4 bytes must be present). `crates/pdbrs/src/native/lines.rs` (`SubsectionKind`, `LineSubsection::decode`, `lines.rs:137-337`) decodes every kind 0xf1-0x100 except 0xfe: 0xf1 (`lines/symbol_records.rs`), 0xf2 and 0xf9 (`lines/blocks.rs`), 0xf3 (`lines/string_table.rs`), 0xf4 (`lines/checksums.rs`), 0xf5 (`lines/frame_data.rs`), 0xf6 (`lines/inlinee.rs`), 0xf7 and 0xf8 (`lines/cross_scope.rs`), 0xfa and 0xfb (`lines/token_map.rs`), 0xfc (`lines/assembly_input.rs`), 0xfd (`lines/coff_symbol_rva.rs`), 0xff and 0x100 (`lines/xfg_hash.rs`) (#31). A kind with bit 31 gives `LineSubsection::Ignored { kind, data }`. 0xfe (`DEBUG_S_INLINEE_MDTOKEN_MAP`) and unknown kinds stay raw (`LineSubsection::Unsupported`). Each value has an encoder; `LineSubsection::encode_framed` and `ModuleLines::encode` write the region again. `lines/module.rs` (`ModuleLines`) rejects a second 0xf4 subsection and `validate()` requires each block and inlinee file reference to be the start of a checksum entry.
- Conflicts:
  - Document vs pdbrs: the document says "Decoders should always use `checksum_size` … rather than making assumptions … based on `checksum_kind`". pdbrs rejects a size that differs from the kind size (`InvalidFormat`) and an unknown kind > 3 (`UnsupportedVersion`) (`checksums.rs:296-303`).
  - Document vs pdbrs: invariant "`num_lines` is greater than zero". pdbrs accepts a block with 0 lines.
  - Document vs pdbrs: invariant "Block offset sorted". pdbrs checks a stricter rule: line offsets do not decrease inside a block and across blocks, and each line offset is below `contribution_size` (`LinesSubsection::decode` in `blocks.rs`).
  - Document vs pdbrs: padding after a checksum entry is "zero" only as a Determinism note. pdbrs requires zero padding (`InvalidFormat` otherwise) and accepts absent padding after the last entry.
  - Document vs pinned `cvinfo.h` and LLVM: the table lists `DEBUG_S_INLINEE_MDTOKEN_MAP` = 0xfe, `DEBUG_S_XFGHASH_TYPE` = 0xff and `DEBUG_S_XFGHASH_VIRTUAL` = 0x100. The pinned `cvinfo.h` `DEBUG_S_SUBSECTION_TYPE` ends at 0xfd. LLVM `DebugSubsectionKind` has 0xff and 0x100 but not 0xfe. `ms-pdb` `lines.rs` ends at 0xfd. 0xfe has no source other than this document; pdbrs takes the name from it. No source gives the layout of 0xfe and no local producer writes it, so pdbrs keeps it raw (external input, #31).
  - Document internal: the headings and text use `DEBUG_S_FILE_CHECKSUMS`; the kind table and `cvinfo.h` use `DEBUG_S_FILECHKSMS`.
  - Document internal: the link `../pdb/dbi_streams.md#dbi-modules-substream` is broken (the files are `pdb/dbi_stream.md` and `pdb/dbi_modules.md`). The link `../pdb/dbi_stream.md#dbi-sources-substream` points to the wrong file (`pdb/dbi_sources.md` exists).
  - Document omissions: no layout for 0xf1, 0xf3 and 0xf5-0xfd, 0xff and 0x100, which pdbrs decodes; no value for the `HAVE_COLUMNS` flag (`cvinfo.h` `CV_LINES_HAVE_COLUMNS 0x0001`); a `TODO` on `file_offset`; the section "Correlation of `FileChecksum` and DBI Sources Substream" has no content; the example refers to highlights that a text document cannot show.
- Undocumented assumptions:
  - 0xfd `DEBUG_S_COFF_SYMBOL_RVA`: a header-less `u32` array, size multiple of 4 (LLVM `DebugSymbolRVASubsection.h`). Entry `i` is the image RVA of COFF symbol record `i`; 0 for no address; 0xffffffff for an external resolved to an absolute value (#13, https://github.com/napbat/pdbrs/issues/13#issuecomment-6409581275, https://github.com/napbat/pdbrs/issues/13#issuecomment-6409665284).
  - 0xf6 `DEBUG_S_INLINEELINES`: `u32` signature 0 or 1 (`CV_INLINEE_SOURCE_LINE_SIGNATURE[_EX]`), entries `inlinee` (IPI index), `fileId`, `sourceLineNum`, and for signature 1 a `u32` count plus that many checksum offsets. Other signatures are `UnsupportedVersion` (`cvinfo.h:4819-4836`, LLVM `DebugInlineeLinesSubsection.cpp`).
  - 0xf1 `DEBUG_S_SYMBOLS`: symbol records without a signature; 0xf3 `DEBUG_S_STRINGTABLE`: NUL-terminated strings, the last byte must be NUL (LLVM `DebugSymbolsSubsection`, `DebugStringTableSubsection`, #31).
  - 0xf5 `DEBUG_S_FRAMEDATA`: an optional `u32` relocation value and 32-byte `FRAMEDATA` records. `cvdump` always reads the relocation value; LLVM reads it only if the size is not a multiple of 32. pdbrs keeps it as `Option<u32>`; a size that is neither 32·n nor 4 + 32·n is `InvalidFormat` (`frame_data.rs:1-66`, #31).
  - 0xf7 `DEBUG_S_CROSSSCOPEIMPORTS`: entries of a `u32` module name offset, a `u32` count and that many `u32` item IDs (`cvinfo.h` `CrossScopeReferences`). 0xf8 `DEBUG_S_CROSSSCOPEEXPORTS`: 8-byte pairs of local and global ID (`LocalIdAndGlobalIdPair`). The IDs stay raw `u32` values (#31). `lines::validate_cross_scope` (`lines/cross_scope/validate.rs`) checks the references and reports findings, not errors: bit 31 of a local ID (`DecoratedItemId`) selects the IPI stream, else the global ID is a TPI index (`ExportGlobalId`, not checked for a `NOTM` or `MINI` PDB); a local ID occurs in one export only (`ExportDuplicate`); the import module name is a `/names` string (`ImportModuleName`) that names a DBI module or object file, ASCII case ignored (`ImportModuleUnknown`); an imported ID is an export of that module if the module has an export subsection (`ImportNotExported`). The rules come from `Mod1::fUpdateTiOrIdReferencedByNonSyms` in `microsoft-pdb`; the document does not state them (#31).
  - 0xf9 `DEBUG_S_IL_LINES` uses the `DEBUG_S_LINES` layout with IL offsets (`DumpModILLines` in `cvdump`, #31).
  - 0xfa `DEBUG_S_FUNC_MDTOKEN_MAP` and 0xfb `DEBUG_S_TYPE_MDTOKEN_MAP`: a `u32` count and `u32` (key, value) pairs; bit 31 of the value marks a metadata token. Bytes after the entries are `InvalidFormat` (`DumpModTokenMap` in `cvdump`, `Mod1::fUpdateTiOrIdReferencedByNonSyms`, #31).
  - 0xfc `DEBUG_S_MERGED_ASSEMBLYINPUT`: time stamp, index (bit 31 = PDB present), `u16` version size that counts itself, version bytes, NUL-terminated name, zero padding to 4 bytes (`DumpModMergedAssemblyInput` in `cvdump`, #31).
  - 0xff `DEBUG_S_XFGHASH_TYPE`: 12-byte entries (`u64` hash, function type index). 0x100 `DEBUG_S_XFGHASH_VIRTUAL`: 16-byte entries (`u64` hash, class type index, `u32` offset). No document defines them; the layouts come from the MSVC 14.44 objects of the `msvc-x64-xfg` fixture (#27, #31). `link.exe` does not copy them to the PDB.
  - 0xf1, 0xf3, 0xf5, 0xff and 0x100 occur only in object files of the fixtures; 0xf7-0xfc have synthetic tests only (`tests/lines_codec/`, `tests/cross_scope.rs`, #31). Checksum kinds: `msvc-x64-sha1` (`cl /ZH:SHA1`) has SHA-1 entries (kind 2) next to SHA-256 entries of the runtime sources (#31).
  - pdbrs rejects `DEBUG_S_LINES` header flags other than bit 0 (`UnsupportedVersion`) and requires `block_size` = 12 + count × (8 or 12) exactly.
  - pdbrs names the line values 0xfeefee (always step into) and 0xf00f00 (never step into) (`blocks.rs:10-16`).
  - pdbrs does not decode a kind with bit 31 set, also if the low bits name a known kind. `cvinfo.h` tells readers to ignore the contents; the encoder writes the data again without a change (#31).
  - The encoders write zero padding after a subsection; the subsection header length excludes the padding. `ModuleStreamParts::encode` requires the C13 region to end at a 4-byte boundary (`Mod1::fCommit`, #31).

### `microsoft-pdb-rs/docs/codeview/codeview.md`
- Documents: An overview of CodeView, where CodeView data is stored (linker PDBs, compiler PDBs, COFF `.debug$S`), and links to the other documents.
- pdbrs: partial — pdbrs reads and writes CodeView in PDB files (TPI, IPI, module streams, global streams; `crates/pdbrs/src/native/**`, `native::PdbBuilder` #33, `native::PdbEditor` #34). pdbrs has no COFF object reader, so it does not find `.debug$S` and `.debug$T` sections in OBJ files. The record and subsection decoders accept the bytes of these sections: in #29, 17,652 type and ID records of 31 object files re-encode with 0 differences; in #31, every `.debug$S` subsection of 102 fixture object files re-encodes with 0 differences. Compiler (`/Zi`) PDBs use the same MSF container: the 15 compiler `obj/*.pdb` files of the fixtures rebuild identically (#29).
- Conflicts:
  - Document omission: the list does not link `calling_convention.md`, which exists in the same directory.
  - None found against pdbrs.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/calling_convention.md`
- Documents: The `CV_call_e` calling convention values 0x00-0x19.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/attributes.rs:231-287` (`CallingConvention`), used by `LF_PROCEDURE` and `LF_MFUNCTION` (`type_record/basic.rs:38-128`). Values 0x00-0x19 decode, including 0x06 `Skipped` (#29). Other values are `InvalidFormat`, so the whole record fails.
- Conflicts:
  - Document and `cvconst.h` vs LLVM: 0x06 `SKIPPED` is in the document and in `cvconst.h` (`CV_CALL_SKIPPED`). LLVM `CallingConvention` has no value 0x06. pdbrs follows `cvconst.h` and decodes it as `Skipped`.
  - Document and `cvconst.h` vs LLVM and pdbrs: 0x19 is `RESERVED` ("first unused call enumeration"). LLVM and pdbrs decode 0x19 as `Swift`.
  - Document vs pinned sources: the document cites `cvinfo.h` as a source, but the pinned `cvinfo.h` does not define `CV_call_e`; only `cvconst.h:31-57` does. The document drops the `CV_CALL_` prefix.
  - Document internal errors: a stray line "T" (line 3); the intro says the file specifies "constants (named values and enums)" but it has only one enum.
- Undocumented assumptions:
  - A value above 0x19 fails the whole record (`InvalidFormat`). The document does not say what a reader must do with it.


## `types/`

### `microsoft-pdb-rs/docs/codeview/types/types.md`
- Documents: The type record framing (`u16 size`, `u16 kind`), the size and alignment invariants, record categories, and a summary table of leaf kinds.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview.rs` frames records; `crates/pdbrs/src/native/codeview/type_record.rs:262-288,463-492` decodes every kind of the summary table and every other modern leaf of `cvinfo.h` and LLVM: also `LF_LABEL`, `LF_VFTABLE`, `LF_INTERFACE`, `LF_COBOL0`, `LF_BARRAY`, `LF_VFTPATH`, `LF_OEM`, `LF_OEM2`, `LF_DERIVED`, `LF_DIMCONU`, `LF_DIMCONLU`, `LF_DIMVARU`, `LF_DIMVARLU`, `LF_TYPESERVER`, `LF_TYPESERVER2`, `LF_DIMARRAY`, `LF_ALIAS`, `LF_DEFARG`, `LF_MANAGED`, `LF_STRIDED_ARRAY`, `LF_HLSL`, `LF_MODIFIER_EX`, `LF_VECTOR`, `LF_MATRIX` and `LF_CLASS2`/`LF_STRUCTURE2`/`LF_UNION2`/`LF_INTERFACE2` (#29). `TypeLeaf::Unsupported(Record)` keeps the raw payload of `LF_RANGELIST`, `LF_TAGGED_UNION`, `LF_TAGGED_UNION_CASE`, `LF_TUCASE`, `LF_IFC_RECORD` (no layout in the pinned sources) and of all `_16t`/`_ST` kinds. `TypeLeaf::encode` writes each record again.
- Conflicts:
  - Document vs `cvinfo.h`, LLVM and pdbrs: the table lists `LF_ARRAY` as 0x1502. 0x1502 is `LF_ENUMERATE`; `LF_ARRAY` is 0x1503 (`cvinfo.h:888`, `CodeViewTypes.def:47`, `type_record.rs:107`). `lf_array.md` has the correct value.
  - Document vs `cvinfo.h`: the table names 0x1205 `LF_BITFIELDS`; `cvinfo.h` names it `LF_BITFIELD`.
  - Document vs pdbrs: invariant "`size` is a multiple of 2". pdbrs does not check it. Observed IPI records are not 4-byte aligned: `LF_UDT_MOD_SRC_LINE` records are 18 bytes without padding (example at offset 0x78 in `items/lf_func_id.md`), so the next record starts at 0x8a. pdbrs does not require alignment.
  - Document omissions: the table omits `LF_INTERFACE` (0x1519, described in `lf_class.md`), `LF_LABEL` (0x000e), `LF_VFTABLE` (0x151d) and the other kinds above, which pdbrs decodes, and `LF_CLASS2`..`LF_INTERFACE2` (LLVM only). The sections "Summary of item records" and "Leaf indices referenced from symbols" are empty. The document does not describe `LF_PAD` (0xf0-0xff) end padding.
- Undocumented assumptions:
  - pdbrs requires the bytes after the last field to be 0-3 `LF_PAD` bytes in the exact `f3 f2 f1` pattern (`codeview.rs:282-296`). Exception: a UDT record can end with the MASM form (see `lf_class.md`, #29).
  - The encoders pad every record to a multiple of 4 bytes, as Microsoft and LLVM do. `LF_OEM`, `LF_OEM2`, `LF_COBOL0`, `LF_SKIP` and unknown kinds keep their producer data at the end and get no padding (`type_record.rs:357-443`, #29).
  - Kind values and layouts follow LLVM `CodeViewTypes.def` and `TypeRecordMapping.cpp` at `b9f2235f`; flag bit layouts follow `cvinfo.h` (`type_record.rs:1-13`). `LF_VFTABLE` and `LF_LABEL` layouts are confirmed against `lld-link` output (#11, https://github.com/napbat/pdbrs/issues/11#issuecomment-6408363519). Fixtures contain `LF_HLSL`, `LF_VECTOR`, `LF_MATRIX`, `LF_MODIFIER_EX` and `LF_STRIDED_ARRAY` (fxc, #27); `LF_PRECOMP`, `LF_ENDPRECOMP` and `LF_TYPESERVER2` occur only in object files. The other added kinds have synthetic tests only (#29).
  - `LF_HLSL`: `cvinfo.h` says that bits 4-15 of the property word are zero. The Windows SDK `fxc` writes `0x07a0`, `0x07b0`, `0x07c0`, `0x4e50` and `0x6260`. pdbrs accepts any value, keeps the raw word and uses only bits 0-3 as the property count (`type_record/hlsl.rs`, #29, https://github.com/napbat/pdbrs/issues/29#issuecomment-6413480651).
  - `validate_type_references` accepts forward references (a reference to the same or a later record). Microsoft and LLVM do not write them. With forward references, it searches for a cycle with depth and work limits (`ReferenceLimits`, #29).
  - The TPI and IPI hash streams follow one producer form (`types::HashLayout`): Microsoft or LLVM block rules for index offsets, and the Microsoft `Map::save` layout for hash adjusters (#29). `msvc-x64-fastlink-full` (`/DEBUG:FASTLINK`, then `mspdbcmf`) has 28 bytes of TPI hash adjusters, and its rebuild gives the same consumer output (#39).
  - In a PDB with the Info feature `NOTM` (`link /DEBUG:LAZY`, fixture `msvc-x64-lazy`), the TPI and IPI streams have 0 records. The module records are in streams that no directory entry, DBI field or named stream gives; `S_MOD_TYPEREF` names them. `TypeStream::from_records` makes a type stream from such a stream: first index 0x1000, no hash stream (#30, #32). The document does not describe this layout.
  - `types::type_size` (`native/types/size.rs`) gives the byte size of a type: primitive size, the `LF_POINTER` size bits, the numeric size of arrays, `LF_VECTOR`, `LF_MATRIX` and UDTs (a forward reference through `ForwardRefs`), and the size of the referenced type for `LF_MODIFIER`, `LF_ALIAS`, `LF_BITFIELD` and `LF_ENUM`. The symbol validation uses it for subfield def-ranges (`Rule::SubfieldOutsideType`, #30).

### `microsoft-pdb-rs/docs/codeview/types/lf_union.md`
- Documents: `LF_UNION` (0x1506): `count`, `property`, `fields`, `Number length`, `name`, optional `unique_name`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/udt.rs:229-285` (`Union`): u16 count, `ClassOptions` u16, field list index, `Numeric` size, name, unique name if bit 9.
- Conflicts:
  - None found in the layout (matches `cvinfo.h` `lfUnion` and LLVM). pdbrs accepts every numeric leaf for `length` (#29).
- Undocumented assumptions:
  - `LF_UNION2` (0x160a, LLVM `CodeViewTypes.def:58`) is not described. pdbrs decodes it as `WideUnion` (`udt.rs:286-344`): a `u32` property word (low 16 bits are `ClassOptions`), the field list, a numeric member count and a numeric size (LLVM `TypeRecordMapping.cpp`; `cvinfo.h` does not define it). No producer is known; only synthetic tests cover it (#29).
  - The encoder rejects a unique name without the unique-name flag, and the flag without a unique name (`udt.rs:52-67`).
  - `LF_UNION` and `LF_UNION2` accept the MASM end padding and keep it in `producer_padding`, as `lf_class.md` describes (#29).

### `microsoft-pdb-rs/docs/codeview/types/lf_vtshape.md`
- Documents: `LF_VTSHAPE` (0x000a): a `u16` count and 4-bit slot descriptors with values 0-6.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/list.rs:311-397` (`VtableShape`), `attributes.rs:591-609` (`VtableSlotKind` 0-6). Descriptor bytes = ceil(count / 2).
- Conflicts:
  - Document omission vs LLVM `TypeRecordMapping.cpp:514-538` and pdbrs: the document does not state the nibble order. LLVM and pdbrs put the first slot in the high 4 bits of each byte.
  - Document vs pdbrs: values 7-15 are "reserved". pdbrs rejects them (`InvalidFormat`); `cvinfo.h` names 7 `CV_VTS_unused`.
  - Document vs `cvinfo.h`: value names 0 "Near", 1 "Far", 5 "Near32", 6 "Far32" match `CV_VTS_near`..`CV_VTS_far32`. pdbrs uses LLVM names `Near16`, `Far16`, `This` (LLVM's spelling of `CV_VTS_thin`), `Outer`, `Meta`, `Near`, `Far`. Same values, no layout difference.
- Undocumented assumptions:
  - For an odd count, pdbrs requires the unused low nibble of the last byte to be zero (`list.rs:332-348`).

### `microsoft-pdb-rs/docs/codeview/types/lf_skip.md`
- Documents: `LF_SKIP` (0x1200): a record that reserves space; its payload is ignored.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/oem.rs:110-137` (`Skip`): the next valid type index, then the pad data as raw bytes (#29).
- Conflicts:
  - Document vs `cvinfo.h:2026-2030` and pdbrs: `lfSkip` has a `CV_typ_t type` field ("next valid index") before the pad data. The document does not describe this field. pdbrs reads it as `Skip::next_index`.
- Undocumented assumptions:
  - pdbrs does not check the pad data and does not change the indices of the next records. The encoder writes the pad data again without `LF_PAD` padding (`type_record.rs:433-436`).
  - Only very old compilers write `LF_SKIP`; no fixture has it, only synthetic tests (#29).

### `microsoft-pdb-rs/docs/codeview/types/lf_procedure.md`
- Documents: `LF_PROCEDURE` (0x1008): return type, calling convention, a "reserved" byte, parameter count, argument list.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/basic.rs:38-76` (`Procedure`): return type, `CallingConvention` u8, `FunctionOptions` u8, u16 count, argument list index.
- Conflicts:
  - Document vs `cvinfo.h:1775-1782` and pdbrs: the byte after `calling_convention` is `CV_funcattr_t funcattr`, not reserved: bit 0 `cxxreturnudt`, bit 1 `ctor`, bit 2 `ctorvbase`, bits 3-7 unused. pdbrs keeps it as `FunctionOptions` (0x01, 0x02, 0x04).
  - Document vs LLVM `CodeViewDebug.cpp:2128-2171`: the document says static methods use `LF_PROCEDURE`. LLVM writes static member functions as `LF_MFUNCTION` with `this` type 0.
- Undocumented assumptions:
  - An unknown calling convention fails the whole record (`InvalidFormat`); see `calling_convention.md`.
  - `validate_type_references` requires the argument list index to be an `LF_ARGLIST` record (`references.rs:118-121`, #29).

### `microsoft-pdb-rs/docs/codeview/types/lf_precomp.md`
- Documents: `LF_PRECOMP` (0x1509): `start`, `count`, `signature`, `name`, and the precompiled type inclusion rules.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/server.rs:79-113` (`Precompiled`): start index, `u32` count, `u32` signature, NUL-terminated name (#29). The record occurs in object files only (`msvc-x64-pch-z7`, #27); 3 `LF_PRECOMP` records from fixture objects re-encode with 0 differences (#29).
- Conflicts:
  - Document vs `cvinfo.h:1937-1943`: `cvinfo.h` says "length prefixed name"; the document says `strz`. LLVM `TypeRecordMapping.cpp:759-766` reads a NUL-terminated string. The `cvinfo.h` comment is a legacy `_ST` comment.
- Undocumented assumptions:
  - pdbrs does not apply the inclusion rules: it does not look up the precompiled types or change type indices. `Pdb::external_dependencies` groups the modules of a PDB by their `S_OBJNAME` signature, not by `LF_PRECOMP` records (`native/external.rs`, #32).

### `microsoft-pdb-rs/docs/codeview/types/lf_modifier.md`
- Documents: `LF_MODIFIER` (0x1001): the modified type and a `u16` with `const` (bit 0), `volatile` (bit 1), `unaligned` (bit 2); bits 3-15 reserved, must be zero.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/basic.rs:10-36` (`Modifier`), `attributes.rs:136-171` (`ModifierOptions` 0x1, 0x2, 0x4; raw word kept).
- Conflicts:
  - Document vs pdbrs: bits 3-15 "must be zero". pdbrs keeps them in `ModifierOptions::bits` and does not reject them. The encoder writes the raw word again.
  - Document internal errors: "`LD_MODIFIER`" is a typing error. The example applies the modifier to `T_CHAR`; LLVM writes plain `char` as `T_RCHAR` (0x70) (`CodeViewDebug.cpp:1929-1930`).
  - Document omission: `LF_MODIFIER_EX` (0x1518, `cvinfo.h` `lfModifierEx`) is not described. pdbrs decodes it (`type_record/hlsl.rs`, `ModifierEx`: the type, a `u16` count and `u16` `CV_modifier_e` values kept raw); fxc fixtures have the values 0x1 and 0x200 (#27, #29).
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_pointer.md`
- Documents: `LF_POINTER` (0x1002): referent type, `u32` attribute bit fields, `ptrtype` and `ptrmode` values, and variant data for based and member pointers.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/pointer.rs` (`Pointer`, `MemberPointer`, `BasedPointer`), `attributes.rs:289-477`. Kind bits 0-4 (`PointerKind` 0x00-0x0c), mode bits 5-7 (`PointerMode` 0-4), flags 0x100 flat32, 0x200 volatile, 0x400 const, 0x800 unaligned, 0x1000 restrict, size bits 13-18, 0x80000 MoCOM, 0x100000 `&this`, 0x200000 `&&this`; bits 22-31 kept raw. For modes 2 and 3 pdbrs reads containing class + `u16` `MemberPointerRepresentation` 0-8. For based kinds 0x03-0x09 pdbrs decodes the base data (`BasedPointer`, `pointer.rs:29-123`, #29): `BASE_SEG` a `u16` segment; `BASE_VAL`, `BASE_SEGVAL`, `BASE_ADDR`, `BASE_SEGADDR` one complete symbol record with its length field; `BASE_TYPE` a type index and a name; `BASE_SELF` no data.
- Conflicts:
  - Document vs `cvinfo.h:1035-1045`, LLVM and pdbrs: the `format` table lists 14 values (0-13) from the old CV4 16:16/16:32 scheme. `CV_pmtype_e` defines 0 `Undef`, 1-4 data (single, multiple, virtual, general), 5-8 function (single, multiple, virtual, general). pdbrs rejects 9-13 (`InvalidFormat`).
  - Document vs `cvinfo.h:1536-1547` and pdbrs: "pointer based on a type: the variant data is a single `TypeIndex`". `cvinfo.h` `btype` has an index and a name, and pdbrs reads both. The document omits the data of `BASE_SEG` (`u16 bseg`) and `BASE_VAL`/`BASE_SEGVAL`/`BASE_ADDR`/`BASE_SEGADDR` (a copy of the base symbol record); pdbrs decodes them.
  - Document vs LLVM and pdbrs: "The data after the `Pointer` structure depends on `ptrtype`". Member-pointer data depends on `ptrmode` (2 or 3) (LLVM `TypeRecordMapping.cpp:389`); based data depends on `ptrtype`. pdbrs reads no based data for a member-pointer mode.
  - Document vs pdbrs: the document lists 0x0d `UNUSEDPTR` as a value; pdbrs rejects kinds ≥ 0x0d and modes ≥ 5 (`InvalidFormat`).
  - Document internal errors: the `ptrtype` row says "Specifies the mode"; "Ttrue"; field name `lsrref` for `isrref`. The struct name `PointerToDataMemberVariant` also covers pointers to member functions. A `TODO` ("convert these") ends the document.
- Undocumented assumptions:
  - The based-pointer layout comes from `cvinfo.h` `lfPointer::pbase`; LLVM does not decode it and no fixture has it, so only synthetic tests cover it (#2, https://github.com/napbat/pdbrs/issues/2#issuecomment-6407739492, #29). pdbrs keeps the base symbol record as bytes (`BasedPointer::Symbol`); `symbol_record()` frames it.
  - The encoder rejects member or based data that does not match the kind and the mode (`pointer.rs:168-191`).
  - `ml64` and `ml` 14.51 write 6 zero bytes after the attribute word of each `LF_POINTER` of a `TYPEDEF PTR`, then the `LF_PAD` run. `Pointer::trailing_zeros` keeps the count, and the encoder writes the bytes again; other bytes there stay `InvalidFormat` (`masm-x64-debug`, `masm-x86-debug`, #29).

### `microsoft-pdb-rs/docs/codeview/types/lf_mfunction.md`
- Documents: `LF_MFUNCTION` (0x1009): return, class and `this` types, calling convention, a "reserved" byte, parameter count, argument list, `uint32_t this_adjust`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/basic.rs:78-128` (`MemberFunction`): three type indices, `CallingConvention`, `FunctionOptions`, u16 count, argument list, `i32 this_adjustment`.
- Conflicts:
  - Document vs `cvinfo.h:1801-1811` and pdbrs: the byte after `calling_convention` is `CV_funcattr_t`, not reserved.
  - Document vs `cvinfo.h`, LLVM and pdbrs: `this_adjust` is `uint32_t` in the document; `cvinfo.h` declares `long thisadjust`, LLVM `int32_t ThisPointerAdjustment`, pdbrs `i32`.
  - Document vs LLVM `CodeViewDebug.cpp:2146-2170`: the document says the record is for "non-static instance" methods. LLVM writes static methods as `LF_MFUNCTION` with `this` = 0; pdbrs documents `TypeIndex::NONE` for a static function. `lf_mfunc_id.md` and `lf_procedure.md` disagree with this document too.
  - Document omission: a `TODO` on `this_adjust`.
- Undocumented assumptions:
  - `validate_type_references` requires the argument list index to be an `LF_ARGLIST` record. It accepts any type for the class and `this` types (`references.rs:122-127`, #29).

### `microsoft-pdb-rs/docs/codeview/types/lf_methodlist.md`
- Documents: `LF_METHODLIST` (0x1206): entries of `u16 attribute`, `u16 padding`, method type, optional `uint32_t vtab_offset` for `mprop` 4 or 6; the `CV_fldattr_t` bits.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/list.rs:164-309` (`MethodList`, `MethodListEntry`), `attributes.rs:479-589` (`MemberAttributes`: access bits 0-1, `MethodKind` bits 2-4, flags 0x20-0x200). Padding must be 0; `vtable_offset: Option<i32>` for kinds 4 and 6.
- Conflicts:
  - Document and `cvinfo.h` (`mlMethod`, `unsigned long vbaseoff`) vs LLVM and pdbrs: the offset is unsigned in the document and `cvinfo.h`, but signed `i32` in pdbrs (LLVM `int32_t VFTableOffset`). Same bytes.
  - Document vs pdbrs: `mprop` 7 is not listed. pdbrs rejects it (`InvalidFormat`).
  - Document internal: the Rust example (`MethodList::parse`, `entry.ty`) is the `ms-codeview` API, not a format rule. `noinherit`/`noconstruct` say "Method"; `cvinfo.h` says "class".
- Undocumented assumptions:
  - pdbrs ends the list when fewer than 8 bytes remain and requires those bytes to be valid `LF_PAD` padding (`list.rs:266-292`).
  - `validate_type_references` requires each method type to be an `LF_MFUNCTION` record (`ReferenceRole::MethodType`, #29).

### `microsoft-pdb-rs/docs/codeview/types/lf_fieldlist.md`
- Documents: `LF_FIELDLIST` (0x1203) and its member records (`LF_BCLASS`, `LF_VBCLASS`, `LF_IVBCLASS`, `LF_INDEX`, `LF_VFUNCTAB`, `LF_VFUNCOFF`, `LF_ENUMERATE`, `LF_FRIENDFCN`, `LF_MEMBER`, `LF_STMEMBER`, `LF_METHOD`, `LF_NESTTYPE`, `LF_ONEMETHOD`, `LF_NESTTYPEEX`, `LF_FRIENDCLS`) and the `CV_fldattr_t` bits.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/field_list.rs:391-499` decodes every member of the document and also `LF_MEMBERMODIFY` (0x1513) and `LF_BINTERFACE` (0x151a) (#29). Numeric fields are `Numeric`. `_ST` and `_16t` member kinds have no decoder; an unknown member kind makes the whole `LF_FIELDLIST` fail (`UnsupportedVersion`), because members have no length. `LF_INDEX` must be the last member (`InvalidFormat` otherwise, `field_list.rs:322-336`). `encode_field_list` (`field_list/encode.rs`) writes a long member list as a chain of records.
- Conflicts:
  - Document vs `cvinfo.h:868-880`, LLVM and pdbrs: `LF_FRIENDCLS` is 0x140a. The document gives 0x040b, which is `LF_FRIENDCLS_16t`.
  - Document vs `cvinfo.h:2615-2618`: `LF_VFUNCTAB` `type` is "type index of pointer" (an `LF_POINTER` to the `LF_VTSHAPE`). The document says it points to the shape and writes `LF_VFTSHAPE`.
  - Document vs `cvinfo.h:2631-2636` and pdbrs: `LF_VFUNCOFF` `offset` is `CV_off32_t` (signed); the document says `uint32_t`. pdbrs reads `i32`.
  - Document vs LLVM and pdbrs: `LF_ONEMETHOD` `vbaseoff` is `uint32_t` in the document and `unsigned long` in `cvinfo.h`; LLVM and pdbrs read `i32`.
  - Document vs `cvinfo.h`: names `LF_NESTEDTYPE` and `LF_NESTEDTYPEX` are `LF_NESTTYPE` and `LF_NESTTYPEEX`.
  - Document omissions vs `cvinfo.h`/LLVM: no `LF_MEMBERMODIFY` (0x1513), `LF_BINTERFACE` (0x151a), `LF_MANAGED`, or `_ST`/`_16t` member kinds. The document does not describe `LF_PAD` bytes between members.
  - Document vs pdbrs: the document says the `LF_FRIENDFCN`, `LF_INDEX`, `LF_NESTTYPE`, `LF_VFUNCTAB`, `LF_FRIENDCLS` padding "must be zero". pdbrs enforces it for these kinds and for `LF_VFUNCOFF` (`field_list.rs:377-384`).
- Undocumented assumptions:
  - `LF_PAD` after a member: the low 4 bits of the first byte give the count; each next byte counts down to 0xf1; a lone 0xf0 is invalid (`field_list.rs:354-375`, LLVM `CodeViewRecordIO::skipPadding`).
  - `LF_MEMBERMODIFY` is attributes, base type and name; `LF_BINTERFACE` uses the `LF_BCLASS` layout (LLVM `TypeRecordMapping.cpp`). No fixture has them, or the friend members; only synthetic tests cover them (#29).
  - pdbrs rejects `mprop` 7 in any attribute word.
  - Split of a long field list: the encoder follows LLVM `ContinuationRecordBuilder`. The first record has the first members and the highest type index; each record ends with an `LF_INDEX` to the next part. The record size limit is a parameter: `LLVM_FIELD_LIST_LIMIT` (`0xff00`). The MSVC fixtures match `0xfb00`; Microsoft does not document this value (inferred, #29, https://github.com/napbat/pdbrs/issues/29#issuecomment-6413170447). With these limits, all 20 field-list chains of the fixtures split again to the original record sizes.
  - `validate_type_references` requires an `LF_INDEX` target to be an `LF_FIELDLIST` record (`ReferenceRole::Continuation`, #29).

### `microsoft-pdb-rs/docs/codeview/types/lf_enum.md`
- Documents: `LF_ENUM` (0x1507): `count`, `property`, underlying type, field list, `name`, optional `unique_name`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/udt.rs:346-400` (`Enum`).
- Conflicts:
  - Document vs `cvinfo.h:1752-1759`: `cvinfo.h` comments the name as "length prefixed"; the document, LLVM and pdbrs use a NUL-terminated name. Legacy comment, no real difference.
  - None found against pdbrs.
- Undocumented assumptions:
  - pdbrs does not check that the underlying type is primitive or that the field list holds only `LF_ENUMERATE`. `validate_type_references` requires the field list to be 0 or an `LF_FIELDLIST` record and accepts any type for the underlying type (`references.rs:161-164`, #29).
  - `LF_ENUM` accepts the MASM end padding and keeps it in `producer_padding`, as `lf_class.md` describes (#29).

### `microsoft-pdb-rs/docs/codeview/types/lf_endprecomp.md`
- Documents: `LF_ENDPRECOMP` (0x0014): a `u32` signature that ends the precompiled types of a creator object.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/server.rs:115-121` (`TypeLeaf::EndPrecompiled(u32)`, #29). The record occurs in object files only; 1 `LF_ENDPRECOMP` from the fixture objects re-encodes with 0 differences (#27, #29).
- Conflicts:
  - Document internal: the file ends with a section "Records referenced from other type records" that belongs in `types.md`.
  - None found in the layout (matches `cvinfo.h:1951-1954` and LLVM `TypeRecordMapping.cpp:768-772`).
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_class.md`
- Documents: `LF_CLASS` (0x1504), `LF_STRUCTURE` (0x1505), `LF_INTERFACE` (0x1519): count, `property` bits 0-15, field list, derivation list, vshape, `Number length`, name, optional unique name; `CV_HFA` and `CV_MOCOM_UDT` values.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/udt.rs:75-153` (`Class`, `ClassKind`), `attributes.rs:15-134` (`ClassOptions`: 0x1 packed … 0x400 sealed, HFA bits 11-12, 0x2000 intrinsic, MoCOM bits 14-15). The size is `Numeric`. Unique name read only if bit 9 is set.
- Conflicts:
  - Document vs `cvinfo.h:1120-1135`: bit names `overops` and `nested` are `ovlops` and `isnested` in `cvinfo.h`. Same bits.
  - Document omission: `LF_CLASS2` 0x1608, `LF_STRUCTURE2` 0x1609, `LF_INTERFACE2` 0x160b (LLVM `CodeViewTypes.def:54-56`) are not described. pdbrs decodes them as `WideClass` (`udt.rs:155-227`, #29).
- Undocumented assumptions:
  - pdbrs does not check that `derivation_list` is zero. `validate_type_references` requires the field list to be 0 or an `LF_FIELDLIST` record and the vshape to be 0 or an `LF_VTSHAPE` record (`references.rs:149-158`, #29).
  - `LF_CLASS2`, `LF_STRUCTURE2` and `LF_INTERFACE2`: a `u32` property word (low 16 bits are `ClassOptions`), field list, derivation list, vshape, a numeric member count and a numeric size (LLVM `TypeRecordMapping.cpp`; `cvinfo.h` does not define them). No producer is known; only synthetic tests cover them (#29).
  - MASM end padding: `ml64` 14.50 writes `f1` and two stale bytes after a name of odd length (with the NUL) in `LF_STRUCTURE` records of the static CRT (`handlers.obj` in `libvcruntime(d).lib`, for example `00 f1 6d 65` after `CsFrameEncoded`). `link.exe` and `lld-link` copy the record without a change. LLVM (`CodeViewRecordIO::endRecord`), DIA2Dump and `pdbtool` accept it. The UDT decoders (`Class`, `WideClass`, `Union`, `WideUnion`, `Enum`) accept 1 to 3 bytes that start with 0xf1-0xf3 and keep them in `producer_padding` (`udt.rs:43-100`); `TypeLeaf::producer_padding` returns them, and the encoder writes them again. Other records keep the strict check. Fixture `clang-lld-x64-static` (`/MTd`) has these records; `tests/type_record_producer.rs` uses the library bytes (#29).

### `microsoft-pdb-rs/docs/codeview/types/lf_bitfield.md`
- Documents: `LF_BITFIELD` (0x1205): type, `u8 length`, `u8 position`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/basic.rs:130-160` (`Bitfield`).
- Conflicts: None found (matches `cvinfo.h:2164-2168` and LLVM).
- Undocumented assumptions:
  - pdbrs does not check that `position + length` fits in the underlying type.

### `microsoft-pdb-rs/docs/codeview/types/lf_array.md`
- Documents: `LF_ARRAY` (0x1503): element type, index type, `Number length`, name.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/basic.rs:162-196` (`Array`). The size is `Numeric`.
- Conflicts:
  - Document vs `types.md`: this document gives 0x1503 (correct); `types.md` gives 0x1502.
  - Document omissions: a `TODO` on `index_type`; `LF_STRIDED_ARRAY` (0x1516) is not described. pdbrs decodes it (`type_record/array.rs:262-300`, `StridedArray`: element type, index type, `u32` stride, numeric size, name; `cvinfo.h` `lfStridedArray`). The fxc fixtures have it with stride 1 (#27, #29).
- Undocumented assumptions:
  - pdbrs does not check the index type. `validate_type_references` accepts any type for the element and index types (#29).

### `microsoft-pdb-rs/docs/codeview/types/lf_arglist.md`
- Documents: `LF_ARGLIST` (0x1201): a `u32` count and that many type indices; an ownership invariant.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/list.rs:106-162` (`ArgumentList`).
- Conflicts:
  - Document vs observed linker behavior: "Each `LF_ARGLIST` record is pointed-to by exactly one `LF_PROCEDURE` or `LF_MFUNCTION`". Inference, not tested: Linkers merge identical records, so two procedure types with the same parameters but different return types share one `LF_ARGLIST`. pdbrs does not check the invariant.
- Undocumented assumptions:
  - The encoder rejects a list with more than `u32::MAX` entries (`LimitExceeded`, `list.rs:21-25`).


## `items/`

### `microsoft-pdb-rs/docs/codeview/items/items.md`
- Documents: IPI item records, the `ItemId` index space and lookup, and the kind summary 0x1601-0x1607.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/id_record.rs` (`IdRecord::decode` for all 7 kinds, `id_record.rs:84-142`; other kinds `Unsupported`), `id_record/encode.rs` (`IdRecord::encode`, #29), `id_record/table.rs` (`IdTable`: O(1) lookup through `TypeTable`, `resolve_string`, `validate`). `codeview/index.rs` `IdIndex`, 0 = none. `types::write_type_stream` writes the IPI stream and its hash stream (#29).
- Conflicts:
  - Document internal: the link `./lf_func_id.md.md` is broken. The table lists 0x1607 before 0x1606.
  - Document vs pdbrs: the document describes a linear scan to find a record; pdbrs builds an offset table (`native/types.rs` `TypeTable`, 4 bytes per record). No format difference.
- Undocumented assumptions:
  - `IdTable::validate` checks each type index against the primitive table or TPI `index_end`, and each ID reference against the IPI range and the expected kind (`table.rs:162-258`). It does not check the backward-reference rule that `lf_buildinfo.md` states. An IPI record of an unknown kind is `UnsupportedVersion` there.
  - IPI hash values: `LF_UDT_SRC_LINE` and `LF_UDT_MOD_SRC_LINE` hash the 4 bytes of their UDT index with `hash_string_v1`; the other ID records use `hash_buffer_v8` of the record bytes (`native/types/hash.rs:591-647`, #29).

### `microsoft-pdb-rs/docs/codeview/items/lf_buildinfo.md`
- Documents: `LF_BUILDINFO` (0x1603): five `ItemId` arguments (cwd, tool, source, PDB, args), truncation rules, and examples.
- pdbrs: implemented — `id_record.rs:111-116,178-228` (`BuildInfo`, `BuildInfoArgument` positions 0-4): a `u16` count, then `count` ID indices. `table.rs:207-213` accepts 0, `LF_STRING_ID` or `LF_SUBSTR_LIST` per argument.
- Conflicts:
  - Document vs `cvinfo.h:1726-1730`, LLVM and pdbrs: the C struct has no count. `cvinfo.h` has `unsigned short count` before `arg[]`; the document's own example text calls it `num_strings` = 5. pdbrs reads the `u16` count.
  - Document vs pdbrs: "this record can be truncated after any field". pdbrs requires exactly `count` IDs; a record shorter than its count is `InvalidFormat`. A record with fewer than 5 IDs (smaller `count`) is valid in pdbrs.
  - Document internal: the first example names the string values but shows no IPI offsets for them; the anchor `../symbols/symbols.md#s_buildinfo-0x114c---build-info` depends on heading text.
  - Document omissions: a `TODO` on response files.
- Undocumented assumptions:
  - pdbrs does not check that referenced IDs are below the record's own ID (the document's backward-reference rule).

### `microsoft-pdb-rs/docs/codeview/items/lf_func_id.md`
- Documents: `LF_FUNC_ID` (0x1601): scope `ItemId`, function type, name, optional `u64` decorated name hash, the padding rule and examples.
- pdbrs: implemented — `id_record.rs:99-104,144-160,364-371` (`FunctionId`): scope 0 → `None`; hash present if ≥ 8 bytes follow the name; then 0-3 `LF_PAD` bytes. `table.rs:197-202` requires the scope to be an `LF_STRING_ID`. The encoder writes the hash only if the decoded record has it (`id_record/encode.rs`).
- Conflicts:
  - Document internal error: example 1 says `type` is 0x1046; the bytes `80 10 00 00` give 0x1080.
  - Document internal error: example 2 says `scope` is 0x0610; the bytes `06 10 00 00` give 0x1006.
  - Document internal error: the text says the scope points to an "`LF_STRING` record"; the kind is `LF_STRING_ID`. It says "symbol records are required to have a size that is a multiple of 4"; this is an IPI record.
  - Document internal: "It is not understood why some records have alignment padding". The third example shows the reason: the preceding `LF_UDT_MOD_SRC_LINE` record (offset 0x78) is 18 bytes with no padding, so later records start at non-4-aligned offsets (0x8a) but each record still pads its own length to a multiple of 4.
  - Document omission: a `TODO` on the hash function.
- Undocumented assumptions:
  - The hash rule follows `microsoft/pdb-rs` `records.rs` (#4, https://github.com/napbat/pdbrs/issues/4#issuecomment-6407539418). pdbrs does not compute or verify the hash.

### `microsoft-pdb-rs/docs/codeview/items/lf_mfunc_id.md`
- Documents: `LF_MFUNC_ID` (0x1602): parent type, function type, name, optional decorated name hash, special method names, examples.
- pdbrs: implemented — `id_record.rs:105-110,162-176` (`MemberFunctionId`), same hash and padding rule as `LF_FUNC_ID`. `table.rs:203-206` checks both type indices against the primitive table and the TPI range only.
- Conflicts:
  - Document vs pdbrs: the parent type "should be" `LF_STRUCTURE`, `LF_CLASS`, `LF_UNION` or `LF_ENUM`. pdbrs does not check the parent kind.
  - None found in the layout (matches `cvinfo.h:1687-1692`).
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/items/lf_string_id.md`
- Documents: `LF_STRING_ID` (0x1605): a substring list `ItemId` (0 = none) and a NUL-terminated string.
- pdbrs: implemented — `id_record.rs:121-124,230-241` (`StringId`); `table.rs:98-160` (`resolve_string`).
- Conflicts:
  - Document omission vs pdbrs: "this string is concatenated with the substrings" does not give the order. pdbrs writes the substring-list parts first, then the record's own string; the `lf_buildinfo.md` example (args at 0x10fc, list at 0x10fb) shows this order.
- Undocumented assumptions:
  - pdbrs allows nested substring lists up to `MAX_STRING_DEPTH` = 8 and caps visits at the record count (#4, https://github.com/napbat/pdbrs/issues/4#issuecomment-6407539418).
  - `validate` requires a non-zero substring reference to be an `LF_SUBSTR_LIST` (`table.rs:217-219`).

### `microsoft-pdb-rs/docs/codeview/items/lf_substr_list.md`
- Documents: `LF_SUBSTR_LIST` (0x1604): a list of `LF_STRING_ID` IDs to concatenate, with an example.
- pdbrs: implemented — `id_record.rs:117-120` (`IdList`): a `u32` count, then `count` IDs. `table.rs:214-216` requires each entry to be an `LF_STRING_ID`.
- Conflicts:
  - Document vs `cvinfo.h:2043-2046` (`lfArgList` is used for `LF_SUBSTR_LIST`), LLVM and pdbrs: the C struct has no count, but the record starts with a `u32` count. The document's example starts with `09 00 00 00`.
  - Document internal: the `TODO` names `LF_SUBSTR_ID`, which does not exist; it means `LF_SUBSTR_LIST`.
- Undocumented assumptions:
  - pdbrs supports nesting through `LF_STRING_ID` substring references up to depth 8; the document's `TODO` leaves the question open.

### `microsoft-pdb-rs/docs/codeview/items/lf_udt_src_line.md`
- Documents: `LF_UDT_SRC_LINE` (0x1606): UDT type, source file, line.
- pdbrs: implemented — `id_record.rs:125-129,243-252` (`UdtSourceLine`: `udt`, `source_file: IdIndex`, `line`). `table.rs:220-223` requires `source_file` to be an `LF_STRING_ID`.
- Conflicts:
  - Document vs `cvinfo.h:1700-1705` and pdbrs: the document says `file_name` is a `NameIndex` into the `/names` stream. `cvinfo.h` declares `CV_ItemId src` ("index to LF_STRING_ID record"); pdbrs reads an IPI ID.
- Undocumented assumptions:
  - No fixture contains this record; only synthetic tests cover it (#4, #29).
  - The IPI hash of the record is `hash_string_v1` of the 4 UDT index bytes (`types/hash.rs:591-605`, #29).

### `microsoft-pdb-rs/docs/codeview/items/lf_udt_mod_src_line.md`
- Documents: `LF_UDT_MOD_SRC_LINE` (0x1607): UDT type, `/names` file name offset, line, `u16` module index.
- pdbrs: implemented — `id_record.rs:130-136,254-270` (`UdtModuleSourceLine`: `name_offset` u32, `line`, `module` u16 raw, `padded`). `table.rs:224` checks only the UDT index; pdbrs does not check the `/names` offset here.
- Conflicts:
  - Document internal error: "the same as `LF_MOD_SRC_LINE`"; the record it extends is `LF_UDT_SRC_LINE`, and the two differ in more than `module_index`: `LF_UDT_SRC_LINE` uses an IPI ID for the file, this record uses a `/names` offset (`cvinfo.h:1707-1713`).
  - Document omissions: two `TODO`/Determinism blocks on `module_index`; the document does not say whether the module index is 0-based or 1-based. pdbrs keeps the raw value.
- Undocumented assumptions:
  - The record has 14 payload bytes. Microsoft `link.exe` writes it without end padding (18 bytes), so later IPI records start at non-4-aligned offsets (see `lf_func_id.md`). pdbrs accepts 0 padding bytes. `UdtModuleSourceLine::padded` keeps the form, so the encoder writes the same bytes (#29, https://github.com/napbat/pdbrs/issues/29#issuecomment-6413170447).
  - The IPI hash of the record is `hash_string_v1` of the 4 UDT index bytes (`types/hash.rs:591-605`, #29).
