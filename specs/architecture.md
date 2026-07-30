# Rucene — Architecture Specification

**Spec ID:** ARCH  
**Scope:** Rust crate structure, public API design, async model, core abstractions, and error handling.

## 1. Crate structure

Rucene is a **single Cargo crate** (`rucene`). Modules inside `src/` mirror Lucene Core's Java package names:

| Java package | Rust module | Cargo feature |
|--------------|-------------|---------------|
| `org.apache.lucene.util` | `rucene::util` | `util` |
| `org.apache.lucene.store` | `rucene::store` | `store` |
| `org.apache.lucene.document` | `rucene::document` | `document` |
| `org.apache.lucene.analysis` | `rucene::analysis` | `analysis` |
| `org.apache.lucene.codecs` | `rucene::codecs` | `codecs` |
| `org.apache.lucene.index` | `rucene::index` | `index` |
| `org.apache.lucene.search` | `rucene::search` | `search` |

The default feature set enables all modules. Downstream users can opt out of higher-level modules if they only need lower-level building blocks.

## 2. Public API design

- **Names:** Prefer Java Lucene class and method names converted to Rust naming conventions (`PascalCase` types, `snake_case` methods). Where a direct conversion would clash with a Rust keyword or reserved name, append an underscore suffix or use a closely related name, and document the mapping.
- **Organization:** Keep the same package/module hierarchy to make side-by-side comparison easy.
- **Idioms:** Use Rust `Result`, `Option`, iterators, and builders where they improve ergonomics without obscuring the Lucene equivalent.

## 3. Async model

- The **public API is fully async** and built on `tokio`.
- CPU-bound work inside `IndexSearcher` is kept on `tokio` tasks; if profiling shows contention, a dedicated blocking pool may be introduced later behind the same async API.
- `Directory` I/O is async at the public API layer, even when the underlying OS calls are synchronous.
- Backpressure is modeled explicitly through bounded channels, semaphores, or configurable in-flight operation caps.
- Cancellation completes the current atomic unit (e.g., a single document add, a single segment read) before returning, leaving the index in a recoverable state.

## 4. Core abstractions

Core Lucene abstractions are modeled as **generic traits with associated types** whenever possible:

- `Directory`
- `IndexInput`
- `IndexOutput`
- `Analyzer`
- `Codec`

`dyn` trait objects are reserved for extension points where runtime polymorphism is required (e.g., user-provided codecs or analyzers registered in a plugin registry).

## 5. Error handling

All recoverable errors are returned as `Result<T, LuceneError>`:

- `LuceneError::Io` wraps `std::io::Error`.
- `LuceneError::CorruptIndex` maps Java `CorruptIndexException`.
- `LuceneError::IndexFormatNotSupported` maps unsupported format exceptions.
- `LuceneError::IllegalArgument` / `LuceneError::IllegalState` map the Java equivalents.
- `LuceneError::ResourceLimit` captures security/resource-boundary violations.
- `LuceneError::Cancelled` signals async cancellation.

`panic!` is not used for error reporting in library code.

## 6. Concurrency model

- `IndexWriter` allows multiple concurrent document operations coordinated internally, matching Java Lucene semantics.
- `Directory` implementations use asynchronous locking primitives (`tokio::sync`) because the public API is async.
- The multi-reader/single-writer invariant is enforced by `IndexWriter` and `Directory` together.
- Segment merges run as background `tokio` tasks and are cancellable when backpressure rises.
- Lucene's per-thread buffers and contexts are modeled with OS thread-local storage.

## 7. Memory management

- Buffer and transient object reuse is implemented via fixed-size object pools.
- Hard memory ceilings are configurable per query and per server.
- Memory-mapped I/O is provided through `memmap2` only for indexes explicitly marked as trusted.

## 8. Unsafe policy

- `#![deny(unsafe_code)]` is enabled at the crate root.
- Any future use of `unsafe` requires a written justification, a security review, and explicit removal of the deny directive for the affected module.
