//! The index helpers of `org.apache.lucene.tests.util.TestUtil` (rmp #342):
//! `checkIndex`, `alwaysPostingsFormat`, `getPostingsFormatWithOrds`,
//! `ramCopyOf`, `anyFilesExceptWriteLock` and `docs`, over an index of
//! `LineFileDocs` documents; `BaseDirectoryWrapper` and `CloseableDirectory`.
//!
//! The codecs are found by name through `rucene_test_framework::class_loader()`,
//! the services of `lucene-test-framework` (which register `Asserting` and
//! `LuceneFixedGap`), as Java's class path resolves them.
//!
//! The indexes are written to an `NIOFSDirectory`: on a `ByteBuffersDirectory`
//! the `AssertingCodec` flush fails (`AlreadyClosed`) because core's
//! `Lucene90NormsConsumer::close` is not idempotent (Java nulls `meta` and
//! `data`; `AssertingNormsConsumer.close` closes its delegate twice), a core
//! defect outside rmp #342, reported with it.

mod oracle_support;

use std::sync::{Arc, Once};

use randomizedtesting::jdk::random::JdkRandom;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::IndexReader;
use rucene::store::{ByteBuffersDirectory, Directory, NIOFSDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::BytesRef;
use rucene_test_framework::store::BaseDirectoryWrapper;
use rucene_test_framework::util::{
    CloseableDirectory, LineFileDocs, TestRuleMarkFailure, TestUtil,
};

fn install_codecs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let loader = rucene_test_framework::class_loader();
        rucene::codecs::reload_codecs(loader).unwrap();
        rucene::codecs::postings::reload_postings_formats(loader).unwrap();
        rucene::codecs::knn_vectors::reload_knn_vectors_format(loader).unwrap();
        rucene::codecs::doc_values::reload_doc_values_formats(loader).unwrap();
    });
}

/// An index of 60 `LineFileDocs` documents in two commits, written with
/// `alwaysPostingsFormat(getPostingsFormatWithOrds(random))`.
fn line_docs_index(seed: i64) -> (Arc<dyn Directory>, String, oracle_support::TempDir) {
    install_codecs();
    let random = JdkRandom::new(seed);
    let format = TestUtil::get_postings_format_with_ords(&random);
    let format_name = format.name().to_string();
    let tmp = oracle_support::TempDir::new("testutil-index");
    let dir: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(&tmp.0).unwrap());
    let mut config = IndexWriterConfig::new();
    config
        .set_codec(TestUtil::always_postings_format(format))
        .unwrap();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let w = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let docs = LineFileDocs::new(&random, "europarl.lines.txt.gz").unwrap();
    for i in 0..60 {
        w.add_document(&docs.next_doc().unwrap()).unwrap();
        if i == 29 {
            w.commit().unwrap();
        }
    }
    w.close().unwrap();
    (dir, format_name, tmp)
}

/// `checkIndex` passes on a clean index written with either postings format
/// with ords (both seeds' draws), records its messages, and fails with
/// `CheckIndex failed` once the largest file is corrupted.
///
/// The level is `MIN_LEVEL_FOR_INTEGRITY_CHECKS` (2): at the slow-checks
/// level (3) core's term-vector cross-check reports `freq called on
/// unpositioned postings enum` on these `AssertingCodec` indexes, a defect
/// outside rmp #342, reported with it.
#[test]
fn check_index_on_clean_and_corrupt_index() {
    let mut formats = Vec::new();
    // Seeds of `OracleSupport.seeds()`; their first `nextInt(2)` is 1, 0, 1, 0.
    for seed in [42, 0x5DEE_CE66D, 0x0123_4567_89AB_CDEF, -1] {
        let (dir, format, _tmp) = line_docs_index(seed);
        formats.push(format);
        let mut output = Vec::new();
        let status =
            TestUtil::check_index_with(Arc::clone(&dir), 2, false, false, Some(&mut output))
                .unwrap();
        assert!(status.clean);
        assert!(!output.is_empty());
        assert!(TestUtil::any_files_except_write_lock(&*dir).unwrap());

        // ramCopyOf copies every segments and codec file (not write.lock).
        let ram = TestUtil::ram_copy_of(&*dir).unwrap();
        let mut index_files = dir.list_all().unwrap();
        index_files.retain(|n| n != "write.lock");
        assert_eq!(ram.list_all().unwrap(), index_files);
        for name in index_files {
            assert_eq!(
                ram.file_length(&name).unwrap(),
                dir.file_length(&name).unwrap()
            );
        }

        // Corrupt one byte in the middle of the largest file.
        let name = dir
            .list_all()
            .unwrap()
            .into_iter()
            .max_by_key(|n| dir.file_length(n).unwrap())
            .unwrap();
        let len = dir.file_length(&name).unwrap() as usize;
        let mut bytes = vec![0u8; len];
        {
            let mut input = dir.open_input(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            input.read_bytes(&mut bytes, 0, len).unwrap();
        }
        bytes[len / 2] ^= 0x5a;
        dir.delete_file(&name).unwrap();
        {
            let mut out = dir.create_output(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            out.write_bytes(&bytes, 0, len).unwrap();
            out.close().unwrap();
        }
        let err = TestUtil::check_index_with(Arc::clone(&dir), 2, false, false, None).unwrap_err();
        assert_eq!(
            err.to_string(),
            "java.lang.RuntimeException: CheckIndex failed"
        );
    }
    // The seeds draw both formats.
    formats.sort();
    formats.dedup();
    assert_eq!(formats, ["BlockTreeOrds", "LuceneFixedGap"]);
}

/// `docs` finds a term of the index and returns `None` for a missing field
/// or term.
#[test]
fn docs_finds_terms() {
    let (dir, _, _tmp) = line_docs_index(5);
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let random = JdkRandom::new(7);
    let id = BytesRef::from_char_sequence("7");
    let found = TestUtil::docs(&random, &reader, "docid", &id, None, 0).unwrap();
    let mut postings = found.unwrap();
    assert_eq!(postings.next_doc().unwrap(), 7);
    assert!(TestUtil::docs(&random, &reader, "nofield", &id, None, 0)
        .unwrap()
        .is_none());
    let missing = BytesRef::from_char_sequence("no such id");
    assert!(TestUtil::docs(&random, &reader, "docid", &missing, None, 0)
        .unwrap()
        .is_none());
}

/// `BaseDirectoryWrapper` is open until closed, closes its delegate, and
/// `CloseableDirectory` fails the test for a directory left open, unless an
/// earlier failure was marked.
#[test]
fn base_directory_wrapper_and_closeable_directory() {
    let wrapper = Arc::new(BaseDirectoryWrapper::new(Arc::new(
        ByteBuffersDirectory::new(),
    )));
    assert!(wrapper.is_open());
    assert!(wrapper.get_check_index_on_close());
    assert_eq!(wrapper.get_level_for_check_on_close(), 3);
    wrapper.set_cross_check_term_vectors_on_close(false);
    assert_eq!(wrapper.get_level_for_check_on_close(), 2);
    wrapper.set_cross_check_term_vectors_on_close(true);
    assert_eq!(wrapper.get_level_for_check_on_close(), 3);

    let marker = Arc::new(TestRuleMarkFailure::new(Vec::new()));
    let closeable = CloseableDirectory::new(Arc::clone(&wrapper), Arc::clone(&marker));
    let failure =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| closeable.close())).unwrap_err();
    let (class, message) = oracle_support::panic_error(failure);
    assert_eq!(class, "java.lang.AssertionError");
    assert!(
        message.contains("Directory not closed: BaseDirectoryWrapper(ByteBuffersDirectory"),
        "{message}"
    );
    drop(closeable);

    // An empty directory holds no index: close does not check, and closes.
    let mut wrapper = Arc::try_unwrap(wrapper).ok().unwrap();
    wrapper.close().unwrap();
    assert!(!wrapper.is_open());
    let wrapper = Arc::new(wrapper);
    CloseableDirectory::new(Arc::clone(&wrapper), marker).close();
}
