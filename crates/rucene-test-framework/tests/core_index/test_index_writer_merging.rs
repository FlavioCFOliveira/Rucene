//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterMerging.java`.
//!
//! * `idField.setStringValue(...)` on a field already added to `document`
//!   mutates the document's own field (`crate::support::field_mut`).
//! * The `Document` `testNoWaitClose` hands to its thread is built again in
//!   that thread (same field, no random draw): Rucene's `Document` holds
//!   `Box<dyn IndexableField>`, which is not `Send`. The thread's `catch
//!   (NullPointerException e)` has no Rust counterpart: Rucene reports a
//!   closed writer as `AlreadyClosedException` only.
//! * `new ConcurrentMergeScheduler() { doMerge }` installs the
//!   `ConcurrentMergeScheduler` `doMerge` override hook.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{
    LogMergePolicy, MergePolicy, MergeTrigger, NoMergePolicy, OneMerge,
};
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, MergeSource, SerialMergeScheduler,
};
use rucene::index::terms::Term;
use rucene::util::SameThreadExecutorService;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{is_instance, java_throwable, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, verbose};
use crate::support::field_mut;

/// `TestIndexWriterMerging`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterMerging",
        vec![
            test("testLucene", test_lucene),
            test("testForceMergeDeletes", test_force_merge_deletes),
            test("testForceMergeDeletes2", test_force_merge_deletes2),
            test("testForceMergeDeletes3", test_force_merge_deletes3),
            test(
                "testForceMergeDeletesWithObserver",
                test_force_merge_deletes_with_observer,
            ),
            test("testMergeObserverNoMerges", test_merge_observer_no_merges),
            test(
                "testMergeObserverAwaitWithTimeout",
                test_merge_observer_await_with_timeout,
            ),
            test(
                "testMergeObserverAwaitTimeout",
                test_merge_observer_await_timeout,
            ),
            test(
                "testForceMergeDeletesBlockingWithObserver",
                test_force_merge_deletes_blocking_with_observer,
            ),
            test(
                "testBlockingModeWithNoMerges",
                test_blocking_mode_with_no_merges,
            ),
            test("testSetMaxMergeDocs", test_set_max_merge_docs),
            test("testNoWaitClose", test_no_wait_close),
            test(
                "testAddEstimatedBytesToMerge",
                test_add_estimated_bytes_to_merge,
            ),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())))
}

/// `new IndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(NoMergePolicy.INSTANCE)`.
fn dont_merge_config() -> Result<IndexWriterConfig> {
    let mut iwc = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    Ok(iwc)
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(NoMergePolicy.INSTANCE)`.
fn no_merge_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    Ok(iwc)
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(newLogMergePolicy())`.
fn log_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    Ok(iwc)
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(newLogMergePolicy(n))`.
fn log_n_iwc(merge_factor: i32) -> Result<IndexWriterConfig> {
    let mut iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(merge_factor)?,
    )))?;
    Ok(iwc)
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMaxBufferedDocs(2)
/// .setRAMBufferSizeMB(IndexWriterConfig.DISABLE_AUTO_FLUSH)`.
fn two_docs_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_ram_buffer_size_mb(
        rucene::index::index_writer_config::LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64,
    )?;
    Ok(iwc)
}

/// `DirectoryReader.open(dir)`'s `maxDoc()` and `numDocs()`.
fn max_and_num_docs(dir: &TestDirectory) -> Result<(i32, i32)> {
    let ir = directory_reader::open(dir.as_directory())?;
    let counts = (ir.max_doc(), ir.num_docs());
    ir.close()?;
    Ok(counts)
}

/// Tests that index merging (specifically addIndexes(Directory...)) doesn't change the index order
/// of documents.
/// Port of `TestIndexWriterMerging.testLucene`.
fn test_lucene() -> Result<()> {
    let num = 100;

    let mut index_a = LuceneTestCase::new_directory();
    let mut index_b = LuceneTestCase::new_directory();

    fill_index(&*random(), &index_a, 0, num)?;
    let fail = verify_index(&index_a, 0)?;
    if fail {
        Assert::fail("Index a is invalid");
    }

    fill_index(&*random(), &index_b, num, num)?;
    let fail = verify_index(&index_b, num)?;
    if fail {
        Assert::fail("Index b is invalid");
    }

    let mut merged = LuceneTestCase::new_directory();

    let writer = IndexWriter::new(merged.as_directory(), log_n_iwc(2)?)?;
    writer.add_indexes_dirs(&[index_a.as_directory(), index_b.as_directory()])?;
    writer.force_merge(1)?;
    writer.close()?;

    let fail = verify_index(&merged, 0)?;

    Assert::assert_false_msg("The merged index is invalid", fail);
    index_a.close()?;
    index_b.close()?;
    merged.close()
}

/// `private boolean verifyIndex(Directory directory, int startAt)`.
fn verify_index(directory: &TestDirectory, start_at: i32) -> Result<bool> {
    let mut fail = false;
    let reader = directory_reader::open(directory.as_directory())?;

    let max = reader.max_doc();
    let stored_fields = reader.stored_fields()?;
    for i in 0..max {
        let temp = stored_fields.document(i)?;
        // System.out.println("doc "+i+"="+temp.getField("count").stringValue());
        // compare the index doc number to the value that it should be
        let count = temp.get_field("count").and_then(|f| f.string_value());
        if count.as_deref() != Some((i + start_at).to_string().as_str()) {
            fail = true;
            println!(
                "Document {} is returning document {}",
                i + start_at,
                count.unwrap_or_else(|| "null".to_string())
            );
        }
    }
    reader.close()?;
    Ok(fail)
}

/// `private void fillIndex(Random random, Directory dir, int start, int numDocs)`.
fn fill_index(random: &dyn Random, dir: &TestDirectory, start: i32, num_docs: i32) -> Result<()> {
    let mut iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(random)));
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(2)?,
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    for i in start..(start + num_docs) {
        let mut temp = Document::new();
        temp.add(Box::new(LuceneTestCase::new_string_field(
            "count",
            &i.to_string(),
            Store::YES,
        )));

        writer.add_document(&temp)?;
    }
    writer.close()
}

/// `customType` (stored only) and `customType1` (untokenized text with term
/// vectors, positions and offsets) of the `testForceMergeDeletes*` tests.
fn force_merge_deletes_types(stored_text: bool) -> Result<(FieldType, FieldType)> {
    let mut custom_type = FieldType::new();
    custom_type.set_stored(true)?;

    let mut custom_type1 = FieldType::new_from(if stored_text {
        TextField::type_stored()
    } else {
        TextField::type_not_stored()
    });
    custom_type1.set_tokenized(false)?;
    custom_type1.set_store_term_vectors(true)?;
    custom_type1.set_store_term_vector_positions(true)?;
    custom_type1.set_store_term_vector_offsets(true)?;
    Ok((custom_type, custom_type1))
}

// LUCENE-325: test forceMergeDeletes, when 2 singular merges
// are required
/// Port of `TestIndexWriterMerging.testForceMergeDeletes`.
fn test_force_merge_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), two_docs_iwc()?)?;
    let mut document = Document::new();

    let (custom_type, custom_type1) = force_merge_deletes_types(true)?;

    let id_field = LuceneTestCase::new_string_field("id", "", Store::NO);
    document.add(Box::new(id_field));
    let stored_field = LuceneTestCase::new_field("stored", "stored", &custom_type);
    document.add(Box::new(stored_field));
    let term_vector_field = LuceneTestCase::new_field("termVector", "termVector", &custom_type1);
    document.add(Box::new(term_vector_field));
    for i in 0..10 {
        field_mut(&mut document, "id").set_string_value(i.to_string())?;
        writer.add_document(&document)?;
    }
    writer.close()?;

    assert_equals(&(10, 10), &max_and_num_docs(&dir)?);

    let writer = IndexWriter::new(dir.as_directory(), dont_merge_config()?)?;
    writer.delete_documents_by_terms(vec![Term::from_text("id", "0")])?;
    writer.delete_documents_by_terms(vec![Term::from_text("id", "7")])?;
    writer.close()?;

    assert_equals(&8, &max_and_num_docs(&dir)?.1);

    let writer = IndexWriter::new(dir.as_directory(), log_iwc()?)?;
    assert_equals(&8, &writer.get_doc_stats()?.num_docs);
    assert_equals(&10, &writer.get_doc_stats()?.max_doc);
    writer.force_merge_deletes()?;
    assert_equals(&8, &writer.get_doc_stats()?.num_docs);
    writer.close()?;
    assert_equals(&(8, 8), &max_and_num_docs(&dir)?);
    dir.close()
}

/// The 98-document index of `testForceMergeDeletes2` and `3`, with every even
/// id deleted.
fn ninety_eight_docs_half_deleted(dir: &TestDirectory) -> Result<()> {
    let mut iwc = two_docs_iwc()?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(50)?,
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    let (custom_type, custom_type1) = force_merge_deletes_types(false)?;
    let mut document = Document::new();

    let stored_field = LuceneTestCase::new_field("stored", "stored", &custom_type);
    document.add(Box::new(stored_field));
    let term_vector_field = LuceneTestCase::new_field("termVector", "termVector", &custom_type1);
    document.add(Box::new(term_vector_field));
    let id_field = LuceneTestCase::new_string_field("id", "", Store::NO);
    document.add(Box::new(id_field));
    for i in 0..98 {
        field_mut(&mut document, "id").set_string_value(i.to_string())?;
        writer.add_document(&document)?;
    }
    writer.close()?;

    assert_equals(&(98, 98), &max_and_num_docs(dir)?);

    let writer = IndexWriter::new(dir.as_directory(), dont_merge_config()?)?;
    let mut i = 0;
    while i < 98 {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
        i += 2;
    }
    writer.close()?;

    assert_equals(&49, &max_and_num_docs(dir)?.1);
    Ok(())
}

// LUCENE-325: test forceMergeDeletes, when many adjacent merges are required
/// Port of `TestIndexWriterMerging.testForceMergeDeletes2`.
fn test_force_merge_deletes2() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    ninety_eight_docs_half_deleted(&dir)?;

    let writer = IndexWriter::new(dir.as_directory(), log_n_iwc(3)?)?;
    assert_equals(&49, &writer.get_doc_stats()?.num_docs);
    writer.force_merge_deletes()?;
    writer.close()?;
    assert_equals(&(49, 49), &max_and_num_docs(&dir)?);
    dir.close()
}

// LUCENE-325: test forceMergeDeletes without waiting, when
// many adjacent merges are required
/// Port of `TestIndexWriterMerging.testForceMergeDeletes3`.
fn test_force_merge_deletes3() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    ninety_eight_docs_half_deleted(&dir)?;

    let writer = IndexWriter::new(dir.as_directory(), log_n_iwc(3)?)?;
    writer.force_merge_deletes_with(false)?;
    writer.close()?;
    assert_equals(&(49, 49), &max_and_num_docs(&dir)?);
    dir.close()
}

/// Just intercepts all merges & verifies that we are never
/// merging a segment with >= 20 (maxMergeDocs) docs
#[derive(Debug, Default)]
struct MyMergeScheduler {
    monitor: Mutex<()>,
    executor: Arc<SameThreadExecutorService>,
}

impl std::fmt::Display for MyMergeScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MyMergeScheduler")
    }
}

impl MergeScheduler for MyMergeScheduler {
    /// `public synchronized void merge(MergeSource mergeSource, MergeTrigger trigger)`.
    fn merge(&self, merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
        let _monitor = self.monitor.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let Some(merge) = merge_source.get_next_merge()? else {
                break;
            };
            let mut num_docs = 0;
            for i in 0..merge.segments.len() {
                let max_doc = merge.segments[i].info.max_doc()?;
                num_docs += max_doc;
                Assert::assert_true(max_doc < 20);
            }
            let handle: OneMerge = merge.clone();
            merge_source.merge(merge)?;
            let merged_max_doc = handle
                .get_merge_info()
                .map(|info| info.info.max_doc())
                .transpose()?;
            assert_equals(&Some(num_docs), &merged_max_doc);
        }
        Ok(())
    }

    fn same_thread_executor(&self) -> &Arc<SameThreadExecutorService> {
        &self.executor
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// The ten-document index of the observer tests, with every even id deleted.
fn ten_docs_half_deleted(dir: &TestDirectory, check_readers: bool) -> Result<()> {
    let indexer = IndexWriter::new(dir.as_directory(), two_docs_iwc()?)?;
    for i in 0..10 {
        let mut doc = Document::new();
        let id_field = LuceneTestCase::new_string_field("id", &i.to_string(), Store::NO);
        doc.add(Box::new(id_field));
        indexer.add_document(&doc)?;
    }
    indexer.close()?;

    if check_readers {
        assert_equals(&(10, 10), &max_and_num_docs(dir)?);
    }

    let deleter = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
    for i in 0..10 {
        if i % 2 == 0 {
            deleter.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
        }
    }
    deleter.close()?;

    if check_readers {
        assert_equals(&(10, 5), &max_and_num_docs(dir)?);
    }
    Ok(())
}

/// Port of `TestIndexWriterMerging.testForceMergeDeletesWithObserver`.
fn test_force_merge_deletes_with_observer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    ten_docs_half_deleted(&dir, true)?;

    let iw = IndexWriter::new(dir.as_directory(), log_iwc()?)?;
    assert_equals(&10, &iw.get_doc_stats()?.max_doc);
    assert_equals(&5, &iw.get_doc_stats()?.num_docs);

    let observer = iw.force_merge_deletes_with(false)?;
    Assert::assert_true_msg("Should have scheduled merges", observer.num_merges() > 0);
    Assert::assert_true_msg(
        "Merges should complete within 30 seconds",
        observer.await_timeout(Duration::from_millis(30_000)),
    );
    assert_equals_msg(
        Some("All merges should be completed after await() returns true"),
        &observer.num_merges(),
        &observer.num_completed_merges(),
    );

    assert_equals(&5, &iw.get_doc_stats()?.max_doc);
    assert_equals(&5, &iw.get_doc_stats()?.num_docs);

    iw.wait_for_merges()?;
    iw.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMerging.testMergeObserverNoMerges`.
fn test_merge_observer_no_merges() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    writer.commit()?;

    let observer = writer.force_merge_deletes_with(false)?;
    assert_equals_msg(Some("Should have zero merges"), &0, &observer.num_merges());

    writer.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMerging.testMergeObserverAwaitWithTimeout`.
fn test_merge_observer_await_with_timeout() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(dir.as_directory(), log_iwc()?)?;
    for i in 0..10 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::NO,
        )));
        iw.add_document(&doc)?;
    }
    iw.commit()?;
    iw.delete_documents_by_terms(vec![Term::from_text("id", "0")])?;
    iw.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
    iw.delete_documents_by_terms(vec![Term::from_text("id", "2")])?;
    iw.commit()?;

    let observer = iw.force_merge_deletes_with(false)?;
    Assert::assert_true_msg(
        "Merges should complete within 30 seconds",
        observer.await_timeout(Duration::from_millis(30_000)),
    );
    assert_equals_msg(
        Some("All merges should be completed after await() returns true"),
        &observer.num_merges(),
        &observer.num_completed_merges(),
    );

    iw.wait_for_merges()?;
    iw.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMerging.testMergeObserverAwaitTimeout`.
fn test_merge_observer_await_timeout() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let merge_started = Arc::new(crate::support::CountDownLatch::new(1));
    let allow_merge_to_finish = Arc::new(crate::support::CountDownLatch::new(1));

    let merge_scheduler = ConcurrentMergeScheduler::new();
    {
        let merge_started = Arc::clone(&merge_started);
        let allow_merge_to_finish = Arc::clone(&allow_merge_to_finish);
        merge_scheduler.set_do_merge_override(Some(Arc::new(
            move |cms: &ConcurrentMergeScheduler,
                  merge_source: &Arc<dyn MergeSource>,
                  merge: OneMerge| {
                merge_started.count_down();
                // Block until test allows completion
                allow_merge_to_finish.await_();
                cms.default_do_merge(merge_source, merge)
            },
        )));
    }

    let mut iwc = log_iwc()?;
    iwc.set_merge_scheduler(Some(Arc::new(merge_scheduler)))?;
    let indexer = IndexWriter::new(dir.as_directory(), iwc)?;

    for i in 0..20 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::NO,
        )));
        indexer.add_document(&doc)?;
    }
    indexer.commit()?;

    for i in 0..10 {
        indexer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }
    indexer.commit()?;

    let observer = indexer.force_merge_deletes_with(false)?;

    if observer.num_merges() > 0 {
        merge_started.await_();
        Assert::assert_false_msg(
            "await should timeout",
            observer.await_timeout(Duration::from_millis(10)),
        );
        allow_merge_to_finish.count_down();
    }

    indexer.wait_for_merges()?;
    indexer.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMerging.testForceMergeDeletesBlockingWithObserver`.
fn test_force_merge_deletes_blocking_with_observer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    ten_docs_half_deleted(&dir, false)?;

    let iw = IndexWriter::new(dir.as_directory(), log_iwc()?)?;
    assert_equals(&10, &iw.get_doc_stats()?.max_doc);
    assert_equals(&5, &iw.get_doc_stats()?.num_docs);

    let observer = iw.force_merge_deletes_with(true)?;
    Assert::assert_true_msg("Should have completed merges", observer.num_merges() > 0);
    Assert::assert_true_msg("await should return true immediately", observer.r#await());

    assert_equals(&5, &iw.get_doc_stats()?.max_doc);
    assert_equals(&5, &iw.get_doc_stats()?.num_docs);

    iw.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMerging.testBlockingModeWithNoMerges`.
fn test_blocking_mode_with_no_merges() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::NO,
    )));
    iw.add_document(&doc)?;
    iw.commit()?;

    let observer = iw.force_merge_deletes_with(true)?;
    assert_equals_msg(Some("Should have zero merges"), &0, &observer.num_merges());
    Assert::assert_true_msg(
        "await with timeout should return true",
        observer.await_timeout(Duration::from_secs(1)),
    );
    Assert::assert_true_msg("await should return true", observer.r#await());

    let future = observer.await_async();
    Assert::assert_true_msg("Future should be done", future.is_done());
    Assert::assert_false_msg(
        "Future should not be exceptional",
        future.is_completed_exceptionally(),
    );

    iw.close()?;
    dir.close()
}

/// `(LogMergePolicy) mp` (a `ClassCastException` when it is not one).
fn as_log_merge_policy(mp: &Arc<dyn MergePolicy>) -> &LogMergePolicy {
    mp.as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a LogMergePolicy").throw()
        })
}

// LUCENE-1013
/// Port of `TestIndexWriterMerging.testSetMaxMergeDocs`.
fn test_set_max_merge_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = mock_iwc();
    conf.set_merge_scheduler(Some(Arc::new(MyMergeScheduler::default())))?;
    conf.set_max_buffered_docs(2)?;
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let mp = conf.merge_policy();
    let lmp = as_log_merge_policy(&mp);
    lmp.set_max_merge_docs(20);
    lmp.set_merge_factor(2)?;

    let iw = IndexWriter::new(dir.as_directory(), conf)?;
    let mut document = Document::new();

    let mut custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;

    document.add(Box::new(LuceneTestCase::new_field(
        "tvtest",
        "a b c",
        &custom_type,
    )));
    for _ in 0..177 {
        iw.add_document(&document)?;
    }
    iw.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMerging.testNoWaitClose`.
fn test_no_wait_close() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();

    let mut doc = Document::new();
    let mut custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_tokenized(false)?;

    let id_field = LuceneTestCase::new_field("id", "", &custom_type);
    doc.add(Box::new(id_field));

    for pass in 0..2 {
        if verbose() {
            println!("TEST: pass={pass}");
        }

        let mut conf = mock_iwc();
        conf.set_open_mode(OpenMode::CREATE)?;
        conf.set_max_buffered_docs(2)?;
        conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        conf.set_commit_on_close(false);
        if pass == 2 {
            conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
        }

        let mut writer = IndexWriter::new(directory.as_directory(), conf)?;
        as_log_merge_policy(&writer.get_config().merge_policy()).set_merge_factor(100)?;

        let mut iter = 0;
        while iter < LuceneTestCase::at_least(3) {
            if verbose() {
                println!("TEST: iter={iter}");
            }
            for j in 0..199 {
                field_mut(&mut doc, "id").set_string_value((iter * 201 + j).to_string())?;
                writer.add_document(&doc)?;
            }

            let mut del_id = iter * 199;
            for _ in 0..20 {
                writer
                    .delete_documents_by_terms(vec![Term::from_text("id", &del_id.to_string())])?;
                del_id += 5;
            }

            writer.commit()?;

            // Force a bunch of merge threads to kick off so we
            // stress out aborting them on close:
            as_log_merge_policy(&writer.get_config().merge_policy()).set_merge_factor(2)?;

            let final_writer = Arc::clone(&writer);
            let failure: Arc<Mutex<Option<Throwable>>> = Arc::new(Mutex::new(None));
            // `doc` holds the one field `id`; the thread builds the same field.
            let (id_name, id_value, id_type) = {
                let f = field_mut(&mut doc, "id");
                (
                    f.name().to_string(),
                    f.string_value().unwrap_or_default(),
                    FieldType::new_from(f.field_type()),
                )
            };
            let t1 = {
                let failure = Arc::clone(&failure);
                Thread::unnamed(move || {
                    let mut doc = Document::new();
                    match rucene::document::Field::new(&id_name, id_value, id_type) {
                        Ok(f) => doc.add(Box::new(f)),
                        Err(e) => rethrow_java(e),
                    }
                    let done = AtomicBool::new(false);
                    while !done.load(Ordering::Relaxed) {
                        for _ in 0..100 {
                            match final_writer.add_document(&doc) {
                                Ok(_) => {}
                                Err(e)
                                    if is_instance(
                                        &e,
                                        "org.apache.lucene.store.AlreadyClosedException",
                                    ) =>
                                {
                                    done.store(true, Ordering::Relaxed);
                                    break;
                                }
                                Err(e) => {
                                    let t = java_throwable(&e);
                                    println!("{t}");
                                    *failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(t);
                                    done.store(true, Ordering::Relaxed);
                                    break;
                                }
                            }
                        }
                        std::thread::yield_now();
                    }
                })
            };

            t1.start();

            writer.close()?;
            let _ = t1.join();

            if let Some(failure) = failure.lock().unwrap_or_else(|p| p.into_inner()).take() {
                failure.throw();
            }

            // Make sure reader can read
            let reader = directory_reader::open(directory.as_directory())?;
            reader.close()?;

            // Reopen
            let mut conf = mock_iwc();
            conf.set_open_mode(OpenMode::APPEND)?;
            conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
            conf.set_commit_on_close(false);
            writer = IndexWriter::new(directory.as_directory(), conf)?;
            iter += 1;
        }
        writer.close()?;
    }

    directory.close()
}

/// Port of `TestIndexWriterMerging.testAddEstimatedBytesToMerge`.
fn test_add_estimated_bytes_to_merge() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            "content",
            Store::YES,
        )));
        for _ in 0..10 {
            writer.add_document(&doc)?;
        }
        writer.flush()?;

        // Create a merge with the segments
        let segment_infos = writer.clone_segment_infos()?;
        let mut merge = OneMerge::new(segment_infos.as_list().to_vec())?;
        writer.add_estimated_bytes_to_merge(&mut merge)?;

        Assert::assert_true(merge.estimated_merge_bytes > 0);
        Assert::assert_true(merge.total_merge_bytes > 0);
        Assert::assert_true(merge.estimated_merge_bytes <= merge.total_merge_bytes);
        writer.close()?;
    }
    dir.close()
}
