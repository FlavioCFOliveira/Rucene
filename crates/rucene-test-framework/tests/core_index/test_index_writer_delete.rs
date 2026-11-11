//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterDelete.java`.
//!
//! * `TestIndexWriter.assertNoUnreferencedFiles` is `support.rs`'s port.
//! * The `IndexWriter` subclass of `testApplyDeletesOnFlush` is an
//!   `IndexWriterOverrides` installed right after construction.
//! * `new Throwable().printStackTrace(System.out)` prints the Rust backtrace.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Assert, Ignore};
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::document::{
    Document, FieldType, NumericDocValuesField, Store, StoredField, StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::check_index::CheckIndex;
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, IndexWriterOverrides};
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{LogDocSize, LogMergePolicy, NoMergePolicy};
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, SerialMergeScheduler};
use rucene::index::multi_bits::MultiBits;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::TermQuery;
use rucene::store::{Directory, FSDirectory};
use rucene::util::IOUtils;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{tokenizer_components, MockAnalyzer};
use rucene_test_framework::index::{MockRandomMergePolicy, RandomIndexWriter};
use rucene_test_framework::store::base_directory_test_case::close_shared;
use rucene_test_framework::store::{Failure, MockDirectoryWrapper};
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{
    Monster, Nightly, SuppressCodecs, TestDirectory,
};
use rucene_test_framework::util::{LuceneTestCase, PrintStreamLines, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_null, class_annotated, random, shuffle, test,
    verbose,
};
use crate::support::{as_cms, assert_no_unreferenced_files, npe, CountDownLatch};

/// `TestIndexWriterDelete`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterDelete",
        vec![
            test("testSimpleCase", test_simple_case),
            test("testNonRAMDelete", test_non_ram_delete),
            test("testRAMDeletes", test_ram_deletes),
            test("testBothDeletes", test_both_deletes),
            test("testBatchDeletes", test_batch_deletes),
            test("testDeleteAllSimple", test_delete_all_simple),
            test("testDeleteAllNoDeadLock", test_delete_all_no_dead_lock),
            test("testDeleteAllRollback", test_delete_all_rollback),
            test("testDeleteAllNRT", test_delete_all_nrt),
            test("testDeleteAllRepeated", test_delete_all_repeated).annotate(Monster {
                value: "Takes 1-2 minutes but writes tons of files to disk.".to_string(),
            }),
            test("testDeletesOnDiskFull", test_deletes_on_disk_full).annotate(Nightly),
            test("testUpdatesOnDiskFull", test_updates_on_disk_full).annotate(Nightly),
            test("testErrorAfterApplyDeletes", test_error_after_apply_deletes).annotate(Ignore {
                value: String::new(),
            }),
            test("testErrorInDocsWriterAdd", test_error_in_docs_writer_add),
            test("testDeleteNullQuery", test_delete_null_query),
            test("testDeleteAllSlowly", test_delete_all_slowly),
            test("testIndexingThenDeleting", test_indexing_then_deleting).annotate(Nightly),
            test(
                "testFlushPushedDeletesByRAM",
                test_flush_pushed_deletes_by_ram,
            ),
            test("testApplyDeletesOnFlush", test_apply_deletes_on_flush).annotate(Nightly),
            test(
                "testDeletesCheckIndexOutput",
                test_deletes_check_index_output,
            ),
            test("testTryDeleteDocument", test_try_delete_document),
            test(
                "testNRTIsCurrentAfterDelete",
                test_nrt_is_current_after_delete,
            ),
            test(
                "testOnlyDeletesTriggersMergeOnClose",
                test_only_deletes_triggers_merge_on_close,
            ),
            test(
                "testOnlyDeletesTriggersMergeOnGetReader",
                test_only_deletes_triggers_merge_on_get_reader,
            ),
            test(
                "testOnlyDeletesTriggersMergeOnFlush",
                test_only_deletes_triggers_merge_on_flush,
            ),
            test(
                "testOnlyDeletesDeleteAllDocs",
                test_only_deletes_delete_all_docs,
            ),
            test("testMergingAfterDeleteAll", test_merging_after_delete_all),
        ],
        |b| {
            // too slow here
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// `new MockAnalyzer(random(), MockTokenizer.WHITESPACE, false)`.
fn whitespace_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ))
}

/// `newIndexWriterConfig(new MockAnalyzer(random(), MockTokenizer.WHITESPACE, false))`.
fn whitespace_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(whitespace_analyzer()))
}

/// [`whitespace_iwc`] with `setMaxBufferedDocs(max)`.
fn whitespace_iwc_max_buffered(max: i32) -> Result<IndexWriterConfig> {
    let iwc = whitespace_iwc();
    iwc.set_max_buffered_docs(max)?;
    Ok(iwc)
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn plain_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

/// The document `addDoc` and `updateDoc` index.
fn id_value_doc(id: i32, value: i32) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        &id.to_string(),
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "value",
        &value.to_string(),
        Store::NO,
    )));
    doc.add(Box::new(NumericDocValuesField::new("dv", value as i64)));
    doc
}

/// `updateDoc(modifier, id, value)`.
fn update_doc(modifier: &IndexWriter, id: i32, value: i32) -> Result<()> {
    let doc = id_value_doc(id, value);
    modifier.update_document(term("id", &id.to_string()), &doc)?;
    Ok(())
}

/// `addDoc(modifier, id, value)`.
fn add_doc(modifier: &IndexWriter, id: i32, value: i32) -> Result<()> {
    let doc = id_value_doc(id, value);
    modifier.add_document(&doc)?;
    Ok(())
}

/// `getHitCount(dir, term)`.
fn get_hit_count(dir: &Arc<dyn Directory>, term: &Term) -> Result<i64> {
    let reader = directory_reader::open(Arc::clone(dir))?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let hit_count = searcher
        .search(Arc::new(TermQuery::new(term.clone())), 1000)?
        .total_hits
        .value();
    reader.close()?;
    Ok(hit_count)
}

/// The two documents `testSimpleCase`, `testErrorAfterApplyDeletes` and
/// `testErrorInDocsWriterAdd` index, over `custom1` (stored only).
fn city_doc(custom1: &FieldType, i: usize) -> Document {
    let keywords = ["1", "2"];
    let unindexed = ["Netherlands", "Italy"];
    let unstored = ["Amsterdam has lots of bridges", "Venice has lots of canals"];
    let text = ["Amsterdam", "Venice"];
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        keywords[i],
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "country",
        unindexed[i],
        custom1,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "contents",
        unstored[i],
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "city",
        text[i],
        Store::YES,
    )));
    doc
}

/// `FieldType custom1 = new FieldType(); custom1.setStored(true);`.
fn stored_type() -> Result<FieldType> {
    let custom1 = FieldType::new();
    custom1.set_stored(true)?;
    Ok(custom1)
}

/// Port of `TestIndexWriterDelete.testSimpleCase`.
fn test_simple_case() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc())?;

    let custom1 = stored_type()?;
    for i in 0..2 {
        modifier.add_document(&city_doc(&custom1, i))?;
    }
    modifier.force_merge(1)?;
    modifier.commit()?;

    let term = term("city", "Amsterdam");
    let hit_count = get_hit_count(&d, &term)?;
    assert_equals(&1, &hit_count);
    if verbose() {
        println!("\nTEST: now delete by term={term}");
    }
    modifier.delete_documents_by_terms(vec![term.clone()])?;
    modifier.commit()?;

    if verbose() {
        println!("\nTEST: now getHitCount");
    }
    let hit_count = get_hit_count(&d, &term)?;
    assert_equals(&0, &hit_count);

    modifier.close()?;
    drop(d);
    dir.close()
}

/// test when delete terms only apply to disk segments
/// Port of `TestIndexWriterDelete.testNonRAMDelete`.
fn test_non_ram_delete() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc_max_buffered(2)?)?;
    let mut id = 0;
    let value = 100;

    for _ in 0..7 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }
    modifier.commit()?;

    assert_equals(&0, &modifier.get_num_buffered_documents());
    Assert::assert_true(0 < modifier.get_segment_count()?);

    modifier.commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    modifier.delete_documents_by_terms(vec![term("value", &value.to_string())])?;

    modifier.commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());
    reader.close()?;
    modifier.close()?;
    drop(d);
    dir.close()
}

/// test when delete terms only apply to ram segments
/// Port of `TestIndexWriterDelete.testRAMDeletes`.
fn test_ram_deletes() -> Result<()> {
    for t in 0..2 {
        if verbose() {
            println!("TEST: t={t}");
        }
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc_max_buffered(4)?)?;
        let mut id = 0;
        let value = 100;

        id += 1;
        add_doc(&modifier, id, value)?;
        if 0 == t {
            modifier.delete_documents_by_terms(vec![term("value", &value.to_string())])?;
        } else {
            modifier.delete_documents_by_queries(vec![Arc::new(TermQuery::new(term(
                "value",
                &value.to_string(),
            )))])?;
        }
        id += 1;
        add_doc(&modifier, id, value)?;
        if 0 == t {
            modifier.delete_documents_by_terms(vec![term("value", &value.to_string())])?;
            assert_equals(&1, &modifier.get_buffered_delete_terms_size());
        } else {
            modifier.delete_documents_by_queries(vec![Arc::new(TermQuery::new(term(
                "value",
                &value.to_string(),
            )))])?;
        }

        id += 1;
        add_doc(&modifier, id, value)?;
        assert_equals(&0, &modifier.get_segment_count()?);
        modifier.commit()?;

        let reader = directory_reader::open(Arc::clone(&d))?;
        assert_equals(&1, &reader.num_docs());

        let hit_count = get_hit_count(&d, &term("id", &id.to_string()))?;
        assert_equals(&1, &hit_count);
        reader.close()?;
        modifier.close()?;
        drop(d);
        dir.close()?;
    }
    Ok(())
}

/// test when delete terms apply to both disk and ram segments
/// Port of `TestIndexWriterDelete.testBothDeletes`.
fn test_both_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc_max_buffered(100)?)?;

    let mut id = 0;
    let mut value = 100;

    for _ in 0..5 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }

    value = 200;
    for _ in 0..5 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }
    modifier.commit()?;

    for _ in 0..5 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }
    modifier.delete_documents_by_terms(vec![term("value", &value.to_string())])?;

    modifier.commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&5, &reader.num_docs());
    modifier.close()?;
    reader.close()?;
    drop(d);
    dir.close()
}

/// test that batched delete terms are flushed together
/// Port of `TestIndexWriterDelete.testBatchDeletes`.
fn test_batch_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc_max_buffered(2)?)?;

    let mut id = 0;
    let value = 100;

    for _ in 0..7 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }
    modifier.commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    id = 0;
    id += 1;
    modifier.delete_documents_by_terms(vec![term("id", &id.to_string())])?;
    id += 1;
    modifier.delete_documents_by_terms(vec![term("id", &id.to_string())])?;

    modifier.commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&5, &reader.num_docs());
    reader.close()?;

    let mut terms = Vec::with_capacity(3);
    for _ in 0..3 {
        id += 1;
        terms.push(term("id", &id.to_string()));
    }
    modifier.delete_documents_by_terms(terms)?;
    modifier.commit()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&2, &reader.num_docs());
    reader.close()?;

    modifier.close()?;
    drop(d);
    dir.close()
}

/// test deleteAll()
/// Port of `TestIndexWriterDelete.testDeleteAllSimple`.
fn test_delete_all_simple() -> Result<()> {
    if verbose() {
        println!("TEST: now start");
    }
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc_max_buffered(2)?)?;

    let mut id = 0;
    let value = 100;

    for _ in 0..7 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }
    if verbose() {
        println!("TEST: now commit");
    }
    modifier.commit()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    // Add 1 doc (so we will have something buffered)
    add_doc(&modifier, 99, value)?;

    // Delete all
    if verbose() {
        println!("TEST: now delete all");
    }
    modifier.delete_all()?;

    // Delete all shouldn't be on disk yet
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    // Add a doc and update a doc (after the deleteAll, before the commit)
    add_doc(&modifier, 101, value)?;
    update_doc(&modifier, 102, value)?;
    if verbose() {
        println!("TEST: now 2nd commit");
    }

    // commit the delete all
    modifier.commit()?;

    // Validate there are no docs left
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&2, &reader.num_docs());
    reader.close()?;

    modifier.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterDelete.testDeleteAllNoDeadLock`.
fn test_delete_all_no_dead_lock() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(Arc::new(MockRandomMergePolicy::new(&*random()))))?;
    let modifier = Arc::new(RandomIndexWriter::new_with_config(
        &*random(),
        Arc::clone(&d),
        iwc,
    )?);
    let num_threads = LuceneTestCase::at_least(2);
    let mut threads = Vec::with_capacity(num_threads as usize);
    let latch = Arc::new(CountDownLatch::new(1));
    let done_latch = Arc::new(CountDownLatch::new(num_threads as i64));
    for i in 0..num_threads {
        let offset = i;
        let modifier = Arc::clone(&modifier);
        let latch = Arc::clone(&latch);
        let done_latch = Arc::clone(&done_latch);
        let thread = Thread::unnamed(move || {
            let mut id = offset * 1000;
            let value = 100;
            let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
                latch.await_();
                for _ in 0..1000 {
                    let mut doc = Document::new();
                    doc.add(Box::new(LuceneTestCase::new_text_field(
                        "content",
                        "aaa",
                        Store::NO,
                    )));
                    doc.add(Box::new(LuceneTestCase::new_string_field(
                        "id",
                        &id.to_string(),
                        Store::YES,
                    )));
                    id += 1;
                    doc.add(Box::new(LuceneTestCase::new_string_field(
                        "value",
                        &value.to_string(),
                        Store::NO,
                    )));
                    doc.add(Box::new(NumericDocValuesField::new("dv", value as i64)));
                    modifier.add_document(&doc)?;
                    if verbose() {
                        println!("\tThread[{offset}]: add doc: {id}");
                    }
                }
                Ok(())
            }));
            // `finally`
            done_latch.count_down();
            if verbose() {
                println!("\tThread[{offset}]: done indexing");
            }
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw()
                }
                Err(payload) => resume_unwind(payload),
            }
        });
        thread.start();
        threads.push(thread);
    }
    latch.count_down();
    while !done_latch.await_millis(1) {
        if verbose() {
            println!("\nTEST: now deleteAll");
        }
        modifier.delete_all()?;
        if verbose() {
            println!("del all");
        }
    }

    if verbose() {
        println!("\nTEST: now final deleteAll");
    }

    modifier.delete_all()?;
    for thread in &threads {
        if let Err(payload) = thread.join() {
            resume_unwind(payload);
        }
    }

    if verbose() {
        println!("\nTEST: now close");
    }
    modifier.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    if verbose() {
        println!("\nTEST: got reader={reader}");
    }
    assert_equals(&0, &reader.max_doc());
    assert_equals(&0, &reader.num_docs());
    assert_equals_delta(0.0, reader.num_deleted_docs() as f64, 0.0);
    reader.close()?;

    drop((modifier, d));
    dir.close()
}

/// test rollback of deleteAll()
/// Port of `TestIndexWriterDelete.testDeleteAllRollback`.
fn test_delete_all_rollback() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc_max_buffered(2)?)?;

    let mut id = 0;
    let value = 100;

    for _ in 0..7 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }
    modifier.commit()?;

    id += 1;
    add_doc(&modifier, id, value)?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    // Delete all
    modifier.delete_all()?;

    // Roll it back
    modifier.rollback()?;

    // Validate that the docs are still there
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    drop(d);
    dir.close()
}

/// test deleteAll() w/ near real-time reader
/// Port of `TestIndexWriterDelete.testDeleteAllNRT`.
fn test_delete_all_nrt() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc_max_buffered(2)?)?;

    let mut id = 0;
    let value = 100;

    for _ in 0..7 {
        id += 1;
        add_doc(&modifier, id, value)?;
    }
    modifier.commit()?;

    let reader = directory_reader::open_with_writer(&*modifier)?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    id += 1;
    add_doc(&modifier, id, value)?;
    id += 1;
    add_doc(&modifier, id, value)?;

    // Delete all
    modifier.delete_all()?;

    let reader = directory_reader::open_with_writer(&*modifier)?;
    assert_equals(&0, &reader.num_docs());
    reader.close()?;

    // Roll it back
    modifier.rollback()?;

    // Validate that the docs are still there
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&7, &reader.num_docs());
    reader.close()?;

    drop(d);
    dir.close()
}

/// Verify that we can call deleteAll repeatedly without leaking field numbers such that we trigger
/// OOME
/// on creation of FieldInfos. See https://issues.apache.org/jira/browse/LUCENE-9617
/// Port of `TestIndexWriterDelete.testDeleteAllRepeated`.
fn test_delete_all_repeated() -> Result<()> {
    let breaking_field_count: i64 = 50_000_000;
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&LuceneTestCase::create_temp_dir())?);
    // Avoid flushing until the end of the test to save time.
    let conf = LuceneTestCase::new_index_writer_config();
    conf.set_max_buffered_docs(1000)?;
    conf.set_ram_buffer_size_mb(1000.0)?;
    conf.set_ram_per_thread_hard_limit_mb(1000)?;
    conf.set_check_pending_flush_update(false)?;
    let modifier = IndexWriter::new(Arc::clone(&dir), conf)?;
    let mut document = Document::new();
    let fields_per_doc: i64 = 1_000;
    for i in 0..fields_per_doc {
        document.add(Box::new(StoredField::new_string(
            &format!("field{i}"),
            String::new(),
        )?));
    }
    let document = Arc::new(document);
    let num_fields = Arc::new(AtomicI64::new(0));
    let mut threads = Vec::new();
    let n_threads = LuceneTestCase::at_least(8);
    for _ in 0..n_threads {
        let modifier = Arc::clone(&modifier);
        let num_fields = Arc::clone(&num_fields);
        let document = Arc::clone(&document);
        let t = Thread::unnamed(move || {
            let outcome = (|| -> Result<()> {
                while num_fields.fetch_add(fields_per_doc, Ordering::SeqCst) < breaking_field_count
                {
                    modifier.add_document(&*document)?;
                    modifier.delete_all()?;
                }
                Ok(())
            })();
            if let Err(e) = outcome {
                if e.is_io_exception() {
                    Throwable::from_cause("java.io.UncheckedIOException", java_throwable(&e))
                        .throw()
                }
                java_throwable(&e).throw()
            }
        });
        t.start();
        threads.push(t);
    }
    for t in &threads {
        if let Err(payload) = t.join() {
            resume_unwind(payload);
        }
    }
    // Add one last document and flush to build FieldInfos.
    modifier.add_document(&*document)?;
    modifier.flush()?;
    modifier.close()?;
    drop((threads, modifier));
    close_shared(dir)
}

/// Port of `TestIndexWriterDelete.testDeletesOnDiskFull`.
fn test_deletes_on_disk_full() -> Result<()> {
    do_test_operations_on_disk_full(false)
}

/// TODO: can we fix MockDirectoryWrapper disk full checking to be more efficient (not recompute on
/// every write)?
/// Port of `TestIndexWriterDelete.testUpdatesOnDiskFull`.
fn test_updates_on_disk_full() -> Result<()> {
    do_test_operations_on_disk_full(true)
}

/// `(IOException) ise.getCause()`.
fn io_cause(ise: &LuceneError) -> LuceneError {
    match ise {
        LuceneError::IllegalState {
            cause: Some(cause), ..
        } => {
            if cause.is_io_exception() {
                cause.duplicate()
            } else {
                Throwable::new(
                    "java.lang.ClassCastException",
                    "the cause is not an IOException",
                )
                .throw()
            }
        }
        _ => npe("ise.getCause()"),
    }
}

/// Make sure if modifier tries to commit but hits disk full that modifier remains consistent and
/// usable. Similar to TestIndexReader.testDiskFull().
/// Port of `TestIndexWriterDelete.doTestOperationsOnDiskFull`.
fn do_test_operations_on_disk_full(updates: bool) -> Result<()> {
    let search_term = term("content", "aaa");
    let start_count = 157;
    let end_count = 144;

    // First build up a starting index:
    let mut start_dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());

    let writer = IndexWriter::new(start_dir.as_directory(), whitespace_iwc())?;
    for i in 0..157 {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        d.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("aaa {i}"),
            Store::NO,
        )));
        d.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
        writer.add_document(&d)?;
    }
    writer.close()?;
    drop(writer);

    let disk_usage = start_dir
        .as_mock()
        .expect("newMockDirectory")
        .size_in_bytes()?;
    let mut disk_free = disk_usage + 10;

    let mut err: Option<LuceneError> = None;

    let mut done = false;

    // Iterate w/ ever-increasing free disk space:
    while !done {
        if verbose() {
            println!("TEST: cycle");
        }
        let copy = TestUtil::ram_copy_of(&*start_dir.as_directory())?;
        let mut dir_holder =
            TestDirectory::Mock(MockDirectoryWrapper::new(&*random(), Arc::from(copy)));
        let dir = dir_holder.as_mock().expect("MockDirectoryWrapper");
        dir.set_allow_random_file_not_found_exception(false);
        let iwc = whitespace_iwc();
        iwc.set_max_buffered_docs(1000)?;
        iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
        let modifier = IndexWriter::new(dir.as_directory(), iwc)?;
        let ms = modifier.get_config().merge_scheduler();
        as_cms(&ms).set_suppress_exceptions();

        // For each disk size, first try to commit against
        // dir that will hit random IOExceptions & disk
        // full; after, give it infinite disk space & turn
        // off random IOExceptions & retry w/ same reader:
        let mut success = false;

        for x in 0..2 {
            if verbose() {
                println!("TEST: x={x}");
            }

            let mut rate = 0.1;
            let disk_ratio = disk_free as f64 / disk_usage as f64;
            let this_disk_free;
            let test_name;

            if 0 == x {
                this_disk_free = disk_free;
                if disk_ratio >= 2.0 {
                    rate /= 2.0;
                }
                if disk_ratio >= 4.0 {
                    rate /= 2.0;
                }
                if disk_ratio >= 6.0 {
                    rate = 0.0;
                }
                if verbose() {
                    println!("\ncycle: {disk_free} bytes");
                }
                test_name = format!("disk full during reader.close() @ {this_disk_free} bytes");
                dir.set_random_io_exception_rate_on_open(random().next_double() * 0.01);
            } else {
                this_disk_free = 0;
                rate = 0.0;
                if verbose() {
                    println!("\ncycle: same writer: unlimited disk space");
                }
                test_name = "reader re-use after disk full".to_string();
                dir.set_random_io_exception_rate_on_open(0.0);
            }

            dir.set_max_size_in_bytes(this_disk_free);
            dir.set_random_io_exception_rate(rate);

            let attempt = (|| -> Result<()> {
                if 0 == x {
                    let mut doc_id = 12;
                    for i in 0..13 {
                        if updates {
                            let mut d = Document::new();
                            d.add(Box::new(LuceneTestCase::new_string_field(
                                "id",
                                &i.to_string(),
                                Store::YES,
                            )));
                            d.add(Box::new(LuceneTestCase::new_text_field(
                                "content",
                                &format!("bbb {i}"),
                                Store::NO,
                            )));
                            d.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
                            modifier.update_document(term("id", &doc_id.to_string()), &d)?;
                        } else {
                            // deletes
                            modifier
                                .delete_documents_by_terms(vec![term("id", &doc_id.to_string())])?;
                            // modifier.setNorm(docId, "contents", (float)2.0);
                        }
                        doc_id += 12;
                    }
                    match modifier.close() {
                        Ok(()) => {}
                        Err(ise) if is_instance(&ise, "java.lang.IllegalStateException") => {
                            // ok
                            return Err(io_cause(&ise));
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(())
            })();
            match attempt {
                Ok(()) => {
                    success = true;
                    if 0 == x {
                        done = true;
                    }
                }
                Err(e) if e.is_io_exception() => {
                    if verbose() {
                        println!("  hit IOException: {e}");
                        println!("{}", std::backtrace::Backtrace::force_capture());
                    }
                    if 1 == x {
                        eprintln!("{e}");
                        Assert::fail(&format!(
                            "{test_name} hit IOException after disk space was freed up"
                        ));
                    }
                    err = Some(e);
                }
                Err(e) => return Err(e),
            }
            // prevent throwing a random exception here!!
            let random_io_exception_rate = dir.get_random_io_exception_rate();
            let max_size_in_bytes = dir.get_max_size_in_bytes();
            dir.set_random_io_exception_rate(0.0);
            dir.set_random_io_exception_rate_on_open(0.0);
            dir.set_max_size_in_bytes(0);
            if !success {
                // Must force the close else the writer can have
                // open files which cause exc in MockRAMDir.close
                if verbose() {
                    println!("TEST: now rollback");
                }
                modifier.rollback()?;
            }

            // If the close() succeeded, make sure index is OK:
            if success {
                TestUtil::check_index(dir.as_directory())?;
            }
            dir.set_random_io_exception_rate(random_io_exception_rate);
            dir.set_max_size_in_bytes(max_size_in_bytes);

            // Finally, verify index is not corrupt, and, if
            // we succeeded, we see all docs changed, and if
            // we failed, we see either all docs or no docs
            // changed (transactional semantics):
            let new_reader = match directory_reader::open(dir.as_directory()) {
                Ok(r) => r,
                Err(e) if e.is_io_exception() => {
                    eprintln!("{e}");
                    Assert::fail(&format!(
                        "{test_name}:exception when creating IndexReader after disk full during close: {e}"
                    ))
                }
                Err(e) => return Err(e),
            };

            let searcher = LuceneTestCase::new_searcher(Arc::clone(&new_reader));
            let hits = match searcher.search(Arc::new(TermQuery::new(search_term.clone())), 1000) {
                Ok(top) => top.score_docs,
                Err(e) if e.is_io_exception() => {
                    eprintln!("{e}");
                    Assert::fail(&format!("{test_name}: exception when searching: {e}"))
                }
                Err(e) => return Err(e),
            };
            let result2 = hits.len();
            if success {
                if x == 0 && result2 != end_count {
                    Assert::fail(&format!(
                        "{test_name}: method did not throw exception but hits.length for search on term 'aaa' is {result2} instead of expected {end_count}"
                    ));
                } else if x == 1 && result2 != start_count && result2 != end_count {
                    // It's possible that the first exception was
                    // "recoverable" wrt pending deletes, in which
                    // case the pending deletes are retained and
                    // then re-flushing (with plenty of disk
                    // space) will succeed in flushing the
                    // deletes:
                    Assert::fail(&format!(
                        "{test_name}: method did not throw exception but hits.length for search on term 'aaa' is {result2} instead of expected {start_count} or {end_count}"
                    ));
                }
            } else {
                // On hitting exception we still may have added
                // all docs:
                if result2 != start_count && result2 != end_count {
                    match &err {
                        Some(e) => eprintln!("{e}"),
                        None => npe::<()>("err"),
                    }
                    Assert::fail(&format!(
                        "{test_name}: method did throw exception but hits.length for search on term 'aaa' is {result2} instead of expected {start_count} or {end_count}"
                    ));
                }
            }
            new_reader.close()?;
            if result2 == end_count {
                break;
            }
        }
        drop(modifier);
        dir_holder.close()?;

        // Try again with more bytes of free space:
        disk_free += std::cmp::max(10, disk_free >> 3);
    }
    start_dir.close()
}

/// The state of `testErrorAfterApplyDeletes`'s anonymous `Failure`.
#[derive(Default)]
struct ApplyDeletesFailureState {
    saw_maybe: bool,
    failed: bool,
    thread: Option<ThreadId>,
}

/// The anonymous `MockDirectoryWrapper.Failure` of `testErrorAfterApplyDeletes`.
#[derive(Default)]
struct ApplyDeletesFailure {
    state: Mutex<ApplyDeletesFailureState>,
    do_fail: AtomicBool,
}

impl Failure for ApplyDeletesFailure {
    fn reset(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.thread = Some(std::thread::current().id());
        state.saw_maybe = false;
        state.failed = false;
    }

    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if Some(std::thread::current().id()) != state.thread {
            // don't fail during merging
            return Ok(());
        }
        if verbose() {
            println!("FAIL EVAL:");
        }
        println!("{}", std::backtrace::Backtrace::force_capture());
        if state.saw_maybe && !state.failed {
            let seen = LuceneTestCase::call_stack_contains_any_of(&[
                "apply_deletes_and_updates",
                "slow_file_exists",
            ]);
            if !seen {
                // Only fail once we are no longer in applyDeletes
                state.failed = true;
                if verbose() {
                    println!("TEST: mock failure: now fail");
                    println!("{}", std::backtrace::Backtrace::force_capture());
                }
                return Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    "fail after applyDeletes",
                ));
            }
        }
        if !state.failed
            && LuceneTestCase::call_stack_contains_any_of(&["apply_deletes_and_updates"])
        {
            if verbose() {
                println!("TEST: mock failure: saw applyDeletes");
                println!("{}", std::backtrace::Backtrace::force_capture());
            }
            state.saw_maybe = true;
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// This test tests that buffered deletes are cleared when
/// an Exception is hit during flush.
/// Port of `TestIndexWriterDelete.testErrorAfterApplyDeletes`.
fn test_error_after_apply_deletes() -> Result<()> {
    let failure = Arc::new(ApplyDeletesFailure::default());

    // create a couple of files

    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let d = dir.as_directory();
    let iwc = whitespace_iwc();
    iwc.set_reader_pooling(false);
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let modifier = IndexWriter::new(Arc::clone(&d), iwc)?;

    let lmp = modifier.get_config().merge_policy();
    lmp.set_no_cfs_ratio(1.0)?;

    failure.reset();
    dir.as_mock()
        .expect("newMockDirectory")
        .fail_on(Arc::clone(&failure) as _);

    let custom1 = stored_type()?;
    for i in 0..2 {
        modifier.add_document(&city_doc(&custom1, i))?;
    }
    // flush

    if verbose() {
        println!("TEST: now full merge");
    }

    modifier.force_merge(1)?;
    if verbose() {
        println!("TEST: now commit");
    }
    modifier.commit()?;

    // one of the two files hits

    let term = term("city", "Amsterdam");
    let hit_count = get_hit_count(&d, &term)?;
    assert_equals(&1, &hit_count);

    // open the writer again (closed above)

    // delete the doc
    // max buf del terms is two, so this is buffered

    if verbose() {
        println!("TEST: delete term={term}");
    }

    modifier.delete_documents_by_terms(vec![term.clone()])?;

    // add a doc,
    // doc remains buffered

    if verbose() {
        println!("TEST: add empty doc");
    }
    let doc = Document::new();
    modifier.add_document(&doc)?;

    // commit the changes, the buffered deletes, and the new doc

    // The failure object will fail on the first write after the del
    // file gets created when processing the buffered delete

    // in the ac case, this will be when writing the new segments
    // files so we really don't need the new doc, but it's harmless

    // a new segments file won't be created but in this
    // case, creation of the cfs file happens next so we
    // need the doc (to test that it's okay that we don't
    // lose deletes if failing while creating the cfs file)

    if verbose() {
        println!("TEST: now commit for failure");
    }
    let expected = expect_throws("java.lang.RuntimeException", || modifier.commit());
    if verbose() {
        println!("TEST: hit exc:");
        println!("{expected}");
    }

    // The commit above failed, so we need to retry it (which will
    // succeed, because the failure is a one-shot)

    let writer_closed = match modifier.commit() {
        Ok(_) => false,
        // The above exc struck during merge, and closed the writer
        Err(ise) if is_instance(&ise, "java.lang.IllegalStateException") => true,
        Err(e) => return Err(e),
    };

    if !writer_closed {
        let hit_count = get_hit_count(&d, &term)?;

        // Make sure the delete was successfully flushed:
        assert_equals(&0, &hit_count);

        modifier.close()?;
    }
    drop((modifier, d));
    dir.close()
}

/// The anonymous `MockDirectoryWrapper.Failure` of `testErrorInDocsWriterAdd`.
#[derive(Default)]
struct DocsWriterAddFailure {
    failed: AtomicBool,
    do_fail: AtomicBool,
}

impl Failure for DocsWriterAddFailure {
    fn reset(&self) {
        self.failed.store(false, Ordering::SeqCst);
    }

    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if !self.failed.load(Ordering::SeqCst) {
            self.failed.store(true, Ordering::SeqCst);
            return Err(LuceneError::Io(std::io::Error::other("fail in add doc")));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// This test tests that the files created by the docs writer before
/// a segment is written are cleaned up if there's an i/o error
/// Port of `TestIndexWriterDelete.testErrorInDocsWriterAdd`.
fn test_error_in_docs_writer_add() -> Result<()> {
    let failure = Arc::new(DocsWriterAddFailure::default());

    // create a couple of files

    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let d = dir.as_directory();
    let modifier = IndexWriter::new(Arc::clone(&d), whitespace_iwc())?;
    modifier.commit()?;
    failure.reset();
    dir.as_mock()
        .expect("newMockDirectory")
        .fail_on(Arc::clone(&failure) as _);

    let custom1 = stored_type()?;
    for i in 0..2 {
        let doc = city_doc(&custom1, i);
        match modifier.add_document(&doc) {
            Ok(_) => {}
            Err(io) if io.is_io_exception() => {
                if verbose() {
                    println!("TEST: got expected exc:");
                    println!("{io}");
                }
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Assert::assert_true(modifier.is_deleter_closed()?);

    assert_no_unreferenced_files(&d, "docsWriter.abort() failed to delete unreferenced files")?;
    drop((modifier, d));
    dir.close()
}

/// Port of `TestIndexWriterDelete.testDeleteNullQuery`.
fn test_delete_null_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let modifier = IndexWriter::new(
        Arc::clone(&d),
        IndexWriterConfig::with_analyzer(Some(whitespace_analyzer())),
    )?;

    for i in 0..5 {
        add_doc(&modifier, i, 2 * i)?;
    }

    modifier.delete_documents_by_queries(vec![Arc::new(TermQuery::new(term("nada", "nada")))])?;
    modifier.commit()?;
    assert_equals(&5, &modifier.get_doc_stats()?.num_docs);
    modifier.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterDelete.testDeleteAllSlowly`.
fn test_delete_all_slowly() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), Arc::clone(&d))?;
    let num_docs = LuceneTestCase::at_least(1000);
    let mut ids: Vec<i32> = (0..num_docs).collect();
    shuffle(&mut ids, &*random());
    for id in &ids {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &id.to_string(),
            Store::NO,
        )));
        w.add_document(&doc)?;
    }
    shuffle(&mut ids, &*random());
    let mut upto = 0;
    while upto < ids.len() {
        let left = ids.len() - upto;
        let inc = std::cmp::min(left, TestUtil::next_int(&*random(), 1, 20) as usize);
        let limit = upto + inc;
        while upto < limit {
            if verbose() {
                println!("TEST: delete id={}", ids[upto]);
            }
            w.delete_documents(term("id", &ids[upto].to_string()))?;
            upto += 1;
        }
        if verbose() {
            println!("\nTEST: now open reader");
        }
        let r = w.get_reader()?;
        assert_equals(&(num_docs - upto as i32), &r.num_docs());
        r.close()?;
    }

    w.close()?;
    drop((w, d));
    dir.close()
}

/// The anonymous `Analyzer` of `testIndexingThenDeleting`: whitespace
/// tokens, lower-cased, without payloads.
#[derive(Debug)]
struct NoPayloadsAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for NoPayloadsAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterDelete$4"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), true)?;
        Ok(tokenizer_components(std::sync::Arc::new(
            rucene::util::atomic_ref_cell::AtomicRefCell::new(tokenizer),
        )))
    }
}

/// TODO: this test can hit pathological cases (IW settings?) where it runs for far too long
/// Port of `TestIndexWriterDelete.testIndexingThenDeleting`.
fn test_indexing_then_deleting() -> Result<()> {
    // TODO: move this test to its own class and just @SuppressCodecs?
    // TODO: is it enough to just use newFSDirectory?
    let field_format = TestUtil::get_postings_format("field");
    LuceneTestCase::assume_false(
        "This test cannot run with SimpleText codec",
        field_format == "SimpleText",
    );
    LuceneTestCase::assume_false(
        "This test cannot run with Direct codec",
        field_format == "Direct",
    );
    let r = random();
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    // note this test explicitly disables payloads
    let analyzer: Arc<dyn Analyzer> = Arc::new(NoPayloadsAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    iwc.set_ram_buffer_size_mb(4.0)?;
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "go 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20",
        Store::NO,
    )));
    let num = LuceneTestCase::at_least(1);
    for _ in 0..num {
        let mut count = 0;

        let do_indexing = r.next_boolean();
        if verbose() {
            println!("TEST: iter doIndexing={do_indexing}");
        }
        if do_indexing {
            // Add docs until a flush is triggered
            let start_flush_count = w.get_flush_count();
            while w.get_flush_count() == start_flush_count {
                w.add_document(&doc)?;
                count += 1;
            }
        } else {
            // Delete docs until a flush is triggered
            let start_flush_count = w.get_flush_count();
            while w.get_flush_count() == start_flush_count {
                w.delete_documents_by_terms(vec![term("foo", &count.to_string())])?;
                count += 1;
            }
        }
        Assert::assert_true_msg(
            &format!(
                "flush happened too quickly during {} count={count}",
                if do_indexing { "indexing" } else { "deleting" }
            ),
            count > 2500,
        );
    }
    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-3340: make sure deletes that we don't apply
/// during flush (ie are just pushed into the stream) are
/// in fact later flushed due to their RAM usage:
/// Port of `TestIndexWriterDelete.testFlushPushedDeletesByRAM`.
fn test_flush_pushed_deletes_by_ram() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    // Cannot use RandomIndexWriter because we don't want to
    // ever call commit() for this test:
    // note: tiny RAM buffer used, as with a 1MB buffer the test is too slow (flush @ 128,999)
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_ram_buffer_size_mb(0.5f32 as f64)?;
    iwc.set_max_buffered_docs(1000)?;
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    iwc.set_reader_pooling(false);
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let mut count = 0;
    loop {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            count.to_string(),
            Store::NO,
        )?));
        let del_term = if count == 1010 {
            // This is the only delete that applies
            term("id", "0")
        } else {
            // These get buffered, taking up RAM, but delete
            // nothing when applied:
            term("id", &format!("x{count}"))
        };
        w.update_document(del_term, &doc)?;
        // Eventually segment 0 should get a del docs:
        // TODO: fix this test
        if LuceneTestCase::slow_file_exists(&*d, "_0_1.del")?
            || LuceneTestCase::slow_file_exists(&*d, "_0_1.liv")?
        {
            if verbose() {
                println!("TEST: deletes created @ count={count}");
            }
            break;
        }
        count += 1;

        // Today we applyDeletes @ count=21553; even if we make
        // sizable improvements to RAM efficiency of buffered
        // del term we're unlikely to go over 100K:
        if count > 100000 {
            Assert::fail("delete's were not applied");
        }
    }
    w.close()?;
    drop(d);
    dir.close()
}

/// The `IndexWriter` subclass of `testApplyDeletesOnFlush`, overriding
/// `doAfterFlush`.
struct ApplyDeletesOnFlushWriter {
    docs_in_segment: Arc<AtomicI32>,
    closing: Arc<AtomicBool>,
    saw_after_flush: Arc<AtomicBool>,
}

impl IndexWriterOverrides for ApplyDeletesOnFlushWriter {
    fn do_after_flush(&self, _writer: &IndexWriter) -> Result<()> {
        Assert::assert_true_msg(
            &format!(
                "only {} in segment",
                self.docs_in_segment.load(Ordering::SeqCst)
            ),
            self.closing.load(Ordering::SeqCst) || self.docs_in_segment.load(Ordering::SeqCst) >= 7,
        );
        self.docs_in_segment.store(0, Ordering::SeqCst);
        self.saw_after_flush.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Make sure buffered (pushed) deletes don't use up so
/// much RAM that it forces long tail of tiny segments:
/// Port of `TestIndexWriterDelete.testApplyDeletesOnFlush`.
fn test_apply_deletes_on_flush() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    // Cannot use RandomIndexWriter because we don't want to
    // ever call commit() for this test:
    let docs_in_segment = Arc::new(AtomicI32::new(0));
    let closing = Arc::new(AtomicBool::new(false));
    let saw_after_flush = Arc::new(AtomicBool::new(false));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_ram_buffer_size_mb(0.5)?;
    iwc.set_max_buffered_docs(-1)?;
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    iwc.set_reader_pooling(false);
    // always use CFS so we don't use tons of file handles in the test
    iwc.set_use_compound_file(true)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    w.set_overrides(Arc::new(ApplyDeletesOnFlushWriter {
        docs_in_segment: Arc::clone(&docs_in_segment),
        closing: Arc::clone(&closing),
        saw_after_flush: Arc::clone(&saw_after_flush),
    }));
    let mut id = 0;
    loop {
        let mut sb = String::new();
        for _ in 0..100 {
            sb.push(' ');
            sb.push_str(&TestUtil::random_realistic_unicode_string(&*random()));
        }
        if id == 500 {
            w.delete_documents_by_terms(vec![term("id", "0")])?;
        }
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &id.to_string(),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "body",
            &sb,
            Store::NO,
        )));
        w.update_document(term("id", &id.to_string()), &doc)?;
        docs_in_segment.fetch_add(1, Ordering::SeqCst);
        // TODO: fix this test
        if LuceneTestCase::slow_file_exists(&*d, "_0_1.del")?
            || LuceneTestCase::slow_file_exists(&*d, "_0_1.liv")?
        {
            if verbose() {
                println!("TEST: deletes created @ id={id}");
            }
            break;
        }
        id += 1;
    }
    closing.store(true, Ordering::SeqCst);
    Assert::assert_true(saw_after_flush.load(Ordering::SeqCst));
    w.close()?;
    drop(d);
    dir.close()
}

/// `new PrintStream(bos, false, UTF_8)` over a `ByteArrayOutputStream(1024)`,
/// with `CheckIndex.checkIndex(null)`: the report, and whether it is clean.
fn check_index_output(d: &Arc<dyn Directory>) -> Result<(bool, String)> {
    let bos = Arc::new(Mutex::new(Vec::with_capacity(1024)));
    let mut checker = CheckIndex::new(Arc::clone(d))?;
    checker.set_info_stream_verbose(Some(Arc::new(PrintStreamLines(Arc::clone(&bos)))), false);
    let index_status = checker.check_index_segments(None)?;
    let clean = index_status.clean;
    checker.close()?;
    let s = String::from_utf8_lossy(&bos.lock().unwrap_or_else(|e| e.into_inner())).into_owned();
    Ok((clean, s))
}

/// LUCENE-4455
/// Port of `TestIndexWriterDelete.testDeletesCheckIndexOutput`.
fn test_deletes_check_index_output() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = plain_iwc();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    iwc.set_max_buffered_docs(2)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "0",
        StringField::type_not_stored(),
    )));
    w.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "1",
        StringField::type_not_stored(),
    )));
    w.add_document(&doc)?;
    w.commit()?;
    assert_equals(&1, &w.get_segment_count()?);

    w.delete_documents_by_terms(vec![term("field", "0")])?;
    w.commit()?;
    assert_equals(&1, &w.get_segment_count()?);
    w.close()?;

    let (clean, s) = check_index_output(&d)?;
    Assert::assert_true(clean);

    // Segment should have deletions:
    Assert::assert_true(s.contains("has deletions"));
    let iwc = plain_iwc();
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    w.force_merge(1)?;
    w.close()?;

    let (clean, s) = check_index_output(&d)?;
    Assert::assert_true(clean);
    Assert::assert_false(s.contains("has deletions"));
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterDelete.testTryDeleteDocument`.
fn test_try_delete_document() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let iwc = plain_iwc();
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let doc = Document::new();
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    w.close()?;

    let iwc = plain_iwc();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    iwc.set_open_mode(OpenMode::APPEND)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let r = directory_reader::open_with_writer_flags(&*w, false, false)?;
    let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    Assert::assert_true(w.try_delete_document(&ir, 1)? != -1);
    Assert::assert_false(r.is_current()?);
    let leaf0 = Arc::clone(&ir).leaves()?[0].leaf_reader().as_index_reader();
    Assert::assert_true(w.try_delete_document(&leaf0, 0)? != -1);
    Assert::assert_false(r.is_current()?);
    r.close()?;
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&2, &r.num_deleted_docs());
    let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    assert_not_null(&MultiBits::get_live_docs(&ir)?);
    r.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterDelete.testNRTIsCurrentAfterDelete`.
fn test_nrt_is_current_after_delete() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = plain_iwc();
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let mut doc = Document::new();
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    doc.add(Box::new(StringField::new(
        "id",
        "1".to_string(),
        Store::YES,
    )?));
    w.add_document(&doc)?;
    w.close()?;
    let iwc = plain_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    let r = directory_reader::open_with_writer_flags(&*w, false, false)?;
    w.delete_documents_by_terms(vec![term("id", "1")])?;
    let r2 = directory_reader::open_with_writer_flags(&*w, true, true)?;
    Assert::assert_false(r.is_current()?);
    Assert::assert_true(r2.is_current()?);
    drop(d);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| r2.close()),
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// `new LogDocMergePolicy()` with `setMinMergeDocs(1)`, and
/// `setMaxBufferedDocs(2)` and a `SerialMergeScheduler`, as the
/// `testOnlyDeletes*` and `testMergingAfterDeleteAll` methods configure.
fn log_doc_iwc() -> Result<IndexWriterConfig> {
    let iwc = plain_iwc();
    iwc.set_max_buffered_docs(2)?;
    let mut mp = LogMergePolicy::new(Arc::new(LogDocSize));
    mp.set_min_merge_docs(1);
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    Ok(iwc)
}

/// `doc.add(newStringField("id", "" + i, Field.Store.NO)); w.addDocument(doc)`.
fn add_id_doc(w: &IndexWriter, i: i32) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        &i.to_string(),
        Store::NO,
    )));
    w.add_document(&doc)?;
    Ok(())
}

/// Port of `TestIndexWriterDelete.testOnlyDeletesTriggersMergeOnClose`.
fn test_only_deletes_triggers_merge_on_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), log_doc_iwc()?)?;
    for i in 0..38 {
        add_id_doc(&w, i)?;
    }
    w.commit()?;

    for i in 0..18 {
        w.delete_documents_by_terms(vec![term("id", &i.to_string())])?;
    }

    w.close()?;
    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&1, &Arc::clone(&r).leaves()?.len());
    r.close()?;

    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterDelete.testOnlyDeletesTriggersMergeOnGetReader`.
fn test_only_deletes_triggers_merge_on_get_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), log_doc_iwc()?)?;
    for i in 0..38 {
        add_id_doc(&w, i)?;
    }
    w.commit()?;

    for i in 0..18 {
        w.delete_documents_by_terms(vec![term("id", &i.to_string())])?;
    }

    // First one triggers, but does not reflect, the merge:
    if verbose() {
        println!("TEST: now get reader");
    }
    directory_reader::open_with_writer(&*w)?.close()?;
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &Arc::clone(&r).leaves()?.len());
    r.close()?;

    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterDelete.testOnlyDeletesTriggersMergeOnFlush`.
fn test_only_deletes_triggers_merge_on_flush() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), log_doc_iwc()?)?;
    for i in 0..38 {
        if verbose() {
            println!("TEST: add doc {i}");
        }
        add_id_doc(&w, i)?;
    }
    if verbose() {
        println!("TEST: commit1");
    }
    w.commit()?;

    // Deleting 18 out of the 20 docs in the first segment make it the same "level" as the other 9
    // which should cause a merge to kick off:
    for i in 0..18 {
        w.delete_documents_by_terms(vec![term("id", &i.to_string())])?;
    }
    if verbose() {
        println!("TEST: commit2");
    }
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&1, &Arc::clone(&r).leaves()?.len());
    r.close()?;

    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterDelete.testOnlyDeletesDeleteAllDocs`.
fn test_only_deletes_delete_all_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), log_doc_iwc()?)?;
    for i in 0..38 {
        add_id_doc(&w, i)?;
    }
    w.commit()?;

    for i in 0..38 {
        w.delete_documents_by_terms(vec![term("id", &i.to_string())])?;
    }

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&0, &Arc::clone(&r).leaves()?.len());
    assert_equals(&0, &r.max_doc());
    r.close()?;

    w.close()?;
    drop(d);
    dir.close()
}

/// Make sure merges still kick off after IW.deleteAll!
/// Port of `TestIndexWriterDelete.testMergingAfterDeleteAll`.
fn test_merging_after_delete_all() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = IndexWriter::new(Arc::clone(&d), log_doc_iwc()?)?;
    for i in 0..10 {
        add_id_doc(&w, i)?;
    }
    w.commit()?;
    w.delete_all()?;

    for i in 0..100 {
        add_id_doc(&w, i)?;
    }

    w.force_merge(1)?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &Arc::clone(&r).leaves()?.len());
    r.close()?;

    w.close()?;
    drop(d);
    dir.close()
}
