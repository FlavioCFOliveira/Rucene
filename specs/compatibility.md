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

## 8. Java codec harness

The reusable reference-index generator lives under
`tests/fixtures/java-codec-harness`. It is a Maven project that depends on
Apache Lucene Core 10.5.0 (plus `lucene-demo`, `lucene-analysis-common` and
`lucene-queryparser`) and writes deterministic indexes using `IndexWriter` with
`Lucene104Codec`.

### 8.1 Manual usage

```bash
cd tests/fixtures/java-codec-harness
mvn -q compile exec:java \
  -Dexec.mainClass=org.apache.lucene.rucene.codec.CodecIndexWriter \
  -Dexec.args="/tmp/rucene-ref-index text"
```

The second argument is the *document shape*. Supported shapes:

| Shape        | Fields exercised                              |
| ------------ | --------------------------------------------- |
| `text`       | `StringField`, `TextField`                    |
| `docvalues`  | numeric, sorted, sorted-set, binary doc-values |
| `points`     | `IntPoint`, `LongPoint`, `FloatPoint`, `DoublePoint` |
| `vectors`    | `KnnFloatVectorField`                         |
| `stored`     | `StoredField`                                   |
| `termvectors`| text field with term vectors enabled          |

### 8.2 Adding a new shape

1. Open `tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/CodecIndexWriter.java`.
2. Add a new `case` in `writeDocuments` and a new `write*Documents` helper that
   adds deterministic documents to the `IndexWriter`.
3. Keep the number of documents and field values deterministic so that the
   generated index is byte-identical across runs.
4. Add the shape name to the integration test in `tests/portability/codecs.rs`
   (`java_harness_supports_all_document_shapes`) so it is exercised by `cargo test`.
5. When Rucene can write the same shape, add a round-trip test that calls
   `assert_directories_equal` to compare the Java reference tree with the
   Rucene-produced tree.

### 8.3 Automated invocation

The integration test target `portability_codecs` compiles and runs the harness
automatically:

```bash
cargo test --test portability_codecs
```

`portability_codecs` validates the reference side only: the harness produces a
deterministic, well-formed Lucene index for every supported shape. It makes no
byte comparison of whole directories, because the files carry per-segment
random ids; its helper `assert_directories_equal` is unused.

Rucene's output is compared with Lucene's by these tests:

| Test file | What it compares byte for byte with Lucene 10.5.0 |
|---|---|
| `tests/portability/stored_fields.rs` | the stored-fields files (`.fdt`, `.fdx`, `.fdm`) |
| `tests/portability/term_vectors.rs` | the term-vectors files |
| `tests/portability/norms.rs` | the norms files |
| `tests/portability/doc_values.rs` | the doc-values files (`.dvd`, `.dvm`) |
| `tests/portability/points.rs` | the points files |
| `tests/portability/bkd_writer_layout.rs` | the files `BKDWriter` writes through `add()` and `finish()` |
| `tests/portability/vectors.rs` | the kNN-vectors files |
| `tests/portability/scalar_quantized_vectors.rs` | the files of `Lucene104ScalarQuantizedVectorsFormat` and its raw delegate |
| `tests/portability/indexing_chain.rs` | the postings files |
| `tests/portability/seal_bytes.rs` | the three files that seal a flushed segment: `.fnm`, the compound file and `.si` |

Whole indexes are exchanged by `tests/portability/index_interop.rs`: Rucene and
Lucene write the same corpus, each side opens the other's index, runs
`CheckIndex` at its maximum level and runs the same query set, and the reports
must agree.

The formats of `lucene/codecs` are compared by
`crates/rucene-codecs/tests/*_oracle.rs`. Their `*writes_lucenes_bytes` tests
check that Rucene writes the same file listing as Lucene and every file of the
format under test byte for byte, once Lucene's segment id is replaced by
Rucene's (`crates/rucene-codecs/tests/oracle/mod.rs`,
`assert_writes_lucenes_bytes`).
