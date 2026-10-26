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

The [design evidence](../design/pdb.md#research-evidence-and-references) records the smoke results and their limits.
Counts prove enumeration, not complete semantics. No native writer or semantic editor exists at this baseline.

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

- [ ] Finish MSF validation, including active free-page-map allocation consistency, reserved intervals, boundary sizes and directory growth.
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
- [ ] Decode function IDs, member-function IDs, string IDs, substring lists, build info and UDT source-line records.
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
