# Rucene

Rucene is a port of **Apache Lucene Core** to Rust.
This port targets **Apache Lucene Core 10.5.0** exclusively.

## Crate Library (Rust)

This project is a Rust crate that ports Apache Lucene Core to a library crate.

## Port parity

Whenever possible, this project aims to provide absolute parity with Apache Lucene Core along two dimensions:

1. **Functional Parity** — same functionality and same modular organization, only in a different language (better performance, better memory management).
2. **100% Index Compatibility** — this crate must be able to **read and write index files** that are 100% compatible with Apache Lucene Core 10.5.0.

## Project structure

- `src/` — Rust source code, mirroring Lucene Core's package layout.
- `specs/` — formal specifications for architecture, compatibility, security, testing, and each module.
- `tests/` — integration and portability tests.
- `benches/` — performance benchmarks against Java Lucene 10.5.0.

## Getting started

```bash
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features
```

## Reference

- Apache Lucene 10.5.0 source: <https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/core>
- Apache Lucene 10.5.0 Javadoc: <https://lucene.apache.org/core/10_5_0/core/index.html>
