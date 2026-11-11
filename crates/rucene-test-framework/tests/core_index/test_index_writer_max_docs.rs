//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterMaxDocs.java`.
//!
//! * `setIndexWriterMaxDocs(n); try { … } finally { restoreIndexWriterMaxDocs(); }`
//!   is `MaxDocsGuard`, whose drop restores the limit on every exit.

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::index_writer::{self, IndexWriter};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::multi_reader::MultiReader;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::TermQuery;
use rucene::store::{Directory, FilterDirectory, NoLockFactory};
use rucene_test_framework::util::java_exceptions::{
    expect_throws, is_instance, java_throwable, rethrow_java,
};
use rucene_test_framework::util::lucene_test_case::{Monster, Nightly, SuppressCodecs};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class_annotated, random, test};
use crate::support::{CountDownLatch, SharedDirectory};

/// `TestIndexWriterMaxDocs`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterMaxDocs",
        vec![
            // The two hour time was achieved on a Linux 3.13 system with these specs:
            // 3-core AMD at 2.5Ghz, 12 GB RAM, 5GB test heap, 2 test JVMs, 2TB SATA.
            test("testExactlyAtTrueLimit", test_exactly_at_true_limit).annotate(Monster {
                value: "takes over two hours".to_string(),
            }),
            test("testAddDocument", test_add_document),
            test("testAddDocuments", test_add_documents),
            test("testUpdateDocument", test_update_document),
            test("testUpdateDocuments", test_update_documents),
            test("testReclaimedDeletes", test_reclaimed_deletes),
            test(
                "testReclaimedDeletesWholeSegments",
                test_reclaimed_deletes_whole_segments,
            ),
            test("testAddIndexes", test_add_indexes),
            test("testMultiReaderExactLimit", test_multi_reader_exact_limit),
            test("testMultiReaderBeyondLimit", test_multi_reader_beyond_limit),
            test("testAddTooManyIndexesDir", test_add_too_many_indexes_dir).annotate(Nightly),
            test(
                "testAddTooManyIndexesCodecReader",
                test_add_too_many_indexes_codec_reader,
            ),
            test("testTooLargeMaxDocs", test_too_large_max_docs),
            test("testDeleteAll", test_delete_all),
            test("testDeleteAllAfterFlush", test_delete_all_after_flush),
            test("testDeleteAllAfterCommit", test_delete_all_after_commit),
            test(
                "testDeleteAllMultipleThreads",
                test_delete_all_multiple_threads,
            ),
            test("testDeleteAllAfterClose", test_delete_all_after_close),
            test("testAcrossTwoIndexWriters", test_across_two_index_writers),
            test(
                "testCorruptIndexExceptionTooLarge",
                test_corrupt_index_exception_too_large,
            ),
            test(
                "testCorruptIndexExceptionTooLargeWriter",
                test_corrupt_index_exception_too_large_writer,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
            .annotate(TimeoutSuite {
                millis: 24 * 3_600_000,
            })
        },
    )
}

const IAE: &str = "java.lang.IllegalArgumentException";

/// `new IndexWriterConfig(null)`.
fn iwc_null() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(None)
}

/// `setIndexWriterMaxDocs(limit)`, restored (`restoreIndexWriterMaxDocs()`)
/// when dropped, as Java's `finally` block does.
struct MaxDocsGuard;

impl MaxDocsGuard {
    fn set(limit: i32) -> Self {
        LuceneTestCase::set_index_writer_max_docs(limit);
        MaxDocsGuard
    }
}

impl Drop for MaxDocsGuard {
    fn drop(&mut self) {
        LuceneTestCase::restore_index_writer_max_docs();
    }
}

/// Port of `TestIndexWriterMaxDocs.testExactlyAtTrueLimit`.
fn test_exactly_at_true_limit() -> Result<()> {
    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BDocs3"));
    let iw = IndexWriter::new(dir.as_directory(), iwc_null())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "text",
        Store::NO,
    )));
    for _ in 0..MAX_DOCS {
        iw.add_document(&doc)?;
        /*
        if (i%1000000 == 0) {
          System.out.println((i/1000000) + " M docs...");
        }
        */
    }
    iw.commit()?;

    // First unoptimized, then optimized:
    for _ in 0..2 {
        let ir = directory_reader::open(dir.as_directory())?;
        assert_equals(&MAX_DOCS, &ir.max_doc());
        assert_equals(&MAX_DOCS, &ir.num_docs());
        let searcher = IndexSearcher::new(Arc::clone(&ir) as Arc<dyn IndexReader>)?;
        let collector_manager = TopScoreDocCollectorManager::new(10, None, i32::MAX)?;
        let hits = searcher.search_with_manager(
            Arc::new(TermQuery::new(Term::from_text("field", "text"))),
            &collector_manager,
        )?;
        assert_equals(&(MAX_DOCS as i64), &hits.total_hits.value());

        // Sort by docID reversed:
        let hits = searcher.search_with_sort(
            Arc::new(TermQuery::new(Term::from_text("field", "text"))),
            10,
            &Sort::new_fields(vec![SortField::new_reverse(
                None,
                SortFieldType::Doc,
                true,
            )?])?,
        )?;
        assert_equals(&(MAX_DOCS as i64), &hits.total_hits.value());
        assert_equals(&10, &hits.score_docs.len());
        assert_equals(&(MAX_DOCS - 1), &hits.score_docs[0].doc());
        ir.close()?;

        iw.force_merge(1)?;
    }

    iw.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMaxDocs.testAddDocument`.
fn test_add_document() -> Result<()> {
    let _limit = MaxDocsGuard::set(10);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..10 {
        w.add_document(&Document::new())?;
    }

    // 11th document should fail:
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMaxDocs.testAddDocuments`.
fn test_add_documents() -> Result<()> {
    let _limit = MaxDocsGuard::set(10);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..10 {
        w.add_document(&Document::new())?;
    }

    // 11th document should fail:
    expect_throws(IAE, || w.add_documents(&[Document::new()]));

    w.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMaxDocs.testUpdateDocument`.
fn test_update_document() -> Result<()> {
    let _limit = MaxDocsGuard::set(10);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..10 {
        w.add_document(&Document::new())?;
    }

    // 11th document should fail:
    expect_throws(IAE, || {
        w.update_document(Term::from_text("field", "foo"), &Document::new())
    });

    w.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMaxDocs.testUpdateDocuments`.
fn test_update_documents() -> Result<()> {
    let _limit = MaxDocsGuard::set(10);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..10 {
        w.add_document(&Document::new())?;
    }

    // 11th document should fail:
    expect_throws(IAE, || {
        w.update_documents(Term::from_text("field", "foo"), &[Document::new()])
    });

    w.close()?;
    dir.close()
}

/// `doc.add(newStringField("id", "" + i, Field.Store.NO))` on a new document.
fn id_doc(i: i32) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        &i.to_string(),
        Store::NO,
    )));
    doc
}

/// Port of `TestIndexWriterMaxDocs.testReclaimedDeletes`.
fn test_reclaimed_deletes() -> Result<()> {
    let _limit = MaxDocsGuard::set(10);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for i in 0..10 {
        w.add_document(&id_doc(i))?;
    }

    // Delete 5 of them:
    for i in 0..5 {
        w.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }

    w.force_merge(1)?;

    assert_equals(&5, &w.get_doc_stats()?.max_doc);

    // Add 5 more docs
    for _ in 0..5 {
        w.add_document(&Document::new())?;
    }

    // 11th document should fail:
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.close()?;
    dir.close()
}

/// Tests that 100% deleted segments (which IW "specializes" by dropping entirely) are not
/// mis-counted
/// Port of `TestIndexWriterMaxDocs.testReclaimedDeletesWholeSegments`.
fn test_reclaimed_deletes_whole_segments() -> Result<()> {
    let _limit = MaxDocsGuard::set(10);
    let mut dir = LuceneTestCase::new_directory();
    let iwc = iwc_null();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for i in 0..10 {
        w.add_document(&id_doc(i))?;
        if i % 2 == 0 {
            // Make a new segment every 2 docs:
            w.commit()?;
        }
    }

    // Delete 5 of them:
    for i in 0..5 {
        w.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }

    w.force_merge(1)?;

    assert_equals(&5, &w.get_doc_stats()?.max_doc);

    // Add 5 more docs
    for _ in 0..5 {
        w.add_document(&Document::new())?;
    }

    // 11th document should fail:
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.close()?;
    dir.close()
}

/// Port of `TestIndexWriterMaxDocs.testAddIndexes`.
fn test_add_indexes() -> Result<()> {
    let _limit = MaxDocsGuard::set(10);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..10 {
        w.add_document(&Document::new())?;
    }
    w.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), iwc_null())?;
    w2.add_document(&Document::new())?;
    expect_throws(IAE, || w2.add_indexes_dirs(&[dir.as_directory()]));

    assert_equals(&1, &w2.get_doc_stats()?.max_doc);
    let ir = directory_reader::open(dir.as_directory())?;
    expect_throws(IAE, || {
        TestUtil::add_indexes_slowly(&w2, &[Arc::clone(&ir)])
    });

    w2.close()?;
    ir.close()?;
    dir.close()?;
    dir2.close()
}

/// `IndexReader[] subReaders = new IndexReader[copies + 1]; Arrays.fill(subReaders, ir);
/// subReaders[subReaders.length - 1] = ir2;`
fn sub_readers(
    copies: i32,
    ir: &Arc<dyn DirectoryReader>,
    ir2: &Arc<dyn DirectoryReader>,
) -> Vec<Arc<dyn IndexReader>> {
    let mut sub_readers: Vec<Arc<dyn IndexReader>> =
        vec![Arc::clone(ir) as Arc<dyn IndexReader>; (copies + 1) as usize];
    let last = sub_readers.len() - 1;
    sub_readers[last] = Arc::clone(ir2) as Arc<dyn IndexReader>;
    sub_readers
}

/// Make sure MultiReader lets you search exactly the limit number of docs:
/// Port of `TestIndexWriterMaxDocs.testMultiReaderExactLimit`.
fn test_multi_reader_exact_limit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let doc = Document::new();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..100000 {
        w.add_document(&doc)?;
    }
    w.close()?;

    let remainder = MAX_DOCS % 100000;
    let mut dir2 = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir2.as_directory(), iwc_null())?;
    for _ in 0..remainder {
        w.add_document(&doc)?;
    }
    w.close()?;

    let copies = MAX_DOCS / 100000;

    let ir = directory_reader::open(dir.as_directory())?;
    let ir2 = directory_reader::open(dir2.as_directory())?;
    let sub_readers = sub_readers(copies, &ir, &ir2);

    let mr = MultiReader::new(sub_readers, false)?;
    assert_equals(&MAX_DOCS, &mr.max_doc());
    assert_equals(&MAX_DOCS, &mr.num_docs());
    ir.close()?;
    ir2.close()?;
    dir.close()?;
    dir2.close()
}

/// Make sure MultiReader is upset if you exceed the limit
/// Port of `TestIndexWriterMaxDocs.testMultiReaderBeyondLimit`.
fn test_multi_reader_beyond_limit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let doc = Document::new();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..100000 {
        w.add_document(&doc)?;
    }
    w.close()?;

    let mut remainder = MAX_DOCS % 100000;

    // One too many:
    remainder += 1;

    let mut dir2 = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir2.as_directory(), iwc_null())?;
    for _ in 0..remainder {
        w.add_document(&doc)?;
    }
    w.close()?;

    let copies = MAX_DOCS / 100000;

    let ir = directory_reader::open(dir.as_directory())?;
    let ir2 = directory_reader::open(dir2.as_directory())?;
    let sub_readers = sub_readers(copies, &ir, &ir2);

    expect_throws(IAE, || MultiReader::new(sub_readers, false));

    ir.close()?;
    ir2.close()?;
    dir.close()?;
    dir2.close()
}

/// The `catch` clauses of `testAddTooManyIndexes*` around `w.addIndexes(…)`.
fn check_add_too_many_indexes(result: Result<i64>) -> Result<()> {
    match result {
        Ok(_) => Assert::fail("didn't get expected exception"),
        Err(expected) if is_instance(&expected, IAE) => {
            // pass
            Ok(())
        }
        Err(fake_disk_full) if fake_disk_full.is_io_exception() => {
            let message = java_throwable(&fake_disk_full)
                .message()
                .map(str::to_string);
            let e = if message.is_some_and(|m| m.starts_with("fake disk full")) {
                let mut e = LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    "test failed: IW checks aren't working and we are executing addIndexes",
                );
                e.add_suppressed(fake_disk_full);
                e
            } else {
                fake_disk_full
            };
            Err(e)
        }
        Err(e) => Err(e),
    }
}

/// LUCENE-6299: Test if addindexes(Dir[]) prevents exceeding max docs.
/// Port of `TestIndexWriterMaxDocs.testAddTooManyIndexesDir`.
// TODO: can we use the setter to lower the amount of docs to be written here?
fn test_add_too_many_indexes_dir() -> Result<()> {
    // we cheat and add the same one over again... IW wants a write lock on each
    let mut dir = LuceneTestCase::new_directory_lock_factory(&*random(), Arc::new(NoLockFactory));
    let doc = Document::new();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..100000 {
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.commit()?;
    w.close()?;

    // wrap this with disk full, so test fails faster and doesn't fill up real disks.
    let mut dir2 = LuceneTestCase::new_mock_directory();
    let w = IndexWriter::new(dir2.as_directory(), iwc_null())?;
    w.commit()?; // don't confuse checkindex
    dir2.set_max_size_in_bytes(dir2.size_in_bytes()? + 65536); // 64KB
    let mut dirs: Vec<Arc<dyn Directory>> = Vec::new();
    for _ in 0..(1 + (MAX_DOCS / 100000)) {
        // bypass iw check for duplicate dirs
        dirs.push(Arc::new(FilterDirectory::new(Box::new(
            SharedDirectory::new(dir.as_directory()),
        ))));
    }

    check_add_too_many_indexes(w.add_indexes_dirs(&dirs))?;

    w.close()?;
    drop(dirs);
    dir.close()?;
    dir2.close()
}

/// LUCENE-6299: Test if addindexes(CodecReader[]) prevents exceeding max docs.
/// Port of `TestIndexWriterMaxDocs.testAddTooManyIndexesCodecReader`.
fn test_add_too_many_indexes_codec_reader() -> Result<()> {
    // we cheat and add the same one over again... IW wants a write lock on each
    let mut dir = LuceneTestCase::new_directory_lock_factory(&*random(), Arc::new(NoLockFactory));
    let doc = Document::new();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    for _ in 0..100000 {
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.commit()?;
    w.close()?;

    // wrap this with disk full, so test fails faster and doesn't fill up real disks.
    let mut dir2 = LuceneTestCase::new_mock_directory();
    let w = IndexWriter::new(dir2.as_directory(), iwc_null())?;
    w.commit()?; // don't confuse checkindex
    dir2.set_max_size_in_bytes(dir2.size_in_bytes()? + 65536); // 64KB
    let r = directory_reader::open(dir.as_directory())?;
    let seg_reader: Arc<dyn CodecReader> = Arc::clone(&r).leaves()?[0]
        .leaf_reader()
        .as_codec_reader()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a CodecReader",
            )
            .throw()
        });

    let readers: Vec<Arc<dyn CodecReader>> = vec![seg_reader; (1 + (MAX_DOCS / 100000)) as usize];

    check_add_too_many_indexes(w.add_indexes_codec_readers(&readers))?;

    drop(readers);
    r.close()?;
    w.close()?;
    dir.close()?;
    dir2.close()
}

/// Port of `TestIndexWriterMaxDocs.testTooLargeMaxDocs`.
fn test_too_large_max_docs() -> Result<()> {
    expect_throws(IAE, || index_writer::set_max_docs(i32::MAX));
    Ok(())
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testDeleteAll`.
fn test_delete_all() -> Result<()> {
    let _limit = MaxDocsGuard::set(1);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w.add_document(&Document::new())?;
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.delete_all()?;
    w.add_document(&Document::new())?;
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.close()?;
    dir.close()
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testDeleteAllAfterFlush`.
fn test_delete_all_after_flush() -> Result<()> {
    let _limit = MaxDocsGuard::set(2);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w.add_document(&Document::new())?;
    directory_reader::open_with_writer(&*w)?.close()?;
    w.add_document(&Document::new())?;
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.delete_all()?;
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.close()?;
    dir.close()
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testDeleteAllAfterCommit`.
fn test_delete_all_after_commit() -> Result<()> {
    let _limit = MaxDocsGuard::set(2);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w.add_document(&Document::new())?;
    w.commit()?;
    w.add_document(&Document::new())?;
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.delete_all()?;
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.close()?;
    dir.close()
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testDeleteAllMultipleThreads`.
fn test_delete_all_multiple_threads() -> Result<()> {
    let limit = TestUtil::next_int(&*random(), 2, 10);
    let _limit = MaxDocsGuard::set(limit);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;

    let starting_gun = Arc::new(CountDownLatch::new(1));
    let mut threads = Vec::with_capacity(limit as usize);
    for _ in 0..limit {
        let starting_gun = Arc::clone(&starting_gun);
        let w = Arc::clone(&w);
        let thread = Thread::unnamed(move || {
            starting_gun.await_();
            if let Err(e) = w.add_document(&Document::new()) {
                rethrow_java(LuceneError::runtime_exception(e));
            }
        });
        thread.start();
        threads.push(thread);
    }

    starting_gun.count_down();

    for thread in &threads {
        let _ = thread.join();
    }

    expect_throws(IAE, || w.add_document(&Document::new()));

    w.delete_all()?;
    for _ in 0..limit {
        w.add_document(&Document::new())?;
    }
    expect_throws(IAE, || w.add_document(&Document::new()));

    w.close()?;
    dir.close()
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testDeleteAllAfterClose`.
fn test_delete_all_after_close() -> Result<()> {
    let _limit = MaxDocsGuard::set(2);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w.add_document(&Document::new())?;
    w.close()?;

    let w2 = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w2.add_document(&Document::new())?;
    expect_throws(IAE, || w2.add_document(&Document::new()));

    w2.delete_all()?;
    w2.add_document(&Document::new())?;
    w2.add_document(&Document::new())?;
    expect_throws(IAE, || w2.add_document(&Document::new()));

    w2.close()?;
    dir.close()
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testAcrossTwoIndexWriters`.
fn test_across_two_index_writers() -> Result<()> {
    let _limit = MaxDocsGuard::set(1);
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w.add_document(&Document::new())?;
    w.close()?;
    let w2 = IndexWriter::new(dir.as_directory(), iwc_null())?;
    expect_throws(IAE, || w2.add_document(&Document::new()));

    w2.close()?;
    dir.close()
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testCorruptIndexExceptionTooLarge`.
fn test_corrupt_index_exception_too_large() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    w.close()?;

    {
        let _limit = MaxDocsGuard::set(1);
        expect_throws("org.apache.lucene.index.CorruptIndexException", || {
            directory_reader::open(dir.as_directory())
        });
    }

    dir.close()
}

/// LUCENE-6299
/// Port of `TestIndexWriterMaxDocs.testCorruptIndexExceptionTooLargeWriter`.
fn test_corrupt_index_exception_too_large_writer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), iwc_null())?;
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    w.close()?;

    {
        let _limit = MaxDocsGuard::set(1);
        expect_throws("org.apache.lucene.index.CorruptIndexException", || {
            IndexWriter::new(dir.as_directory(), iwc_null())
        });
    }

    dir.close()
}
