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
* **The store wrappers of `org.apache.lucene.tests.store` (rmp #343):**
  `org.apache.lucene.tests.store.MockDirectoryWrapperOracle` writes
  `crates/rucene-test-framework/tests/fixtures/mdw_index.tsv` (the files, as
  hex, of a small index written with the default codec: 5 documents, a stored
  `StringField` and a `TextField`, no compound files, no doc values) and
  `mdw_steps.tsv`: per scenario, seed and step, the call and its outcome
  (`ok<TAB>value` or `err<TAB>class<TAB>message`, plus the cause of a plain
  `RuntimeException`). The scenarios drive `MockDirectoryWrapper` over a
  `ByteBuffersDirectory` with fixed seeds: random `IOException`s on write and
  on open, open-file tracking and the close-time checks, `Failure`s (counting
  the failure points of each call, and firing on a `deleteFile` stack frame),
  disk full, `crash()` of synthetic unsynced files (`alwaysCorrupt` too),
  `crash()`, `checkIndexOnClose`, `corruptUnknownFiles` and the
  unreferenced-files check over the index, the slow open/closers and
  throttling; and `SerialIOCountingDirectory` counts on a fixed read pattern,
  `CorruptingIndexOutput` and `RawDirectoryWrapper`. Each scenario that draws
  ends with the wrapper's next `randomState.nextLong()`, which pins the number
  of draws. The Rust side is
  `crates/rucene-test-framework/tests/mock_directory_wrapper_oracle.rs`, which
  replays every scenario and compares the rows; its `KNOWN_CORE_DIVERGENCES`
  lists the `close dir` rows whose `CheckIndex` message differs for core
  reasons, and fails when one of them starts to match. The index's segment id
  is random, so the two files change together on every run and must be
  copied together.
