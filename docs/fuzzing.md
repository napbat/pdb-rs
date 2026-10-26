# Fuzzing

This document records the fuzz targets of `pdbrs`, the commands, the limits and the recorded
runs (#35). The targets are in `crates/pdbrs/fuzz/`. `crates/pdbrs/fuzz/README.md` tells why the
package is outside the stable workspace and how to install the tools.

## Targets

Each target calls one function of the `pdbrs-fuzz` library. The library uses only the public API
of `pdbrs`. A reader target ignores errors, because an error is a valid result for malformed
input. A panic, a failed assertion, a sanitizer report, a hang or memory use above the limit is a
finding. A round-trip target checks self-consistency only: the output of an encoder must decode
to the same value. It does not check that other readers accept the output.

| Target | Input | Public API under test |
| --- | --- | --- |
| `msf` | MSF 7.0 file | `format::probe`, `Msf::open`, `Msf::with_limits`, each stream, `check_free_page_map`, `OldDirectory` |
| `msf_repack` | selector byte, MSF file | `Msf::repack` round trip: page size and stream 0 policy, written streams, free page map |
| `msf_build` | page size byte, tagged streams | `MsfBuilder` round trip: `plan`, `to_vec`, `Msf::open`, stream bytes |
| `msfz` | MSFZ file | `format::probe`, `Msfz::with_limits`, `Container::with_limits`, each stream twice (`read_all` and three windows), small directory limit |
| `msfz_chunks` | selector byte, two declared chunk sizes, fragment offset, stored chunk bytes | an MSFZ container around one or two chunks (stored, zstd or raw DEFLATE) and one fragment across them; reads with and without the chunk cache must agree; stored chunks must give their bytes |
| `native_pdb` | native PDB file | `native::Pdb` and every stream that it names: Info, named streams, TPI, IPI, hash streams, DBI, FPO, frame data, modules, symbol records, GSI, PSI |
| `native_info` | Info stream | `Info::parse`, named stream lookup, feature codes |
| `native_hash_table` | hash table | `HashTable::<u32>::parse`, `check_placement` |
| `native_names` | `/names` stream | `StringTable::parse`, `get`, `find`, `verify_buckets` |
| `native_types` | hash part, TPI stream | `TypeStream`, `TypeHashes`, `TypeTable`, `ForwardRefs`, `TypeLeaf`, field lists, `type_record_hash` |
| `native_ids` | hash part, `/names` part, TPI end, IPI stream | `IdTable`: `decode`, `validate`, `resolve_string` |
| `native_leaf` | record kind, payload | `TypeLeaf`, `IdRecord`, `Symbol`, `FieldList`, `Integer::parse_prefix` |
| `native_dbi` | section header part, DBI stream | `Dbi` substreams, `SectionHeaders`, `AddressMap` |
| `native_frames` | stream | `FpoStream`, `FrameDataStream`, `SectionHeaders` |
| `native_module` | region sizes, module stream | `ModuleStream`, `ModuleLines`, `ModuleSymbols`, C13 subsections |
| `native_lines` | subsection kind, data | `LineSubsection::decode` and its entries |
| `native_symbols` | symbol records | `Records`, `Symbol`, `RecordOffsets`, `ModuleSymbols`, symbol attributes |
| `native_indexes` | bucket selector, GSI part, PSI part, records | `GlobalSymbolIndex`, `PublicSymbolIndex`, lookup, record and address checks |
| `native_named` | kind selector, stream | `LinkInfo`, `TmCache`, `TypeMapStream`, `UdtSourceLineUndone`, `SourceLink`, `SourceHeaderBlock`, `DotNetDocument`, `OldDirectory` |
| `portable_pdb` | Portable PDB or `MPDB` payload | `portable::Pdb`: rows, heaps, documents, sequence points, scopes, imports, embedded sources, `validate_debug` |
| `portable_metadata` | file | `MetadataRoot::read`, `format::probe`, `pdbrs::Pdb::parse` |
| `portable_blobs` | kind selector, blob | round trips of `DocumentName`, `SequencePoints`, `LocalConstantSignature`, `Imports`, compressed integers; `split_document_name` |
| `custom_debug_info` | kind selector, value | round trip of each `CustomDebugValue` kind; native `MD2`, `asyncMethodInfo` and symbol attributes |
| `mpdb` | payload | `decode_embedded` and `encode_embedded` round trips |
| `embedded_source` | blob | `EmbeddedSource::parse`, `decode` and `encode` round trips in both formats |
| `portable_rebuild` | Portable PDB | `copy_exact`, `to_parts`, `PdbParts::plan`, `to_embedded`, `Pdb::edit` round trips |
| `portable_build` | builder commands | `PdbBuilder` with typed records: `plan`, `to_vec`, read back documents and sequence points |
| `native_type_encode` | as `native_types` | `TypeLeaf::encode`, `encode_field_list`, `write_type_stream`, `TypeHashes::verify` of the output |
| `native_id_encode` | as `native_ids` | `IdRecord::encode`, `write_type_stream` |
| `native_leaf_encode` | as `native_leaf` | `TypeLeaf`, `IdRecord`, `Symbol` and `Numeric` encoders on one record |
| `native_symbol_encode` | as `native_symbols` | `Symbol::encode`, `SymbolRecordWriter`, `ModuleSymbolWriter` |
| `native_index_encode` | as `native_indexes` (bit 1 selects LLVM) | `GlobalSymbolIndex::build`, `PublicSymbolIndex::build` |
| `native_lines_encode` | as `native_lines` | `LineSubsection::encode`, `encode_framed` |
| `native_module_encode` | as `native_module` | `ModuleStream::encode`, `ModuleLines::encode`, `ModuleStreamParts::encode` |
| `native_info_encode` | as `native_info` | `Info::encode`, `InfoBuilder::from_info` and `encode` |
| `native_names_encode` | as `native_names` | `StringTable::encode`, `StringTableBuilder` |
| `native_dbi_encode` | as `native_dbi` | `Dbi::encode`, `encode_substream`, module records, `OwnedModule`, `DbiBuilder::from_dbi` |
| `native_debug_stream_encode` | kind selector, stream | FPO, frame data, fixup, function entry, token map, section header, OMAP and `RvaVaBlob` encoders |
| `native_named_encode` | kind selector, stream | `LinkInfo`, `TmCache` and `TmCacheBuilder`, `TypeMapStream`, `UdtSourceLineUndone`, `SourceLink`, `SourceServer`, `SourceHeaderBlock` and its builder, `DotNetDocument` |

The MSFZ targets (#37) use 16 MiB chunks and a 32 MiB chunk cache (`pdbrs_fuzz::msfz::limits`);
`native_pdb` uses the same MSFZ limits. `msfz` does not compare reads without the cache: for
a whole PDZ file, each uncached read decompresses a chunk again and takes too long.

### Round-trip properties of the native encoders

For each decoded value, the encoder must succeed, and the decoder must give the same value
from the output. Where the documentation of a codec says that a decoded value gives its
input bytes again, the target also compares the bytes. These rules apply:

- A record offset is a location, not a value. The targets decode records at offset 0
  before they compare values.
- `FieldList` and `MethodList` values keep their raw bytes. The encoders write the standard
  `LF_PAD` bytes between field-list members, so the targets compare the decoded members and
  entries of these lists, and not their bytes.
- `TypeLeaf::encode` and `IdRecord::encode` give the input bytes for a record "of a stream
  that pads its records". The targets compare the bytes only for a record with a length that
  is a multiple of 4.
- `write_type_stream` and the GSI and PSI builders can reject decoded contents, for example
  an LLVM layout with hash adjusters. If they accept the contents, the output must parse
  with the same records, buckets, thunks and section starts.

## Seed corpora

The `seeds` example of the fuzz package writes the corpora to `crates/pdbrs/fuzz/corpus/`.
It builds these inputs with the public API of `pdbrs`:

- A synthetic native PDB with streams 0 to 12: Info with a named stream map, TPI with a hash
  stream, DBI with two modules and each substream, IPI, `/names`, GSI, PSI, symbol records,
  one module stream with a procedure and C13 lines, section headers and `sourcelink`.
- A synthetic Portable PDB from `PdbBuilder`: two documents with embedded sources, methods
  with sequence points, scopes, variables, constants, import scopes, a state machine method and
  one value of each custom debug information kind.
- Small inputs: MSF containers, builder commands, FPO and frame data records, compressed
  integers and embedded-source blobs.
- MSFZ inputs: chunks with a zstd frame of one raw block, a raw DEFLATE stored block and stored
  bytes, and the containers around them. A PDZ fixture (MSFZ signature) gives its file to
  `msfz` and `native_pdb` and each chunk of its Chunk Table to `msfz_chunks`.

The generator splits each PDB into the inputs of the targets: streams, records, subsections,
blobs and values. It caps the seeds of one PDB to 48 for each target.

A fixture path argument adds the `.pdb` and `.mpdb` files below it in the same way. The
recorded runs used the local fixtures of `target/fixtures/` in the primary checkout (58 files,
with the native fixture variants of #27). The fixtures and the corpora are local inputs: do not
commit them. The compiler PDBs (`obj/compile.pdb`) have no DBI stream, so the generator prints
"skipped" for them after it adds their other streams.

## Commands

Run the commands in `crates/pdbrs/fuzz/` with the nightly toolchain.

```sh
cargo +nightly run --release --example seeds -- C:/Users/dev/Documents/Git/napbat/pdbrs/target/fixtures
cargo +nightly fuzz build
cargo +nightly fuzz run <target> -- -max_total_time=900 -rss_limit_mb=2048 -timeout=10 -print_final_stats=1
```

To run a crash input again:

```sh
cargo +nightly fuzz run <target> artifacts/<target>/crash-<hash>
```

## Limits

- `-rss_limit_mb=2048`: a run that uses more than 2 GiB of memory is a finding.
- `-timeout=10`: an input that takes more than 10 seconds is a finding.
- `-max_len`: not set. libFuzzer takes the largest seed as the limit, so the native PDB
  targets use inputs of up to 1 MiB.
- The targets read at most 16 MiB of a stream or a decoded payload (`MAX_STREAM`), and visit
  at most 65 536 records or 16 384 rows in one walk. The round trips decode at most 1 MiB of
  an `MPDB` payload or an embedded source.

## Environment

- AMD Ryzen 9 9950X3D (16 cores), 62 GiB of memory, Windows 11 (`win32 10.0.26200`).
- `rustc 1.101.0-nightly (8d1a76430 2026-10-06)`, `cargo-fuzz 0.13.2`, `libfuzzer-sys` 0.4.13.
- AddressSanitizer of MSVC 14.51 (`clang_rt.asan_dynamic-x86_64.dll`), the default
  `cargo fuzz` sanitizer. LeakSanitizer is not available on Windows.
- Up to 13 targets ran at the same time, with other builds on the same computer. The
  execution rates are not benchmarks.

### Panic inputs on Windows

The panic hook of `libfuzzer-sys` calls `std::process::abort`. On Windows, `abort` ends the
process with `__fastfail` (`0xc0000409`), so libFuzzer does not write the input. Each target
installs `pdbrs_fuzz::exit_on_panic` in its `init` block. The hook prints the panic, writes the
input to `artifacts/<target>/crash-<hash>` and exits with code 101.

## Recorded runs

Date: 2026-10-26. Branch `fuzzing`, based on `main` 8b77fb2. The table gives the sum of the
runs without a finding. `cov` and `corpus` are from the last run of the target.

| Target | Time (s) | Executions | cov | corpus | Peak RSS (MiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| `custom_debug_info` | 901 | 7,981,911 | 1750 | 1145 | 607 |
| `embedded_source` | 901 | 539,034 | 1168 | 642 | 665 |
| `mpdb` | 901 | 117,762 | 1444 | 510 | 438 |
| `msf` | 901 | 1,076,116 | 792 | 125 | 504 |
| `msf_build` | 901 | 2,291,779 | 479 | 87 | 638 |
| `msf_repack` | 901 | 1,149,522 | 935 | 106 | 508 |
| `native_dbi` | 901 | 599,345 | 546 | 209 | 537 |
| `native_frames` | 901 | 9,920,109 | 191 | 71 | 1028 |
| `native_hash_table` | 901 | 35,944,021 | 174 | 76 | 551 |
| `native_ids` | 901 | 564,856 | 829 | 381 | 532 |
| `native_indexes` | 901 | 267,258 | 1251 | 538 | 540 |
| `native_info` | 901 | 20,886,093 | 520 | 273 | 598 |
| `native_leaf` | 901 | 13,543,512 | 2737 | 1925 | 707 |
| `native_lines` | 901 | 38,205,058 | 318 | 129 | 661 |
| `native_module` | 2702 | 3,821,482 | 2836 | 1038 | 658 |
| `native_named` | 901 | 11,978,252 | 1367 | 739 | 686 |
| `native_names` | 901 | 2,982,646 | 206 | 159 | 452 |
| `native_pdb` | 2702 | 234,733 | 6632 | 1101 | 1025 |
| `native_symbols` | 2702 | 4,234,863 | 2839 | 2233 | 618 |
| `native_types` | 2702 | 254,092 | 2166 | 466 | 541 |
| `portable_blobs` | 902 | 11,678,551 | 1416 | 1043 | 677 |
| `portable_build` | 2702 | 1,939,240 | 4589 | 2132 | 683 |
| `portable_metadata` | 901 | 10,681,094 | 724 | 364 | 583 |
| `portable_pdb` | 902 | 321,390 | 4943 | 1798 | 718 |
| `portable_rebuild` | 901 | 73,989 | 8179 | 1428 | 587 |
| `native_type_encode` | 901 | 140,202 | 2035 | 393 | 598 |
| `native_id_encode` | 901 | 389,372 | 958 | 200 | 550 |
| `native_leaf_encode` | 901 | 8,065,652 | 7496 | 2949 | 606 |
| `native_symbol_encode` | 901 | 218,384 | 3707 | 1324 | 674 |
| `native_index_encode` | 901 | 545,886 | 2285 | 569 | 572 |
| `native_lines_encode` | 901 | 27,330,779 | 762 | 358 | 960 |
| `native_module_encode` | 901 | 8,731,834 | 1129 | 288 | 616 |
| `native_info_encode` | 901 | 28,852,926 | 634 | 156 | 576 |
| `native_names_encode` | 901 | 10,484,756 | 218 | 53 | 452 |
| `native_dbi_encode` | 901 | 1,030,786 | 1171 | 263 | 615 |
| `native_debug_stream_encode` | 901 | 50,452,815 | 369 | 88 | 499 |
| `native_named_encode` | 901 | 55,190,434 | 1058 | 565 | 959 |
| `msfz` | 902 | 47,554 | 1697 | 496 | 659 |
| `msfz_chunks` | 901 | 43,043 | 1835 | 561 | 837 |

The MSFZ targets ran on branch `msfz-reader` (#37) on 2026-10-26, with seeds from the
synthetic inputs and the 52 PDZ fixtures of the native fixtures of `main` 8dc001c (`msfz`: 56
seeds, `msfz_chunks`: 631 seeds). The `msfz` row is the run after the fix of finding 4. The
`msfz_chunks` run used the library before that fix, which does not change its inputs.

The 12 encoder targets ran on the branch with `main` aa552a7 (2026-10-26). Their seeds are
the seeds of the reader targets with the same input layout.

No run reported a sanitizer error, a timeout or memory use above the limit.

After the merge of `main` 1f8fd09 into `fuzzing`, each reader target ran once over its whole
corpus (`-runs=0`) without a finding. These replays are not new fuzzing time. After the last
build, each crash input in `artifacts/` runs without a finding.

## Findings

1. `portable_blobs`, `portable_pdb` and `portable_rebuild`: the sequence point round trip
   failed for the header `0x11000000`, a `StandAloneSig` token with the nil row.
   `SequencePoints::local_signature_form` gave the token form, but `encode_sequence_points`
   writes a nil row as 0 in both forms, so the output decoded with the row form. The fix gives
   the row form for each nil row, as the documentation says. Regression test:
   `a_nil_local_signature_token_has_the_row_form` in `crates/pdbrs/tests/portable_debug/sequence.rs`.
   After the fix, the three targets ran for the times in the table without a finding.
2. `native_leaf_encode`: `Symbol::encode` added alignment padding to a decoded record with a
   length that is not a multiple of 4, so the output was not the input record. Regression
   test: `keeps_the_length_of_an_unaligned_record` in `crates/pdbrs/tests/codeview.rs`.
3. `native_symbol_encode`: a decoded record with the length field `0xffff` did not encode,
   because the alignment padding made it too long (`LimitExceeded`). Regression test:
   `encodes_a_record_of_the_largest_length_without_padding` in `crates/pdbrs/tests/codeview.rs`.
   The fix of findings 2 and 3: `Symbol::encode` writes the fields without alignment padding
   and then the zero bytes of the original record.
4. `msfz`: a timeout. A real PDZ file with one changed byte in a zstd chunk took 45 s: each
   read of a fragment in the corrupt chunk decoded the chunk again, because the chunk cache
   keeps only decoded chunks. The fix: the cache keeps the error kind of a chunk that does not
   decode, so the reader decodes each chunk at most once. Regression test:
   `a_corrupt_chunk_is_decoded_once` in `crates/pdbrs/src/native/msfz/tests/reads.rs`. The
   first `msfz` runs also compared reads without the cache for whole PDZ files; that part of
   the target took more than 10 s for the seeds, and the target now leaves it to `msfz_chunks`.

The other first failures of the encoder targets came from the target code, not from the
library: a comparison of record offsets, of the raw bytes of field lists and method lists,
and of bytes of unpadded type records (see the round-trip properties above).
