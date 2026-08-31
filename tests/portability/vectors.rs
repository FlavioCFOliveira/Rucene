//! Portability tests for the KNN-vectors half of the indexing chain.
//!
//! Every test drives the same document table through Rucene's indexing chain
//! and through a Java harness that indexes the identical documents with Apache
//! Lucene Core 10.5.0, then asserts three ways:
//!
//! 1. the `.vec`, `.vemf`, `.vem` and `.vex` files the two sides write are
//!    **byte for byte identical**;
//! 2. Rucene reads Lucene's segment and decodes exactly the vectors Lucene's
//!    own reader decodes, for every document, in the same ordinal order;
//! 3. Lucene reads Rucene's segment through its real `PerFieldKnnVectorsFormat`
//!    reader and decodes exactly what it decodes from its own files.
//!
//! # The grid
//!
//! The cases span the shape dimensions of the format, not only its values,
//! because a byte difference can hide behind a shape:
//!
//! * **both encodings** — `FLOAT32`, whose vector data is padded to a 64-byte
//!   boundary, and `BYTE`, padded to 4;
//! * **all four similarity functions** — each is written as an ordinal into the
//!   per-field metadata, and `COSINE` and `MAXIMUM_INNER_PRODUCT` reach code
//!   the other two do not;
//! * **dimension counts at the edges** — 1, 16 (exactly one 64-byte float
//!   alignment unit, the only width that needs no padding) and 1024, the
//!   maximum `Lucene99HnswVectorsFormat` accepts;
//! * **dense and sparse fields** — a sparse field makes `DocsWithFieldSet`
//!   switch from "every document" to a bit set, which adds the
//!   `DirectMonotonicWriter` ordinal-to-doc mapping to the metadata; `lastonly`
//!   puts the single value at the very end of the doc-id space;
//! * **corpus sizes across the tiny-segment threshold** — `thresholdN` builds a
//!   segment of exactly N vectors, which is how the cut-off that decides
//!   whether an HNSW graph is built at all is *located* rather than assumed;
//! * **several fields in one segment** — `multi` uses names whose Java hash
//!   codes put them in a field-hash order that is neither the order they are
//!   first seen nor their field-number order, and `fieldorder` introduces the
//!   second field only in the second document, so the first-seen order and the
//!   alphabetical order disagree. Both pin the fact that this format's per-field
//!   entries follow **document-encounter order**, unlike doc values and points,
//!   whose entries follow the field-hash table;
//! * **a segment with no vectors at all** — `novec`, where the consumer must
//!   never create its writer and the segment must gain no vector file.
//!
//! A missing Maven or JDK is a hard failure, not a skip: a portability test has
//! nothing to assert without the reference implementation, so skipping would
//! report success while proving nothing. This matches the other tests under
//! `tests/portability/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::analysis::{Analyzer, StandardAnalyzer};
use rucene::codecs::knn_vectors::KnnVectorsReader;
use rucene::codecs::state::SegmentReadState;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::document::{Document, KnnByteVectorField, KnnFloatVectorField};
use rucene::index::documents_writer::{IndexingChain, IndexingChainFlushState};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::indexing_chain::DefaultIndexingChain;
use rucene::index::{FieldInfos, SegmentInfo, VectorEncoding, VectorSimilarityFunction};
use rucene::search::DocIdSetIterator;
use rucene::store::{
    flush_io_context, Directory, FSDirectory, FlushInfo, TrackingDirectoryWrapper,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::{NoOutputInfoStream, Version};

/// The four files a vectors segment is made of under the default codec: the
/// flat format's data and metadata, and the HNSW graph's index and metadata.
const EXTENSIONS: [&str; 4] = ["vec", "vemf", "vex", "vem"];

static HARNESS_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// The corpus, mirrored from VectorsFixture.java
// ---------------------------------------------------------------------------

/// One vector field of one document.
#[derive(Debug, Clone)]
enum Vec3 {
    Floats(&'static str, VectorSimilarityFunction, Vec<f32>),
    Bytes(&'static str, VectorSimilarityFunction, Vec<u8>),
}

/// The float vector `VectorsFixture.floatVector` builds, integer for integer.
///
/// The arithmetic is deliberately integral until the final division by 8, which
/// is exact in binary32, so both sides produce the same bits without depending
/// on any floating-point detail.
fn float_vector(doc: i32, dim: usize, salt: i32) -> Vec<f32> {
    let mut value = Vec::with_capacity(dim);
    for i in 0..dim {
        let raw = ((doc * 7919) + (i as i32 * 104_729) + (salt * 15_485_863)) % 2003;
        value.push((raw - 1000) as f32 / 8.0f32);
    }
    value[0] += 1.0;
    value
}

/// The byte vector `VectorsFixture.byteVector` builds.
fn byte_vector(doc: i32, dim: usize, salt: i32) -> Vec<u8> {
    let mut value = Vec::with_capacity(dim);
    for i in 0..dim {
        let raw = ((doc * 31) + (i as i32 * 17) + (salt * 7)) % 255;
        value.push((raw - 127) as i8 as u8);
    }
    if value[0] == 0 {
        value[0] = 1;
    }
    value
}

fn floats(name: &'static str, sim: VectorSimilarityFunction, v: Vec<f32>) -> Vec3 {
    Vec3::Floats(name, sim, v)
}

fn bytes(name: &'static str, sim: VectorSimilarityFunction, v: Vec<u8>) -> Vec3 {
    Vec3::Bytes(name, sim, v)
}

/// The documents of one case, mirroring `VectorsFixture.documents`.
fn documents(case: &str) -> Vec<Vec<Vec3>> {
    use VectorSimilarityFunction as S;
    let mut docs: Vec<Vec<Vec3>> = Vec::new();
    match case {
        "f32tiny" => {
            for doc in 0..8 {
                docs.push(vec![floats("v", S::EUCLIDEAN, float_vector(doc, 3, 0))]);
            }
        }
        "f32dense" => {
            for doc in 0..300 {
                docs.push(vec![floats("v", S::DOT_PRODUCT, float_vector(doc, 4, 0))]);
            }
        }
        "f32sparse" => {
            for doc in 0..400 {
                if doc % 3 == 0 {
                    docs.push(Vec::new());
                } else {
                    docs.push(vec![floats("v", S::COSINE, float_vector(doc, 2, 0))]);
                }
            }
        }
        "f32cosine" => {
            for doc in 0..150 {
                docs.push(vec![floats("v", S::COSINE, float_vector(doc, 5, 3))]);
            }
        }
        "f32mip" => {
            for doc in 0..150 {
                docs.push(vec![floats(
                    "v",
                    S::MAXIMUM_INNER_PRODUCT,
                    float_vector(doc, 6, 5),
                )]);
            }
        }
        "dim1" => {
            for doc in 0..200 {
                docs.push(vec![floats("v", S::EUCLIDEAN, float_vector(doc, 1, 0))]);
            }
        }
        "dim16" => {
            for doc in 0..200 {
                docs.push(vec![floats("v", S::EUCLIDEAN, float_vector(doc, 16, 0))]);
            }
        }
        "dim1024" => {
            for doc in 0..12 {
                docs.push(vec![floats("v", S::EUCLIDEAN, float_vector(doc, 1024, 0))]);
            }
        }
        "bytetiny" => {
            for doc in 0..8 {
                docs.push(vec![bytes("v", S::EUCLIDEAN, byte_vector(doc, 3, 0))]);
            }
        }
        "bytedense" => {
            for doc in 0..300 {
                docs.push(vec![bytes("v", S::EUCLIDEAN, byte_vector(doc, 8, 0))]);
            }
        }
        "bytesparse" => {
            for doc in 0..250 {
                if doc % 4 == 1 {
                    docs.push(Vec::new());
                } else {
                    docs.push(vec![bytes("v", S::DOT_PRODUCT, byte_vector(doc, 4, 1))]);
                }
            }
        }
        "bytecosine" => {
            for doc in 0..120 {
                docs.push(vec![bytes("v", S::COSINE, byte_vector(doc, 7, 2))]);
            }
        }
        "bytemip" => {
            for doc in 0..120 {
                docs.push(vec![bytes(
                    "v",
                    S::MAXIMUM_INNER_PRODUCT,
                    byte_vector(doc, 5, 4),
                )]);
            }
        }
        "multi" => {
            for doc in 0..140 {
                docs.push(vec![
                    floats("zeta", S::EUCLIDEAN, float_vector(doc, 3, 1)),
                    bytes("alpha", S::DOT_PRODUCT, byte_vector(doc, 4, 2)),
                    floats("mid", S::COSINE, float_vector(doc, 2, 3)),
                ]);
            }
        }
        "multisparse" => {
            for doc in 0..200 {
                let mut fields = Vec::new();
                if doc % 2 == 0 {
                    fields.push(floats("even", S::EUCLIDEAN, float_vector(doc, 3, 6)));
                }
                if doc % 3 == 0 {
                    fields.push(bytes("third", S::EUCLIDEAN, byte_vector(doc, 3, 7)));
                }
                docs.push(fields);
            }
        }
        "fieldorder" => {
            for doc in 0..120 {
                let mut fields = Vec::new();
                if doc > 0 {
                    fields.push(floats("a", S::EUCLIDEAN, float_vector(doc, 2, 8)));
                }
                fields.push(floats("b", S::EUCLIDEAN, float_vector(doc, 2, 9)));
                docs.push(fields);
            }
        }
        "one" => {
            docs.push(vec![floats("v", S::EUCLIDEAN, float_vector(0, 4, 0))]);
        }
        "lastonly" => {
            for doc in 0..130 {
                if doc == 129 {
                    docs.push(vec![floats("v", S::EUCLIDEAN, float_vector(doc, 3, 0))]);
                } else {
                    docs.push(Vec::new());
                }
            }
        }
        "novec" => {
            for _ in 0..20 {
                docs.push(Vec::new());
            }
        }
        other => {
            let count: i32 = other
                .strip_prefix("threshold")
                .unwrap_or_else(|| panic!("unknown case {other}"))
                .parse()
                .unwrap_or_else(|e| panic!("unknown case {other}: {e}"));
            for doc in 0..count {
                docs.push(vec![floats("v", S::EUCLIDEAN, float_vector(doc, 4, 0))]);
            }
        }
    }
    docs
}

// ---------------------------------------------------------------------------
// Java harness driver
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-codec-harness")
}

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "/usr/bin/mvn", "/usr/local/bin/mvn"] {
        if Command::new(candidate)
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    Err("Maven is not available on PATH".to_string())
}

/// Fails the test when Maven is unavailable.
///
/// A portability test proves compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("vectors portability tests require Maven and a JDK: {reason}");
    }
}

/// What the Java harness reports about the segment it committed.
#[derive(Debug)]
struct JavaSegment {
    name: String,
    id: [u8; 16],
    max_doc: i32,
    compound: bool,
    /// One `vec=` line per value Lucene's own reader decoded, in ordinal order
    /// per field.
    dump: Vec<String>,
    /// The vector half of the `.fnm` entry of every field, as Lucene committed
    /// it.
    field_infos: Vec<String>,
}

fn run_java(main_class: &str, args: &[String]) -> Result<String, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    if !harness.join("pom.xml").exists() {
        return Err(format!("pom.xml not found in {}", harness.display()));
    }
    let mvn = which_mvn()?;
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg(format!("-Dexec.mainClass={main_class}"))
        .arg(format!("-Dexec.args={}", args.join(" ")))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "{main_class} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    if !stdout.lines().any(|line| line.trim() == "read_ok=true") {
        return Err(format!("{main_class} did not finish:\n{stdout}"));
    }
    Ok(stdout)
}

fn run_java_fixture(out_dir: &Path, case: &str) -> Result<JavaSegment, String> {
    let stdout = run_java(
        "org.apache.lucene.rucene.codec.VectorsFixture",
        &[out_dir.display().to_string(), case.to_string()],
    )?;
    parse_segment(&stdout)
}

fn parse_segment(stdout: &str) -> Result<JavaSegment, String> {
    let mut name = None;
    let mut id = None;
    let mut max_doc = None;
    let mut compound = None;
    let mut dump = Vec::new();
    let mut field_infos = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("segment=") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("segment_id=") {
            id = Some(parse_id(value.trim())?);
        } else if let Some(value) = line.strip_prefix("max_doc=") {
            max_doc = Some(
                value
                    .trim()
                    .parse::<i32>()
                    .map_err(|e| format!("bad max_doc: {e}"))?,
            );
        } else if let Some(value) = line.strip_prefix("compound=") {
            compound = Some(value.trim() == "true");
        } else if let Some(value) = line.strip_prefix("fieldinfo=") {
            field_infos.push(value.trim().to_string());
        } else if line.starts_with("vec=") {
            dump.push(line.trim_end().to_string());
        }
    }
    Ok(JavaSegment {
        name: name.ok_or_else(|| format!("harness printed no segment name:\n{stdout}"))?,
        id: id.ok_or_else(|| format!("harness printed no segment id:\n{stdout}"))?,
        max_doc: max_doc.ok_or_else(|| format!("harness printed no max doc:\n{stdout}"))?,
        compound: compound.ok_or_else(|| format!("harness printed no compound flag:\n{stdout}"))?,
        dump,
        field_infos,
    })
}

fn parse_id(raw: &str) -> Result<[u8; 16], String> {
    if raw.len() != 32 {
        return Err(format!("unexpected segment id {raw:?}"));
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16)
            .map_err(|e| format!("bad segment id {raw:?}: {e}"))?;
    }
    Ok(bytes)
}

/// Reads the vectors Rucene wrote with the real Lucene reader, and returns the
/// same `vec=` lines `VectorsFixture` would have printed.
fn read_with_java(
    dir: &Path,
    segment_name: &str,
    segment_id: [u8; 16],
    max_doc: i32,
    field_infos: &FieldInfos,
) -> Result<Vec<String>, String> {
    let mut spec: Vec<String> = Vec::new();
    for info in field_infos.iter() {
        if info.vector_dimension == 0 {
            continue;
        }
        let format = info
            .get_attribute("PerFieldKnnVectorsFormat.format")
            .ok_or_else(|| {
                format!(
                    "field \"{}\" carries no PerFieldKnnVectorsFormat.format attribute, so the \
                     index Rucene wrote cannot be opened by Lucene",
                    info.name
                )
            })?;
        let suffix = info
            .get_attribute("PerFieldKnnVectorsFormat.suffix")
            .ok_or_else(|| {
                format!(
                    "field \"{}\" carries no PerFieldKnnVectorsFormat.suffix attribute",
                    info.name
                )
            })?;
        spec.push(format!(
            "{}:{}:{}:{}:{}:{}:{}",
            info.name,
            info.number,
            info.vector_dimension,
            match info.vector_encoding {
                VectorEncoding::BYTE => "BYTE",
                VectorEncoding::FLOAT32 => "FLOAT32",
            },
            similarity_name(info.vector_similarity_function),
            format,
            suffix
        ));
    }
    let spec = if spec.is_empty() {
        "-".to_string()
    } else {
        spec.join(",")
    };

    let stdout = run_java(
        "org.apache.lucene.rucene.codec.VectorsReaderFixture",
        &[
            dir.display().to_string(),
            segment_name.to_string(),
            hex(&segment_id),
            max_doc.to_string(),
            spec,
        ],
    )?;
    Ok(stdout
        .lines()
        .filter(|line| line.starts_with("vec="))
        .map(|line| line.trim_end().to_string())
        .collect())
}

fn similarity_name(similarity: VectorSimilarityFunction) -> &'static str {
    match similarity {
        VectorSimilarityFunction::EUCLIDEAN => "EUCLIDEAN",
        VectorSimilarityFunction::DOT_PRODUCT => "DOT_PRODUCT",
        VectorSimilarityFunction::COSINE => "COSINE",
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => "MAXIMUM_INNER_PRODUCT",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// The Rucene side
// ---------------------------------------------------------------------------

fn ensure_codec() -> Arc<dyn Codec> {
    let codec: Arc<dyn Codec> = Arc::new(Lucene104Codec::new());
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    codec
}

/// Drives the same documents through Rucene's indexing chain, into `out_dir`.
fn write_with_rucene(
    out_dir: &Path,
    segment_name: &str,
    segment_id: [u8; 16],
    documents: &[Vec<Vec3>],
) -> FieldInfos {
    let codec = ensure_codec();

    // The config requires an analyzer, but no field here is tokenized: a vector
    // field is never inverted, so the analyzer is never consulted.
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    let live = Arc::new(LiveIndexWriterConfig::new(analyzer));
    let directory: Box<dyn Directory> = Box::new(FSDirectory::open(out_dir).expect("directory"));
    let tracking = Arc::new(TrackingDirectoryWrapper::new(directory));
    let make_info = |max_doc: i32| {
        SegmentInfo::new(
            Arc::new(FSDirectory::open(out_dir).expect("directory")),
            Version::LATEST,
            Some(Version::LATEST),
            segment_name.to_string(),
            max_doc,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            segment_id,
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info")
    };

    // `maxDoc` is unset while documents are indexed, exactly as it is in
    // `DocumentsWriterPerThread`, which only calls `setMaxDoc` at flush. The
    // vectors writer is created during indexing, so it must tolerate that.
    let indexing_info = make_info(-1);
    let mut chain = DefaultIndexingChain::new_for_segment(
        Arc::clone(&live),
        Arc::clone(&tracking),
        &indexing_info,
    )
    .expect("bind segment");

    let numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
    let mut field_infos = FieldInfosBuilder::new(numbers);
    for (doc_id, vectors) in documents.iter().enumerate() {
        let mut document = Document::new();
        for vector in vectors {
            match vector {
                Vec3::Floats(name, similarity, value) => document.add(Box::new(
                    KnnFloatVectorField::new(name, value, *similarity).expect("float vector field"),
                )),
                Vec3::Bytes(name, similarity, value) => document.add(Box::new(
                    KnnByteVectorField::new(name, value, *similarity).expect("byte vector field"),
                )),
            }
        }
        chain
            .process_document(doc_id as i32, &document, true, &mut field_infos)
            .unwrap_or_else(|error| panic!("document {doc_id} must index cleanly: {error}"));
    }
    let finished = field_infos.finish().expect("field infos");

    let segment_info = make_info(documents.len() as i32);
    let info_stream = NoOutputInfoStream;
    let context = flush_io_context(FlushInfo::new(documents.len() as i32, 0));
    let state = IndexingChainFlushState {
        info_stream: &info_stream,
        directory: &tracking,
        segment_info: &segment_info,
        field_infos: &finished,
        context: context.as_ref(),
        live_docs: None,
        del_count_on_flush: 0,
        delete_terms: &[],
    };
    chain.flush(&state).expect("flush");
    finished
}

/// Reads every vector of `dir` with Rucene, producing the same `vec=` lines the
/// Java fixture prints.
fn read_with_rucene(
    dir: &Path,
    segment_info: &SegmentInfo,
    field_infos: &FieldInfos,
) -> Vec<String> {
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    let context = &*DEFAULT_IO_CONTEXT;
    let read_state = SegmentReadState::new(&*directory, segment_info, field_infos, context);
    let codec = ensure_codec();
    if !field_infos.iter().any(|info| info.vector_dimension != 0) {
        return Vec::new();
    }
    let reader: Box<dyn KnnVectorsReader> = codec
        .knn_vectors_format()
        .fields_reader(&read_state)
        .expect("vectors reader");
    reader.check_integrity().expect("check integrity");

    let mut lines = Vec::new();
    for info in field_infos.iter() {
        if info.vector_dimension == 0 {
            continue;
        }
        match info.vector_encoding {
            VectorEncoding::FLOAT32 => {
                let values = reader
                    .get_float_vector_values(&info.name)
                    .expect("float vector values");
                let mut iter = values.iterator().expect("iterator");
                while iter.next_doc().expect("next doc") != rucene::search::NO_MORE_DOCS {
                    let ord = iter.index();
                    let value = values.vector_value(ord).expect("vector value");
                    let encoded = value
                        .iter()
                        .map(|component| format!("{:x}", component.to_bits()))
                        .collect::<Vec<_>>()
                        .join(":");
                    lines.push(format!(
                        "vec={},doc={},ord={},value={}",
                        info.name,
                        iter.doc_id(),
                        ord,
                        encoded
                    ));
                }
            }
            VectorEncoding::BYTE => {
                let values = reader
                    .get_byte_vector_values(&info.name)
                    .expect("byte vector values");
                let mut iter = values.iterator().expect("iterator");
                while iter.next_doc().expect("next doc") != rucene::search::NO_MORE_DOCS {
                    let ord = iter.index();
                    let value = values.vector_value(ord).expect("vector value");
                    let encoded = value.iter().map(|b| format!("{b:02x}")).collect::<String>();
                    lines.push(format!(
                        "vec={},doc={},ord={},value={}",
                        info.name,
                        iter.doc_id(),
                        ord,
                        encoded
                    ));
                }
            }
        }
    }
    lines
}

/// Builds the `SegmentInfo` a reader needs for a directory written elsewhere.
fn reader_segment_info(dir: &Path, name: &str, id: [u8; 16], max_doc: i32) -> SegmentInfo {
    let codec = ensure_codec();
    SegmentInfo::new(
        Arc::new(FSDirectory::open(dir).expect("directory")),
        Version::LATEST,
        Some(Version::LATEST),
        name.to_string(),
        max_doc,
        false,
        false,
        codec,
        HashMap::new(),
        id,
        HashMap::new(),
        Default::default(),
    )
    .expect("segment info")
}

// ---------------------------------------------------------------------------
// Byte comparison
// ---------------------------------------------------------------------------

/// The vector files of `segment` in `dir`, keyed by extension.
fn vector_files(dir: &Path, segment: &str) -> HashMap<String, PathBuf> {
    let mut found = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !file_name.starts_with(&format!("{segment}_"))
            && !file_name.starts_with(&format!("{segment}."))
        {
            continue;
        }
        let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if EXTENSIONS.contains(&extension) {
            found.insert(extension.to_string(), path);
        }
    }
    found
}

fn hex_window(bytes: &[u8], centre: usize) -> String {
    let start = centre.saturating_sub(16);
    let end = (centre + 16).min(bytes.len());
    bytes[start..end]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compares the vector files of the two directories byte for byte, returning
/// the extensions that matched and a description of the first difference in
/// each that did not.
fn compare_vector_bytes(
    java_dir: &Path,
    rust_dir: &Path,
    segment: &str,
) -> (Vec<String>, Vec<String>) {
    let java = vector_files(java_dir, segment);
    let rust = vector_files(rust_dir, segment);

    let mut matched = Vec::new();
    let mut differences = Vec::new();

    let mut extensions: Vec<&str> = EXTENSIONS.to_vec();
    extensions.sort_unstable();
    for extension in extensions {
        match (java.get(extension), rust.get(extension)) {
            (None, None) => {}
            (Some(path), None) => differences.push(format!(
                ".{extension}: Lucene wrote {} but Rucene wrote nothing",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
            (None, Some(path)) => differences.push(format!(
                ".{extension}: Rucene wrote {} but Lucene wrote nothing",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
            (Some(java_path), Some(rust_path)) => {
                let java_name = java_path.file_name().unwrap_or_default().to_string_lossy();
                let rust_name = rust_path.file_name().unwrap_or_default().to_string_lossy();
                if java_name != rust_name {
                    differences.push(format!(
                        ".{extension}: file names differ: Lucene {java_name}, Rucene {rust_name}"
                    ));
                    continue;
                }
                let java_bytes = std::fs::read(java_path).expect("read java file");
                let rust_bytes = std::fs::read(rust_path).expect("read rust file");
                if java_bytes == rust_bytes {
                    matched.push(extension.to_string());
                    continue;
                }
                let first = java_bytes
                    .iter()
                    .zip(rust_bytes.iter())
                    .position(|(a, b)| a != b)
                    .unwrap_or_else(|| java_bytes.len().min(rust_bytes.len()));
                differences.push(format!(
                    ".{extension}: {} vs {} bytes, first difference at {first}\n  java: {}\n  rust: {}",
                    java_bytes.len(),
                    rust_bytes.len(),
                    hex_window(&java_bytes, first),
                    hex_window(&rust_bytes, first)
                ));
            }
        }
    }
    (matched, differences)
}

// ---------------------------------------------------------------------------
// One case, end to end
// ---------------------------------------------------------------------------

struct Comparison {
    case: String,
    /// The extensions Lucene itself wrote for this segment. Comparing against
    /// this is what stops the byte assertion from passing vacuously when
    /// neither side wrote a file the test believed it was comparing.
    java_extensions: Vec<String>,
    matched: Vec<String>,
    differences: Vec<String>,
    java_dump: Vec<String>,
    rucene_reads_java: Vec<String>,
    java_reads_rucene: Vec<String>,
    rucene_dump: Vec<String>,
    java_field_infos: Vec<String>,
    rucene_field_infos: Vec<String>,
}

fn run_case(case: &str) -> Comparison {
    require_maven();
    let root = std::env::temp_dir().join(format!(
        "rucene-vectors-{case}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let java_dir = root.join("java");
    let rust_dir = root.join("rust");
    std::fs::create_dir_all(&java_dir).expect("java dir");
    std::fs::create_dir_all(&rust_dir).expect("rust dir");

    let java = run_java_fixture(&java_dir, case).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        !java.compound,
        "the fixture must not bundle the segment into a .cfs"
    );

    let docs = documents(case);
    assert_eq!(
        java.max_doc,
        docs.len() as i32,
        "the Rust corpus for {case} has a different document count than the Java one"
    );

    let rucene_field_infos = write_with_rucene(&rust_dir, &java.name, java.id, &docs);

    let (matched, differences) = compare_vector_bytes(&java_dir, &rust_dir, &java.name);
    let mut java_extensions: Vec<String> = vector_files(&java_dir, &java.name)
        .keys()
        .cloned()
        .collect();
    java_extensions.sort();

    // Direction 2: Rucene reads the segment Lucene wrote.
    let java_segment_info = reader_segment_info(&java_dir, &java.name, java.id, java.max_doc);
    let rucene_reads_java = read_with_rucene(&java_dir, &java_segment_info, &rucene_field_infos);

    // Rucene reads its own segment, which is what direction 3 is compared to.
    let rust_segment_info = reader_segment_info(&rust_dir, &java.name, java.id, java.max_doc);
    let rucene_dump = read_with_rucene(&rust_dir, &rust_segment_info, &rucene_field_infos);

    // Direction 3: Lucene reads the segment Rucene wrote.
    let java_reads_rucene = read_with_java(
        &rust_dir,
        &java.name,
        java.id,
        java.max_doc,
        &rucene_field_infos,
    )
    .unwrap_or_else(|e| panic!("{e}"));

    let rucene_infos: Vec<String> = rucene_field_infos
        .iter()
        .map(|info| {
            format!(
                "{},number={},dim={},encoding={},similarity={}",
                info.name,
                info.number,
                info.vector_dimension,
                match info.vector_encoding {
                    VectorEncoding::BYTE => "BYTE",
                    VectorEncoding::FLOAT32 => "FLOAT32",
                },
                similarity_name(info.vector_similarity_function)
            )
        })
        .collect();

    let comparison = Comparison {
        case: case.to_string(),
        java_extensions,
        matched,
        differences,
        java_dump: java.dump,
        rucene_reads_java,
        java_reads_rucene,
        rucene_dump,
        java_field_infos: java.field_infos,
        rucene_field_infos: rucene_infos,
    };
    if std::env::var_os("RUCENE_VECTORS_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&root);
    }
    comparison
}

/// Asserts the three read directions agree, for any case.
fn assert_reads_agree(result: &Comparison) {
    assert_eq!(
        result.rucene_field_infos, result.java_field_infos,
        "{}: the field infos Rucene registered differ from Lucene's",
        result.case
    );

    assert_eq!(
        result.rucene_reads_java, result.java_dump,
        "{}: Rucene decoded Lucene's segment differently from Lucene",
        result.case
    );

    assert_eq!(
        result.java_reads_rucene, result.java_dump,
        "{}: Lucene decoded Rucene's segment differently from its own",
        result.case
    );

    assert_eq!(
        result.rucene_dump, result.java_dump,
        "{}: Rucene decoded its own segment differently from Lucene's",
        result.case
    );
}

/// Asserts everything a case can prove: the three read directions and full
/// byte identity of every vector file Lucene wrote.
///
/// The last assertion is the one that could go vacuous — a byte comparison over
/// an empty set of files passes — so the matched set is compared against the
/// files Lucene actually produced, not merely checked for the absence of
/// differences.
fn assert_case_matches_lucene(case: &str) {
    let result = run_case(case);
    assert_reads_agree(&result);

    assert!(
        result.differences.is_empty(),
        "{}: the vector files are not byte-identical to Lucene's:\n{}\n(matched: {:?})",
        result.case,
        result.differences.join("\n"),
        result.matched
    );
    assert_eq!(
        result.matched, result.java_extensions,
        "{}: the byte comparison covered {:?} but Lucene wrote {:?}",
        result.case, result.matched, result.java_extensions
    );
    assert_eq!(
        result.java_extensions,
        vec!["vec", "vem", "vemf", "vex"],
        "{}: a segment with vectors must carry all four vector files",
        result.case
    );
}

/// Asserts what a case whose segment carries an **HNSW graph** can prove today.
///
/// # The gap this records
///
/// `Lucene99HnswVectorsWriter` writes the graph itself into the `.vex`, and its
/// length and offsets into the `.vem`. Building that graph bit-identically to
/// Lucene needs `HnswGraphBuilder` — the beam search, the neighbour array and
/// the diversity heuristic — to agree with Java step for step, and this port's
/// builder does not yet. Measured on `threshold648`, with the level assignment
/// already matching after the `SplittableRandom` port: the `.vem` is the same
/// 215 bytes and differs in 19 of them — the `.vex` length at 95-96, the
/// offsets that follow it, and the footer checksum — while the `.vex` is 6583
/// bytes against Rucene's 5946 and first differs at offset 83, the neighbour
/// count of the graph's first node.
///
/// So this asserts what the vectors **consumer** owns — the flat vector data
/// and its metadata, byte for byte, in every direction — and pins the graph
/// files as *not yet* identical. The pin is deliberate: when the graph builder
/// is made faithful this assertion fails, and whoever does that work is told to
/// promote these three cases to [`assert_case_matches_lucene`] rather than
/// discovering the gap closed by accident.
fn assert_flat_files_match_lucene_and_the_graph_does_not(case: &str) {
    let result = run_case(case);
    assert_reads_agree(&result);

    assert_eq!(
        result.java_extensions,
        vec!["vec", "vem", "vemf", "vex"],
        "{}: a segment with vectors must carry all four vector files",
        result.case
    );
    let mut matched = result.matched.clone();
    matched.sort();
    assert_eq!(
        matched,
        vec!["vec", "vemf"],
        "{}: the flat vector data and its metadata must be byte-identical to \
         Lucene's, and the graph files must not be — differences were:\n{}",
        result.case,
        result.differences.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn a_tiny_float_segment_matches_lucene() {
    assert_case_matches_lucene("f32tiny");
}

#[test]
fn a_dense_float_segment_matches_lucene() {
    assert_case_matches_lucene("f32dense");
}

#[test]
fn a_sparse_float_segment_matches_lucene() {
    assert_case_matches_lucene("f32sparse");
}

#[test]
fn cosine_float_vectors_match_lucene() {
    assert_case_matches_lucene("f32cosine");
}

#[test]
fn maximum_inner_product_float_vectors_match_lucene() {
    assert_case_matches_lucene("f32mip");
}

#[test]
fn one_dimensional_vectors_match_lucene() {
    assert_case_matches_lucene("dim1");
}

#[test]
fn a_full_alignment_unit_of_dimensions_matches_lucene() {
    assert_case_matches_lucene("dim16");
}

#[test]
fn the_maximum_dimension_count_matches_lucene() {
    assert_case_matches_lucene("dim1024");
}

#[test]
fn a_tiny_byte_segment_matches_lucene() {
    assert_case_matches_lucene("bytetiny");
}

#[test]
fn a_dense_byte_segment_matches_lucene() {
    assert_case_matches_lucene("bytedense");
}

#[test]
fn a_sparse_byte_segment_matches_lucene() {
    assert_case_matches_lucene("bytesparse");
}

#[test]
fn cosine_byte_vectors_match_lucene() {
    assert_case_matches_lucene("bytecosine");
}

#[test]
fn maximum_inner_product_byte_vectors_match_lucene() {
    assert_case_matches_lucene("bytemip");
}

#[test]
fn several_vector_fields_in_one_segment_match_lucene() {
    assert_case_matches_lucene("multi");
}

#[test]
fn several_sparse_vector_fields_match_lucene() {
    assert_case_matches_lucene("multisparse");
}

#[test]
fn the_metadata_field_order_follows_the_first_document_that_used_each_field() {
    assert_case_matches_lucene("fieldorder");
}

#[test]
fn a_single_vector_matches_lucene() {
    assert_case_matches_lucene("one");
}

#[test]
fn a_vector_on_the_last_document_only_matches_lucene() {
    assert_case_matches_lucene("lastonly");
}

/// The last corpus size at which `shouldCreateGraph` is still false.
///
/// `Lucene99HnswVectorsWriter.shouldCreateGraph` builds a graph only when
/// `numNodes > (int)(log(numNodes) * tinySegmentsThreshold)`, and the default
/// threshold is 100. Solved over the integers, the first size that builds one is
/// 648; 647 is the last that does not. The two cases straddle that line, which
/// is the only way to know a graph case is exercising the graph rather than the
/// empty-graph metadata beside it.
#[test]
fn the_last_size_below_the_graph_threshold_matches_lucene() {
    assert_case_matches_lucene("threshold647");
}

#[test]
fn the_first_size_that_builds_a_graph_matches_lucene_except_for_the_graph() {
    assert_flat_files_match_lucene_and_the_graph_does_not("threshold648");
}

#[test]
fn a_multi_level_graph_matches_lucene_except_for_the_graph() {
    assert_flat_files_match_lucene_and_the_graph_does_not("threshold2000");
}

#[test]
fn a_segment_without_vectors_writes_no_vector_file() {
    require_maven();
    let result = run_case("novec");
    assert!(
        result.matched.is_empty() && result.differences.is_empty(),
        "novec: neither side may write a vector file, but got matched {:?} and differences {:?}",
        result.matched,
        result.differences
    );
    assert!(
        result.java_dump.is_empty(),
        "novec: Lucene decoded vectors from a segment that has none"
    );
}
