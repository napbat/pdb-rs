# PDB reader and writer design

Status: the full reader/writer design remains proposed. The library reads, writes and edits modern native PDBs (MSF 7.0); the [native coverage](native-coverage.md) document gives the state of each record kind, stream version and container feature.
Kinds without a local producer or a layout definition remain open there as `external-input`. The library writes MSF 7.0 containers (exact copies, stream-preserving repacks and new layouts).
It reads and writes MSFZ containers (PDZ files) and converts between MSF 7.0 and MSFZ (see [Implemented MSFZ writing](#implemented-msfz-writing)).
The library reads the metadata of standalone Portable PDBs and the raw rows of their eight debug tables (see [Implemented Portable PDB reading](#implemented-portable-pdb-reading)).
The library writes Portable PDBs: exact copies, new layouts from raw rows and heaps, semantic builds and edits (see [Implemented Portable PDB writing](#implemented-portable-pdb-writing)).
Portable PDB blob decoding is partial. The library builds new native PDBs from semantic records (see [Implemented native building](#implemented-native-building)).
It edits native PDBs and keeps the streams that an edit does not change (see [Implemented native editing](#implemented-native-editing)).

The root epic [#18](https://github.com/napbat/pdbrs/issues/18) tracks the work. Its children are one epic for each format family:
[#1](https://github.com/napbat/pdbrs/issues/1) (native MSF 7.0), [#20](https://github.com/napbat/pdbrs/issues/20) (native MSFZ), [#17](https://github.com/napbat/pdbrs/issues/17) (Portable PDB) and [#16](https://github.com/napbat/pdbrs/issues/16) (legacy MSF 2.0, deferred).
The [reference snapshots](../reference/README.md) contain the pinned LLVM and Microsoft documentation.

## Implemented primitives

The `pdbrs` package is a library with no default executable. It uses `nostdio` for sequential I/O.
Its default `std` feature enables `nostdio/std` and native file adapters.
With `--no-default-features`, the library supports `no_std + alloc`.
Slice reads, signature probing and fixed-buffer cursor operations do not allocate.

- `io::ReadAt` provides checked positional reads and little-endian integer decoding for slices and references.
- `io` re-exports `nostdio` reading, writing, seeking, cursor and endian helpers.
- `io::SeekReader` adapts `Read + Seek` inputs to `ReadAt` in both feature modes.
  It captures the input length and restores the original cursor during successful construction.
  Subsequent reads move the cursor. The adapter is not `Sync`.
- `io::FileReader` provides bounded file access on Unix and Windows under `std`.
  It captures the file length and owns the handle. Windows positional reads can change the underlying cursor.
- `pdbrs::probe` identifies modern MSF, MSFZ, legacy Small MSF, CLI metadata and embedded PDB envelope signatures.
  It examines a bounded prefix, not container or record validity.
  `ContainerKind::CliMetadata` deliberately does not assert that the input is a Portable PDB.
- `Error` retains an error kind, physical or stream-relative location, operation context and a typed `nostdio::IoError`.
  `Error::with_portable` adds a `PortableLocation`: metadata stream, table, row, column and heap offset.
- `native::Identity` distinguishes modern GUID-and-age from legacy signature-and-age.
- `portable::Identity` encodes the 20-byte ID and checks GUID, stamp and age fields from a portable PE reference.
- `portable::custom` decodes and encodes the custom debug information blobs of all kinds in the Portable PDB specification and Roslyn (#24).
  It keeps unknown kinds as raw bytes. The native `MD2` Edit and Continue records (kinds 6, 7 and 9) use the same codecs.
  `portable::Pdb::custom_debug_information` reads a row with its kind GUID and its value blob.
- `portable::debug` has the typed rows of the debug tables and the codecs of their blobs: document names, sequence points, local constant signatures and imports (#23).
  Each codec has an encoder that takes resolved heap offsets and row IDs, for the writer (#26). `Pdb::validate_debug` checks the ordering and reference rules.

Run signature inspection on Unix or Windows with:

```sh
cargo run -p pdbrs --example inspect -- path/to/input.pdb
```

Exercise sequential output and positional input in either feature mode:

```sh
cargo run -p pdbrs --example streams
cargo run -p pdbrs --example streams --no-default-features
```

The examples report their validation scope. Semantic editors, PDB builders and embedded compression are not exposed as placeholder APIs.

## Implemented native reading

`native::Pdb::open` opens a file under `std`. `Pdb::parse` borrows a byte slice.
`Pdb::read` and `Pdb::with_options` accept any stable `ReadAt` source, including `SeekReader`.
These source-based APIs also work with `no_std + alloc`.

Opening reads the container directory (MSF 7.0 or MSFZ, through `native::container::Container`) and the PDB Info stream. Other typed accessors load one requested stream at a time.
Raw `container::Stream` reads join fragmented MSF blocks or MSFZ chunks without loading the complete stream.
The default limits are 64 MiB for the directory and 64 MiB for each typed stream load.
Callers can change these limits through `native::ReadOptions` (`with_limits` and `with_max_stream_bytes`). Allocation failures return `LimitExceeded`.

| Area | Implemented | Not yet implemented |
| --- | --- | --- |
| MSF 7.0 | Directory, nil and empty streams, fragmented reads, reserved and duplicate page checks, active free-page-map consistency and the stream 0 snapshot. Writing: exact copies, stream-preserving repacks and new layouts for page sizes 512 to 32768 (see [MSF writing](#implemented-msf-writing)) | Edits in place and incremental commits (not planned); writing 65536-byte pages (consumers reject them) |
| PDB Info and named streams | Identity, `niMac`, typed feature codes, named-stream slot placement and the Microsoft or LLVM hash table growth rules, `/LinkInfo` versions 1 and 2, `/TMCache` and its type map streams, `/UDTSRCLINEUNDONE`, `/src/headerblock`, verified uncompressed injected sources, .NET document records (compression byte 101) with CRC, size and SHA-256 checks through the shared `portable::embedded` codec, Natvis files, `sourcelink` and `srcsrv` as checked bytes, an Info age larger than the DBI age (`pdbstr -w`), encoders for each stream and `InfoBuilder` with an explicit `IdentityPolicy` (#32) | Run-length, Huffman or LZ injected sources (no producer and no algorithm description); Info header versions other than VC70 |
| TPI and IPI | Modern headers, indexed record iteration, typed decoders and encoders for every modern leaf of `cvinfo.h` and the wide UDT leaves of LLVM, every field-list member and every ID record (#29), typed based-pointer data, `LF_INDEX` continuation splitting, offset-table lookup, continuation chains, forward-reference resolution, reference validation with depth and work limits, hash values, index offsets and hash adjusters, stream writers that rebuild the TPI and IPI streams and their hash streams in the Microsoft or LLVM layout, the MASM end padding of UDT records (`trailing`), and the module type streams of `NOTM` PDBs (`TypeStream::from_records`, #30) | Auxiliary hash streams; `LF_RANGELIST`, `LF_TAGGED_UNION`, `LF_TAGGED_UNION_CASE`, `LF_TUCASE` and `LF_IFC_RECORD` have no layout in the pinned definitions and stay raw |
| DBI | Modern header with typed flags, module records with every field, debug-stream references, section contributions (VC++ 4.0, V60 and V2), section map, source file info, type server maps (also non-empty), EC names, an encoder for each substream and `DbiBuilder` (#32) | DBI versions other than V70 |
| Optional debug streams | A decoder and an encoder for all 11 slots: FPO, exception data, fixups, OMAP in both directions, section headers, token/RID maps, `.xdata`, `.pdata`, new FPO (frame data) and original section headers (#32) | — |
| String tables | `/names` and EC layout, offset lookup, version 1 and version 2 hash lookup and bucket checks, `StringTable::encode` and `StringTableBuilder` with the Microsoft `NMT` bucket rules (#32) | — |
| Sections and addresses | Section header streams, section-offset to RVA translation in both directions, end-of-section offsets, and OMAP translation in both directions with explicit results for unmapped addresses (#32); `SymbolLocator` gives the procedure, source lines and inline chain of an address, also in separated code chunks (`S_SEPCODE`) as DIA does (#30) | OMAP evidence from a real producer (`external-input`) |
| Symbol indexes | Global and public hash tables, bucket checks, name lookup candidates with the 16-bit bucket rule of minimal (FASTLINK) indexes, address maps, thunk maps and section starts. Writing: GSI and PSI builders with the producer bucket order and the MSVC or LLVM conventions for empty tables and address order (`native::Producer`); the GSI and PSI of every native fixture rebuild to the same bytes (#30) | Pre-compressed-bucket hash layouts |
| Symbols | Typed decoders and encoders for 126 symbol kinds: every modern kind of `cvinfo.h` with a known layout and the later kinds of LLVM and the DIA SDK (#30), including the managed kinds of .NET Windows PDBs (#14) with the Roslyn `MD2` custom debug information and `asyncMethodInfo` attributes. The decoder keeps the count of zero padding bytes (fxc writes 4 or more). `ModuleSymbolWriter` and `SymbolRecordWriter` write the module symbol regions and the symbol record stream. `S_FRAMEREG`, `S_FASTLINK` and `S_REGREL32_ENCTMP` layouts come from fixtures (#13) | `S_ALTOBJNAME`, `S_LOCAL_2005`, `S_DEFRANGE_2005` and `S_DEFRANGE2_2005` have no layout in any source and stay raw |
| Symbol validation | `codeview::symbol::validate` checks module and shared symbol records (#30): procedure end kinds, `next` links, scope code ranges, TPI and IPI references with IPI record kinds (`S_CALLERS` too; only the range for `S_SOURCELINK`), module references (`S_PROCREF`, `S_LPROCREF`, `S_TOKENREF`, `S_DATAREF`, `S_ANNOTATIONREF`), def-range locals, ranges and gaps (also program, HLSL and `_ON_ENTRY` def-ranges, and def-ranges in the `S_SEPCODE` code of their procedure), def-ranges against the code of their inline sites, subfield offsets against type sizes (`types::type_size`), local flags, section lengths and register numbers for the module CPU. In a `NOTM` PDB, the references of a module resolve against the module type streams that its `S_MOD_TYPEREF` names. It collects each broken rule in a report | — |
| Module lines | Region bounds, C13 subsection framing, typed decoders and encoders for all named subsection kinds except `0xfe` (#31: symbols, line blocks with columns, string table, file checksums, frame data, inlinee lines, cross-scope imports and exports, IL lines, metadata token maps, merged assembly input, COFF symbol RVAs, XFG hashes), explicit `DEBUG_S_IGNORE` handling, checksum reference validation, cross-scope import and export checks (`validate_cross_scope`), global references checked against the symbol record stream, and a module stream writer that returns the DBI module record sizes | `DEBUG_S_INLINEE_MDTOKEN_MAP` (`0xfe`) data stays raw: no layout source or producer |
| Numeric leaves | Every leaf tag of `cvinfo.h` (`numeric::Numeric`): integers through 128 bits with their encoding width, real, complex, `DECIMAL` and `DATE` values with their exact bytes and typed accessors, `LF_VARSTRING` and `LF_UTF8STRING`, and an encoder that gives the input bytes again | — |

Unknown record kinds retain their raw payloads. Names remain bytes; display code chooses how to decode them.
Header validation does not validate every record payload. Iterate records to check their framing.
Unsupported container and stream versions return `UnsupportedVersion`; no heuristic fallback parses them as modern data.
Full native support remains required. This coverage table does not reduce the scope below.
The [native coverage](native-coverage.md) document gives the state of each record kind, stream version and container feature.
It also reconciles each imported reference document with the implementation.

Read a complete native PDB with:

```sh
cargo run -p pdbrs --example native -- path/to/input.pdb
```

The example decodes every stream role and checks the references between streams.
It joins line and inlinee data to `/names`, procedure references to module symbols and index records to name lookup.
It translates symbol, contribution and line addresses to RVAs, and counts the absolute addresses that have none.
It fails if any record kind, subsection kind or stream has no typed decoder.
It reports a decoded stream that no other stream names, with the reason, and does not fail for it.
It runs the checks of `codeview::symbol::validate` on each module stream and on the shared symbols, and reports each broken rule as an inconsistency.

Print the type information of a native PDB in a C-like form with:

```sh
cargo run -p pdbrs --example types -- path/to/input.pdb
cargo run -p pdbrs --example types -- --type <name> path/to/input.pdb
cargo run -p pdbrs --example types -- --functions path/to/input.pdb
```

The `types` example uses only the public API: `TypeTable`, `ForwardRefs`, `TypeLeaf`, `TypeTable::members` and `Numeric`.
Without an option, it prints each UDT definition and each enum of the TPI stream in index order. `--type` selects the definitions with one name.
A UDT gives its size and one line for each member: base classes, virtual base classes, the virtual function table pointer, data members with their offsets, static members, nested types and methods with their signatures.
A bit field also gives its width and its first bit.
An enum gives its underlying type and each enumerator. The example reads each value as the underlying integer type, up to 128 bits.
For example, MSVC writes `-1` for the largest value of an `unsigned __int64` enum, and the example prints `18446744073709551615`.
`--functions` prints the signature of each procedure symbol (`S_GPROC32`, `S_LPROC32` and their `_ID` kinds through `LF_FUNC_ID` and `LF_MFUNC_ID`), then the signature of each `LF_FUNC_ID` and `LF_MFUNC_ID` record.

Type names use C declarators, for example `void (__cdecl *callback)(void *)` or `char name[16]`.
A member function gives its qualifiers after the arguments and its `this` type in brackets.
The type index `0x0103` prints as `std::nullptr_t`, as in LLVM. A procedure without a type, for example an MSVC unwind funclet, gives only its name.
Sizes follow forward references to their definitions. The example stops at 64 nested types and prints `<...>` there.

Evidence (#1): the example printed every UDT definition, enum and procedure of four fixtures.
The table gives the items that agree with DIA2Dump (`-t` and `-s`) and with `llvm-pdbutil pretty` (`-classes -enums` and `-module-syms`).

| Fixture | UDTs: DIA2Dump, llvm | Enums: DIA2Dump, llvm | Procedures: DIA2Dump, llvm |
| --- | --- | --- | --- |
| `msvc-x64-debug` | 275, 309 of 331 | 49, 52 of 54 | 222, 362 of 523 |
| `msvc-x86-debug` | 246, 281 of 302 | 48, 51 of 53 | 243, 156 of 482 |
| `clang-lld-x64-debug` | 233, 241 of 263 | 42, 44 of 45 | 247, 322 of 487 |
| `rust-link-x64-debug` | 395, 274 of 422 | 59, 61 of 61 | 459, 405 of 509 |

Each other item differs in printing style only. No difference shows another offset, size, value or type record.
Examples: DIA2Dump prints `&&` as `*` and a function pointer as `function *`, and takes many signatures from the decorated name.
`llvm-pdbutil` moves a pointer `const` to the front, omits `__thiscall`, and prints the parameter symbols instead of the `LF_ARGLIST`.
The comment on #1 lists each difference class with its count.

`Pdb::stream_bytes` and `Pdb::named_stream_bytes` load bounded stream bytes for decoders that borrow their input.
`Pdb::global_symbols` and `Pdb::public_symbols` load the symbol indexes that the DBI header names.
`Pdb::fpo` and `Pdb::frame_data` load the FPO and new-FPO streams that the DBI optional debug header names.
GitHub issues #1 to #14, #29 to #34, #37 and #38 record the implementation and the evidence for each area.

## Implemented Portable PDB reading

`portable::Pdb::open` opens a standalone Portable PDB under `std`. `Pdb::parse` borrows a byte slice.
`Pdb::read` and `Pdb::with_options` accept any stable `ReadAt` source in both feature modes ([#22](https://github.com/napbat/pdbrs/issues/22)).
`portable::ReadOptions` limits the stream count (64), the bytes of one heap value (64 MiB) and the bytes of `Pdb::stream_bytes` (64 MiB).

Opening validates the metadata root, the stream headers and ranges, the `#Pdb` stream, the `#~` header, the table schema and the row extents.
It does not read rows or heap values. `Pdb::row` and `Pdb::rows` read one row at a time from its computed offset.
`Pdb::string`, `blob`, `user_string` and `guid` read one heap value and check it before allocation.

| Area | Implemented | Not yet implemented |
| --- | --- | --- |
| Metadata root | `BSJB`, version 1.1, version string, stream headers, 4-byte alignment of stream offsets and sizes, ranges, overlaps and duplicate names; unknown streams stay in the header list | — |
| `#Pdb` | PDB ID, entry point (nil or an external `MethodDef` row), referenced type-system tables and their row counts, exact stream size | — |
| `#~` | Version 2.0, `HeapSizes`, `Valid` and `Sorted` masks, row counts up to `0xFFFFFF`, heap, table and coded index widths from local and external row counts, row extents. A table outside the debug schema with rows gives `InvalidFormat`; `#-` and delta `HeapSizes` flags give `UnsupportedVersion` | Uncompressed `#-` tables |
| Heaps | `#Strings` termination, `#Blob` and `#US` length prefixes, `#GUID` indexes, the empty first entry and 16-byte GUID boundaries | — |
| Debug tables | Raw rows of all eight tables with typed row IDs, heap handles and `HasCustomDebugInformation` tokens; heap and row references checked. `portable::debug` (#23): typed rows of seven tables (`CustomDebugInformation` in `portable::custom`, #24); document names, languages and hash algorithms; sequence points (hidden points, document records, signed deltas, IL offset and line and column bounds); local constant signatures (all type codes, enums, general values, custom modifiers); imports of the nine kinds; an encoder for each blob in the Roslyn form; `Pdb::validate_debug` | Instruction boundaries and the IL size of the first scope of a method: they need the method bodies of the assembly |
| Dispatch | `pdbrs::Pdb` and `pdbrs::Format`: native MSF 7.0 and Portable PDB by contents; ordinary CLI metadata without `#Pdb` is rejected with `MissingStream`; `MPDB` payloads need `portable::embedded::decode_embedded` | Legacy MSF 2.0 (`UnsupportedVersion`) |
| Embedded data (#25) | `portable::embedded::decode_embedded` and `encode_embedded` for `MPDB` payloads (signature, reserved sizes, limit before allocation, exact output length); `Pdb::document_source` decodes the embedded source of a document and checks it against the document hash; `Document::verify_source` | Hash algorithms other than SHA-256 give `SourceCheck::NotVerified` |

Read a complete Portable PDB or an `MPDB` payload with:

```sh
cargo run -p pdbrs --example portable -- path/to/input.pdb
```

The example reads every row and every heap value that a row refers to. It fails at the first value that it cannot read.
It decodes every blob of the debug tables and every custom debug information value, encodes each again and compares the bytes.
It decodes the embedded source of each document and fails if a source does not match its SHA-256 hash.
Then it runs `Pdb::validate_debug` and prints the report: each rule with the number of checked items, or the reason why the rule was not checked.
The .NET verifier in `fixtures/dotnet/verifier/` prints a canonical dump with `System.Reflection.Metadata`.
`cargo run -p pdbrs-fixtures -- verify-portable` compares it with the pdbrs dump; `fixtures/README.md` describes the format.
The dump has the raw rows, the decoded debug tables (#23), the decoded custom debug information (#24) and the embedded source checks (#25).
For the embedded fixture, pdbrs reads the `.mpdb` payload and the verifier reads the assembly with `PEReader.ReadEmbeddedPortablePdbDebugDirectoryData`.
Evidence ([#25](https://github.com/napbat/pdbrs/issues/25)): the 15 embedded sources of the five Portable PDB fixtures match their SHA-256 hashes in both dumps.
Evidence ([#22](https://github.com/napbat/pdbrs/issues/22), [#23](https://github.com/napbat/pdbrs/issues/23)): the four standalone Portable PDB fixtures (C#, C# with embedded sources, VB and F#) and the embedded C# PDB give dumps without differences.
The encoders give the same bytes for every debug table blob of the four standalone fixtures (62, 62, 25 and 45 blobs).
In 26 F# sequence point blobs, the `LocalSignature` header is a full `StandAloneSig` token, not a row. `SequencePoints::local_signature_form` reports the form, and `encode_sequence_points` takes a `LocalSignatureForm` to keep it.
`cargo run -p pdbrs-fixtures -- detect` opens the 17 native fixture PDBs with the native backend and the Portable PDBs with the portable backend.
It rejects the CLI metadata of each fixture assembly as ordinary assembly metadata.

`Pdb::validate_debug` checks these rules and stops at the first violation:

- Unique document names; hash sizes of the defined hash algorithms.
- The `MethodDebugInformation` row count against the `MethodDef` count from `ValidateOptions` or from the `#Pdb` stream.
- Sequence point blobs: documents and the `LocalSignature` row exist; a method without sequence points has a nil document.
- `LocalScope` order (method, start, descending length), non-nil methods, IL ranges, nesting, and the first scope of each method at offset 0 and around the other scopes.
- Contiguous variable and constant runs that give each row to one scope; unique variable indexes and names, and unique constant names, in each scope.
- Constant signatures and their external type rows; import blobs and their assemblies, types, aliases and namespaces; import-parent chains without cycles.
- `StateMachineMethod` sorted by `MoveNextMethod` with unique methods; `CustomDebugInformation` sorted by `Parent`.

A nil `VariableList` or `ConstantList` is an empty run if its table has no rows (the F# compiler writes nil `ConstantList` values).

## Implemented Portable PDB writing

The `portable::write` and `portable::build` modules write Portable PDBs through these operations ([#26](https://github.com/napbat/pdbrs/issues/26)):

| Operation | API | Guarantee |
| --- | --- | --- |
| Exact copy | `Pdb::copy_exact`, `Pdb::copy_exact_file` | Every source byte, through one buffer of at most 64 KiB |
| New layout | `PdbParts::plan`, then `WritePlan::write_to`, `to_vec`, `to_embedded` or `write_file` | A checked Portable PDB with the given raw rows and heaps |
| Semantic build | `PdbBuilder::plan` | A checked Portable PDB from records with typed handles, in new heaps |
| Semantic edit | `Pdb::edit`, then `PdbEditor::plan` | The source records with the changes; each source heap entry keeps its offset |

`PdbParts` holds the version string, the entry point, the external row counts, the four heaps and the raw rows of the eight debug tables.
The heap builders remove duplicates. `#Blob`, `#US` and `#GUID` offsets follow the insertion order.
`StringHeapBuilder::finish` sorts the strings by their reversed bytes and puts each suffix string inside a longer string.
`Pdb::to_parts` reads all rows and re-adds the heap entries. It keeps the row IDs and the row order.
If a `#Blob` offset changes, it decodes the document name and imports blobs with the codecs of [#23](https://github.com/napbat/pdbrs/issues/23), gives them the new offsets and adds the encoded blobs.

`PdbBuilder` holds owned records: documents, the sequence points of each method, local scopes with variables and constants, import scopes, state machine methods and custom debug information.
Documents and import scopes keep the order of addition, so their handles are `DocumentId` and `ImportScopeId`.
Local scopes, variables, constants and custom debug information have opaque handles; the plan gives them their rows.
The plan sorts the local scopes, gives each scope one variable run and one constant run, and sorts state machine methods and custom debug information.
It splits document names as Roslyn does and encodes the blobs with the codecs of #23. Index widths follow the final heap sizes and row counts.

`Pdb::edit` makes a `PdbEditor`: a builder with the source records whose heaps start with the source heap bytes.
Without a change, the output has the source bytes. Documents and import scopes keep their row IDs.
Custom debug information of an unknown kind keeps its bytes. If a change gives a source local scope, local variable or local constant a new row ID, such a value can contain the old row ID.
`PdbEditor::plan` then fails with `InvalidReference` before output. The parent of a known kind follows its record to the new row.

The plan checks every heap and row reference, the `#Pdb` fields and the required row order before it writes a byte.
It uses the layout of `System.Reflection.Metadata`: the stream order, the `Valid` and `Sorted` masks, the index width rules and the trailing zero byte of `#~`.
`IdPolicy` selects an explicit PDB ID or the content-derived ID of `BlobContentId.FromHash`: SHA-256 over the output with a zero ID, with the GUID version and variant bits and bit 31 of the stamp set.
`WritePlan::identity`, `codeview` (GUID, age 1, stamp) and `checksum` (SHA-256, SHA-384 or SHA-512 with a zero ID, the PDB checksum entry of the addendum) give the values for the debug directory of the assembly.
`WritePlan::write_file` and `copy_exact_file` use the temporary-sibling contract of the MSF file helpers.

Evidence: `cargo run -p pdbrs-fixtures -- verify-rebuild` rebuilds the four standalone Portable PDB fixtures and the decoded `MPDB` payload of the embedded fixture.
Each rebuild has the original bytes and the original content-derived ID, and its verifier dump has no differences.
`cargo run -p pdbrs-fixtures -- verify-edit` edits and builds the same five PDBs and compares the verifier dumps with the expected change.
It runs each standalone fixture program with a built PDB and with a PDB whose sequence points move 1000 lines (see `fixtures/README.md`).

## Implemented MSF writing

The `native::msf` module writes Big MSF 7.0 containers through three operations ([#15](https://github.com/napbat/pdbrs/issues/15)):

| Operation | API | Guarantee |
| --- | --- | --- |
| Exact copy | `io::copy_exact`, `io::copy_exact_file` | Every source byte. The copy does not validate the source |
| Stream-preserving repack | `Msf::repack` | Stream IDs, nil and empty streams, sizes and all stream bytes. `OldDirectoryPolicy` selects stream 0 |
| New layout | `MsfBuilder::plan` | A consistent container with the given streams, without a PDB semantic claim |

`Msf::repack` and `MsfBuilder::plan` return a `WritePlan`. The plan computes and checks the full layout before output.
A planning error writes nothing. `WritePlan::write_to` writes to a `nostdio` writer in page order, and `WritePlan::to_vec` writes to memory.
Both work with `no_std + alloc`. The writer reads stream payloads one page at a time.
`io::copy_exact` copies through one buffer of at most 64 KiB.

Under `std`, `WritePlan::write_file` and `io::copy_exact_file` write a temporary file in the destination directory and rename it.
On error, they delete the temporary file and do not change the destination. Writing over the source file is not supported.
After an error, `write_to` and `io::copy_exact` can leave a partial output in the writer.

The layout is deterministic:

1. Page 0 is the header. Pages 1 and 2 of each interval of `page_size` pages are FPM1 and FPM2.
   If the last page is the first page of an interval, the plan adds the FPM pages of that interval.
2. The stream pages (in stream ID order), the directory pages and the page map pages have ascending page numbers.
3. The active free page map is FPM1. It marks the stream 0 pages and the bits after the last page as free.
   It marks all other pages as allocated. FPM2 has `0xFF` in all bytes.
4. The unused bytes of the header page and of the last page of each stream, the directory and the page map are zero.

The default page size is 4096. The plan accepts every power of two from 512 to 32768.
It rejects 65536-byte pages with `UnsupportedVersion` before planning, because independent consumers reject them
([#15](https://github.com/napbat/pdbrs/issues/15)): DIA2Dump fails with `loadDataFromPdb failed - HRESULT = 806D000C`,
`llvm-pdbutil` reports "Unsupported block size." and `link.exe` 14.51 fails on `/PDBPAGESIZE:65536` with `LNK1399`.
The reader accepts 65536-byte pages. `link.exe` 14.51 writes 4096 to 32768 with `/PDBPAGESIZE`.
LLVM also rejects a directory that needs more than one page map page.

The plan returns `LimitExceeded` for a stream of `0xFFFFFFFF` bytes or more, a directory larger than `MsfLimits::max_directory_bytes`
or than `u32`, a page map array that does not fit in the header page, or a page count that does not fit in `u32`.

The default stream 0 policy is `OldDirectoryPolicy::Empty`, as LLD and `ms-pdb-msf` write it.
`OldDirectoryPolicy::Preserve` copies stream 0. If stream 0 has bytes, it needs the page size of the source.

Copy or repack a native PDB, and compare the result with its source:

```sh
cargo run -p pdbrs --example msf -- copy input.pdb copy.pdb
cargo run -p pdbrs --example msf -- repack --page-size 8192 --old-directory empty input.pdb repacked.pdb
cargo run -p pdbrs --example msf -- compare input.pdb repacked.pdb
```

`compare` compares the presence, size and bytes of each stream and checks the free page map of the written file.

Evidence ([#15](https://github.com/napbat/pdbrs/issues/15)): exact copies of the 17 native producer fixtures are byte-identical.
Their repacks with 512, 4096, 8192 and 16384-byte pages and both stream 0 policies pass `pdbtool check` and `pdbtool compare`.
DIA2Dump gives the same output as for the source. `llvm-pdbutil dump --all` gives the same result as for the source.
Its output differs only in block numbers, block counts and the stream 0 size.

## Implemented native building

`native::write::PdbBuilder` builds a new native PDB from semantic records ([#33](https://github.com/napbat/pdbrs/issues/33)).
It reuses the encoders of each record family. The module documentation of `native::write` describes the handles and the plan.

- The caller gives the identity (signature, GUID and age). `PdbBuilder::apply_identity` applies an `IdentityPolicy`.
  `PdbWritePlan::pe_match` returns the GUID and the DBI age for the `RSDS` entry of the image.
- `TypeHandle`, `IdHandle`, `ModuleHandle`, `ModuleSymbolHandle`, `SymbolHandle` and `StreamHandle` exist before on-disk values.
  A deferred record gets a `Resolver` when the plan encodes it. `reserve_type` and `reserve_id` give a handle before its record.
- `PdbBuilder::plan` assigns type indices, ID indices, stream IDs, module symbol offsets and symbol record offsets.
  It checks the TPI, IPI, module symbol, shared symbol and C13 references, and it checks each field width before output.
- The plan builds the TPI and IPI hash streams, the scope links, the global and public indexes, the DBI source info,
  `/names`, the named-stream map and the MSF layout. The same builder and options give the same bytes.

`Pdb::contents` and `PdbContents::to_builder` give a semantic rebuild: every record is decoded and encoded again.
The rebuild keeps the record order, so type indices, ID indices and `/names` offsets stay the same. Stream IDs change.
It drops stream 0 and streams that no other stream names (`PdbContents::unclaimed_streams`).
`PdbContents::build_options` keeps the page size of the source; change `BuildOptions::msf` to select another page size.
The rebuild puts each public symbol in the bucket of its name; LLD 22.1.3 puts some 8-byte names in another bucket.

```sh
cargo run -p pdbrs --example native_rebuild -- input.pdb rebuilt.pdb
cargo run -p pdbrs --example native_synthetic -- synthetic.pdb
```

DIA lists a procedure only if its record names a TPI type (`S_GPROC32`), as `link.exe` writes it.
DIA2Dump does not list `S_GPROC32_ID` procedures of the synthetic PDB.

## Implemented native editing

`native::Pdb::edit` makes a `native::PdbEditor` ([#34](https://github.com/napbat/pdbrs/issues/34)). The editor borrows the source PDB and owns only the changes.
`PdbEditor::plan` checks all changes and encodes the changed streams. It returns a `PdbEditPlan`, which writes like `PdbWritePlan`.
The module documentation of `native::edit` gives all rules.

- The output keeps the stream ID of each source stream. The plan reads each unchanged stream from the source when it writes, so `pdbtool compare` shows it as identical.
  A removed stream becomes a nil stream. A new stream gets the next stream ID. `PdbEditPlan::changed_streams` lists the changed, removed and new streams.
- Named streams: `set_named_stream` adds or replaces a stream, and `remove_named_stream` removes it. The Info named-stream map changes. The plan decodes new `srcsrv`, `sourcelink` and `/LinkInfo` bytes with their decoders.
- Symbols: `add_public`, `add_global`, `rename_public` and `rename_global`. A renamed record stays at its position, and an added record goes to the end of the symbol record stream.
  The plan builds the global and public indexes again with the source buckets, reference counts, thunk map and section starts. It writes moved offsets into the global references of the module streams.
- Types and IDs: `add_type` and `add_id` append records. The hash stream keeps the source hash values, index offsets and adjusters, and gets the values of the new records.
- Identity: the caller gives an `IdentityPolicy` to `Pdb::edit`. The Info and DBI ages stay consistent. `PdbEditPlan::pe_match` returns the values for the image.
- Strip: `strip_private` writes the streams that `link /PDBSTRIPPED` (MSVC 14.51) writes: no types or IDs, only `S_PUB32` and `S_LPROCREF` records, local procedures without types in the module streams, no source files, lines or EC names, zero section contribution CRCs and the DBI stripped flag.

The plan rejects these edits before it writes a byte. A planning error writes nothing.

- An edit that moves a symbol record, and a strip, if opaque data can refer to the changed offsets or indices: a stream that no other stream names and that does not decode as a type map,
  a named stream of an unknown kind, and for moved records also a symbol record or a C13 subsection of an unknown kind.
  A type map without an owner holds type indices only, so it does not block an edit. The strip removes it. Stream 61 of the `msvc-x64-incremental` fixture is such a type map: `/TMCache` does not name it.
- A change of `/names`, `/TMCache`, `/UDTSRCLINEUNDONE` or a `/src/` stream, a rename of a reference record such as `S_PROCREF`, a strip together with another symbol or type edit, and a reference that does not resolve.

Caution: `IdentityPolicy::Preserve` after a semantic change keeps the match with the old image, so a debugger can misidentify the changed symbols.
Use `IncrementAge` or `Replace` for a semantic change, and write the returned `PeMatch` to the image.

```sh
cargo run -p pdbrs-fixtures -- verify-native-edit
```

## Required formats and scope

Support both native Windows PDB and Portable .NET PDB files.
Both formats require inspection, preservation, semantic editing and new-file construction.
The requirement to support both formats is settled. The API details below remain proposed.
Keep reading useful without requiring support for writing every record kind.
Make every write guarantee explicit.

For the initial native implementation, target MSF 7.0 containers and modern PDB streams. Include these capabilities:

- Raw access to every stream, including unnamed and unknown streams.
- PDB identity, named streams and feature codes.
- TPI and IPI type records and their hash streams.
- DBI module data, section data, source files and referenced debug streams.
- Module symbols, C13 line data, checksums and inline information.
- Shared symbol records and global and public symbol indexes.
- Section-relative addresses, RVA conversion and OMAP translation when present.
- New-file construction and edits to existing files.

For Portable PDBs, target the Portable PDB 1.0 metadata schema. Include these capabilities:

- Metadata roots, stream headers, debug tables and string, blob and GUID heaps.
- The `#Pdb` stream, content identity and external type-system table counts.
- Documents, checksums, sequence points, local scopes, variables, constants and imports.
- State-machine method mappings and custom debug information.
- Source Link and embedded source, including compressed source content.
- Standalone files and embedded Portable PDB payload decoding and encoding.
- New-file construction and edits with coherent row IDs, heap offsets and external references.

These are two format backends, not two variants of the MSF parser.
Legacy native PDB support is also required, but its implementation is deferred.
Plan legacy reading, preservation, editing and writing without making them an initial release gate.
Use explicit legacy codecs; do not interpret older layouts as MSF 7.0.
Detect recognized but unimplemented formats and return an unsupported-version error, not a corruption error.

Native PDBs can also use the MSFZ container (PDZ files). MSFZ replaces only the MSF layer; the PDB streams do not change.
Support MSFZ reading, conversion between MSF 7.0 and MSFZ, and MSFZ writing.
Keep the MSFZ codec separate from the MSF 7.0 codec. Run the same PDB stream decoders on both containers.

Do not make this crate a linker, symbol server client or PE parser.
Expose external references, including native type servers, FASTLINK dependencies and portable assembly metadata.
Do not fetch dependencies or claim that their missing records are available.
Distinguish standalone inspection from semantic validation that needs external metadata or method bodies.
Do not promise conversion between native and Portable PDBs; that requires a separate mapping design.

## What to take from portex

The local reference is `coconutbird/portex`. Its README distinguishes `PeHeaders`,
`PeImage`, `PeFile` and `PeBuilder`. Its I/O module combines sequential I/O with positional reads.

Adopt these principles:

- Keep inspection separate from editing and construction.
- Distinguish byte preservation from normalized serialization.
- Use positional reads for large inputs and concurrent independent readers.
- Keep the format code usable with `no_std + alloc`.
- Make malformed input and invalid builds return contextual errors.

Do not copy the PE object model directly. Native PDB uses fragmented streams; Portable PDB uses CLI metadata.
Do not add a `PdbImage` type or require a complete owned model for ordinary reads.
Do not copy portex's infallible `build` wrapper or whole-image cloning during layout.
Use one fallible build path and an immutable output plan.

## Package and module boundaries

Keep one public library package at `crates/pdbrs`, with its root in `src/lib.rs`.
Use the `inspect` example for the current signature-inspection workflow.
Do not add empty modules or another crate just to reserve names.

Proposed modules, relative to `crates/pdbrs/src/`:

| Module | Responsibility |
| --- | --- |
| `io.rs` | Positional sources, slice adapters and feature-gated file adapters |
| `compression.rs` | Crate-private raw DEFLATE and zstd decoding and encoding with size limits; the only user of `miniz_oxide` and `ruzstd` |
| `crypto.rs` | Crate-private SHA-2 digests; the only user of `sha2` |
| `native.rs` and `native/` | Native reader, editor, builder, identity and capability reporting |
| `native/msf.rs` and `native/msf/` | Superblock, directory, block maps, stream access and container writing |
| `native/msfz.rs` and `native/msfz/` | MSFZ header, stream directory, Chunk Table, fragment reads, chunk cache and container writing |
| `native/container.rs` | One stream container type (`Container`) over MSF 7.0 and MSFZ for the native `Pdb` facade |
| `native/codeview.rs` and `native/codeview/` | Record framing, type records, symbol records and numeric leaves |
| `native/types.rs` and `native/types/` | TPI/IPI access, type lookup and hash generation |
| `native/dbi.rs` and `native/dbi/` | Modules, contributions, section maps, source file info and debug stream references |
| `native/names.rs` | String tables of `/names` and the DBI EC substream |
| `native/section.rs` | Image section headers from DBI debug streams |
| `native/symbols.rs` and `native/symbols/` | Shared symbols, global/public indexes and lookup |
| `native/lines.rs` and `native/lines/` | C13 subsections, checksums, line tables and inline annotations |
| `native/address.rs` | Section offsets, RVAs and OMAP mappings |
| `native/write.rs` and `native/write/` | Native dependency checks, indexes and block allocation |
| `portable.rs` and `portable/` | Portable reader, editor, builder, identity and capability reporting |
| `portable/metadata.rs` and `portable/metadata/` | Metadata root, tables, heaps, compressed integers and coded indexes |
| `portable/debug.rs` and `portable/debug/` | Documents, sequence points, scopes, locals, imports and state machines |
| `portable/custom.rs` and `portable/custom/` | Known custom debug codecs and raw unknown payloads |
| `portable/embedded.rs` | Embedded PDB envelopes and embedded-source blobs, also used by native .NET document records |
| `portable/write.rs` and `portable/write/` | Row assignment, table sorting, heap layout and reference rewriting |
| `format.rs` | Checked format detection and dispatch to the selected backend |
| `error.rs` | Error kinds and structured locations |
| `validation.rs` | Explicit whole-file validation and capability reports |

Keep MSF independent of PDB semantics. Keep record codecs independent of filesystem I/O.
Use the same checked codecs for readers and writers; do not create parallel record definitions.
Extract another crate only when an actual consumer needs a separate dependency boundary.

Share checked byte access, I/O and error conventions. Keep layout and reference logic in each backend.
Do not create a generic table-or-stream framework or a common mutable symbol model.
Evaluate an existing CLI metadata codec before implementing the portable metadata layer.
Reuse it only if it supports the required read/write, preservation, safety and `no_std + alloc` contracts.
Portable support must not depend on installing a .NET runtime.

Keep legacy codecs inside the native backend, next to the structures they decode.
Separate container dispatch from PDB stream-version and CodeView record dispatch.
Do not add a third backend that duplicates native stream and record infrastructure.

## Public API shape

Names below are proposed contracts, not compilable examples.

| Type or operation | Contract |
| --- | --- |
| `Format` and `Pdb<S>` | Identify the format and hold either `native::Pdb<S>` or `portable::Pdb<S>` |
| `Pdb::parse(&[u8])` | Detect and validate a standalone file through a borrowed slice source |
| `Pdb::open(path)` | Open a file source under the `std` feature and dispatch by its contents |
| `Error`, `ErrorKind`, `Result`, `PortableLocation`, `probe`, `PROBE_LEN`, `ContainerKind` | Live at the crate root; the `error` and `format` modules are private |
| `io::copy_exact`, `io::copy_exact_file` | Copy every byte of a source; an exact copy does not depend on the format |
| `native::msf::Msf<S>` | Own a stable positional source and validated stream-directory metadata |
| `native::msf::Msf::read`, `native::msfz::Msfz::read`, `native::container::Container::read` | Read a container from a positional source; `with_limits` takes `MsfLimits`, `MsfzLimits` or `ContainerLimits` |
| `native::msf::Msf::stream(StreamId)` | Distinguish an invalid index, an absent stream and a present stream |
| `native::Pdb<S>` | Expose lazy typed access to native streams |
| `native::Pdb::info`, `types`, `ids`, `debug_info` | Decode only the requested native stream or view |
| `portable::Pdb<S>` | Expose lazy access to metadata tables, heaps and debug blobs |
| `portable::Pdb::documents`, `method_debug_info`, `local_scopes` | Return portable records using typed row IDs and method context |
| Backend `Pdb::validate` | Report checked invariants, unsupported content and required external information |
| Backend `Pdb::edit` | Borrow the source and create a format-specific editor without copying all payloads |
| `native::PdbEditor`, `portable::PdbEditor` | Retain unchanged source content and own changes for that format |
| `native::PdbBuilder` | Construct native records with explicit identity and target information |
| `portable::PdbBuilder` | Construct debug tables with explicit external table counts and identity policy |
| `native::MsfBuilder` | Construct arbitrary streams without claiming PDB semantic validity |
| Backend `WritePlan` | Freeze format-specific lengths, references, identity and layout before output |
| Backend `WritePlan::write_to` | Write to a fresh destination without mutating the source |
| `portable::embedded::decode_embedded`, `encode_embedded` | Decode or emit an `MPDB` payload; PE directory handling stays with the caller |

Use the root facade for detection and dispatch, not to hide format-specific operations.
Match the selected backend before editing or constructing records.
Do not expose native `types()` or RVA lookup on a Portable PDB.

Detect native MSF by its signature. A `BSJB` signature identifies CLI metadata, not necessarily a Portable PDB.
Require the portable `#Pdb` and `#~` streams and validate their headers before reporting portable success.
Return a format or structural error for ordinary assembly metadata, truncated input or unsupported versions.
Use the explicit embedded adapter for `MPDB` payloads; do not treat a PE file as a standalone PDB.
Never use the filename extension or a PE debug-directory hint as proof of format.
Expose detected native container and stream versions separately from supported operation capabilities.
Recognizing a legacy signature must not imply that semantic reading or writing is available.

Use distinct native newtypes for `StreamId`, `TypeIndex`, `IdIndex`, `ModuleIndex`,
`SymbolOffset`, `SectionOffset` and `Rva`. Use a shared `FileOffset` only for physical source positions.
Use distinct portable row IDs, `MethodDefId`, `MetadataToken`, `BlobOffset`, `GuidIndex` and `IlOffset`.
An IL offset needs its method context; it is not an RVA.
Record offsets also need their owning stream, table or module context.
Do not accept a bare integer where two address or index spaces could be confused.

Keep names and paths as bytes at the format boundary. Offer fallible text decoding separately.
Do not replace invalid text during parsing or writing.
Decode integers explicitly as little-endian values. Do not cast input bytes to Rust structs.

## Reading and ownership

Use a small positional source contract with a known length and exact reads into caller buffers.
A read must complete the buffer or return an error. The source must remain unchanged while borrowed.
Do not infer that positional access makes every source thread-safe.

Use `nostdio` for sequential output and cursor adapters, following portex.
Keep `std` as a default feature for filesystem access and standard I/O conversion.
The core requires `alloc`, but not filesystem APIs, a runtime or memory mapping.
No dependency on portex is required; callers can pass PE identity and section data explicitly.

At native open, validate the superblock and directory before exposing streams.
At portable open, validate the metadata root, stream ranges, table schemas and computed row extents.
Validate record framing, heap values and references as their typed APIs are used.
A successful open does not mean that every record in the file is valid.
Expose full validation separately, including a report of records not semantically understood.

A native stream can cross arbitrary, noncontiguous blocks. A field can cross a block boundary too.
Use a stream cursor that translates logical offsets into physical blocks.
Decode small fields through fixed-size scratch storage.
Borrow contiguous bytes when the source permits it. Do not promise universal zero-copy access.

Native record readers load the bytes of a stream and bound them before they iterate.
Use ordinary `Iterator` types over these loaded, bounded stream bytes. Do not use a lending iterator.
The `iter()` function of a view returns `<View>Iter`.
Give a name to each iterator type that a function returns, so that a caller can name it.
Offer an explicit owned conversion for callers that retain records.
Offer lazy native type-offset indexing; do not build a complete type graph during open.
Do not require native type references to form a strictly ordered DAG when reading.
For portable tables, calculate row widths once and read individual rows directly.
Keep sequence-point and custom-blob decoding lazy.

Keep metadata proportional to directory or table metadata and requested indexes.
Bound directory bytes, stream counts, record buffers, nesting, decompressed bytes and optional index allocations.
Expose documented limits and return `LimitExceeded` before crossing them.
Do not allocate a buffer from an unchecked length in the file.

## Preserved producer encodings

Some producers write CodeView records and stream bytes that the format does not require or that a canonical encoder does not write.
`native::codeview` and the native stream modules keep these details, so that a decoded record or stream encodes to the same bytes.
Microsoft and LLVM writers also differ in the layout of some hash streams and hash tables. `native::Producer` selects the layout of one producer.
The names of these items use one vocabulary (rule 13 of #40):

- `trailing_zeros`: the count of zero bytes after the last field of a record.
- `trailing`: the bytes after the last field that are not zero bytes, for example the MASM end bytes of a UDT record.
- `padding*`, `reserved*` and `unused*`: a field with the name that the source structure (`cvinfo.h` or the producer) gives it.
- `*Form` or `*Kind` enums, or `bool`s: the encoding form when more than one form is valid.

| Item | Producer | What the decoder keeps | What the encoder writes |
| --- | --- | --- | --- |
| `Symbol::trailing_zeros()` | `fxc` (4 or more zero bytes after an `S_LOCAL` name) | The count of zero bytes after the fields. A nonzero byte is `InvalidFormat`. | `Symbol::encode`, `ModuleSymbolWriter::push_symbol` and `SymbolRecordWriter::push_symbol` write the same count. `SymbolData::encode` writes the shortest padding. |
| `Compile3::trailing_zeros` | `ml64` (one more NUL after the version string) | The count of zero bytes after the version string. | The same count of zero bytes. |
| `type_record::Pointer::trailing_zeros` | `ml64` and `ml` 14.51 (6 zero bytes in each `LF_POINTER` record) | The count of zero bytes after the last field, before the `LF_PAD` run. | The same count of zero bytes, then the `LF_PAD` run. |
| `trailing` on `Class`, `WideClass`, `Union`, `WideUnion` and `Enum`; `TypeLeaf::trailing()` | MASM (`ml64`) in UDT records of the static CRT, for example `f1 6d 65` | 1 to 3 end bytes that start with `0xf1` to `0xf3`. Empty for the standard `LF_PAD` run. | These bytes instead of the `LF_PAD` run. |
| `IntegerKind`, `IntegerLeaf` | All producers (the numeric leaf width form) | The leaf tag and the value, also a width that is not the smallest (for example `LF_ULONG` 5). | The same tag and width. `Numeric::from_integer` selects the smallest form. |
| `RegisterList::parse(bytes, wide)`, `RegisterList::is_wide()` | All producers (the kind selects 8-bit or 16-bit register numbers, for example `S_MANYREG` or `S_MANYREG2`) | The width of the register numbers. | The same width. The encoder rejects a list with a width that does not agree with the kind. |
| `StringList::is_terminated()` | All producers (the `S_COMPILE2` and `S_ENVBLOCK` string block is absent or ends with an empty string) | `true` if the record holds the empty string that ends the sequence. | The stored form. |
| `UdtModuleSourceLine::padded` | `link.exe` (writes `LF_UDT_MOD_SRC_LINE` without `LF_PAD`, 18 bytes) | `true` if `LF_PAD` padding follows the module field. | The `LF_PAD` run only if `padded` is `true`. |
| `numeric::Decimal::reserved` | Not known (`LF_DECIMAL`; no fixture has this leaf) | The reserved `u16` field of the OLE `DECIMAL` value. The `Numeric` value keeps all 16 value bytes. | The 16 value bytes of the `Numeric` value. |
| `Hlsl::property_word`, `Hlsl::unused_bits()` | `fxc` (bits 4 to 15 are not zero, for example `0x07a0`) | The raw property word. `cvinfo.h` requires zero in bits 4 to 15. | The same word. |
| `EnvironmentBlock::flags` | Not known | The raw flags byte. `cvinfo.h` requires zero in bits 1 to 7. | The same byte. |
| `Discarded::raw_reason` | Not known | The reason in bits 0 to 7 and the unused bits 8 to 31, raw. | The same `u32`. |
| `ModuleTypeReference::flags` | Not known | The flags, with the reserved bits 6 to 14. | The same flags. |
| `CustomDebugInfoRecord::alignment`, `CustomDebugInfoRecord::reserved` | Roslyn (`MD2` in `S_OEM`; Roslyn writes 0 in `reserved`) | The reserved header byte and the count of zero padding bytes at the end of the record, raw. | No typed encoder. The `S_OEM` encoder writes the raw `Oem::data` bytes. |
| `CustomDebugInfo::reserved` | Roslyn (writes zeros) | The two reserved bytes of the global header, raw. | No typed encoder. The `S_OEM` encoder writes the raw `Oem::data` bytes. |
| `dbi::Module::unused1`, `OwnedModule::unused1` | Microsoft writers (a stale `pmod` pointer at offset 0) | The raw `u32`. | The same `u32`. |
| `dbi::Module::file_name_offset`, `OwnedModule::file_name_offset` | Microsoft writers (a stale `mpifileichFile` pointer at offset 52) | The raw `u32`. Readers ignore it. | The same `u32`. |
| `dbi::Module::padding`, `OwnedModule::padding` | Not known | The raw `u16` at offset 50. | The same `u16`. |
| `dbi::Module::name_padding`, `OwnedModule::name_padding` | Not known | The bytes after the object file name up to the next 4-byte boundary. | The same bytes, as many as the record needs. |
| `SectionContribution::padding1`, `SectionContribution::padding2` | Writers that leave the bytes uninitialized | The raw `u16` after the section number and after the module index. | The same `u16` values. |
| `dbi::TypeServer::reserved`, `dbi::TypeServer::padding` | Microsoft writers (a stale pointer in `reserved`) | The raw `u32` and the bytes after the path up to the next 4-byte boundary. | The same `u32` and bytes. |
| `DbiHeader::reserved` | Not known | The raw `u32` at offset 60. | The same `u32`. |
| `ModuleSources::raw_start`, `OwnedModuleSources::raw_start` | Microsoft writers (the low 16 bits of the first file entry index); LLVM writers (the module index) | The raw `u16` start index. The decoder does not use it for the layout. | The same `u16`. `OwnedSourceInfo::from_file_names` writes the Microsoft value. |
| `SourceInfo::raw_file_count()`, `OwnedSourceInfo::raw_file_count` | All producers (the low 16 bits of the file entry count) | The raw `u16` count of the header. | The same `u16`. |
| `SourceInfo::names()`, `OwnedSourceInfo::names` | All producers (alignment padding after the last name) | The names buffer with the bytes after the last name. | The same bytes. |
| `BlobHeader::reserved` | `link.exe` (writes 0) | The two reserved `u32` values of the `DbgRvaVaBlob` header. | The same values. `BlobHeader::default()` sets 0. |
| `RvaVaBlob::extra_header()`, `RvaVaBlob::new(.., extra_header, ..)` | Not known (version 1 writers write none) | The header bytes after offset 32. | The same bytes. |
| `Fixup::extra` | `link.exe /DEBUGTYPE:FIXUP` | The raw `wExtra` value. The PDB sources do not give its meaning. | The same `u16`. |
| `fpo::FpoData::reserved` | Not known | The `reserved` bit 13 of the attribute word. | The same bit. |
| `symbols::PublicHeader::padding` | Microsoft and LLVM writers (2 bytes after the thunk table section of `PSGSIHDR`) | The 2 raw bytes. | `PublicSymbolIndex::encode_parts` writes zeros, so a PSI that the rebuild or the editor writes again has zero padding. |
| `symbols::SectionStart::padding` | Microsoft and LLVM writers (the `pad` field of `SO`) | The 2 raw bytes after the section number. | The same bytes. |
| `symbols::HashEntry::reference_count` | MSVC (counts the modules that share a record, for example an `S_UDT` record); LLVM writes 1 | The reference count of each hash record (`HashRecord::reference_count`). | The same count. `HashEntry::new` sets 1. |
| `symbols::HashEntry::bucket` | LLD 22.1.3 (hashes a public name of exactly 8 bytes with the bytes after the short name in the COFF symbol table) | The bucket of each hash record (`Bucket::index`). | The record in the stored bucket. `None` selects the bucket of the name. `PdbEditor` keeps the source bucket; the rebuild uses the bucket of the name. |
| `GlobalSymbolIndex::encode_parts(.., producer, ..)`, `PublicSymbolIndex::encode_parts(.., producer, ..)` (`native::Producer`) | Microsoft (`mspdbcore.dll`) and LLVM (`GSIStreamBuilder.cpp`) | The decoder accepts both forms. | `Producer::Microsoft`: a table without records has no bucket data, and the address map compares sections and offsets as signed values. `Producer::Llvm`: a table without records has a bitmap with no bits set, and the address map compares unsigned values. |
| `TypeHeader::producer()`, `TypeStreamOptions::with_producer` (`native::Producer`) | Microsoft (`tpi.cpp`) and LLVM (`TpiStreamBuilder`) | The header buffers give the form: LLVM puts the empty hash adjuster buffer at the end of the hash values, or writes zero-length buffers without a hash stream. | `encode_type_stream`: the buffer order, the index offset pair rule and the buffers without a hash stream of the producer. The LLVM form has no hash adjusters. |
| `BuildOptions::{hash_layout, index_producer, info_producer}` (`native::Producer`), `PdbContents::build_options()` | Microsoft and LLVM | The rebuild selects one producer for the three fields from `TypeHeader::producer()` of the TPI stream. `PdbEditor` uses the same rule. | The TPI and IPI hash streams, the GSI and PSI, and the named-stream map of the Info stream in the form of the producer. |
| `OwnedHashTable::{new, with_capacity, from_table, producer}`, `InfoBuilder::{new, from_info_with}`, `SourceHeaderBlockBuilder::{new, from_block}` (`native::Producer`) | Microsoft (`map.h`) and LLVM (`HashTable.h`) | The bucket of each entry and the words of both bit vectors of a decoded table. | `Producer::Microsoft`: a default table has 1 bucket, the present vector has at least 1 word, and a vector keeps its words until the next growth. `Producer::Llvm`: a default table has 8 buckets, and a vector has words up to its highest set bit. A new `/src/headerblock` table has 1 bucket (Microsoft) or 2 buckets (LLVM). |
| `InjectedSource::reserved` | Not known (`pvReserved1` of `SrcHeaderOut`) | The 8 reserved bytes of a `/src/headerblock` entry, raw. | The same bytes. `SourceHeaderBlockBuilder::add_file` writes zeros. |
| `LinkInfo::trailing` | Not known (bytes after the `/LinkInfo` record) | The stream bytes after the record, raw. | The same bytes. `LinkInfo::new` sets no bytes. |
| `SourceServer::trailing()` | Source indexing tools (bytes after the `SRCSRV: end` line) | The stream bytes after the end marker line, raw. | The same bytes. |

## Modern MSF invariants

The reader must check these conditions before using the affected data:

- The magic and block size select a supported layout.
- Size calculations, block counts and offset arithmetic do not overflow.
- Every referenced block lies within the declared container and available source bytes.
- Directory lengths and block lists are internally consistent.
- An absent stream remains distinct from a present stream of length zero.
- Reserved blocks are not used as ordinary stream payload.
- Directory and payload blocks do not overlap in unsupported ways.
- The active free-page map and its repeated layout are interpreted correctly.

Validate free-page allocation consistency in full validation. Emit consistent maps on every rebuild.
Preserve stream numbers and absent slots during repacking.
Do not compact stream IDs implicitly.

The reader supports MSF 7.0 page sizes that are powers of two from 512 to 65536.
The writer supports 512 to 32768, with 4096 as the default. It rejects 65536 with `UnsupportedVersion`,
because DIA (`HRESULT 806D000C`), LLVM ("Unsupported block size") and `link.exe` (`LNK1399`) reject that size.
The FPM interval has `page_size` pages for each size (LLVM `getFpmIntervalLength`, `ms-pdb-msf` `interval_to_page`).
Test each supported size independently and enforce that variant's limits.
Reject layouts beyond implemented limits instead of truncating block-directory metadata.
Cross-check historical documentation against Microsoft and LLVM implementations before encoding limits.

## MSFZ invariants

The reader must check these conditions before using the affected data:

- The signature and version 0 select the only supported layout.
- Directory, Chunk Table, fragment and chunk ranges lie inside the file, and their arithmetic does not overflow.
- No chunk or uncompressed fragment overlaps another one, the header, the stream directory or the Chunk Table.
- A nil stream remains distinct from a present stream of length zero.
- A compressed fragment stays inside the virtual byte array of the decompressed chunks.
- Each chunk decompresses to exactly its declared size, within the configured limit.

Treat other versions and compression codes as unsupported, not malformed.
Writers zero-fill gaps and give the same chunk layout for the same input and options.
Where the MSFZ specification and Microsoft's reference implementation differ, follow the implementation and record the decision in #20.

### Implemented MSFZ reading

Status: the reader is implemented ([#37](https://github.com/napbat/pdbrs/issues/37)). The writer and the conversions are in [Implemented MSFZ writing](#implemented-msfz-writing) ([#38](https://github.com/napbat/pdbrs/issues/38)).

- `pdbrs::probe` gives `ContainerKind::NativeMsfz`. `pdbrs::Pdb` sends MSFZ input to the native backend (`Format::Native`).
- `native::msfz::Msfz` checks every invariant above when it opens a file. It decodes stored (code 0), zstd (code 1) and raw DEFLATE (code 2) chunks and stream directories through `compression.rs`. Other codes and versions give `UnsupportedVersion`.
- `native::container::Container` holds an `Msf` or an `Msfz`. `native::Pdb::container` replaces `Pdb::msf`. All typed accessors, `Pdb::contents` and `Pdb::edit` read through it. The builder and the editor write MSF 7.0 output with the default page size (4096) for MSFZ input.
- The free page map and the stream 0 snapshot are MSF 7.0 structures. `Container::msf` gives them; the `native` example reports them as not applicable for MSFZ.
- `MsfzLimits` bounds the stream directory (64 MiB), the Chunk Table (64 MiB), one chunk (256 MiB) and the chunk cache (256 MiB). The cache removes the least recently used chunk first; the same reads give the same evictions. The `ContainerLimits` of `ReadOptions::limits` holds the limits.
- Stream sizes are `u64` values: a stream can be larger than 4 GiB. Nil and empty streams stay distinct.
- The decisions for the differences between the specification and `ms-pdb-msfz` 0.1.10 are in [#37](https://github.com/napbat/pdbrs/issues/37#issuecomment-6415233037). Two decisions follow the specification, because it is a superset: a compressed fragment can continue into the next chunks, and non-zero reserved location bits are an error.
- `pdbtool pdz-encode` writes stream 0 (the previous MSF stream directory) as an empty stream. Stream comparisons start at stream 1, as `pdbtool compare` does.
- Evidence: every stream of the 63 PDZ fixtures (`fixtures/README.md`) is byte-identical to the MSF 7.0 original, through pdbrs and through `pdbtool compare`. The `native` example gives the same output for each pair, apart from the lines that start with `container:`. At #37, `msvc-x64-lazy` stopped at the same symbol reader error on both files; since the `NOTM` reader of #30, the `native` example reads its PDZ file with `undecoded items: 0`. The `msfz` and `msfz_chunks` fuzz targets are in `docs/fuzzing.md`.

### Implemented MSFZ writing

The `native::msfz` module writes MSFZ containers, and `native::container::Container` converts between MSF 7.0 and MSFZ ([#38](https://github.com/napbat/pdbrs/issues/38)).
[#38](https://github.com/napbat/pdbrs/issues/38#issuecomment-6416340034) records the decisions.

| Operation | API | Guarantee |
| --- | --- | --- |
| New MSFZ container | `MsfzBuilder::plan` | A consistent container with the given streams, without a PDB semantic claim. A stream can be larger than 4 GiB |
| MSF 7.0 or MSFZ to MSFZ | `Container::to_msfz` | Stream IDs, nil and empty streams and all stream bytes. `OldDirectoryPolicy` selects stream 0 |
| MSF 7.0 or MSFZ to MSF 7.0 | `Container::to_msf` | The same, through the MSF writer (`Msf::repack` for MSF 7.0 input). A stream of `0xFFFFFFFF` bytes or more gives `LimitExceeded` |

`msfz::WriteOptions` has the options of `pdbtool pdz-encode` and the chunk codec:

- The maximum chunk size: 1 MiB by default, from 4 KiB to 1 GiB. `pdz-encode` changes other values to the nearest limit; the plan rejects them with `InvalidFormat`.
- One stream for each chunk: each stream with data starts a new chunk.
- Stream directory compression (`with_directory_codec`): off by default, because some readers do not read a compressed directory. `pdz-encode --compress-stream-dir` uses zstd.
- Padding to 16 KiB (`with_pad_16k`): a workaround for the first MSVC PDZ decoder.
- The chunk codec: zstd (the default) or raw DEFLATE, through `compression.rs`.
- `MsfzLimits`: the plan rejects a chunk, a directory or a Chunk Table that a reader with these limits does not open.

The plan computes and checks the full layout before output. The layout is deterministic:

1. The 80-byte header is at offset 0. The chunks follow from offset 80 in Chunk Table order, without gaps.
2. The stream directory and then the Chunk Table start at the next multiple of 16. With padding, zero bytes follow to 16 384 bytes. All gaps are zero.
3. The bytes of the streams with data, in stream ID order, form one sequence. Each chunk holds the next `max_chunk_size` bytes. Only the last chunk (of each stream, with one stream for each chunk) can be smaller.
4. A fragment never crosses a chunk boundary: the `ms-pdb-msfz` reader (`pdbtool compare`) reads a fragment from one chunk only. The writer writes no uncompressed fragments.

The header holds the offsets of the directory and the Chunk Table, and these depend on the compressed chunk sizes. Thus planning reads all stream bytes and compresses each chunk once to get its size.
`WritePlan::write_to` compresses each chunk again and fails with `InvalidFormat` if a size changed (the source changed).
The memory is one chunk buffer, its compressed bytes, the directory and the Chunk Table. `write_to`, `to_vec` and `write_file` have the contract of the MSF writer: a planning error writes nothing, and `write_file` leaves no partial file.

Write a PDZ file, convert it back and compare the streams:

```sh
cargo run -p pdbrs --example msf -- pdz-encode --max-chunk-size 64K --compress-stream-dir input.pdb output.pdz.pdb
cargo run -p pdbrs --example msf -- repack output.pdz.pdb back.pdb
cargo run -p pdbrs --example msf -- compare input.pdb back.pdb
```

`pdz-encode` takes `--max-chunk-size <bytes>[K|M]`, `--one-stream-per-chunk`, `--compress-stream-dir`, `--pad16k`, `--deflate` and `--old-directory empty|preserve`.
`repack` reads MSF 7.0 or MSFZ input.

Evidence ([#38](https://github.com/napbat/pdbrs/issues/38)): `cargo run --release -p pdbrs-fixtures -- verify-msfz` (see `fixtures/README.md`).

- Encode: 48 native PDBs with six option sets (defaults, a compressed directory, one stream for each chunk, 4 KiB chunks, padding and DEFLATE chunks) give 288 PDZ files. All are deterministic, `pdbtool compare` reports "Files are IDENTICAL" for each, and the pdbrs reader and the `native` example give the same streams and output as for the source.
- DIA2Dump `-all` gives the same output as for the source for 235 files. For `msvc-x64-incremental`, DIA gives another output for each MSFZ file, also for the `pdz-encode` file; the pdbrs files give the same output as the `pdz-encode` file (5 files).
  DIA rejects all 48 files with raw DEFLATE chunks (`HRESULT = 806D000C`): DIA reads only zstd chunks. `pdbtool` and pdbrs read them.
- Decode: the 63 PDZ fixtures convert to MSF 7.0 (with the page size of the original) deterministically. `pdbtool check` reports 0 errors, and `llvm-pdbutil dump --all` and DIA2Dump give the same output as for the original. The streams are equal to the PDZ file and, from stream 1, to the original.
- `pdz-encode` output: no pdbrs file is byte-identical, and none needs to be. `pdz-encode` stores the Info stream and the DBI header uncompressed, ends chunks at record and substream boundaries, and uses the zstd C library. The streams are the same.
- The `msfz_convert` fuzz target (`docs/fuzzing.md`) converts MSF 7.0 and MSFZ input to MSFZ and back.

## Portable metadata invariants

Use the ECMA-335 metadata layout and Portable PDB debug-table schema.
The portable `#~` stream contains debug tables, not copies of the assembly's type-system tables.
Read external type-system row counts from `#Pdb`; they affect reference widths even without the assembly.
Do not confuse heap offsets, GUID indexes, table row IDs and metadata tokens.

Check the metadata root, version, stream headers, alignment and every range before reading payloads.
Reject duplicate required streams, overlapping unsupported ranges and arithmetic overflow.
Calculate table and coded-index widths from all applicable row counts.
Calculate heap-index widths from their declared flags and validate that referenced entries fit.
Validate compressed signed and unsigned integers, blob lengths, string termination and GUID boundaries.
An unknown table schema cannot be skipped safely when its row width is unknown.

Implement all eight debug tables in both the reader and writer:

| Table | Required semantics |
| --- | --- |
| `Document` | Component-encoded names, language IDs, checksum algorithm IDs and checksum bytes |
| `MethodDebugInformation` | MethodDef row correspondence, local signature references and sequence-point blobs |
| `LocalScope` | Method and import references, sorted scopes, variable/constant runs and IL ranges |
| `LocalVariable` | Attributes, local slot indexes and names |
| `LocalConstant` | Names and constant signatures, including external type references |
| `ImportScope` | Parent chains and kind-specific import blobs |
| `StateMachineMethod` | MoveNext/kickoff method mappings, uniqueness and required ordering |
| `CustomDebugInformation` | Coded parents, kind GUIDs, payloads and required parent ordering |

`MethodDebugInformation` is absent/empty or has the same row count as the external `MethodDef` table.
Preserve that correspondence; do not remove rows just because a method has no sequence points.
Keep hidden sequence points, document switches and signed deltas distinct.
Check monotonic IL offsets and the specified line and column bounds.
Check scope ordering, nesting, ownership ranges and import-parent cycles.

Standalone access must work without loading an assembly.
Accept external metadata and method-body information explicitly for name resolution and deeper validation.
Report missing context for instruction-boundary checks, local signatures and external type resolution.
Do not invent method names or claim full validation from row counts alone.

Expose known custom debug information through typed codecs and all kinds through raw payload access.
Include Source Link, embedded source, async stepping information, hoisted local scopes and state-machine relationships.
Include documented compiler records for dynamic locals, namespaces, Edit and Continue maps and compilation metadata.
Preserve unknown GUID kinds without interpreting or silently discarding them.
Compiler-specific data must not be described as supporting a live Edit and Continue session.
Source Link exposes source mappings; it does not fetch URLs.

Decode and encode the `MPDB` envelope around a DEFLATE-compressed Portable PDB.
Validate its declared size and supported version before allocating or decompressing.
Enforce a decompression limit and require the output length to match the declared size.
The decompressed result must pass the ordinary portable reader's checks.
Embedded source is a different custom blob: its format field distinguishes raw bytes from compressed content.
Apply independent size checks to that blob. Do not confuse it with the embedded PDB envelope.

## Compression and digests

Use portable `no_std + alloc` crates: `miniz_oxide` for raw DEFLATE, `ruzstd` for zstd and `sha2` for SHA-2 digests.
Another crate can replace one of them later without a change to the public API.
Only `compression.rs` names `miniz_oxide` and `ruzstd`, and only `crypto.rs` names `sha2`. Keep both modules crate-private.
Do not expose their types, traits or errors in the public API. Map their failures to `Error` with a context.
The caller adds the location of the compressed data.
Check the declared size against a limit before allocation. Decode into a buffer of the declared size.
Fail on short output, extra output, unused input or a corrupt stream.
Report checksums in other algorithms, such as SHA-1 or MD5, as not verified. Do not report them as errors.
Add another digest only when a fixture needs it, and only inside `crypto.rs`.
GitHub issue [#19](https://github.com/napbat/pdbrs/issues/19) tracks this work.

## Three distinct write guarantees

| Operation | Guarantee | Not guaranteed |
| --- | --- | --- |
| Exact copy | Preserve every source byte, including free blocks and padding | No semantic edits |
| Stream-preserving repack (native only) | Preserve stream IDs, absent slots, lengths and all logical stream bytes | Original block positions, free-space bytes or physical padding |
| Semantic build or edit | Emit coherent supported records and all affected indexes | Byte-identical encoding or arbitrary unknown-reference relocation |

Status: exact copies and stream-preserving repacks are implemented for MSF 7.0 (see [MSF writing](#implemented-msf-writing)).
Exact copies, new layouts, semantic builds and semantic edits are implemented for Portable PDBs (see [Portable PDB writing](#implemented-portable-pdb-writing)).
Native semantic builds and edits are implemented (see [Implemented native building](#implemented-native-building) and [Implemented native editing](#implemented-native-editing)).

Keep these operations explicit. Do not silently fall back from preservation to normalization.
An exact copy is a copy operation, not proof that the source is a valid PDB.
A repack proves container consistency, not semantic validity of opaque streams.

Exact copying and semantic editing apply to both formats.
Portable output that changes metadata layout uses its semantic write plan, not the native repack operation.
Preserve row IDs and heap references unless an explicit, supported edit requires changing them.

Unknown streams and unknown length-delimited records remain available as raw bytes.
A typed accessor may return an unknown record kind without losing its payload.
Do not skip unknown nested records unless their framing gives a safe boundary.
For example, an unknown native field-list member can prevent decoding the rest of that field list.
An unknown portable import kind can similarly prevent decoding the rest of its imports blob.

Preserving raw bytes does not prove that their references remain valid after an edit.
Block an edit when an opaque region could refer to indexes or offsets that the edit changes.
Do not add a switch that silently drops unknown data to make the build succeed.
Raw container replacement carries no PDB semantic-validity guarantee in either format.
For portable edits, opaque custom blobs can conceal row IDs, heap offsets or method-relative offsets.
Reject affected edits unless those references remain stable or a known codec can rewrite them.

## Writer plan and dependency checks

Build a plan before writing any output bytes:

1. Validate the requested edit and its capability requirements.
2. Determine every stream and index affected by the edit.
3. Assign stable IDs, resolve typed handles and determine final index widths.
4. Encode changed records, rebuild affected indexes and compute checked lengths.
5. Assign the backend's physical layout without changing the established reference widths.
6. Finalize format-specific identity and freeze the plan before output.

Native layout allocates MSF blocks and a stream directory.
Portable layout assigns heaps, table widths, metadata streams and root offsets.
Emit the frozen layout in output order.

Retain unchanged source streams without cloning their contents.
Keep changed payload storage proportional to the change where possible.
Planning may encode changed streams once for reuse during emission.
Do not hide a whole-file buffer or repeated record encoding behind `write_to`.
Portable identity hashing may traverse the planned bytes before emission without re-encoding them.
Allow an explicit `to_vec` convenience method for callers that want owned output.

A plan borrows immutable inputs or owns them. Edits cannot change a live plan.
A planning error leaves the source and destination untouched.
An output I/O error may leave a partial destination; document this contract.
Do not support writing over the source file or incremental in-place commits in this design.
The MSF file helpers (`WritePlan::write_file`, `io::copy_exact_file`) write a temporary sibling file and rename it.
On error, they delete the temporary file, so the destination does not change.

Native dependencies include:

- Type edits affect TPI/IPI hashes and any changed type or ID references.
- Symbol edits affect shared record offsets, scope links, module references and global/public indexes.
- Module edits affect DBI lengths, module stream references, source data and section contributions.
- Line edits affect C13 subsection lengths, string offsets and checksum references.
- Stream assignment affects DBI references, named streams and type hash stream references.

Native constructors must distinguish type handles from IPI handles before on-disk indices exist.
Resolve handles once in the plan. Reject unresolved references and integer-width overflow.
Do not add automatic type deduplication until its semantics and reference rewriting are specified.

Portable constructors use typed handles for documents, scopes, locals, imports and custom records.
Keep external assembly tokens stable; a PDB-only edit cannot renumber the assembly's metadata.
The portable plan must:

1. Preserve MethodDef row correspondence and resolve portable handles.
2. Order tables as required and derive contiguous local-variable and local-constant runs.
3. Rewrite affected row references, coded parents and references inside known blobs.
4. Assign heap offsets and GUID indexes, including references nested inside document and import blobs.
5. Select index widths from final heap sizes and both local and external table counts.
6. Encode table rows, sequence-point deltas and custom payloads with checked lengths.
7. Lay out streams and finalize the portable identity.

Heap growth and row-count thresholds can widen indexes across an entire table.
Do not assume that a local edit can always be emitted as a local byte patch.
Build plans must account for these width changes before serializing the affected tables.

## Identity and address semantics

Keep native and portable identity types distinct.
Expose the native GUID, age and signature without requiring a PE dependency.
Use GUID field encoding from the format; do not assume UUID text order matches file bytes.

Exact copy preserves all identity bytes. Native stream repack preserves the native identity.
A native semantic edit requires an explicit policy: preserve identity or supply a replacement.
Warn that preserving identity after semantic changes can misidentify symbols to debuggers.
Native builders receive identity values; do not hide random or clock-dependent input.
Keep native PDB Info and DBI ages consistent.
Represent native matching identity as a versioned value: modern GUID-and-age or legacy signature-and-age.
Keep the legacy signature as its own field; do not synthesize a GUID from it.

Portable identity is the 20-byte ID in `#Pdb`, not the native GUID-and-age pair.
For PE matching, it corresponds to the portable CodeView GUID plus the debug-directory timestamp.
The portable CodeView age is 1. The embedded PDB directory's stamp is zero, not the identity stamp.
Return the final identity and matching fields for callers that create or update PE debug entries.
Updating the PE remains the caller's responsibility.

Portable builders and semantic editors require an explicit ID policy: caller-supplied or content-derived.
Content-derived IDs use a documented deterministic provider over the planned output with the ID field zeroed.
Changing layout can change the content-derived identity even if decoded records remain equivalent.
Do not preserve the old portable ID implicitly during a rebuild.
Expose the portable checksum input with the same 20-byte zeroed field.
Do not confuse a matching identity with cryptographic proof of unchanged content.

Keep section-relative addresses, original RVAs and transformed RVAs distinct.
Use optional results for unmapped OMAP ranges; do not invent an address for deleted code.
Do not treat every symbol as having an address or every PDB as having section headers.
Keep portable IL offsets paired with MethodDef IDs. Native address conversion does not apply to them.
Defer a shared source-location query API until actual consumers establish the common semantics.

## Errors and validation

Use a structured error kind with format-specific locations.
Native context includes stream ID, logical offset, physical offset and record kind.
Portable context includes metadata stream, table, row, column or blob offset and custom kind GUID.
Use static context labels where possible. Formatting an error should not require temporary allocations.

Distinguish malformed data, unsupported format, unsupported edit, absent data, unresolved dependency,
invalid reference, integer overflow, resource limit and I/O failure.
A missing optional stream is not the same error as a truncated stream.
An unsupported feature is not proof that the container is malformed.

Use validation levels for container structure, records and cross-reference semantics in each backend.
Every level reports what was checked. Unknown content prevents a claim of full semantic validation.
Do not add a permissive mode that turns corruption into empty results.

## Deferred legacy native PDB support

Legacy support is planned, not implemented, and is not required for the initial modern-native and portable release.
It remains part of the eventual reader/writer scope.
Do not equate legacy support with signature detection or raw copying.

### Compatibility families

Track these axes independently. A container version alone does not identify all stream and record layouts.

| Axis | Planned legacy coverage | Design constraint |
| --- | --- | --- |
| Container | Small MSF, identified by the `program database 2.00` signature | Separate headers, directory entries, page-number widths and free-page-map rules from Big MSF |
| PDB streams | Older Info, TPI and DBI layouts, including files without IPI | Dispatch by actual stream version; do not require modern optional streams |
| CodeView records | Older type and symbol families, including narrow type indexes and length-prefixed names | Use record-kind-specific codecs and preserve original name bytes |
| Source lines | C11-era module line data alongside modern C13 data | Decode using the module's declared format and lengths |
| Matching identity | Signature-and-age matching associated with `NB10` PE records | Keep it distinct from `RSDS` GUID-and-age and portable content IDs |

`NB10` identifies a PE reference to a PDB, not the PDB container's own magic.
Standalone historical CodeView data and separate `.dbg` files are not automatically PDBs.
Keep any future support for those inputs separate from this legacy PDB requirement.

### Decisions required now

- Record container kind and individual stream versions in native format metadata.
- Keep format version separate from producer version; dispatch from the actual serialized structure.
- Make stream requirements depend on format capabilities, not on assumptions about modern PDBs.
- Retain raw payloads and original record kinds when a normalized view would lose encoding information.
- Decode narrow fields into checked internal IDs without assuming that every writer can encode those IDs.
- Allow separate read, preserve, edit and write capabilities for each verified compatibility profile.
- Return precise errors for recognized versions that are not implemented.
- Keep modern codecs strict; do not add heuristic fallback decoding after a modern parse fails.

Introduce a native output profile when legacy writing begins.
It selects the container, stream versions and permitted record encodings as a validated combination.
Use modern output for new files by default. Legacy output must be explicit.
An editor must not silently upgrade or downgrade its input to reach a supported writer.
Before emitting legacy output, check index widths, size limits and feature representability.
Reject data that cannot be represented; do not truncate values, drop records or replace names.

Microsoft recommends Big MSF for new encoders. Legacy emission serves explicit compatibility requirements,
not the default write path. Container conversion alone does not prove compatibility with an older debugger.

### Deferred delivery and acceptance

1. Assemble a fixture-backed matrix of producer versions, container kinds, stream versions and record families.
   Record provenance, redistribution rights and the independent consumer available for each profile.
2. Implement Small MSF decoding and validate its header, directory, free-page-map and width limits.
3. Add legacy Info/TPI/DBI, record and line codecs with semantic assertions from independent consumers.
4. Add explicit legacy writer profiles and editors with reference and representability checks.
5. Verify new files, preservation and edits with a consumer that supports the selected historical profile.

Preserve exact-copy and raw-payload guarantees throughout the work.
Test absent IPI, old identity matching, non-UTF-8 names, narrow-index boundaries and malformed lengths.
Test older streams in supported containers without assuming that every historical version combination is valid.
Verify that unsupported output features fail before emission and that edits retain the selected output profile.
Do not claim all historical PDBs are supported from one old fixture.
The fixture matrix defines verified coverage; known gaps remain explicit.

## Implementation order and acceptance

Modern native and Portable PDB read/write support are both initial requirements.
Completion of one backend does not complete the initial scope.
Legacy native support follows the deferred track above and does not block that initial release.
Keep separate dependency tracks rather than making portable work depend on finishing native CodeView support.

Initial modern-native track:

1. Implement MSF reading and writing together. Prove fragmented-stream reads, absent streams,
   empty streams, boundary fields, directory growth and free-page-map boundaries.
2. Add PDB Info and named streams. Prove identity and byte-preserving stream repacks.
3. Add record framing, TPI/IPI and DBI views. Preserve unknown data and expose external dependencies.
4. Add symbols, indexes, C13 data and address translation. Prove actual lookup results.
5. Add semantic builders and editors with the affected-index rebuilds for each operation.
   Prove that supported edits preserve unrelated streams and produce coherent references.

Portable track:

1. Implement metadata roots, tables, heaps, compressed integers and external row counts for read and write.
2. Implement all eight debug tables and their blobs. Prove document, method, scope and import results.
3. Add typed custom records, state-machine data, Source Link, embedded source and embedded PDB payloads.
4. Implement builders and editors with row sorting, reference rewriting, width changes and identity finalization.
5. Prove interoperability for new, edited and embedded PDBs with their matching managed assemblies.

Shared acceptance:

- Detect valid native and portable files without using their extension.
- Reject ordinary CLI metadata, malformed headers and unsupported formats without misclassification.
- Identify recognized legacy inputs and report unsupported capabilities until their codecs are implemented.
- Provide exact copying, format-specific semantic building and safe editing for both backends.
- Preserve unknown data or reject incompatible edits rather than dropping it.
- Keep backend-specific indexes, addresses and identities distinct throughout the public API.

For each writable record family, implement its reader, writer and dependent index updates together.
Do not advertise generic PDB editing while only raw stream replacement works.

Use independent consumers for interoperability. For native output, use `llvm-pdbutil` and a Windows
DIA-based consumer when available. Include fixtures from both MSVC and LLVM.
For portable output, use .NET `System.Reflection.Metadata` and a managed debugger integration scenario.
Include Roslyn C# and VB fixtures plus F# fixtures for compiler-specific records.
Include standalone and embedded files, async methods, iterators, multiple documents and methods without sequence points.
Record tool versions, fixture provenance, redistribution rights and any unavailable checks.
A writer-to-own-reader round trip cannot establish compatibility on its own.

Verify exact copies with full-byte equality. Verify native repacks with stream IDs, absence and content equality.
Verify native semantic builds through symbol names, type relationships, addresses, line results and identity.
Verify portable semantic builds through MethodDef correspondence, sequence points, scope ownership,
imports, constants, state-machine mappings, custom records and identity matching.
Require deterministic output for identical inputs and write options.
Test truncated and malformed inputs, unknown records, cross-block records and unsupported edits.
For portable output, cross heap and table index-width thresholds, including coded indexes affected by external row counts.
Test signed compressed integers, hidden sequence points, document changes, cyclic imports and invalid scope ranges.
Test raw and compressed embedded source, embedded PDB size mismatches and decompression limits.
Prove that incompatible edits with opaque custom records fail before writing output.
Fuzz parsing, record framing and parse/write/parse operations without treating self-consistency as interoperability.
Run both default features and `--no-default-features` once the library feature configuration exists.

## Research evidence and references

The full-read example decoded four local Rust build outputs with zero undecoded items and zero unread streams:
`target/debug/pdbrs.pdb` and the `inspect`, `streams` and `native` example PDBs.
LLVM `llvm-pdbutil` 22.1.6 agreed on these counts:

| Item | `pdbrs.pdb` | `inspect.pdb` | `streams.pdb` | `native.pdb` |
| --- | ---: | ---: | ---: | ---: |
| TPI records | 811 | 21,279 | 34,217 | 28,273 |
| Field-list members | 660 | 10,123 | 16,286 | 12,914 |
| IPI records | 6,281 | 15,754 | 21,736 | 18,963 |
| Modules | 73 | 90 | 90 | 96 |
| Module symbol records | 10,818 | 16,561 | 15,193 | 77,153 |
| Shared symbol records | 993 | 4,415 | 4,393 | 11,022 |
| Global index records | 567 | 3,856 | 3,849 | 7,913 |
| Public index records | 426 | 559 | 544 | 3,109 |
| Section contributions | 1,577 | 2,334 | 2,302 | 10,185 |
| Named streams | 9 | 9 | 9 | 9 |

The example PDBs were rebuilt during the work, so their counts differ from earlier per-area runs.
The per-area comparisons in GitHub issues #2 to #9 compared decoded fields with LLVM, not only counts.
They covered type leaves, forward references, hashes, IDs, symbols, annotations, line data, string tables,
DBI substreams, symbol indexes, free-page maps and injected sources.
LLVM 22.1.6 cannot decode `S_DEFRANGE_REGISTER_REL_INDIR` and hides some def-range gaps;
issue #5 records the independent checks for those records.

A separate consumer read `pdbrs.pdb` with default features disabled through `SeekReader`.
It decoded every TPI leaf, module symbol, C13 subsection, shared symbol and global index record.
A truncated copy of that PDB failed with `UnexpectedEof` and the physical end offset.

These are local Rust build outputs, not redistributed fixtures. Automated tests do not depend on them.
The tests use synthetic data for boundaries, malformed input and resource limits.
The smoke runs establish complete reading of these samples, not support for every producer or writer interoperability.
Legacy fixtures remain required by the acceptance plan.

### Producer fixtures

`cargo run -p pdbrs-fixtures` built 22 fixtures from the demo programs in `fixtures/` (see its README):
MSVC 14.51 and 14.44 (`/DEBUG:FASTLINK`), Clang/LLD 22.1.3 from Visual Studio, rustc 1.99 with `link.exe` and LLD,
and .NET 10 SDK with portable, embedded and Windows PDB output.
The full-read example read all 17 native fixtures to the end. It reported these gaps:

- MSVC and Clang C/C++ (#13): `S_HEAPALLOCSITE`, `S_FRAMECOOKIE`, `S_GTHREAD32`, `S_LTHREAD32`, x86 `S_BPREL32`,
  `S_DEFRANGE_SUBFIELD_REGISTER`, `S_REGISTER`, `S_FILESTATIC`, ARM64 `S_FRAMEREG`, and x86 FPO and new-FPO streams.
  Incremental links add `S_TRAMPOLINE`, `S_REGREL32_ENCTMP`, a non-empty `/TMCache` and the streams it uses.
  FASTLINK PDBs add `S_FASTLINK` and C13 subsection `0xfd`.
- .NET Windows PDBs (#14): managed procedures and slots, `S_OEM`, `S_MANCONSTANT`, `S_TOKENREF`,
  .NET document records in `/src/files/` streams (compression byte 101) and the `sourcelink` named stream.
  `llvm-pdbutil` 22.1.6 cannot dump their symbols. pdbrs now decodes all of them. Its managed procedures
  (32), blocks (12), slots (11), constants (2) and token references (32) agree with DIA2Dump. The five
  document records have a correct entry CRC and size. The two embedded sources (one raw, one DEFLATE) match
  their SHA-256 checksums. The 22 `MD2` and 3 `asyncMethodInfo` attributes decode with 40 records. Pdb2Pdb
  converts the fixture to a Portable PDB with the same imports, hoisted scopes, async steps and edit-and-continue
  data. Only the kind 9 data differs: the Roslyn reader keeps its alignment bytes.

After #13, the full-read example decodes every symbol kind and C13 subsection of the MSVC, Clang and rustc fixtures.
The decoded fields and the counts per kind match `llvm-pdbutil` 22.1.6. `llvm-pdbutil` cannot decode `S_FRAMEREG`,
`S_FASTLINK`, `S_REGREL32_ENCTMP` and subsection `0xfd`; the record bytes and the object files of the link confirm
them. The example joins all 532 `S_TRAMPOLINE` records of the incremental fixture to the 532 public thunk map
entries, and resolves the `S_FILESTATIC` file name in `/names`.

The fixtures also exposed a producer defect that pdbrs reports without failing:

- LLD 22.1.3 puts some public symbols (`__chkstk`, `g_c_name`, `___favor`) in the wrong hash bucket.
  LLD 22.1.6 puts `__chkstk` in the correct bucket. Each of these names has exactly 8 bytes. LLD hashes the
  name together with the bytes after the short name in the COFF symbol table, up to a NUL byte (#30).

The fixtures found two reader defects, now fixed. Incremental links give end markers and empty COFF groups an
offset equal to the section size; `AddressMap::rva` now accepts the end of a section. Managed procedures open
scopes; the scope walker now decodes them and checks their links instead of reporting a corrupt scope end.
An empty `/src/headerblock` stream now means that the PDB has no injected sources.

Evidence for the FPO, `/TMCache` and FASTLINK decisions of #13:

- The x86 fixtures have 3, 2 and 3 FPO records and 489, 1307 and 1841 frame data records. All frame data
  fields and frame programs match `llvm-pdbutil dump --fpo`. The FPO fields also match, except `fHasSEH`:
  LLVM reads bit 9 of the attribute word, but `winnt.h` puts it at bit 11. The records with bit 11 set are
  SEH filter blocks of procedures with `has seh` frames.
- The layout of `/TMCache` and its type map streams comes from `DBI1::DumpTMCache` in `mspdbcore.dll`
  (MSVC 14.51). Its output agrees with the decoded values. In the incremental fixture, 43 entries name 42
  type map streams. One more stream is byte-identical to a named one: two inputs with the same checksum
  share an entry. The example reports it as a type map stream that `/TMCache` does not name.
- `GSI1::hashSz` returns the 16-bit `HASH` type, so the bucket is `(hash % count) & 0xffff`. With this rule,
  lookup finds all 836 records of the MSVC 14.44 FASTLINK public index.

The signature probe was also exercised against a .NET-produced Portable PDB, an embedded PDB payload and ordinary CLI assembly metadata.
A synthetic Small MSF signature was identified. These checks establish signature behavior, not portable or legacy parsing.
The remaining format contracts come from the references below.

References used for this proposal:

- [Portex README](https://github.com/coconutbird/portex/blob/main/README.md): API roles, preservation and I/O.
- [Portex writer](https://github.com/coconutbird/portex/blob/main/src/pe/build.rs): fallible layout and serialization.
- [LLVM PDB overview](https://llvm.org/docs/PDB/index.html): MSF, PDB streams and CodeView boundaries.
- [LLVM MSF format](https://llvm.org/docs/PDB/MsfFile.html): blocks, directory, free maps and fragmentation.
- [LLVM PDB Info](https://llvm.org/docs/PDB/PdbStream.html): identity, named streams and features.
- [LLVM TPI/IPI](https://llvm.org/docs/PDB/TpiStream.html): type indices, hashes and record layout.
- [LLVM DBI](https://llvm.org/docs/PDB/DbiStream.html): module data and cross-stream references.
- [Microsoft pdb-rs documentation](https://github.com/microsoft/pdb-rs/blob/main/docs/index.md): complementary format references.
- [Portable PDB format](https://github.com/dotnet/runtime/blob/main/docs/design/specs/PortablePdb-Metadata.md): tables, heaps, blobs and external references.
- [CLI PE/COFF addendum](https://github.com/dotnet/runtime/blob/main/docs/design/specs/PE-COFF.md): portable identity, embedded payloads and checksum rules.
- [Microsoft MSF formats](https://github.com/microsoft/pdb-rs/blob/main/docs/pdb/msf.md): Small/Big MSF distinctions and compatibility limits.
- [LLVM CodeView debug references](https://github.com/llvm/llvm-project/blob/main/llvm/include/llvm/Object/CVDebugRecord.h): `NB10` and `RSDS` matching structures.

Format documentation has historical assumptions and incomplete sections. Use it to guide implementation,
then verify each supported layout with source references and independent producer and consumer fixtures.
