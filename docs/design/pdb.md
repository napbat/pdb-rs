# PDB reader and writer design

Status: the full reader/writer design remains proposed. The library reads every stream, record and subsection in the Rust-produced sample PDBs.
Formats and record kinds outside those samples remain incomplete. Portable PDB parsing and PDB writing are not implemented.

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
- `format::probe` identifies modern MSF, legacy Small MSF, CLI metadata and embedded PDB envelope signatures.
  It examines a bounded prefix, not container or record validity.
  `ContainerKind::CliMetadata` deliberately does not assert that the input is a Portable PDB.
- `Error` retains an error kind, physical or stream-relative location, operation context and a typed `nostdio::IoError`.
- `native::Identity` distinguishes modern GUID-and-age from legacy signature-and-age.
- `portable::Identity` encodes the 20-byte ID and checks GUID, stamp and age fields from a portable PE reference.

Run signature inspection on Unix or Windows with:

```sh
cargo run -p pdbrs --example inspect -- path/to/input.pdb
```

Exercise sequential output and positional input in either feature mode:

```sh
cargo run -p pdbrs --example streams
cargo run -p pdbrs --example streams --no-default-features
```

The examples report their validation scope. Editors, builders and embedded compression are not exposed as placeholder APIs.

## Implemented native reading

`native::Pdb::open` opens a file under `std`. `Pdb::parse` borrows a byte slice.
`Pdb::read` and `Pdb::with_options` accept any stable `ReadAt` source, including `SeekReader`.
These source-based APIs also work with `no_std + alloc`.

Opening reads the MSF directory and PDB Info stream. Other typed accessors load one requested stream at a time.
Raw `msf::Stream` reads join fragmented blocks without loading the complete stream.
The default limits are 64 MiB for the directory and 64 MiB for each typed stream load.
Callers can change these limits through `ReadOptions`. Allocation failures return `LimitExceeded`.

| Area | Implemented | Not yet implemented |
| --- | --- | --- |
| MSF 7.0 | Directory, nil and empty streams, fragmented reads, reserved and duplicate page checks, active free-page-map consistency and the stream 0 snapshot | Writing and repacking |
| PDB Info and named streams | Identity, `niMac`, feature codes, named-stream slot placement, `/LinkInfo` version 2, `/TMCache` and its type map streams, `/UDTSRCLINEUNDONE`, `/src/headerblock`, verified uncompressed injected sources, .NET document records (compression byte 101) with CRC, size and SHA-256 checks through the shared `portable::embedded` codec, and `sourcelink` JSON bytes | `/LinkInfo` version 1 and run-length, Huffman or LZ injected sources |
| TPI and IPI | Modern headers, indexed record iteration, typed TPI leaves (including `LF_VFTABLE` and `LF_LABEL`) and field lists, offset-table lookup, continuation chains, forward-reference resolution, hash values, index offsets and hash adjusters | 32-bit-option UDT leaves, based-pointer data, auxiliary hash streams and type graphs |
| DBI | Modern header, module records, debug-stream references, section contributions (VC++ 4.0, V60 and V2), section map, source file info, empty type server map, EC names, and FPO and new-FPO (frame data) streams | Non-empty type server maps |
| String tables | `/names` and EC layout, offset lookup, version 1 hash lookup and bucket checks | Version 2 hashing |
| Sections and addresses | Section header streams and section-offset to RVA translation in both directions; end-of-section offsets translate to RVAs | OMAP translation |
| Symbol indexes | Global and public hash tables, bucket checks, name lookup candidates with the 16-bit bucket rule of minimal (FASTLINK) indexes, address maps, thunk maps and section starts | Pre-compressed-bucket hash layouts |
| Symbols | Typed decoding of 57 symbol kinds, including every kind in the Rust sample PDBs, the MSVC, Clang and rustc producer fixtures (#13), the LLD `S_SKIP` filler and the managed kinds of .NET Windows PDBs (#14: `S_GMANPROC`, `S_LMANPROC`, `S_MANSLOT`, `S_MANCONSTANT`, `S_TOKENREF`, `S_OEM` with the Roslyn `MD2` custom debug information and `asyncMethodInfo` attributes); zero-padding checks, module scope-link checks, def-range gaps and inline-site binary annotations. `S_FRAMEREG`, `S_FASTLINK` and `S_REGREL32_ENCTMP` layouts come from fixtures (#13) | Other symbol kinds (kept raw as `Unsupported`) |
| Module lines | Region bounds, C13 subsection framing, line blocks with columns, file checksums, inlinee lines, COFF symbol RVAs (`0xfd`) and checksum reference validation | String table, frame data, cross-scope, IL line and token-map subsections |
| Numeric leaves | Immediate and signed/unsigned integer leaves through 128 bits, with suffix-preserving decoding, in typed records and field lists; `S_MANCONSTANT` also keeps `LF_VARSTRING` and fixed-size real, complex, decimal and date leaves | Non-integer leaves in other records |

Unknown record kinds retain their raw payloads. Names remain bytes; display code chooses how to decode them.
Header validation does not validate every record payload. Iterate records to check their framing.
Unsupported container and stream versions return `UnsupportedVersion`; no heuristic fallback parses them as modern data.
Full native support remains required. This coverage table does not reduce the scope below.

Read a complete native PDB with:

```sh
cargo run -p pdbrs --example native -- path/to/input.pdb
```

The example decodes every stream role and checks the references between streams.
It joins line and inlinee data to `/names`, procedure references to module symbols and index records to name lookup.
It translates symbol, contribution and line addresses to RVAs, and counts the absolute addresses that have none.
It fails if any record kind, subsection kind or stream has no typed decoder.
It reports a decoded stream that no other stream names, with the reason, and does not fail for it.

`Pdb::stream_bytes` and `Pdb::named_stream_bytes` load bounded stream bytes for decoders that borrow their input.
`Pdb::global_symbols` and `Pdb::public_symbols` load the symbol indexes that the DBI header names.
`Pdb::fpo` and `Pdb::frame_data` load the FPO and new-FPO streams that the DBI optional debug header names.
GitHub issues #2 to #10 record the implementation and the evidence for each area.

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
| `native::Msf<S>` | Own a stable positional source and validated stream-directory metadata |
| `native::Msf::stream(StreamId)` | Distinguish an invalid index, an absent stream and a present stream |
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
| `portable::decode_embedded`, `encode_embedded` | Decode or emit an `MPDB` payload; PE directory handling stays with the caller |

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

A record cursor may reuse a bounded scratch buffer for fragmented records.
Use a lending `next_record` method for views tied to that buffer, not an ordinary `Iterator`.
Offer an explicit owned conversion for callers that retain records.
Offer lazy native type-offset indexing; do not build a complete type graph during open.
Do not require native type references to form a strictly ordered DAG when reading.
For portable tables, calculate row widths once and read individual rows directly.
Keep sequence-point and custom-blob decoding lazy.

Keep metadata proportional to directory or table metadata and requested indexes.
Bound directory bytes, stream counts, record buffers, nesting, decompressed bytes and optional index allocations.
Expose documented limits and return `LimitExceeded` before crossing them.
Do not allocate a buffer from an unchecked length in the file.

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

Initially support 512, 1024, 2048 and 4096-byte MSF 7.0 blocks, with 4096 as the writer default.
Microsoft's newer MSF documentation also describes larger Big MSF page sizes.
Treat recognized but unimplemented sizes as unsupported, not necessarily malformed.
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
A later filesystem helper can handle temporary files and platform-specific replacement semantics separately.

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
  LLD 22.1.6 puts `__chkstk` in the correct bucket.

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
