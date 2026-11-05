# randomizedtesting oracle harness

Java programs that run randomizedtesting 2.8.1 and JDK 25 on fixed inputs and
write the fixtures (`../*.tsv`) the crate's parity tests (`src/tests/`) read.

* **Rebuild everything:** `./regenerate.sh` (JDK 25; jars from `~/.m2` or
  `$M2_REPO`: randomizedtesting-runner 2.8.1, junit 4.13.1,
  hamcrest-core 1.3). It compiles every `src/**/*.java`, runs each class in
  `oracles.txt` with the output directory as its argument, copies the
  `*.tsv` files to `tests/fixtures/` and the JDK data lists to `src/jdk/data/`.
* **Format:** tab-separated rows, `#` comment lines. A sequence of values is
  `OracleSupport.Rec`: an FNV-1a 64-bit digest over the little-endian bytes
  of each value (`int`/`long` as is, `float`/`double` by raw bits, `boolean`
  as one byte, `byte[]` and `String` prefixed by a 4-byte length, strings as
  UTF-16 code units) plus the first value. Strings in inputs are written as
  hex UTF-16 code units joined by `.` (`-` when empty); errors as
  `class<TAB>message`. The Rust side is `src/tests/mod.rs` (`Rec`, `rows`,
  `from_units`, `java_error`).
* **Extend:** add a class under `src/com/carrotsearch/randomizedtesting/`
  (that package reaches the package-private members), use `OracleSupport`
  (`Rec`, `units`, `err`, `seeds`, `tryRecord`), write its own `*.tsv` names
  into the directory it receives, list it in `oracles.txt`, and add the Rust
  test module that reads the new files.
* **Lucene oracles:** the classes under `src/org/apache/lucene/` drive
  Lucene 10.5.0 (`lucene-core`, `lucene-codecs`, `lucene-test-framework`, on
  Hamcrest 2.2) for the other crates of the workspace. Each writes its files
  into `<out>/<crate>/`; `regenerate.sh` copies the `*.tsv` files to
  `crates/<crate>/tests/fixtures/` and the JDK data lists to
  `crates/<crate>/src/jdk/data/`. Their temporary directories are masked as
  `$D` and identity hash codes as `@HASH`.
* **`SimpleText` oracles:** `org.apache.lucene.codecs.simpletext.SimpleTextOracle`
  writes `crates/rucene-codecs/tests/fixtures/simpletext_*.tsv` (every file of
  the indexes it writes with `SimpleTextCodec`, what Lucene reads back, the
  errors of corrupted copies, the service lookups). Its reader,
  `SimpleTextReadOracle.java`, depends only on `lucene-core` and
  `lucene-codecs`: `crates/rucene-codecs/tests/simpletext_oracle.rs` also
  launches it as a single source file (`java -cp <jars>
  SimpleTextReadOracle.java <dir>`) on the indexes Rucene writes.
* **The other `lucene/codecs` formats:**
  `org.apache.lucene.codecs.simpletext.PostingsFormatOracle <prefix> <kind>
  <format> <extension>...` (one line per format in `oracles.txt`) writes the
  corpus of `SimpleTextOracle` (`nocfs`, `cfs`, `edge`) with `Lucene104Codec`
  routing every field but `e_id` (`kind` `postings`, `format` an SPI name,
  `BloomFilter(Lucene104)`, or `UniformSplit(t,d,h)` /
  `SharedTermsUniformSplit(t,d,h)` for the formats built with those block
  settings and `dictionaryOnHeap`) or the byte-vector fields (`kind` `knn`)
  to the format, and records `crates/rucene-codecs/tests/fixtures/<prefix>_files.tsv`,
  `_reads.tsv` (`SimpleTextReadOracle.dump`), `_enum.tsv` (ords, term states
  and intersections; `postings` only), `_errors.tsv` (three corruptions of each
  given extension) and `_spi.tsv`. The `UniformSplit` formats also write a
  `shared` case (`writeShared`: terms that occur in several fields, every
  postings index option, three batches with deletions, `forceMerge(1)`) into
  `_files.tsv`, `_reads.tsv` and `_enum.tsv`. The Rust side is
  `crates/rucene-codecs/tests/oracle/mod.rs` (`FormatOracle`), shared by one
  test file per format family; its `java -cp` run of `SimpleTextReadOracle.java`
  adds `lucene-test-framework` (with randomizedtesting and JUnit) for the
  blockterms assemblies, which only that jar registers.
* **The test codecs:** `org.apache.lucene.tests.codecs.TestCodecsOracle`
  writes `crates/rucene-test-framework/tests/fixtures/testcodecs_*.tsv`: the
  `nocfs` and `cfs` corpus of `SimpleTextOracle.writeCorpus` (public for it)
  written with each codec of `lucene-test-framework`'s `asserting`,
  `cheapbastard`, `compressing` and `vector` packages (`_files.tsv`), the
  error of each misuse of an `Asserting` object (`_misuse.tsv`), the bounds
  and cranky methods of every draw a `CrankyCodec` makes while writing the
  corpus (`_cranky_draws.tsv`), the first failure per seed (`_cranky_seeds.tsv`)
  and `CompressingCodec`'s random factories per seed
  (`_compressing_random.tsv`). It also runs `PostingsFormatOracle.run` (the
  body of `PostingsFormatOracle.main`, with the crate directory and a supplier
  of the format, called once per corpus) for the postings formats of rmp #341
  into `rucene-test-framework/`: `tc_bloom_*` (`TestBloomFilteredLucenePostings`),
  `tc_rot13_*` and `tc_strot13_*` (the two Rot13 `UniformSplit` formats, with
  their `shared` case) and `tc_mockrandom_s{1,2,3}_*` (`new
  MockRandomPostingsFormat(new Random(seed))`, a new one per corpus); the
  blockterms formats reuse `rucene-codecs`' `fixedgap_*`, `vargap_fixed_*` and
  `vargap_docfreq_*`. `RAMOnlyPostingsFormat` cannot write the corpus (offsets,
  `DOCS` fields): `testcodecs_ramonly_corpus.tsv` records each root cause,
  and `_files.tsv`, `_reads.tsv` (`TestCodecsOracle.ramOnlyTrace`) and
  `_enum.tsv` the `ramonly` index of `writeRamOnly`, written and read in the
  oracle's JVM with the instance found by name. `testcodecs_rot13_flags.tsv`
  holds the Rot13 encoding flags, `testcodecs_spi.tsv` the 20 entries of the
  crate's service files. The Rust side is
  `crates/rucene-test-framework/tests/test_codecs_oracle.rs`, which includes
  the corpus helpers of `crates/rucene-codecs/tests/oracle/mod.rs`; its
  `fixture()` reads a crate's own fixture, or `rucene-codecs`' when the crate
  has none of that name. To refresh only these fixtures, run
  `TestCodecsOracle` alone (as `regenerate.sh` does) and copy the new files.
* **The helpers of `org.apache.lucene.tests.util` (rmp #342):**
  `org.apache.lucene.tests.util.TestUtilOracle` writes
  `crates/rucene-test-framework/tests/fixtures/testutil_*.tsv`: `English`
  (`_english.tsv`), every `TestUtil` random generator per parameter set and
  seed (`_generators.tsv`: digest of 40 results, the `nextLong()` left next,
  the first result of the first seed), the `randomPattern` sequences
  (`_patterns.tsv`), `LineFileDocs` over the embedded `europarl.lines.txt.gz`
  (`_linefiledocs.tsv`), `AutomatonTestUtil` (`_automaton.tsv`, with
  `randomSingleAutomaton` called by reflection), `FSTTester.doTest`
  (`_fsttester.tsv`: FST bytes, node and arc counts, random state) and the
  comparators and ULP checks (`_compare.tsv`), the other `TestUtil` members
  (`_members.tsv`: `reduceOpenFiles` on a live writer per merge policy and
  scheduler, `getPostingsFormat`/`getDocValuesFormat` per codec,
  `cloneDocument`, `addIndexesSlowly`, `assertConsistent` messages) and
  `RamUsageTester` (`_ramusage.tsv`). The Rust side is
  `crates/rucene-test-framework/tests/test_util_oracle.rs`, `test_util_index.rs`
  (`_members.tsv`) and `ram_usage_tester_oracle.rs` (`_ramusage.tsv`).
* **The time-zone rules (rmp #357 block 5):**
  `org.apache.lucene.tests.util.TimeZoneOracle` (run with `--add-opens
  java.base/sun.util.calendar=ALL-UNNAMED`, which `regenerate.sh` passes)
  writes `crates/rucene-test-framework/tests/fixtures/time_zone_hours.tsv`:
  per ID of `TimeZone.getAvailableIDs()`, sorted, the number of instants of
  the zone's grid and the FNV-1a digest of each (instant, `HOUR_OF_DAY`)
  pair, the hour read from a new `GregorianCalendar` over a fresh
  `TimeZone`. The grid is the extremes of `long`, `t - 1, t, t + 1` for every
  instant of the zone's `ZoneInfo` transition table, `e - 1, e` for every
  offset change of its last rule found in the years of `YEARS`, and 64 draws
  of `TestUtil.nextLong(new Random(row), 0, Long.MAX_VALUE)`. It also writes
  `alcoholic_hours.tsv`: per seed, `LuceneTestCase.randomTimeZone(new
  Random(seed))` and the `HOUR_OF_DAY` and `maxMergeSize` of `new
  AlcoholicMergePolicy(tz, new Random(seed))`. The Rust side is
  `crates/rucene-test-framework/tests/time_zone_oracle.rs` and the unit test
  of `src/index/alcoholic_merge_policy.rs`; the rules themselves come from
  `tools/gen/TimeZoneRules.java`.
* **The store wrappers of `org.apache.lucene.tests.store` (rmp #343):**
  `org.apache.lucene.tests.store.MockDirectoryWrapperOracle` writes
  `crates/rucene-test-framework/tests/fixtures/mdw_index.tsv` (the files, as
  hex, of a small index written with the default codec: 5 documents, a stored
  `StringField` and a `TextField`, no compound files, no doc values),
  `mdw_corpus.tsv` (the files of the corpus of `SimpleTextOracle.writeCorpus`
  written with the default codec, `nocfs` and `cfs`) and `mdw_steps.tsv`: per scenario, seed and step, the call and its outcome
  (`ok<TAB>value` or `err<TAB>class<TAB>message`, plus the cause of a plain
  `RuntimeException`). The scenarios drive `MockDirectoryWrapper` over a
  `ByteBuffersDirectory` with fixed seeds: random `IOException`s on write and
  on open, open-file tracking and the close-time checks, `Failure`s (counting
  the failure points of each call, and firing on a `deleteFile` stack frame),
  disk full, `crash()` of synthetic unsynced files (`alwaysCorrupt` too),
  `crash()`, `checkIndexOnClose`, `corruptUnknownFiles` and the
  unreferenced-files check over the index, the slow open/closers and
  throttling; and `SerialIOCountingDirectory` counts on a fixed read pattern,
  `CorruptingIndexOutput` and `RawDirectoryWrapper`; and the corpus read in
  full through the wrapper (a `DirectoryReader` opened and closed, `CheckIndex`
  at `MIN_LEVEL_FOR_SLOW_CHECKS`, the wrapper closed, which fails while a
  reader left an input open). Each scenario that draws
  ends with the wrapper's next `randomState.nextLong()`, which pins the number
  of draws. The Rust side is
  `crates/rucene-test-framework/tests/mock_directory_wrapper_oracle.rs`, which
  replays every scenario and compares the rows. The segment ids of
  the index and the corpus are random, so the three files change together on
  every run and must be copied together.
* **The index readers and wrappers of `org.apache.lucene.tests.index` (rmp #344):**
  `org.apache.lucene.tests.index.IndexWrappersOracle` (listed after
  `MockDirectoryWrapperOracle`, whose `mdw_corpus.tsv` it reads from the
  output directory; to run it alone, copy the committed
  `crates/rucene-test-framework/tests/fixtures/mdw_corpus.tsv` there first)
  writes `crates/rucene-test-framework/tests/fixtures/iw_wrappers.tsv`: per
  corpus case (`nocfs`, `cfs`), scenario and step, `ok<TAB>value` or
  `err<TAB>class<TAB>message` (identity hashes masked as `@HASH`, thread ids
  as `#N`). Per leaf of each wrapper (`AssertingDirectoryReader`,
  `AllDeletedFilterReader`, `FieldFilterLeafReader` with and without negation,
  `MismatchedDirectoryReader`/`MismatchedLeafReader`/`MismatchedCodecReader`
  for three seeds, `MergingDirectoryReaderWrapper`, `MergingCodecReader`,
  `MergeReaderWrapper`): `toString()`, document counts, live docs (and whether
  they are a `LiveDocs`), which accessors answer for a fixed set of fields, the
  cache-helper identity relations, the field numbers the stored fields report,
  and the digest (`OracleSupport.Rec`'s line digest and the line count) of
  `SimpleTextReadOracle.dumpLeaf` (public, with a `codec=null` line for a leaf
  that is not a `SegmentReader`) through the wrapper, its `   live` impact lines
  digested apart, with the dump's `leaf`/`live`/`fi` lines in full. It also
  records `MismatchedLeafReader.shuffleInfos` for every seed of the grid and the
  `nextLong()` each `Mismatched*` wrapper leaves; `PerThreadPKLookup` lookups,
  `reopen` and two segments holding the same ids (`pkdup`);
  `OwnCacheKeyMultiReader`'s key and listeners; the class each directory
  wrapper's reopen returns; each wrapper's life cycle (the wrapped reader
  closed, the wrapper closed); the misuse of the asserting views; and
  `PointsStackTracker` fed by the `compare` calls of an intersection over an
  index of 5000 points (`points`). The Rust side is
  `crates/rucene-test-framework/tests/index_wrappers_oracle.rs`, which replays
  every scenario and compares the rows, except those its `DECLARED` list names
  with their reason; `IW_DUMP_DIR=<dir>` makes it write each dump in full.
* **The search wrappers of `org.apache.lucene.tests.search` (rmp #345):**
  `org.apache.lucene.tests.search.SearchWrappersOracle` (listed after
  `MockDirectoryWrapperOracle`, whose `mdw_corpus.tsv` it reads from the
  output directory) writes
  `crates/rucene-test-framework/tests/fixtures/search_wrappers.tsv`: per corpus
  case (`nocfs`, `cfs`), scenario and step, `ok<TAB>value` or
  `err<TAB>class<TAB>message`. The corpus scenarios run inside the suite
  `TestSearchWrappersScenarios` under `RandomizedRunner` (`tests.seed`
  `5EA4C4E5`), because `AssertingLeafCollector` draws `rarely()` from the test
  context; the suite's last row is the context's next `random().nextLong()`.
  They record the top hits (doc and score bits), total hits, count and top
  explanation (match, value bits, description) of term, boolean (`SHOULD`,
  `MUST`, `FILTER`, `MUST_NOT`), phrase, prefix, wildcard, point-range,
  field-exists, constant-score, boost, disjunction-max, match-all/none and kNN
  queries and of the `AssertingQuery`, `RandomApproximationQuery`,
  `BlockScoreQueryWrapper` and `DisablingBulkScorerQuery` wrappers, through
  `AssertingIndexSearcher` (seeds 1, 2, 3, each ending with the searcher's
  next `nextLong()`), `ScorerIndexSearcher`, and an `AssertingIndexSearcher`
  whose similarity is `AssertingSimilarity(RandomSimilarity)`; the misuse of
  `AssertingScorer` (and of its two-phase view), `AssertingCollector`,
  `AssertingLeafCollector`, `AssertingBulkScorer`, `AssertingWeight` and its
  scorer supplier, `AssertingMatches`/`AssertingMatchesIterator`,
  `AssertingQuery`, `AssertingIndexSearcher.search` and
  `RandomApproximationQuery.RandomTwoPhaseView`, with Java's message; and
  `FixedBitSetCollector`, `DummyTotalHitCountCollector`,
  `BulkScorerWrapperScorer` and `DisablingBulkScorerQuery` over a term without
  postings (Java's `NullPointerException`). Outside the runner it records
  `RandomSimilarity`'s pick per field, its `toString()` and the next draw for a
  grid of seeds, and `AssertingSimilarity`'s `toString()`. After the suite's
  `ctx` row, `SearcherChecksOracle` (rows written last, after the `sim` rows;
  `nocfs` corpus; run with the class's shared executor set to `null` and no
  default query cache, so that every draw happens on the test thread) records:
  the searcher `LuceneTestCase.newSearcher` builds for a sequence of calls of
  each overload (asserting or plain, reader wrapped or not, leaves, timeout,
  caching policy, slices), the reader `wrapReader`/`maybeWrapReader` returns
  (class kind, leaves, counts, hits of a term); `QueryUtils.check(random, q,
  s)` for the query set, pass or Java's message; `CheckHits` on correct and
  deliberately wrong expectations (`checkHitCollector`, `checkHits`,
  `checkDocIds`, `checkHitsQuery`, `checkEqual`, `hits2str`, `topdocsString`,
  `checkNoMatchExplanations`, deep `checkExplanations`, `checkMatches`,
  `checkTopScores` per query, `verifyExplanation` within and outside its
  tolerances); and the `Scorer` branch of `AssertingLeafCollector` (a `Scorer`
  reaching a collector, a score read during a batch, the `AssertingScorer`
  state carried across calls, an `AssertingScorer` passed through unchanged,
  nested collectors); and sorted searches (`sorted_search`: per sort and query,
  `search(Query, int, Sort)` through `CheckHits.ExplanationAssertingSearcher`,
  `search(Query, int, Sort, true)`, `searchAfter(ScoreDoc, Query, int, Sort)`
  and its scoring overload, each hit as `FieldDoc.toString()`; their argument
  errors; and a similarity whose explanations exceed the score, which fails
  only the explanation-checking searcher); each group ends with the context's
  next draw. The Rust
  side is `crates/rucene-test-framework/tests/search_wrappers_oracle.rs`,
  which replays every scenario (the suite through the port's runner) and
  compares the rows.
* **The analysis mocks of `org.apache.lucene.tests.analysis` (rmp #346) and
  `BaseTokenStreamTestCase` (rmp #357):**
  `org.apache.lucene.tests.analysis.AnalysisMocksOracle` writes
  `crates/rucene-test-framework/tests/fixtures/analysis_mocks.tsv`, one row
  per scenario, seed and step (`scenario<TAB>seed<TAB>step<TAB>value...`). It
  runs inside the suite `TestAnalysisMocksScenarios` under `RandomizedRunner`
  (`tests.seed` `1DB1C0DE`), and every scenario that draws from the test
  context runs in `runWithPrivateRandomness(new Randomness(seed,
  RandomSupplier.DEFAULT))`, so its draws depend on the seed of the grid
  only. A token stream is dumped as its tokens (term, start and end offset,
  position increment and length, type, payload, flags, term frequency and
  keyword flag, `-` for an absent attribute, strings as hex UTF-16 units),
  `end:` and the attributes after `end()`, an exception ending the dump with
  `!class<TAB>message`. The scenarios: `tok`, `MockTokenizer` over six
  patterns (`WHITESPACE`, `KEYWORD`, `SIMPLE`, `[A-Z][a-z]*`, `..`, `.`) and
  a ten-text corpus (empty input, supplementary characters, a 36-character
  token, case mapping, stop words, control whitespace), with the
  implementation `newAttributeFactory` picked and the context's next
  `nextLong()`; `rd`, `MockReaderWrapper` (spoon-feeding and
  `throwExcAfterChar`) and `MockCharFilter` (remainders 0, 2, 7) under the
  tokenizer; `an`, five `MockAnalyzer` configurations over three fields
  (`normalize` and the next draw included); `fl`, every filter (fixed and
  variable payloads, holes, graph, random lookahead, synonyms, lower-casing,
  stop set, cranky, validating, simple payload) with the next draw of the
  filter's own random and of the context; `fx`, the canned streams, the
  payload, bytes and synonym analyzers and `TokenStreamToDot`; `mis`,
  `MockTokenizer` workflow violations with and without checks, the analyzer
  misuses and the assertion messages of `BaseTokenStreamTestCase`
  (`ComparisonFailure` included); `crd`, the strings `checkRandomData` reads
  on the calling thread, its outcome and the next draws, and
  `TestUtil.randomAnalysisString`; `core`, `StandardTokenizer`'s attributes
  after `end()` and the attribute sharing between a tokenizer and its
  filters; `u16`, offsets, final offsets and length limits in UTF-16 units
  over supplementary text (`StandardTokenizer` with maximum token lengths
  255, 5, 2 and 1, `MockTokenizer` with a limit of 3, both plain and behind
  `MockCharFilter`, `StandardAnalyzer` and `MockAnalyzer`, on two corpus
  texts and the inputs of Lucene's `TestStandardAnalyzer`); `fac`, the core
  filters (`LowerCaseFilter`, `StopFilter`, `MockSynonymFilter` over them,
  `CachingTokenFilter` replayed, a `GraphTokenFilter` walk) and `QueryBuilder`
  over a `StandardTokenizer` built with each of the three factories
  `newAttributeFactory` picks; `stdtok`, `StandardTokenizer` (the JFlex
  `StandardTokenizerImpl`) over the inputs of Lucene's `TestStandardAnalyzer`
  (Thai, Lao, Hangul, Japanese, combining marks, numerics, `testMid`, emoji,
  ZWJ, flag, keycap and tag sequences, the default maximum token length) plus
  odd regional indicators and supplementary digits, per maximum token length
  (1, 2, 3, 255) and reader (plain, `MockCharFilter` 0/2/7, the spoon-feed
  reader of `TestStandardAnalyzer` at 1, 2 and 9 chars per read), then
  `setMaxTokenLength` between two streams of one tokenizer and in the middle
  of a stream (`setBufferSize`, whose shrinking fails as Java's does). The
  Rust side is `crates/rucene-test-framework/tests/analysis_mocks_oracle.rs`,
  which replays every scenario through the port's runner and compares the
  rows.
