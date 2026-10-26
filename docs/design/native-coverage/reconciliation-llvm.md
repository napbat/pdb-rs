# Native coverage: reconciliation of the LLVM PDB documents

This page is part of the [native coverage](../native-coverage.md) accounting. It covers `docs/reference/llvm/llvm/` at LLVM `b9f2235f`.
Each section names one imported document. Paths are relative to `docs/reference/`. The fields are:

- **Documents**: the format items that the document describes.
- **pdbrs**: `implemented` (pdbrs reads every item that the document describes), `partial`, `not implemented` or `not applicable` (no format content), with the source files.
- **Conflicts**: each difference between the document and pdbrs, or between the document and another source (`cvinfo.h`, LLVM source, `ms-pdb`, other documents). Errors inside the document are conflicts too.
- **Undocumented assumptions**: layouts and rules that pdbrs uses but the document does not state, with the issue that records the evidence.

The comparison used `main` at `8b77fb2`, `cvinfo.h` at `805655a2`, LLVM at `b9f2235f` and the `ms-codeview` 0.1.7, `ms-pdb` 0.1.22, `ms-pdb-msf` 0.1.7 and `ms-pdb-msfz` 0.1.10 crates. "Inference, not tested" marks a statement that comes from reading source code only.

### `llvm/llvm/docs/PDB/index.md`
- Documents: The PDB file layout overview: the MSF container, the fixed streams 0–4, the named streams `/LinkInfo`, `/src/headerblock` and `/names`, the DBI-referenced module, public and global streams, the TPI/IPI hash streams, and the CodeView record pages.
- pdbrs: implemented — pdbrs reads every stream role in the table. Stream 0: `OldDirectory` snapshot (`crates/pdbrs/src/native/msf/old_directory.rs`). Stream 1: `crates/pdbrs/src/native/info.rs`. Streams 2/4 and hash streams: `crates/pdbrs/src/native/types.rs`, `types/hash.rs`. Stream 3: `crates/pdbrs/src/native/dbi.rs`, `dbi/*.rs`. Module streams: `dbi/module.rs`, `lines*.rs`, `codeview/symbol*.rs`. GSI/PSI: `crates/pdbrs/src/native/symbols*.rs`. `/LinkInfo` v2: `auxiliary/link_info.rs`. `/src/headerblock`: `injected_source.rs`. `/names`: `names.rs` (hash v1 full; hash v2 partial, no name hashing). `crates/pdbrs/examples/native/` reads all roles and joins them.
- Conflicts:
  - index.md vs DbiStream.md and pdbrs: the table says the Public Stream and the Global Stream hold "Symbol Records" and an "Index of Public/Global Hash Stream". The records are in the symbol record stream (`DbiStreamHeader::SymRecordStream`). The GSI/PSI streams hold the hash table themselves; no separate hash stream exists. pdbrs reads the records from the symbol record stream and the hash tables from the GSI/PSI streams (`symbols.rs`).
  - index.md omits the symbol record stream from the stream table.
  - index.md says `/LinkInfo` contents are "Unknown". pdbrs decodes `/LinkInfo` version 2 from Microsoft `pdb.h` `struct LinkInfo` and `dbi.cpp` `GetUTF8LinkInfo` (`auxiliary/link_info.rs`); version 1 is `UnsupportedVersion`.
  - index.md omits named streams that pdbrs reads: `/TMCache` (`auxiliary/tm_cache.rs`), type map streams that `/TMCache` names (`auxiliary/type_map.rs`), `/UDTSRCLINEUNDONE` (`auxiliary/udt_source_line_undone.rs`), `sourcelink` (`auxiliary/source_link.rs`, partial) and `/src/files/*` (`injected_source/document.rs`, partial). `srcsrv` is also absent; pdbrs classifies it as `Other` (not decoded).
  - index.md omits the DBI optional debug streams (FPO, new FPO, section headers, OMAP and others) from the table; it mentions only "FPO / PGO Data" in the DBI row.
- Undocumented assumptions:
  - Stream 0 policy for writing (#15): `OldDirectoryPolicy::Empty` (default, stream 0 present with length 0, as LLD and `ms-pdb-msf`) or `Preserve` (same page size only).
  - `/TMCache` layout (version `0x20200229`, 20-byte header, `u16` stream table, 16-byte entries) and type map stream versions 1 and 2, from `mspdbcore.dll` `DBI1::DumpTMCache` evidence (#13).
  - `/UDTSRCLINEUNDONE` is a `u32` type index list; no reference defines it (#9).

### `llvm/llvm/docs/PDB/MsfFile.md`
- Documents: The MSF 7.00 container: the 56-byte-prefix SuperBlock, the two free block maps and their intervals, the stream directory, and the rule that fields can cross block boundaries.
- pdbrs: implemented — reader `crates/pdbrs/src/native/msf.rs`, `msf/directory.rs`, `msf/free_page_map.rs`, `msf/old_directory.rs`; writer `msf/write.rs`, `msf/write/layout.rs`, `msf/write/emit.rs`, `msf/write/builder.rs`, `msf/write/file.rs`. The reader accepts every power of two from 512 to 65536 (`MAX_PAGE_SIZE`, `msf.rs:72`). The writer accepts 512 to 32768 and returns `UnsupportedVersion` for 65536 (`MAX_WRITE_PAGE_SIZE`, `write/layout.rs:20-37`). The reader rejects the small MSF 2.00 container with `UnsupportedVersion`.
- Conflicts:
  - MsfFile.md vs LLVM `MSFCommon.h` and pdbrs: the document says valid block sizes are 512, 1024, 2048 and 4096, and that "LLVM only supports 4096 byte blocks". LLVM `isValidBlockSize` accepts 512 to 32768, and `getMaxFileSizeFromBlockSize` gives file limits for 8192–32768. The pdbrs reader accepts 512 to 65536. The pdbrs writer accepts 512 to 32768 (#15).
  - MsfFile.md vs pdbrs reader vs `ms-pdb-msf`: 65536-byte pages. `ms-pdb-msf` 0.1.7 and the pdbrs reader accept them. LLVM ("Unsupported block size."), DIA2Dump (`HRESULT 806D000C`) and `link.exe /PDBPAGESIZE:65536` (`LNK1399`) reject them (#15). The document does not mention 65536.
  - MsfFile.md vs `ms-pdb-msf` and pdbrs: offset 48 is `Unknown` in the document. `ms-pdb-msf` names it `stream_dir_small_page_map` (MSPDB `mpspnpn`). pdbrs ignores it on read and writes 0.
  - MsfFile.md vs pdbrs: `BlockMapAddr` is one `u32` block index. MSPDB/`ms-pdb-msf` and pdbrs treat offset 52 as an array of page-map page numbers (`stream_dir_big_page_map`). The pdbrs reader and writer accept more than one page-map page if the array fits in the header page (`directory.rs:71-77`, `write/layout.rs:98-104`). LLVM `validateSuperBlock` rejects that file ("Too many directory blocks."). A pdbrs writer output with a directory of more than `page_size/4` pages is thus not readable by LLVM.
  - MsfFile.md vs pdbrs reader: the document says `NumBlocks * BlockSize` should equal the file size. The pdbrs reader requires the declared pages to fit and ignores bytes after them (`msf.rs:16`, `:246-249`).
  - MsfFile.md vs LLVM `MSFCommon.h`: the pdbrs writer checks only that the page count fits in `u32`. It does not apply LLVM `getMaxFileSizeFromBlockSize` (4 GiB at ≤ 4096-byte pages). (Inference, not tested: no such check found in `msf/write/*`..)
  - Errors inside the document:
    - "the second block of each FPM refers to the next 8 blocks" is wrong. It refers to the next 8 intervals (the next 32768 blocks at 4096-byte blocks).
    - The `uint16` split example has the bytes reversed. In little-endian order the low byte is in the last byte of block N and the high byte is in the first byte of block N+1.
    - "The file consists of ... `SuperBlock::BlockSize - 3` blocks of data" uses BlockSize as a block count. This is correct only because the interval length equals the block size in blocks; the document does not state this rule.
- Undocumented assumptions:
  - Nil stream size `0xFFFFFFFF` in the stream directory (`NIL_STREAM_SIZE`, `msf.rs:66`); a nil stream has no pages. Not in the document.
  - The directory size must be a multiple of 4 and at least 8 bytes; the stream count must be nonzero; the page lists must fill the directory exactly (`directory.rs:53-58`, `:153-189`).
  - No page reference can name page 0 or page `p` with `p % page_size` in {1, 2}; no page can occur twice (`msf.rs:157-162`, `directory.rs` `check_unique`).
  - FPM read rule: the active FPM is `ceil(page_count/8)` bytes across pages `active + k*page_size`; a set bit is a free page; stream 0 pages can be free or allocated; unreferenced allocated pages are reported, not errors (`free_page_map.rs`, #9).
  - Writer layout (#15): pages in ascending order (stream pages by stream ID, then directory, then page map); if the last page is the first page of an interval, the writer adds that interval's FPM pages; active FPM is 1; FPM2 is all `0xFF`; bytes after the bitmap are `0xFF`; unused bytes are zero; the default page size is 4096.
  - A preserved stream 0 needs the source page size; otherwise `InvalidFormat` before output (#15 addendum).

### `llvm/llvm/docs/PDB/PdbStream.md`
- Documents: The PDB Info stream: the header (version, signature, age, GUID), the version enum, the named stream map (string buffer and hash table), the feature codes and the PDB/EXE match rule.
- pdbrs: implemented — `crates/pdbrs/src/native/info.rs`. pdbrs decodes version VC70 (20000404) only; other versions are `UnsupportedVersion`. It decodes signature, age, the 16 GUID bytes, the string buffer, the hash table (`hash_table.rs`), `niMac` (`Info::next_name_index`) and the feature words (kept raw). `MINI` selects the GSI/PSI bucket count (`symbols.rs:37-115`). VC110, VC140 and NOTM stay raw codes.
- Conflicts:
  - PdbStream.md vs LLVM `NamedStreamMap.cpp` and Microsoft `nmtni.h` and pdbrs: the document says the feature codes follow the named stream map directly. LLVM reads and writes a `u32` `NMTNI::niMac` after the hash table (`NamedStreamMap.cpp:62-63`, `:80-82`). pdbrs reads it as `next_name_index` and starts the features after it (`info.rs:111-112`). An earlier pdbrs build followed the document and reported `niMac` as feature code 0 (defect fixed in #9).
  - PdbStream.md vs LLVM `NamedStreamMap.cpp` and pdbrs: the document says the bucket hash is the hash of the string. LLVM `NamedStreamMapTraits::hashLookupKey` truncates `hashStringV1` to 16 bits (`NMTNI::hash`). pdbrs uses `hash_string_v1(name) & 0xffff` and checks the linear-probing placement of each name (`info.rs:96-100`).
  - PdbStream.md vs MSPDB and pdbrs: `MinimalDebugInfo` says "There is no TPI / IPI stream". The document does not say that the feature also changes the GSI/PSI bucket count to `0x3ffff` (MSPDB `GSI1::GSI1`). pdbrs uses this rule (`BucketCount::from_features`). pdbrs does not change TPI/IPI handling for `MINI`; the claim about absent TPI/IPI streams was not checked against the FASTLINK fixture.
  - PdbStream.md vs pdbrs: LLVM `InfoStream::reload` stops at VC110 and skips unknown words. pdbrs keeps all words and requires the remaining length to be a multiple of 4 (`info.rs:113-115`).
- Undocumented assumptions:
  - Duplicate names in the named stream map are `InvalidFormat` (`info.rs:101-109`).
  - The named-stream hash table layout rules come from `HashTable.md` plus LLVM `HashTable::load` checks (see `HashTable.md` section).
  - Feature `0x494E494D` (`MINI`) selects 262143 GSI/PSI buckets (#8, #13).

### `llvm/llvm/docs/PDB/TpiStream.md`
- Documents: The TPI (stream 2) and IPI (stream 4) layout: the TPI/IPI record split, type index encoding with simple-type kinds and modes, the 56-byte header with version enum, the hash stream buffers (hash values, index offsets, hash adjusters) and the record list.
- pdbrs: implemented — `crates/pdbrs/src/native/types.rs` (header V80 only, `HEADER_SIZE` 56, hash key size 4, buckets 0x1000..=0x40000), `types/hash.rs` (`TypeHashes`, `type_record_hash`, `verify`, `parse_adjusters`), `types/lookup.rs` (O(1) table, index-offset lookup), `types/forward.rs` (forward reference resolution), `codeview/index.rs` (primitive type decoding). Records: `codeview/type_record.rs`, `codeview/field_list.rs`, `codeview/id_record.rs`. Not implemented: the auxiliary hash stream (`hash_aux_stream` is parsed as a stream number only), `LF_TYPESERVER2`, `LF_PRECOMP`, `LF_ENDPRECOMP` (raw `TypeLeaf::Unsupported`), versions V40/V41/V50/V70 (`UnsupportedVersion`).
- Conflicts:
  - TpiStream.md vs Microsoft `tpi.cpp` (`mpnitiHead`), LLVM `DumpOutputStyle.cpp` and pdbrs: the document says hash adjuster keys are hash values. Microsoft and LLVM read the keys as `/names` offsets of UDT names; values are type indices. pdbrs `HashAdjuster { name_offset, index }` follows the implementations (`types/hash.rs:10-15`, #3).
  - TpiStream.md vs `cvinfo.h` (`CV_MMASK 0x700`) and LLVM `TypeIndex.h` (`SimpleModeMask 0x700`): the diagram gives Mode bits 8–11 (4 bits). The mode is bits 8–10; bit 11 is reserved. pdbrs reads byte 1 and accepts only modes 0–7 (`codeview/index.rs:41-47`, `:93-104`), so a set bit 11 is `InvalidFormat`.
  - TpiStream.md vs pinned `cvinfo.h`: the document lists `Character8 0x7c`, `Boolean128 0x34`, `Complex16 0x56`, `Complex32PartialPrecision 0x55` and `Complex48 0x54`. The pinned `cvinfo.h` does not define them (LLVM `TypeIndex.h` does). pdbrs accepts them (`codeview/index.rs:184-249`).
  - TpiStream.md vs `cvinfo.h` and pdbrs: the document omits `T_ABS 0x01`, `T_SEGMENT 0x02`, `T_CURRENCY 0x04`, `T_NBASICSTR 0x05`, `T_FBASICSTR 0x06`, `T_BIT 0x60`, `T_PASCHAR 0x61` and `T_BOOL32FF 0x62`. pdbrs decodes them.
  - TpiStream.md vs LLVM `TypeIndex.h` and pdbrs: the document says a type index with bit 31 set comes from the IPI. LLVM names this `DecoratedItemIdMask`. pdbrs has no decoration handling; such an index is out of range for `IdTable::validate` and `TypeTable::record`. [INFERENCE from `codeview/index.rs` and `id_record/table.rs:245-251`.]
  - TpiStream.md vs pdbrs: "HashKeySize - usually 4 bytes". pdbrs requires exactly 4 (`types.rs:391-393`). The document also does not give the bucket limits; LLVM `TpiStream.cpp` and pdbrs both require 0x1000..=0x40000.
  - Errors inside the document: `TypeEndBegin` (twice) should be `TypeIndexBegin`; `HashBufferLength` should be `HashValueBufferLength`.
  - TpiStream.md says the records form a topologically sorted DAG. pdbrs does not check this order. (Inference, not tested: no check found in `types/*`..)
- Undocumented assumptions:
  - The hash value of each record is `type_record_hash(record) % buckets` (LLVM `hashTypeRecord`): UDTs use `hash_string_v1` of the name, or of the unique name for scoped types; forward references and anonymous UDTs use the CRC (`hash_buffer_v8`); `LF_UDT_SRC_LINE`/`LF_UDT_MOD_SRC_LINE` hash the 4-byte UDT index; others hash the full record. `LF_CLASS2`..`LF_INTERFACE2` (0x1608–0x160b) give `UnsupportedVersion` (`types/hash.rs:35-36`, `:613-616`, #3).
  - The three hash buffers must tile the hash stream without gaps or overlaps; index offset pairs must be strictly sorted and name record starts (#3).
  - Forward reference resolution follows LLVM `findFullDeclForForwardRef` (#2).

### `llvm/llvm/docs/PDB/DbiStream.md`
- Documents: The DBI stream: the 64-byte header (version enum, build number, flags, machine), the module info records, the section contribution versions, the section map, the file info substream, the type server map, the EC substream and the 11 optional debug stream slots.
- pdbrs: partial — `crates/pdbrs/src/native/dbi.rs` decodes the header (signature −1 and V70 only), all seven substream ranges, module records (flags raw, stream, sizes, file count, names; 4-byte record alignment), the EC substream as a `/names` string table (`names.rs`), and the optional debug header slots. `dbi/section_contribution.rs`: V60 (28 bytes), V2 (32 bytes) and headerless SC40 (20 bytes). `dbi/section_map.rs`: header and 20-byte entries with `SegmentFlags`. `dbi/sources.rs`: file info with summed counts. Slot 0 FPO and slot 9 FRAMEDATA: `fpo.rs`. Slots 5 and 10: `section.rs` (40-byte headers). Not implemented: typed DBI `Flags` and `BuildNumber` (raw `u16`), the module's embedded section contribution, `SourceFileNameIndex`/`PdbFilePathNameIndex` (not exposed), a non-empty type server map (`UnsupportedVersion`, `dbi.rs:335-345`), slots 1 Exception, 2 Fixup, 3/4 OMAP (detected; `address.rs` refuses translation), 6 TokenRidMap, 7 Xdata, 8 Pdata; DBI versions VC41/V50/V60/V110.
- Conflicts:
  - DbiStream.md vs LLVM `RawTypes.h` (`MachineType`, "See PDB_MachineType enum") and pdbrs: the document says `Machine` is a `CV_CPU_TYPE_e` value. The field holds `IMAGE_FILE_MACHINE_*` values; the document's own examples (`0x8664`, `0x14C`) are PE machine values. pdbrs documents `machine` as the `IMAGE_FILE_MACHINE` value (`dbi.rs:142-143`).
  - DbiStream.md vs `cvinfo.h` (`tagFRAMEDATA`), LLVM `DbiStream.cpp` (`initializeNewFpoRecords` reads `DebugFrameDataSubsectionRef`) and pdbrs: the document says slot 9 (New FPO) is a debug directory of type `IMAGE_DEBUG_TYPE_FPO`. Slot 9 holds 32-byte `FRAMEDATA` records; slot 0 holds 16-byte `FPO_DATA` (`IMAGE_DEBUG_TYPE_FPO`). pdbrs decodes slot 0 as `FpoData` and slot 9 as `FrameData` with frame programs in `/names` (`fpo.rs:3-16`). The document's sentence ".debug$F sections are only emitted by MASM" is attached to the wrong slot.
  - LLVM `COFF.h` `object::FpoData` vs `winnt.h` `FPO_DATA` and pdbrs (FPO slot 0, not described in the document): LLVM reads `cbProlog` as bits 0–3, `fHasSEH` as bit 9 and `fUseBP` as bit 10. `winnt.h` and pdbrs use `cbProlog` bits 0–7, `cbRegs` bits 8–10, `fHasSEH` bit 11, `fUseBP` bit 12, `reserved` bit 13, `cbFrame` bits 14–15 (`fpo.rs:46-100`). Fixture evidence: 7 of 8 x86 FPO records have attribute word `0x0800`; pdbrs follows `winnt.h` (#13).
  - DbiStream.md vs Microsoft `dbi.cpp` and pdbrs: the document says the section contribution substream starts with `Ver60` or `V2`. Microsoft `dbi.cpp` reads any other first word as headerless VC++ 4.0 `SC40` records (20 bytes, no CRCs). pdbrs does the same (`SectionContributionVersion::V40`). LLVM `DbiStream.cpp` returns "Unsupported DBI Section Contribution version" instead.
  - DbiStream.md vs LLVM `DbiStream.cpp` and pdbrs: the document says the EC substream "will not be discussed". LLVM reads it as a `PDBStringTable` (`ECNames.reload`). pdbrs decodes it with `StringTable::parse` (`dbi.rs:347-357`).
  - DbiStream.md vs LLVM `RawTypes.h` (`SrcFileNameNI`, `PdbFilePathNI`: "Name Index"): the document calls `SourceFileNameIndex` and `PdbFilePathNameIndex` offsets "in the names buffer". They are `/names` string table indices. pdbrs does not expose them.
  - DbiStream.md vs pdbrs: the document says `ModIndices` "does not appear to be useful". They are 16-bit per-module start indexes into the file name offsets; they overflow above 65535 entries. pdbrs keeps them raw and uses summed counts (`dbi/sources.rs:12-16`), as LLVM `DbiModuleList::initializeFileInfo`.
  - DbiStream.md says `NumModules` "should match" the DBI module count. pdbrs requires equality and returns `InvalidFormat` otherwise (`dbi.rs:308-324`).
  - Error inside the document: `char NamesBuffer[][NumSourceFiles]` is not a valid layout; the buffer is a run of NUL-terminated strings, and alignment padding can follow the last one (pdbrs keeps the padding in `names()`).
- Undocumented assumptions:
  - Module info records are padded to 4 bytes (`dbi.rs:475-484`).
  - The substream size fields are read in disk order 24, 28, 32, 36, 40, 52 (EC), 48 (optional debug header) (`dbi.rs:48-60`); sizes must be non-negative and the substreams must fill the stream exactly.
  - The optional debug header size must be even; slots after 10 are returned (`dbi.rs:207-213`, `:251-263`).
  - The section map frame 0 with the absolute flag names the absolute segment; addresses in section 0 or past the last section header do not translate (#7).
  - FPO bit layout from `winnt.h` (#13); FRAMEDATA from `cvinfo.h` `tagFRAMEDATA`.

### `llvm/llvm/docs/PDB/ModiStream.md`
- Documents: The module info stream: signature, CodeView symbol substream, C11 and C13 line blocks, and the global refs substream.
- pdbrs: partial — `crates/pdbrs/src/native/dbi/module.rs` splits the stream into all four regions and requires them to fill the stream exactly. Symbols: signature 4 only, decoded by `codeview/symbol*.rs`. C13: framed (8-byte header, data padded to 4) and decoded for `DEBUG_S_LINES` (0xf2), `DEBUG_S_FILECHKSMS` (0xf4), `DEBUG_S_INLINEELINES` (0xf6) and `DEBUG_S_COFF_SYMBOL_RVA` (0xfd) (`lines.rs`, `lines/*.rs`); other kinds and the ignore bit stay raw. Not implemented: C11 lines (raw), global refs (raw bytes after the size), C13 kinds 0xf1, 0xf3, 0xf5, 0xf7–0xfc, 0xfe–0x100.
- Conflicts:
  - ModiStream.md vs `cvinfo.h`: the document guesses that signature 4 means "C13 line information". `cvinfo.h` defines `CV_SIGNATURE_C13 4` as "C13 (vc7.x) zero terminated names", a symbol encoding; `CV_SIGNATURE_C7 1`, `CV_SIGNATURE_C11 2`. pdbrs accepts only 4 and returns `UnsupportedVersion` for other values (`dbi/module.rs:79-86`).
  - ModiStream.md vs pdbrs: the document does not say what a `SymByteSize` of 0 means. pdbrs treats 0 as "no signature and no symbols" and rejects 1–3 bytes (`dbi/module.rs:68-88`).
  - The document gives no C13 subsection layout. pdbrs uses an 8-byte header (`u32` kind, `u32` length) and 4-byte alignment (LLVM `DebugSubsectionRecord`).
- Undocumented assumptions:
  - `DEBUG_S_COFF_SYMBOL_RVA` (0xfd) is a header-less `u32` array of image RVAs, one per COFF symbol table record; values 0 and `0xffffffff` occur (#13).
  - C13 rules: one checksum subsection per module; line block size exactly 12 + n×8 (12 + n×12 with columns); checksum padding to 4 bytes; inlinee signatures 0 and 1 only (#6).
  - Module scope rules: `S_INLINESITE_END` closes only inline sites; the depth bound and parent/end links are checked (`ModuleSymbols`, #5).

### `llvm/llvm/docs/PDB/PublicStream.md`
- Documents: Nothing. The file has only the title "The PDB Public Symbol Stream" (an empty TODO page).
- pdbrs: implemented — `crates/pdbrs/src/native/symbols/psi.rs` (28-byte `PSGSIHDR`: hash size, address map size, thunk count, thunk size, thunk table section, 2 padding bytes, thunk table offset, section count), the shared hash table in `symbols/table.rs`, the address map (`u32` symbol offsets), the thunk map (`thunk_count * 4` bytes) and the section starts (8-byte `SO` entries). `check_records` and `check_address_order` validate references and address order.
- Conflicts:
  - PublicStream.md is empty, so it gives no layout. Facts come from Microsoft `gsi.h`/`gsi.cpp`, LLVM `RawTypes.h`/`PublicsStream.cpp` and `microsoft/pdb-rs` `docs/pdb/globals.md`.
  - LLVM `GlobalsStream.cpp` (shared by `PublicsStream`) vs Microsoft `gsi.cpp` and pdbrs: LLVM always reads a bitmap for `IPHR_HASH` = 4096 buckets and hashes with `% 4096`. With `MINI` (`/DEBUG:FASTLINK`) the producer uses 0x3ffff buckets. `llvm-pdbutil dump --publics` fails on `msvc-x64-fastlink` with "Corrupted publics stream" (#15 evidence). pdbrs selects the bucket count from the Info features.
  - `ms-pdb` 0.1.22 (`globals/name_table.rs`) vs Microsoft `gsi.cpp` and pdbrs: `ms-pdb` uses `hash % buckets` without the 16-bit truncation. MSPDB `GSI1::hashSz` returns `HASH` (`USHORT`). pdbrs uses `(hash_string_v1(name) % count) & 0xffff`; this finds all 836 FASTLINK public records, where the untruncated rule misses 642 (#13).
  - Producer defect, not a document conflict: LLD 22.1.3 places some publics (`__chkstk` and others) in the wrong bucket. pdbrs reports these as inconsistencies (#11).
- Undocumented assumptions:
  - Section starts are read only if bytes follow the thunk map; then they must be exactly `section_count * 8` bytes (LLVM behavior; MSPDB leaves `nSects` uninitialized) (#8).
  - The address map size must be a multiple of 4; address order is checked with `S_PUB32` `(section, offset)` keys (#8).
  - `S_TRAMPOLINE` join rule (#13): thunk section = `thunk_table_section`; index = `(thunk offset − thunk_table_offset) / thunk_size` with no remainder; thunk map entry = section start of the target section + target offset.
  - All GSI hash table rules (see `GlobalStream.md` section) apply.

### `llvm/llvm/docs/PDB/GlobalStream.md`
- Documents: Nothing. The file has only the title "The PDB Global Symbol Stream" (an empty TODO page).
- pdbrs: implemented — `crates/pdbrs/src/native/symbols/gsi.rs`, `symbols/table.rs`, `symbols.rs`. 16-byte `GSIHashHdr` (signature `0xffffffff`, version `0xeffe0000 + 19990810` = `0xf12f091a`, record bytes, bucket bytes); 8-byte `HRFile` records (stored offset is symbol offset + 1; reference count); compressed bucket bitmap of `(count + 1).div_ceil(32)` words; one bucket start per set bit, scaled by 12. Lookup with `BucketCount::bucket_of`. Not implemented: the old uncompressed GSI layout (signature other than `0xffffffff`, `UnsupportedVersion`).
- Conflicts:
  - GlobalStream.md is empty, so it gives no layout. Facts come from Microsoft `gsi.h`/`gsi.cpp`, LLVM `RawTypes.h`/`GlobalsStream.cpp`/`GSIStreamBuilder.cpp`.
  - LLVM `GlobalsStream.cpp` vs Microsoft `gsi.cpp` and pdbrs: LLVM fixes the bucket count at 4096 (`IPHR_HASH`). MSPDB and pdbrs use 0x3ffff buckets for `MINI` PDBs (`symbols.rs:80-115`).
  - `ms-pdb` 0.1.22 vs Microsoft `gsi.cpp` and pdbrs: no 16-bit bucket truncation in `ms-pdb` (see `PublicStream.md`). pdbrs follows MSPDB (#13).
- Undocumented assumptions:
  - The bucket offset scale is 12, the size of the in-memory `HROffsetCalc`, not the 8-byte file record (`table.rs:12-18`, `:36`); LLVM `GlobalsStream.cpp` also divides by 12.
  - Bitmap rules: no bit at or past the bucket count is set (this includes the free-list bucket); offsets start at 0, increase strictly, are below the record count; an empty table may omit the bucket data (MSPDB `CompressBuckets`) or have an empty bitmap (LLVM builder) (#8).
  - Stored offsets must be in `1..=i32::MAX`; reference counts must be in `1..=i32::MAX` (#8).
  - `MINI` → 0x3ffff buckets; bucket = `(hash_string_v1(name) % count) & 0xffff` (#13).

### `llvm/llvm/docs/PDB/HashTable.md`
- Documents: The serialized PDB hash table: size, capacity, present and deleted bit vectors (word count + words), and the key/value pairs in bucket order.
- pdbrs: implemented — `crates/pdbrs/src/native/hash_table.rs` (`HashTable<V>`, `HashTableValue`, `check_placement`). Users: the Info named-stream map (`info.rs`, `u32` values) and `/src/headerblock` (`injected_source.rs`, 40-byte `InjectedSource` values). The TPI/IPI hash adjuster table has a second, separate parser (`types/hash.rs:458-500`, `parse_adjusters`): it checks size/capacity, present bits below capacity, present∩deleted and the count, but not the word-count limit of the deleted vector or placement.
- Conflicts:
  - HashTable.md vs LLVM `NamedStreamMap.cpp` and pdbrs: the document says the hash function "is not discussed further". The named stream map uses `hashStringV1(name)` truncated to 16 bits; `/src/headerblock` uses the key (a `/names` offset) truncated to 16 bits (MSPDB `HcNi`/`HASH`, LLVM `StringTableHashTraits`). Insertion is linear probing from `hash % capacity` (MSPDB `map.h` `Map::find`). pdbrs checks this placement (`hash_table.rs:217-247`).
  - HashTable.md vs LLVM `HashTable::load` and pdbrs: the document does not state the load checks. LLVM and pdbrs require `capacity != 0`, `size <= capacity*2/3 + 1`, popcount(present) = size and present∩deleted = ∅.
  - HashTable.md vs pdbrs: the document's introduction names only the named stream map and the TPI hash adjusters. pdbrs also reads `/src/headerblock` with this format (value size 40).
- Undocumented assumptions:
  - A bit vector can have at most `ceil(capacity/32)` words, and no bit at or after `capacity` can be set (`hash_table.rs:280-308`). LLVM does not check the word count. [INFERENCE from `HashTable.cpp` `readSparseBitVector`.]
  - Missing trailing words read as zero (sparse vector) (`hash_table.rs:319-326`).

### `llvm/llvm/docs/PDB/CodeViewTypes.md`
- Documents: CodeView type records: the 4-byte record prefix, the `LF_POINTER` layout with attribute bit fields and enums, a list of leaf and member kinds (headings only, no layouts) and the `LF_PADn` padding kinds.
- pdbrs: partial — leaves in `crates/pdbrs/src/native/codeview/type_record.rs` (`LF_VTSHAPE`, `LF_LABEL`, `LF_MODIFIER`, `LF_POINTER`, `LF_PROCEDURE`, `LF_MFUNCTION`, `LF_ARGLIST`, `LF_FIELDLIST`, `LF_BITFIELD`, `LF_METHODLIST`, `LF_ARRAY`, `LF_CLASS`, `LF_STRUCTURE`, `LF_UNION`, `LF_ENUM`, `LF_INTERFACE`, `LF_VFTABLE`); members in `codeview/field_list.rs` (`LF_BCLASS`, `LF_VBCLASS`, `LF_IVBCLASS`, `LF_INDEX`, `LF_VFUNCTAB`, `LF_ENUMERATE`, `LF_MEMBER`, `LF_STMEMBER`, `LF_METHOD`, `LF_NESTTYPE`, `LF_ONEMETHOD`); IDs in `codeview/id_record.rs` (0x1601–0x1607); numeric leaves in `codeview/numeric.rs` (integer leaves only). Not implemented from the document's list: `LF_TYPESERVER2` (0x1515), `LF_PRECOMP` (0x1509), `LF_ENDPRECOMP` (0x0014) (raw `Unsupported`), `LF_BINTERFACE` (0x151a, member: `UnsupportedVersion`, which stops the field list). Based-pointer data of `LF_POINTER` stays raw.
- Conflicts:
  - CodeViewTypes.md vs LLVM `CodeViewRecordIO::skipPadding`, `cvinfo.h` and pdbrs: the document says padding is "a decreasing sequence of padding records that terminates with `LF_PAD0`". The sequence counts down to `LF_PAD1` (0xf1); the low 4 bits of the first byte give the byte count (for example `f3 f2 f1`). pdbrs follows this and rejects a standalone `0xf0` run (`field_list.rs:7-9`, `:283-304`).
  - CodeViewTypes.md vs `cvinfo.h` `lfPointerAttr` and pdbrs: the text says `Size` "is a 1-byte value". It is 6 bits (bits 13–18). pdbrs reads `(raw >> 13) & 0x3f` (`type_record/attributes.rs:466-467`).
  - Error inside the document: the attribute diagram labels the top bit position `0x100`; it should be `+0x20` (bit 32). Other labels are hexadecimal bit positions (`0x16` = 22, `0x13` = 19, `0xD` = 13).
  - CodeViewTypes.md vs `cvinfo.h`/LLVM `CodeViewTypes.def` (missing kinds): the document omits `LF_CLASS2` 0x1608, `LF_STRUCTURE2` 0x1609, `LF_UNION2` 0x160a, `LF_INTERFACE2` 0x160b (in LLVM, not in pinned `cvinfo.h`), all numeric leaves 0x8000–0x801c, `LF_MODIFIER_EX`, `LF_ALIAS`, `LF_DIMARRAY`, `LF_STRIDED_ARRAY`, `LF_HLSL`, `LF_VECTOR`, `LF_MATRIX`, `LF_MANAGED`, `LF_DEFARG`, `LF_DERIVED`, `LF_SKIP`, `LF_VFTPATH`, `LF_OEM`/`LF_OEM2`, the member kinds `LF_FRIENDFCN`, `LF_FRIENDCLS`, `LF_VFUNCOFF`, `LF_NESTTYPEEX`, `LF_MEMBERMODIFY`, the `_ST` and `_16t` kinds, and LLVM-only `LF_RANGELIST` 0x120c, `LF_TAGGED_UNION` 0x151e, `LF_TAGGED_UNION_CASE` 0x151f, `LF_TUCASE` 0x1520, `LF_IFC_RECORD` 0x1522. pdbrs decodes none of these; top-level leaves stay raw, member kinds stop the field list.
  - The document is a TODO page for every kind except `LF_POINTER`: no layout for the other leaves and members, and no member-pointer layout (`u32` containing class + `u16` representation; pdbrs decodes it).
- Undocumented assumptions:
  - Each decoded record must end with valid `LF_PAD` bytes only; ID records end with at most three pad bytes (`id_record.rs:21-22`).
  - A member after `LF_INDEX` is `InvalidFormat` (#2).
  - `LF_FUNC_ID`/`LF_MFUNC_ID` may carry an optional 8-byte decorated-name hash after the name (`microsoft/pdb-rs` `records.rs`) (#4).
  - Non-integer numeric leaves are decoded only inside `S_MANCONSTANT` (raw fixed-width bytes; `LF_VARSTRING` as counted bytes) (`codeview/symbol/managed.rs`); `LF_UTF8STRING` is not decoded.

### `llvm/llvm/docs/PDB/CodeViewSymbols.md`
- Documents: CodeView symbol records: record categories (public, global, module), `S_LOCAL` with `LocalSymFlags`, the `S_DEFRANGE*` headers with `LocalVariableAddrRange`/`LocalVariableAddrGap`, `S_DEFRANGE_REGISTER_REL_INDIR`, `S_REGREL32_INDIR` with an example, and a list of other kinds (headings only).
- pdbrs: partial — `crates/pdbrs/src/native/codeview/symbol.rs:540-605` and `codeview/symbol/*.rs`. Decoded from the document's list: `S_PUB32`, `S_GDATA32`, `S_GTHREAD32`, `S_PROCREF`, `S_LPROCREF`, `S_END`, `S_FRAMEPROC`, `S_OBJNAME`, `S_THUNK32`, `S_BLOCK32`, `S_LABEL32`, `S_REGISTER`, `S_BPREL32`, `S_LPROC32`, `S_GPROC32`, `S_REGREL32`, `S_COMPILE2`, `S_UNAMESPACE`, `S_TRAMPOLINE`, `S_SECTION`, `S_COFFGROUP`, `S_CALLSITEINFO`, `S_FRAMECOOKIE`, `S_COMPILE3`, `S_ENVBLOCK`, `S_LOCAL`, `S_DEFRANGE_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL`, `S_DEFRANGE_SUBFIELD_REGISTER`, `S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE`, `S_DEFRANGE_REGISTER_REL`, `S_DEFRANGE_REGISTER_REL_INDIR`, `S_LPROC32_ID`, `S_GPROC32_ID`, `S_BUILDINFO`, `S_INLINESITE`, `S_INLINESITE_END`, `S_PROC_ID_END`, `S_FILESTATIC`, `S_CALLEES`, `S_HEAPALLOCSITE`, `S_FASTLINK`, `S_INLINEES`, `S_CONSTANT`, `S_UDT`, `S_LDATA32`, `S_LTHREAD32`, `S_MANCONSTANT`. Not implemented from the document's list (raw `SymbolData::Unsupported`): `S_GMANDATA` 0x111d, `S_LMANDATA` 0x111c, `S_EXPORT` 0x1138, `S_DEFRANGE` 0x113f, `S_DEFRANGE_SUBFIELD` 0x1140, `S_LPROC32_DPC` 0x1155, `S_LPROC32_DPC_ID` 0x1156, `S_CALLERS` 0x115b, `S_REGREL32_INDIR` 0x1171 (the document gives its layout).
- Conflicts:
  - CodeViewSymbols.md vs DbiStream.md, LLVM and pdbrs: "Symbol records only appear in the PublicStream, GlobalStream, and Module Info Streams". Public and global records are in the symbol record stream; the GSI/PSI streams hold only offsets. pdbrs frames records from the symbol record stream (`symbols.rs:42-78`).
  - Internal contradiction: the document says all global symbols "refer directly to the full symbol record via a module/offset combination", but lists `S_GDATA32`, `S_GTHREAD32`, `S_CONSTANT` and `S_UDT` as global symbols. Only `S_PROCREF`/`S_LPROCREF`/`S_DATAREF`/`S_TOKENREF` use module/offset; the others are full records.
  - CodeViewSymbols.md vs pdbrs: for `S_PUB32` the document says the DBI section map gives the module of an address. The section contribution substream gives the module; the section map gives segment descriptors. pdbrs translates addresses with section headers and the section map (`address.rs`) and keeps the module index in `SectionContribution::module`.
  - CodeViewSymbols.md vs `cvinfo.h` and pdbrs: a scope runs "up until the next `S_END` record". Scopes nest; `S_INLINESITE` ends with `S_INLINESITE_END`, and `S_*PROC32_ID` can end with `S_PROC_ID_END`. pdbrs follows the parent/end links and checks the closer kind (`ModuleSymbols`, #5).
  - Error inside the document: the `S_REGREL32_INDIR` example table shows name bytes `7900` ("y") but the decoded row says `"a"`.
  - LLVM `CodeViewRecordIO::mapVectorTail` vs pdbrs (def-range gaps): LLVM stops reading gaps at the first gap whose first byte is ≥ 0xF0 (it treats it as `LF_PAD`). pdbrs returns all gaps; 47 sample records had hidden gaps (#5).
  - CodeViewSymbols.md vs pinned `cvinfo.h`: `S_FASTLINK` 0x1167, `S_INLINEES` 0x1168, `S_REGREL32_INDIR` 0x1171 and `S_DEFRANGE_REGISTER_REL_INDIR` 0x1177 are not in the pinned `cvinfo.h` (it ends at `S_RECTYPE_LAST` 0x1165; `S_RECTYPE_MAX` 0x1166 is a marker). LLVM `CodeViewSymbols.def` uses 0x1166 for `S_FRAMEREG`. pdbrs decodes `S_FRAMEREG` 0x1166 as a record, not as the old marker.
  - The document gives no hex value for `S_GDATA32` (0x110d) and no layout for most listed kinds (TODO headings).
  - Missing kinds vs `cvinfo.h`/LLVM `.def`: the document omits kinds that pdbrs decodes (`S_SKIP` 0x0007, `S_OEM` 0x0404, `S_MANSLOT` 0x1120, `S_GMANPROC` 0x112a, `S_LMANPROC` 0x112b, `S_TOKENREF` 0x1129, `S_ARMSWITCHTABLE` 0x1159, `S_FRAMEREG` 0x1166, `S_REGREL32_ENCTMP` 0x1179) and kinds that neither decodes (`S_ANNOTATION`, `S_SEPCODE`, `S_INLINESITE2`, `S_POGODATA`, `S_REF_MINIPDB`, `S_PDBMAP`, HLSL kinds, `S_MANYREG*`, `S_HOTPATCHFUNC` 0x1169, `S_ASSOCIATION` 0x117c, `S_BPREL32_INDIR` 0x1170, `S_GPROC32EX` 0x1172 and others).
- Undocumented assumptions:
  - `S_FASTLINK` (0x1167; LLVM comment "also known as S_REF_MINIPDB2"): `u32` value, `u16` flags, NUL-terminated name; flag bits 0 local, 1 data, 2 UDT, 3 label, 4 constant (from `REFMINIPDB`) and bit 6 `NAMESPACE` (observed). Value: object-file type index if the UDT bit is set, else the 1-based COFF section number of the `.debug$S` section (#13).
  - `S_FRAMEREG` (0x1166): `u16` register then 2 zero bytes (#13).
  - `S_REGREL32_ENCTMP` (0x1179): the `S_REGREL32` layout (#13).
  - `S_DEFRANGE_REGISTER_REL_INDIR`: `OffsetInUdt` is `i32` (LLVM `little32_t`); `S_REGREL32` offset is `u32` (#5).
  - Each record ends with fewer than 4 zero padding bytes; `S_COMPILE3` accepts any count of zero bytes after the version string (ml64 writes an extra NUL); `S_SECTION`/`S_CALLSITEINFO` reserved fields must be zero (#5).
  - Thunks with a non-standard ordinal keep the variant bytes raw (#5).
  - Managed records follow `cvinfo.h` `#pragma pack(1)`; `S_MANSLOT` holds a metadata token, not a TPI index; `S_OEM` with GUID `c6ea3fc9-59b3-49d6-bc25-0902bbabb460` decodes Roslyn `MD2` and `asyncMethodInfo` attributes (#14).

### `llvm/llvm/LICENSE.TXT`
- Documents: The Apache License v2.0 with LLVM Exceptions for the copied LLVM documents.
- pdbrs: not applicable — license text, no format content.
- Conflicts: None found.
- Undocumented assumptions: None.
