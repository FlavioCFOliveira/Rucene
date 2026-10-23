//! Portability tests for the Lucene 10.4 scalar-quantized vectors format.
//!
//! Every case drives `Lucene104ScalarQuantizedVectorsFormat` **directly** — no
//! `IndexWriter`, no indexing chain — on both sides, so that a byte difference
//! can only come from this format: from the quantization, from the centroid, or
//! from the metadata layout. The Java harness
//! (`ScalarQuantizedVectorsFixture`) builds the same `SegmentWriteState` by hand
//! and writes the same vectors.
//!
//! Each case asserts four ways:
//!
//! 1. the `.vemq` and `.veq` this format writes, and the `.vec` and `.vemf` its
//!    raw delegate writes, are **byte for byte identical** to Lucene's;
//! 2. Rucene's reader decodes, from **Lucene's own files**, exactly the floats
//!    Lucene's reader decodes, for every document, in the same ordinal order;
//! 3. the encoding's wire number — the value the `.vemq` carries — is the one
//!    Lucene writes. `PACKED_NIBBLE` is 1 and `SEVEN_BIT` is 2, which is *not*
//!    the order the two are declared in; a swap writes a file Lucene decodes at
//!    the wrong width, and nothing else in the file says so;
//! 4. the packed length and the discretised dimension count agree with Lucene's,
//!    since they set the on-disk stride of every vector.
//!
//! # The grid
//!
//! * **all three symmetric encodings** — `UNSIGNED_BYTE`, `PACKED_NIBBLE` and
//!   `SEVEN_BIT`. Lucene also declares two asymmetric ones, which this port does
//!   not model; see `ScalarEncoding`'s declared divergence.
//! * **all four similarity functions** — each is an ordinal in the metadata, and
//!   `COSINE` additionally normalises both the vectors and the centroid before
//!   quantizing, which changes every quantized byte.
//! * **dimensions at the packing edges** — 1, 3 and 5 are odd, so
//!   `PACKED_NIBBLE` pads them to a whole byte; 8 and 16 are exact.
//! * **dense and sparse fields** — a sparse field adds the
//!   `DirectMonotonicWriter` ordinal-to-document mapping to the metadata.
//!
//! A missing Maven or JDK is a hard failure, not a skip: a portability test has
//! nothing to assert without the reference implementation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::codecs::hnsw::flat_vectors::FlatVectorsFormat;
use rucene::codecs::knn_vectors::KnnVectorsFormat;
use rucene::codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsFormat;
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::stub::BufferedUpdates;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::index::merge::{identity_doc_map, MergeState};
use rucene::index::{FieldInfo, FieldInfos, SegmentInfo, VectorEncoding, VectorSimilarityFunction};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{Directory, FSDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::quantization::ScalarEncoding;
use rucene::util::{default_info_stream, Version};

/// The segment name and id the Java fixture uses, so the two sides write files
/// with the same names and the same header id.
const SEGMENT: &str = "_sq";
const SEGMENT_ID: [u8; 16] = [
    0x51, 0x56, 0x45, 0x43, 0x54, 0x4f, 0x52, 0x53, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
];
const FIELD: &str = "vec";
const MAX_DOC: i32 = 12;

/// The four files a scalar-quantized field is made of: the two this format
/// writes and the two its raw delegate writes.
const EXTENSIONS: [&str; 4] = ["vemq", "veq", "vec", "vemf"];

/// The segment the merge case writes into.
const MERGED_SEGMENT: &str = "_sqm";

/// Maven has one local repository, so two harness runs at once can race on it.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// The vector of `doc`, or `None` when the field is sparse and skips it.
///
/// Mirrors `ScalarQuantizedVectorsFixture.vectorFor` exactly.
fn vector_for(doc: i32, dim: i32, dense: bool) -> Option<Vec<f32>> {
    if !dense && doc % 3 != 0 {
        return None;
    }
    let mut v = Vec::with_capacity(dim as usize);
    for i in 0..dim {
        v.push(((doc * 31 + i * 17) % 23 - 11) as f32 + (i % 5) as f32 * 0.25);
    }
    Some(v)
}

fn encoding_for(name: &str) -> ScalarEncoding {
    match name {
        "unsignedbyte" => ScalarEncoding::UnsignedByte,
        "packednibble" => ScalarEncoding::PackedNibble,
        "sevenbit" => ScalarEncoding::SevenBit,
        other => panic!("unknown encoding: {other}"),
    }
}

fn similarity_for(name: &str) -> VectorSimilarityFunction {
    match name {
        "euclidean" => VectorSimilarityFunction::EUCLIDEAN,
        "dot" => VectorSimilarityFunction::DOT_PRODUCT,
        "cosine" => VectorSimilarityFunction::COSINE,
        "mip" => VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        other => panic!("unknown similarity: {other}"),
    }
}

fn field_infos(dim: i32, similarity: VectorSimilarityFunction) -> FieldInfos {
    let mut info = FieldInfo::new(FIELD, 0);
    info.vector_dimension = dim;
    info.vector_encoding = VectorEncoding::FLOAT32;
    info.vector_similarity_function = similarity;
    FieldInfos::new(vec![info]).expect("field infos")
}

fn segment_info(directory: &Arc<dyn Directory>) -> SegmentInfo {
    SegmentInfo::new(
        Arc::clone(directory),
        Version::LATEST,
        Some(Version::LATEST),
        SEGMENT.to_string(),
        MAX_DOC,
        false,
        false,
        codec(),
        HashMap::new(),
        SEGMENT_ID,
        HashMap::new(),
        Default::default(),
    )
    .expect("segment info")
}

/// Writes one case with Rucene, through the format's own writer.
fn write_with_rucene(
    dir: &Path,
    encoding: ScalarEncoding,
    dim: i32,
    similarity: VectorSimilarityFunction,
    dense: bool,
) {
    let directory: Arc<dyn Directory> =
        Arc::new(FSDirectory::open(dir).expect("open the output directory"));
    let info = segment_info(&directory);
    let infos = field_infos(dim, similarity);
    let write_state = SegmentWriteState::new(
        default_info_stream(),
        directory.as_ref(),
        &info,
        &infos,
        &BufferedUpdates,
        &*DEFAULT_IO_CONTEXT,
    );

    let format = Lucene104ScalarQuantizedVectorsFormat::with_encoding(encoding);
    let mut writer = format
        .fields_writer_flat(&write_state)
        .expect("scalar-quantized writer");
    let field_info = infos.field_info_by_number(0).expect("field info").clone();
    let mut field_writer = match writer.add_field(&field_info).expect("add field") {
        rucene::codecs::knn_vectors::FieldVectorWriter::Float(writer) => writer,
        other => panic!("expected a float field writer, got {other:?}"),
    };
    for doc in 0..MAX_DOC {
        if let Some(v) = vector_for(doc, dim, dense) {
            field_writer.add_value(doc, v).expect("add value");
        }
    }
    writer.flush(MAX_DOC, None).expect("flush");
    writer.finish().expect("finish");
    writer.close().expect("close");

    merge_with_rucene(dir, encoding, dim, similarity);
}

/// Merges the segment just written into a second one, through
/// `FlatVectorsWriter::merge_one_flat_vector_field`.
///
/// One source segment with no deletions is the shape where the merged view is
/// exactly the source's vectors in order, which is what this port's merge walks
/// — see the divergence declared on that method.
fn merge_with_rucene(
    dir: &Path,
    encoding: ScalarEncoding,
    dim: i32,
    similarity: VectorSimilarityFunction,
) {
    let directory: Arc<dyn Directory> =
        Arc::new(FSDirectory::open(dir).expect("open the output directory"));
    let source_info = segment_info(&directory);
    let infos = field_infos(dim, similarity);
    let read_state = SegmentReadState::new(
        directory.as_ref(),
        &source_info,
        &infos,
        &*DEFAULT_IO_CONTEXT,
    );
    let format = Lucene104ScalarQuantizedVectorsFormat::with_encoding(encoding);
    let source: Arc<dyn rucene::codecs::knn_vectors::KnnVectorsReader> =
        Arc::from(format.fields_reader(&read_state).expect("source reader"));

    let merged_info = SegmentInfo::new(
        Arc::clone(&directory),
        Version::LATEST,
        Some(Version::LATEST),
        MERGED_SEGMENT.to_string(),
        MAX_DOC,
        false,
        false,
        codec(),
        HashMap::new(),
        SEGMENT_ID,
        HashMap::new(),
        Default::default(),
    )
    .expect("merged segment info");
    let write_state = SegmentWriteState::new(
        default_info_stream(),
        directory.as_ref(),
        &merged_info,
        &infos,
        &BufferedUpdates,
        &*DEFAULT_IO_CONTEXT,
    );
    let merge_state = MergeState::new(
        vec![identity_doc_map(MAX_DOC, 0)],
        merged_info.clone(),
        infos.clone(),
        vec![None],
        vec![None],
        vec![None],
        vec![None],
        vec![infos.clone()],
        vec![None],
        vec![None],
        vec![None],
        vec![Some(source)],
        vec![MAX_DOC],
        false,
    );
    let field_info = infos.field_info_by_number(0).expect("field info").clone();
    let mut writer = format
        .fields_writer_flat(&write_state)
        .expect("merge writer");
    writer
        .merge_one_flat_vector_field(&field_info, &merge_state)
        .expect("merge one field");
    writer.finish().expect("finish");
    writer.close().expect("close");
}

/// Reads a directory with Rucene's reader and returns one `doc:ord:hex,..` line
/// per vector, in the format the Java fixture prints.
fn read_with_rucene(
    dir: &Path,
    encoding: ScalarEncoding,
    dim: i32,
    similarity: VectorSimilarityFunction,
) -> Vec<String> {
    let directory: Arc<dyn Directory> =
        Arc::new(FSDirectory::open(dir).expect("open the directory"));
    let info = segment_info(&directory);
    let infos = field_infos(dim, similarity);
    let read_state = SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
    let format = Lucene104ScalarQuantizedVectorsFormat::with_encoding(encoding);
    let reader = format.fields_reader_flat(&read_state).expect("reader");
    reader.check_integrity().expect("check integrity");

    let values = reader.get_float_vector_values(FIELD).expect("float values");
    let mut iter = values.iterator().expect("iterator");
    let mut out = Vec::new();
    loop {
        let doc = iter.next_doc().expect("next doc");
        if doc == NO_MORE_DOCS {
            break;
        }
        let ord = iter.index();
        let v = values.vector_value(ord).expect("vector value");
        let hex: Vec<String> = v.iter().map(|f| format!("{:08x}", f.to_bits())).collect();
        out.push(format!("vec={doc}:{ord}:{}", hex.join(",")));
    }
    out
}

// ---------------------------------------------------------------------------
// Java harness
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

/// What the Java harness reported about the segment it wrote.
struct JavaRun {
    wire_number: i32,
    doc_packed_length: i32,
    discrete_dimensions: i32,
    dump: Vec<String>,
}

fn run_java(out_dir: &Path, case: &str) -> JavaRun {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mvn = which_mvn()
        .unwrap_or_else(|e| panic!("scalar-quantized portability tests need Maven and a JDK: {e}"));
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.ScalarQuantizedVectorsFixture")
        .arg(format!("-Dexec.args={} {case}", out_dir.display()))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "read_ok=true"),
        "the Java fixture failed for case {case}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let mut wire_number = None;
    let mut doc_packed_length = None;
    let mut discrete_dimensions = None;
    let mut dump = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("wire_number=") {
            wire_number = Some(v.parse().expect("wire number"));
        } else if let Some(v) = line.strip_prefix("doc_packed_length=") {
            doc_packed_length = Some(v.parse().expect("packed length"));
        } else if let Some(v) = line.strip_prefix("discrete_dimensions=") {
            discrete_dimensions = Some(v.parse().expect("discrete dimensions"));
        } else if line.starts_with("vec=") {
            dump.push(line.to_string());
        }
    }
    JavaRun {
        wire_number: wire_number.expect("the fixture printed no wire number"),
        doc_packed_length: doc_packed_length.expect("the fixture printed no packed length"),
        discrete_dimensions: discrete_dimensions
            .expect("the fixture printed no discrete dimension count"),
        dump,
    }
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

/// Every case the sweep runs, as `<encoding>_<similarity>_<dim>_<density>`.
fn cases() -> Vec<String> {
    let mut cases = Vec::new();
    // Every encoding against every similarity, at a dimension that packing pads.
    for encoding in ["unsignedbyte", "packednibble", "sevenbit"] {
        for similarity in ["euclidean", "dot", "cosine", "mip"] {
            cases.push(format!("{encoding}_{similarity}_5_dense"));
        }
    }
    // The packing edges, and the sparse shape.
    for encoding in ["unsignedbyte", "packednibble", "sevenbit"] {
        for dim in [1, 3, 8, 16] {
            cases.push(format!("{encoding}_euclidean_{dim}_dense"));
        }
        cases.push(format!("{encoding}_euclidean_5_sparse"));
        cases.push(format!("{encoding}_cosine_8_sparse"));
    }
    cases
}

#[test]
fn scalar_quantized_vectors_match_lucene_byte_for_byte() {
    let root = std::env::temp_dir().join(format!("rucene-sq-vectors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    for case in cases() {
        let parts: Vec<&str> = case.split('_').collect();
        let encoding = encoding_for(parts[0]);
        let similarity = similarity_for(parts[1]);
        let dim: i32 = parts[2].parse().expect("dimension");
        let dense = parts[3] == "dense";

        let java_dir = root.join(&case).join("java");
        let rust_dir = root.join(&case).join("rust");
        std::fs::create_dir_all(&java_dir).expect("java dir");
        std::fs::create_dir_all(&rust_dir).expect("rust dir");

        let java = run_java(&java_dir, &case);

        // 3. and 4. — the values that set the on-disk stride.
        assert_eq!(
            encoding.get_wire_number(),
            java.wire_number,
            "case {case}: the wire number written into the .vemq must be Lucene's"
        );
        assert_eq!(
            encoding.get_doc_packed_length(dim),
            java.doc_packed_length,
            "case {case}: the packed length sets the stride of every vector"
        );
        assert_eq!(
            encoding.get_discrete_dimensions(dim),
            java.discrete_dimensions,
            "case {case}: the discretised dimension count sets the quantization scratch"
        );

        write_with_rucene(&rust_dir, encoding, dim, similarity, dense);

        // 1. — byte-for-byte parity of all four files, for the flushed segment
        // and for the one the merge wrote.
        for (segment, ext) in EXTENSIONS
            .iter()
            .map(|ext| (SEGMENT, *ext))
            .chain(EXTENSIONS.iter().map(|ext| (MERGED_SEGMENT, *ext)))
        {
            let name = format!("{segment}.{ext}");
            let java_bytes = std::fs::read(java_dir.join(&name))
                .unwrap_or_else(|e| panic!("case {case}: reading Lucene's {name}: {e}"));
            let rust_bytes = std::fs::read(rust_dir.join(&name))
                .unwrap_or_else(|e| panic!("case {case}: reading Rucene's {name}: {e}"));
            assert_eq!(
                java_bytes.len(),
                rust_bytes.len(),
                "case {case}: {name} has a different length ({} vs {})",
                java_bytes.len(),
                rust_bytes.len()
            );
            if java_bytes != rust_bytes {
                let first = java_bytes
                    .iter()
                    .zip(&rust_bytes)
                    .position(|(a, b)| a != b)
                    .expect("the two differ somewhere");
                panic!(
                    "case {case}: {name} differs at byte {first}: Lucene {:#04x}, Rucene {:#04x}",
                    java_bytes[first], rust_bytes[first]
                );
            }
        }

        // 2. — Rucene reads Lucene's own files and decodes the same floats.
        let rucene_from_java = read_with_rucene(&java_dir, encoding, dim, similarity);
        assert_eq!(
            java.dump, rucene_from_java,
            "case {case}: Rucene decoded different values from Lucene's own files"
        );

        // ...and reads back what it wrote.
        let rucene_from_rust = read_with_rucene(&rust_dir, encoding, dim, similarity);
        assert_eq!(
            java.dump, rucene_from_rust,
            "case {case}: Rucene decoded different values from its own files"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}
