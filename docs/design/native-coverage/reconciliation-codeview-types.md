# Native coverage: reconciliation of the Microsoft pdb-rs CodeView type, item and line documents

This page is part of the [native coverage](../native-coverage.md) accounting. It covers `docs/reference/microsoft-pdb-rs/docs/codeview/` (top level, `types/` and `items/`).
Each section names one imported document. Paths are relative to `docs/reference/`. The fields are:

- **Documents**: the format items that the document describes.
- **pdbrs**: `implemented` (pdbrs reads every item that the document describes), `partial`, `not implemented` or `not applicable` (no format content), with the source files.
- **Conflicts**: each difference between the document and pdbrs, or between the document and another source (`cvinfo.h`, LLVM source, `ms-pdb`, other documents). Errors inside the document are conflicts too.
- **Undocumented assumptions**: layouts and rules that pdbrs uses but the document does not state, with the issue that records the evidence.

The comparison used `main` at `8b77fb2`, `cvinfo.h` at `805655a2`, LLVM at `b9f2235f` and the `ms-codeview` 0.1.7, `ms-pdb` 0.1.22, `ms-pdb-msf` 0.1.7 and `ms-pdb-msfz` 0.1.10 crates. "Inference, not tested" marks a statement that comes from reading source code only.

Common pdbrs facts used below:
- Record framing: `crates/pdbrs/src/native/codeview.rs` (`Records`). The `u16` length counts the kind and the payload. Length < 2 is `InvalidFormat`. pdbrs does not require an even length or an aligned record start.
- End padding of TPI and IPI records: `Fields::finish_type_padding` (`codeview.rs:277-290`) accepts 0 to 3 bytes `f3 f2 f1` only. Other trailing bytes are `InvalidFormat`.
- Names: NUL-terminated bytes (`Fields::name`), not decoded as text.
- Numeric leaves in type records: `Integer::parse_prefix` (`codeview/numeric.rs`). Non-integer leaves are `UnsupportedVersion`.


## Top level

### `microsoft-pdb-rs/docs/codeview/primitive_types.md`
- Documents: The bit layout of primitive type indices (size, type, mode), the `TY_*` type families, the `MODE_*` pointer modes and tables of `T_*` values.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/index.rs` decodes an index below `FIRST_RECORD_INDEX` (0x1000) as `PrimitiveKind` (whole low byte, bits 0-7, 57 values) and `PrimitiveMode` (byte 1, bits 8-11, values 0-7). An unknown kind or mode is `InvalidFormat`. pdbrs does not split the kind into type and size nibbles and gives no byte size. `native/types.rs:359-362` rejects a TPI/IPI `index_begin` below 0x1000.
- Conflicts:
  - `cvinfo.h` vs pdbrs: `cvinfo.h` defines kind `0xf0` with pointer modes only (`T_NCVPTR` `0x01f0`, `T_32NCVPTR` `0x04f0`, `T_64NCVPTR` `0x06f0`). `PrimitiveKind` has no value for it, so `TypeIndex::primitive` returns `InvalidFormat` for these valid indices (#29).
  - Document vs `cvinfo.h`: the document says bit 3 is reserved and "always zero" and `size` is bits 0-2. `cvinfo.h` defines `CV_SMASK 0x00f` (subtype is bits 0-3). Real values set bit 3: `T_INT1` 0x68, `T_UINT1` 0x69, `T_HRESULT` 0x08, `T_INT16` 0x78, `T_UINT16` 0x79, `T_CHAR16` 0x7a, `T_CHAR32` 0x7b. pdbrs decodes all of them.
  - Document vs `cvinfo.h` (`CV_prmode_e`): the mode table stops at 6. `cvinfo.h` defines `CV_TM_NPTR128 = 7`. pdbrs decodes 7 as `NearPointer128`.
  - Document vs `cvinfo.h` (`CV_integral_e`, `CV_real_e`, `CV_special_e`, `CV_special2_e`): the size table marks these values "reserved" or "obsolete", but `cvinfo.h` defines them: `CV_IN_16BYTE = 4` (`T_OCT` 0x14, `T_UOCT` 0x24); `CV_RC_REAL32PP = 5`, `CV_RC_REAL16 = 6` (0x45, 0x46); `CV_SP_CURRENCY`..`CV_SP_NOTTRANS` = 4-7 and `CV_SP_HRESULT = 8`; `CV_S2_BOOL32FF = 2` (0x62); `CV_INT` subtypes 8-0xb. pdbrs decodes all of these.
  - Document vs LLVM `TypeIndex.h`: the tables omit kinds that LLVM and pdbrs decode and the pinned `cvinfo.h` does not define: `Boolean128` 0x34, `Complex48` 0x54, `Complex32PartialPrecision` 0x55, `Complex16` 0x56, `Character8` (`char8_t`) 0x7c.
  - Document vs LLVM `CodeViewDebug.cpp:1880-1930`: the table maps C/C++ `char` to `T_CHAR` (0x10) and gives no index for `int`. LLVM writes plain `char` as `NarrowCharacter` (`T_RCHAR` 0x70) and 4-byte `int` as `Int32` (`T_INT4` 0x74); `T_CHAR` is `signed char`. The `T_RCHAR` row has no size and asks "is this `__int8`?"; `__int8` is `T_INT1` 0x68, which the document omits.
  - Document vs pdbrs: the document says every index below `type_index_begin` is primitive. pdbrs `TypeIndex::is_primitive` uses the fixed bound 0x1000, not the stream header value.
  - Document errors: a `TODO` remains ("actually delete obsolete variants"). The 48-bit to 80-bit real and complex sections put the description in the "C/C++ Type" column. The text "thse" and "can taken" are typing errors.
- Undocumented assumptions:
  - pdbrs accepts mode bits for every kind, for example 0x0600 (`T_NOTYPE` with mode 6). `cvinfo.h` defines no such value.
  - pdbrs rejects bit 11 set (mode byte 8-15) as `InvalidFormat`. This agrees with the document's reserved bit 11 but the document does not say that a reader must reject it.

### `microsoft-pdb-rs/docs/codeview/number.md`
- Documents: The numeric leaf (`Number`): immediate values below 0x8000 and leaf tags 0x8000-0x801b with their payload sizes, `LF_VARSTRING`, and examples.
- pdbrs: partial — `crates/pdbrs/src/native/codeview/numeric.rs` decodes immediates and `LF_CHAR`, `LF_SHORT`, `LF_USHORT`, `LF_LONG`, `LF_ULONG`, `LF_QUADWORD`, `LF_UQUADWORD`, `LF_OCTWORD`, `LF_UOCTWORD` into `Integer::{Signed,Unsigned}` (i128/u128). Every other tag is `UnsupportedVersion`. Type records (`LF_ARRAY`, `LF_CLASS`, `LF_UNION`, field members) and `S_CONSTANT` (`codeview/symbol/variable.rs`) use only this decoder. `S_MANCONSTANT` (`codeview/symbol/managed.rs:233-280`) also reads `LF_VARSTRING` (u16 length + bytes) and keeps fixed-size leaves raw: `LF_REAL16`=2, `LF_REAL32`=4, `LF_REAL48`=6, `LF_REAL64`/`LF_COMPLEX32`/`LF_DATE`=8, `LF_REAL80`=10, `LF_REAL128`/`LF_COMPLEX64`/`LF_DECIMAL`=16, `LF_COMPLEX80`=20, `LF_COMPLEX128`=32. Not implemented: `LF_UTF8STRING` (0x801b) in all contexts; real, complex, decimal and date values outside managed constants; typed conversion of floating-point values.
- Conflicts:
  - Document internal error: example row `04 80 fe ff ff ff | LF_LONG | -2` uses leaf 0x8004, which is `LF_ULONG`. `LF_LONG` -2 is `03 80 fe ff ff ff`.
  - Document internal error: example `10 80 04 00 44 45 46` declares length 4 but has 3 string bytes for "DEF". The length must be `03 00`.
  - Document vs `cvinfo.h`, LLVM `CodeViewTypes.def` and `ms-codeview` `number.rs`: the table omits `LF_REAL16` = 0x801c (2 bytes). pdbrs reads it only in managed constants.
  - Document vs pdbrs: the document lists `LF_UTF8STRING` (NUL-terminated). `ms-codeview` decodes it. pdbrs rejects it everywhere (`UnsupportedVersion`).
  - Document vs pdbrs: the document presents all leaves as valid `Number` values. pdbrs rejects non-integer leaves in type records and in `S_CONSTANT`; a record with such a leaf fails to decode.
- Undocumented assumptions:
  - pdbrs reports immediate values as `Unsigned` and keeps signedness for each leaf; the original width is not kept (`numeric.rs:14-17`).
  - `S_MANCONSTANT` uses the `cvinfo.h` `lfReal*`/`lfCmplx*`/`lfDecimal`/`lfDate` sizes for raw fixed leaves (#14).

### `microsoft-pdb-rs/docs/codeview/line_data.md`
- Documents: The C13 line region of a module stream: subsection framing and alignment, the `DEBUG_S_*` kind list 0xf1-0x100 and the ignore bit, the `DEBUG_S_LINES` header, blocks, line and column entries, and the `DEBUG_S_FILECHKSMS` entries with checksum kinds.
- pdbrs: partial — framing in `crates/pdbrs/src/native/dbi/module.rs` (`ModuleStream::c13_subsections`: 8-byte header, data, padding to 4 bytes must be present). `crates/pdbrs/src/native/lines.rs` classifies every kind 0xf1-0x100, `Ignored(raw)` for bit 31 and `Unknown(raw)`. Decoded: 0xf2 (`lines/blocks.rs`), 0xf4 (`lines/checksums.rs`), 0xf6 (`lines/inlinee.rs`), 0xfd (`lines/coff_symbol_rva.rs`). Raw only (`LineSubsection::Unsupported`): 0xf1, 0xf3, 0xf5, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfe, 0xff, 0x100, all ignored and unknown kinds. `lines/module.rs` (`ModuleLines`) rejects a second 0xf4 subsection and `validate()` requires each block and inlinee file reference to be the start of a checksum entry.
- Conflicts:
  - Document vs pdbrs: the document says "Decoders should always use `checksum_size` … rather than making assumptions … based on `checksum_kind`". pdbrs rejects a size that differs from the kind size (`InvalidFormat`) and an unknown kind > 3 (`UnsupportedVersion`) (`checksums.rs:224-239`).
  - Document vs pdbrs: invariant "`num_lines` is greater than zero". pdbrs accepts a block with 0 lines.
  - Document vs pdbrs: invariant "Block offset sorted". pdbrs checks a stricter rule: line offsets do not decrease inside a block and across blocks, and each line offset is below `contribution_size` (`blocks.rs:125-135`).
  - Document vs pdbrs: padding after a checksum entry is "zero" only as a Determinism note. pdbrs requires zero padding (`InvalidFormat` otherwise) and accepts absent padding after the last entry.
  - Document vs pinned `cvinfo.h` and LLVM: the table lists `DEBUG_S_INLINEE_MDTOKEN_MAP` = 0xfe, `DEBUG_S_XFGHASH_TYPE` = 0xff and `DEBUG_S_XFGHASH_VIRTUAL` = 0x100. The pinned `cvinfo.h` `DEBUG_S_SUBSECTION_TYPE` ends at 0xfd. LLVM `DebugSubsectionKind` has 0xff and 0x100 but not 0xfe. `ms-pdb` `lines.rs` ends at 0xfd. 0xfe has no source other than this document; pdbrs takes the name from it.
  - Document internal: the headings and text use `DEBUG_S_FILE_CHECKSUMS`; the kind table and `cvinfo.h` use `DEBUG_S_FILECHKSMS`.
  - Document internal: the link `../pdb/dbi_streams.md#dbi-modules-substream` is broken (the files are `pdb/dbi_stream.md` and `pdb/dbi_modules.md`). The link `../pdb/dbi_stream.md#dbi-sources-substream` points to the wrong file (`pdb/dbi_sources.md` exists).
  - Document omissions: no layout for 0xf6 and 0xfd, which pdbrs decodes; no value for the `HAVE_COLUMNS` flag (`cvinfo.h` `CV_LINES_HAVE_COLUMNS 0x0001`); a `TODO` on `file_offset`; the section "Correlation of `FileChecksum` and DBI Sources Substream" has no content; the example refers to highlights that a text document cannot show.
- Undocumented assumptions:
  - 0xfd `DEBUG_S_COFF_SYMBOL_RVA`: a header-less `u32` array, size multiple of 4 (LLVM `DebugSymbolRVASubsection.h`). Entry `i` is the image RVA of COFF symbol record `i`; 0 for no address; 0xffffffff for an external resolved to an absolute value (#13, https://github.com/napbat/pdbrs/issues/13#issuecomment-6409581275, https://github.com/napbat/pdbrs/issues/13#issuecomment-6409665284).
  - 0xf6 `DEBUG_S_INLINEELINES`: `u32` signature 0 or 1 (`CV_INLINEE_SOURCE_LINE_SIGNATURE[_EX]`), entries `inlinee` (IPI index), `fileId`, `sourceLineNum`, and for signature 1 a `u32` count plus that many checksum offsets. Other signatures are `UnsupportedVersion` (`cvinfo.h:4819-4836`, LLVM `DebugInlineeLinesSubsection.cpp`).
  - pdbrs rejects `DEBUG_S_LINES` header flags other than bit 0 (`UnsupportedVersion`) and requires `block_size` = 12 + count × (8 or 12) exactly.
  - pdbrs names the line values 0xfeefee (always step into) and 0xf00f00 (never step into) (`blocks.rs:8-14`).
  - pdbrs does not decode a kind with bit 31 set, also if the low bits name a known kind.

### `microsoft-pdb-rs/docs/codeview/codeview.md`
- Documents: An overview of CodeView, where CodeView data is stored (linker PDBs, compiler PDBs, COFF `.debug$S`), and links to the other documents.
- pdbrs: partial — pdbrs reads CodeView from PDB files only (TPI, IPI, module streams, global streams; `crates/pdbrs/src/native/**`). pdbrs has no COFF object reader, so `.debug$S` and `.debug$T` in OBJ files are not implemented. Inference, not tested: Compiler (`/Zi`) PDBs use the same MSF container; no fixture in the evidence is a compiler PDB.
- Conflicts:
  - Document omission: the list does not link `calling_convention.md`, which exists in the same directory.
  - None found against pdbrs.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/calling_convention.md`
- Documents: The `CV_call_e` calling convention values 0x00-0x19.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/attributes.rs:231-285` (`CallingConvention`), used by `LF_PROCEDURE` and `LF_MFUNCTION` (`type_record.rs:270-324`). Values 0x00-0x05 and 0x07-0x19 decode. Other values are `InvalidFormat`, so the whole record fails.
- Conflicts:
  - Document vs pdbrs: 0x06 `SKIPPED` is in the document and in `cvconst.h` (`CV_CALL_SKIPPED`). pdbrs (like LLVM `CallingConvention`) has no value 0x06 and rejects the record.
  - Document and `cvconst.h` vs LLVM and pdbrs: 0x19 is `RESERVED` ("first unused call enumeration"). LLVM and pdbrs decode 0x19 as `Swift`.
  - Document vs pinned sources: the document cites `cvinfo.h` as a source, but the pinned `cvinfo.h` does not define `CV_call_e`; only `cvconst.h:31-57` does. The document drops the `CV_CALL_` prefix.
  - Document internal errors: a stray line "T" (line 3); the intro says the file specifies "constants (named values and enums)" but it has only one enum.
- Undocumented assumptions: None.


## `types/`

### `microsoft-pdb-rs/docs/codeview/types/types.md`
- Documents: The type record framing (`u16 size`, `u16 kind`), the size and alignment invariants, record categories, and a summary table of leaf kinds.
- pdbrs: partial — `crates/pdbrs/src/native/codeview.rs` frames records; `crates/pdbrs/src/native/codeview/type_record.rs:117-152` decodes `LF_MODIFIER`, `LF_POINTER`, `LF_PROCEDURE`, `LF_MFUNCTION`, `LF_ARGLIST`, `LF_FIELDLIST`, `LF_BITFIELD`, `LF_METHODLIST`, `LF_ARRAY`, `LF_CLASS`, `LF_STRUCTURE`, `LF_INTERFACE`, `LF_UNION`, `LF_ENUM`, `LF_VTSHAPE`, `LF_VFTABLE`, `LF_LABEL`. All other kinds give `TypeLeaf::Unsupported(Record)` with the raw payload, including `LF_SKIP`, `LF_PRECOMP`, `LF_ENDPRECOMP`, `LF_CLASS2`/`LF_STRUCTURE2`/`LF_UNION2`/`LF_INTERFACE2` (0x1608-0x160b), `LF_TYPESERVER2`, `LF_ALIAS`, `LF_STRIDED_ARRAY`, `LF_VECTOR`, `LF_MATRIX`, `LF_HLSL`, `LF_MODIFIER_EX`, and all `_16t`/`_ST` kinds.
- Conflicts:
  - Document vs `cvinfo.h`, LLVM and pdbrs: the table lists `LF_ARRAY` as 0x1502. 0x1502 is `LF_ENUMERATE`; `LF_ARRAY` is 0x1503 (`cvinfo.h:888`, `CodeViewTypes.def:47`, `type_record.rs:54`). `lf_array.md` has the correct value.
  - Document vs `cvinfo.h`: the table names 0x1205 `LF_BITFIELDS`; `cvinfo.h` names it `LF_BITFIELD`.
  - Document vs pdbrs: invariant "`size` is a multiple of 2". pdbrs does not check it. Observed IPI records are not 4-byte aligned: `LF_UDT_MOD_SRC_LINE` records are 18 bytes without padding (example at offset 0x78 in `items/lf_func_id.md`), so the next record starts at 0x8a. pdbrs does not require alignment.
  - Document omissions: the table omits `LF_INTERFACE` (0x1519, described in `lf_class.md`), `LF_LABEL` (0x000e) and `LF_VFTABLE` (0x151d), which pdbrs decodes, and `LF_CLASS2`..`LF_INTERFACE2` (LLVM only). The sections "Summary of item records" and "Leaf indices referenced from symbols" are empty. The document does not describe `LF_PAD` (0xf0-0xff) end padding.
- Undocumented assumptions:
  - pdbrs requires the bytes after the last field to be 0-3 `LF_PAD` bytes in the exact `f3 f2 f1` pattern (`codeview.rs:277-290`).
  - Kind values and layouts follow LLVM `CodeViewTypes.def` and `TypeRecordMapping.cpp` at `b9f2235f`; flag bit layouts follow `cvinfo.h` (`type_record.rs:1-11`). `LF_VFTABLE` and `LF_LABEL` layouts are confirmed against `lld-link` output (#11, https://github.com/napbat/pdbrs/issues/11#issuecomment-6408363519).

### `microsoft-pdb-rs/docs/codeview/types/lf_union.md`
- Documents: `LF_UNION` (0x1506): `count`, `property`, `fields`, `Number length`, `name`, optional `unique_name`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/udt.rs:112-157` (`Union`): u16 count, `ClassOptions` u16, field list index, `Integer` size, name, unique name if bit 9.
- Conflicts:
  - Document vs pdbrs: `length` is a `Number`. pdbrs accepts only integer leaves; a non-integer leaf is `UnsupportedVersion`.
  - None found in the layout (matches `cvinfo.h` `lfUnion` and LLVM).
- Undocumented assumptions:
  - `LF_UNION2` (0x160a, LLVM `CodeViewTypes.def:58`) is not described and pdbrs does not decode it (`Unsupported`).

### `microsoft-pdb-rs/docs/codeview/types/lf_vtshape.md`
- Documents: `LF_VTSHAPE` (0x000a): a `u16` count and 4-bit slot descriptors with values 0-6.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/list.rs:162-227` (`VtableShape`), `attributes.rs:589-607` (`VtableSlotKind` 0-6). Descriptor bytes = ceil(count / 2).
- Conflicts:
  - Document omission vs LLVM `TypeRecordMapping.cpp:514-538` and pdbrs: the document does not state the nibble order. LLVM and pdbrs put the first slot in the high 4 bits of each byte.
  - Document vs pdbrs: values 7-15 are "reserved". pdbrs rejects them (`InvalidFormat`); `cvinfo.h` names 7 `CV_VTS_unused`.
  - Document vs `cvinfo.h`: value names 0 "Near", 1 "Far", 5 "Near32", 6 "Far32" match `CV_VTS_near`..`CV_VTS_far32`. pdbrs uses LLVM names `Near16`, `Far16`, `This` (LLVM's spelling of `CV_VTS_thin`), `Outer`, `Meta`, `Near`, `Far`. Same values, no layout difference.
- Undocumented assumptions:
  - For an odd count, pdbrs requires the unused low nibble of the last byte to be zero (`list.rs:182-191`).

### `microsoft-pdb-rs/docs/codeview/types/lf_skip.md`
- Documents: `LF_SKIP` (0x1200): a record that reserves space; its payload is ignored.
- pdbrs: not implemented — `LF_SKIP` gives `TypeLeaf::Unsupported` with the raw payload (`type_record.rs:150`).
- Conflicts:
  - Document vs `cvinfo.h:2026-2030`: `lfSkip` has a `CV_typ_t type` field ("next valid index") before the pad data. The document does not describe this field.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_procedure.md`
- Documents: `LF_PROCEDURE` (0x1008): return type, calling convention, a "reserved" byte, parameter count, argument list.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record.rs:255-284` (`Procedure`): return type, `CallingConvention` u8, `FunctionOptions` u8, u16 count, argument list index.
- Conflicts:
  - Document vs `cvinfo.h:1775-1782` and pdbrs: the byte after `calling_convention` is `CV_funcattr_t funcattr`, not reserved: bit 0 `cxxreturnudt`, bit 1 `ctor`, bit 2 `ctorvbase`, bits 3-7 unused. pdbrs keeps it as `FunctionOptions` (0x01, 0x02, 0x04).
  - Document vs LLVM `CodeViewDebug.cpp:2128-2171`: the document says static methods use `LF_PROCEDURE`. LLVM writes static member functions as `LF_MFUNCTION` with `this` type 0.
- Undocumented assumptions:
  - An unknown calling convention fails the whole record (`InvalidFormat`); see `calling_convention.md`.

### `microsoft-pdb-rs/docs/codeview/types/lf_precomp.md`
- Documents: `LF_PRECOMP` (0x1509): `start`, `count`, `signature`, `name`, and the precompiled type inclusion rules.
- pdbrs: not implemented — `TypeLeaf::Unsupported` (`type_record.rs:150`). pdbrs reads linker PDBs; the record belongs to object files and compiler PDBs.
- Conflicts:
  - Document vs `cvinfo.h:1937-1943`: `cvinfo.h` says "length prefixed name"; the document says `strz`. LLVM `TypeRecordMapping.cpp:759-766` reads a NUL-terminated string. The `cvinfo.h` comment is a legacy `_ST` comment.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_modifier.md`
- Documents: `LF_MODIFIER` (0x1001): the modified type and a `u16` with `const` (bit 0), `volatile` (bit 1), `unaligned` (bit 2); bits 3-15 reserved, must be zero.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record.rs:177-197` (`Modifier`), `attributes.rs:136-171` (`ModifierOptions` 0x1, 0x2, 0x4; raw word kept).
- Conflicts:
  - Document vs pdbrs: bits 3-15 "must be zero". pdbrs keeps them in `ModifierOptions::bits` and does not reject them.
  - Document internal errors: "`LD_MODIFIER`" is a typing error. The example applies the modifier to `T_CHAR`; LLVM writes plain `char` as `T_RCHAR` (0x70) (`CodeViewDebug.cpp:1929-1930`).
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_pointer.md`
- Documents: `LF_POINTER` (0x1002): referent type, `u32` attribute bit fields, `ptrtype` and `ptrmode` values, and variant data for based and member pointers.
- pdbrs: partial — `crates/pdbrs/src/native/codeview/type_record.rs:199-253` (`Pointer`), `attributes.rs:287-475`. Kind bits 0-4 (`PointerKind` 0x00-0x0c), mode bits 5-7 (`PointerMode` 0-4), flags 0x100 flat32, 0x200 volatile, 0x400 const, 0x800 unaligned, 0x1000 restrict, size bits 13-18, 0x80000 MoCOM, 0x100000 `&this`, 0x200000 `&&this`; bits 22-31 kept raw. For modes 2 and 3 pdbrs reads containing class + `u16` `MemberPointerRepresentation` 0-8. For based kinds 0x03-0x09 pdbrs keeps the rest of the payload raw (`Pointer::based`) and does not check padding. Not implemented: decoding of based-pointer data.
- Conflicts:
  - Document vs `cvinfo.h:1035-1045`, LLVM and pdbrs: the `format` table lists 14 values (0-13) from the old CV4 16:16/16:32 scheme. `CV_pmtype_e` defines 0 `Undef`, 1-4 data (single, multiple, virtual, general), 5-8 function (single, multiple, virtual, general). pdbrs rejects 9-13 (`InvalidFormat`).
  - Document vs `cvinfo.h:1536-1547`: "pointer based on a type: the variant data is a single `TypeIndex`". `cvinfo.h` `btype` has an index and a name. The document omits the data of `BASE_SEG` (`u16 bseg`) and `BASE_VAL`/`BASE_SEGVAL`/`BASE_ADDR`/`BASE_SEGADDR` (a copy of the base symbol record).
  - Document vs LLVM and pdbrs: "The data after the `Pointer` structure depends on `ptrtype`". Member-pointer data depends on `ptrmode` (2 or 3) (LLVM `TypeRecordMapping.cpp:389`); based data depends on `ptrtype`.
  - Document vs pdbrs: the document lists 0x0d `UNUSEDPTR` as a value; pdbrs rejects kinds ≥ 0x0d and modes ≥ 5 (`InvalidFormat`).
  - Document internal errors: the `ptrtype` row says "Specifies the mode"; "Ttrue"; field name `lsrref` for `isrref`. The struct name `PointerToDataMemberVariant` also covers pointers to member functions. A `TODO` ("convert these") ends the document.
- Undocumented assumptions:
  - pdbrs keeps based-pointer bytes raw because LLVM does not decode them and no sample has them (#2, https://github.com/napbat/pdbrs/issues/2#issuecomment-6407739492).

### `microsoft-pdb-rs/docs/codeview/types/lf_mfunction.md`
- Documents: `LF_MFUNCTION` (0x1009): return, class and `this` types, calling convention, a "reserved" byte, parameter count, argument list, `uint32_t this_adjust`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record.rs:286-324` (`MemberFunction`): three type indices, `CallingConvention`, `FunctionOptions`, u16 count, argument list, `i32 this_adjustment`.
- Conflicts:
  - Document vs `cvinfo.h:1801-1811` and pdbrs: the byte after `calling_convention` is `CV_funcattr_t`, not reserved.
  - Document vs `cvinfo.h`, LLVM and pdbrs: `this_adjust` is `uint32_t` in the document; `cvinfo.h` declares `long thisadjust`, LLVM `int32_t ThisPointerAdjustment`, pdbrs `i32`.
  - Document vs LLVM `CodeViewDebug.cpp:2146-2170`: the document says the record is for "non-static instance" methods. LLVM writes static methods as `LF_MFUNCTION` with `this` = 0; pdbrs documents `TypeIndex::NONE` for a static function. `lf_mfunc_id.md` and `lf_procedure.md` disagree with this document too.
  - Document omission: a `TODO` on `this_adjust`.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_methodlist.md`
- Documents: `LF_METHODLIST` (0x1206): entries of `u16 attribute`, `u16 padding`, method type, optional `uint32_t vtab_offset` for `mprop` 4 or 6; the `CV_fldattr_t` bits.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/list.rs:61-160` (`MethodList`, `MethodListEntry`), `attributes.rs:477-587` (`MemberAttributes`: access bits 0-1, `MethodKind` bits 2-4, flags 0x20-0x200). Padding must be 0; `vtable_offset: Option<i32>` for kinds 4 and 6.
- Conflicts:
  - Document and `cvinfo.h` (`mlMethod`, `unsigned long vbaseoff`) vs LLVM and pdbrs: the offset is unsigned in the document and `cvinfo.h`, but signed `i32` in pdbrs (LLVM `int32_t VFTableOffset`). Same bytes.
  - Document vs pdbrs: `mprop` 7 is not listed. pdbrs rejects it (`InvalidFormat`).
  - Document internal: the Rust example (`MethodList::parse`, `entry.ty`) is the `ms-codeview` API, not a format rule. `noinherit`/`noconstruct` say "Method"; `cvinfo.h` says "class".
- Undocumented assumptions:
  - pdbrs ends the list when fewer than 8 bytes remain and requires those bytes to be valid `LF_PAD` padding (`list.rs:117-143`).

### `microsoft-pdb-rs/docs/codeview/types/lf_fieldlist.md`
- Documents: `LF_FIELDLIST` (0x1203) and its member records (`LF_BCLASS`, `LF_VBCLASS`, `LF_IVBCLASS`, `LF_INDEX`, `LF_VFUNCTAB`, `LF_VFUNCOFF`, `LF_ENUMERATE`, `LF_FRIENDFCN`, `LF_MEMBER`, `LF_STMEMBER`, `LF_METHOD`, `LF_NESTTYPE`, `LF_ONEMETHOD`, `LF_NESTTYPEEX`, `LF_FRIENDCLS`) and the `CV_fldattr_t` bits.
- pdbrs: partial — `crates/pdbrs/src/native/codeview/field_list.rs` decodes `LF_BCLASS`, `LF_VBCLASS`, `LF_IVBCLASS`, `LF_INDEX`, `LF_VFUNCTAB`, `LF_ENUMERATE`, `LF_MEMBER`, `LF_STMEMBER`, `LF_METHOD`, `LF_NESTTYPE`, `LF_ONEMETHOD`. Not implemented: `LF_FRIENDFCN` (0x150c), `LF_FRIENDCLS` (0x140a), `LF_VFUNCOFF` (0x140c), `LF_NESTTYPEEX` (0x1512), and the undocumented `LF_MEMBERMODIFY` (0x1513) and `LF_BINTERFACE` (0x151a). An unknown member kind makes the whole `LF_FIELDLIST` fail (`UnsupportedVersion`), because members have no length. `LF_INDEX` must be the last member (`InvalidFormat` otherwise).
- Conflicts:
  - Document vs `cvinfo.h:868-880` and LLVM: `LF_FRIENDCLS` is 0x140a. The document gives 0x040b, which is `LF_FRIENDCLS_16t`.
  - Document vs `cvinfo.h:2615-2618`: `LF_VFUNCTAB` `type` is "type index of pointer" (an `LF_POINTER` to the `LF_VTSHAPE`). The document says it points to the shape and writes `LF_VFTSHAPE`.
  - Document vs `cvinfo.h:2631-2636`: `LF_VFUNCOFF` `offset` is `CV_off32_t` (signed); the document says `uint32_t`.
  - Document vs LLVM and pdbrs: `LF_ONEMETHOD` `vbaseoff` is `uint32_t` in the document and `unsigned long` in `cvinfo.h`; LLVM and pdbrs read `i32`.
  - Document vs `cvinfo.h`: names `LF_NESTEDTYPE` and `LF_NESTEDTYPEX` are `LF_NESTTYPE` and `LF_NESTTYPEEX`.
  - Document omissions vs `cvinfo.h`/LLVM: no `LF_MEMBERMODIFY` (0x1513), `LF_BINTERFACE` (0x151a), `LF_MANAGED`, or `_ST`/`_16t` member kinds. The document does not describe `LF_PAD` bytes between members.
  - Document vs pdbrs: the document says the `LF_FRIENDFCN`, `LF_INDEX`, `LF_NESTTYPE`, `LF_VFUNCTAB`, `LF_FRIENDCLS` padding "must be zero". pdbrs enforces it for the kinds it decodes (`field_list.rs:306-313`).
- Undocumented assumptions:
  - `LF_PAD` after a member: the low 4 bits of the first byte give the count; each next byte counts down to 0xf1; a lone 0xf0 is invalid (`field_list.rs:283-304`, LLVM `CodeViewRecordIO::skipPadding`).
  - `LF_BCLASS`, `LF_VBCLASS`, `LF_ENUMERATE`, `LF_MEMBER` numeric fields accept only integer leaves.
  - pdbrs rejects `mprop` 7 in any attribute word.

### `microsoft-pdb-rs/docs/codeview/types/lf_enum.md`
- Documents: `LF_ENUM` (0x1507): `count`, `property`, underlying type, field list, `name`, optional `unique_name`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/udt.rs:159-204` (`Enum`).
- Conflicts:
  - Document vs `cvinfo.h:1752-1759`: `cvinfo.h` comments the name as "length prefixed"; the document, LLVM and pdbrs use a NUL-terminated name. Legacy comment, no real difference.
  - None found against pdbrs.
- Undocumented assumptions:
  - pdbrs does not check that the underlying type is primitive or that the field list holds only `LF_ENUMERATE`; it accepts any type index.

### `microsoft-pdb-rs/docs/codeview/types/lf_endprecomp.md`
- Documents: `LF_ENDPRECOMP` (0x0014): a `u32` signature that ends the precompiled types of a creator object.
- pdbrs: not implemented — `TypeLeaf::Unsupported` (`type_record.rs:150`).
- Conflicts:
  - Document internal: the file ends with a section "Records referenced from other type records" that belongs in `types.md`.
  - None found in the layout (matches `cvinfo.h:1951-1954` and LLVM `TypeRecordMapping.cpp:768-772`).
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_class.md`
- Documents: `LF_CLASS` (0x1504), `LF_STRUCTURE` (0x1505), `LF_INTERFACE` (0x1519): count, `property` bits 0-15, field list, derivation list, vshape, `Number length`, name, optional unique name; `CV_HFA` and `CV_MOCOM_UDT` values.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/udt.rs:43-110` (`Class`, `ClassKind`), `attributes.rs:15-134` (`ClassOptions`: 0x1 packed … 0x400 sealed, HFA bits 11-12, 0x2000 intrinsic, MoCOM bits 14-15). Unique name read only if bit 9 is set.
- Conflicts:
  - Document vs pdbrs: `length` is a `Number`; pdbrs accepts only integer leaves.
  - Document vs `cvinfo.h:1120-1135`: bit names `overops` and `nested` are `ovlops` and `isnested` in `cvinfo.h`. Same bits.
  - Document omission: `LF_CLASS2` 0x1608, `LF_STRUCTURE2` 0x1609, `LF_INTERFACE2` 0x160b (LLVM `CodeViewTypes.def:54-56`) are not described. pdbrs gives `Unsupported` for them (#2).
- Undocumented assumptions:
  - pdbrs does not check that `derivation_list` is zero.

### `microsoft-pdb-rs/docs/codeview/types/lf_bitfield.md`
- Documents: `LF_BITFIELD` (0x1205): type, `u8 length`, `u8 position`.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record.rs:326-349` (`Bitfield`).
- Conflicts: None found (matches `cvinfo.h:2164-2168` and LLVM).
- Undocumented assumptions:
  - pdbrs does not check that `position + length` fits in the underlying type.

### `microsoft-pdb-rs/docs/codeview/types/lf_array.md`
- Documents: `LF_ARRAY` (0x1503): element type, index type, `Number length`, name.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record.rs:351-377` (`Array`).
- Conflicts:
  - Document vs `types.md`: this document gives 0x1503 (correct); `types.md` gives 0x1502.
  - Document vs pdbrs: `length` accepts only integer leaves in pdbrs.
  - Document omissions: a `TODO` on `index_type`; `LF_STRIDED_ARRAY` (0x1516) is not described and pdbrs does not decode it.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/types/lf_arglist.md`
- Documents: `LF_ARGLIST` (0x1201): a `u32` count and that many type indices; an ownership invariant.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/type_record/list.rs:16-59` (`ArgumentList`).
- Conflicts:
  - Document vs observed linker behavior: "Each `LF_ARGLIST` record is pointed-to by exactly one `LF_PROCEDURE` or `LF_MFUNCTION`". Inference, not tested: Linkers merge identical records, so two procedure types with the same parameters but different return types share one `LF_ARGLIST`. pdbrs does not check the invariant.
- Undocumented assumptions: None.


## `items/`

### `microsoft-pdb-rs/docs/codeview/items/items.md`
- Documents: IPI item records, the `ItemId` index space and lookup, and the kind summary 0x1601-0x1607.
- pdbrs: implemented — `crates/pdbrs/src/native/codeview/id_record.rs` (`IdRecord::decode` for all 7 kinds; other kinds `Unsupported`), `id_record/table.rs` (`IdTable`: O(1) lookup through `TypeTable`, `resolve_string`, `validate`). `codeview/index.rs` `IdIndex`, 0 = none.
- Conflicts:
  - Document internal: the link `./lf_func_id.md.md` is broken. The table lists 0x1607 before 0x1606.
  - Document vs pdbrs: the document describes a linear scan to find a record; pdbrs builds an offset table (`native/types.rs` `TypeTable`, 4 bytes per record). No format difference.
- Undocumented assumptions:
  - `IdTable::validate` checks each type index against the primitive table or TPI `index_end`, and each ID reference against the IPI range and the expected kind (`table.rs:160-243`). It does not check the backward-reference rule that `lf_buildinfo.md` states.

### `microsoft-pdb-rs/docs/codeview/items/lf_buildinfo.md`
- Documents: `LF_BUILDINFO` (0x1603): five `ItemId` arguments (cwd, tool, source, PDB, args), truncation rules, and examples.
- pdbrs: implemented — `id_record.rs:110-115,178-229` (`BuildInfo`, `BuildInfoArgument` positions 0-4): a `u16` count, then `count` ID indices. `table.rs:207-213` accepts 0, `LF_STRING_ID` or `LF_SUBSTR_LIST` per argument.
- Conflicts:
  - Document vs `cvinfo.h:1726-1730`, LLVM and pdbrs: the C struct has no count. `cvinfo.h` has `unsigned short count` before `arg[]`; the document's own example text calls it `num_strings` = 5. pdbrs reads the `u16` count.
  - Document vs pdbrs: "this record can be truncated after any field". pdbrs requires exactly `count` IDs; a record shorter than its count is `InvalidFormat`. A record with fewer than 5 IDs (smaller `count`) is valid in pdbrs.
  - Document internal: the first example names the string values but shows no IPI offsets for them; the anchor `../symbols/symbols.md#s_buildinfo-0x114c---build-info` depends on heading text.
  - Document omissions: a `TODO` on response files.
- Undocumented assumptions:
  - pdbrs does not check that referenced IDs are below the record's own ID (the document's backward-reference rule).

### `microsoft-pdb-rs/docs/codeview/items/lf_func_id.md`
- Documents: `LF_FUNC_ID` (0x1601): scope `ItemId`, function type, name, optional `u64` decorated name hash, the padding rule and examples.
- pdbrs: implemented — `id_record.rs:98-103,142-159,364-370` (`FunctionId`): scope 0 → `None`; hash present if ≥ 8 bytes follow the name; then 0-3 `LF_PAD` bytes. `table.rs:197-202` requires the scope to be an `LF_STRING_ID`.
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
- pdbrs: implemented — `id_record.rs:104-109,161-176` (`MemberFunctionId`), same hash and padding rule as `LF_FUNC_ID`. `table.rs:203-206` checks both type indices against the TPI range only.
- Conflicts:
  - Document vs pdbrs: the parent type "should be" `LF_STRUCTURE`, `LF_CLASS`, `LF_UNION` or `LF_ENUM`. pdbrs does not check the parent kind.
  - None found in the layout (matches `cvinfo.h:1687-1692`).
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/codeview/items/lf_string_id.md`
- Documents: `LF_STRING_ID` (0x1605): a substring list `ItemId` (0 = none) and a NUL-terminated string.
- pdbrs: implemented — `id_record.rs:120-123,231-243` (`StringId`); `table.rs:98-158` (`resolve_string`).
- Conflicts:
  - Document omission vs pdbrs: "this string is concatenated with the substrings" does not give the order. pdbrs writes the substring-list parts first, then the record's own string; the `lf_buildinfo.md` example (args at 0x10fc, list at 0x10fb) shows this order.
- Undocumented assumptions:
  - pdbrs allows nested substring lists up to `MAX_STRING_DEPTH` = 8 and caps visits at the record count (#4, https://github.com/napbat/pdbrs/issues/4#issuecomment-6407539418).
  - `validate` requires a non-zero substring reference to be an `LF_SUBSTR_LIST`.

### `microsoft-pdb-rs/docs/codeview/items/lf_substr_list.md`
- Documents: `LF_SUBSTR_LIST` (0x1604): a list of `LF_STRING_ID` IDs to concatenate, with an example.
- pdbrs: implemented — `id_record.rs:116-119` (`IdList`): a `u32` count, then `count` IDs. `table.rs:214-216` requires each entry to be an `LF_STRING_ID`.
- Conflicts:
  - Document vs `cvinfo.h:2043-2046` (`lfArgList` is used for `LF_SUBSTR_LIST`), LLVM and pdbrs: the C struct has no count, but the record starts with a `u32` count. The document's example starts with `09 00 00 00`.
  - Document internal: the `TODO` names `LF_SUBSTR_ID`, which does not exist; it means `LF_SUBSTR_LIST`.
- Undocumented assumptions:
  - pdbrs supports nesting through `LF_STRING_ID` substring references up to depth 8; the document's `TODO` leaves the question open.

### `microsoft-pdb-rs/docs/codeview/items/lf_udt_src_line.md`
- Documents: `LF_UDT_SRC_LINE` (0x1606): UDT type, source file, line.
- pdbrs: implemented — `id_record.rs:124-128,245-255` (`UdtSourceLine`: `udt`, `source_file: IdIndex`, `line`). `table.rs:220-223` requires `source_file` to be an `LF_STRING_ID`.
- Conflicts:
  - Document vs `cvinfo.h:1700-1705` and pdbrs: the document says `file_name` is a `NameIndex` into the `/names` stream. `cvinfo.h` declares `CV_ItemId src` ("index to LF_STRING_ID record"); pdbrs reads an IPI ID.
- Undocumented assumptions:
  - No fixture contains this record; only synthetic tests cover it (#4).

### `microsoft-pdb-rs/docs/codeview/items/lf_udt_mod_src_line.md`
- Documents: `LF_UDT_MOD_SRC_LINE` (0x1607): UDT type, `/names` file name offset, line, `u16` module index.
- pdbrs: implemented — `id_record.rs:129-134,257-269` (`UdtModuleSourceLine`: `name_offset` u32, `line`, `module` u16 raw). `table.rs:224` checks only the UDT index; pdbrs does not check the `/names` offset here.
- Conflicts:
  - Document internal error: "the same as `LF_MOD_SRC_LINE`"; the record it extends is `LF_UDT_SRC_LINE`, and the two differ in more than `module_index`: `LF_UDT_SRC_LINE` uses an IPI ID for the file, this record uses a `/names` offset (`cvinfo.h:1707-1713`).
  - Document omissions: two `TODO`/Determinism blocks on `module_index`; the document does not say whether the module index is 0-based or 1-based. pdbrs keeps the raw value.
- Undocumented assumptions:
  - The record has 14 payload bytes and often no end padding, so later IPI records start at non-4-aligned offsets (see `lf_func_id.md`). pdbrs accepts 0 padding bytes.
