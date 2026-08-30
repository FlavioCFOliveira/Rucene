//! Defensive fuzz-style tests for the KNN-vectors reader.
//!
//! Everything the reader consumes comes straight off disk and is therefore
//! untrusted. Lucene's own decoder answers corruption with an exception —
//! `CorruptIndexException`, `EOFException`, `ArrayIndexOutOfBoundsException` —
//! or, where Java's arithmetic wraps, with a nonsensical value; none of them
//! aborts the JVM. A Rust port must match that: an `Err`, or a wrapped value
//! where Java wraps, but never a panic and never an allocation sized by an
//! attacker-controlled length.
//!
//! These tests take a valid segment written through
//! [`VectorValuesConsumer`](rucene::index::vector_values_consumer::VectorValuesConsumer)
//! and the real `Lucene99HnswVectorsFormat`, corrupt it systematically, and
//! assert exactly that. They are the reader-side counterpart of
//! `tests/portability/vectors.rs`, which proves the same files are the ones
//! Lucene writes.
//!
//! # Shapes, and why one is not enough
//!
//! The four files branch on shape in ways a single fixture cannot reach:
//!
//! * `dense` — every document has a vector, so `DocsWithFieldSet` stays dense
//!   and the metadata carries **no** ordinal-to-doc mapping at all;
//! * `sparse` — most documents have none, so the metadata gains the
//!   `DirectMonotonicWriter` mapping, which is the part with block shifts,
//!   per-block minima and a bit width read off disk;
//! * `byte` — the `BYTE` encoding, whose vector data is aligned to 4 bytes
//!   rather than 64 and whose per-value length is a different multiple of the
//!   dimension count;
//! * `multi` — two fields in one segment, the only shape where a corrupt
//!   offset or length in one field's entry can be made to point into the other
//!   field's region;
//! * `graph` — 648 vectors, one past the size at which
//!   `Lucene99HnswVectorsWriter` starts building an HNSW graph. It is the only
//!   shape whose `.vex` holds anything and whose `.vem` holds real graph level
//!   offsets, so it is the only one that can reach the graph decoder at all.
//!   Every fixture below it writes an empty graph, and a sweep over an empty
//!   graph proves nothing about the code that walks a real one.
//!
//! # The read has to go far enough to touch what was corrupted
//!
//! Iterating the values touches the `.vec` and the `.vemf`; nothing but a
//! **search** touches the `.vex` and the graph half of the `.vem`. Every sweep
//! here therefore runs both, and the `graph` fixture is what makes the search
//! traverse a graph rather than fall back to an exhaustive scan.
//!
//! # Budget
//!
//! The metadata files are hundreds of bytes, so they are swept with every one
//! of the 255 other values at every byte. The data files are kilobytes and are
//! mostly packed vector components — a byte that names nothing behaves the same
//! for every value it can take — so they get the boundary values a length, a
//! count or a bit width can sit on. Truncation is swept at every length of
//! every file, because a short read is the cheapest corruption to produce and
//! the one most likely to reach an unchecked subtraction.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::codecs::codec_util;
use rucene::codecs::knn_vectors::{FieldVectorWriter, KnnVectorsReader};
use rucene::codecs::state::SegmentReadState;
use rucene::codecs::{Codec, Lucene104Codec};
use rucene::index::vector_values_consumer::VectorValuesConsumer;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, FieldInfos, IndexOptions, SegmentInfo,
    VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::knn::TopKnnCollector;
use rucene::search::{from_live_docs, NO_MORE_DOCS};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::{NoOutputInfoStream, StringHelper, Version};

/// The four files the default codec writes for a segment that has vectors.
const EXTENSIONS: [&str; 4] = ["vec", "vemf", "vex", "vem"];

/// The per-field suffix `PerFieldKnnVectorsFormat` gives every file, because
/// `Lucene104Codec` resolves every field to the same concrete format.
const SUFFIX: &str = "Lucene99HnswVectorsFormat_0";

fn file_name(extension: &str) -> String {
    format!("_0_{SUFFIX}.{extension}")
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// One field of a fixture: its schema and the documents that carry a value.
struct FieldSpec {
    name: &'static str,
    number: i32,
    dimension: i32,
    encoding: VectorEncoding,
    similarity: VectorSimilarityFunction,
    /// `(doc id, vector)`, in strictly increasing doc order.
    values: Vec<(i32, Vec<f32>)>,
}

struct Fixture {
    name: &'static str,
    max_doc: i32,
    fields: Vec<FieldSpec>,
}

fn deterministic(doc: i32, dim: i32, salt: i32) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let raw = ((doc * 7919) + (i * 104_729) + (salt * 15_485_863)) % 2003;
            (raw - 1000) as f32 / 8.0
        })
        .collect()
}

fn float_field(
    name: &'static str,
    number: i32,
    dimension: i32,
    similarity: VectorSimilarityFunction,
    values: Vec<(i32, Vec<f32>)>,
) -> FieldSpec {
    FieldSpec {
        name,
        number,
        dimension,
        encoding: VectorEncoding::FLOAT32,
        similarity,
        values,
    }
}

fn fixtures() -> Vec<Fixture> {
    let dense: Vec<(i32, Vec<f32>)> = (0..6).map(|doc| (doc, deterministic(doc, 2, 0))).collect();
    let sparse: Vec<(i32, Vec<f32>)> = vec![
        (1, deterministic(1, 3, 1)),
        (4, deterministic(4, 3, 1)),
        (9, deterministic(9, 3, 1)),
    ];
    let byte_values: Vec<(i32, Vec<f32>)> = (0..5)
        .map(|doc| {
            (
                doc,
                (0..3)
                    .map(|i| (((doc * 31 + i * 17) % 255) - 127) as f32)
                    .collect(),
            )
        })
        .collect();
    let graph: Vec<(i32, Vec<f32>)> = (0..648)
        .map(|doc| (doc, deterministic(doc, 2, 2)))
        .collect();

    vec![
        Fixture {
            name: "dense",
            max_doc: 6,
            fields: vec![float_field(
                "v",
                0,
                2,
                VectorSimilarityFunction::EUCLIDEAN,
                dense,
            )],
        },
        Fixture {
            name: "sparse",
            max_doc: 12,
            fields: vec![float_field(
                "v",
                0,
                3,
                VectorSimilarityFunction::DOT_PRODUCT,
                sparse.clone(),
            )],
        },
        Fixture {
            name: "byte",
            max_doc: 5,
            fields: vec![FieldSpec {
                name: "b",
                number: 0,
                dimension: 3,
                encoding: VectorEncoding::BYTE,
                similarity: VectorSimilarityFunction::EUCLIDEAN,
                values: byte_values,
            }],
        },
        Fixture {
            name: "multi",
            max_doc: 12,
            fields: vec![
                float_field(
                    "first",
                    0,
                    2,
                    VectorSimilarityFunction::EUCLIDEAN,
                    (0..12).map(|doc| (doc, deterministic(doc, 2, 3))).collect(),
                ),
                FieldSpec {
                    name: "second",
                    number: 1,
                    dimension: 4,
                    encoding: VectorEncoding::BYTE,
                    similarity: VectorSimilarityFunction::COSINE,
                    values: sparse
                        .iter()
                        .map(|(doc, _)| (*doc, vec![1.0, 2.0, 3.0, 4.0]))
                        .collect(),
                },
            ],
        },
        Fixture {
            name: "graph",
            max_doc: 648,
            fields: vec![float_field(
                "v",
                0,
                2,
                VectorSimilarityFunction::EUCLIDEAN,
                graph,
            )],
        },
    ]
}

fn field_info(spec: &FieldSpec) -> FieldInfo {
    FieldInfo::new_full(
        spec.name,
        spec.number,
        false,
        false,
        false,
        IndexOptions::NONE,
        DocValuesType::NONE,
        DocValuesSkipIndexType::NONE,
        -1,
        HashMap::new(),
        0,
        0,
        0,
        spec.dimension,
        spec.encoding,
        spec.similarity,
        false,
        false,
    )
    .expect("field info")
}

fn codec() -> Arc<dyn Codec> {
    Arc::new(Lucene104Codec::new())
}

fn segment_info(directory: &Arc<dyn Directory>, max_doc: i32, id: [u8; 16]) -> SegmentInfo {
    SegmentInfo::new(
        Arc::clone(directory),
        Version::LATEST,
        Some(Version::LATEST),
        "_0".to_string(),
        max_doc,
        false,
        false,
        codec(),
        HashMap::new(),
        id,
        HashMap::new(),
        Default::default(),
    )
    .expect("segment info")
}

/// Writes a valid segment for `fixture` through the vectors consumer, and
/// returns the field infos the reader needs — including the two per-field
/// attributes the consumer's codec stamped onto them.
fn build(fixture: &Fixture) -> (Arc<dyn Directory>, SegmentInfo, FieldInfos) {
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let id = StringHelper::random_id();
    // `maxDoc` is unset while documents are indexed, as it is in a real
    // `DocumentsWriterPerThread`; the consumer builds its writer at that point.
    let indexing_info = segment_info(&directory, -1, id);
    let infos: Vec<FieldInfo> = fixture.fields.iter().map(field_info).collect();

    {
        let mut consumer = VectorValuesConsumer::new(
            codec(),
            Arc::clone(&directory),
            indexing_info,
            Arc::new(NoOutputInfoStream),
        );
        for (spec, info) in fixture.fields.iter().zip(infos.iter()) {
            let writer = consumer.add_field(info).expect("add field");
            match writer {
                FieldVectorWriter::Float(mut writer) => {
                    for (doc, value) in &spec.values {
                        writer.add_value(*doc, value.clone()).expect("add value");
                    }
                }
                FieldVectorWriter::Byte(mut writer) => {
                    for (doc, value) in &spec.values {
                        let bytes: Vec<u8> = value
                            .iter()
                            .map(|component| *component as i8 as u8)
                            .collect();
                        writer.add_value(*doc, bytes).expect("add value");
                    }
                }
            }
        }
        consumer.flush(fixture.max_doc, None).expect("flush");
    }

    let read_info = segment_info(&directory, fixture.max_doc, id);
    let field_infos = FieldInfos::new(infos).expect("field infos");
    (directory, read_info, field_infos)
}

// ---------------------------------------------------------------------------
// Reading, corrupting and truncating
// ---------------------------------------------------------------------------

fn read_file(directory: &Arc<dyn Directory>, name: &str) -> Vec<u8> {
    let length = directory.file_length(name).expect("file length") as usize;
    let mut input = directory
        .open_input(name, &*DEFAULT_IO_CONTEXT)
        .expect("open input");
    let mut bytes = vec![0u8; length];
    input.read_bytes(&mut bytes, 0, length).expect("read bytes");
    bytes
}

fn write_file(directory: &Arc<dyn Directory>, name: &str, bytes: &[u8]) {
    let mut output = directory
        .create_output(name, &*DEFAULT_IO_CONTEXT)
        .expect("create output");
    output
        .write_bytes(bytes, 0, bytes.len())
        .expect("write bytes");
    output.close().expect("close output");
}

/// A fresh directory holding a copy of the segment with one byte replaced.
fn corrupt_copy(
    original: &Arc<dyn Directory>,
    file: &str,
    at: usize,
    value: u8,
) -> Arc<dyn Directory> {
    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    for extension in EXTENSIONS {
        let name = file_name(extension);
        let mut bytes = read_file(original, &name);
        if name == file {
            bytes[at] = value;
        }
        write_file(&corrupt, &name, &bytes);
    }
    corrupt
}

/// A fresh directory holding a truncated copy of one of the segment's files.
fn truncated_copy(original: &Arc<dyn Directory>, file: &str, length: usize) -> Arc<dyn Directory> {
    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    for extension in EXTENSIONS {
        let name = file_name(extension);
        let mut bytes = read_file(original, &name);
        if name == file {
            bytes.truncate(length);
        }
        write_file(&corrupt, &name, &bytes);
    }
    corrupt
}

/// A fresh directory whose `file` body is `body`, with a valid footer over it.
///
/// The metadata files are read through a checksumming input, so a plain byte
/// flip is ultimately reported as a checksum mismatch. That does not make the
/// sweeps vacuous — the decoder reads the entries before the footer is checked,
/// so it still sees the garbage — but it does mean a patched value never
/// reaches the structures built afterwards. Re-signing closes that gap.
fn resigned_copy(original: &Arc<dyn Directory>, file: &str, body: &[u8]) -> Arc<dyn Directory> {
    let signed: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    {
        let mut output = signed
            .create_output("resigned", &*DEFAULT_IO_CONTEXT)
            .expect("create");
        output
            .write_bytes(body, 0, body.len())
            .expect("write patched body");
        codec_util::write_footer(output.as_mut()).expect("footer");
        output.close().expect("close");
    }
    let resigned = read_file(&signed, "resigned");

    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    for extension in EXTENSIONS {
        let name = file_name(extension);
        let bytes = if name == file {
            resigned.clone()
        } else {
            read_file(original, &name)
        };
        write_file(&corrupt, &name, &bytes);
    }
    corrupt
}

/// Drives the reader over every field: integrity, every value, and a search.
///
/// Every failure is swallowed; the assertion this whole file makes is that
/// none of them is a panic or an abort.
fn read_everything(
    directory: &Arc<dyn Directory>,
    info: &SegmentInfo,
    field_infos: &FieldInfos,
    fixture: &Fixture,
) {
    let context = &*DEFAULT_IO_CONTEXT;
    let state = SegmentReadState::new(&**directory, info, field_infos, context);
    let Ok(mut reader) = codec().knn_vectors_format().fields_reader(&state) else {
        return;
    };
    let _ = reader.check_integrity();

    for spec in &fixture.fields {
        match spec.encoding {
            VectorEncoding::FLOAT32 => {
                if let Ok(values) = reader.get_float_vector_values(spec.name) {
                    if let Ok(mut iter) = values.iterator() {
                        while let Ok(doc) = iter.next_doc() {
                            if doc == NO_MORE_DOCS {
                                break;
                            }
                            let _ = values.vector_value(iter.index());
                        }
                    }
                }
            }
            VectorEncoding::BYTE => {
                if let Ok(values) = reader.get_byte_vector_values(spec.name) {
                    if let Ok(mut iter) = values.iterator() {
                        while let Ok(doc) = iter.next_doc() {
                            if doc == NO_MORE_DOCS {
                                break;
                            }
                            let _ = values.vector_value(iter.index());
                        }
                    }
                }
            }
        }

        // Only a search reaches the `.vex` and the graph half of the `.vem`.
        let Ok(mut accept_docs) = from_live_docs(None, fixture.max_doc) else {
            continue;
        };
        // A bounded visit budget. The sweep's question is whether a corrupt
        // graph can make the decoder abort, and the decoder is entered on the
        // first hop; letting the search visit every node of the 648-node
        // fixture on each of the hundreds of thousands of attempts would buy no
        // extra decoder path and would make this file the slowest test in the
        // crate. Lucene has the same knob, and its own reader applies one.
        let mut collector = TopKnnCollector::new(4, 64);
        match spec.encoding {
            VectorEncoding::FLOAT32 => {
                let target = vec![0.5f32; spec.dimension as usize];
                let _ = reader.search(spec.name, &target, &mut collector, &mut accept_docs);
            }
            VectorEncoding::BYTE => {
                let target = vec![3u8; spec.dimension as usize];
                let _ = reader.search_byte(spec.name, &target, &mut collector, &mut accept_docs);
            }
        }
    }
    let _ = reader.close();
}

/// The byte values the data-file sweeps try at each position.
///
/// Every boundary a length, a count, a bit width or a group-varint selector can
/// sit on, rather than all 255: the `.vec` is packed vector components and the
/// `.vex` is mostly delta-encoded node ids, and a byte that names nothing
/// behaves the same for every value it can take.
const BOUNDARY_VALUES: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x07, 0x0f, 0x10, 0x1f, 0x20, 0x40, 0x7f, 0x80, 0x81, 0xc0, 0xfe, 0xff,
];

/// The hostile values tried for each word of a metadata body.
fn hostile_values() -> Vec<i64> {
    vec![
        0,
        1,
        -1,
        2,
        -2,
        i32::MIN as i64,
        i32::MAX as i64,
        i64::MIN,
        i64::MAX,
        1 << 30,
        1 << 31,
        1 << 40,
        -(1 << 40),
        0x7fff_ffff_ffff,
    ]
}

// ---------------------------------------------------------------------------
// Sweeps
// ---------------------------------------------------------------------------

#[test]
fn every_value_at_every_byte_of_the_flat_metadata_is_survived() {
    let mut attempts = 0usize;
    for fixture in fixtures() {
        let (original, info, field_infos) = build(&fixture);
        let name = file_name("vemf");
        let valid = read_file(&original, &name);
        for (at, &original_byte) in valid.iter().enumerate() {
            for value in 0..=u8::MAX {
                if value == original_byte {
                    continue;
                }
                let corrupt = corrupt_copy(&original, &name, at, value);
                read_everything(&corrupt, &info, &field_infos, &fixture);
                attempts += 1;
            }
        }
    }
    assert!(attempts > 100_000, "the sweep barely ran: {attempts}");
}

#[test]
fn every_value_at_every_byte_of_the_graph_metadata_is_survived() {
    let mut attempts = 0usize;
    for fixture in fixtures() {
        let (original, info, field_infos) = build(&fixture);
        let name = file_name("vem");
        let valid = read_file(&original, &name);
        for (at, &original_byte) in valid.iter().enumerate() {
            for value in 0..=u8::MAX {
                if value == original_byte {
                    continue;
                }
                let corrupt = corrupt_copy(&original, &name, at, value);
                read_everything(&corrupt, &info, &field_infos, &fixture);
                attempts += 1;
            }
        }
    }
    assert!(attempts > 100_000, "the sweep barely ran: {attempts}");
}

#[test]
fn boundary_values_at_every_byte_of_the_vector_data_are_survived() {
    let mut attempts = 0usize;
    for fixture in fixtures() {
        let (original, info, field_infos) = build(&fixture);
        let name = file_name("vec");
        let valid = read_file(&original, &name);
        for (at, &original_byte) in valid.iter().enumerate() {
            for value in BOUNDARY_VALUES {
                if value == original_byte {
                    continue;
                }
                let corrupt = corrupt_copy(&original, &name, at, value);
                read_everything(&corrupt, &info, &field_infos, &fixture);
                attempts += 1;
            }
        }
    }
    assert!(attempts > 10_000, "the sweep barely ran: {attempts}");
}

#[test]
fn boundary_values_at_every_byte_of_the_graph_are_survived() {
    let mut attempts = 0usize;
    for fixture in fixtures() {
        let (original, info, field_infos) = build(&fixture);
        let name = file_name("vex");
        let valid = read_file(&original, &name);
        for (at, &original_byte) in valid.iter().enumerate() {
            for value in BOUNDARY_VALUES {
                if value == original_byte {
                    continue;
                }
                let corrupt = corrupt_copy(&original, &name, at, value);
                read_everything(&corrupt, &info, &field_infos, &fixture);
                attempts += 1;
            }
        }
    }
    assert!(attempts > 10_000, "the sweep barely ran: {attempts}");
}

#[test]
fn a_truncated_file_is_survived_at_every_length() {
    let mut attempts = 0usize;
    for fixture in fixtures() {
        let (original, info, field_infos) = build(&fixture);
        for extension in EXTENSIONS {
            let name = file_name(extension);
            let valid = read_file(&original, &name);
            for length in 0..valid.len() {
                let corrupt = truncated_copy(&original, &name, length);
                read_everything(&corrupt, &info, &field_infos, &fixture);
                attempts += 1;
            }
        }
    }
    assert!(attempts > 10_000, "the sweep barely ran: {attempts}");
}

#[test]
fn hostile_metadata_that_passes_its_checksum_is_survived() {
    // Every four- and eight-byte aligned word of each metadata body, given a
    // value chosen to be hostile to a length, a count or an offset, with the
    // footer re-signed so the reader accepts the file and the patched word
    // actually reaches the decoder. This is where an allocation sized off disk
    // shows up.
    let mut attempts = 0usize;
    for fixture in fixtures() {
        let (original, info, field_infos) = build(&fixture);
        for extension in ["vemf", "vem"] {
            let name = file_name(extension);
            let valid = read_file(&original, &name);
            let body_end = valid.len() - 8;
            for at in (0..body_end.saturating_sub(8)).step_by(4) {
                for value in hostile_values() {
                    for width in [4usize, 8] {
                        if at + width > body_end {
                            continue;
                        }
                        let mut body = valid[..body_end].to_vec();
                        if width == 4 {
                            body[at..at + 4].copy_from_slice(&(value as i32).to_be_bytes());
                        } else {
                            body[at..at + 8].copy_from_slice(&value.to_be_bytes());
                        }
                        let corrupt = resigned_copy(&original, &name, &body);
                        read_everything(&corrupt, &info, &field_infos, &fixture);
                        attempts += 1;
                    }
                }
            }
        }
    }
    assert!(attempts > 5_000, "the sweep barely ran: {attempts}");
}

/// Every fixture must read back exactly what was written, so that the sweeps
/// above are corrupting a segment that was right to begin with.
#[test]
fn every_fixture_reads_back_exactly() {
    for fixture in fixtures() {
        let (directory, info, field_infos) = build(&fixture);
        let context = &*DEFAULT_IO_CONTEXT;
        let state = SegmentReadState::new(&*directory, &info, &field_infos, context);
        let reader: Box<dyn KnnVectorsReader> = codec()
            .knn_vectors_format()
            .fields_reader(&state)
            .expect("reader");
        reader.check_integrity().expect("integrity");

        for spec in &fixture.fields {
            let mut seen: Vec<(i32, Vec<f32>)> = Vec::new();
            match spec.encoding {
                VectorEncoding::FLOAT32 => {
                    let values = reader
                        .get_float_vector_values(spec.name)
                        .expect("float values");
                    let mut iter = values.iterator().expect("iterator");
                    while iter.next_doc().expect("next doc") != NO_MORE_DOCS {
                        let value = values.vector_value(iter.index()).expect("value");
                        seen.push((iter.doc_id(), value));
                    }
                }
                VectorEncoding::BYTE => {
                    let values = reader
                        .get_byte_vector_values(spec.name)
                        .expect("byte values");
                    let mut iter = values.iterator().expect("iterator");
                    while iter.next_doc().expect("next doc") != NO_MORE_DOCS {
                        let value = values.vector_value(iter.index()).expect("value");
                        seen.push((
                            iter.doc_id(),
                            value.iter().map(|b| *b as i8 as f32).collect(),
                        ));
                    }
                }
            }
            assert_eq!(
                seen.len(),
                spec.values.len(),
                "{}/{}: value count",
                fixture.name,
                spec.name
            );
            for ((doc, value), (expected_doc, expected)) in seen.iter().zip(spec.values.iter()) {
                assert_eq!(doc, expected_doc, "{}/{}: doc id", fixture.name, spec.name);
                assert_eq!(value, expected, "{}/{}: value", fixture.name, spec.name);
            }
        }
    }
}
