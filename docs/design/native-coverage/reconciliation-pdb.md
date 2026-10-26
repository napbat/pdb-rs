# Native coverage: reconciliation of the Microsoft pdb-rs PDB documents, the original Microsoft PDB project and the manifest

This page is part of the [native coverage](../native-coverage.md) accounting. It covers `docs/reference/microsoft-pdb-rs/docs/pdb/`, the top-level `microsoft-pdb-rs/docs/` files and licenses, `docs/reference/microsoft-pdb/` and `SOURCES.json`.
Each section names one imported document. Paths are relative to `docs/reference/`. The fields are:

- **Documents**: the format items that the document describes.
- **pdbrs**: `implemented` (pdbrs reads every item that the document describes), `partial`, `not implemented` or `not applicable` (no format content), with the source files.
- **Conflicts**: each difference between the document and pdbrs, or between the document and another source (`cvinfo.h`, LLVM source, `ms-pdb`, other documents). Errors inside the document are conflicts too.
- **Undocumented assumptions**: layouts and rules that pdbrs uses but the document does not state, with the issue that records the evidence.

The comparison used `main` at `8b77fb2`, `cvinfo.h` at `805655a2`, LLVM at `b9f2235f` and the `ms-codeview` 0.1.7, `ms-pdb` 0.1.22, `ms-pdb-msf` 0.1.7 and `ms-pdb-msfz` 0.1.10 crates. "Inference, not tested" marks a statement that comes from reading source code only.

### `microsoft-pdb-rs/docs/pdb/tpi_stream.md`
- Documents: the TPI stream header (56 bytes), version values, type index range, record framing and padding, the hash stream buffers (hash values, index offsets, hash adjusters) and part of the type record hash rule.
- pdbrs: implemented — `crates/pdbrs/src/native/types.rs` decodes the V80 (20040203) header, frames records and checks the record count; `types/hash.rs` decodes and verifies hash values, index offset pairs and hash adjusters (`type_record_hash`, `TypeHashes::verify`); `types/lookup.rs` (`TypeTable`) and `types/forward.rs` (`ForwardRefs`) give lookup and forward-reference resolution. Typed leaves are in `codeview/type_record.rs` and `codeview/field_list.rs`. Not implemented: versions V40/V41/V50/V70 (rejected), the auxiliary hash stream (`hash_aux_stream_index`, only checked for existence in `reader.rs`), `LF_*2` UDT leaves in the hash rule (`UnsupportedVersion`).
- Conflicts:
  - `header_size`: the document says "use `header_size`, it can be larger than 56, multiple of 4". pdbrs requires `header_size == 56` (`types.rs` `HEADER_SIZE`).
  - Stream length: the document allows `header_size + type_record_bytes <= stream size` (extra data permitted). pdbrs rejects bytes after the records.
  - Hash function, internal: the "Hash Value Substream" section says every hash is CRC-32 over the full record. The next section says global UDTs use `LHashPbCb` of the name. pdbrs follows LLVM `TpiHashing.cpp`: name hash (`hash_string_v1`) for non-forward, non-scoped, non-anonymous UDTs; unique-name hash for scoped UDTs with a unique name; CRC (`hash_buffer_v8`) for forward references and anonymous UDTs; `hash_string_v1` of the 4-byte UDT index for `LF_UDT_SRC_LINE`/`LF_UDT_MOD_SRC_LINE`; CRC for all other records.
  - Scoped UDTs: the document lists "scoped bit set" as a reason that the record is not a global UDT (implied CRC). LLVM `getHashForUdt` and pdbrs hash the unique name of a scoped UDT that has one.
  - `LF_ALIAS`: the document includes it in the UDT name-hash rule. LLVM `hashTypeRecord` and pdbrs do not; pdbrs has no `LF_ALIAS` decoder.
  - Anonymous rule: the document tests the name only. LLVM `isAnonymous` applies only when `HasUniqueName` is set.
  - Padding: the document says writers pad with `0xF1` to a 2-byte boundary. `cvinfo.h` defines `LF_PAD0`..`LF_PAD15` (`0xF0 + n`) with descending runs (`F3 F2 F1`), and the document's own example shows `f2 f1` at 0x86. pdbrs validates descending `LF_PAD` runs (`codeview/field_list.rs`, `codeview.rs`).
  - Hash value invariant typo: "`type_index_end – type_index_end`" must be `type_index_end - type_index_begin`.
  - Index offset example, internal: the table gives offsets 0x8450 then 0x604C, which violates the document's own "stream_offset strictly increasing" invariant. pdbrs requires strictly increasing pairs inside the records.
  - Hash adjusters: the document says the structure is "not well-understood". pdbrs decodes it as a serialized hash table (LLVM `HashTable::load`): keys are `/names` offsets and values are type indices (mspdb `tpi.cpp` `mpnitiHead`, LLVM `DumpOutputStyle.cpp`; evidence #3). LLVM `TpiStream.md` calls the keys hash values, which conflicts with both implementations.
  - Version table: the document lists `impv50Interim` = 19960307. LLVM `PdbRaw_TpiVer` has no such value. pdbrs rejects every version except 20040203.
- Undocumented assumptions:
  - Hash key size must be 4 and the bucket count must be in 0x1000..=0x40000 (LLVM `MinTpiHashBuckets`/`MaxTpiHashBuckets`; `types.rs`).
  - Each stored hash value must be less than the bucket count (`types/hash.rs`).
  - Microsoft writes hash buffer offset 0 and length 0xFFFFFFFF when there is no hash stream (`types.rs` `HashBuffer` doc).
  - The three hash buffers must tile the hash stream without gaps, overlaps or trailing bytes (#3).
  - Forward-reference resolution follows LLVM `findFullDeclForForwardRef` (#2).

### `microsoft-pdb-rs/docs/pdb/relationships.md`
- Documents: the reference graph between PDB structures (stream indexes, byte offsets, record indexes) as a Mermaid diagram.
- pdbrs: partial — pdbrs decodes and checks many of these references: DBI stream IDs (`reader.rs` `require_stream`), GSI/PSI offsets into the symbol record stream (`symbols.rs` `RecordOffsets`, `check_records`), `S_PROCREF` module/offset joins and `/names` joins (`crates/pdbrs/examples/native/`). pdbrs has no encoder, so it does not rewrite references.
- Conflicts:
  - `dbi_sources --NameIndex--> names_stream` is wrong. DBI source file offsets point into the substream's own `names_buffer`, not into `/names` (`dbi_sources.md` itself; LLVM `DbiModuleList.cpp`; pdbrs `dbi/sources.rs`).
  - `c13line --"Mod#,File"--> dbi_sources`: no C13 subsection that pdbrs decodes stores a module index or a DBI file index. C13 file checksums store `/names` offsets (`lines/checksums.rs`). Inference, not tested: The edge has no on-disk field.
  - "The reference graph is acyclic": `/TMCache` version 2 type map streams can name a precompiled-header cache stream, and stream references in Info and DBI can name any stream; the document gives no proof. Inference, not tested: Not a format defect, but an unverified claim.
  - Missing edges (all observed in pdbrs decoders): GSS `S_PROCREF`/`S_LPROCREF`/`S_TOKENREF` → module symbol streams (1-based module index + offset); section contributions → module index; PSI address map/thunk map/section starts; IPI `LF_UDT_MOD_SRC_LINE` → `/names`; `S_FILESTATIC` → `/names`; new-FPO frame programs → `/names` (`fpo.rs`); TPI hash adjusters → `/names`; `/src/headerblock` → `/names` and `/src/files/*`; module streams → DBI EC names; `/TMCache` → type map streams and precompiled-header cache streams (#13).
- Undocumented assumptions:
  - pdbrs treats the module index in `S_PROCREF` as 1-based (`codeview/symbol.rs` `ProcedureReference::module_index`, #5).
  - The `native` example joins `S_TRAMPOLINE` to the PSI thunk map by `(thunk offset - thunk_table_offset) / thunk_size` (#13).

### `microsoft-pdb-rs/docs/pdb/pdbi_stream.md`
- Documents: the PDB Information stream header, version values, the named stream map (hash table with present/deleted bit vectors), the obsolete name-index tail and feature codes.
- pdbrs: implemented — `crates/pdbrs/src/native/info.rs` decodes version 20000404 only, signature, age, GUID, the string buffer, the hash table (`hash_table.rs`), `niMac` and feature codes. Feature codes stay raw u32 values; `MINI` selects the GSI/PSI bucket count (`symbols.rs`). Not implemented: versions VC2..VC70Dep and VC80/VC110/VC140 as header versions (rejected with `UnsupportedVersion`).
- Conflicts:
  - Tail field: the document says `num_name_index` is a count followed by `NameIndex name_indexes[num_name_index]` ("expected always zero"). mspdb `NMTNI` (`PDB/include/nmtni.h`) writes one `u32 niMac` (next name index). pdbrs reads one `u32` as `Info::next_name_index()` and starts features after it (#9 defect fix). ms-pdb `pdbi.rs` rejects a non-zero `niMac`; pdbrs accepts any value.
  - Bit-vector invariant: "`present_mask_size >= num_hashes * 4`" is wrong. `present_mask_size` counts u32 words; the example (1 word, capacity 14) violates it. pdbrs (LLVM rule): at most `ceil(capacity / 32)` words and no bit at or after the capacity (`hash_table.rs`).
  - "`present_mask_size == deleted_mask_size`": not required. LLVM writes an empty deleted vector. pdbrs reads each vector with its own word count.
  - Example, internal: `present_mask` bytes `f4 05` give bits {2, 4, 5, 6, 7, 8, 10}. The document prints `0 0 0 1 1 1 1 1, ...` (bits 3..7), which is the expansion of 0xF8.
  - Encoding steps, internal: "`num_hashes * 1.5`" must be `num_names * 1.5`; "append a new record to `keys_data`" must be `hash_entries`; entries are serialized in bucket order (present bits ascending), not insertion order (LLVM `HashTable::commit`; pdbrs `hash_table.rs` reads "one key and one value for each present bucket, in bucket order").
  - Capacity rule: the document only requires `num_hashes >= num_names`. LLVM `HashTable::load` and pdbrs require `size <= capacity * 2 / 3 + 1` and capacity ≠ 0.
  - Hash function: the document says `hash(k)` and does not name it. mspdb `NMTNI::hash` and pdbrs use `hash_string_v1(name) & 0xffff` (16-bit truncation), then `% capacity` (`info.rs`).
  - Struct field names are inconsistent: `num_present`/`num_deleted`/`name_count` do not match `present_mask_size`/`deleted_mask_size`/`num_names`.
  - Versions: the table presents VC80, VC110 and VC140 as header versions. All observed producers write 20000404 and place 20091201/20140508 in the feature list; pdbrs accepts only 20000404.
- Undocumented assumptions:
  - Lookup stops at the first bucket that is neither present nor deleted; pdbrs checks with `check_placement` that each entry is reachable (#9).
  - No bucket can be both present and deleted; the present popcount must equal `size` (`hash_table.rs`).
  - Two entries with the same name are rejected (`info.rs`).
  - The feature area length must be a multiple of 4 (`info.rs`).
  - The keys buffer can hold strings that no entry names (the example holds `sourcelink$2` twice and `embedspd` without an entry). pdbrs only resolves the names that entries reference.
  - Presence of the IPI stream: LLVM uses the VC140 feature; pdbrs loads stream 4 when the directory has it (`reader.rs` `ids`).

### `microsoft-pdb-rs/docs/pdb/pdb.md`
- Documents: an overview of the PDB: containers (MSF, MSFZ), fixed streams 1-4, DBI substreams, GSS/GSI/PSI, module streams, `/names`, named streams (NatVis, source files, Source Link), hashing, relationships and mini PDBs.
- pdbrs: partial — MSF 7.0 read and container write (`native/msf.rs`, `native/msf/`), Info (`info.rs`), DBI (`dbi.rs`, `dbi/`), TPI/IPI (`types.rs`), module streams (`dbi/module.rs`), GSI/PSI (`symbols.rs`, `symbols/`), `/names` (`names.rs`), named streams (`auxiliary.rs`, `injected_source.rs`). Not implemented: MSFZ (no `native/msfz.rs`; `format.rs` `probe` has no MSFZ kind and returns `InvalidSignature`, #20); `srcsrv` (classified as `Other`); NatVis XML semantics.
- Conflicts:
  - NatVis and source contents: the document says they are named streams with the file name as the key (`my_types.natvis`, `my_generated_code.cpp`). In MSVC PDBs they are injected sources: `/src/headerblock` lists entries and each content stream is `/src/files/<vname>` (pdbrs `injected_source.rs`; 16 NatVis entries in the samples, #9).
  - "IPI Stream ... contains various ids instruction addresses to source locations": garbled; the IPI holds ID records (`LF_FUNC_ID`..`LF_UDT_MOD_SRC_LINE`).
  - Stream list omits the TPI/IPI hash streams, the DBI optional debug streams and the `/LinkInfo`, `/TMCache`, `/UDTSRCLINEUNDONE` and `sourcelink` named streams, which pdbrs decodes.
- Undocumented assumptions:
  - `/TMCache` layout (version 0x20200229, stream table, 16-byte entries) and type map stream versions 1 and 2 come from `mspdbcore.dll` `DBI1::DumpTMCache` (#13).
  - `/LinkInfo` version 2 layout (mspdb `pdb.h` `struct LinkInfo`, #9).
  - `/src/headerblock` version 19980827, 40-byte entries, bucket placement `key & 0xffff` (#9).
  - .NET document records (compression byte 101) (#14).

### `microsoft-pdb-rs/docs/pdb/names_stream.md`
- Documents: the `/names` string table: signature 0xEFFEEFFE, hash version 1 (`LHashPbCb`) or 2 (`LHashPbCbV2`), string buffer, bucket array, name count, query and build procedures.
- pdbrs: partial — `crates/pdbrs/src/native/names.rs` parses both versions, resolves offsets (also offsets into the middle of a name), checks that every non-empty bucket names a NUL-terminated string and that the name count equals the used bucket count. Version 1: `find` and `verify_buckets` hash with `hash_string_v1 % bucket_count` and linear probing. Version 2: hashing is not implemented; `find` and `verify_buckets` return `UnsupportedVersion` (#7). The DBI EC substream uses the same decoder (`dbi.rs` `ec_names`).
- Conflicts:
  - Version 2 hashing: the document names `LHashPbCbV2` (mspdb `misc.h` `HasherV2`: seed 0xB170A1BF, `hash += w; hash += hash << 10; hash ^= hash >> 6` per u32 then per byte, final `hash * 1664525 + 1013904223`; LLVM `hashStringV2`). pdbrs has no V2 hash (`hash.rs`). ms-pdb `names.rs` rejects any version other than 1; pdbrs accepts version 2 without hash checks.
  - Query procedure, internal: step 3 sets `p = h` without `% num_hashes`; step 4.4 increments `i` (the `NameIndex`) instead of `p`; step 4.5 compares `i == h`. The probe must be `p = (p + 1) % num_hashes` and stop when `p` returns to the start.
  - Padding, internal: "the size of `string_data` must be a multiple of 4" contradicts "There is no alignment requirement for `strings_size`" and the example value 0x713C9.
  - Offset, internal: "empty string ... at stream offset 8" is wrong; the header is 12 bytes, so `string_data` starts at offset 12 (the example section says 12).
  - NameIndex users: the document says `LF_UDT_SRC_LINE` and "`LF_MOD_UDT_SRC_LINE`" contain `NameIndex` values. The `cvinfo.h` name is `LF_UDT_MOD_SRC_LINE`. Only that record has a `NameIndex`; `LF_UDT_SRC_LINE` has an `IdIndex` of an `LF_STRING_ID` (`cvinfo.h` `lfUdtSrcLine`; pdbrs `codeview/id_record.rs` `UdtSourceLine { source_file: IdIndex }`).
  - Omitted NameIndex users that pdbrs resolves: `S_FILESTATIC`, new-FPO frame programs, injected-source names, TPI hash adjuster keys.
  - Open TODO in the document: "How does this handle hashes that are zero?" (an empty slot is the value 0, not hash 0; offset 0 is the empty string, which is never in a bucket).
- Undocumented assumptions:
  - The table must end exactly after the name count (`names.rs` rejects trailing bytes).
  - Version 1 uses the full 32-bit `LHashPbCb` value modulo the bucket count (no 16-bit truncation), as in LLVM `PDBStringTable::getIDForString`.

### `microsoft-pdb-rs/docs/pdb/msf.md`
- Documents: the MSF container: Big MSF header (52-byte prefix + page map array), stream directory, nil streams, directory page map, free page maps and intervals, Small MSF header and directory, determinism rules, an annotated example.
- pdbrs: implemented (Big MSF read and write) — reader `crates/pdbrs/src/native/msf.rs`, `msf/directory.rs`, `msf/free_page_map.rs`, `msf/old_directory.rs`; writer `msf/write.rs`, `msf/write/layout.rs`, `msf/write/emit.rs`, `msf/write/builder.rs`, `msf/write/file.rs` (`copy_exact`, `Msf::repack`, `MsfBuilder`). Small MSF is recognized and rejected with `UnsupportedVersion` (`msf.rs`, `format.rs`), as the document permits. Not implemented: Small MSF decoding (#16), the 2-phase commit / in-place edits.
- Conflicts:
  - Page sizes: the document allows 512-65536 for Big MSF. The pdbrs reader accepts 512-65536; the pdbrs writer rejects 65536 with `UnsupportedVersion` and writes 512-32768 (`write/layout.rs` `MAX_WRITE_PAGE_SIZE`), because DIA2Dump (`HRESULT 806D000C`), `llvm-pdbutil` 22.1.6 ("Unsupported block size", LLVM `isValidBlockSize` 512-32768) and `link.exe` 14.51 (`LNK1399`) reject 65536 (#15). `ms-pdb-msf` accepts 512-65536.
  - FPM state of the directory, internal: the table marks Stream Directory and page map pages busy (0), but the next paragraph says "pages that are assigned to the Stream Directory are considered 'free'". pdbrs reader rejects a free directory or page-map page; the pdbrs writer marks them allocated, as LLVM `MSFBuilder` does (`free_page_map.rs`, `write.rs`).
  - Determinism "There are no nil streams": `Msf::repack` keeps nil streams and `MsfBuilder::push_nil` creates them (`write/builder.rs`), because stream IDs must stay stable (#15).
  - Header page map limit: the document does not limit `big_stream_pages_map`. pdbrs requires the array to fit in page 0 (`(page_size - 52) / 4` entries; reader `directory.rs`, writer `LimitExceeded`). LLVM `validateSuperBlock` accepts only one page-map page ("Too many directory blocks"); pdbrs reads and writes several (#15 tests), so a pdbrs file with more than one page-map page fails in LLVM.
  - Example, internal: "`big_stream_pages_map` at offset 0x3c" is wrong; the dump shows `ba 51 00 00` at offset 0x34 (52), which is also pdbrs `PAGE_MAP_OFFSET`.
  - Example, internal: `num_pages` is given as "20,923 (0x51B)"; the correct hex is 0x51BB. The derived FPM size "164 bytes" uses the wrong value; `ceil(20923 / 8)` = 2616 bytes.
  - Interval table, internal: the column formulas "`(page_size + 1) * page_size`" omit the interval term; the byte offset of FPM1 in interval `k` is `(k * page_size + 1) * page_size` (the table values are correct).
  - Open sections: Small MSF `active_fpm` constraints (TODO), "Location on disk - Small MSF" (empty), "2-phase commit protocol" (TODO).
  - Stream 0: the document says to ignore the Old Stream Directory. pdbrs decodes it as a snapshot (`OldDirectory`), and the writer default writes stream 0 as a present stream of length 0 (`OldDirectoryPolicy::Empty`); MSVC `link.exe` writes a 48-byte previous directory, LLD and `ms-pdb-msf` write an empty stream 0 (#15). `Preserve` requires the source page size (#15 addendum).
- Undocumented assumptions:
  - The FPM bitmap is contiguous across pages `active + k * page_size` (LLVM `getFpmStreamLayout`, mspdb `MSF_HB::serializeFpm`; `free_page_map.rs`).
  - Stream 0 pages can be free or allocated; unreferenced allocated pages are reported as leaks, not errors (`check_free_page_map`, #9).
  - Directory size must be at least 8 bytes and is limited to 64 MiB by default (`MsfLimits`).
  - No page reference can name page 0 or pages 1 and 2 of any interval (`is_reserved_page`).
  - Writer layout: stream pages in stream ID order, then directory pages, then page-map pages, ascending; FPM pages are added when the last page starts an interval; FPM bytes after the bitmap are 0xFF; inactive FPM2 is all 0xFF; unused bytes of the header page and of every last page are zero (#15, `write.rs` `WritePlan` doc).
  - The writer rejects a stream size of 0xFFFFFFFF or more, a page count that does not fit in `u32`, and a directory above `max_directory_bytes`, before it writes any byte (#15).

### `microsoft-pdb-rs/docs/pdb/msfz.md`
- Documents: the MSFZ container: 80-byte header, signature, version 0, variable-length stream directory of 12-byte fragments, fragment location encoding, 20-byte Chunk Table entries, compression codes, overlap rules, validation and read procedures.
- pdbrs: not implemented — no MSFZ code exists; `crates/pdbrs/src/format.rs` `probe` does not know the MSFZ signature and returns `InvalidSignature` (#20). `docs/design/pdb.md` plans `native/msfz.rs` and `native/msfz/` and says to follow `ms-pdb-msfz` where it differs from this specification.
- Conflicts:
  - Directory size check (document vs `ms-pdb-msfz` `reader.rs`): the validation procedure checks `num_streams * size_of::<StreamEntry>() == stream_dir_size_uncompressed`. The layout sections define no `StreamEntry`, and the directory is variable-length; the reference reader does not apply the check (#20).
  - Read procedure (document vs its own layout): it names a per-stream `compression` field, `COMPRESSION_CHUNKED`, `location` and `uncompressed_size` of a `StreamEntry`. These do not exist in the fragment layout (#20).
  - Compression (document vs `ms-pdb-msfz` `lib.rs`): "the only supported algorithm is Zstd", but the constants define `COMPRESSION_DEFLATE = 2`, and the reference implementation reads and writes raw DEFLATE chunks (`Compression::Deflate`) (#20).
  - Wording (#20): "`first_chunk` is the index, within the Chunk Table, of the first fragment" must say "first chunk".
  - Example bytes, internal: the location of stream 3 is `e0 10 00 00 80 00 00 e5`. As a little-endian `u64` this is 0xE5000080_000010E0: bit 63 set, chunk index 0x65000080, not 0xE5. The high word bytes are in the wrong order (`e5 00 00 80` gives chunk 0xE5).
  - Reserved bits (document vs `ms-pdb-msfz`): the document requires bits 48-62 of an uncompressed location to be 0. `ms-pdb-msfz` masks the offset with `(1 << 48) - 1` and does not check the reserved bits.
  - Validation (document vs `ms-pdb-msfz`): the document requires non-zero `uncompressed_size`/`compressed_size` per chunk, file-range checks and full overlap checks. The reference reader checks only signature, version 0, `num_streams >= 1`, `chunk_table_size == num_chunks * 20`, compressed == uncompressed directory size when the directory is uncompressed, chunk index < `num_chunks` and `offset_within_chunk < uncompressed_size` of the first chunk. It does not check overlaps or chunk sizes.
  - Rules in `ms-pdb-msfz` that the document does not state: a fragment location of 0xFFFFFFFF_FFFFFFFF is invalid; a fragment size of 0xFFFFFFFF after the first fragment is invalid.
  - "Uncompressed streams ... stored in a single contiguous sequence of bytes" contradicts the multi-fragment model of the same document.
  - Writer behavior not in the document (`ms-pdb-msfz` `writer.rs`): 16-byte alignment of written pieces, default chunk threshold 1 MiB, default Zstd, stream 0 reserved as zero-length, and `MIN_FILE_SIZE_16K` (0x4000) padding as a workaround for the PDZ decoder in DIA (`pdbtool pdz-encode --pad16k`).
  - Large streams: the document allows streams larger than 4 GiB. MSF 7.0 and the PDB stream structures use 32-bit sizes, so such a stream cannot convert to MSF 7.0. Inference, not tested: pdbrs conversion (#20) must reject it.
- Undocumented assumptions: None. (pdbrs has no MSFZ code. #20 records the decisions: follow `ms-pdb-msfz`; zstd through `ruzstd`, raw DEFLATE through `miniz_oxide`, only inside `compression.rs`.)

### `microsoft-pdb-rs/docs/pdb/mini_pdb.md`
- Documents: mini PDBs (`/DEBUG:FASTLINK`), the `MinimalDebugInfo` feature code and its effect on the GSI/PSI bucket count (0x1000 or 0x3FFFF).
- pdbrs: partial — `crates/pdbrs/src/native/symbols.rs` `BucketCount::from_features` selects 0x3FFFF when the Info features contain 0x494E494D (`MINIMAL_DEBUG_INFO_FEATURE`); `bucket_of` uses `(hash_string_v1(name) % count) & 0xffff`. `S_FASTLINK` (0x1167) is decoded (`codeview/symbol.rs`, #13). C13 `DEBUG_S_COFF_SYMBOL_RVA` (0xFD) is decoded as a `u32` RVA array (`lines/coff_symbol_rva.rs`). Not implemented: `S_REF_MINIPDB` (0x1160) and `S_PDBMAP` (0x1161) decoders; reading the object files that FASTLINK records point to.
- Conflicts:
  - Bucket rule (document vs mspdb and pdbrs): the document gives only the bucket count. mspdb `GSI1::hashSz` (`PDB/dbi/gsi.cpp`) returns `HASH` (`USHORT`, `pdbtypdefs.h`), so the producer bucket is `(hash % 0x3FFFF) & 0xFFFF`; buckets 0x10000 and higher are never used. pdbrs follows this rule; with it, lookup finds all 836 public records of `msvc-x64-fastlink`, against 194 with the plain modulo (#13).
  - `ms-pdb` 0.1.22 vs mspdb: `globals/name_table.rs` uses `hash_mod_u32(name, num_buckets)` without the 16-bit truncation, so its lookup misses the same 642 records (#13).
  - Evidence record, internal: one #13 comment says "lookup uses the bucket `(hash % count) & 0xffff`, as `ms-pdb` does". The #13 decision comment and the `ms-pdb` source show that `ms-pdb` does not truncate.
  - The document does not give the feature value (0x494E494D, "MINI"); `pdbi_stream.md` gives it.
  - The document does not mention FASTLINK-specific records (`S_FASTLINK`/`S_REF_MINIPDB2` 0x1167, `S_REF_MINIPDB` 0x1160) or C13 0xFD.
- Undocumented assumptions:
  - `S_FASTLINK` layout (#13): `u32` value, `u16` flags, NUL-terminated name (= `REFMINIPDB` without `imod`). Flags: bit 0 local, bit 1 data, bit 2 UDT, bit 3 label, bit 4 constant (from `REFMINIPDB`); bit 6 (0x40) `NAMESPACE` from fixture observation. If the UDT bit is set, the value is an object-file type index; otherwise it is the 1-based COFF section number of the `.debug$S` section in the object file. The first record of each module has value 0, flags 0 and an empty name.
  - `DEBUG_S_COFF_SYMBOL_RVA`: entry `i` is the image RVA of COFF symbol `i` of the object file; 0 = no image address; 0xFFFFFFFF = external symbol resolved to an absolute value (kept raw) (#13).
  - `llvm-pdbutil` 22.1.6 reports "Corrupted publics stream" for the FASTLINK publics layout (#15 evidence).

### `microsoft-pdb-rs/docs/pdb/module_stream.md`
- Documents: the module stream regions (symbols with a 4-byte prefix, C11 lines, C13 lines, global refs with a size prefix).
- pdbrs: partial — `crates/pdbrs/src/native/dbi/module.rs` frames all four regions and requires the symbol signature 4 (C13). Symbols decode through `codeview/symbol.rs`; C13 subsections decode through `lines.rs` (0xF2, 0xF4, 0xF6, 0xFD typed; others raw). Not implemented: C11 line decoding (raw bytes); global refs are a raw region, not a typed offset array, and are not checked against the GSS.
- Conflicts:
  - Global refs size field: the document says `global_refs` can have length zero, "meaning this Module Stream does not contain Global Refs". pdbrs (as LLVM `ModuleDebugStreamRef::reload`) requires the 4-byte size field after the C13 region and fails with `UnexpectedEof` if it is absent.
  - Trailing data: `dbi_modules.md` allows region sizes whose sum is less than the stream size. pdbrs rejects bytes after the global references (`InvalidFormat`).
  - The document does not give the symbol prefix value. pdbrs accepts only `CV_SIGNATURE_C13` = 4 and returns `UnsupportedVersion` for other values (for example C11 = 1, C7 = 2).
- Undocumented assumptions:
  - A symbol size of 0 means that the region and its signature are absent; a size of 1 to 3 is `InvalidFormat` (`dbi/module.rs`).
  - A module with more than one checksum subsection is rejected (`lines/module.rs`, #6).
  - The C13 ignore flag 0x80000000 gives a raw `Ignored` subsection (`lines.rs`).

### `microsoft-pdb-rs/docs/pdb/ipi_stream.md`
- Documents: the IPI stream (stream 4): same header, framing and hash structures as TPI; only ID records; sequential lookup by `ItemId`.
- pdbrs: implemented — `crates/pdbrs/src/native/types.rs` (shared `TypeStream`), `codeview/id_record.rs` (`IdRecord`: `LF_FUNC_ID`..`LF_UDT_MOD_SRC_LINE` 0x1601-0x1607) and `codeview/id_record/table.rs` (`IdTable`: lookup, string resolution, reference validation). `reader.rs` `ids` loads stream 4.
- Conflicts:
  - Hash values: the document says the IPI hash substream has "the same structure" as TPI and points to `tpi_stream.md`, which says CRC-32 over the full record. LLVM `hashTypeRecord` and pdbrs hash `LF_UDT_SRC_LINE`/`LF_UDT_MOD_SRC_LINE` with `hash_string_v1` of the 4-byte UDT index, not with the CRC (`types/hash.rs`, #3).
  - Lookup: the document describes only a linear scan. pdbrs uses an offset table (`TypeTable`, 4 bytes per record) or the index-offset buffer (`TypeHashes::locate`).
  - The link `../codeview/items/items.md` is outside this reconciliation; `data_types.md` links `ipi_stream.md#itemid`, but this file has no `ItemId` heading.
- Undocumented assumptions:
  - `LF_FUNC_ID`/`LF_MFUNC_ID` can carry an optional 8-byte decorated-name hash after the name (from `microsoft/pdb-rs` `records.rs`; #4).
  - String resolution follows substring lists with a depth bound of 8 and a visit cap (`MAX_STRING_DEPTH`, #4).
  - The document does not say how a reader knows that the IPI exists. LLVM uses the VC140 feature; pdbrs uses the presence of stream 4.

### `microsoft-pdb-rs/docs/pdb/hashing.md`
- Documents: the 32-bit PDB string hash (`LHashPbCb`) and the CRC-32 buffer hash (`SigForPbCb`).
- pdbrs: partial — `crates/pdbrs/src/native/hash.rs` `hash_string_v1` (mspdb `Hasher::lhashPbCb`, returns the unreduced value; callers apply `% m` and, where needed, `& 0xffff`) and `hash_buffer_v8` (CRC-32, polynomial 0xEDB88320 reflected, initial value 0, no final XOR). Not implemented: `LHashPbCbV2` (`hash_string_v2`).
- Conflicts:
  - CRC definition (document vs mspdb/LLVM/pdbrs): the document calls `SigForPbCb` "the standard, well-known CRC-32". Standard CRC-32 uses initial value 0xFFFFFFFF and a final XOR 0xFFFFFFFF. `SigForPbCb(pb, cb, sig)` (`langapi/shared/crc32.h`) uses the caller's `sig` as the initial value and no final XOR; PDB callers pass 0 (LLVM `hashBufferV8` = `JamCRC(Init=0)`; pdbrs initial and final value 0).
  - Reference link, internal: the `HashPbCb` link points to `langapi/shared/crc32.h#L8`, which is `SigForPbCb`. `lhashPbCb`, `HashPbCb` and `LHashPbCb` are in `PDB/include/misc.h` (lines 15, 68, 77).
  - 16-bit variant omitted: mspdb `HashPbCb` = `HASH(lhashPbCb(...))` truncated to `USHORT`, used by `hashSz` (GSI/PSI buckets) and the named-stream map. pdbrs applies `& 0xffff` in `info.rs` and `symbols.rs` (#9, #13).
  - V2 hash omitted: `names_stream.md` names `LHashPbCbV2`, but this document does not define it (mspdb `HasherV2::lhashPbCb`; LLVM `hashStringV2`). pdbrs does not implement it.
  - "Accidental special-case that must be considered carefully" is announced but never described. mspdb, LLVM `hashStringV1` and pdbrs agree on the tail rule (one `u16` if 2 or 3 bytes remain, then one byte if 1 byte remains).
  - The type record hash (`TPI1::hashPrec`) is not here; `tpi_stream.md` covers it partly.
- Undocumented assumptions:
  - The final reduction belongs to the caller; tables differ: `/names` v1 uses the full 32-bit value `% buckets`; GSI/PSI use `(% buckets) & 0xffff`; the named-stream map uses `(& 0xffff) % capacity`; injected sources use `key & 0xffff` placement (#9, #13).

### `microsoft-pdb-rs/docs/pdb/globals.md`
- Documents: the GSS (allowed kinds, reference symbols), the GSI/PSI name table (header 0xFFFFFFFF / 0xF12F091A, 8-byte hash records, compressed buckets with a bitmap and ×12 offsets), the GSI kind list, the PSI header (28 bytes), the PSI address table and pointer relationships.
- pdbrs: implemented — `crates/pdbrs/src/native/symbols.rs`, `symbols/table.rs` (header, records, bucket bitmap of `(count + 1).div_ceil(32)` words, offsets multiple of 12, strictly increasing, first 0, empty-table forms), `symbols/gsi.rs`, `symbols/psi.rs` (header, address map, thunk map, section starts, `check_records`, `check_address_order`). Shared records decode through `codeview/symbol.rs`. Not implemented: the old uncompressed ("large") table (signature ≠ 0xFFFFFFFF, rejected `UnsupportedVersion`); decoders for `S_GMANDATA` (0x111D), `S_ANNOTATIONREF` (0x1128), `S_DATAREF` (0x1126).
- Conflicts:
  - GSI kind list: the list omits `S_PROCREF` (0x1125) and `S_LTHREAD32` (0x1112). The samples index `S_PROCREF` through the GSI (1,661 records), and every non-`S_PUB32` record of the symbol record stream is found through the GSI (#8, #13).
  - 16-bit bucket truncation: "Compute the hash ... gives the bucket index" omits `% num_buckets` and the mspdb `HASH` truncation. pdbrs uses `(hash_string_v1 % count) & 0xffff`; this matters only for 0x3FFFF buckets (#13). `ms-pdb` 0.1.22 does not truncate.
  - Bitmap size, internal: the byte formula `(num_buckets + 32) / 8` gives 32771 for 0x3FFFF (not a multiple of 4). The build step 9 formula `(num_buckets + 32) / 32 * 4` gives 32768, which matches mspdb, `ms-pdb` `nonempty_bitmap_size_bytes` and pdbrs `(count + 1).div_ceil(32)` words.
  - Example offset, internal: "Name Table ... at offset 0x1A (28)": 28 is 0x1C, and the dump shows `ff ff ff ff` at 0x1C.
  - Build step 7, internal: "bucket `j` is non-empty if `hash_buckets[j]` is less than `hash_buckets[j – 1]`" is reversed; a bucket is non-empty when the next bucket start is greater than its own start.
  - Bucket offsets: "sorted in increasing order. Duplicates are permitted" describes the decompressed array. In the compressed form, pdbrs requires strictly increasing offsets, each less than the record count.
  - Address table order, internal: one paragraph says sorted by `(segment, offset)` of the `S_PUB32`; the next paragraph says sorted by `name`. pdbrs `check_address_order` uses `(section, offset)` keys and matches `llvm-pdbutil --public-extras` order (#8). The example heading says "GSI Address Table"; it is the PSI.
  - PSI layout omissions: the `PsiStream` struct ends with `address_table`; the thunk table is "TODO". pdbrs reads a thunk map of `num_thunks * 4` bytes and, if bytes follow, `num_sections` 8-byte `SO` entries (offset `u32`, section `u16`, padding `u16`) (mspdb `pdb.h` `SO`, LLVM `PublicsStream.cpp`).
  - Reference targets: `S_PROCREF` "refers to `S_GPROC32`". MSVC module streams hold `S_GPROC32_ID`/`S_LPROC32_ID`; the `native` example resolves `S_PROCREF`/`S_LPROCREF` to module procedures of both kinds (#10). The document does not say that the stored module index is 1-based.
  - "Lookups are case-insensitive": only the hash is case-insensitive (0x20202020). pdbrs `HashTable::lookup` returns the bucket candidates; the caller compares names.
- Undocumented assumptions:
  - Hash record offset must be in `1..=i32::MAX` and the reference count in `1..=i32::MAX` (`symbols/table.rs`, #8).
  - No bitmap bit at or past `count` can be set, including the extra free-list bit (#8).
  - An empty table can have no bucket data (mspdb `CompressBuckets`) or an empty bitmap (LLVM builder) (#8).
  - PSI section starts are read only when bytes follow the thunk map, because mspdb leaves `nSects` uninitialized otherwise (#8).
  - Trampoline join with the thunk map (#13). LLD 22.1.3 writes some public symbols in wrong buckets; pdbrs reports them as inconsistencies (#11).

### `microsoft-pdb-rs/docs/pdb/dbi_stream.md`
- Documents: the DBI header (64 bytes), signature and versions, age, global symbol stream fields, substream order and sizes, type server map and EC substreams.
- pdbrs: implemented — `crates/pdbrs/src/native/dbi.rs` decodes the modern header (signature -1, version 19990903 only), all seven substream ranges in disk order (EC before the optional debug header), module records, section contributions, section map, source info, EC names (`names.rs` table) and debug stream slots. Not implemented: versions V41/V50/V60/V110 and the headerless old header (rejected); non-empty type server map (`verify_type_server_map` → `UnsupportedVersion`); typed `flags` and `build_number` bit fields (raw).
- Conflicts:
  - Current version (document vs mspdb/pdbrs): the document says "MSVC 11.0 (current)" = `DBIImpV110` (20091201) and covers only V110+. mspdb `PDB/dbi/dbi.h` sets `DBIImpv = DBIImpvV70` (19990903); pdbrs accepts only 19990903, and all fixtures use it.
  - EC substream: the document says "not specified; decoders ignore it; encoders set size zero". LLVM `DbiStreamBuilder` writes it with `ECNamesBuilder` (a `/names`-format table); pdbrs decodes it (`Dbi::ec_names`); the samples have 4 names each (#7).
  - Trailing data: the document says decoders should ignore data after the last substream. pdbrs rejects it (`InvalidFormat`, "DBI stream has bytes after the last substream").
  - Age mismatch: the document says readers should use the PDBI age if the ages differ. pdbrs `reader.rs` `debug_info` returns `InvalidFormat` ("PDB Info and DBI ages differ").
  - Layout tree, internal: it lists 9 optional debug slots (FPO..PData); `dbi_opt_debug.md` and mspdb `DBGTYPE` define 11 (also NewFPO and SectionHdrOrig).
  - Alignment: the document requires the first four sizes to be multiples of 4. pdbrs checks only non-negative sizes, exact fill and an even optional-debug-header size.
- Undocumented assumptions:
  - `flags` bits (mspdb `NewDBIHdr`): bit 0 `fIncLink`, bit 1 `fStripped`, bit 2 `fCTypes`, bits 3-15 reserved; `build_number` bit fields (LLVM: bits 0-7 minor, 8-14 major, bit 15 new-format). pdbrs keeps both raw.
  - Stream fields equal to 0xFFFF are absent (`dbi.rs` `stream_id`); `reader.rs` checks that present stream IDs exist.

### `microsoft-pdb-rs/docs/pdb/dbi_sources.md`
- Documents: the DBI source file info substream: module count, obsolete 16-bit file count, per-module start and count arrays, `u32` name offsets and the names buffer; invariants, limits, determinism, an example.
- pdbrs: implemented — `crates/pdbrs/src/native/dbi/sources.rs` takes the layout from the summed per-module counts, ignores the 16-bit file count and the 16-bit start indexes for layout (as LLVM `DbiModuleList.cpp` `initializeFileInfo`), keeps `raw_start`/`raw_file_count` raw, and keeps trailing padding in `names()`. `dbi.rs` `source_info` checks that the module count equals the module record count.
- Conflicts:
  - Module ranges (document vs pdbrs/LLVM): the document slices `file_offsets[module_file_starts[m] .. + module_file_counts[m]]` and states `module_file_starts[m] + module_file_counts[m] <= num_file_offsets`. `module_file_starts` is `u16` and wraps when there are more than 65,535 entries, as in the document's own example (0x4D02F entries). pdbrs and LLVM compute each start as the running sum of the counts.
  - Determinism, internal: "`num_modules` is set to the low 16 bits of `num_file_offsets`" must be `num_sources`.
  - Field names, internal: `file_offsets` (struct) and `file_name_offsets` (text and tables) name the same array.
  - Text encoding: the document says names are UTF-8. pdbrs keeps them as raw bytes; producers can write paths in an ANSI code page (`dbi.rs` `Module` doc).
- Undocumented assumptions:
  - Each module's start is the sum of the counts of earlier modules, independent of the stored start (`dbi/sources.rs`).
  - The names buffer end is not marked; pdbrs exposes it with the padding and resolves only offsets that the file lists name.

### `microsoft-pdb-rs/docs/pdb/dbi_sections.md`
- Documents: the DBI section map (header + 20-byte entries) and the section contribution substream (version header; 28-byte V60 and 32-byte V2 records).
- pdbrs: implemented — `crates/pdbrs/src/native/dbi/section_map.rs` (`SectionMap`, `SegmentFlags`; descriptor count must cover all bytes) and `dbi/section_contribution.rs` (V60 0xF12EBA2D, V2 0xF13151E4 and headerless V40/SC40 20-byte records). Padding fields stay raw. `address.rs` translates contribution addresses through the image section headers.
- Conflicts:
  - Headerless VC++ 4.0 layout omitted: mspdb `dbi.cpp` reads a substream that does not start with a known version as 20-byte `SC40` records (no CRCs) (`PDB/include/dbicommon.h`); pdbrs does the same (#7). The document defines only V60 and V2.
  - Sort invariant: the document requires `contribs` sorted by `(section, offset)`. pdbrs does not check the order.
  - Name, internal: the heading and text use `DBISCImpV2`/`DBISCImpvV2`; mspdb uses `DBISCImpv2`.
- Undocumented assumptions:
  - Section map flag bits (LLVM `OMFSegDescFlags`: Read 0x1, Write 0x2, Execute 0x4, AddressIs32Bit 0x8, IsSelector 0x100, IsAbsoluteAddress 0x200, IsGroup 0x400) (`section_map.rs`).
  - The descriptor count in the header must match the bytes exactly.
  - Section number 0 and the absolute section (frame 0, absolute flag) have no RVA (`address.rs`, #7).

### `microsoft-pdb-rs/docs/pdb/dbi_opt_debug.md`
- Documents: the optional debug header substream: an array of `u16` stream indexes, slots 0-10, 0xFFFF for an absent stream, alignment advice, determinism.
- pdbrs: partial — `crates/pdbrs/src/native/dbi.rs` parses every slot (`DebugStream`, `debug_streams`, also slots after 10). Decoded slots: 0 FPO (`fpo.rs` `FpoStream`, 16-byte `FPO_DATA`), 5 section headers (`section.rs`, 40-byte `IMAGE_SECTION_HEADER`), 9 new FPO (`fpo.rs` `FrameDataStream`, 32-byte `FRAMEDATA`, frame program as `/names` offset), 10 original section headers (same 40-byte layout; no OMAP translation). Not decoded: 1 exception, 2 fixup, 3 OMAP to source, 4 OMAP from source, 6 token RID map, 7 xdata, 8 pdata. `address.rs` refuses translation (`UnsupportedVersion`) when slot 3 or 4 names a stream.
- Conflicts:
  - Alignment advice (document vs LLVM): "encoders should pad this substream to an alignment of 4 with 0xFFFF". LLVM `DbiStreamBuilder::calculateDbgStreamsSize` writes `DbgStreams.size() * 2` bytes, with 11 slots (`DbgHeaderType::Max`) = 22 bytes. Inference, not tested: MSVC also writes 22 bytes. pdbrs accepts any even size and rejects an odd size.
  - Slot 1: mspdb `pdb.h` marks `dbgtypeException` "deprecated"; the document does not.
  - Slots 6, 9, 10 have no description in the document.
  - FPO bit layout (winnt.h vs LLVM, #13): `winnt.h` `FPO_DATA` attributes are `cbProlog:8, cbRegs:3, fHasSEH:1 (bit 11), fUseBP:1 (bit 12), reserved:1, cbFrame:2`. LLVM `object::FpoData` reads `cbProlog` from 4 bits and `fHasSEH`/`fUseBP` from bits 9/10. In 7 of 8 fixture records the word is 0x0800 (SEH filter blocks); pdbrs follows `winnt.h`.
- Undocumented assumptions:
  - FPO and FRAMEDATA streams have no header; the 4-byte relocation prefix exists only in object-file `DEBUG_S_FRAMEDATA` (`fpo.rs`).
  - Xdata stores an RVA base in front of the data (mspdb `pdb.h` comment); pdbrs does not decode it.
  - Slot 10 uses the slot-5 layout (LLVM `initializeSectionHeadersData`).

### `microsoft-pdb-rs/docs/pdb/dbi_modules.md`
- Documents: the DBI module info substream: 64-byte fixed part with an embedded section contribution, flags, stream index, region sizes, source count, EC name indexes, two NUL-terminated names, 4-byte alignment; invariants and determinism.
- pdbrs: partial — `crates/pdbrs/src/native/dbi.rs` `Modules` decodes `flags` (offset 32, raw), `stream` (34), symbol/C11/C13 sizes (36/40/44), `source_file_count` (48), module name and object file name, with 4-byte record alignment. Not decoded: `old_module_index` (0), the embedded `SectionContrib` (4-31), `unused2` (52), `source_file_name_index` (56), `pdb_file_path_name_index` (60).
- Conflicts:
  - Stream size: the document allows `sym + c11 + c13 <= module stream size`. pdbrs requires the regions, the global-refs size field and the global refs to fill the stream exactly (`dbi/module.rs`).
  - Unchecked invariants: pdbrs does not check that region sizes are multiples of 4, that C11 and C13 are not both non-zero, that module streams are unique, or that `source_file_count` equals the per-module count in the sources substream.
  - Text encoding: the document says names are UTF-8. pdbrs keeps raw bytes ("paths in an unspecified code page").
- Undocumented assumptions:
  - The linker module is named `* Linker *` with an empty object file name (`tests/dbi.rs`).
  - Flag bits match mspdb (`fWritten:1, fECEnabled:1, unused:6, iTSM:8`) and LLVM `ModInfoFlags`; pdbrs keeps them raw.

### `microsoft-pdb-rs/docs/pdb/dbi_fixups.md`
- Documents: the 12-byte fixup record (`fixup_type` u16, `fixup_extra` u16, `rva` u32, `rva_target` u32) of the fixup debug stream.
- pdbrs: not implemented — `crates/pdbrs/src/native/dbi.rs` exposes the slot (`DebugStream::Fixup`, slot 2) as a stream ID only; no decoder exists (#10 lists "FPO/exception/fixup streams" as out of sample).
- Conflicts:
  - Location, internal: "See [dbi_sections.md] for locating this substream" points to the wrong file (the slot is in `dbi_opt_debug.md`, slot 2), and the link has no target. The fixups are a separate stream, not a DBI substream.
  - The record matches mspdb `pdbdump/pdbdump.cpp` `XFIXUP` (`WORD wType; WORD wExtra; DWORD rva; DWORD rvaTarget`). The document does not list `wType` values.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/terminology.md`
- Documents: glossary: age, binding key, COFF, contribution, DIA, executable, GSS, interval, module index, MSF, FPM, `ItemId`, module, `NameIndex`, page, stream, TPI, `TypeIndex`, UDT.
- pdbrs: not applicable — glossary; the terms map to `native::info::Identity`, `native::msf`, `native::codeview::index`.
- Conflicts:
  - "module index ... 0-based": `S_PROCREF`/`S_LPROCREF`/`S_TOKENREF` store a 1-based module number; pdbrs converts it (`ProcedureReference::module_index`). The glossary does not say so.
  - "age ... tools should verify that the age in the executable matches": pdbrs also requires the DBI age to equal the Info age (`reader.rs`), which `dbi_stream.md` says to tolerate.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/references.md`
- Documents: the sources of the specification (microsoft-pdb paths, LLVM docs, willglynn/pdb, raw_pdb, MS_Symbol_Type_v1.0.pdf, talks).
- pdbrs: not applicable — bibliography.
- Conflicts:
  - Path `langapi/include/cvinfo.h` is wrong; at 805655a the file is `include/cvinfo.h` (repository tree).
  - `langapi/include/cvexefmt.h` does not exist in the repository tree at 805655a.
  - `PDB/src/tools/cvdump/dumppdb.cpp` is wrong; the file is `cvdump/dumppdb.cpp`.
  - The LLVM link is unpinned (`llvm.org/docs/PDB`); the local snapshot pins LLVM at b9f2235 (`SOURCES.json`).
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/intro.md`
- Documents: purpose, scope (MSF, 2-phase commit, FPM, well-known streams, CodeView records, CFLAGS records, NatVis, SourceLink, module streams) and the invariant notation.
- pdbrs: not applicable — introduction.
- Conflicts:
  - Scope vs content, internal: the 2-phase commit is a TODO in `msf.md`; NatVis embedding is described wrongly in `pdb.md` (named streams instead of injected sources); no file in this tree describes CFLAGS records or the SourceLink stream layout.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/determinism.md`
- Documents: the idea of determinism rules that are stronger than invariants, with a hash-record ordering example.
- pdbrs: partial — the MSF writer is deterministic: the same streams and `WriteOptions` give the same bytes (`crates/pdbrs/src/native/msf/write.rs` `WritePlan`; #15 acceptance "Identical input and options produce identical bytes"). pdbrs writes no PDB stream contents: `repack` copies stream bytes unchanged, so it keeps any non-determinism inside the streams. `copy_exact` keeps every source byte.
- Conflicts:
  - Example, internal: the second ordering key is the reference count, but `globals.md` says `c_refs` is always 1, so the key orders nothing.
  - MSF rules (msf.md) vs pdbrs writer: pdbrs follows page size 4096 default, `active_fpm` = 1, deterministic inactive FPM (all 0xFF), ascending page assignment, zeroed unused bytes, `num_pages * page_size` = file length. pdbrs does not follow "no nil streams" (repack keeps nil streams to keep stream IDs). Stream 0 policy `Empty` (default) differs from MSVC output (48-byte previous directory) but matches LLD and `ms-pdb-msf` (#15).
- Undocumented assumptions:
  - Determinism of the writer was checked on 17 fixtures × 6 outputs (`pdbtool compare` identical streams; #15 evidence).

### `microsoft-pdb-rs/docs/index.md`
- Documents: the table of contents and the "non-authoritative" status of the specification.
- pdbrs: not applicable — index.
- Conflicts: None found.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/docs/data_types.md`
- Documents: notation: integer types (little-endian), GUID, embedded arrays, `strz`, `Number`, `TypeIndex`, `ItemId` (NIL = 0), `NameIndex`.
- pdbrs: partial — numeric leaves: `crates/pdbrs/src/native/codeview/numeric.rs` decodes immediate values and LF_CHAR..LF_UQUADWORD, LF_OCTWORD/LF_UOCTWORD; real, complex, decimal and date leaves are raw bytes only inside managed constants (`codeview/symbol/managed.rs`); `LF_UTF8STRING` is not decoded. `TypeIndex`/`IdIndex` in `codeview/index.rs` (first record index 0x1000; ID 0 = `None`).
- Conflicts:
  - `strz` is "UTF-8". pdbrs keeps all names as raw bytes; module and file names can use an ANSI code page (`dbi.rs`).
  - Link `pdb/ipi_stream.md#itemid`: `ipi_stream.md` has no `ItemId` heading.
- Undocumented assumptions: None.

### `microsoft-pdb-rs/LICENSE`
- Documents: the MIT license of `microsoft/pdb-rs` (Copyright Microsoft Corporation).
- pdbrs: not applicable — license text.
- Conflicts: None found.
- Undocumented assumptions: None.

### `microsoft-pdb/README.md`
- Documents: the archived status of `microsoft/microsoft-pdb`, its purpose, `pdb.h`/`mscorpdb.dll`, and a table of PDB stream numbers.
- pdbrs: not applicable — upstream README.
- Conflicts:
  - Stream table (README vs pdbrs, LLVM and the microsoft-pdb-rs docs): stream 4 is "NameMap (hashed string table)". In modern PDBs stream 4 is the IPI stream (pdbrs `reader.rs` `ids`); `/names` is a named stream.
  - Module streams "4-(n+4)", global/public hash and symbol records "n+4..n+6", type hash "n+7": these numbers are not fixed. The DBI header and module records name the module, GSI, PSI and symbol record streams; the TPI header names the hash stream (pdbrs `dbi.rs`, `types.rs`).
  - "Public symbol hash ... searching in public symbols by addresses": the PSI also has a name hash table (`globals.md`, pdbrs `symbols/psi.rs`).
- Undocumented assumptions: None.

### `microsoft-pdb/docs/ExternalResources.md`
- Documents: external links (Breakpad, dump_syms, pdbparser, the Clang CodeView RFC, MS_Symbol_Type_v1.0.pdf, PE/COFF, calling conventions).
- pdbrs: not applicable — link list.
- Conflicts:
  - Inference, not tested: The `code.google.com/p/...` links (pdbparser, corkami) point to Google Code, which closed in 2016; the links are probably dead. I did not fetch them.
- Undocumented assumptions: None.

### `microsoft-pdb/LICENSE`
- Documents: the MIT license of `microsoft/microsoft-pdb` (Copyright 2015 Microsoft Corporation).
- pdbrs: not applicable — license text.
- Conflicts: None found.
- Undocumented assumptions: None.

### `SOURCES.json`
- Documents: the snapshot manifest: UTC time, scope, three pinned sources (llvm/llvm-project b9f2235, 12 files; microsoft/pdb-rs 2081a0dd, 80 files; microsoft/microsoft-pdb 805655a, 3 files) and 95 file entries with upstream path, git blob SHA-1, SHA-256 and size.
- pdbrs: not applicable — manifest.
- Conflicts:
  - None found. I recomputed SHA-256 and size for all 95 entries: 0 mismatches, 0 missing files. `docs/reference/README.md` and `docs/reference/.gitattributes` are local files and are not listed (correct for upstream-only scope).
  - Note: the Rust crates used for cross-checks (`ms-pdb` 0.1.22, `ms-pdb-msf` 0.1.7, `ms-pdb-msfz` 0.1.10) are registry releases, not files of this snapshot; #20 ties `ms-pdb-msfz` 0.1.10 to `microsoft/pdb-rs` 2081a0dd. Inference, not tested: The other crate versions are not proven to match 2081a0dd.
- Undocumented assumptions: None.
