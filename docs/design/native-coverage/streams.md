# Native coverage: MSF container and PDB streams

This page is part of the [native coverage](../native-coverage.md) accounting. Read the [column and value definitions](../native-coverage.md#columns) first.
The version lists come from LLVM `RawConstants.h` (`PdbRaw_ImplVer`, `PdbRaw_TpiVer`, `PdbRaw_DbiVer`, `PdbRaw_DbiSecContribVer`, `PdbRaw_FeatureSig`, `DbgHeaderType`), `cvinfo.h` and Microsoft `pdb.h` and `dbi.h` as the pdbrs sources cite them.

Every stream keeps its bytes: `Pdb::stream_bytes` reads any stream, and `Msf::repack` and `copy_exact` keep every stream byte (#15).
A recognized version that pdbrs does not decode gives `UnsupportedVersion`. Malformed data gives `InvalidFormat`, `InvalidReference` or `UnexpectedEof`. The two classes stay distinct.
An unknown version value also gives `UnsupportedVersion`. pdbrs does not separate it from a recognized older version.

## MSF 7.0 container

MSFZ is a different container (#20) and Portable PDB is a different format (#17). This document does not count them.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Page size 512 | `512` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][exm], [check][e15]) | 1 fixture PDB |
| Page size 1024 | `1024` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | not applicable (no producer known: `link.exe` accepts only 4096 to 32768; pdbrs output passes DIA2Dump, `llvm-pdbutil` and `pdbtool` (#15)) | No fixture |
| Page size 2048 | `2048` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | not applicable (no producer known: `link.exe` accepts only 4096 to 32768; pdbrs output passes DIA2Dump, `llvm-pdbutil` and `pdbtool` (#15)) | No fixture |
| Page size 4096 | `4096` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][exm], [check][e15]) | 27 fixture PDBs |
| Page size 8192 | `8192` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | missing (#27: `link.exe /PDBPAGESIZE` makes this size) | No fixture |
| Page size 16384 | `16384` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | missing (#27: `link.exe /PDBPAGESIZE` makes this size) | No fixture |
| Page size 32768 | `32768` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | missing (#27: `link.exe /PDBPAGESIZE` makes this size) | No fixture |
| Page size 65536 | `65536` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | not applicable (DIA, LLVM and `link.exe` reject 65536-byte pages; the writer returns `UnsupportedVersion` (#15)) | not applicable (DIA, LLVM and `link.exe` reject 65536-byte pages; the writer returns `UnsupportedVersion` (#15)) | not applicable (no producer writes it; `link.exe` returns `LNK1399`) | No fixture; the reader accepts it |
| Superblock (header page) | `Microsoft C/C++ MSF 7.00` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][exm], [check][e15]) | 28 fixture PDBs |
| Active free page map FPM1 | `1` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][fpm], [check][t:free_page_map], [check][e9]) | complete ([code][fpm], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][fpm], [check][e9], [check][e15]) | 22 fixture PDBs (MSVC, .NET) |
| Active free page map FPM2 | `2` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][fpm], [check][t:free_page_map], [check][e9]) | complete ([code][fpm], [check][t:free_page_map], [check][e9]) | not applicable (the writer always makes FPM1 active and fills FPM2 with `0xFF`; consumers accept both) | not applicable (the writer always makes FPM1 active and fills FPM2 with `0xFF`; consumers accept both) | complete ([code][fpm], [check][e9], [check][e15]) | 6 fixture PDBs (LLD) |
| Free page map intervals after the first | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][fpm], [check][t:free_page_map]) | complete ([code][fpm], [check][t:free_page_map]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | missing (#27: no fixture is larger than one interval (16 MiB at 4096-byte pages); pdbrs repacks with 512-byte pages pass the consumers (#15)) | The interval length is one page size of pages |
| Stream directory with one page-map page | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf], [check][e9]) | complete ([code][mdir], [code][fpm], [check][t:msf], [check][t:free_page_map], [check][e9]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][mw], [code][mwl], [check][t:msf_write], [check][e15]) | complete ([code][exm], [check][e15]) | 28 fixture PDBs |
| Stream directory with more than one page-map page | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [code][mdir], [check][t:msf_write]) | complete ([code][mdir], [check][t:msf_write]) | complete ([code][mw], [code][mwl], [check][t:msf_write]) | complete ([code][mw], [code][mwl], [check][t:msf_write]) | not applicable (LLVM rejects it ("Too many directory blocks"); only very large directories need it) | Tested with the pdbrs reader only (#15) |
| Nil stream (size `0xffffffff`) | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [check][t:msf]) | not applicable (no contents) | complete ([code][mw], [check][t:msf_write]) | complete ([code][mw], [check][t:msf_write]) | complete ([code][exm], [check][e27]) | 1 fixture PDB (`msvc-x64-pgo`, 7 nil streams); distinct from an empty stream |
| Empty stream (size 0) | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][msf], [check][t:msf]) | not applicable (no contents) | complete ([code][mw], [check][t:msf_write]) | complete ([code][mw], [check][t:msf_write]) | complete ([code][exm], [check][e15]) | Stream 0 of the LLD fixtures |
| Stream 0, previous directory (`OldDirectory`) | stream 0 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][old], [check][t:old_directory], [check][e9]) | complete ([code][old], [check][t:old_directory], [check][e9]) | complete ([code][mw], [check][t:msf_write], [check][e15d]) | complete ([code][mw], [check][t:msf_write], [check][e15s]) | complete ([code][ex], [check][e9], [check][e15]) | 23 fixture PDBs (MSVC, .NET); `OldDirectoryPolicy::Preserve` needs the source page size |
| Stream 0, empty | stream 0 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][old], [check][t:old_directory]) | not applicable (no contents) | complete ([code][mw], [check][t:msf_write], [check][e15d]) | complete ([code][mw], [check][t:msf_write], [check][e15d]) | complete ([code][exm], [check][e15]) | 5 fixture PDBs (LLD); the default `OldDirectoryPolicy::Empty` |
| Small MSF (`Microsoft C/C++ program database 2.00`) | — | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `format::probe` identifies the signature |

## PDB Info stream (stream 1)

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `PdbImpvVC2` | `19941610` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Info::parse` returns `UnsupportedVersion` |
| `PdbImpvVC4` | `19950623` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Info::parse` returns `UnsupportedVersion` |
| `PdbImpvVC41` | `19950814` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Info::parse` returns `UnsupportedVersion` |
| `PdbImpvVC50` | `19960307` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Info::parse` returns `UnsupportedVersion` |
| `PdbImpvVC98` | `19970604` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Info::parse` returns `UnsupportedVersion` |
| `PdbImpvVC70Dep` | `19990604` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Info::parse` returns `UnsupportedVersion` |
| `PdbImpvVC70` | `20000404` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][info], [check][t:info], [check][e9]) | complete ([code][info], [code][ht], [check][t:info], [check][t:hash_table], [check][e9]) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e9], [check][e10]) | 28 fixture PDBs; every producer writes this version |
| `PdbImpvVC80` | `20030901` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `Info::parse` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | not applicable (no producer known writes it as the header version) | LLVM lists it; no producer known |
| `PdbImpvVC110` | `20091201` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `Info::parse` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | not applicable (no producer known writes it as the header version) | MSVC writes `VC110` and `VC140` as feature codes, not as the version |
| `PdbImpvVC140` | `20140508` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `Info::parse` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | not applicable (no producer known writes it as the header version) | MSVC writes `VC110` and `VC140` as feature codes, not as the version |
| Identity: signature, age, GUID | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][info], [check][t:info], [check][t:identity], [check][e9]) | complete ([code][reader], [check][t:native]) | missing (#32) | missing (#32: identity policy) | complete ([code][exn], [check][e9]) | `Pdb::debug_info` compares the DBI age with the Info age |
| Info age that differs from the DBI age | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `Pdb::debug_info` returns `InvalidFormat` ("PDB Info and DBI ages differ")) | missing (#32) | missing (#32) | missing (#32, #34) | partial (#32: `msvc-x64-srcsrv/demo.srcsrv.pdb` (`pdbstr` sets the Info age to 2 and keeps the DBI age 1); the reader stops (#27)) | A producer form, not corrupt data |
| Named-stream map (hash table) | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][info], [code][ht], [check][t:hash_table], [check][e9]) | complete ([code][ht], [check][t:hash_table], [check][e9]) | missing (#32) | missing (#32) | complete ([code][exn], [check][e9]) | Slots use `hash_string_v1(name) & 0xffff` with linear probing |
| `niMac` word after the map | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][info], [check][t:info], [check][e9]) | not applicable (a counter) | missing (#32) | missing (#32) | complete ([code][exn], [check][e9]) | LLVM `PdbStream.md` omits it (#9) |
| Feature `PdbImplVC110` | `20091201` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: kept as a raw `u32` code; no typed feature) | not applicable (no references) | missing (#32) | missing (#32) | missing (#27, #32: no fixture) | No fixture |
| Feature `PdbImplVC140` | `20140508` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: kept as a raw `u32` code; no typed feature) | not applicable (no references) | missing (#32) | missing (#32) | complete ([code][exn], [check][e9]) | 28 fixture PDBs |
| Feature `PdbFeatureNoTypeMerge` (`NOTM`) | `0x4d544f4e` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: kept as a raw `u32` code; no typed feature) | not applicable (no references) | missing (#32) | missing (#32) | missing (#27, #32: no fixture) | No fixture |
| Feature `PdbFeatureMinimalDebugInfo` (`MINI`) | `0x494e494d` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][syms], [code][info], [check][t:symbols], [check][e13m]) | not applicable (no references) | missing (#32) | missing (#32) | partial (#32: in `msvc-x64-fastlink`; `llvm-pdbutil` fails on its publics, so lookup checks the producer bucket rule only (#13)) | 1 fixture PDB |
| Unknown feature code | any other value | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][info], [check][t:info]) | not applicable (no references) | missing (#32) | missing (#32) | not applicable (no producer known) | `Info::features` returns every code |

## TPI and IPI streams (streams 2 and 4)

The record kinds are on the [types](types.md) page.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `PdbTpiV40` | `19950410` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `TypeStream::parse` returns `UnsupportedVersion` |
| `PdbTpiV41` | `19951122` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `TypeStream::parse` returns `UnsupportedVersion` |
| `PdbTpiV50` | `19961031` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `TypeStream::parse` returns `UnsupportedVersion` |
| `PdbTpiV70` | `19990903` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `TypeStream::parse` returns `UnsupportedVersion` |
| `PdbTpiV80` | `20040203` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][types], [check][t:types], [check][e2]) | complete ([code][types], [code][tlook], [check][t:types], [check][e2]) | missing (#29) | missing (#29, #34) | complete ([code][ext], [check][e2], [check][e3]) | 28 fixture PDBs (TPI and IPI); the header must be 56 bytes |
| Hash stream: hash values (key size 4) | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][thash], [check][t:type_hash], [check][e3]) | complete ([code][thash], [check][t:type_hash], [check][e3]) | missing (#29) | missing (#29, #34) | complete ([code][ext], [check][e3]) | 27 fixture PDBs; `TypeHashes::verify` recomputes every value |
| Hash stream: index offsets | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][thash], [check][t:type_hash], [check][e3]) | complete ([code][thash], [check][t:type_hash], [check][e3]) | missing (#29) | missing (#29, #34) | complete ([code][ext], [check][e3]) | 27 fixture PDBs |
| Hash stream: hash adjusters | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][thash], [check][t:type_hash], [check][e3]) | complete ([code][thash], [check][t:type_hash], [check][e3]) | missing (#29) | missing (#29, #34) | missing (#27, #29: every fixture hash stream has an empty adjuster buffer) | Keys are `/names` offsets, as in Microsoft `tpi.cpp` and the LLVM code |
| Hash key size other than 4 | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | not applicable (V80 requires 4; `TypeStream::parse` returns `InvalidFormat`) | not applicable (no layout) | not applicable (no layout) | not applicable (no layout) | not applicable (no producer known) |  |
| Auxiliary hash stream (`hash_aux_stream`) | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#29: the stream ID is read; the stream is not decoded) | missing (#29) | missing (#29) | missing (#29, #34) | missing (#27, #29: no fixture names one) |  |
| No hash stream | `0xffff` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][types], [check][t:types]) | not applicable (no references) | missing (#29) | missing (#29, #34) | partial (#29: `dotnet-csharp-full`; no recorded consumer comparison) | The .NET writer leaves out the TPI and IPI hash streams |

## DBI stream (stream 3)

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `PdbDbiVC41` | `930803` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Dbi::parse` returns `UnsupportedVersion` |
| `PdbDbiV50` | `19960307` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Dbi::parse` returns `UnsupportedVersion` |
| `PdbDbiV60` | `19970606` | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `Dbi::parse` returns `UnsupportedVersion` |
| DBI header without the `0xffffffff` signature | — | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | Pre-VC++ 7.0 layout; `Dbi::parse` returns `UnsupportedVersion` |
| `PdbDbiV70` | `19990903` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dbi], [check][t:dbi], [check][e7]) | complete ([code][dbi], [code][reader], [check][t:dbi], [check][t:native]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7], [check][e10]) | 28 fixture PDBs; the substreams must fill the stream exactly |
| `PdbDbiV110` | `20091201` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `Dbi::parse` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | not applicable (no producer known writes it; MSVC writes V70) | LLVM lists it |
| Header flag `fIncLink` (incremental link) | `0x1` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: `DbiHeader::flags` is the raw word) | not applicable (no references) | missing (#32) | missing (#32, #34) | partial (#32: in `msvc-x64-incremental`; no recorded consumer comparison of the bit) | 1 fixture PDB |
| Header flag `fStripped` (private data removed) | `0x2` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: `DbiHeader::flags` is the raw word) | not applicable (no references) | missing (#32) | missing (#32, #34) | partial (#32: in `msvc-x64-stripped/demo.stripped.pdb`; no recorded consumer comparison of the bit) | 1 fixture PDB |
| Header flag `fCTypes` | `0x4` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: `DbiHeader::flags` is the raw word) | not applicable (no references) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| Module info records (`ModInfo`) | substream 1 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dbi], [check][t:dbi], [check][e7]) | complete ([code][dbi], [code][reader], [check][t:dbi], [check][e10]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7], [check][e10]) | Module stream IDs and region sizes are checked against the MSF |
| Section contributions `DBISCImpv` V40 (headerless, 20-byte records) | none | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dsc], [check][t:dbi_tables]) | complete ([code][dsc], [code][exd], [check][t:dbi_tables]) | not applicable (legacy, #16) | not applicable (legacy, #16) | missing (#16: no fixture) | Legacy layout that pdbrs reads, as Microsoft `dbi.cpp` does; a first word that is not V60 or V2 selects it; LLVM rejects it |
| Section contributions `DBISCImpv` V60 | `0xeffe0000 + 19970605` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dsc], [check][t:dbi_tables], [check][e7]) | complete ([code][exd], [check][e7], [check][e10]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7]) | 27 fixture PDBs |
| Section contributions `DBISCImpv2` V2 | `0xeffe0000 + 20140516` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dsc], [check][t:dbi_tables]) | complete ([code][exd], [check][t:dbi_tables]) | missing (#32) | missing (#32, #34) | partial (#32: in `msvc-x64-fastlink`; no recorded consumer comparison) | 1 fixture PDB; adds the COFF section index |
| Section map | substream 3 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dmap], [check][t:dbi_tables], [check][e7]) | complete ([code][exd], [check][e7]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7]) | 28 fixture PDBs |
| Source file information | substream 4 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dsrc], [check][t:dbi_tables], [check][e7]) | complete ([code][dsrc], [check][t:dbi_tables], [check][e7]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7]) | 28 fixture PDBs; the layout comes from the summed counts, so 16-bit overflow works |
| Type server map, empty | substream 5 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dbi], [check][t:dbi_tables]) | not applicable (no contents) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7]) | 28 fixture PDBs |
| Type server map, not empty | substream 5 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `Dbi::verify_type_server_map` returns `UnsupportedVersion`) | missing (#32: external type server reference) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture; `msvc-x64-pch-zi` keeps `LF_TYPESERVER2` in the objects only) | An external dependency: pdbrs does not load type servers |
| EC names (`/names` layout) | substream 6 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dbi], [code][names], [check][t:dbi_tables], [check][e7]) | complete ([code][names], [check][t:names], [check][e7]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7]) | 28 fixture PDBs; hash version 1 in all |
| Optional debug header | substream 7 | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dbi], [check][t:dbi], [check][e7]) | complete ([code][reader], [check][t:native]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7], [check][e13m]) | 11 slots (5 fixture PDBs, LLD) or 12 slots (23 fixture PDBs, MSVC and .NET); `Dbi::debug_streams` also returns slot 11 |

## Optional debug streams

The optional debug header holds one stream index for each slot. The slot order is `DbgHeaderType` of LLVM `RawConstants.h`.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Slot 0: FPO (`FPO_DATA`, 16-byte records) | `0` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][fpo], [check][t:fpo], [check][e13m]) | not applicable (no references) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e13m]) | 3 fixture PDBs (x86); `fHasSEH` is bit 11 as in `winnt.h`, not bit 9 as in LLVM |
| Slot 1: Exception | `1` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| Slot 2: Fixup | `2` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32) | missing (#32) | missing (#32) | missing (#32, #34) | partial (#32: in `msvc-x64-debugtype` (`/DEBUGTYPE:CV,FIXUP,PDATA`); no decoder, so no comparison) | 1 fixture PDB |
| Slot 3: OMAP to source | `3` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture; `AddressMap` returns `UnsupportedVersion` when an OMAP slot is present; no local producer makes OMAP |
| Slot 4: OMAP from source | `4` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture; `AddressMap` returns `UnsupportedVersion` when an OMAP slot is present; no local producer makes OMAP |
| Slot 5: Section headers | `5` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][sec], [check][t:section], [check][e7]) | complete ([code][addr], [check][t:section], [check][e7]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e7]) | 28 fixture PDBs |
| Slot 6: Token to RID map | `6` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture; C++/CLI with MSVC 14.51 (`msvc-x64-clr`) writes none (#27) |
| Slot 7: `.xdata` | `7` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32) | missing (#32) | missing (#32) | missing (#32, #34) | partial (#32: in `msvc-x64-debugtype` (`/DEBUGTYPE:CV,FIXUP,PDATA`); no decoder, so no comparison) | 1 fixture PDB |
| Slot 8: `.pdata` | `8` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32) | missing (#32) | missing (#32) | missing (#32, #34) | partial (#32: in `msvc-x64-debugtype` (`/DEBUGTYPE:CV,FIXUP,PDATA`); no decoder, so no comparison) | 1 fixture PDB |
| Slot 9: New FPO (`FRAMEDATA`, 32-byte records) | `9` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][fpo], [check][t:fpo], [check][e13m]) | complete ([code][fpo], [code][exd], [check][t:fpo], [check][e13m]) | missing (#32) | missing (#32, #34) | complete ([code][exd], [check][e13m]) | 3 fixture PDBs (x86); frame programs are joined to `/names` |
| Slot 10: Original section headers | `10` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: `SectionHeaders::parse` reads the layout; no OMAP translation uses it) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| Slot 11 (no name) | `11` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][dbi], [check][t:dbi]) | not applicable (nil in all fixtures) | missing (#32) | missing (#32, #34) | not applicable (nil (`0xffff`) in every MSVC and .NET fixture) | `cvinfo.h`, LLVM and Microsoft `dbi.h` name only slots 0 to 10 |

## Address translation

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Section offset to RVA and RVA to section offset | — | not applicable (a computation, not stored data) | complete ([code][addr], [check][t:section], [check][e7]) | complete ([code][addr], [check][t:section], [check][e10]) | not applicable (a computation) | not applicable (a computation) | complete ([code][exd], [check][e7], [check][e10]) | The end of a section translates (incremental links put records there) |
| OMAP translation (both directions) | — | not applicable (a computation, not stored data) | missing (#32: `AddressMap` returns `UnsupportedVersion`) | missing (#32) | not applicable (a computation) | not applicable (a computation) | missing (#32: no local producer makes OMAP streams (`external-input`)) | Unmapped ranges need an explicit result |

## `/names` string table

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Header signature | `0xeffeeffe` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][names], [check][t:names], [check][e7]) | complete ([code][names], [check][t:names]) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e7]) | 27 fixture PDBs |
| Hash version 1 (`LHashPbCb`) | `1` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][names], [check][t:names], [check][e7]) | complete ([code][names], [check][t:names], [check][e7]) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e7]) | 27 fixture PDBs; `verify_buckets` checks the placement of every name |
| Hash version 2 (`LHashPbCbV2`) | `2` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: names and offsets read; `find` and `verify_buckets` return `UnsupportedVersion`) | partial (#32: offsets checked; buckets not checked) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| Other hash versions | any other value | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | not applicable (`StringTable::parse` returns `UnsupportedVersion`) | not applicable (no layout) | not applicable (no layout) | not applicable (no layout) | not applicable (no producer known) |  |
| No `/names` stream (stripped PDB) | — | not applicable (no stream) | missing (#32: the `native` example stops with "missing /names") | missing (#32) | missing (#32) | missing (#32, #34) | partial (#32: `msvc-x64-stripped/demo.stripped.pdb` (#27); the reader stops) | `/PDBSTRIPPED` removes `/names`, the module streams and private symbols |

## Global and public symbol indexes (GSI and PSI)

The symbol records are on the [symbols](symbols.md) page.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Hash header `GSIHashSCImpvV70` (compressed buckets) | signature `0xffffffff`, version `0xf12f091a` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][stab], [code][gsi], [check][t:symbols], [check][e8]) | complete ([code][stab], [check][t:symbols], [check][e8]) | missing (#30) | missing (#30, #34) | complete ([code][exs], [check][e8], [check][e13m]) | 28 fixture PDBs (GSI and PSI) |
| Hash layout before `GSIHashSCImpvV70` (no header, uncompressed buckets) | — | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | not applicable (legacy, #16) | `GlobalSymbolIndex::parse` returns `UnsupportedVersion` |
| Standard bucket count | `4096` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][syms], [check][t:symbols], [check][e8]) | complete ([code][stab], [check][t:symbols], [check][e8]) | missing (#30) | missing (#30, #34) | complete ([code][exs], [check][e8]) | 27 fixture PDBs |
| Minimal bucket count (`MINI` feature) | `0x3ffff` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][syms], [check][t:symbols], [check][e13m]) | complete ([code][stab], [check][t:symbols], [check][e13m]) | missing (#30) | missing (#30, #34) | partial (#30: `msvc-x64-fastlink`; LLVM fails on its publics, and `ms-pdb` 0.1.22 does not truncate the bucket (#13)) | The producer bucket is `(hash % count) & 0xffff` |
| PSI header (`PSGSIHDR`) | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][psi], [check][t:symbols], [check][e8]) | complete ([code][psi], [check][t:symbols], [check][e8]) | missing (#30) | missing (#30, #34) | complete ([code][exs], [check][e8]) | 28 fixture PDBs |
| PSI address map | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][psi], [check][t:symbols], [check][e8]) | complete ([code][psi], [code][exs], [check][t:symbols], [check][e8]) | missing (#30) | missing (#30, #34) | complete ([code][exs], [check][e8]) | Sorted by section and offset |
| PSI thunk map | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][psi], [check][t:symbols], [check][e8]) | complete ([code][exs], [check][e13s]) | missing (#30) | missing (#30, #34) | complete ([code][exs], [check][e13s]) | 1 fixture PDB (`msvc-x64-incremental`): 532 thunks join 532 `S_TRAMPOLINE` records |
| PSI section starts | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][psi], [check][t:symbols], [check][e8]) | complete ([code][exs], [check][e13s]) | missing (#30) | missing (#30, #34) | complete ([code][exs], [check][e13s]) | Read only if bytes follow the thunk map, as LLVM does |

## Named and auxiliary streams

`NamedStreamKind::classify` selects the decoder by the exact name. An unrecognized name is `NamedStreamKind::Other`, and its stream keeps its bytes.

| Kind | Value | Raw preservation | Semantic reading | Reference validation | Writing | Editing and index rebuilding | Independent fixture evidence | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `/LinkInfo`, empty | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][link], [check][t:auxiliary]) | not applicable (no contents) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e9]) | 27 fixture PDBs |
| `/LinkInfo` version 1 | `1` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `LinkInfo::parse` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| `/LinkInfo` version 2 | `2` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][link], [check][t:auxiliary], [check][e9]) | not applicable (no references) | missing (#32) | missing (#32, #34) | partial (#32: no consumer prints it) | 1 fixture PDB; libraries and trailing bytes stay raw |
| `/TMCache`, empty | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][tmc], [check][t:tm_cache]) | not applicable (no contents) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e9]) | 22 fixture PDBs |
| `/TMCache` version `0x20200229` | `0x20200229` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][tmc], [check][t:tm_cache], [check][e13t]) | complete ([code][tmc], [code][exn], [check][t:tm_cache], [check][e13m]) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e13m]) | 1 fixture PDB; layout from `DBI1::DumpTMCache` of `mspdbcore.dll` (#13); the entry checksums cannot be checked |
| Type map stream version 1 (type server map) | `1` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: sizes checked; map values stay raw) | partial (#32: values checked against the larger index end) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e13m]) | Unnamed stream that `/TMCache` names |
| Type map stream version 2 (object map) | `2` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: sizes checked; map values stay raw) | partial (#32: values checked against the larger index end) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e13m]) | 41 streams in `msvc-x64-incremental`; one more copy has no owner (#13) |
| `/UDTSRCLINEUNDONE` | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][udtu], [check][t:auxiliary], [check][e9]) | missing (#32: the type indices are not checked) | missing (#32) | missing (#32, #34) | partial (#32: no reference defines the stream; the layout comes from the fixtures (#9)) | 22 fixture PDBs, 2 not empty |
| `/src/headerblock` version `19980827` | `19980827` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][inj], [check][t:injected_source], [check][e9]) | complete ([code][inj], [check][t:injected_source], [check][e9]) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e9]) | 20 fixture PDBs; an empty stream means no injected sources |
| `/src/files/*`, compression none | `0` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][inj], [check][t:injected_source], [check][e9]) | complete ([code][inj], [check][t:injected_source], [check][e9]) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e9]) | 5 fixture PDBs (Natvis files of Rust and `msvc-x64-natvis`); size and CRC are checked |
| `/src/files/*`, compression run-length encoding | `1` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `verify_content` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| `/src/files/*`, compression Huffman | `2` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `verify_content` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| `/src/files/*`, compression LZ | `3` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: `verify_content` returns `UnsupportedVersion`) | missing (#32) | missing (#32) | missing (#32, #34) | missing (#27, #32: no fixture) | No fixture |
| `/src/files/*`, .NET document record | `101` | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | complete ([code][doc], [check][t:dotnet_documents], [check][e14]) | complete ([code][doc], [check][t:dotnet_documents], [check][e14]) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e14]) | 1 fixture PDB; CRC, size and SHA-256 are checked |
| Natvis file in injected sources | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: the file bytes are checked; the XML is not decoded) | not applicable (no references) | missing (#32) | missing (#32, #34) | partial (#32: Rust and `msvc-x64-natvis` (`/NATVIS`); the XML is not compared) | rustc and `link.exe /NATVIS` embed Natvis files |
| `sourcelink` | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | partial (#32: UTF-8 and outer JSON object checked; the JSON is kept raw) | not applicable (URLs are not fetched) | missing (#32) | missing (#32, #34) | complete ([code][exn], [check][e14]) | 1 fixture PDB (.NET) |
| `srcsrv` | — | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | missing (#32: classified as `Other`) | missing (#32) | missing (#32) | missing (#32, #34) | partial (#32: `msvc-x64-srcsrv/demo.srcsrv.pdb` (`pdbstr`, #27); the reader stops at the age check) | Text stream that `pdbstr -w -s:srcsrv` writes |
| Other named stream | any other name | complete ([code][msf], [code][reader], [check][t:msf], [check][e15]) | not applicable (no layout; `NamedStreamKind::Other` keeps the bytes) | not applicable (no layout) | missing (#32: raw copy) | missing (#32, #34) | not applicable (no producer known) | The `native` example reports it |

[addr]: ../../../crates/pdbrs/src/native/address.rs
[dbi]: ../../../crates/pdbrs/src/native/dbi.rs
[dmap]: ../../../crates/pdbrs/src/native/dbi/section_map.rs
[doc]: ../../../crates/pdbrs/src/native/injected_source/document.rs
[dsc]: ../../../crates/pdbrs/src/native/dbi/section_contribution.rs
[dsrc]: ../../../crates/pdbrs/src/native/dbi/sources.rs
[e10]: https://github.com/napbat/pdbrs/issues/10#issuecomment-6407894516
[e13m]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409858972
[e13s]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409780019
[e13t]: https://github.com/napbat/pdbrs/issues/13#issuecomment-6409615221
[e14]: https://github.com/napbat/pdbrs/issues/14#issuecomment-6409977445
[e15]: https://github.com/napbat/pdbrs/issues/15#issuecomment-6410580545
[e15d]: https://github.com/napbat/pdbrs/issues/15#issuecomment-6410087219
[e15s]: https://github.com/napbat/pdbrs/issues/15#issuecomment-6410251692
[e2]: https://github.com/napbat/pdbrs/issues/2#issuecomment-6407739492
[e27]: https://github.com/napbat/pdbrs/issues/27#issuecomment-6412654019
[e3]: https://github.com/napbat/pdbrs/issues/3#issuecomment-6407739737
[e7]: https://github.com/napbat/pdbrs/issues/7#issuecomment-6407578735
[e8]: https://github.com/napbat/pdbrs/issues/8#issuecomment-6407490702
[e9]: https://github.com/napbat/pdbrs/issues/9#issuecomment-6407615021
[ex]: ../../../crates/pdbrs/examples/native/main.rs
[exd]: ../../../crates/pdbrs/examples/native/dbi.rs
[exm]: ../../../crates/pdbrs/examples/msf.rs
[exn]: ../../../crates/pdbrs/examples/native/named.rs
[exs]: ../../../crates/pdbrs/examples/native/symbols.rs
[ext]: ../../../crates/pdbrs/examples/native/types.rs
[fpm]: ../../../crates/pdbrs/src/native/msf/free_page_map.rs
[fpo]: ../../../crates/pdbrs/src/native/fpo.rs
[gsi]: ../../../crates/pdbrs/src/native/symbols/gsi.rs
[ht]: ../../../crates/pdbrs/src/native/hash_table.rs
[info]: ../../../crates/pdbrs/src/native/info.rs
[inj]: ../../../crates/pdbrs/src/native/injected_source.rs
[link]: ../../../crates/pdbrs/src/native/auxiliary/link_info.rs
[mdir]: ../../../crates/pdbrs/src/native/msf/directory.rs
[msf]: ../../../crates/pdbrs/src/native/msf.rs
[mw]: ../../../crates/pdbrs/src/native/msf/write.rs
[mwl]: ../../../crates/pdbrs/src/native/msf/write/layout.rs
[names]: ../../../crates/pdbrs/src/native/names.rs
[old]: ../../../crates/pdbrs/src/native/msf/old_directory.rs
[psi]: ../../../crates/pdbrs/src/native/symbols/psi.rs
[reader]: ../../../crates/pdbrs/src/native/reader.rs
[sec]: ../../../crates/pdbrs/src/native/section.rs
[stab]: ../../../crates/pdbrs/src/native/symbols/table.rs
[syms]: ../../../crates/pdbrs/src/native/symbols.rs
[t:auxiliary]: ../../../crates/pdbrs/tests/auxiliary.rs
[t:dbi]: ../../../crates/pdbrs/tests/dbi.rs
[t:dbi_tables]: ../../../crates/pdbrs/tests/dbi_tables.rs
[t:dotnet_documents]: ../../../crates/pdbrs/tests/dotnet_documents.rs
[t:fpo]: ../../../crates/pdbrs/tests/fpo.rs
[t:free_page_map]: ../../../crates/pdbrs/tests/free_page_map.rs
[t:hash_table]: ../../../crates/pdbrs/tests/hash_table.rs
[t:identity]: ../../../crates/pdbrs/tests/identity.rs
[t:info]: ../../../crates/pdbrs/tests/info.rs
[t:injected_source]: ../../../crates/pdbrs/tests/injected_source.rs
[t:msf]: ../../../crates/pdbrs/tests/msf.rs
[t:msf_write]: ../../../crates/pdbrs/tests/msf_write.rs
[t:names]: ../../../crates/pdbrs/tests/names.rs
[t:native]: ../../../crates/pdbrs/tests/native.rs
[t:old_directory]: ../../../crates/pdbrs/tests/old_directory.rs
[t:section]: ../../../crates/pdbrs/tests/section.rs
[t:symbols]: ../../../crates/pdbrs/tests/symbols.rs
[t:tm_cache]: ../../../crates/pdbrs/tests/tm_cache.rs
[t:type_hash]: ../../../crates/pdbrs/tests/type_hash.rs
[t:types]: ../../../crates/pdbrs/tests/types.rs
[thash]: ../../../crates/pdbrs/src/native/types/hash.rs
[tlook]: ../../../crates/pdbrs/src/native/types/lookup.rs
[tmc]: ../../../crates/pdbrs/src/native/auxiliary/tm_cache.rs
[types]: ../../../crates/pdbrs/src/native/types.rs
[udtu]: ../../../crates/pdbrs/src/native/auxiliary/udt_source_line_undone.rs
