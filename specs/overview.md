# Rucene — Project Specification Overview

**Version:** 0.1.0  
**Target reference:** Apache Lucene Core 10.5.0  
**Reference URLs:**
- Source tree: <https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/core>
- Documentation / Javadoc: <https://lucene.apache.org/core/10_5_0/core/index.html>

## 1. Purpose

Rucene is a Rust port of **Apache Lucene Core 10.5.0**. It is published as a single Cargo crate that mirrors Lucene Core's modular organization so that contributors can navigate directly between the Java reference and the Rust implementation.

The project pursues two dimensions of parity, whenever technically feasible:

1. **Functional parity** — same search/indexing behavior and same public API surface, expressed in Rust.
2. **100% index-file compatibility** — Rucene must be able to read index files produced by Java Lucene Core 10.5.0 and write index files that Java Lucene Core 10.5.0 can read back.

## 2. Scope

- **In scope:** all public classes and interfaces of `org.apache.lucene.core` (packages under `org.apache.lucene.*` in `lucene/core`).
- **Out of scope:** modules outside `lucene/core`, including `lucene/demo`, `lucene/facet`, `lucene/highlighter`, etc. The `lucene/demo` examples may be ported later as separate examples, but they are not part of the Core port.

## 3. High-level decisions

| Topic | Decision |
|-------|----------|
| Rust edition / MSRV | 2021 / 1.80+ |
| Crate structure | Single crate with Cargo features per module |
| API style | Mirror Java Lucene class/method names and organization as closely as idiomatic Rust permits |
| Async model | Public API is fully async with `tokio`; backpressure and cancellation required from the start |
| Directory I/O | First-class Rust implementations; `mmap` via `memmap2` for trusted indexes; buffered I/O otherwise |
| Core abstractions | Generic traits with associated types where possible; `dyn` trait objects only where runtime polymorphism is required |
| Error handling | `Result`-based `LuceneError` hierarchy; selective mapping of important Java exceptions |
| `unsafe` | Prohibited by default; only allowed with explicit justification and review |
| Dependencies | Minimal; prefer `std` + hand-picked essential crates (`tokio`, `log`, `thiserror`, optional `memmap2`) |
| Compatibility | Read and write full index trees byte-compatible with Lucene 10.5.0; compare complete directory trees |
| Codecs | Default Lucene 10.5.0 codec (`Lucene104`) only; no legacy codec support in the initial phase |
| Similarity / scoring | `BM25Similarity` and `ClassicSimilarity` |
| Directory implementations | All standard ones: `RAMDirectory`, `MMapDirectory`, `FSDirectory`, `NIOFSDirectory`, `SimpleFSDirectory` |
| Merge policies | All standard policies (`TieredMergePolicy`, `LogByteSizeMergePolicy`, `LogDocMergePolicy`) |
| Query parser | Classic `QueryParser` only |
| NRT search | Supported from the start (`IndexWriter.getReader()`) |
| Test fixtures | Pre-compiled Java Lucene 10.5.0 artefacts used to generate reference indexes |
| Benchmarks | Compare against Java Lucene 10.5.0; run per release |
| Fuzzing | Property-based / fuzzing tests from the start for critical data structures |
| Documentation | Central `specs/` directory + complete `rustdoc` + `README` |
| Reviews | Mandatory human review before closing any task |
| Traceability | Specification sections carry IDs referenced in code comments, tests, and `rmp` tasks |

## 4. Module porting order

The initial sprints follow Lucene Core's dependency order, breaking Java package cycles at the Rust module boundary by introducing traits and stub types where needed:

1. `util`
2. `store`
3. `document`
4. `analysis`
5. `codecs`
6. `index`
7. `search`

## 5. Success criteria

A Rucene release is considered functionally ready when:

- All public APIs in scope have a Rust counterpart documented with their Lucene equivalent.
- `cargo test`, `cargo fmt`, and `cargo clippy` pass cleanly.
- Portability tests demonstrate that Rucene reads and writes index trees that Java Lucene 10.5.0 accepts.
- No `unsafe` is present unless explicitly approved and documented.

## 6. References

- `specs/architecture.md` — Rust architecture, async model, traits, and crate layout.
- `specs/compatibility.md` — index-file compatibility and portability testing.
- `specs/security.md` — threat model, resource limits, trusted/untrusted indexes.
- `specs/testing.md` — unit, integration, fuzzing, and benchmark strategy.
- `specs/workflow.md` — task lifecycle, acceptance criteria, and traceability.
- `specs/modules/*.md` — per-module detailed specifications.
