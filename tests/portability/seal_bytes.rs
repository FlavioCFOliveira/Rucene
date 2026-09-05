//! Byte-parity tests for the three files that seal a flushed segment.
//!
//! `IndexingChain.flush` writes the `.fnm` last
//! (`IndexingChain.java:377-387`), and
//! `DocumentsWriterPerThread.sealFlushedSegment` writes the compound file and
//! then the `.si` (`DocumentsWriterPerThread.java:625-712`). Until Rucene wrote
//! those files, no index it produced could be reopened by anybody; these tests
//! prove that what it now writes is what Lucene 10.5.0 writes, byte for byte.
//!
//! Each case drives the codec writers directly on both sides, with every input
//! pinned — segment name, segment id, versions, field metadata, diagnostics,
//! sub-file contents — and compares the resulting file bytes. Driving a whole
//! `IndexWriter` and comparing its output byte for byte is impossible in
//! principle: Lucene stamps every segment with `StringHelper.randomId()`, which
//! the `.si` records and every codec header in the segment embeds, and stamps
//! the `.si` diagnostics with a wall-clock timestamp, so two runs of Lucene
//! itself already differ. That the same operations produce the same *files* end
//! to end, and that each implementation reads the other's index, is proven in
//! `tests/portability/flush_files.rs`.
//!
//! # What the maps and sets can and cannot prove
//!
//! `.fnm` serialises each field's codec attributes with
//! `DataOutput.writeMapOfStrings`, and `.si` serialises its diagnostics and
//! attributes the same way and its file list with `writeSetOfStrings`. All
//! three walk the collection in *iteration order*, which no format rule fixes:
//! on the Java side it falls out of `java.util.HashMap`'s bucket layout, and on
//! the Rust side out of [`std::collections::HashMap`]'s per-process random seed.
//! Two implementations therefore cannot agree on the order of a multi-entry map
//! unless one models the other's hashing.
//!
//! The cases below consequently pin every map and set to at most one entry,
//! which makes the byte comparison exact and about the format itself — the
//! encoding of names, numbers, option bits, doc-values and skip-index types,
//! point and vector dimensions, the soft-deletes and parent flags, the index
//! header and id, the sort encoding, the file list and the footer checksum.
//! [`fnm_multi_attribute_matches_lucene_except_for_map_order`] and
//! [`si_multi_file_matches_lucene_except_for_set_order`] then cover the
//! multi-entry shapes the real flush path produces, and state exactly what they
//! can still assert.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::codecs::{codec_util, register_codec, Codec, Lucene104Codec};
use rucene::index::field_infos::{FieldInfo, FieldInfos};
use rucene::index::segment_info::SegmentInfo;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, VectorEncoding, VectorSimilarityFunction,
};
use rucene::store::{Directory, FSDirectory, TrackingDirectoryWrapper, DEFAULT_IO_CONTEXT};
use rucene::util::Version;

/// The segment name every case writes; mirrors `SealBytesFixture.SEGMENT`.
const SEGMENT: &str = "_9";

/// The segment id every case writes; mirrors `SealBytesFixture.SEGMENT_ID`.
const SEGMENT_ID: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

/// The `maxDoc` every case pins; mirrors `SealBytesFixture.segmentInfo`.
const MAX_DOC: i32 = 7;

/// One sub-file of the compound-file case; mirrors `SealBytesFixture.SubFile`.
///
/// The body lengths are pairwise distinct on purpose:
/// `Lucene90CompoundFormat.writeCompoundFile` orders the entries by ascending
/// file length through a priority queue
/// (`Lucene90CompoundFormat.java:126-131`), and only distinct lengths make that
/// order independent of the iteration order of `si.files()`, which is a hash
/// set on both sides.
struct SubFile {
    extension: &'static str,
    codec: &'static str,
    version: i32,
    body_length: i32,
}

/// The sub-files of the compound-file case; mirrors `SealBytesFixture.SUB_FILES`.
const SUB_FILES: [SubFile; 3] = [
    SubFile {
        extension: ".fdt",
        codec: "RuceneSealTestA",
        version: 3,
        body_length: 700,
    },
    SubFile {
        extension: ".nvd",
        codec: "RuceneSealTestB",
        version: 0,
        body_length: 61,
    },
    SubFile {
        extension: ".tim",
        codec: "RuceneSealTestC",
        version: 11,
        body_length: 313,
    },
];

// ---------------------------------------------------------------------------
// Java harness driver
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Serialises Maven invocations, which all share one `target/` directory.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "mvnw"] {
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
/// A portability test proves byte compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("seal-bytes portability tests require Maven and a JDK: {reason}");
    }
}

/// Runs `SealBytesFixture` for `case`, writing into `out_dir`.
fn run_java_fixture(out_dir: &Path, case: &str) {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    assert!(
        harness.join("pom.xml").exists(),
        "pom.xml not found in {}",
        harness.display()
    );
    let mvn = which_mvn().expect("Maven");
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.SealBytesFixture")
        .arg(format!("-Dexec.args={} {}", out_dir.display(), case))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn Maven");
    assert!(
        output.status.success(),
        "Java harness failed for case {case}:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

fn ensure_codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

fn open_dir(path: &Path) -> Arc<dyn Directory> {
    std::fs::create_dir_all(path).expect("create output directory");
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).expect("open directory"));
    for name in dir.list_all().expect("list directory") {
        dir.delete_file(&name).expect("clear output directory");
    }
    dir
}

/// Builds the `SegmentInfo` every case pins; mirrors
/// `SealBytesFixture.segmentInfo`.
fn segment_info(dir: &Arc<dyn Directory>, codec: &Arc<dyn Codec>) -> SegmentInfo {
    let mut diagnostics = HashMap::new();
    diagnostics.insert("source".to_string(), "flush".to_string());
    let mut attributes = HashMap::new();
    attributes.insert(
        "Lucene90StoredFieldsFormat.mode".to_string(),
        "BEST_SPEED".to_string(),
    );
    SegmentInfo::new(
        Arc::clone(dir),
        Version::LATEST,
        Some(Version::LATEST),
        SEGMENT.to_string(),
        MAX_DOC,
        false,
        false,
        Arc::clone(codec),
        diagnostics,
        SEGMENT_ID,
        attributes,
        // Rucene encodes "unsorted" as the relevance sort; see
        // `write_segment_sort` in `codecs/lucene99/segment_info_format.rs`.
        Default::default(),
    )
    .expect("segment info")
}

/// One field of the `.fnm` case; mirrors `SealBytesFixture.field`.
#[allow(clippy::too_many_arguments)]
fn field(
    name: &str,
    number: i32,
    store_term_vector: bool,
    omit_norms: bool,
    store_payloads: bool,
    index_options: IndexOptions,
    doc_values: DocValuesType,
    doc_values_skip_index: DocValuesSkipIndexType,
    attributes: &[(&str, &str)],
    point_dimension_count: i32,
    point_index_dimension_count: i32,
    point_num_bytes: i32,
    vector_dimension: i32,
    vector_encoding: VectorEncoding,
    vector_similarity: VectorSimilarityFunction,
    soft_deletes: bool,
    is_parent: bool,
) -> FieldInfo {
    FieldInfo::new_full(
        name,
        number,
        store_term_vector,
        omit_norms,
        store_payloads,
        index_options,
        doc_values,
        doc_values_skip_index,
        -1,
        attributes
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        point_dimension_count,
        point_index_dimension_count,
        point_num_bytes,
        vector_dimension,
        vector_encoding,
        vector_similarity,
        soft_deletes,
        is_parent,
    )
    .expect("field info")
}

/// The field table of the `.fnm` cases; mirrors
/// `SealBytesFixture.writeFieldInfos`.
///
/// The shapes span what the format encodes: postings options with offsets and
/// payloads, norms on and off, term vectors, doc values with and without a skip
/// index, points, byte vectors with a non-default similarity, the soft-deletes
/// field and the parent field.
fn field_table(multi_attribute: bool) -> Vec<FieldInfo> {
    const ONE: &[(&str, &str)] = &[("PerFieldPostingsFormat.format", "Lucene104")];
    const TWO: &[(&str, &str)] = &[
        ("PerFieldPostingsFormat.format", "Lucene104"),
        ("PerFieldPostingsFormat.suffix", "0"),
    ];
    const NONE: &[(&str, &str)] = &[];

    vec![
        field(
            "body",
            0,
            true,
            false,
            true,
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            if multi_attribute { TWO } else { NONE },
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        ),
        field(
            "id",
            1,
            false,
            true,
            false,
            IndexOptions::DOCS,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            if multi_attribute { TWO } else { ONE },
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        ),
        field(
            "price",
            2,
            false,
            true,
            false,
            IndexOptions::NONE,
            DocValuesType::SORTED_NUMERIC,
            DocValuesSkipIndexType::RANGE,
            NONE,
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        ),
        field(
            "point",
            3,
            false,
            true,
            false,
            IndexOptions::NONE,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            NONE,
            2,
            1,
            4,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        ),
        field(
            "vector",
            4,
            false,
            true,
            false,
            IndexOptions::NONE,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            NONE,
            0,
            0,
            0,
            3,
            VectorEncoding::BYTE,
            VectorSimilarityFunction::DOT_PRODUCT,
            false,
            false,
        ),
        field(
            "__soft_deletes",
            5,
            false,
            true,
            false,
            IndexOptions::NONE,
            DocValuesType::NUMERIC,
            DocValuesSkipIndexType::NONE,
            NONE,
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            true,
            false,
        ),
        field(
            "_parent",
            6,
            false,
            true,
            false,
            IndexOptions::NONE,
            DocValuesType::NUMERIC,
            DocValuesSkipIndexType::NONE,
            NONE,
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            true,
        ),
    ]
}

/// Writes the `.fnm` of the pinned segment into `out_dir`.
fn write_field_infos(out_dir: &Path, multi_attribute: bool) {
    let codec = ensure_codec();
    let dir = open_dir(out_dir);
    let info = segment_info(&dir, &codec);
    let field_infos = FieldInfos::new(field_table(multi_attribute)).expect("field infos");
    codec
        .field_infos_format()
        .write(
            dir.as_ref(),
            &info,
            "",
            &field_infos,
            &*rucene::store::DEFAULT_IO_CONTEXT,
        )
        .expect("write field infos");
}

/// The extra files the multi-file `.si` case lists; mirrors
/// `SealBytesFixture.SI_EXTRA_FILES`.
const SI_EXTRA_FILES: [&str; 3] = [".fdt", ".fdx", ".nvd"];

/// Writes the `.si` of the pinned segment into `out_dir`.
///
/// With `multi_file` false the segment starts with no files, so the only entry
/// the format writes is the `.si` itself, which the writer adds before
/// serialising, exactly as `Lucene99SegmentInfoFormat.write` does
/// (`Lucene99SegmentInfoFormat.java:175`). One entry has one possible encoding,
/// which makes the byte comparison exact.
fn write_segment_info(out_dir: &Path, multi_file: bool) {
    let codec = ensure_codec();
    let dir = open_dir(out_dir);
    let info = segment_info(&dir, &codec);
    info.set_files(if multi_file {
        SI_EXTRA_FILES
            .iter()
            .map(|extension| format!("{SEGMENT}{extension}"))
            .collect()
    } else {
        std::collections::HashSet::new()
    });
    codec
        .segment_info_format()
        .write(dir.as_ref(), &info, &*DEFAULT_IO_CONTEXT)
        .expect("write segment info");
}

/// Writes the sub-files of the compound-file case and packs them.
fn write_compound_file(out_dir: &Path) {
    let codec = ensure_codec();
    let dir = open_dir(out_dir);
    let mut files = std::collections::HashSet::new();
    for sub in &SUB_FILES {
        let name = format!("{SEGMENT}{}", sub.extension);
        let mut output = dir
            .create_output(&name, &*DEFAULT_IO_CONTEXT)
            .expect("create sub-file");
        codec_util::write_index_header(output.as_mut(), sub.codec, sub.version, &SEGMENT_ID, "")
            .expect("write index header");
        for i in 0..sub.body_length {
            output
                .write_byte((i * 7 + sub.body_length) as u8)
                .expect("write body");
        }
        codec_util::write_footer(output.as_mut()).expect("write footer");
        drop(output);
        files.insert(name);
    }

    let info = segment_info(&dir, &codec);
    info.set_files(files);
    // `IndexWriter::create_compound_file` refuses a tracking directory that has
    // already created files, so it gets a fresh one over the same directory,
    // exactly as `sealFlushedSegment` gives it one.
    let tracking = TrackingDirectoryWrapper::new(Box::new(
        FSDirectory::open(out_dir).expect("open directory for the tracking wrapper"),
    ));
    let mut delete_files = |_files: std::collections::HashSet<String>| Ok(());
    rucene::index::index_writer::IndexWriter::create_compound_file(
        &rucene::util::NoOutputInfoStream,
        &tracking,
        &info,
        &*DEFAULT_IO_CONTEXT,
        &mut delete_files,
    )
    .expect("create compound file");
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// Compares one file of the two directories byte for byte.
fn assert_file_bytes_equal(java_dir: &Path, rust_dir: &Path, file_name: &str, case: &str) {
    let java = std::fs::read(java_dir.join(file_name))
        .unwrap_or_else(|e| panic!("[{case}] lucene did not write {file_name}: {e}"));
    let rust = std::fs::read(rust_dir.join(file_name))
        .unwrap_or_else(|e| panic!("[{case}] rucene did not write {file_name}: {e}"));
    if java == rust {
        return;
    }
    let first = java
        .iter()
        .zip(rust.iter())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| java.len().min(rust.len()));
    panic!(
        "[{case}] {file_name} differs at byte {first} (lucene {} bytes, rucene {} bytes)\n  \
         lucene: {}\n  rucene: {}",
        java.len(),
        rust.len(),
        hex_window(&java, first),
        hex_window(&rust, first)
    );
}

/// Renders a readable window of `bytes` around `centre`.
fn hex_window(bytes: &[u8], centre: usize) -> String {
    let from = centre.saturating_sub(16);
    let to = std::cmp::min(centre + 16, bytes.len());
    let body: Vec<String> = bytes[from..to].iter().map(|b| format!("{b:02x}")).collect();
    format!("[{from}..{to}] {}", body.join(" "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn fnm_matches_lucene_byte_for_byte() {
    require_maven();
    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");
    run_java_fixture(java_dir.path(), "fnm");
    write_field_infos(rust_dir.path(), false);
    assert_file_bytes_equal(java_dir.path(), rust_dir.path(), "_9.fnm", "fnm");
}

#[test]
fn si_matches_lucene_byte_for_byte() {
    require_maven();
    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");
    run_java_fixture(java_dir.path(), "si");
    write_segment_info(rust_dir.path(), false);
    assert_file_bytes_equal(java_dir.path(), rust_dir.path(), "_9.si", "si");
}

/// Covers the multi-file `.si` a real segment carries.
///
/// A sealed segment always lists more than one file, and
/// `Lucene99SegmentInfoFormat.write` serialises that list with
/// `writeSetOfStrings` (`Lucene99SegmentInfoFormat.java:220`), which walks the
/// set in iteration order. The set is a `java.util.HashSet` on one side and a
/// [`std::collections::HashSet`] on the other, so beyond one entry the two
/// orders are unrelated and the bytes cannot be compared directly — the same
/// obstacle as in
/// [`fnm_multi_attribute_matches_lucene_except_for_map_order`].
///
/// What is asserted instead is that the two files have the same size, so both
/// sides wrote the same number of entries with the same total name length, and
/// that Rucene's own reader decodes both into the same segment: same versions,
/// maxDoc, flags, id, diagnostics, attributes, sort and — as a set — the same
/// files.
#[test]
fn si_multi_file_matches_lucene_except_for_set_order() {
    require_maven();
    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");
    run_java_fixture(java_dir.path(), "si_multi_file");
    write_segment_info(rust_dir.path(), true);

    let java_bytes = std::fs::read(java_dir.path().join("_9.si")).expect("lucene .si");
    let rust_bytes = std::fs::read(rust_dir.path().join("_9.si")).expect("rucene .si");
    assert_eq!(
        java_bytes.len(),
        rust_bytes.len(),
        "the two .si files have different sizes, so they do not list the same files"
    );

    let codec = ensure_codec();
    let describe = |dir_path: &Path| -> String {
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir_path).expect("open"));
        let info = codec
            .segment_info_format()
            .read(Arc::clone(&dir), SEGMENT, &SEGMENT_ID, &*DEFAULT_IO_CONTEXT)
            .expect("read segment info");
        let mut files: Vec<String> = info.files().expect("files").into_iter().collect();
        files.sort();
        let mut diagnostics: Vec<String> = info
            .get_diagnostics()
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        diagnostics.sort();
        let mut attributes: Vec<String> = info
            .get_attributes()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        attributes.sort();
        format!(
            "name={} maxDoc={} cfs={} hasBlocks={} version={:?} minVersion={:?} id={:02x?}              diagnostics=[{}] attributes=[{}] files=[{}] sort={:?}",
            info.name,
            info.max_doc().expect("maxDoc"),
            info.get_use_compound_file(),
            info.get_has_blocks(),
            info.version(),
            info.min_version(),
            info.id(),
            diagnostics.join(","),
            attributes.join(","),
            files.join(","),
            info.index_sort()
        )
    };
    assert_eq!(
        describe(java_dir.path()),
        describe(rust_dir.path()),
        "the two .si files decode to different segments"
    );
}

#[test]
fn cfs_matches_lucene_byte_for_byte() {
    require_maven();
    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");
    run_java_fixture(java_dir.path(), "cfs");
    write_compound_file(rust_dir.path());
    // The sub-files must survive the packing unchanged, and the two products of
    // the packing must match: the data file and its entry table.
    for sub in &SUB_FILES {
        assert_file_bytes_equal(
            java_dir.path(),
            rust_dir.path(),
            &format!("_9{}", sub.extension),
            "cfs",
        );
    }
    assert_file_bytes_equal(java_dir.path(), rust_dir.path(), "_9.cfs", "cfs");
    assert_file_bytes_equal(java_dir.path(), rust_dir.path(), "_9.cfe", "cfe");
}

/// Covers the two-attribute field shape the real flush path produces.
///
/// `PerFieldPostingsFormat.FieldsWriter.write` puts two attributes on every
/// field it handles (`PerFieldPostingsFormat.java:253-254`), so this is the
/// `.fnm` an actual segment carries. Its bytes cannot be compared with Lucene's
/// directly: `writeMapOfStrings` walks the map in iteration order, which is a
/// property of `java.util.HashMap`'s bucket layout on one side and of
/// [`std::collections::HashMap`]'s per-process random seed on the other, so the
/// two key/value pairs can come out in either order.
///
/// What is still provable, and is what this test asserts, is that the two files
/// are the same *size* — so both sides encoded both attributes, and encoded
/// every other field identically — and that Rucene's own reader decodes
/// Lucene's file and Rucene's file into the same `FieldInfos`. A missing
/// attribute, a wrong key, a wrong value or any other difference in the file
/// would break one of the two.
///
/// Making this a byte comparison requires Rucene to reproduce
/// `java.util.HashMap`'s iteration order, which is a separate piece of work: it
/// affects the `.si` diagnostics, attributes and file list in the same way, and
/// it is the reason the cases above pin every collection to one entry.
#[test]
fn fnm_multi_attribute_matches_lucene_except_for_map_order() {
    require_maven();
    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");
    run_java_fixture(java_dir.path(), "fnm_multi_attribute");
    write_field_infos(rust_dir.path(), true);

    let java_bytes = std::fs::read(java_dir.path().join("_9.fnm")).expect("lucene .fnm");
    let rust_bytes = std::fs::read(rust_dir.path().join("_9.fnm")).expect("rucene .fnm");
    assert_eq!(
        java_bytes.len(),
        rust_bytes.len(),
        "the two .fnm files have different sizes, so they do not hold the same attributes"
    );

    let codec = ensure_codec();
    let read_back = |dir_path: &Path| -> Vec<String> {
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir_path).expect("open"));
        let info = segment_info(&dir, &codec);
        let infos = codec
            .field_infos_format()
            .read(dir.as_ref(), &info, "", &*DEFAULT_IO_CONTEXT)
            .expect("read field infos");
        infos
            .iter()
            .map(|fi| {
                let mut attributes: Vec<String> = fi
                    .attributes()
                    .into_iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                attributes.sort();
                format!(
                    "{}#{} options={:?} dv={:?} skip={:?} points={}/{}/{} vector={}/{:?}/{:?} \
                     tv={} omitNorms={} payloads={} soft={} parent={} attrs=[{}]",
                    fi.name,
                    fi.number,
                    fi.index_options,
                    fi.doc_values_type,
                    fi.doc_values_skip_index_type,
                    fi.point_dimension_count,
                    fi.point_index_dimension_count,
                    fi.point_num_bytes,
                    fi.vector_dimension,
                    fi.vector_encoding,
                    fi.vector_similarity_function,
                    fi.has_term_vectors(),
                    fi.omits_norms(),
                    fi.has_payloads(),
                    fi.is_soft_deletes_field(),
                    fi.is_parent_field(),
                    attributes.join(",")
                )
            })
            .collect()
    };
    assert_eq!(
        read_back(java_dir.path()),
        read_back(rust_dir.path()),
        "the two .fnm files decode to different field metadata"
    );
}
