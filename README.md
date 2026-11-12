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
  - `rucene-test-framework` — `lucene/test-framework`.
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
cargo test --workspace                        # tests of all four workspace crates
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

## Reference

- Apache Lucene 10.5.0 source: <https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/core>
- Apache Lucene 10.5.0 Javadoc: <https://lucene.apache.org/core/10_5_0/core/index.html>
