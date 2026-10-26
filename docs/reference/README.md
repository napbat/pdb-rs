# Native PDB reference snapshots

These are upstream documents, not the pdbrs specification. Read the [design](../design/pdb.md) and GitHub issues [#1](https://github.com/napbat/pdbrs/issues/1) (native) and [#17](https://github.com/napbat/pdbrs/issues/17) (Portable PDB) for project requirements.

## Sources

| Source | Local entry point | Pinned revision | Files |
| --- | --- | --- | ---: |
| [LLVM PDB manual](https://llvm.org/docs/PDB/index.html) | [PDB index](llvm/llvm/docs/PDB/index.md) | `b9f2235fc3ac1948ef7e63954b04167cd0749088` | 12 |
| [Microsoft pdb-rs](https://github.com/microsoft/pdb-rs/tree/2081a0dd6f1150cf965dec10855e75cfecfde176/docs) | [PDB and CodeView](microsoft-pdb-rs/docs/index.md) | `2081a0dd6f1150cf965dec10855e75cfecfde176` | 80 |
| [Original Microsoft PDB project](https://github.com/microsoft/microsoft-pdb/tree/805655a28bd8198004be2ac27e6e0290121a5e89) | [Project README](microsoft-pdb/README.md), [external resources](microsoft-pdb/docs/ExternalResources.md) | `805655a28bd8198004be2ac27e6e0290121a5e89` | 3 |

Counts include license files. The snapshot contains every file in LLVM's `llvm/docs/PDB/` and both Microsoft `docs/` trees.
The original Microsoft project has one file in its `docs/` tree. Its README is included separately.
External sites, linked papers and implementation source trees are not mirrored.

[SOURCES.json](SOURCES.json) records each upstream path, revision, Git blob ID, SHA-256 digest and byte count.
The imported files match their pinned upstream Git blobs byte for byte. Keep upstream text and paths unchanged.
Do not apply local writing rules or formatting to these snapshots. Put corrections and interpretation in project-owned documents.

## Reading the references

MSF 7.0 specifies the container. PDB streams and CodeView records have separate versions and layouts.
Neither documentation set is complete. Microsoft explicitly marks its pdb-rs documents as non-authoritative.
LLVM's record lists also omit some record kinds. Do not use either list as a closed-world support boundary.
Resolve conflicts through pinned implementation sources and independent producer and consumer fixtures.
The [native coverage](../design/native-coverage.md#reference-reconciliation) document lists each imported document with its conflicts and the undocumented assumptions of pdbrs.

LLVM uses MyST roles, labels and directives. A plain Markdown viewer does not render all cross-references.
Relative file paths are preserved. Links outside the copied trees still require upstream resources.
The snapshot also retains upstream links to missing documents. Those links are not evidence that a codec exists.

## Licenses

- LLVM: [license and notices](llvm/llvm/LICENSE.TXT), including Apache-2.0 with LLVM exceptions and the legacy license text.
- Microsoft pdb-rs: [MIT license](microsoft-pdb-rs/LICENSE).
- Original Microsoft PDB project: [MIT license](microsoft-pdb/LICENSE).

The imported documents retain their upstream license terms. The provenance manifest does not replace those licenses.

## Updating a snapshot

1. Resolve each upstream branch to a full commit ID.
2. Enumerate the complete documentation subtree at that commit. Include its license files.
3. Download the source files without rendering or rewriting them.
4. Verify each file against its Git blob ID. Record its SHA-256 digest and byte count.
5. Update `SOURCES.json` and this index together. Record coverage changes in the GitHub issues.
6. Review new, changed and removed formats. Do not remove coverage requirements merely because upstream removed a document.
