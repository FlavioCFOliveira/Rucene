# Module Specification: `store`

**Spec ID:** STORE  
**Java package:** `org.apache.lucene.store`  
**Cargo feature:** `store`

## 1. Purpose

Abstract all index file I/O through `Directory`, `IndexInput`, and `IndexOutput`, and provide the concrete implementations required for Lucene 10.5.0 index-file compatibility.

## 2. Key classes / concepts to port

- `Directory` — directory abstraction.
- `IndexInput` — sequential/random-access file reader.
- `IndexOutput` — file writer with checksum.
- `DataInput` / `DataOutput` — primitive type read/write (may live in `util` but are used here).
- `FSDirectory` — base for file-system directories.
- `MMapDirectory` — memory-mapped directory for trusted indexes.
- `NIOFSDirectory` — NIO-based file access.
- `SimpleFSDirectory` — simple buffered file access.
- `RAMDirectory` — in-memory directory for tests and small indexes.
- `LockFactory` / `Lock` — index locking abstractions.
- `IOContext` — hints for read/write behavior.
- `ChecksumIndexInput` / `ChecksumIndexOutput` — transparent checksum wrapping.

## 3. Design notes

- Public API is async; implementations may use synchronous OS calls internally.
- Use async locking primitives (`tokio::sync`) because the API surface is async.
- `mmap` is gated behind the `mmap` Cargo feature and only enabled for trusted indexes.
- All directory types are first-class Rust implementations from the start.

## 4. Acceptance criteria

- All directory types compile and pass unit tests.
- A RAMDirectory round-trip test writes and reads bytes correctly.
- Checksum behavior matches Lucene 10.5.0 for the same payload.
