# Module Specification: `index`

**Spec ID:** INDEX  
**Java package:** `org.apache.lucene.index`  
**Cargo feature:** `index`

## 1. Purpose

Implement the indexing engine: document ingestion, segment creation, merging, commits, and near-real-time readers.

## 2. Key classes / concepts to port

- `IndexWriter` — main indexing entry point.
- `IndexWriterConfig` — configuration builder.
- `IndexReader` / `DirectoryReader` / `SegmentReader`.
- `IndexCommit` / `IndexDeletionPolicy`.
- `SegmentInfos` / `SegmentInfo` / `SegmentCommitInfo`.
- `MergePolicy` / `MergeScheduler` / `MergeTrigger`.
- `CodecReader` / `StoredFieldsReader` / `NormsProducer` / `DocValuesProducer`.
- `LiveDocs` / `Bits` for deleted documents.

## 3. Design notes

- Public API is async; multiple concurrent document operations are coordinated internally like Java Lucene.
- Merge scheduler runs as cancellable `tokio` background tasks.
- Durability semantics (fsync, two-phase commit) match Java Lucene byte-for-byte.
- Graceful shutdown flushes pending documents and finishes active merges.

## 4. Acceptance criteria

- `IndexWriter` can create an index, add documents, commit, and close.
- Committed indexes are readable by Java Lucene 10.5.0.
- NRT reader returns a reader that sees recent uncommitted changes.
- Merge policies produce the same segment layout as Java Lucene for deterministic inputs.
