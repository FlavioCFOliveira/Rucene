# Module Specification: `memory`

**Spec ID:** MEMORY  
**Java module:** `lucene/memory` (package `org.apache.lucene.index.memory`)  
**Crate:** `crates/rucene-memory`

## 1. Purpose

Provide a high-performance single-document main memory full-text search index: index the fields of one document in memory and run queries against it without a `Directory`, as Lucene 10.5.0 `MemoryIndex` does.

## 2. Key classes / concepts ported

- `MemoryIndex` — the single-document index: `addField` (text through an analyzer, token streams, `IndexableField`s with doc values and points), `search(Query)`, `createSearcher()`, `toStringDebug()`, `reset()`, `freeze()`.
- `MemoryIndex.MemoryIndexReader` — the `LeafReader` over the document: terms, postings with positions/offsets/payloads, norms, doc values, points, field infos.
- `SlicedIntBlockPool` / `SliceWriter` / `SliceReader` — the int-block slices holding the postings.

## 3. Design notes

- One crate module per Java source file (`MemoryIndex.java` is `memory_index`); the types of the package sit at the crate root.
- Offsets count UTF-16 units, as in Java.
- `IndexableField.tokenStream` returning `null` for custom fields is reached through the defaulted `token_stream_or_null` of core's `IndexableField`.

## 4. Acceptance criteria

- The 3 Lucene 10.5.0 test classes (`TestMemoryIndex`, `TestMemoryIndexAgainstDirectory`, `TestSlicedIntBlockPool`, 42 methods) are ported in `crates/rucene-memory/tests/` and pass.
- The `MemoryIndex` oracle (`memory_index_oracle.rs`) gives score bits and `toStringDebug()` equal to Lucene 10.5.0 on seeded documents and queries.
