# Rucene

Rucene is a port of **Apache Lucene Core** to Rust.
This port targets **Apache Lucene Core 10.5.0** exclusively.

## Crate Library (Rust)

This project is a Cargo workspace of Rust library crates. The root package,
`rucene`, ports Lucene Core (`lucene/core`).

## Port parity

The project aims at parity with Apache Lucene Core 10.5.0 along two dimensions:

1. **Functional parity** — the same functionality and the same modular organization, in a different language.
2. **Index compatibility** — reading and writing index files that are compatible with Apache Lucene Core 10.5.0.

## Project structure

- `src/` — the `rucene` crate (`lucene/core`). Its top-level modules mirror Lucene Core's packages: `analysis`, `codecs`, `document`, `geo`, `index`, `internal`, `search`, `store`, `util`, plus `error`.
- `crates/` — the other workspace members, one per ported module:
  - `rucene-codecs` — `lucene/codecs` (the `org.apache.lucene.codecs.*` formats outside core);
  - `randomizedtesting` — randomizedtesting 2.8.1 (`com.carrotsearch.randomizedtesting`), the runner of Lucene's tests;
  - `rucene-test-framework` — `lucene/test-framework`;
  - `rucene-analysis-common` — `lucene/analysis/common` (see [Analysis: `rucene-analysis-common`](#analysis-rucene-analysis-common)).
- `tests/` — integration tests, portability tests (`tests/portability/`), fuzz tests (`tests/fuzz/`), and the Java oracle harness (`tests/fixtures/java-codec-harness/`).
- `specs/` — specifications: overview, architecture, compatibility, security, testing, workflow, and one file per module (`specs/modules/`).
- `tools/` — development tooling, not part of any crate:
  - `tools/kg/` — Python scripts that build the project Knowledge Graph (see `tools/kg/README.md`);
  - `tools/gen/` — code generators (DFA tables, JDK collator and time-zone data) and the test-translation guide `J2R_GUIDE.md`.
- `knowledge-model.md` — the label and predicate model of the project Knowledge Graph.

## Getting started

Prerequisite: Rust 1.89 or later (`rust-version` of every workspace crate).

```bash
cargo build                                   # the rucene crate
cargo test                                    # tests of the rucene package only
cargo test --workspace                        # tests of every workspace crate
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
```

The root `Cargo.toml` declares no `default-members`, so `cargo build` and
`cargo test` without `--workspace` cover only the `rucene` package.

### Features

`default = ["std", "mmap"]`. The `zlib-c` feature links `flate2` against the C
zlib, which makes `BEST_COMPRESSION` stored fields byte-identical to Lucene's;
it requires a C compiler at build time. Without it the pure-Rust `zlib-rs`
backend is used. `--all-features` enables `zlib-c`, so the `clippy` command
above needs a C toolchain.

### Java-oracle tests

Many integration and portability tests, and oracle tests of `rucene-codecs`
and `rucene-test-framework`, compare Rucene with Lucene 10.5.0 by running
Java. They fail, rather than skip, when Java is missing. They need:

- **A JDK on `PATH`** (`java`). Lucene 10.5.0 requires Java 21 or later; the
  committed oracle fixtures were produced with JDK 25.
- **Maven on `PATH`** (`mvn`). The tests that drive the harness in
  `tests/fixtures/java-codec-harness/` run `mvn -q compile exec:java` in that
  directory; its `pom.xml` depends on `lucene-core`, `lucene-demo`,
  `lucene-analysis-common` and `lucene-queryparser` 10.5.0.
- **The Lucene jars in the local Maven repository.** The tests that run
  `java` directly read `$HOME/.m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar`;
  the oracle tests of `rucene-codecs` and `rucene-test-framework` also read
  `lucene-codecs` and `lucene-test-framework` 10.5.0,
  `randomizedtesting-runner` 2.8.1, `junit` 4.13.1 and `hamcrest` 2.2 from the
  same repository.

The vector oracles pin `lucene.useScalarFMA=false` on both sides themselves:
they pass `-Dlucene.useScalarFMA=false` to the JVM and set the environment
variable `lucene.useScalarFMA`, this port's form of that system property. No
manual setting is needed.

## Analysis: `rucene-analysis-common`

The crate `crates/rucene-analysis-common` ports Lucene 10.5.0
`lucene/analysis/common`: the analyzers, tokenizers, token filters and char
filters for many languages (the per-language packages, Snowball, hunspell,
compound words and hyphenation, synonyms, n-grams, shingles, patterns,
`CustomAnalyzer`, collation, and the rest), with the module's 130 SPI
factories. Its module tree mirrors the Java packages:
`org.apache.lucene.analysis.*` under `analysis` (the Java package
`analysis.in` is `analysis::r#in`, `in` being a Rust keyword),
`org.apache.lucene.collation.*` under `collation` and
`org.tartarus.snowball.*` under `tartarus`. Every type of the module carries
its port claim (`Equivalent to ...`).

### Using the analyzers and factories

An analyzer is used as its Java class is: `CatalanAnalyzer::new()`,
`CatalanAnalyzer::with_stop_words(set)` and
`CatalanAnalyzer::with_stem_exclusion_set(set, &exclusions)` are the three
Java constructors, and the `rucene::analysis::Analyzer` trait gives
`token_stream` and `normalize`.

The factories are found through `lucene-core`'s SPI entry points once the
crate is put "on the class path", as a Java application does by adding the
jar: reload the core registries with the crate's class loader, once per
process.

```rust
use rucene::analysis::factories::{
    reload_char_filters, reload_token_filters, reload_tokenizers, token_filter_for_name,
};
use rucene_analysis_common::spi;

reload_char_filters(spi::class_loader())?;
reload_token_filters(spi::class_loader())?;
reload_tokenizers(spi::class_loader())?;
// then `tokenizer_for_name`, `token_filter_for_name`, `char_filter_for_name`,
// the `*_lookup_class` functions and the `available_*` lists find every
// factory under its Lucene SPI name (case-insensitively), and
// `CustomAnalyzer::builder()` builds chains from them.
```

### JDK emulation

The module calls JDK behaviour that `rucene` does not emulate (locale-aware
case mapping and date parsing, `java.nio.charset` decoders,
`BreakIterator`, `Collator`, `Character.UnicodeBlock`, `java.util.regex`
matching, SAX parsing, and others). The crate's `jdk` module reproduces the
part of JDK 25 the module calls, from the JDK's specification and verified
against a JDK 25 oracle; no JDK code or data is copied (the JDK is GPL with
the Classpath Exception). Each emulation states at its point where it
differs from the JDK. The hyphenation pattern files are read with
`quick-xml` under a SAX-shaped reader.

**Thai dictionary.** `ThaiTokenizer` segments words with the JDK's Thai
dictionary, which is not shipped in this crate: it is read at run time, once
per process, from the run-time image (`lib/modules`) of a JDK installed on
the machine — `$JAVA_HOME` first, then the JDK of the `java` executable on
`PATH`. When neither is readable, the tokenizer falls back to the Thai word
list of ICU 79.1, embedded under the Unicode License V3; that list differs
from the JDK's, so Thai text can then split into different words than on
Lucene (see `jdk::break_dictionary`).

### Tests

The crate's tests port Lucene's test classes of the module onto
`rucene-test-framework` (one suite per Java class, run by the ported
randomized runner), with the test resources copied byte for byte under
`crates/rucene-analysis-common/tests/resources/`. The regular per-language
test classes are generated by `tools/translate/lang_tests.py` into
`tests/analysis_lang_generated/` (`--check` verifies the committed output).

Its Java-oracle tests (`tests/analysis_oracle*.rs`, `tests/spi_registry.rs`
and the JDK oracles) run Lucene 10.5.0 and compare every token attribute.
They fail, rather than skip, when a requirement is missing:

- **A JDK 25 on `PATH`** (`java`): the oracles launch Java source files
  directly (no Maven), and the JDK oracles compare with JDK 25's behaviour.
  The few oracles that need a class compiled into a Lucene package run
  `java -m jdk.compiler/com.sun.tools.javac.Main`, so a JRE with the
  `jdk.compiler` module suffices.
- **The Lucene jars in the local Maven repository**:
  `$HOME/.m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar`
  and `lucene-analysis-common-10.5.0.jar` beside it.

## Reference

- Apache Lucene 10.5.0 source: <https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/core>
- Apache Lucene 10.5.0 Javadoc: <https://lucene.apache.org/core/10_5_0/core/index.html>
