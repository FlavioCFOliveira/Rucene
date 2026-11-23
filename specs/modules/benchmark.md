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

## 4. Acceptance criteria

- The 18 Lucene 10.5.0 test classes under `lucene/benchmark/src/test` (90 methods) are ported in `crates/rucene-benchmark/tests/`, with `conf/*.alg` and the test resources shipped as test resources, and pass (rmp #633).
- The benchmark Java oracle (`tests/benchmark_oracle.rs`, `BenchmarkOracle.java`) gives equal `Algorithm.toString()` for every `conf/*.alg`, `DemoHTMLParser` output, TREC and line-doc parsing, report text (timings masked) and root spellout.
