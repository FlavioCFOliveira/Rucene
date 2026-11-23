# Module Specification: `benchmark`

**Spec ID:** BENCHMARK  
**Java module:** `lucene/benchmark` (package `org.apache.lucene.benchmark` and subpackages)  
**Crate:** `crates/rucene-benchmark`

## 1. Purpose

Provide Lucene's benchmarking framework, as Lucene 10.5.0 `lucene/benchmark` does: run indexing and search algorithms (`.alg` files) and report their statistics, and evaluate search quality on TREC topics and judgments.

## 2. Key classes / concepts ported

- `byTask`: `Benchmark`, `PerfRunData`, `utils.Algorithm` (the `.alg` parser), `utils.Config`, `utils.StreamUtils`, `utils.Format`; every task of `byTask.tasks` (`CreateIndex`, `AddDoc`, `Search*`, `Rep*`, `WriteLineDoc`, taxonomy tasks, ...); `stats` (`Points`, `TaskStats`, `Report`).
- `byTask.feeds`: content sources (`LineDocSource`, `TrecContentSource` and the TREC parsers, `EnwikiContentSource`, `ReutersContentSource`, `DirContentSource`, `SingleDocSource`, ...), `DocMaker`, query makers, `DemoHTMLParser`, facet sources, spatial doc and query makers.
- `quality`: `QualityBenchmark`, `QualityStats`, `trec` (`TrecTopicsReader`, `TrecJudge`, `QueryDriver`), `utils` (`SimpleQQParser`, `SubmissionReport`, ...); `utils.ExtractWikipedia`.

## 3. Design notes

- Java's `Class.forName` lookups of tasks, sources, makers, parsers and index-writer components go through the registry `classes`; an application (or a test) registers its own classes there.
- The JDK and icu4j behaviours the module uses are reproduced in the public modules `jdk` (including a replaceable `System.out`/`System.err` sink, `jdk::system`) and `icu` (the root `%spellout-numbering` rules of `RuleBasedNumberFormat`).
- GZIP and BZIP2 streams use `flate2` and `bzip2` (declared divergence, rmp #611: written bytes may differ from commons-compress; decoding is equal). HTML is parsed by the `nekohtml` crate.

## 4. Executables

The five `main` classes of the module are binaries of the crate (rmp #639). Each passes its command-line arguments (without the program name) to the ported `main(String[] args)` through `jdk::run_main`, which reproduces the `java` launcher: an exception thrown out of `main` prints `Exception in thread "main" ` followed by the exception's `printStackTrace()` text on stderr (`toString()`, `Caused by: ` causes; `\tat` frames only when `RUST_BACKTRACE` enables Rust backtraces) and exits with status 1; a `System.exit(n)` in `main` exits with `n`.

| Binary | Java class | Arguments |
| --- | --- | --- |
| `benchmark` | `byTask.Benchmark` | `<algorithm file>` |
| `query_driver` | `quality.trec.QueryDriver` | `<topicsFile> <qrelsFile> <submissionFile> <indexDir> [querySpec]` |
| `quality_queries_finder` | `quality.utils.QualityQueriesFinder` | `<index-dir>` |
| `extract_wikipedia` | `utils.ExtractWikipedia` | `--input\|-i <Path to Wikipedia XML file> [--output\|-o <Output Path>] [--discardImageOnlyDocs\|-d]` |
| `sample` | `byTask.programmatic.Sample` | none (ignored) |

Lucene's `gradle run` runs `Benchmark` on `-PtaskAlg=<file>`, default `conf/micro-standard.alg`; the equivalent here, from the crate directory `crates/rucene-benchmark` (the `.alg` files are shipped under `tests/conf`):

```sh
cargo run -p rucene-benchmark --bin benchmark -- tests/conf/micro-standard.alg
```

As with `gradle run`, the dataset the algorithm reads must be present first: `micro-standard.alg` reads the Reuters collection from `work/reuters-out` (`work.dir`, `docs.dir`), which Lucene's `getReuters` task downloads and extracts. Relative paths inside an `.alg` (`work.dir`, `docs.dir`, `docs.file`) resolve against the working directory, as in Java. The Gradle dataset-download tasks and the Perl scripts of `lucene/benchmark` are not ported.

Run without arguments, every binary prints what the Java class prints on JDK 25: the usage of `Benchmark`, `QueryDriver` and `QualityQueriesFinder` with status 1; `ExtractWikipedia` throws `NullPointerException` (it dereferences the missing input path before it can print its usage); `Sample` prints its properties and throws `ClassCastException`, because its `initProps()` names `ReutersContentSource` as `doc.maker`.

## 5. Acceptance criteria

- The 18 Lucene 10.5.0 test classes under `lucene/benchmark/src/test` (90 methods) are ported in `crates/rucene-benchmark/tests/`, with `conf/*.alg` and the test resources shipped as test resources, and pass (rmp #633).
- The benchmark Java oracle (`tests/benchmark_oracle.rs`, `BenchmarkOracle.java`) gives equal `Algorithm.toString()` for every `conf/*.alg`, `DemoHTMLParser` output, TREC and line-doc parsing, report text (timings masked) and root spellout.
- `cargo build -p rucene-benchmark --bins` builds the five binaries; `tests/executables.rs` runs `benchmark` on `tests/resources/bin/line-docs.alg` (exit 0, algorithm and report text) and every binary without arguments (Java's output and exit status) (rmp #639).
