//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterReader.java`.
//!
//! * `id.setStringValue(...)` on a field already added to the document
//!   changes the document's own field (`crate::support::field_mut`).
//! * `reader1 == reader2` (object identity) compares the `Arc` addresses.
//! * A `Thread` whose `run` catches a `Throwable`, records it and throws
//!   `new RuntimeException(t)` panics with that throwable.
//! * `Comparator.comparingLong(...)` (with `reversed()`) is a
//!   [`LeafComparator`] over the same key.

use std::cmp::Ordering as CmpOrdering;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::{Document, Field, LongPoint, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{
    IndexWriterConfig, LeafComparator, LiveIndexWriterConfig, MergedSegmentWarmer,
};
use rucene::index::leaf_reader::LeafReader;
use rucene::index::merge_policy::{LogMergePolicy, NoMergePolicy};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::multi_bits::MultiBits;
use rucene::index::sorting_codec_reader::SimpleMergedSegmentWarmer;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::{TermQuery, NO_MORE_DOCS};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::{BytesRef, InfoStream, Version};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::{AssertingDirectoryReader, DocHelper};
use rucene_test_framework::store::{
    fake_io_exception, Failure, MockDirectoryWrapper, FAKE_IO_EXCEPTION,
};
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::lucene_test_case::{
    Nightly, SuppressCodecs, TestDirectory, TEST_NIGHTLY,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class_annotated, random, test,
    verbose,
};
use crate::support::field_mut;

const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";

/// `numThreads`.
fn num_threads() -> usize {
    if *TEST_NIGHTLY {
        5
    } else {
        2
    }
}

/// `TestIndexWriterReader` (`@SuppressCodecs("SimpleText")`, too slow here).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterReader",
        vec![
            test("testAddCloseOpen", test_add_close_open),
            test("testUpdateDocument", test_update_document),
            test("testIsCurrent", test_is_current),
            test("testAddIndexes", test_add_indexes),
            test("testAddIndexes2", test_add_indexes2),
            test("testDeleteFromIndexWriter", test_delete_from_index_writer),
            test(
                "testAddIndexesAndDoDeletesThreads",
                test_add_indexes_and_do_deletes_threads,
            ),
            test(
                "testIndexWriterReopenSegmentFullMerge",
                test_index_writer_reopen_segment_full_merge,
            ),
            test(
                "testIndexWriterReopenSegment",
                test_index_writer_reopen_segment,
            ),
            test("testMergeWarmer", test_merge_warmer),
            test("testAfterCommit", test_after_commit),
            test("testAfterClose", test_after_close),
            test("testDuringAddIndexes", test_during_add_indexes).annotate(Nightly),
            test("testDuringAddDelete", test_during_add_delete),
            test("testForceMergeDeletes", test_force_merge_deletes),
            test("testDeletesNumDocs", test_deletes_num_docs),
            test("testEmptyIndex", test_empty_index),
            test("testSegmentWarmer", test_segment_warmer),
            test(
                "testSimpleMergedSegmentWarmer",
                test_simple_merged_segment_warmer,
            ),
            test(
                "testReopenAfterNoRealChange",
                test_reopen_after_no_real_change,
            ),
            test("testNRTOpenExceptions", test_nrt_open_exceptions),
            test("testTooManySegments", test_too_many_segments),
            test(
                "testReopenNRTReaderOnCommit",
                test_reopen_nrt_reader_on_commit,
            ),
            test(
                "testIndexReaderWriterWithLeafSorter",
                test_index_reader_writer_with_leaf_sorter,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMaxFullFlushMergeWaitMillis(0)`.
fn no_wait_iwc() -> Result<IndexWriterConfig> {
    let iwc = mock_iwc();
    iwc.set_max_full_flush_merge_wait_millis(0);
    Ok(iwc)
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

/// `reader` as the `IndexReader` it is.
fn ir(reader: &Arc<dyn DirectoryReader>) -> Arc<dyn IndexReader> {
    Arc::clone(reader) as Arc<dyn IndexReader>
}

/// `a == b` on two readers.
fn same<A: ?Sized, B: ?Sized>(a: &Arc<A>, b: &Arc<B>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// The public static `count(Term, IndexReader)`.
pub fn count(t: &Term, r: &Arc<dyn IndexReader>) -> Result<i32> {
    let mut count = 0;
    let td = TestUtil::docs(
        &*random(),
        r,
        t.field(),
        &BytesRef::from_char_sequence(&t.text()),
        None,
        0,
    )?;

    if let Some(mut td) = td {
        let live_docs = MultiBits::get_live_docs(r)?;
        while td.next_doc()? != NO_MORE_DOCS {
            td.doc_id();
            if live_docs
                .as_ref()
                .is_none_or(|l| l.get(td.doc_id() as usize))
            {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Port of `TestIndexWriterReader.testAddCloseOpen`.
fn test_add_close_open() -> Result<()> {
    // Can't use assertNoDeletes: this test pulls a non-NRT
    // reader in the end:
    let mut dir1 = LuceneTestCase::new_directory();
    let iwc = mock_iwc();

    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;
    for i in 0..97 {
        let reader = directory_reader::open_with_writer(&*writer)?;
        if i == 0 {
            writer.add_document(&DocHelper::create_document(
                i,
                "x",
                1 + random().next_int_bound(5),
            ))?;
        } else {
            let previous = random().next_int_bound(i);
            // a check if the reader is current here could fail since there might be
            // merges going on.
            match random().next_int_bound(5) {
                0..=2 => {
                    writer.add_document(&DocHelper::create_document(
                        i,
                        "x",
                        1 + random().next_int_bound(5),
                    ))?;
                }
                3 => {
                    writer.update_document(
                        term("id", &previous.to_string()),
                        &DocHelper::create_document(previous, "x", 1 + random().next_int_bound(5)),
                    )?;
                }
                4 => {
                    writer.delete_documents_by_terms(vec![term("id", &previous.to_string())])?;
                }
                _ => {}
            }
        }
        Assert::assert_false(reader.is_current()?);
        reader.close()?;
    }
    writer.force_merge(1)?; // make sure all merging is done etc.
    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.commit()?; // no changes that are not visible to the reader

    // A commit is now seen as a change to an NRT reader:
    Assert::assert_false(reader.is_current()?);
    reader.close()?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(reader.is_current()?);
    writer.close()?;

    Assert::assert_true(reader.is_current()?);
    let iwc = mock_iwc();
    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;
    Assert::assert_true(reader.is_current()?);
    writer.add_document(&DocHelper::create_document(
        1,
        "x",
        1 + random().next_int_bound(5),
    ))?;
    Assert::assert_true(reader.is_current()?); // segments in ram but IW is different to the readers one
    writer.close()?;
    Assert::assert_false(reader.is_current()?); // segments written
    reader.close()?;
    drop(writer);
    dir1.close()
}

/// Port of `TestIndexWriterReader.testUpdateDocument`.
fn test_update_document() -> Result<()> {
    let do_full_merge = true;

    let mut dir1 = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    if iwc.max_buffered_docs() < 20 {
        iwc.set_max_buffered_docs(20)?;
    }
    // no merging
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    if verbose() {
        println!("TEST: make index");
    }
    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;

    // create the index
    create_index_no_close(!do_full_merge, "index1", &writer)?;

    // writer.flush(false, true, true);

    // get a reader
    let r1 = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(r1.is_current()?);

    let id10 = r1
        .stored_fields()?
        .document(10)?
        .get_field("id")
        .and_then(|f| f.string_value())
        .unwrap_or_else(|| crate::support::npe("id"));

    let mut new_doc = r1.stored_fields()?.document(10)?;
    new_doc.remove_field("id");
    new_doc.add(Box::new(Field::new(
        "id",
        8000.to_string(),
        DocHelper::string_type_stored_with_tvs(),
    )?));
    writer.update_document(term("id", &id10), &new_doc)?;
    Assert::assert_false(r1.is_current()?);

    println!("TEST: now get reader");
    let r2 = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(r2.is_current()?);
    assert_equals(&0, &count(&term("id", &id10), &ir(&r2))?);
    if verbose() {
        println!("TEST: verify id");
    }
    assert_equals(&1, &count(&term("id", &8000.to_string()), &ir(&r2))?);

    r1.close()?;
    Assert::assert_true(r2.is_current()?);
    writer.close()?;
    // writer.close wrote a new commit
    Assert::assert_false(r2.is_current()?);

    let r3 = directory_reader::open(dir1.as_directory())?;
    Assert::assert_true(r3.is_current()?);
    Assert::assert_false(r2.is_current()?);
    assert_equals(&0, &count(&term("id", &id10), &ir(&r3))?);
    assert_equals(&1, &count(&term("id", &8000.to_string()), &ir(&r3))?);

    let writer = IndexWriter::new(dir1.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Assert::assert_false(r2.is_current()?);
    Assert::assert_true(r3.is_current()?);

    writer.close()?;

    Assert::assert_false(r2.is_current()?);
    Assert::assert_true(!r3.is_current()?);

    r2.close()?;
    r3.close()?;

    drop(writer);
    dir1.close()
}

/// Port of `TestIndexWriterReader.testIsCurrent`.
fn test_is_current() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();

    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    writer.close()?;

    let iwc = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c",
        Store::NO,
    )));
    let nrt_reader = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(nrt_reader.is_current()?);
    writer.add_document(&doc)?;
    Assert::assert_false(nrt_reader.is_current()?); // should see the changes
    writer.force_merge(1)?; // make sure we don't have a merge going on
    Assert::assert_false(nrt_reader.is_current()?);
    nrt_reader.close()?;

    let dir_reader = directory_reader::open(dir.as_directory())?;
    let nrt_reader = directory_reader::open_with_writer(&*writer)?;

    Assert::assert_true(dir_reader.is_current()?);
    Assert::assert_true(nrt_reader.is_current()?); // nothing was committed yet so we are still current
    assert_equals(&2, &nrt_reader.max_doc()); // sees the actual document added
    assert_equals(&1, &dir_reader.max_doc());
    writer.close()?; // close is actually a commit both should see the changes
    Assert::assert_false(nrt_reader.is_current()?);
    Assert::assert_false(dir_reader.is_current()?); // this reader has been opened before the writer was closed / committed

    dir_reader.close()?;
    nrt_reader.close()?;
    drop(writer);
    dir.close()
}

/// Test using IW.addIndexes
///
/// Port of `TestIndexWriterReader.testAddIndexes`.
fn test_add_indexes() -> Result<()> {
    let do_full_merge = false;

    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let iwc = no_wait_iwc()?;
    if iwc.max_buffered_docs() < 20 {
        iwc.set_max_buffered_docs(20)?;
    }
    // no merging
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;

    // create the index
    create_index_no_close(!do_full_merge, "index1", &writer)?;
    writer.flush_with(false, true)?;

    // create a 2nd index
    let mut dir2 = LuceneTestCase::new_directory();
    let writer2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    create_index_no_close(!do_full_merge, "index2", &writer2)?;
    writer2.close()?;

    let r0 = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(r0.is_current()?);
    writer.add_indexes_dirs(&[dir2.as_directory()])?;
    Assert::assert_false(r0.is_current()?);
    r0.close()?;

    let r1 = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(r1.is_current()?);

    writer.commit()?;

    // A commit is seen as a change to NRT reader:
    Assert::assert_false(r1.is_current()?);

    assert_equals(&200, &r1.max_doc());

    let index2df = r1.doc_freq(&term("indexname", "index2"))?;

    assert_equals(&100, &index2df);

    // verify the docs are from different indexes
    let doc5 = r1.stored_fields()?.document(5)?;
    assert_equals(&Some("index1".to_string()), &doc5.get("indexname"));
    let doc150 = r1.stored_fields()?.document(150)?;
    assert_equals(&Some("index2".to_string()), &doc150.get("indexname"));
    r1.close()?;
    writer.close()?;
    drop((writer, writer2));
    dir1.close()?;
    dir2.close()
}

/// Port of `TestIndexWriterReader.testAddIndexes2`.
fn test_add_indexes2() -> Result<()> {
    let do_full_merge = false;

    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let writer = IndexWriter::new(dir1.as_directory(), no_wait_iwc()?)?;

    // create a 2nd index
    let mut dir2 = LuceneTestCase::new_directory();
    let writer2 = IndexWriter::new(dir2.as_directory(), no_wait_iwc()?)?;
    create_index_no_close(!do_full_merge, "index2", &writer2)?;
    writer2.close()?;

    for _ in 0..5 {
        writer.add_indexes_dirs(&[dir2.as_directory()])?;
    }

    let r1 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&500, &r1.max_doc());

    r1.close()?;
    writer.close()?;
    drop((writer, writer2));
    dir1.close()?;
    dir2.close()
}

/// Deletes using IW.deleteDocuments
///
/// Port of `TestIndexWriterReader.testDeleteFromIndexWriter`.
fn test_delete_from_index_writer() -> Result<()> {
    let do_full_merge = true;

    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let writer = IndexWriter::new(dir1.as_directory(), no_wait_iwc()?)?;
    // create the index
    create_index_no_close(!do_full_merge, "index1", &writer)?;
    writer.flush_with(false, true)?;
    // get a reader
    let r1 = directory_reader::open_with_writer(&*writer)?;

    let stored_id = |r: &Arc<dyn DirectoryReader>, doc: i32| -> Result<String> {
        Ok(r.stored_fields()?
            .document(doc)?
            .get_field("id")
            .and_then(|f| f.string_value())
            .unwrap_or_else(|| crate::support::npe("id")))
    };
    let id10 = stored_id(&r1, 10)?;

    // deleted IW docs should not show up in the next getReader
    writer.delete_documents_by_terms(vec![term("id", &id10)])?;
    let r2 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&1, &count(&term("id", &id10), &ir(&r1))?);
    assert_equals(&0, &count(&term("id", &id10), &ir(&r2))?);

    let id50 = stored_id(&r1, 50)?;
    assert_equals(&1, &count(&term("id", &id50), &ir(&r1))?);

    writer.delete_documents_by_terms(vec![term("id", &id50)])?;

    let r3 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&0, &count(&term("id", &id10), &ir(&r3))?);
    assert_equals(&0, &count(&term("id", &id50), &ir(&r3))?);

    let id75 = stored_id(&r1, 75)?;
    writer.delete_documents_by_queries(vec![Arc::new(TermQuery::new(term("id", &id75)))])?;
    let r4 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&1, &count(&term("id", &id75), &ir(&r3))?);
    assert_equals(&0, &count(&term("id", &id75), &ir(&r4))?);

    r1.close()?;
    r2.close()?;
    r3.close()?;
    r4.close()?;
    writer.close()?;

    // reopen the writer to verify the delete made it to the directory
    let writer = IndexWriter::new(dir1.as_directory(), no_wait_iwc()?)?;
    let w2r1 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&0, &count(&term("id", &id10), &ir(&w2r1))?);
    w2r1.close()?;
    writer.close()?;
    drop(writer);
    dir1.close()
}

/// Port of `TestIndexWriterReader.testAddIndexesAndDoDeletesThreads`.
fn test_add_indexes_and_do_deletes_threads() -> Result<()> {
    let num_iter = if *TEST_NIGHTLY { 2 } else { 1 };
    let num_dirs = if *TEST_NIGHTLY { 3 } else { 2 };

    let mut main_dir = get_assert_no_deletes_directory(LuceneTestCase::new_directory());

    let iwc = no_wait_iwc()?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let main_writer = IndexWriter::new(main_dir.as_directory(), iwc)?;
    TestUtil::reduce_open_files(&main_writer)?;

    let mut add_dir_threads = AddDirectoriesThreads::new(num_iter, Arc::clone(&main_writer))?;
    add_dir_threads.launch_threads(num_dirs);
    add_dir_threads.join_threads();

    // assertEquals(100 + numDirs * (3 * numIter / 4) * addDirThreads.numThreads
    //    * addDirThreads.NUM_INIT_DOCS, addDirThreads.mainwriter.getDocStats().numDocs);
    assert_equals(
        &add_dir_threads.shared.count.load(Ordering::SeqCst),
        &add_dir_threads.shared.main_writer.get_doc_stats()?.num_docs,
    );

    add_dir_threads.close(true)?;

    Assert::assert_true(
        add_dir_threads
            .shared
            .failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty(),
    );

    TestUtil::check_index(main_dir.as_directory())?;

    let reader = directory_reader::open(main_dir.as_directory())?;
    assert_equals(
        &add_dir_threads.shared.count.load(Ordering::SeqCst),
        &reader.num_docs(),
    );
    // assertEquals(100 + numDirs * (3 * numIter / 4) * addDirThreads.numThreads
    //    * addDirThreads.NUM_INIT_DOCS, reader.numDocs());
    reader.close()?;

    add_dir_threads.close_dir()?;
    drop((add_dir_threads, main_writer));
    main_dir.close()
}

/// `AddDirectoriesThreads.NUM_INIT_DOCS`.
const NUM_INIT_DOCS: i32 = 100;

/// The state `AddDirectoriesThreads` shares with its threads.
struct AddDirectoriesShared {
    add_dir: Arc<dyn Directory>,
    num_dirs: usize,
    main_writer: Arc<IndexWriter>,
    failures: Mutex<Vec<Throwable>>,
    readers: Vec<Arc<dyn DirectoryReader>>,
    count: AtomicI32,
    numadd_indexes: AtomicI32,
}

/// The private inner class `AddDirectoriesThreads`.
struct AddDirectoriesThreads {
    add_dir: TestDirectory,
    threads: Vec<Thread>,
    shared: Arc<AddDirectoriesShared>,
}

impl AddDirectoriesThreads {
    fn new(num_dirs: usize, main_writer: Arc<IndexWriter>) -> Result<Self> {
        let add_dir = LuceneTestCase::new_directory();
        let iwc = no_wait_iwc()?;
        iwc.set_max_buffered_docs(2)?;
        let writer = IndexWriter::new(add_dir.as_directory(), iwc)?;
        TestUtil::reduce_open_files(&writer)?;
        for i in 0..NUM_INIT_DOCS {
            let doc = DocHelper::create_document(i, "addindex", 4);
            writer.add_document(&doc)?;
        }

        writer.close()?;

        let mut readers = Vec::with_capacity(num_dirs);
        for _ in 0..num_dirs {
            readers.push(directory_reader::open(add_dir.as_directory())?);
        }
        Ok(AddDirectoriesThreads {
            shared: Arc::new(AddDirectoriesShared {
                add_dir: add_dir.as_directory(),
                num_dirs,
                main_writer,
                failures: Mutex::new(Vec::new()),
                readers,
                count: AtomicI32::new(0),
                numadd_indexes: AtomicI32::new(0),
            }),
            add_dir,
            threads: Vec::new(),
        })
    }

    fn join_threads(&self) {
        for t in &self.threads {
            let _ = t.join();
        }
    }

    fn close(&self, do_wait: bool) -> Result<()> {
        if do_wait {
            self.shared.main_writer.close()
        } else {
            self.shared.main_writer.rollback()
        }
    }

    fn close_dir(&mut self) -> Result<()> {
        for r in &self.shared.readers {
            r.close()?;
        }
        self.add_dir.close()
    }

    fn launch_threads(&mut self, num_iter: i32) {
        for _ in 0..num_threads() {
            let shared = Arc::clone(&self.shared);
            self.threads.push(Thread::unnamed(move || {
                let run = || -> Result<()> {
                    let mut dirs: Vec<Arc<dyn Directory>> = Vec::with_capacity(shared.num_dirs);
                    for _ in 0..shared.num_dirs {
                        let copy = TestUtil::ram_copy_of(&*shared.add_dir)?;
                        dirs.push(
                            MockDirectoryWrapper::new(&*random(), Arc::from(copy)).as_directory(),
                        );
                    }
                    // int j = 0;
                    // while (true) {
                    // System.out.println(Thread.currentThread().getName() + ": iter
                    // j=" + j);
                    for x in 0..num_iter {
                        // only do addIndexes
                        shared.do_body(x, &dirs)?;
                    }
                    // if (numIter > 0 && j == numIter)
                    //  break;
                    // doBody(j++, dirs);
                    // doBody(5, dirs);
                    // }
                    Ok(())
                };
                if let Err(e) = run() {
                    shared.handle(java_throwable(&e));
                }
            }));
        }
        for t in &self.threads {
            t.start();
        }
    }
}

impl AddDirectoriesShared {
    fn handle(&self, t: Throwable) {
        println!("{t}");
        self.failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(t);
    }

    fn do_body(&self, j: i32, dirs: &[Arc<dyn Directory>]) -> Result<()> {
        match j % 4 {
            0 => {
                self.main_writer.add_indexes_dirs(dirs)?;
                self.main_writer.force_merge(1)?;
            }
            1 => {
                self.main_writer.add_indexes_dirs(dirs)?;
                self.numadd_indexes.fetch_add(1, Ordering::SeqCst);
            }
            2 => {
                TestUtil::add_indexes_slowly(&self.main_writer, &self.readers)?;
            }
            3 => {
                self.main_writer.commit()?;
            }
            _ => {}
        }
        self.count
            .fetch_add(dirs.len() as i32 * NUM_INIT_DOCS, Ordering::SeqCst);
        Ok(())
    }
}

/// Port of `TestIndexWriterReader.testIndexWriterReopenSegmentFullMerge`.
fn test_index_writer_reopen_segment_full_merge() -> Result<()> {
    do_test_index_writer_reopen_segment(true)
}

/// Port of `TestIndexWriterReader.testIndexWriterReopenSegment`.
fn test_index_writer_reopen_segment() -> Result<()> {
    do_test_index_writer_reopen_segment(false)
}

/// Tests creating a segment, then check to insure the segment can be seen via IW.getReader
///
/// The public `doTestIndexWriterReopenSegment(boolean)`.
fn do_test_index_writer_reopen_segment(do_full_merge: bool) -> Result<()> {
    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let writer = IndexWriter::new(dir1.as_directory(), no_wait_iwc()?)?;
    let r1 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&0, &r1.max_doc());
    create_index_no_close(false, "index1", &writer)?;
    writer.flush_with(!do_full_merge, true)?;

    let iwr1 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&100, &iwr1.max_doc());

    let r2 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&r2.max_doc(), &100);
    // add 100 documents
    for x in 10000..10000 + 100 {
        let d = DocHelper::create_document(x, "index1", 5);
        writer.add_document(&d)?;
    }
    writer.flush_with(false, true)?;
    // verify the reader was reopened internally
    let iwr2 = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(!same(&iwr2, &r1));
    assert_equals(&200, &iwr2.max_doc());
    // should have flushed out a segment
    let r3 = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_true(!same(&r2, &r3));
    assert_equals(&200, &r3.max_doc());

    // dec ref the readers rather than close them because
    // closing flushes changes to the writer
    r1.close()?;
    iwr1.close()?;
    r2.close()?;
    r3.close()?;
    iwr2.close()?;
    writer.close()?;

    // test whether the changes made it to the directory
    let writer = IndexWriter::new(dir1.as_directory(), no_wait_iwc()?)?;
    let w2r1 = directory_reader::open_with_writer(&*writer)?;
    // insure the deletes were actually flushed to the directory
    assert_equals(&200, &w2r1.max_doc());
    w2r1.close()?;
    writer.close()?;

    drop(writer);
    dir1.close()
}

// Delete a document by term and return the doc id
//
// public static int deleteDocument(Term term, IndexWriter writer) throws
// IOException { IndexReader reader = writer.getReader(); TermDocs td =
// reader.termDocs(term); int doc = -1; //if (td.next()) { // doc = td.storedFields().document();
// //} //writer.deleteDocuments(term); td.close(); return doc; }

/// The public static `createIndex(Random, Directory, String, boolean)`.
#[allow(dead_code)]
pub fn create_index(
    random: &dyn Random,
    dir1: Arc<dyn Directory>,
    index_name: &str,
    multi_segment: bool,
) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config_with(
        random,
        Some(Arc::new(MockAnalyzer::new(random))),
    );
    iwc.set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))?;
    let w = IndexWriter::new(dir1, iwc)?;
    for i in 0..100 {
        w.add_document(&DocHelper::create_document(i, index_name, 4))?;
    }
    if !multi_segment {
        w.force_merge(1)?;
    }
    w.close()
}

/// The public static `createIndexNoClose(boolean, String, IndexWriter)`.
pub fn create_index_no_close(multi_segment: bool, index_name: &str, w: &IndexWriter) -> Result<()> {
    for i in 0..100 {
        w.add_document(&DocHelper::create_document(i, index_name, 4))?;
    }
    if !multi_segment {
        w.force_merge(1)?;
    }
    Ok(())
}

/// `(leafReader) -> warmCount.incrementAndGet()`.
#[derive(Debug)]
struct CountingWarmer(Arc<AtomicI32>);

impl std::fmt::Display for CountingWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestIndexWriterReader$$Lambda")
    }
}

impl MergedSegmentWarmer for CountingWarmer {
    fn warm(&self, _reader: &Arc<dyn LeafReader>) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `writer.getConfig().getMergeScheduler()` as the `ConcurrentMergeScheduler` it is.
fn cms_of(writer: &IndexWriter) -> Arc<dyn rucene::index::merge_scheduler::MergeScheduler> {
    writer.get_config().merge_scheduler()
}

/// `(ConcurrentMergeScheduler) ms`.
fn as_cms(
    ms: &Arc<dyn rucene::index::merge_scheduler::MergeScheduler>,
) -> &ConcurrentMergeScheduler {
    ms.as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "not a ConcurrentMergeScheduler",
            )
            .throw()
        })
}

/// Port of `TestIndexWriterReader.testMergeWarmer`.
fn test_merge_warmer() -> Result<()> {
    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    // Enroll warmer
    let warm_count = Arc::new(AtomicI32::new(0));
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_max_full_flush_merge_wait_millis(0);
    iwc.set_merged_segment_warmer(Some(Arc::new(CountingWarmer(Arc::clone(&warm_count)))))?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;

    // create the index
    create_index_no_close(false, "test", &writer)?;

    // get a reader to put writer into near real-time mode
    let r1 = directory_reader::open_with_writer(&*writer)?;

    let mp = writer.get_config().merge_policy();
    mp.as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a LogMergePolicy").throw()
        })
        .set_merge_factor(2)?;

    let num = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(100)
    } else {
        LuceneTestCase::at_least(10)
    };
    for i in 0..num {
        writer.add_document(&DocHelper::create_document(i, "test", 4))?;
    }
    as_cms(&cms_of(&writer)).sync();

    Assert::assert_true(warm_count.load(Ordering::SeqCst) > 0);
    let count = warm_count.load(Ordering::SeqCst);

    writer.add_document(&DocHelper::create_document(17, "test", 4))?;
    writer.force_merge(1)?;
    Assert::assert_true(warm_count.load(Ordering::SeqCst) > count);

    writer.close()?;
    r1.close()?;
    drop(writer);
    dir1.close()
}

/// Port of `TestIndexWriterReader.testAfterCommit`.
fn test_after_commit() -> Result<()> {
    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let iwc = mock_iwc();
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_max_full_flush_merge_wait_millis(0);
    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;
    writer.commit()?;

    // create the index
    create_index_no_close(false, "test", &writer)?;

    // get a reader to put writer into near real-time mode
    let mut r1 = directory_reader::open_with_writer(&*writer)?;
    TestUtil::check_index(dir1.as_directory())?;
    writer.commit()?;
    TestUtil::check_index(dir1.as_directory())?;
    assert_equals(&100, &r1.num_docs());

    for i in 0..10 {
        writer.add_document(&DocHelper::create_document(i, "test", 4))?;
    }
    as_cms(&cms_of(&writer)).sync();

    if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&r1))? {
        r1.close()?;
        r1 = r2;
    }
    assert_equals(&110, &r1.num_docs());
    writer.close()?;
    r1.close()?;
    drop(writer);
    dir1.close()
}

/// Make sure reader remains usable even if IndexWriter closes
///
/// Port of `TestIndexWriterReader.testAfterClose`.
fn test_after_close() -> Result<()> {
    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let writer = IndexWriter::new(dir1.as_directory(), no_wait_iwc()?)?;

    // create the index
    create_index_no_close(false, "test", &writer)?;

    let r = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;

    TestUtil::check_index(dir1.as_directory())?;

    // reader should remain usable even after IndexWriter is closed:
    assert_equals(&100, &r.num_docs());
    let q = Arc::new(TermQuery::new(term("indexname", "test")));
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    assert_equals(&100, &searcher.count(q)?);

    expect_throws(ACE, || directory_reader::open_if_changed(Arc::clone(&r)));

    r.close()?;
    drop(writer);
    dir1.close()
}

/// Stress test reopen during addIndexes
///
/// Port of `TestIndexWriterReader.testDuringAddIndexes`.
fn test_during_add_indexes() -> Result<()> {
    let mut dir1 = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let iwc = no_wait_iwc()?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(2)?,
    )))?;
    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;

    // create the index
    create_index_no_close(false, "test", &writer)?;
    writer.commit()?;

    let mut dirs: Vec<Arc<dyn Directory>> = Vec::with_capacity(10);
    for _ in 0..10 {
        let copy = TestUtil::ram_copy_of(&*dir1.as_directory())?;
        dirs.push(MockDirectoryWrapper::new(&*random(), Arc::from(copy)).as_directory());
    }
    let dirs = Arc::new(dirs);

    let mut r = directory_reader::open_with_writer(&*writer)?;

    let num_iterations = 10;
    let excs: Arc<Mutex<Vec<Throwable>>> = Arc::new(Mutex::new(Vec::new()));

    // Only one thread can addIndexes at a time, because
    // IndexWriter acquires a write lock in each directory:
    let thread_done = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::with_capacity(1);
    for _ in 0..1 {
        let writer = Arc::clone(&writer);
        let dirs = Arc::clone(&dirs);
        let excs = Arc::clone(&excs);
        let thread_done = Arc::clone(&thread_done);
        let t = Thread::unnamed(move || {
            let mut count = 0;
            loop {
                count += 1;
                let r = writer
                    .add_indexes_dirs(&dirs)
                    .and_then(|_| writer.maybe_merge());
                if let Err(e) = r {
                    let t = java_throwable(&e);
                    excs.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(t.clone());
                    Throwable::from_cause("java.lang.RuntimeException", t).throw();
                }
                if count >= num_iterations {
                    break;
                }
            }
            thread_done.store(true, Ordering::SeqCst);
        });
        t.set_daemon(true);
        t.start();
        threads.push(t);
    }

    let mut last_count: i64 = 0;
    while !thread_done.load(Ordering::SeqCst) {
        if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&r))? {
            r.close()?;
            r = r2;
            let q = Arc::new(TermQuery::new(term("indexname", "test")));
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
            let count = searcher.count(q)? as i64;
            Assert::assert_true(count >= last_count);
            last_count = count;
        }
    }

    for thread in &threads {
        let _ = thread.join();
    }
    // final check
    if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&r))? {
        r.close()?;
        r = r2;
    }
    let q = Arc::new(TermQuery::new(term("indexname", "test")));
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let count = searcher.count(q)? as i64;
    Assert::assert_true(count >= last_count);

    assert_equals(&0, &excs.lock().unwrap_or_else(|p| p.into_inner()).len());
    r.close()?;
    if let Some(mock) = dir1.as_mock() {
        let open_deleted_files = mock.get_open_deleted_files();
        assert_equals_msg(
            Some(&format!("openDeleted={open_deleted_files:?}")),
            &0,
            &open_deleted_files.len(),
        );
    }

    writer.close()?;

    drop((writer, dirs));
    dir1.close()
}

/// The private `getAssertNoDeletesDirectory(Directory)`.
fn get_assert_no_deletes_directory(directory: TestDirectory) -> TestDirectory {
    if let Some(mock) = directory.as_mock() {
        mock.set_assert_no_delete_open_file(true);
    }
    directory
}

/// Stress test reopen during add/delete
///
/// Port of `TestIndexWriterReader.testDuringAddDelete`.
fn test_during_add_delete() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(2)?,
    )))?;
    if *TEST_NIGHTLY {
        // if we have a ton of iterations we need to make sure we don't do unnecessary
        // extra flushing otherwise we will time out on nightly
        iwc.set_ram_buffer_size_mb(LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB)?;
        iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    }
    let writer = IndexWriter::new(dir1.as_directory(), iwc)?;

    // create the index
    create_index_no_close(false, "test", &writer)?;
    writer.commit()?;

    let mut r = directory_reader::open_with_writer(&*writer)?;

    let iters = if *TEST_NIGHTLY { 1000 } else { 10 };
    let excs: Arc<Mutex<Vec<Throwable>>> = Arc::new(Mutex::new(Vec::new()));

    let n = num_threads();
    let mut threads = Vec::with_capacity(n);
    let remaining_threads = Arc::new(AtomicI32::new(n as i32));
    for _ in 0..n {
        let writer = Arc::clone(&writer);
        let excs = Arc::clone(&excs);
        let remaining_threads = Arc::clone(&remaining_threads);
        let rnd = JdkRandom::new(random().next_long());
        let t = Thread::unnamed(move || {
            let mut count = 0;
            loop {
                let mut body = || -> Result<()> {
                    for doc_upto in 0..10 {
                        writer.add_document(&DocHelper::create_document(
                            10 * count + doc_upto,
                            "test",
                            4,
                        ))?;
                    }
                    count += 1;
                    let limit = count * 10;
                    for _ in 0..5 {
                        let x = rnd.next_int_bound(limit);
                        writer.delete_documents_by_terms(vec![term("field3", &format!("b{x}"))])?;
                    }
                    Ok(())
                };
                if let Err(e) = body() {
                    let t = java_throwable(&e);
                    excs.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(t.clone());
                    Throwable::from_cause("java.lang.RuntimeException", t).throw();
                }
                if count >= iters {
                    break;
                }
            }
            remaining_threads.fetch_sub(1, Ordering::SeqCst);
        });
        t.set_daemon(true);
        t.start();
        threads.push(t);
    }

    let mut sum = 0;
    while remaining_threads.load(Ordering::SeqCst) > 0 {
        if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&r))? {
            r.close()?;
            r = r2;
            let q = Arc::new(TermQuery::new(term("indexname", "test")));
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
            sum += searcher.count(q)?;
        }
    }

    for t in &threads {
        let _ = t.join();
    }
    // at least search once
    if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&r))? {
        r.close()?;
        r = r2;
    }
    let q = Arc::new(TermQuery::new(term("indexname", "test")));
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    sum += searcher.count(q)?;
    Assert::assert_true_msg("no documents found at all", sum > 0);

    assert_equals(&0, &excs.lock().unwrap_or_else(|p| p.into_inner()).len());
    writer.close()?;

    r.close()?;
    drop(writer);
    dir1.close()
}

/// Port of `TestIndexWriterReader.testForceMergeDeletes`.
fn test_force_merge_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "",
        Store::NO,
    )));
    field_mut(&mut doc, "id").set_string_value("0".to_string())?;
    w.add_document(&doc)?;
    field_mut(&mut doc, "id").set_string_value("1".to_string())?;
    w.add_document(&doc)?;
    w.delete_documents_by_terms(vec![term("id", "0")])?;

    let r = directory_reader::open_with_writer(&*w)?;
    w.force_merge_deletes()?;
    w.close()?;
    r.close()?;
    let r = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &r.num_docs());
    Assert::assert_false(r.has_deletions());
    r.close()?;
    drop(w);
    dir.close()
}

/// Port of `TestIndexWriterReader.testDeletesNumDocs`.
fn test_deletes_num_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "",
        Store::NO,
    )));
    field_mut(&mut doc, "id").set_string_value("0".to_string())?;
    w.add_document(&doc)?;
    field_mut(&mut doc, "id").set_string_value("1".to_string())?;
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&2, &r.num_docs());
    r.close()?;

    w.delete_documents_by_terms(vec![term("id", "0")])?;
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.num_docs());
    r.close()?;

    w.delete_documents_by_terms(vec![term("id", "1")])?;
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&0, &r.num_docs());
    r.close()?;

    w.close()?;
    drop(w);
    dir.close()
}

/// Port of `TestIndexWriterReader.testEmptyIndex`.
fn test_empty_index() -> Result<()> {
    // Ensures that getReader works on an empty index, which hasn't been committed yet.
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&0, &r.num_docs());
    r.close()?;
    w.close()?;
    drop(w);
    dir.close()
}

/// The warmer lambda of `testSegmentWarmer`.
#[derive(Debug)]
struct SearchingWarmer(Arc<AtomicBool>);

impl std::fmt::Display for SearchingWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestIndexWriterReader$$Lambda")
    }
}

impl MergedSegmentWarmer for SearchingWarmer {
    fn warm(&self, r: &Arc<dyn LeafReader>) -> Result<()> {
        let s = LuceneTestCase::new_searcher(Arc::clone(r));
        let count = s.count(Arc::new(TermQuery::new(term("foo", "bar"))))?;
        assert_equals(&20, &count);
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Port of `TestIndexWriterReader.testSegmentWarmer`.
fn test_segment_warmer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let did_warm = Arc::new(AtomicBool::new(false));
    let mut mp = LuceneTestCase::new_log_merge_policy_merge_factor(10)?;
    mp.set_target_search_concurrency(1)?;
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_reader_pooling(true);
    iwc.set_merged_segment_warmer(Some(Arc::new(SearchingWarmer(Arc::clone(&did_warm)))))?;
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "foo",
        "bar",
        Store::NO,
    )));
    for _ in 0..20 {
        w.add_document(&doc)?;
    }
    w.wait_for_merges()?;
    w.close()?;
    drop(w);
    dir.close()?;
    Assert::assert_true(did_warm.load(Ordering::SeqCst));
    Ok(())
}

/// The anonymous `InfoStream` of `testSimpleMergedSegmentWarmer`.
#[derive(Debug)]
struct SmswInfoStream(Arc<AtomicBool>);

impl InfoStream for SmswInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterReader$3"
    }

    fn close(&self) {}

    fn message(&self, component: &str, _message: &str) {
        if "SMSW" == component {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }
}

/// Port of `TestIndexWriterReader.testSimpleMergedSegmentWarmer`.
fn test_simple_merged_segment_warmer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let did_warm = Arc::new(AtomicBool::new(false));
    let info_stream: Arc<dyn InfoStream> = Arc::new(SmswInfoStream(Arc::clone(&did_warm)));
    let mut mp = LuceneTestCase::new_log_merge_policy_merge_factor(10)?;
    mp.set_target_search_concurrency(1)?;
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_reader_pooling(true);
    iwc.set_info_stream(Arc::clone(&info_stream))?;
    iwc.set_merged_segment_warmer(Some(Arc::new(SimpleMergedSegmentWarmer::new(info_stream))))?;
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "foo",
        "bar",
        Store::NO,
    )));
    for _ in 0..20 {
        w.add_document(&doc)?;
    }
    w.wait_for_merges()?;
    w.close()?;
    drop(w);
    dir.close()?;
    Assert::assert_true(did_warm.load(Ordering::SeqCst));
    Ok(())
}

/// Port of `TestIndexWriterReader.testReopenAfterNoRealChange`.
fn test_reopen_after_no_real_change() -> Result<()> {
    let mut d = get_assert_no_deletes_directory(LuceneTestCase::new_directory());
    let w = IndexWriter::new(d.as_directory(), no_wait_iwc()?)?;

    let r = directory_reader::open_with_writer(&*w)?; // start pooling readers

    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_null(&r2.map(|_| ()));

    w.add_document(&Document::new())?;
    let r3 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_not_null(&r3);
    let r3 = r3.expect("checked above");
    Assert::assert_true(r3.version()? != r.version()?);
    Assert::assert_true(r3.is_current()?);

    // Deletes nothing in reality...:
    w.delete_documents_by_terms(vec![term("foo", "bar")])?;

    // ... but IW marks this as not current:
    Assert::assert_false(r3.is_current()?);
    let r4 = directory_reader::open_if_changed(Arc::clone(&r3))?;
    assert_null(&r4.map(|_| ()));

    // Deletes nothing in reality...:
    w.delete_documents_by_terms(vec![term("foo", "bar")])?;
    let r5 = directory_reader::open_if_changed_with_writer(Arc::clone(&r3), &*w, true)?;
    assert_null(&r5.map(|_| ()));

    r3.close()?;

    w.close()?;
    drop(w);
    d.close()
}

/// The anonymous `Failure` of `testNRTOpenExceptions`.
struct NrtOpenFailure {
    do_fail: AtomicBool,
    should_fail: Arc<AtomicBool>,
}

impl Failure for NrtOpenFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.should_fail.load(Ordering::SeqCst)
            && LuceneTestCase::call_stack_contains_any_of(&["get_read_only_clone"])
        {
            if verbose() {
                println!("TEST: now fail; exc:");
                println!("{}", std::backtrace::Backtrace::force_capture());
            }
            self.should_fail.store(false, Ordering::SeqCst);
            return Err(fake_io_exception());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriterReader.testNRTOpenExceptions`.
fn test_nrt_open_exceptions() -> Result<()> {
    // LUCENE-5262: test that several failed attempts to obtain an NRT reader
    // don't leak file handles.
    let mut dir_holder =
        get_assert_no_deletes_directory(TestDirectory::Mock(LuceneTestCase::new_mock_directory()));
    let dir = dir_holder.as_mock().expect("newMockDirectory");
    let should_fail = Arc::new(AtomicBool::new(false));
    dir.fail_on(Arc::new(NrtOpenFailure {
        do_fail: AtomicBool::new(false),
        should_fail: Arc::clone(&should_fail),
    }));

    let conf = no_wait_iwc()?;
    conf.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?; // prevent merges from getting in the way
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // create a segment and open an NRT reader
    writer.add_document(&Document::new())?;
    directory_reader::open_with_writer(&*writer)?.close()?;

    // add a new document so a new NRT reader is required
    writer.add_document(&Document::new())?;

    // try to obtain an NRT reader twice: first time it fails and closes all the
    // other NRT readers. second time it fails, but also fails to close the
    // other NRT reader, since it is already marked closed!
    for _ in 0..2 {
        should_fail.store(true, Ordering::SeqCst);
        expect_throws(FAKE_IO_EXCEPTION, || {
            directory_reader::open_with_writer(&*writer)?.close()
        });
    }

    writer.close()?;
    drop(writer);
    dir_holder.close()
}

/// Make sure if all we do is open NRT reader against writer, we don't see merge starvation.
///
/// Port of `TestIndexWriterReader.testTooManySegments`.
fn test_too_many_segments() -> Result<()> {
    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    // getAssertNoDeletesDirectory: a ByteBuffersDirectory is no MockDirectoryWrapper.
    // Don't use newIndexWriterConfig, because we need a
    // "sane" mergePolicy:
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_max_full_flush_merge_wait_millis(0);
    let w = IndexWriter::new(Arc::clone(&dir), iwc)?;
    // Create 500 segments:
    for i in 0..500 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::NO,
        )));
        w.add_document(&doc)?;
        let r = directory_reader::open_with_writer(&*w)?;
        // Make sure segment count never exceeds 100:
        Assert::assert_true(ir(&r).leaves()?.len() < 100);
        r.close()?;
    }
    w.close()?;
    drop(w);
    Arc::get_mut(&mut dir)
        .unwrap_or_else(|| Throwable::new("java.lang.IllegalStateException", "dir shared").throw())
        .close()
}

/// LUCENE-5912: make sure when you reopen an NRT reader using a commit point, the SegmentReaders
/// are in fact shared:
///
/// Port of `TestIndexWriterReader.testReopenNRTReaderOnCommit`.
fn test_reopen_nrt_reader_on_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;

    // Pull NRT reader; it has 1 segment:
    let r1 = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &ir(&r1).leaves()?.len());
    w.add_document(&Document::new())?;
    w.commit()?;

    let commits = directory_reader::list_commits(dir.as_directory())?;
    assert_equals(&1, &commits.len());
    let r2 = directory_reader::open_if_changed_with_commit(Arc::clone(&r1), commits[0].as_ref())?
        .unwrap_or_else(|| crate::support::npe("r2"));
    assert_equals(&2, &ir(&r2).leaves()?.len());

    // Make sure we shared same instance of SegmentReader w/ first reader:
    Assert::assert_true(same(
        &ir(&r1).leaves()?[0].leaf_reader(),
        &ir(&r2).leaves()?[0].leaf_reader(),
    ));
    r1.close()?;
    r2.close()?;
    w.close()?;
    drop(w);
    dir.close()
}

/// The `Comparator<LeafReader>` of `testIndexReaderWriterWithLeafSorter`:
/// `Comparator.comparingLong(min or max point value)`, `reversed()` for a
/// descending sort.
#[derive(Debug)]
struct PointLeafSorter {
    field_name: &'static str,
    asc_sort: bool,
    missing_value: i64,
}

impl std::fmt::Display for PointLeafSorter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("java.util.Comparator$$Lambda")
    }
}

impl PointLeafSorter {
    /// The key lambda.
    fn key(&self, r: &dyn LeafReader) -> i64 {
        let value = (|| -> Result<Option<i64>> {
            if let Some(points) = r.get_point_values(self.field_name)? {
                let sort_value = if self.asc_sort {
                    points.min_packed_value()?
                } else {
                    points.max_packed_value()?
                };
                return Ok(sort_value.map(|v| LongPoint::decode_dimension(&v, 0)));
            }
            Ok(None)
        })();
        match value {
            Ok(Some(v)) => v,
            // IOException e: ignored
            Ok(None) | Err(_) => self.missing_value,
        }
    }
}

impl LeafComparator for PointLeafSorter {
    fn compare(&self, a: &dyn LeafReader, b: &dyn LeafReader) -> CmpOrdering {
        let ord = self.key(a).cmp(&self.key(b));
        if self.asc_sort {
            ord
        } else {
            ord.reverse()
        }
    }
}

/// Port of `TestIndexWriterReader.testIndexReaderWriterWithLeafSorter`.
fn test_index_reader_writer_with_leaf_sorter() -> Result<()> {
    const FIELD_NAME: &str = "field1";
    let asc_sort = RandomizedTest::random_boolean();
    let missing_value = if asc_sort { i64::MAX } else { i64::MIN }; // missing values at the end

    // create a comparator that sort leaf readers according to
    // the min value (asc sort) or max value (desc sort) of its points
    let leaf_sorter: Arc<dyn LeafComparator> = Arc::new(PointLeafSorter {
        field_name: FIELD_NAME,
        asc_sort,
        missing_value,
    });

    let num_docs = LuceneTestCase::at_least(30);
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::new();
    iwc.set_leaf_sorter(Some(Arc::clone(&leaf_sorter)));
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LongPoint::new(
            FIELD_NAME,
            &[RandomizedTest::random_long_between(1, 99)],
        )?));
        writer.add_document(&doc)?;
        if i > 0 && i % 10 == 0 {
            writer.flush()?;
        }
    }

    let first_value: i64 = if asc_sort { 0 } else { 100 };
    // add more documents that should be sorted first
    let add_first = || -> Result<()> {
        for _ in 0..10 {
            let mut doc = Document::new();
            doc.add(Box::new(LongPoint::new(FIELD_NAME, &[first_value])?));
            writer.add_document(&doc)?;
        }
        writer.commit()?;
        Ok(())
    };
    // and open again
    let reopen_and_check = |reader: &Arc<dyn DirectoryReader>| -> Result<()> {
        let reader2 = directory_reader::open_if_changed(Arc::clone(reader))?
            .unwrap_or_else(|| crate::support::npe("reader2"));
        let checked = assert_leaves_sorted(&reader2, &*leaf_sorter);
        reader2.close()?;
        checked
    };

    // Test1: test that leafReaders are sorted according to leafSorter provided in IndexWriterConfig
    {
        let reader = directory_reader::open_with_writer(&*writer)?;
        assert_leaves_sorted(&reader, &*leaf_sorter)?;
        add_first()?;
        reopen_and_check(&reader)?;
        reader.close()?;
    }

    // Test2: test that leafReaders are sorted according to the provided leafSorter when opened from
    // directory
    {
        let reader = directory_reader::open_with_leaf_sorter(
            dir.as_directory(),
            Some(Arc::clone(&leaf_sorter)),
        )?;
        assert_leaves_sorted(&reader, &*leaf_sorter)?;
        add_first()?;
        reopen_and_check(&reader)?;
        reader.close()?;
    }

    // Test3: test that FilterDirectoryReader sorts leaves according
    // to leafSorter of its wrapped reader
    {
        let reader: Arc<dyn DirectoryReader> = Arc::new(AssertingDirectoryReader::new(
            directory_reader::open_with_leaf_sorter(
                dir.as_directory(),
                Some(Arc::clone(&leaf_sorter)),
            )?,
        )?);
        assert_leaves_sorted(&reader, &*leaf_sorter)?;
        add_first()?;
        reopen_and_check(&reader)?;
        reader.close()?;
    }

    // Test4: test that leafReaders are sorted according to the provided leafSorter when opened from
    // commit
    {
        let commits = directory_reader::list_commits(dir.as_directory())?;
        let latest_commit = &commits[commits.len() - 1];
        let reader = directory_reader::open_with_commit_min_version(
            latest_commit.as_ref(),
            Version::MIN_SUPPORTED_MAJOR,
            Some(Arc::clone(&leaf_sorter)),
        )?;
        assert_leaves_sorted(&reader, &*leaf_sorter)?;
        add_first()?;
        reopen_and_check(&reader)?;
        reader.close()?;
    }

    writer.close()?;
    drop(writer);
    dir.close()
}

/// assert that the leaf readers of the provided directory reader are sorted according to the
/// provided leafSorter
///
/// The private static `assertLeavesSorted(DirectoryReader, Comparator<LeafReader>)`.
fn assert_leaves_sorted(
    reader: &Arc<dyn DirectoryReader>,
    leaf_sorter: &dyn LeafComparator,
) -> Result<()> {
    let lrs: Vec<Arc<dyn LeafReader>> = ir(reader)
        .leaves()?
        .iter()
        .map(|c| c.leaf_reader())
        .collect();
    let mut expected_sortedlrs = lrs.clone();
    expected_sortedlrs.sort_by(|a, b| leaf_sorter.compare(&**a, &**b));
    let addr = |v: &[Arc<dyn LeafReader>]| -> Vec<usize> {
        v.iter()
            .map(|r| Arc::as_ptr(r) as *const () as usize)
            .collect()
    };
    assert_equals(&addr(&expected_sortedlrs), &addr(&lrs));
    Ok(())
}
