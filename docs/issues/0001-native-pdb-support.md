# NATIVE-0001: Complete native PDB support

Status: open

Baseline: `2c6957b` (`pdbrs: add native container and core stream readers`).

GitHub issue: [napbat/pdbrs#1](https://github.com/napbat/pdbrs/issues/1).
Use the GitHub issue for live progress. Keep this file as the repository-local scope and acceptance record.

## Goal and boundaries

Complete native PDB inspection, preservation, semantic editing and new-file construction.
Cover MSF containers, PDB streams and CodeView records separately. MSF 7.0 support alone does not complete this ticket.

Use the [design](../design/pdb.md) as the product contract and the [pinned references](../reference/README.md) as format evidence.
The [source manifest](../reference/SOURCES.json) identifies the exact upstream revisions and files.
The documentation is incomplete. A missing document or an unknown record in LLVM is not permission to discard data.

Modern MSF 7.0 is the initial native release target. Legacy native support remains required on a deferred track.
Portable PDB support remains a separate initial requirement. Closing the modern-native work does not complete Portable PDB support.
MSFZ is a separate container, not MSF 7.0. Track its disposition explicitly; do not imply support through the native label.
Do not add a PE parser, linker, symbol server client or automatic external-file fetching.

## Current evidence

- [x] Read MSF 7.0 directories and fragmented streams. Keep nil and empty streams distinct.
- [x] Decode modern PDB Info, named streams and feature codes.
- [x] Read TPI/IPI headers and frame indexed records without claiming leaf semantics.
- [x] Read modern DBI headers, module records and referenced stream IDs.
- [x] Frame shared and module symbols. Decode `S_PUB32`, `S_LDATA32`, `S_GDATA32`, `S_LPROC32`, `S_GPROC32`, `S_LPROC32_ID` and `S_GPROC32_ID`.
- [x] Frame C13 subsections. Keep unknown kinds raw.
- [x] Match LLVM record/module counts on four local Rust-produced PDBs. Exercise native reading with default features disabled.
- [x] Decode every TPI leaf and field-list member in the four Rust PDBs. Match LLVM `dump --types` names, sizes, members, enumerators, pointers, signatures, method lists and forward-reference targets.
- [x] Verify all stored TPI/IPI hash values and index offset pairs against the records.
- [x] Read every stream, record and C13 subsection of four Rust-produced PDBs with zero undecoded items (`examples/native.rs`).

The [design evidence](../design/pdb.md#research-evidence-and-references) records the smoke results and their limits.
No native writer or semantic editor exists at this baseline.

## Rust sample milestone

The GitHub sub-issues of #1 track complete reading of the Rust-produced samples. Each issue holds its evidence comment.

- [x] #2 TPI leaves, field lists, lookup and forward references.
- [x] #3 TPI/IPI hash values, index offsets and adjusters.
- [x] #4 IPI ID records.
- [x] #5 Every Rust symbol kind, scope links, def-ranges and inline annotations.
- [x] #6 C13 lines, file checksums and inlinee lines.
- [x] #7 `/names`, DBI substreams, section headers and address translation.
- [x] #8 Global and public symbol indexes.
- [x] #9 Free-page map, stream 0 snapshot and auxiliary named streams.
- [x] #10 Facade integration and the full-read example.

The samples do not contain every format variant. The checklists below stay open for variants outside them.

## Producer fixture milestone

`cargo run -p pdbrs-fixtures` builds MSVC, Clang/LLD, Rust and .NET fixtures from `fixtures/`.
The full-read example reads every native fixture to the end and lists the remaining gaps.

- [x] #11 LLD and clang-cl kinds: `S_SKIP`, `LF_VFTABLE` and `LF_LABEL`.
- [x] #12 Fixture demo programs and generator.
- [ ] #13 MSVC and Clang C/C++ record kinds, FPO streams, incremental and FASTLINK data.
- [ ] #14 .NET Windows PDB records, compressed injected sources and `sourcelink`.

## Implementation map and starting point

Start with integer numeric leaves, then field lists and typed TPI records.
These expose Rust type sizes, field offsets, enum values and function signatures.
Do not start with hash acceleration or writing: neither adds meaning to opaque records.

All paths below are relative to `crates/pdbrs/src/`.
Steps 1 to 9 are implemented for the record kinds in the Rust samples. Step 10 is not started.

| Order | Module boundary | Deliverable and dependencies |
| --- | --- | --- |
| 1 | `native/codeview/numeric.rs` | Implemented integer leaves. Shared by type sizes, member offsets, enumerators and integer constants. |
| 2 | `native/codeview/field_list.rs` | Implemented borrowed field-list member iteration, attributes and continuations. Uses numeric leaves; unknown unframed members stop decoding. |
| 3 | `native/codeview/type_record.rs` | Implemented borrowed typed leaves: modifiers, pointers, arrays, aggregates, enums, procedures, member functions, argument lists, bitfields, method lists and vtable shapes. |
| 4 | `native/types/lookup.rs`, `native/types/forward.rs`, `native/types/hash.rs` | Implemented offset-table lookup, bounded continuation chains, forward-reference resolution and hash-stream decoding and verification. Keep stream ownership in `TypeStream`. |
| 5 | `native/codeview/id_record.rs` | Implemented IPI strings, function IDs, build info and UDT source references, with `IdTable` lookup over `TypeTable`. |
| 6 | Existing `native/codeview/symbol.rs` and children | Implemented 38 symbol kinds, scope links, def-range gaps and inline-site annotations. |
| 7 | `native/lines.rs` and children | Implemented C13 line blocks, columns, file checksums and inlinee lines. |
| 8 | Existing `native/dbi.rs` and children; `native/names.rs`, `native/section.rs`, `native/address.rs` | Implemented string tables, section contributions/maps, source files, section headers and RVA translation. OMAP is not decoded. |
| 9 | `native/symbols.rs` and children | Implemented GSI/PSI hash tables, lookup candidates, address maps and thunk data. |
| 10 | `native/msf/write.rs`, then native builder/editor modules | Implement checked output plans, container repacking, semantic encoders and dependent-index rebuilds. No public write API before a real output path exists. |

Steps 7 and 8 can proceed separately from full type resolution once their reference contracts are defined.
Auxiliary streams, compatibility variants and complete record coverage remain required by the checklists below.
This order prioritizes the Rust sample; it does not make that sample the format specification.

### Shared contracts

- Decode payloads into borrowed views. Keep names and unknown payloads as bytes.
- Return the unconsumed suffix when decoding embedded variable-length values. Do not assume alignment between fields.
- Keep framing, payload decoding and cross-record resolution separate.
- Represent primitive type indices explicitly. Do not index them into TPI.
- Keep type indices, IPI IDs, stream IDs and record offsets distinct in new semantic APIs.
- Preserve unknown top-level records. Never skip an unknown unframed field-list member by guessing its length.
- Keep allocations fallible and optional indexes bounded. Bound continuation chains and recursive resolution.
- Attach stream/record locations at the owning layer. Shared payload decoders must not invent physical offsets.
- Add each new module only when it implements exercised behavior. Do not add empty modules or methods that return dummy results.

### First completed component

`codeview::numeric::Integer::parse_prefix` decodes immediate unsigned values and all nine explicit integer leaf tags.
It retains signedness through 128 bits and returns the untouched suffix without allocation.
Non-integer and unknown tags return `UnsupportedVersion`; truncated integer encodings return `UnexpectedEof`.
Floating-point, complex, decimal, date and string representations remain separate work.

Boundary tests cover every integer width, sign extremes, the immediate/tag boundary, truncation and suffix preservation.
A temporary consumer decoded all 40 shared `S_CONSTANT` integer values in the original Rust PDB.
Names and values matched LLVM exactly in both feature modes, including `18446744073709551615`.
`Record::symbol` now uses this decoder for `S_CONSTANT` values (#5).

### Next acceptance milestone: Rust type inspection

- [x] Decode field lists and the aggregate, enum, pointer, modifier and function families present in the Rust sample.
- [x] Resolve field types and continuation records without recursion failures or repeated full-stream scans.
- [ ] Print type names, sizes, field names/offsets, enum values and function signatures from a real PDB.
- [x] Compare those semantic results with LLVM, not only record counts.
- [x] Preserve unknown leaves and report incomplete semantic coverage explicitly.
- [x] Exercise signed enum values, recursive types, forward declarations, malformed members and both feature modes.

## Coverage accounting

- [ ] Reconcile every imported document against the implementation. Record conflicts and undocumented assumptions.
- [ ] Inventory stream versions, type leaves, field-list members, numeric leaves, symbol kinds and C13 kinds from pinned implementation definitions as well as prose.
- [ ] Give every concrete kind and version its own coverage row. Separate aliases, legacy forms and producer-specific extensions.
- [ ] Track raw preservation, semantic reading, reference validation, writing, editing/index rebuilding and independent fixture evidence in separate columns.
- [ ] Use `missing`, `partial`, `complete` or `not applicable` in each column. Explain every `not applicable` decision.
- [ ] Link each complete cell to its implementation and consumer-visible verification. Raw framing must not mark semantic reading complete.
- [ ] Keep recognized but unsupported versions distinct from corrupt input. Report unresolved external dependencies explicitly.
- [ ] Require an explicit scope decision for formats outside the settled design, including MSFZ. Do not silently drop documented formats.

## Container and PDB streams

- [x] Finish MSF validation, including active free-page-map allocation consistency, reserved intervals, boundary sizes and directory growth.
- [ ] Implement exact copying and stream-preserving repacking. Preserve stream IDs, nil slots, empty streams and unknown stream bytes.
- [ ] Implement deterministic MSF writing with coherent directory pages and free-page maps. Validate output plans before writing.
- [ ] Complete Info serialization, named-stream hash tables, feature handling and the native GUID/age identity policy.
- [ ] Complete `/names` string tables and their hashes. Preserve raw names and supported encodings.
- [ ] Complete TPI/IPI type lookup, hash values, index-offset buffers, hash adjusters and auxiliary references.
- [ ] Complete DBI section contributions and their versions, section maps, source-file tables, type-server maps and EC data.
- [ ] Complete module stream regions and global references. Validate symbol scope and module/record offsets.
- [ ] Complete GSI/PSI lookup, hash buckets, address maps and thunk-related tables. Rebuild all affected indexes after edits.
- [ ] Complete optional debug streams: FPO, new FPO, exceptions, fixups, section headers, original section headers, OMAP, token/RID maps, `.pdata` and `.xdata`.
- [ ] Implement section-relative address and RVA conversion. Handle both OMAP directions and missing mappings explicitly.
- [ ] Inventory auxiliary named streams, embedded source, Natvis payloads, source-server/source-link data, `/LinkInfo` and producer-specific metadata.
- [ ] Expose type-server, precompiled-type and FASTLINK/mini-PDB dependencies without silently loading external files.

## CodeView semantics

- [ ] Decode primitive types, pointer modes, calling conventions and numeric leaves, including signed, unsigned and extended values.
- [ ] Decode all modern type and ID families. Include modifiers, pointers, arrays, classes, structures, interfaces, unions, enums, procedures and member functions.
- [ ] Decode argument lists, field lists, member records, method lists, bitfields, vtables and continuation records.
- [x] Decode function IDs, member-function IDs, string IDs, substring lists, build info and UDT source-line records.
- [ ] Resolve forward declarations and type/ID references. Detect invalid references and bound cycles, depth and traversal work.
- [ ] Complete symbol families beyond the initial seven kinds. Include constants, UDTs, TLS, locals, registers, relative addresses and location ranges.
- [ ] Decode scope/end records, blocks, labels, thunks, trampolines, references, compile/object metadata and namespace records.
- [ ] Decode inline sites, call sites, caller/callee records, annotations, frame data, section/COFF-group records and architecture-specific symbols.
- [ ] Validate symbol links, type references, module references, range gaps and optimized-variable locations.
- [ ] Decode C13 line blocks, columns, file checksums, string tables, inlinee lines, cross-scope data and applicable remapping/frame subsections.
- [ ] Decode inline binary annotations and associate addresses with source files, lines and inline call chains.
- [ ] Preserve unknown kinds and ignore flags. Reject semantic edits that could invalidate opaque references.

## Writers and editors

- [ ] Implement each writable record family with its reader, encoder and dependent-index updates.
- [ ] Build new native PDBs with coherent Info, TPI/IPI, DBI, modules, symbols and line information.
- [ ] Plan type-index, ID-index, stream-ID and symbol-offset remapping before serialization.
- [ ] Rebuild hashes, name tables, address maps, scopes and other dependent structures after supported edits.
- [ ] Preserve unrelated streams and raw names. Reject unsupported transformations before output begins.
- [ ] Apply an explicit identity policy. Keep PDB Info and DBI ages consistent and return PE matching fields to the caller.
- [ ] Prove deterministic output for identical inputs and options. Distinguish deterministic reconstruction from exact copying.

## Deferred legacy track

- [ ] Support Small MSF/PDB 2.0 through a separate container codec.
- [ ] Inventory older Info and DBI layouts, absent IPI, old type indices, Pascal-style names and old CodeView kinds.
- [ ] Implement C11 line data and NB10 signature/age matching.
- [ ] Implement legacy preservation, editing and writing with explicit representability checks. Do not silently upgrade or downgrade formats.

These tasks do not block the initial modern-native release, but remain open requirements.

## Verification and release gates

- [ ] Keep redistributable MSVC, clang-cl/LLD and Rust fixtures with producer versions, provenance, licenses and content hashes.
- [ ] Cover x86, x64 and ARM64, optimized and unoptimized code, stripped PDBs, incremental linking and external-type cases.
- [ ] Compare actual type relationships, symbol names/addresses, source lines, inline chains and variable locations with independent consumers.
- [ ] Validate new and edited files with `llvm-pdbutil` and a Windows DIA-based consumer where available. Record unavailable checks.
- [ ] Verify exact copies by byte equality and repacks by stream ID, presence and content equality.
- [ ] Verify edits through independent lookup results, not only writer-to-own-reader round trips.
- [ ] Cover malformed/truncated data, integer boundaries, resource limits, cross-block records and unknown kinds in deterministic tests.
- [ ] Fuzz container parsing, record framing and semantic decoding. Fuzz read/write transformations without treating self-consistency as interoperability.
- [ ] Run formatting, Clippy, tests and rustdoc in both feature modes. Exercise real files through file, slice and seek-adapted inputs.
- [ ] Update the design coverage table and public API documentation with every completed family and every remaining limitation.

Do not close the modern-native milestone while any required modern coverage cell is missing or partial.
Do not close this umbrella ticket until the deferred legacy track is complete or explicitly moved to a linked, open successor ticket.
