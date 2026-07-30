# Module Specification: `codecs`

**Spec ID:** CODEC  
**Java package:** `org.apache.lucene.codecs`  
**Cargo feature:** `codecs`

## 1. Purpose

Implement the Lucene codec framework and the default `Lucene104` codec so that Rucene reads and writes byte-compatible index files.

## 2. Key classes / concepts to port

- `Codec` — registry and abstract codec.
- `PostingsFormat`, `StoredFieldsFormat`, `TermVectorsFormat`, `DocValuesFormat`, `NormsFormat`, `FieldInfosFormat`, `SegmentInfoFormat`, `LiveDocsFormat`, `PointsFormat`, `VectorsFormat`.
- `Lucene104Codec` and all its bundled sub-format implementations.
- `CompressingStoredFieldsFormat` if used by the default codec.

## 3. Design notes

- Use generic traits with associated types for codec components.
- Register built-in codecs in an explicit plugin registry (no reflection).
- Only the default Lucene 10.5.0 codec is mandatory; legacy codecs are out of scope initially.

## 4. Acceptance criteria

- A Rucene-written index using `Lucene104Codec` is byte-identical to a Java Lucene 10.5.0 index for the same documents.
- Round-trip portability tests pass.
- Parser fuzz tests produce no panics on malformed codec footers/headers.
