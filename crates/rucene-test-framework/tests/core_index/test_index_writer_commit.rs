//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterCommit.java`.
//!
//! * `TestIndexWriter.addDoc`, `addDocWithIndex` and
//!   `assertNoUnreferencedFiles` are `support.rs`'s ports.
//! * `w.setLiveCommitData(commitData.entrySet())` hands Rucene a snapshot of
//!   the map's entries; Java hands the live entry set, which the writer reads
//!   at commit (`testCommitDataIsLive` depends on that).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::NoDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::TermQuery;
use rucene::store::Directory;
use rucene_test_framework::analysis::mock_tokenizer::MockTokenizer;
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, MockAnalyzer, MockFixedLengthPayloadFilter,
};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, random, test, verbose,
};
use crate::support::{add_doc, add_doc_with_index, assert_no_unreferenced_files, npe};

/// `TestIndexWriterCommit`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterCommit",
        vec![
            test("testCommitOnClose", test_commit_on_close),
            test("testCommitOnCloseAbort", test_commit_on_close_abort),
            test(
                "testCommitOnCloseDiskUsage",
                test_commit_on_close_disk_usage,
            )
            .annotate(Nightly),
            test(
                "testCommitOnCloseForceMerge",
                test_commit_on_close_force_merge,
            ),
            test("testCommitThreadSafety", test_commit_thread_safety).annotate(Nightly),
            test("testForceCommit", test_force_commit),
            test("testFutureCommit", test_future_commit),
            test("testZeroCommits", test_zero_commits),
            test("testPrepareCommit", test_prepare_commit),
            test("testPrepareCommitRollback", test_prepare_commit_rollback),
            test("testPrepareCommitNoChanges", test_prepare_commit_no_changes),
            test("testCommitUserData", test_commit_user_data),
            test("testPrepareCommitThenClose", test_prepare_commit_then_close),
            test("testCommitDataIsLive", test_commit_data_is_live),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `mock_iwc()` with `setMaxBufferedDocs(max)`.
fn iwc_max_buffered(max: i32) -> Result<IndexWriterConfig> {
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(max)?;
    Ok(iwc)
}

/// `newSearcher(r).search(new TermQuery(term), 1000).scoreDocs.length`.
fn hit_count(r: &Arc<dyn DirectoryReader>, term: &Term) -> Result<usize> {
    let searcher = LuceneTestCase::new_searcher(Arc::clone(r));
    Ok(searcher
        .search(Arc::new(TermQuery::new(term.clone())), 1000)?
        .score_docs
        .len())
}

/// Simple test for "commit on close": open writer then
/// add a bunch of docs, making sure reader does not see
/// these docs until writer is closed.
/// Port of `TestIndexWriterCommit.testCommitOnClose`.
fn test_commit_on_close() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let writer = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    for _ in 0..14 {
        add_doc(&writer)?;
    }
    writer.close()?;

    let search_term = Term::from_text("content", "aaa");
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(
        Some("first number of hits"),
        &14,
        &hit_count(&reader, &search_term)?,
    );
    reader.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;

    let writer = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    for _ in 0..3 {
        for _ in 0..11 {
            add_doc(&writer)?;
        }
        let r = directory_reader::open(Arc::clone(&d))?;
        assert_equals_msg(
            Some("reader incorrectly sees changes from writer"),
            &14,
            &hit_count(&r, &search_term)?,
        );
        r.close()?;
        Assert::assert_true_msg(
            "reader should have still been current",
            reader.is_current()?,
        );
    }

    writer.close()?;
    Assert::assert_false_msg("reader should not be current now", reader.is_current()?);

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(
        Some("reader did not see changes after writer was closed"),
        &47,
        &hit_count(&r, &search_term)?,
    );
    r.close()?;
    reader.close()?;
    drop(d);
    dir.close()
}

/// Simple test for "commit on close": open writer, then
/// add a bunch of docs, making sure reader does not see
/// them until writer has closed.  Then instead of
/// closing the writer, call abort and verify reader sees
/// nothing was added.  Then verify we can open the index
/// and add docs to it.
/// Port of `TestIndexWriterCommit.testCommitOnCloseAbort`.
fn test_commit_on_close_abort() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let writer = IndexWriter::new(Arc::clone(&d), iwc_max_buffered(10)?)?;
    for _ in 0..14 {
        add_doc(&writer)?;
    }
    writer.close()?;

    let search_term = Term::from_text("content", "aaa");
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(
        Some("first number of hits"),
        &14,
        &hit_count(&reader, &search_term)?,
    );
    reader.close()?;

    let iwc = iwc_max_buffered(10)?;
    iwc.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    for _ in 0..17 {
        add_doc(&writer)?;
    }
    writer.delete_documents_by_terms(vec![search_term.clone()])?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(
        Some("reader incorrectly sees changes from writer"),
        &14,
        &hit_count(&reader, &search_term)?,
    );
    reader.close()?;

    writer.rollback()?;

    assert_no_unreferenced_files(&d, "unreferenced files remain after rollback()")?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(
        Some("saw changes after writer.abort"),
        &14,
        &hit_count(&reader, &search_term)?,
    );
    reader.close()?;

    let iwc = iwc_max_buffered(10)?;
    iwc.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;

    for _ in 0..12 {
        for _ in 0..17 {
            add_doc(&writer)?;
        }
        let r = directory_reader::open(Arc::clone(&d))?;
        assert_equals_msg(
            Some("reader incorrectly sees changes from writer"),
            &14,
            &hit_count(&r, &search_term)?,
        );
        r.close()?;
    }

    writer.close()?;
    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(
        Some("didn't see changes after close"),
        &218,
        &hit_count(&r, &search_term)?,
    );
    r.close()?;

    drop(d);
    dir.close()
}

/// The two anonymous `Analyzer`s of `testCommitOnCloseDiskUsage`: whitespace
/// tokens, with fixed-length payloads when `payload_length` is set.
#[derive(Debug)]
struct DiskUsageAnalyzer {
    state: AnalyzerState,
    payload_length: Option<i32>,
}

impl Analyzer for DiskUsageAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        if self.payload_length.is_some() {
            "org.apache.lucene.index.TestIndexWriterCommit$2"
        } else {
            "org.apache.lucene.index.TestIndexWriterCommit$1"
        }
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::whitespace_lower_case()?));
        match self.payload_length {
            None => Ok(tokenizer_components(tokenizer)),
            Some(length) => {
                let filter =
                    MockFixedLengthPayloadFilter::new(random(), shared(&tokenizer), length)?;
                Ok(components(tokenizer, sink(filter)))
            }
        }
    }
}

/// Verify that a writer with "commit on close" indeed
/// cleans up the temp segments created after opening
/// that are not referenced by the starting segments
/// file.  We check this by using MockDirectoryWrapper to
/// measure max temp disk space used.
/// Port of `TestIndexWriterCommit.testCommitOnCloseDiskUsage`.
fn test_commit_on_close_disk_usage() -> Result<()> {
    let dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let d = dir.as_directory();
    let analyzer: Arc<dyn Analyzer> = if random().next_boolean() {
        Arc::new(DiskUsageAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            payload_length: None,
        })
    } else {
        let length = random().next_int_bound(200);
        Arc::new(DiskUsageAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            payload_length: Some(length),
        })
    };

    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    iwc.set_max_buffered_docs(10)?;
    iwc.set_reader_pooling(false);
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    for j in 0..30 {
        add_doc_with_index(&writer, j)?;
    }
    writer.close()?;
    let mock = dir.as_mock().unwrap_or_else(|| npe("dir"));
    mock.reset_max_used_size_in_bytes()?;

    mock.set_track_disk_usage(true);
    let start_disk_usage = mock.get_max_used_size_in_bytes();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    iwc.set_open_mode(OpenMode::APPEND)?;
    iwc.set_max_buffered_docs(10)?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    iwc.set_reader_pooling(false);
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;

    for j in 0..1470 {
        add_doc_with_index(&writer, j)?;
    }
    let mid_disk_usage = mock.get_max_used_size_in_bytes();
    mock.reset_max_used_size_in_bytes()?;
    writer.force_merge(1)?;
    writer.close()?;

    directory_reader::open(Arc::clone(&d))?.close()?;

    let end_disk_usage = mock.get_max_used_size_in_bytes();

    Assert::assert_true_msg(
        &format!(
            "writer used too much space while adding documents: mid={mid_disk_usage} start={start_disk_usage} end={end_disk_usage} max={}",
            start_disk_usage * 150
        ),
        mid_disk_usage < 150 * start_disk_usage,
    );
    Assert::assert_true_msg(
        &format!(
            "writer used too much space after close: endDiskUsage={end_disk_usage} startDiskUsage={start_disk_usage} max={}",
            start_disk_usage * 150
        ),
        end_disk_usage < 150 * start_disk_usage,
    );
    drop(d);
    dir.close()
}

/// Verify that calling forceMerge when writer is open for
/// "commit on close" works correctly both for rollback()
/// and close().
/// Port of `TestIndexWriterCommit.testCommitOnCloseForceMerge`.
fn test_commit_on_close_force_merge() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = iwc_max_buffered(10)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    for j in 0..17 {
        add_doc_with_index(&writer, j)?;
    }
    writer.close()?;

    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    writer.force_merge(1)?;

    let reader = directory_reader::open(Arc::clone(&d))?;

    Assert::assert_true_msg(
        "Reader incorrectly sees one segment",
        (Arc::clone(&reader) as Arc<dyn IndexReader>)
            .leaves()?
            .len()
            > 1,
    );
    reader.close()?;

    writer.rollback()?;
    assert_no_unreferenced_files(&d, "aborted writer after forceMerge")?;

    let reader = directory_reader::open(Arc::clone(&d))?;

    Assert::assert_true_msg(
        "Reader incorrectly sees one segment",
        (Arc::clone(&reader) as Arc<dyn IndexReader>)
            .leaves()?
            .len()
            > 1,
    );
    reader.close()?;

    if verbose() {
        println!("TEST: do real full merge");
    }
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    writer.force_merge(1)?;
    writer.close()?;

    if verbose() {
        println!("TEST: writer closed");
    }
    assert_no_unreferenced_files(&d, "aborted writer after forceMerge")?;

    let reader = directory_reader::open(Arc::clone(&d))?;

    assert_equals_msg(
        Some("Reader incorrectly sees more than one segment"),
        &1,
        &(Arc::clone(&reader) as Arc<dyn IndexReader>)
            .leaves()?
            .len(),
    );
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterCommit.testCommitThreadSafety`.
fn test_commit_thread_safety() -> Result<()> {
    const NUM_THREADS: usize = 5;
    let max_iterations = 10;
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = Arc::new(RandomIndexWriter::new_with_config(
        &*random(),
        Arc::clone(&d),
        iwc,
    )?);
    TestUtil::reduce_open_files(w.w())?;
    w.commit()?;
    let failed = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::with_capacity(NUM_THREADS);
    for i in 0..NUM_THREADS {
        let final_i = i;
        let w = Arc::clone(&w);
        let d = Arc::clone(&d);
        let failed = Arc::clone(&failed);
        let thread = Thread::unnamed(move || {
            let r = (|| -> Result<()> {
                let mut doc = Document::new();
                let mut r = directory_reader::open(Arc::clone(&d))?;
                let f = LuceneTestCase::new_string_field("f", "", Store::NO);
                doc.add(Box::new(f));
                let mut iterations = 0;
                let mut count = 0;
                loop {
                    if failed.load(Ordering::SeqCst) {
                        break;
                    }
                    for _ in 0..10 {
                        let s = format!("{final_i}_{count}");
                        count += 1;
                        crate::support::field_mut(&mut doc, "f").set_string_value(s.clone())?;
                        w.add_document(&doc)?;
                        w.commit()?;
                        let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
                        assert_not_null(&r2);
                        let r2 = r2.unwrap_or_else(|| npe("r2"));
                        Assert::assert_true(!Arc::ptr_eq(&r2, &r));
                        r.close()?;
                        r = r2;
                        assert_equals_msg(
                            Some(&format!("term=f:{s}; r={r}")),
                            &1,
                            &r.doc_freq(&Term::from_text("f", &s))?,
                        );
                    }
                    iterations += 1;
                    if iterations >= max_iterations {
                        break;
                    }
                }
                r.close()
            })();
            if let Err(t) = r {
                failed.store(true, Ordering::SeqCst);
                rethrow_java(LuceneError::runtime_exception(t));
            }
        });
        thread.start();
        threads.push(thread);
    }
    for thread in &threads {
        if let Err(payload) = thread.join() {
            failed.store(true, Ordering::SeqCst);
            std::panic::resume_unwind(payload);
        }
    }
    Assert::assert_false(failed.load(Ordering::SeqCst));
    w.close()?;
    drop((w, d));
    dir.close()
}

/// Port of `TestIndexWriterCommit.testForceCommit`.
fn test_force_commit() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let iwc = iwc_max_buffered(2)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(5)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    writer.commit()?;

    for _ in 0..23 {
        add_doc(&writer)?;
    }

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());
    writer.commit()?;
    let reader2 = directory_reader::open_if_changed(Arc::clone(&reader))?;
    assert_not_null(&reader2);
    let reader2 = reader2.unwrap_or_else(|| npe("reader2"));
    assert_equals(&0, &reader.num_docs());
    assert_equals(&23, &reader2.num_docs());
    reader.close()?;

    for _ in 0..17 {
        add_doc(&writer)?;
    }
    assert_equals(&23, &reader2.num_docs());
    reader2.close()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&23, &reader.num_docs());
    reader.close()?;
    writer.commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&40, &reader.num_docs());
    reader.close()?;
    writer.close()?;
    drop(d);
    dir.close()
}

/// The commit whose user data's `tag` is `tag`.
fn commit_tagged(d: &Arc<dyn Directory>, tag: &str) -> Result<Option<Arc<dyn IndexCommit>>> {
    for c in directory_reader::list_commits(Arc::clone(d))? {
        if c.get_user_data()?.get("tag").map(String::as_str) == Some(tag) {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

/// Port of `TestIndexWriterCommit.testFutureCommit`.
fn test_future_commit() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let iwc = mock_iwc();
    iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let doc = Document::new();
    w.add_document(&doc)?;

    let mut commit_data: HashMap<String, String> = HashMap::new();
    commit_data.insert("tag".to_string(), "first".to_string());
    w.set_live_commit_data(commit_data.clone())?;
    w.commit()?;

    w.add_document(&doc)?;
    commit_data.insert("tag".to_string(), "second".to_string());
    w.set_live_commit_data(commit_data.clone())?;
    w.close()?;

    let commit = commit_tagged(&d, "first")?;

    assert_not_null(&commit);

    let iwc = mock_iwc();
    iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
    iwc.set_index_commit(commit);
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;

    assert_equals(&1, &w.get_doc_stats()?.num_docs);

    w.add_document(&doc)?;
    commit_data.insert("tag".to_string(), "third".to_string());
    w.set_live_commit_data(commit_data.clone())?;
    w.close()?;

    let commit = commit_tagged(&d, "second")?;

    assert_not_null(&commit);

    drop((commit, d));
    dir.close()
}

/// Port of `TestIndexWriterCommit.testZeroCommits`.
fn test_zero_commits() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let writer = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    expect_throws("org.apache.lucene.index.IndexNotFoundException", || {
        directory_reader::list_commits(Arc::clone(&d))
    });

    writer.close()?;
    assert_equals_msg(
        Some("expected 1 commits!"),
        &1,
        &directory_reader::list_commits(Arc::clone(&d))?.len(),
    );
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterCommit.testPrepareCommit`.
fn test_prepare_commit() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let iwc = iwc_max_buffered(2)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(5)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    writer.commit()?;

    for _ in 0..23 {
        add_doc(&writer)?;
    }

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());

    writer.prepare_commit()?;

    let reader2 = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader2.num_docs());

    writer.commit()?;

    let reader3 = directory_reader::open_if_changed(Arc::clone(&reader))?;
    assert_not_null(&reader3);
    let reader3 = reader3.unwrap_or_else(|| npe("reader3"));
    assert_equals(&0, &reader.num_docs());
    assert_equals(&0, &reader2.num_docs());
    assert_equals(&23, &reader3.num_docs());
    reader.close()?;
    reader2.close()?;

    for _ in 0..17 {
        add_doc(&writer)?;
    }

    assert_equals(&23, &reader3.num_docs());
    reader3.close()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&23, &reader.num_docs());
    reader.close()?;

    writer.prepare_commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&23, &reader.num_docs());
    reader.close()?;

    writer.commit()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&40, &reader.num_docs());
    reader.close()?;
    writer.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterCommit.testPrepareCommitRollback`.
fn test_prepare_commit_rollback() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let iwc = iwc_max_buffered(2)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(5)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    writer.commit()?;

    for _ in 0..23 {
        add_doc(&writer)?;
    }

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());

    writer.prepare_commit()?;

    let reader2 = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader2.num_docs());

    writer.rollback()?;

    let reader3 = directory_reader::open_if_changed(Arc::clone(&reader))?;
    assert_null(&reader3.map(|_| ()));
    assert_equals(&0, &reader.num_docs());
    assert_equals(&0, &reader2.num_docs());
    reader.close()?;
    reader2.close()?;

    let writer = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    for _ in 0..17 {
        add_doc(&writer)?;
    }

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());
    reader.close()?;

    writer.prepare_commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());
    reader.close()?;

    writer.commit()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&17, &reader.num_docs());
    reader.close()?;
    writer.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterCommit.testPrepareCommitNoChanges`.
fn test_prepare_commit_no_changes() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let writer = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    writer.prepare_commit()?;
    writer.commit()?;
    writer.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterCommit.testCommitUserData`.
fn test_commit_user_data() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), iwc_max_buffered(2)?)?;
    for _ in 0..17 {
        add_doc(&w)?;
    }
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &r.index_commit()?.get_user_data()?.len());
    r.close()?;

    let w = IndexWriter::new(Arc::clone(&d), iwc_max_buffered(2)?)?;
    for _ in 0..17 {
        add_doc(&w)?;
    }
    let mut data: HashMap<String, String> = HashMap::new();
    data.insert("label".to_string(), "test1".to_string());
    w.set_live_commit_data(data.clone())?;
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(
        &Some("test1".to_string()),
        &r.index_commit()?.get_user_data()?.get("label").cloned(),
    );
    r.close()?;

    let w = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    w.force_merge(1)?;
    w.close()?;

    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterCommit.testPrepareCommitThenClose`.
fn test_prepare_commit_then_close() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    w.add_document(&Document::new())?;

    w.prepare_commit()?;
    expect_throws("java.lang.IllegalStateException", || w.close());
    w.commit()?;
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&1, &r.max_doc());
    r.close()?;
    drop(d);
    dir.close()
}

/// `commitData.entrySet()`: a live view of the test's map, which the writer
/// walks when it commits.
#[derive(Clone)]
struct EntrySet(Arc<Mutex<HashMap<String, String>>>);

impl EntrySet {
    fn put(&self, key: &str, value: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
    }

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

impl IntoIterator for EntrySet {
    type Item = (String, String);
    type IntoIter = std::vec::IntoIter<(String, String)>;

    fn into_iter(self) -> Self::IntoIter {
        let entries: Vec<(String, String)> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        entries.into_iter()
    }
}

/// Port of `TestIndexWriterCommit.testCommitDataIsLive`.
fn test_commit_data_is_live() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), mock_iwc())?;
    w.add_document(&Document::new())?;

    let commit_data = EntrySet(Arc::new(Mutex::new(HashMap::new())));
    commit_data.put("foo", "bar");

    // make sure "foo" / "bar" doesn't take
    w.set_live_commit_data(commit_data.clone())?;

    commit_data.clear();
    commit_data.put("boo", "baz");

    // this finally does the commit, and should burn "boo" / "baz"

    w.close()?;

    let commits = directory_reader::list_commits(Arc::clone(&d))?;
    assert_equals(&1, &commits.len());

    let commit = &commits[0];
    let data = commit.get_user_data()?;
    assert_equals(&1, &data.len());
    assert_equals(&Some("baz".to_string()), &data.get("boo").cloned());
    drop((commits, d));
    dir.close()
}
