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
  (`_compressing_random.tsv`). The Rust side is
  `crates/rucene-test-framework/tests/test_codecs_oracle.rs`, which includes
  the corpus helpers of `crates/rucene-codecs/tests/oracle/mod.rs`.
