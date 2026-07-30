# Rucene — Index-File Compatibility Specification

**Spec ID:** COMPAT  
**Scope:** File formats, codecs, headers, checksums, validation methodology, and portability tests.

## 1. Compatibility goal

Rucene must be able to:

1. **Read** index directory trees written by Apache Lucene Core 10.5.0 using its default codec.
2. **Write** index directory trees that Apache Lucene Core 10.5.0 can read back using its default codec.

The validation compares **complete directory trees** (file lists, names, headers, checksums, and payloads) produced by identical operations in both implementations.

## 2. Format scope

- **Default codec:** `Lucene104` and its bundled sub-formats (postings, stored fields, term vectors, doc values, points, vectors).
- **Legacy codecs:** not supported in the initial phase. Rucene may refuse to open indexes written by older codecs, matching the stance that only Lucene 10.5.0 native formats are targeted.
- **Format versions:** exact versions used by Java Lucene Core 10.5.0; no deviation is permitted without a portability test proving interchange.

## 3. Byte-level requirements

The following metadata must match byte-for-byte for the same logical operation:

- File names and directory layout (`segments_N`, `.si`, `.fdx`, `.fdt`, `.tim`, `.tip`, `.doc`, `.pos`, `.pay`, `.dvd`, `.dvm`, `.vec`, `.vem`, etc.).
- File headers and version magic numbers.
- Checksum algorithms (`CRC32` / `CRC32C` as used by Lucene).
- Segment info (`SegmentInfo`) serialization.
- Codec footer format.

Non-deterministic elements (timestamps, segment generation counters, checksum random salts) are compared after normalization or excluded from strict byte comparison as documented per file type.

## 4. Validation methodology

1. Generate reference indexes using the official Apache Lucene Core 10.5.0 distribution or pre-compiled artefacts.
2. Perform the same indexing operation in Rucene.
3. Compare the resulting directory trees file by file using a deterministic binary diff harness.
4. Run round-trip tests: Java → Rucene → Java and Rucene → Java → Rucene.

## 5. Reference data

- Reference indexes are produced from pre-compiled Java Lucene 10.5.0 artefacts.
- Fixture generation scripts live under `tests/fixtures/` and are documented in `tests/fixtures/README.md`.
- Fixtures are versioned alongside the crate so that regression tests remain reproducible.

## 6. Durability semantics

Rucene must match Java Lucene's durability semantics:

- `IndexWriter.commit()` follows the same two-phase commit and `fsync` ordering.
- Partial writes and crashes leave the index in a recoverable state consistent with Java Lucene behavior.
- File metadata operations are atomic across all `Directory` implementations.

## 7. Acceptance criteria

A module is considered compatible when:

- At least one round-trip portability test passes for its primary output files.
- `cargo test` includes a test that fails if byte output diverges from the reference fixture.
