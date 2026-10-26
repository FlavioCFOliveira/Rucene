use std::sync::Arc;

use rucene::document::{Document, Store, StringField, TextField};
use rucene::index::check_index::{CheckIndex, Level};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::store::{Directory, RamDirectory};

/// The integration crate does not share the library's test-time codec registry,
/// so every entry point registers the default codec before writing an index.
fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
}

/// A directory of this test binary's own, unique to the calling test.
///
/// The process id alone is not enough: every test in this binary shares it, so
/// two tests that both wrote on disk would meet in the same directory and, with
/// them, on the same `write.lock`. The counter makes the name unique per test.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("rucene-smoke-{}-{seq}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn build(dir: Arc<dyn Directory>, segments: usize) {
    ensure_codec();
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for s in 0..segments {
        for d in 0..5 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", format!("{s}-{d}"), Store::YES).unwrap(),
            ));
            doc.add(Box::new(
                TextField::new("body", format!("hello world {s} {d}"), Store::YES).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
    }
    writer.close().unwrap();
}

#[test]
fn sequential_and_concurrent_checks_agree() {
    for threads in [1i32, 4] {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        build(Arc::clone(&dir), 3);

        let mut checker = CheckIndex::with_write_lock(Arc::clone(&dir), None);
        checker.set_level(Level::MAX_VALUE).unwrap();
        checker.set_thread_count(threads).unwrap();
        let status = checker.check_index().unwrap();
        checker.close().unwrap();

        assert!(status.clean, "threads={threads} status={status:?}");
        assert_eq!(status.num_segments, 3, "threads={threads}");
        assert_eq!(status.segment_infos.len(), 3, "threads={threads}");
        assert_eq!(status.num_bad_segments, 0, "threads={threads}");
        assert!(status.valid_counter, "threads={threads}");
        for seg in &status.segment_infos {
            assert!(seg.open_reader_passed);
            assert_eq!(seg.max_doc, 5);
            assert!(seg.error.is_none());
        }
    }
}

#[test]
fn only_segments_marks_the_status_partial() {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    build(Arc::clone(&dir), 2);

    let mut checker = CheckIndex::with_write_lock(Arc::clone(&dir), None);
    checker.set_level(Level::MAX_VALUE).unwrap();
    let all = checker.check_index().unwrap();
    let first = all.segment_infos[0].name.clone();
    let status = checker
        .check_index_segments(Some(&[first.clone()]))
        .unwrap();
    checker.close().unwrap();

    assert!(status.partial);
    assert_eq!(status.segments_checked, vec![first]);
    assert_eq!(status.segment_infos.len(), 1);
}

#[test]
fn do_main_reports_a_clean_index() {
    let tmp = scratch_dir("do-main");
    let dir: Arc<dyn Directory> = Arc::new(rucene::store::FSDirectory::open(&tmp).unwrap());
    build(Arc::clone(&dir), 2);
    drop(dir);

    let code = rucene::index::check_index::do_main(&[
        tmp.to_string_lossy().to_string(),
        "-level".to_string(),
        "3".to_string(),
    ])
    .unwrap();
    assert_eq!(code, 0);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn upgrader_parse_args_rejects_a_missing_directory() {
    let empty: [String; 0] = [];
    assert!(rucene::index::check_index::IndexUpgrader::parse_args(&empty).is_err());
    assert!(
        rucene::index::check_index::IndexUpgrader::parse_args(&["-dir-impl".to_string()]).is_err()
    );
}

/// Regression for an index-compatibility defect in the block-tree terms reader.
///
/// `Lucene103BlockTreeTermsWriter` always writes both the field's first and last
/// term into the `.tmd` (`Lucene103BlockTreeTermsWriter.java:1091-1092`), and
/// `Lucene103BlockTreeTermsReader` always reads both; its `numTerms == 1` branch
/// only aliases `maxTerm` to `minTerm` afterwards, to save an object. This port
/// skipped the *read* in that branch, leaving the max term's length and bytes in
/// the stream. Any segment holding a field with exactly one term — which is
/// every single-document segment, and any segment whose `id`-like field happens
/// to hold one value — was therefore unreadable: the following field decoded
/// garbage, and with a single field the `.tmd` footer landed two bytes early
/// ("misplaced codec footer (file extended?): remaining=18, expected=16").
///
/// The existing suites all indexed five or more documents per segment, so no
/// field ever had exactly one term and none of them saw it.
#[test]
fn a_segment_whose_fields_have_a_single_term_can_be_read_back() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", "only".to_string(), Store::YES).unwrap(),
    ));
    doc.add(Box::new(
        TextField::new("body", "solo".to_string(), Store::YES).unwrap(),
    ));
    writer.add_document(&doc).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = rucene::index::directory_reader::open(Arc::clone(&dir))
        .expect("a one-term segment must be readable");
    assert_eq!(reader.max_doc(), 1);

    // Both fields decode, and their single term is both the min and the max.
    let leaves = Arc::clone(&reader).leaves();
    let leaf = leaves[0].leaf_reader();
    for (field, term) in [("id", "only"), ("body", "solo")] {
        let terms = rucene::index::leaf_reader::LeafReader::terms(leaf.as_ref(), field)
            .expect("terms")
            .unwrap_or_else(|| panic!("field {field} must have a terms dictionary"));
        assert_eq!(terms.size().unwrap(), 1);
        assert_eq!(terms.min().unwrap().unwrap().slice(), term.as_bytes());
        assert_eq!(terms.max().unwrap().unwrap().slice(), term.as_bytes());
    }

    // And CheckIndex agrees the segment is sound.
    let mut checker = CheckIndex::with_write_lock(Arc::clone(&dir), None);
    checker.set_level(Level::MAX_VALUE).unwrap();
    let status = checker.check_index().expect("check index");
    checker.close().unwrap();
    assert!(status.clean, "{status:?}");
}

/// `IndexUpgrader.main(String[])` is `parseArgs(args).upgrade()`; the port
/// exposes that body as `IndexUpgrader::do_main`, so a bad command line is
/// rejected before any directory is touched and a real index is upgraded in
/// place.
#[test]
fn upgrader_do_main_parses_then_upgrades() {
    let empty: [String; 0] = [];
    assert!(rucene::index::check_index::IndexUpgrader::do_main(&empty).is_err());

    let tmp = scratch_dir("upgrader-do-main");
    let dir: Arc<dyn Directory> = Arc::new(rucene::store::FSDirectory::open(&tmp).unwrap());
    build(Arc::clone(&dir), 1);
    let before = rucene::index::SegmentInfos::read_latest_commit(Arc::clone(&dir))
        .unwrap()
        .generation();
    drop(dir);

    rucene::index::check_index::IndexUpgrader::do_main(&[tmp.to_string_lossy().to_string()])
        .expect("upgrade an index already in the current format");

    let dir: Arc<dyn Directory> = Arc::new(rucene::store::FSDirectory::open(&tmp).unwrap());
    let after = rucene::index::SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    // The index was already in the current format, so `UpgradeIndexMergePolicy`
    // finds nothing to rewrite and the commit point is left where it was; what
    // the run proves is that `do_main` parsed, opened, upgraded and closed
    // without disturbing the index.
    assert!(after.generation() >= before);
    assert_eq!(after.size(), 1, "the single segment survives the upgrade");
    let reader = rucene::index::directory_reader::open(Arc::clone(&dir))
        .expect("the upgraded index must still open");
    assert_eq!(reader.max_doc(), 5);
    drop(dir);
    let _ = std::fs::remove_dir_all(&tmp);
}
