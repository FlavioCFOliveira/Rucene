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
//! Task #177 closed that for the two collections whose Java implementation is a
//! real `java.util` hash collection: `FieldInfo.attributes`, which
//! `putAttribute` rebuilds as a `HashMap` (`FieldInfo.java:670-679`), and
//! `SegmentInfo`'s file list, which `setFiles` builds as a `HashSet`
//! (`SegmentInfo.java:309-320`). Both are reproduced by
//! [`rucene::util::java_collections`], so
//! [`fnm_multi_attribute_matches_lucene_byte_for_byte`] and
//! [`si_multi_file_matches_lucene_byte_for_byte`] are exact byte comparisons of
//! the shapes a real flush produces, and
//! [`rucenes_own_fnm_is_byte_stable_across_runs`] shows the order is not
//! accidental.
//!
//! The `.si` diagnostics and attributes are a different matter: Java holds
//! `Map.copyOf` results there, whose iteration order is drawn per JVM run, so
//! two runs of *Lucene* disagree with each other —
//! [`lucenes_own_si_diagnostics_order_is_not_stable_between_jvms`] measures it.
//! Those two stay at one entry in the cases below, which is all that can be
//! compared.

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
use rucene::util::java_collections::{JavaHashMap, JavaHashSet, JavaStringMap};
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
        // `SealBytesFixture.field` hands `new HashMap<>(attributes)` to the
        // constructor, where `attributes` was itself built by
        // `new HashMap<>()` plus one `put` per pair. Both steps matter: the
        // copy constructor pre-sizes its table from the source's size, so it
        // lands on a different table length — and therefore a different
        // iteration order — than the map it copied.
        JavaStringMap::Hash(JavaHashMap::copy_of(
            &attributes
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect::<JavaHashMap<String>>(),
        )),
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
    /// The six attributes a field handled by all three per-field formats
    /// carries. Six is the smallest of these shapes for which the copy
    /// constructor is observable: `new HashMap<>()` plus six puts fills a table
    /// of 16, while `new HashMap<>(m)` pre-sizes to 8, and these keys iterate
    /// differently at the two lengths — so this case is what pins
    /// `putAttribute`'s copy, which the two-attribute one cannot.
    const SIX: &[(&str, &str)] = &[
        ("PerFieldPostingsFormat.format", "Lucene104"),
        ("PerFieldPostingsFormat.suffix", "0"),
        ("PerFieldDocValuesFormat.format", "Lucene90"),
        ("PerFieldDocValuesFormat.suffix", "0"),
        (
            "PerFieldKnnVectorsFormat.format",
            "Lucene99HnswVectorsFormat",
        ),
        ("PerFieldKnnVectorsFormat.suffix", "0"),
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
            if multi_attribute { SIX } else { NONE },
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
///
/// The extensions are chosen so that the names collide in the backing table:
/// at the default capacity of 16, `_9.fdt` and `_9.tvx` share a bucket, and so
/// do `_9.dvd` and `_9.tvd`. Only a shared bucket puts the chain order — and
/// therefore the order the names were added in — into the file; with pairwise
/// distinct buckets every insertion order yields the same bytes and the case
/// would prove nothing about the order.
const SI_EXTRA_FILES: [&str; 5] = [".fdt", ".tvx", ".dvd", ".tvd", ".nvm"];

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
    // `SealBytesFixture.writeSegmentInfo` fills a `new HashSet<>()` in the
    // order of `SI_EXTRA_FILES` and hands it to `setFiles`, which fills a
    // second `new HashSet<>()` from the first one's iteration order.
    let files: JavaHashSet = if multi_file {
        SI_EXTRA_FILES
            .iter()
            .map(|extension| format!("{SEGMENT}{extension}"))
            .collect()
    } else {
        JavaHashSet::new()
    };
    info.set_files(files);
    codec
        .segment_info_format()
        .write(dir.as_ref(), &info, &*DEFAULT_IO_CONTEXT)
        .expect("write segment info");
}

/// The diagnostics of the multi-entry case; mirrors
/// `SealBytesFixture.writeMultiDiagnosticSegmentInfo`.
const MULTI_DIAGNOSTICS: [(&str, &str); 8] = [
    ("source", "flush"),
    ("lucene.version", "10.5.0"),
    ("os", "Linux"),
    ("os.arch", "amd64"),
    ("os.version", "6.8.0"),
    ("java.version", "21"),
    ("java.vendor", "Ubuntu"),
    ("timestamp", "20260905000000000"),
];

/// Writes a `.si` whose diagnostics carry several entries.
fn write_multi_diagnostic_segment_info(out_dir: &Path) {
    let codec = ensure_codec();
    let dir = open_dir(out_dir);
    let diagnostics: HashMap<String, String> = MULTI_DIAGNOSTICS
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let mut attributes = HashMap::new();
    attributes.insert(
        "Lucene90StoredFieldsFormat.mode".to_string(),
        "BEST_SPEED".to_string(),
    );
    let info = SegmentInfo::new(
        Arc::clone(&dir),
        Version::LATEST,
        Some(Version::LATEST),
        SEGMENT.to_string(),
        MAX_DOC,
        false,
        false,
        Arc::clone(&codec),
        diagnostics,
        SEGMENT_ID,
        attributes,
        Default::default(),
    )
    .expect("segment info");
    info.set_files(JavaHashSet::new());
    codec
        .segment_info_format()
        .write(dir.as_ref(), &info, &*DEFAULT_IO_CONTEXT)
        .expect("write segment info");
}

/// Writes the sub-files of the compound-file case and packs them.
fn write_compound_file(out_dir: &Path) {
    let codec = ensure_codec();
    let dir = open_dir(out_dir);
    let mut files = JavaHashSet::new();
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

/// The `.si` Rucene writes is the same on every run of Rucene, diagnostics
/// included.
///
/// The counterpart of
/// [`lucenes_own_si_diagnostics_order_is_not_stable_between_jvms`]: Lucene's
/// multi-entry diagnostics order is drawn per JVM, Rucene's is fixed. The
/// diagnostics here are the eight entries a real flush writes, and the input is
/// handed over as a [`std::collections::HashMap`] on purpose — the same
/// unordered container `IndexWriter` would hand over — so the test fails if
/// anything downstream of the constructor lets that container's order through.
#[test]
fn rucenes_own_si_is_byte_stable_across_runs() {
    let mut first: Option<Vec<u8>> = None;
    for run in 0..8 {
        let dir = tempfile::tempdir().expect("rucene temp dir");
        write_multi_diagnostic_segment_info(dir.path());
        let bytes = std::fs::read(dir.path().join("_9.si")).expect("rucene .si");
        match &first {
            None => first = Some(bytes),
            Some(expected) => assert_eq!(
                &bytes, expected,
                "run {run} wrote different .si bytes than the first run"
            ),
        }
    }
}

/// Covers the multi-file `.si` a real segment carries, byte for byte.
///
/// A sealed segment always lists more than one file, and
/// `Lucene99SegmentInfoFormat.write` serialises that list with
/// `writeSetOfStrings` (`Lucene99SegmentInfoFormat.java:220`), which walks the
/// set in iteration order. That set is a `java.util.HashSet` on the Java side,
/// and since task #177 it is a [`JavaHashSet`] here, so the order is
/// reproducible and the comparison is the same exact byte comparison the
/// single-file case makes.
///
/// The `.si` diagnostics and attributes stay at one entry in this case on
/// purpose: those two maps are `Map.copyOf` results in Java, whose order is
/// drawn per JVM run, so a multi-entry version of them could not be compared
/// with anything — see
/// [`lucenes_own_si_diagnostics_order_is_not_stable_between_jvms`].
#[test]
fn si_multi_file_matches_lucene_byte_for_byte() {
    require_maven();
    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");
    run_java_fixture(java_dir.path(), "si_multi_file");
    write_segment_info(rust_dir.path(), true);
    assert_file_bytes_equal(java_dir.path(), rust_dir.path(), "_9.si", "si_multi_file");
}

/// Measures why the `.si` diagnostics cannot be compared byte for byte beyond
/// one entry: Lucene does not agree with *itself* about their order.
///
/// `SegmentInfo`'s constructor stores `Map.copyOf(diagnostics)`
/// (`SegmentInfo.java:137`), and `Map.copyOf` returns a
/// `java.util.ImmutableCollections.MapN` whose iteration is seeded by a
/// `SALT32L` drawn once per JVM at class initialisation. This runs the very
/// same fixture, with every input pinned, in a series of fresh JVMs and shows
/// that the `.si` files they write differ. There is therefore no order for a
/// port to reproduce here, which is why
/// [`JavaStringMap::immutable_copy_of`](rucene::util::java_collections::JavaStringMap::immutable_copy_of)
/// picks a fixed one instead — Rucene's own output is byte-stable, which
/// Lucene's is not, and no reader can tell either way.
#[test]
fn lucenes_own_si_diagnostics_order_is_not_stable_between_jvms() {
    require_maven();
    const RUNS: usize = 10;
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for _ in 0..RUNS {
        let dir = tempfile::tempdir().expect("java temp dir");
        run_java_fixture(dir.path(), "si_multi_diagnostic");
        seen.push(std::fs::read(dir.path().join("_9.si")).expect("lucene .si"));
    }
    let lengths: std::collections::HashSet<usize> = seen.iter().map(Vec::len).collect();
    assert_eq!(
        lengths.len(),
        1,
        "the runs did not even write the same number of bytes, so the inputs \
         were not pinned"
    );
    let distinct: std::collections::HashSet<&Vec<u8>> = seen.iter().collect();
    assert!(
        distinct.len() > 1,
        "{RUNS} JVMs all wrote the same .si bytes for a multi-entry diagnostics \
         map. Either java.util.ImmutableCollections stopped randomising its \
         iteration order, in which case this port should reproduce the new one, \
         or SegmentInfo stopped using Map.copyOf. Re-read both before trusting \
         the declared divergence."
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

/// Covers the two-attribute field shape the real flush path produces, byte for
/// byte.
///
/// `PerFieldPostingsFormat.FieldsWriter.write` puts two attributes on every
/// field it handles (`PerFieldPostingsFormat.java:253-254`), so this is the
/// `.fnm` an actual segment carries. `writeMapOfStrings` walks the map in
/// iteration order, which is `java.util.HashMap`'s bucket order; since task
/// #177 [`JavaHashMap`] reproduces it, including the effect of the copy
/// constructor `putAttribute` and the fixture both go through, so the two files
/// are identical rather than merely the same size.
#[test]
fn fnm_multi_attribute_matches_lucene_byte_for_byte() {
    require_maven();
    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");
    run_java_fixture(java_dir.path(), "fnm_multi_attribute");
    write_field_infos(rust_dir.path(), true);
    assert_file_bytes_equal(
        java_dir.path(),
        rust_dir.path(),
        "_9.fnm",
        "fnm_multi_attribute",
    );
}

/// The `.fnm` Rucene writes is the same on every run of Rucene.
///
/// The counterpart of the byte comparison above: that one proves the order
/// agrees with Lucene's, this one proves it is not accidental. Before task #177
/// the attributes lived in a [`std::collections::HashMap`], whose iteration
/// order comes from a per-process random seed, so two runs of Rucene wrote
/// different bytes. Each iteration here writes from a freshly built
/// `FieldInfos` in a fresh directory.
#[test]
fn rucenes_own_fnm_is_byte_stable_across_runs() {
    let mut first: Option<Vec<u8>> = None;
    for run in 0..8 {
        let dir = tempfile::tempdir().expect("rucene temp dir");
        write_field_infos(dir.path(), true);
        let bytes = std::fs::read(dir.path().join("_9.fnm")).expect("rucene .fnm");
        match &first {
            None => first = Some(bytes),
            Some(expected) => assert_eq!(
                &bytes, expected,
                "run {run} wrote different .fnm bytes than the first run"
            ),
        }
    }
}
