# Rucene — Testing and Quality Specification

**Spec ID:** TEST  
**Scope:** Unit tests, integration tests, portability tests, fuzzing, benchmarks, and CI/quality gates.

## 1. Test pyramid

Every module ships with:

1. **Unit tests** in the same file (`#[cfg(test)] mod tests`) for individual structs and functions.
2. **Integration tests** under `tests/` that exercise end-to-end workflows (index, search, commit).
3. **Portability tests** that compare Rucene output with reference Java Lucene 10.5.0 index trees.
4. **Property-based / fuzzing tests** for parsers and data structures that consume untrusted bytes.

## 2. Portability tests

Portability tests are first-class citizens. They must:

- Use reference indexes produced by pre-compiled Java Lucene 10.5.0 artefacts.
- Compare complete directory trees byte-by-byte.
- Cover round-trips: Java → Rucene → Java and Rucene → Java → Rucene.
- Include edge cases (empty index, single segment, multiple segments, deleted docs, merges).

## 3. Benchmarks

- Benchmark harness lives under `benches/` and uses `criterion` with async Tokio support.
- Benchmarks compare indexing throughput, query latency (p50/p99), and memory usage against Java Lucene 10.5.0.
- The target is to be **equal or better** than Java Lucene 10.5.0 across all dimensions.
- Benchmarks run per release on a documented hardware/OS configuration.

## 4. Fuzzing

- `proptest` is used from the first sprint for structures such as `BytesRef`, bit sets, packed ints, and numeric encoders.
- Dedicated fuzz targets are added for every parser that reads external index bytes.
- Every crash is converted into a regression test before the bug is considered fixed.

## 5. Quality gates

Before any task is closed:

- `cargo test` passes.
- `cargo fmt` produces no changes.
- `cargo clippy` passes with no warnings.
- Unit and integration tests cover new code.
- Portability tests are added or updated when index-file behavior changes.
- `rustdoc` is written for every public item.
- A mandatory human review confirms acceptance criteria.

## 6. CI strategy

CI is **local first**. The project uses `just` or shell scripts to run the quality gates locally. A GitHub Actions workflow may be added later when the test suite stabilizes.

## 7. Fixture management

- Fixture generation scripts are under `tests/fixtures/`.
- Generated reference indexes are stored in a versioned location (e.g., LFS or pinned release artefacts).
- Fixture README documents how to regenerate them for a new Lucene version.
