# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each entry names the evidence behind it: a knowledge-graph (KG) query (run with
`python3 tools/kg/ask.py` up to 0.1.0, with `rmp graph client` since), a test file, or
an `rmp` task or sprint of the `rucene` roadmap.

## [Unreleased]

## [0.2.0] - 2026-11-27

Second release. It covers the 97 commits of `develop` from `v0.1.0` to
`070015e6` (sprints 17, 18, 19 and 20 of the `rucene` roadmap; every
non-merge commit names the `rmp` task it serves) and the release fixes on
`release/0.2.0` (`0306398d`).

Release notes: [`docs/releases/0.2.0.md`](docs/releases/0.2.0.md).

**Breaking changes.** The public API of `rucene` changed incompatibly; see
[Changed — breaking changes to the `rucene` API](#changed--breaking-changes-to-the-rucene-api).
The crate is pre-1.0, so the minor version is bumped.

**Coverage figures.** The KG coverage and divergence figures of 0.1.0 were not
re-measured for this release.

### Added — ported Lucene modules

Each crate ports one Lucene 10.5.0 module (sprint 17). Evidence: the crate's
`tests/` directory, including its Java-oracle tests, and the `rmp` tasks.

* **`rucene-memory`** (`lucene/memory`, `MemoryIndex`): #612, #629
  (`d58abe96`, `4aaea6c9`, `c995a5b2`). Oracle: `memory_index_oracle.rs`.
* **`rucene-facet`** (`lucene/facet`: root, `sortedset`, `taxonomy`, `range`,
  `rangeonrange`, `facetset`): #613–#615, #630 (`d58abe96`, `4aaea6c9`,
  `568d2e58`, `cfa4ca49`, `01e8f4a9`). Oracle: `facet_oracle.rs`.
* **`rucene-highlighter`** (`lucene/highlighter`: `highlight`,
  `vectorhighlight`, `uhighlight`, `matchhighlight`): #616, #617, #629
  (`d58abe96`, `568d2e58`, `4f999b12`, `dcb4d959`, `648ac89a`). Oracle:
  `highlighter_fragments_oracle.rs`.
* **`rucene-spatial3d`** (`lucene/spatial3d`): #618–#620, #631 (`3f097814`,
  `c995a5b2`). Oracle: `spatial3d_oracle.rs`.
* **`rucene-spatial-extras`** (`lucene/spatial-extras`, `prefix` and
  `prefix.tree` included): #623, #624, #632 (`3f097814`, `12d3ca60`,
  `eaf78583`). Oracle: `spatial_extras_oracle.rs`.
* **`rucene-spatial-test-fixtures`** (`lucene/spatial-test-fixtures`): #631
  (`0b6f3838`).
* **`rucene-benchmark`** (`lucene/benchmark`: `byTask`, `quality`, `trec`,
  `utils`): #625–#627, #633 (`12d3ca60`, `f685660a`, `2cf6b192`, `cfa4ca49`).
  Lucene's 37 `conf/*.alg` files are copied unchanged into `tests/conf/`.
  Executables for the five main classes (`benchmark`, `query_driver`,
  `quality_queries_finder`, `extract_wikipedia`, `sample`): #639 (`be24f481`).
  Oracle: `benchmark_oracle.rs`.
* **`rucene-analysis-icu`** (`lucene/analysis/icu`, with the build tools
  `rbbi_rule_compiler` and `generate_utr30_data_files`): #641, #642
  (`df1d7dee`, `7c66c9cb`, `8f6e8648`, `1394d707`). Oracle: `icu_oracle.rs`.

### Added — ported third-party libraries

Each is limited to the part the ported Lucene modules use, and each translated
file keeps its upstream copyright and licence notice (`NOTICE`).

* **`icu4j`**: ICU4J 77.1, the classes `lucene/analysis/icu` reaches, whole
  files, with the 3476 ICU data files they load embedded unchanged: #640,
  #643–#656 (`37433a99`, `77fe5e56`, `4750c3f1`, `6a1980ea`, `d693be32`,
  `df8a62a8`, `49214b11`, `aebd5f6e`).
* **`spatial4j`**: spatial4j 0.8: #621 (`3f097814`).
* **`s2-geometry`**: s2-geometry-library-java 1.0.0: #622 (`3f097814`).
* **`nekohtml`**: NekoHTML 1.9.17, with the Xerces-J 2.12.2 and SAX 2.0.2
  types it needs: #635 (`4aaea6c9`).

### Added — `rucene`

* `impl TwoPhaseCommit for IndexWriter` (`IndexWriter.java:198`): #659
  (`412b9ff2`).
* `HasIndexSlice` on the off-heap float, byte and scalar-quantized vector
  values; the memory-segment flat scorer routes through it as Lucene does:
  #660 (`412b9ff2`).
* `MergedTermsEnum`, returned by the merged sorted and sorted-set doc values:
  #662 (`e6a0bf9b`).
* Members the KG reported missing outside the sprint-17 modules: #634, #647
  (`2df9511a`, `cc7df513`).

### Added — tooling and licensing

* **`tools/bench-compare`**: runs one `byTask` algorithm on Reuters-21578 with
  Lucene 10.5.0 and Rucene and prints both reports side by side, with medians
  over several runs: #673, #677 (`ce1d6a95`, `35aef0ca`).
* **`tools/kgsync`** (Rust) replaces the `tools/kg` Python chain for the
  Knowledge Graph. Each layer derives the desired state of the slices it owns
  (the `syn` AST of the crate, the javac AST of every reference module, the
  port claims in the doc comments, `git` and `rmp`). It reads the current state
  concurrently with the parse, writes only the difference, and skips a layer
  whose inputs are unchanged. Measured on 16 cores: 0.15 s with no change,
  about 1.8 s for a commit that changes Rust sources, 4–7 s for a full
  verification. Every identity now carries a `UNIQUE` constraint, and Rust
  types are keyed by `qualifiedName` (#658, `4ff33119`;
  `tools/kgsync/README.md`, `knowledge-model.md`).
* **`licenses/Apache-2.0.txt`**, and `NOTICE` entries for ICU4J 77.1,
  spatial4j 0.8, s2-geometry-library-java 1.0.0, NekoHTML 1.9.17, Apache
  Xerces-J 2.12.2, SAX 2.0.2, and the ICU data and Unicode-derived files of
  `lucene/analysis/icu`.

### Changed — breaking changes to the `rucene` API

* **Minimum supported Rust version 1.100** (was 1.89) in every manifest, by
  user decision. The technical minimum is 1.95.0, the first release where the
  code compiles: it uses `if let` guards on match arms, stable since Rust
  1.95.0 (rust-lang/rust `RELEASES.md`, PR #141295); `f4a54f53` introduced
  them. 1.100 is a policy choice above that minimum: every release gate ran on
  rustc 1.100.0 (2026-11-09). **Migration:** build with Rust 1.100 or later.
* **`LuceneError` is one pointer wide**: `pub struct LuceneError(Box<LuceneErrorInner>)`,
  which dereferences to `LuceneErrorInner` (`kind`, `suppressed`, …). A struct
  pattern `LuceneError { kind: …, .. }` no longer compiles. **Migration:**
  match on `error.kind` (`matches!(error.kind, LuceneErrorKind::…)`),
  `error.kind()` or `error.into_kind()`. #675, #676 (`f4a54f53`).
* **`SortedDocValues` and `SortedSetDocValues`**: `terms_enum` and
  `intersect` take `&mut self` instead of `self: Box<Self>` and return
  `Box<dyn TermsEnum + '_>`, which borrows the values, as Lucene calls
  `termsEnum()` on the same object more than once; both traits now require
  `Send`. **Migration:** implementors change the two signatures and make the
  type `Send`; callers keep the values alive while the terms enum is in use.
  #662, #668 (`e6a0bf9b`, `f7151f46`).
* **`SortedDocValuesTermsEnum` and `SortedSetDocValuesTermsEnum`** take a type
  parameter `P: DocValuesRef` (new trait, implemented for `&mut V`, `Box<V>`
  and `Arc<Mutex<Box<V>>>`) instead of the values type `V`. `new(Box<V>)` still
  compiles; a type written as `SortedDocValuesTermsEnum<dyn SortedDocValues>`
  does not. #668 (`f7151f46`).
* **`FilteredTermsEnum`, `TermsEnumIndex` and `AutomatonTermsEnum::new`** carry
  the lifetime of the terms enum they wrap (`FilteredTermsEnum<'a>`,
  `TermsEnumIndex<'a>`). **Migration:** add the lifetime, `'static` for an
  owned terms enum. #662 (`e6a0bf9b`).
* **`HnswUtil::graph_is_rooted(reader, vector_field)`** drops its third
  parameter: it unwraps `FilterLeafReader` and casts to `CodecReader` itself,
  as `HnswUtil.java:208-215` does. #664 (`412b9ff2`).
* **`DocIdSetBuilder::build`** returns `Box<dyn DocIdSet + Send + Sync>`
  (`d58abe96`, #612–#617).
* **LZ4 `compress` and `compress_with_dictionary`** are generic over the
  output and the hash table (`O: DataOutput + ?Sized`, `H: Lz4HashTable + ?Sized`)
  instead of taking `&mut dyn` arguments. Calls compile unchanged; use as a
  function pointer needs the type parameters. #679 (`7e5a6923`).
* `search::bayesian_score_estimator::JavaRandom` moved to
  `util::java_random::JavaRandom`; the old path re-exports it (`d58abe96`).

### Changed — behaviour made faithful to Lucene

* `HnswUtil.graphIsRooted` unwrapping, see above: #664 (`412b9ff2`).
* The three geo doc-values queries build a `TwoPhaseIterator` in a
  `ConstantScoreScorer` with Lucene's match costs, and `isCacheable` uses
  `DocValues.isCacheable(ctx, field)`: #665 (`412b9ff2`).
* `LatLonPointDistanceQuery`, `XYPointInGeometryQuery` and every
  `SpatialQuery`-based query intersect only in `ScorerSupplier.get`, with the
  cost estimated once (`SpatialQuery.java:150-391`): #666 (`412b9ff2`).
* `SortedSetSelector.wrap` returns the unwrapped singleton first
  (`SortedSetSelector.java:70-75`): #667 (`412b9ff2`).
* `TermOrdValComparator` and `DocValuesRewriteMethod` share one values object
  with its terms enum, as Java does: #668 (`f7151f46`).
* The benchmark puts `analysis-icu` on the analysis SPI class path, as
  Lucene's benchmark runtime class path does: #661 (`412b9ff2`).

### Changed — performance

Sprint 20 compared Rucene with Lucene 10.5.0 on the `byTask` algorithms of
`lucene/benchmark/conf` (`tools/bench-compare`) and optimised Rucene's hot
paths, keeping Lucene's behaviour and file formats: #675, #676, #679–#682,
#685 (40 `perf` commits from `1f274d95` to `2c4f248c`, each naming its tasks;
seven of them include the `rucene-benchmark` harness in their scope).

* **Result at `2c4f248c`** (ratio R/L of Rucene to Lucene elapsed time; sprint
  20 FINDING of 2026-11-27): **17 of 20 algorithms take less total time in
  Rucene than in Lucene**, among them `standard` 0.70, `micro-standard` 0.56
  and `highlights` 0.64, and the `Warm` operation 0.42. Three take more:
  `collector` about 1.16, `sort-standard` about 1.11, `sloppy-phrase` about
  1.07. See [Known limitations](#known-limitations).
* **These figures predate a release fix.** They were measured at `2c4f248c`,
  before `0306398d` removed the batch `collect_stream` path of
  `BooleanScorer::score_window_single_scorer` (see [Fixed](#fixed)). That
  path was part of `49502b4b`, which reported −4.0 % instructions on
  `standard` search for the whole commit; the share given back by the
  removal was not measured, and no figure was re-measured after it.
* **Build target:** on x86-64 Linux, `.cargo/config.toml` builds the workspace
  for `target-cpu=x86-64-v3` (AVX2, FMA, BMI2). Builds of this workspace,
  tests included, therefore require such a CPU; a crate that depends on
  `rucene` does not inherit the setting. FMA contraction is not enabled, so
  scores stay bit-identical. #680, #681 (`9b3ca2ba`, `73ea161c`).
* **Safe vectorisable kernels** where Lucene vectorises: LZ4 16-byte copies,
  kNN lane accumulators (bit-identical results), 8-byte mismatch, UTF-16 to
  UTF-8 ASCII blocks, BM25 cache. No `unsafe` AVX2 kernels. #685 (`e6fd9fa6`).

### Changed — other

* `rucene-benchmark` compresses with the workspace `zlib-rs` backend of
  `flate2`: #611 (`482d07cc`).

### Fixed

* `BooleanScorer::score_window_single_scorer` collects one document at a
  time again, as Lucene does (`BooleanScorer.java:284-289`). The batch
  `collect_stream` path that `49502b4b` added for the no-score case drew one
  extra `rarely()` from the test framework's random source, which shifted
  every later draw: `search_wrappers_match_lucene` (37 of 1168 rows) and
  `test_geo_point_suite_draws_match_lucene` (7 of 132 rows) diverged from
  Lucene. `src/search/boolean_scorer.rs` is back to its state before
  `49502b4b` (`0306398d`).
* `%f` in the similarities' explanations is rounded as JDK 25 rounds it
  (1130 of 8025 `double` and 1104 of 12028 `float` rows differed): #657
  (`7c2d0766`).
* `SoftDeletesDirectoryReaderWrapper` applies soft deletes found in every
  doc-values type; before, soft-deleted documents of a binary doc-values field
  stayed visible: #663 (`412b9ff2`).
* `TotalTermFreqValueSource` and `SumTotalTermFreqValueSource` `getValues`
  return Lucene's null stand-in before `createWeight` and fail only on first
  use (`TotalTermFreqValueSource.java:62-65`): #669 (`f7151f46`).
* `reflectAsString` prints values as Java `String.valueOf` and, with
  `prependAttClass`, prefixes the Java attribute interface name: #642
  (`ef43efb2`, `9cd0b34f`).
* `TermQuery` and `TermInSetQuery` subclasses match as Java `instanceof`
  does: #613, #616, #617 (`568d2e58`).
* Test: the concurrent-merge-scheduler `merge_panic` test accepts the three
  outcomes Lucene's own `forceMerge` race gives: #638 (`82514e3d`).

### Removed

* `tools/kg`, the Python Knowledge Graph chain, replaced by `tools/kgsync`:
  #658 (`4ff33119`).
* The unused `tokio` dependency of `rucene` (`0306398d`).

### Known limitations

* **Slower than Lucene** at `2c4f248c` (sprint 20 FINDING of 2026-11-27):
  `collector` about 1.16, `sort-standard` about 1.11 and `sloppy-phrase` about
  1.07 in total (`sloppy-phrase` search 1.36 on the same index). Operations
  above 1.0: `standard` `SearchSameRdr` about 1.09–1.17; `sort-standard`
  `MAddDocs` 1.21 and `CloseIndex` 1.50; `collector` `ForceMerge` 1.27, and its
  `MAddDocs` drifts after round 0; some `english-porter`, `shingle` and `wstok`
  operations at 1.02–1.06.
* **Open performance findings**, not acted on: `collector` memory grows by
  about 280 MB per round (not traced); about 24 store-slice allocations per
  query (task cancelled); `IndexFileDeleter::check` on the merge thread
  (2.15 %); `Rc<RefCell>` shared postings in phrase matching (awaits a user
  decision); the UTF-16 term buffer (#686, in the backlog).
* **CPU requirement** of builds made in this workspace on x86-64 Linux: AVX2,
  FMA and BMI2 (see [Changed — performance](#changed--performance)).
* **Out-of-scope Lucene modules**: of the 28 listed for 0.1.0, this release
  ports `analysis/icu`, `benchmark`, `facet`, `highlighter`, `memory`,
  `spatial-extras`, `spatial-test-fixtures` and `spatial3d`. The other 20 are
  not ported. The ICU4J, spatial4j, s2-geometry and NekoHTML ports are limited
  to the parts these modules use.
* **Not on crates.io**: the crate name `rucene` is taken there by another
  project.
* The limitations of 0.1.0 on older index formats, `BEST_COMPRESSION` and Thai
  segmentation still apply.

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
  [Known limitations](#known-limitations-1).
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
  feature (see [Known limitations](#known-limitations-1)).

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

[Unreleased]: https://github.com/FlavioCFOliveira/Rucene/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/FlavioCFOliveira/Rucene/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/FlavioCFOliveira/Rucene/releases/tag/v0.1.0
