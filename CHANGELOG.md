# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each entry names the evidence behind it: a knowledge-graph (KG) query (run with
`python3 tools/kg/ask.py` up to 0.1.0, with `rmp graph client` since), a test file, or
an `rmp` task or sprint of the `rucene` roadmap.

## [Unreleased]

### Changed

- Knowledge Graph synchronisation: `tools/kgsync` (Rust) replaces the `tools/kg`
  Python chain, which is removed. Each layer derives the desired state of the slices it
  owns (the `syn` AST of the crate, the javac AST of every reference module, the port
  claims in the doc comments, `git` and `rmp`). It reads the current state concurrently
  with the parse, writes only the difference, and skips a layer whose inputs are
  unchanged. Measured on 16 cores: 0.15 s with no change, about 1.8 s for a commit
  that changes Rust sources, 4–7 s for a full verification. Every identity now carries a
  `UNIQUE` constraint, and Rust types are keyed by `qualifiedName`
  (`tools/kgsync/README.md`, `knowledge-model.md`).

## [0.1.0] - 2026-11-21

First release. It covers the whole history of the `develop` branch up to
`83d13c9`: 583 commits, 16 sprints, 565 completed `rmp` tasks.

Release notes: [`docs/releases/0.1.0.md`](docs/releases/0.1.0.md).

### Source of the coverage figures

The coverage figures below are **structural**: they count the Lucene types and
members that have a Rust counterpart, as the KG records them (`ask.py coverage`,
`ask.py absent`). They do not, on their own, prove behavioural equality; the
tests listed under each section are the behavioural evidence.

The KG was loaded from the `83d13c9` source tree. Its final audit
(`tools/kg/audit_kg.py`) reported one problem only, unrelated to the code: rmp
task #609, created during the sync, had no `Task` node. As a result, the graph's
own currency marker still reads `6bb0cb3`. The two commits differ in three files,
none of them ported library code (`Cargo.toml` `[profile.test]`,
`crates/rucene-codecs/tests/oracle/mod.rs`,
`crates/rucene-test-framework/src/jdk/stack_walker.rs`).

### Added — scope of the port

* **8 port modules in scope**: 7 modules of the Apache Lucene 10.5.0
  distribution, plus the randomizedtesting 2.8.1 runner that Lucene's tests run
  on. **28 Lucene modules are out of scope** (`Module.inScope = false`). See
  [Known limitations](#known-limitations).
* Every in-scope module reports `fullyPorted: true`: every in-scope top-level
  type `ported` or `notApplicable`, every nested type `ported` or `absorbed`,
  and **0 absent members** (`ask.py coverage`, `ask.py absent`).

| Lucene module | Crate | Top-level types ported | Nested types ported / absorbed | Members of in-scope types |
|---|---|---:|---:|---:|
| `lucene/core` | `rucene` | 1196 | 648 / 260 | 22542 |
| `lucene/analysis/common` | `rucene-analysis-common` | 498 | 116 / 0 | 5985 |
| `lucene/queryparser` | `rucene-queryparser` | 221 | 20 / 0 | 2122 |
| `lucene/test-framework` | `rucene-test-framework` | 210 (+2 not applicable) | 143 / 23 | 4115 |
| `lucene/queries` | `rucene-queries` | 157 | 90 / 0 | 2138 |
| randomizedtesting 2.8.1 `randomized-runner` | `randomizedtesting` | 95 | 28 / 21 | 909 |
| `lucene/codecs` | `rucene-codecs` | 86 | 92 / 7 | 2345 |
| `lucene/sandbox` (`FuzzyLikeThisQuery.java` only) | `rucene-sandbox` | 1 | 3 / 0 | 31 |

Of the member names that do not match Lucene's literally, the KG triage records
3394 as present under another name (`renamed`) and 301 as not applicable to Rust
(`na`), counted per `portTriageUnit`.

### Added — `rucene` (Lucene Core)

Top-level types ported per Lucene package (KG, `lucene/core`, `portScope = 'in'`;
all ported):

| Package | Types | Package | Types |
|---|---:|---|---:|
| `util` | 282 | `store` | 63 |
| `search` | 280 | `internal` | 55 |
| `index` | 203 | `analysis` | 53 |
| `codecs` | 125 | `geo` | 27 |
| `document` | 108 | | |

* **`store`**: directories, inputs and outputs, lock factories, `IOContext`.
  `NativeFSLockFactory` takes the same POSIX `fcntl` record lock the JDK takes,
  so a Rucene writer and a Lucene writer exclude each other
  (`Cargo.toml`, `rustix` comment; `tests/native_fs_lock_interop.rs`).
  Evidence: `tests/portability/io_context_contracts.rs`,
  `tests/portability/store_analysis_contracts.rs`, `tests/store_portability.rs`.
* **`index`**: `IndexWriter` (flush, commit, merges, deletes and updates, soft
  deletes, index sorting, `addIndexes`), readers and NRT, `IndexFileDeleter`,
  deletion policies, `CheckIndex`. Evidence: the `tests/index_writer_*.rs`
  integration tests, `tests/portability/{flush_files,indexing_chain,deletion_policy,index_file_deleter,delete_by_query}.rs`.
* **`search`**: queries, scorers, collectors, similarities, kNN search, sorting.
  Evidence: `tests/portability/{similarity_scores,multi_term_queries,scoring_queries,dv_range_feature_queries,knn_variants,search_domain}.rs`,
  `tests/search_on_disk.rs`.
* **`codecs`**: the formats of `lucene/core` 10.5.0 (`lucene90`, `lucene94`,
  `lucene95`, `lucene99`, `lucene103`, `lucene104` under `src/codecs/`), HNSW
  graphs and scalar quantization. See
  [Index compatibility](#added--index-compatibility-with-lucene-1050).
* **`document`**, **`geo`**: fields, points, ranges, geo points and shapes.
  Evidence: `tests/portability/{geo_shapes,inet_address_point,vector_point}.rs`.
* **`util`**: `BytesRef`, bit sets, packed ints, FSTs, automata, BKD, HNSW.
  Evidence: `tests/portability/{fst_arcs_buffer,fst_enum_reader,bkd_doc_ids,bkd_writer_layout,hnsw_graph,token_stream_to_automaton,prefix_coded_terms}.rs`.
* **`analysis`**: the core analysis API, `StandardTokenizer`,
  `StandardAnalyzer`. Evidence: `tests/portability/analysis_tokens.rs`,
  `tests/portability/utf16_offsets.rs`.
* **Panama vectorization**: the scalar and vectorized kernels match Lucene's
  results. Evidence: `tests/portability/panama_vectorization.rs`.

### Added — other workspace crates

* **`rucene-codecs`** (`lucene/codecs`): `SimpleText`, `blockterms`,
  `blocktreeords`, `bloom`, `memory` (`FSTPostingsFormat`,
  `DirectPostingsFormat`), `uniformsplit` and its `sharedterms` package,
  `bitvectors`. Evidence: `crates/rucene-codecs/tests/*_oracle.rs` (oracle parity
  with `lucene-codecs` 10.5.0) and the `base_*_format_codecs.rs` suites, which
  run Lucene's `Base*FormatTestCase` classes on these formats.
* **`rucene-analysis-common`** (`lucene/analysis/common`): analyzers, tokenizers,
  token filters and char filters, with the module's 130 SPI factories
  (`README.md`). It emulates the part of JDK 25 the module calls (locale case
  mapping, charsets, `BreakIterator`, `Collator`, regular expressions),
  verified against a JDK 25 oracle. Evidence:
  `crates/rucene-analysis-common/tests/analysis_oracle*.rs`.
* **`rucene-queries`** (`lucene/queries`): function queries and value sources,
  intervals, spans, payload queries, more-like-this (sprint 13).
* **`rucene-queryparser`** (`lucene/queryparser`): classic, flexible, simple,
  surround, complex-phrase, extendable and XML query parsers (sprint 13).
* **`rucene-sandbox`** (`lucene/sandbox`): `FuzzyLikeThisQuery` only.
* **`rucene-cli-example`** (binary `rucene-cli`): a command-line example that
  indexes and searches the USGS earthquake catalog (sprint 10). Not a Lucene
  module.
* **`rucene-cli-base`** (binary `rucene-base`): a command-line example with one
  index per Portal BASE (Portuguese public procurement) entity and pt-PT
  accent-insensitive search (sprint 14). Not a Lucene module.

### Added — index compatibility with Lucene 10.5.0

* **Default codec `Lucene104`**, in both directions
  (`specs/compatibility.md` §2). `tests/portability/index_interop.rs`
  (`rucene_and_lucene_accept_each_others_indexes`) writes the same corpus with
  both sides (every field kind: postings with positions, offsets and payloads,
  stored fields, term vectors, the five doc-values types, points, float and byte
  kNN vectors; deletes, doc-values updates, five commits, force-merges). Lucene
  opens Rucene's index and runs `CheckIndex` at `Level.MAX_VALUE`; Rucene opens
  Lucene's index and runs its own `CheckIndex`; both run the same query set on
  both indexes and the four reports must agree. A flipped byte is the positive
  control.
* **Byte-for-byte equality with Lucene's output**, file by file, for stored
  fields (`.fdt`, `.fdx`, `.fdm`), term vectors, norms, doc values (`.dvd`,
  `.dvm`), points and BKD, kNN vectors, scalar-quantized vectors, postings, and
  the files that seal a segment (`.fnm`, `.si`, compound files). Evidence:
  `tests/portability/{stored_fields,term_vectors,norms,doc_values,points,bkd_writer_layout,vectors,scalar_quantized_vectors,indexing_chain,seal_bytes}.rs`.
* **`lucene/codecs` formats**: for `SimpleText`, `blockterms`,
  `BlockTreeOrdsPostingsFormat`, `BloomFilteringPostingsFormat`,
  `DirectPostingsFormat`, `UniformSplitPostingsFormat` and its `sharedterms`
  variant, and `HnswBitVectorsFormat`, the oracle tests check that Rucene writes
  Lucene's bytes, that it reads what Lucene reads, that Lucene reads what Rucene
  writes, and that corrupted files raise Lucene's errors
  (`crates/rucene-codecs/tests/{simpletext,blockterms,blocktreeords,bloom,memory,uniformsplit,bitvectors}_oracle.rs`).
  `FSTPostingsFormat` is covered by Lucene's own `TestFSTPostingsFormat` suite
  (`base_postings_format_codecs.rs`).
* `BEST_COMPRESSION` stored fields are byte-identical only with the `zlib-c`
  feature (see [Known limitations](#known-limitations)).

### Added — test framework and test suite

* **`randomizedtesting`** (randomizedtesting 2.8.1 runner) and
  **`rucene-test-framework`** (`lucene/test-framework`) are ported, so Lucene's
  own test classes run in Rust on the ported randomized runner (KG coverage
  above; sprint 9 "Port Tests Framework", which holds rmp #361–#403 and
  #511–#529).
* **Test suite: 5330 tests** (nextest, every crate but `randomizedtesting`,
  which runs under `cargo test` beside it), all passing on this tree:
  rucene 3350, rucene-test-framework 1098, rucene-analysis-common 400,
  rucene-codecs 195, rucene-queryparser 98, rucene-queries 81, rucene-cli-base
  62, rucene-cli-example 44, rucene-sandbox 2. Evidence:
  `junit.xml` of the run of 2026-11-20 (see the release notes).
* **Java oracles**: many tests run Lucene 10.5.0 on a JVM and compare results;
  they fail, rather than skip, when Java or the Lucene jars are missing
  (`README.md`, "Java-oracle tests").
* **Fuzz tests** for the parsers of index bytes: `tests/fuzz/*.rs`
  (codec headers, stored fields, term vectors, norms, doc values, index file
  deleter, points, vectors).

### Added — licensing and attribution

* **`NOTICE`** at the repository root. It carries the Apache Lucene 10.5.0
  attributions, taken from Lucene's `NOTICE.txt` and limited to the material
  Rucene embeds: Brics Automaton, the moman/finenight Levenshtein tables,
  `WeakIdentityMap`, LZ4, the Snowball stemmers and stopword lists, KStem, the
  Jacques Savoy stopword lists and light stemmers, and the Unicode-derived
  word-break test data. It also attributes the ICU 79.1 Thai word list
  (`crates/rucene-analysis-common/resources/icu/thaidict.txt`), distributed
  under the Unicode License V3.

### Known limitations

* **Out-of-scope Lucene modules (28)**, not ported: `analysis/icu`,
  `analysis/kuromoji`, `analysis/morfologik`, `analysis/nori`,
  `analysis/opennlp`, `analysis/phonetic`, `analysis/smartcn`,
  `analysis/stempel`, `backward-codecs`, `benchmark`, `benchmark-jmh`,
  `classification`, `core.tests`, `demo`, `expressions`, `facet`, `grouping`,
  `highlighter`, `join`, `luke`, `memory`, `misc`, `monitor`, `replicator`,
  `spatial-extras`, `spatial-test-fixtures`, `spatial3d`, `suggest` (KG,
  `Module.inScope = false`). `lucene/sandbox` is limited to
  `FuzzyLikeThisQuery`.
* **No older index formats.** `backward-codecs` is out of scope, so Rucene
  reads only the formats Lucene 10.5.0's `lucene/core` and `lucene/codecs`
  write; it does not open indexes in formats that only `backward-codecs` reads
  (`specs/compatibility.md` §2).
* **968 declared divergences from Lucene 10.5.0** (KG `Divergence` nodes, one
  per declaration in the code; `ask.py divergences --file` / `--type` lists
  them): rucene 656 (`search` 228, `index` 113, `util` 108, `document` 48,
  `store` 46, `codecs` 55, `internal` 34, `analysis` 21, `geo` 2, `error` 1),
  rucene-test-framework 105, rucene-analysis-common 58, rucene-queryparser 49,
  rucene-queries 45, randomizedtesting 41, rucene-codecs 13, rucene-sandbox 1.
  Each is documented at its point of divergence, stating what Lucene does and
  why the port differs.
* **`BEST_COMPRESSION` stored fields** are byte-identical to Lucene's only with
  the `zlib-c` feature, which needs a C compiler. The default `zlib-rs` backend
  writes streams Lucene reads, but the bytes differ: 18 of 80 measured inputs
  matched JDK 21 `Deflater`, against 80 of 80 for C zlib (`Cargo.toml`).
  `BEST_SPEED`, the default mode, is byte-identical with either backend.
* **Thai segmentation**: `ThaiTokenizer` reads the JDK's Thai dictionary from an
  installed JDK; without one it falls back to the ICU 79.1 word list, which can
  split Thai text differently from Lucene (`README.md`).
* **Two test-framework classes are not applicable**: `TestSecurityManager` and
  `TestObjectInputFilterFactory` (KG, `portState = 'notApplicable'`).
* **Out-of-scope findings recorded in `rmp`**: the sprint logs hold 29
  `FINDING` entries marked out of scope and recorded without action (sprints 7,
  8, 9, 10, 11, 12, 13, 15 and 16). Their current state was not re-verified
  for this release, except: the licence mismatch (sprint 7) is resolved, every
  manifest declares MIT; the 91 `clippy` errors (sprint 10) are resolved,
  `clippy -D warnings` passes; the assertions-off failures (sprint 15) were
  fixed by rmp #576; the missing `NOTICE` file (sprint 12) is added in this
  release (see [Licensing and attribution](#added--licensing-and-attribution)).

[Unreleased]: https://github.com/FlavioCFOliveira/Rucene/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/FlavioCFOliveira/Rucene/releases/tag/v0.1.0
