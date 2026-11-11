//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSoftDeletesRetentionMergePolicy.java`.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, Field, IntPoint, LongPoint, NumericDocValuesField, NumericValue, Store, StringField,
};
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::MergedSegmentWarmer;
use rucene::index::merge_policy::{
    LogMergePolicy, MergePolicy, NoMergePolicy, RetentionQuerySupplier,
    SoftDeletesRetentionMergePolicy,
};
use rucene::index::terms::Term;
use rucene::index::{DirectoryReader, IndexReader, LeafReader};
use rucene::search::field_exists_query::FieldExistsQuery;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::searcher_factory::DefaultSearcherFactory;
use rucene::search::searcher_manager::SearcherManager;
use rucene::search::term_query::TermQuery;
use rucene::search::{IndexSearcher, Query};
use rucene::util::{Bits, IOUtils};
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_not_null, assert_null, class, random, test, JavaHashSet,
};
use crate::support::CountDownLatch;

/// `TestSoftDeletesRetentionMergePolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSoftDeletesRetentionMergePolicy",
        vec![
            test("testForceMergeFullyDeleted", test_force_merge_fully_deleted),
            test(
                "testKeepFullyDeletedSegments",
                test_keep_fully_deleted_segments,
            ),
            test("testFieldBasedRetention", test_field_based_retention),
            test(
                "testKeepAllDocsAcrossMerges",
                test_keep_all_docs_across_merges,
            ),
            test(
                "testSoftDeleteWithRetention",
                test_soft_delete_with_retention,
            ),
            test("testForceMergeDeletes", test_force_merge_deletes),
            test(
                "testDropFullySoftDeletedSegment",
                test_drop_fully_soft_deleted_segment,
            ),
            test(
                "testSoftDeleteWhileMergeSurvives",
                test_soft_delete_while_merge_survives,
            ),
            test(
                "testDeleteDocWhileMergeThatIsSoftDeleted",
                test_delete_doc_while_merge_that_is_soft_deleted,
            ),
            test("testUndeleteDocument", test_undelete_document),
            test(
                "testMergeSoftDeleteAndHardDelete",
                test_merge_soft_delete_and_hard_delete,
            ),
            test(
                "testSoftDeleteWithTryUpdateDocValue",
                test_soft_delete_with_try_update_doc_value,
            ),
            test(
                "testMixedSoftDeletesAndHardDeletes",
                test_mixed_soft_deletes_and_hard_deletes,
            ),
            test("testRewriteRetentionQuery", test_rewrite_retention_query),
        ],
    )
}

/// `new NumericDocValuesField(name, value)`, as the `Field` the update APIs
/// take.
fn ndv(name: &str, value: i64) -> Result<Field> {
    Field::new_with_number(
        name,
        NumericValue::Long(value),
        NumericDocValuesField::type_().clone(),
    )
}

/// `new NumericDocValuesField(name, (Long) null)`.
fn ndv_null(name: &str) -> Result<Field> {
    Field::new_with_type(name, NumericDocValuesField::type_().clone())
}

/// `new StringField(name, value, Field.Store.YES)`.
fn stored_string(name: &str, value: &str) -> Result<Box<StringField>> {
    Ok(Box::new(StringField::new(
        name,
        value.to_string(),
        Store::YES,
    )?))
}

/// A retention query supplier answering `query()` on each call.
fn supplier(query: impl Fn() -> Arc<dyn Query> + Send + Sync + 'static) -> RetentionQuerySupplier {
    Arc::new(query)
}

/// `MatchAllDocsQuery::new` / `() -> MatchAllDocsQuery.INSTANCE`.
fn match_all() -> RetentionQuerySupplier {
    supplier(|| Arc::new(MatchAllDocsQuery))
}

/// `() -> MatchNoDocsQuery.INSTANCE`.
fn match_none() -> RetentionQuerySupplier {
    supplier(|| Arc::new(MatchNoDocsQuery::instance()))
}

/// `(SegmentReader) leaf` as the `CodecReader` a reader supplier hands out.
fn segment_codec_reader(leaf: Arc<dyn LeafReader>) -> Arc<dyn CodecReader> {
    if leaf.as_segment_reader().is_none() {
        Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
    }
    leaf.as_codec_reader()
        .expect("a SegmentReader is a CodecReader")
}

/// `policy.keepFullyDeletedSegment(() -> reader)`.
fn keep_fully_deleted(policy: &dyn MergePolicy, reader: &Arc<dyn CodecReader>) -> Result<bool> {
    policy.keep_fully_deleted_segment(&mut || Ok(Arc::clone(reader)))
}

/// `policy.numDeletesToMerge(segmentReader.getSegmentInfo(), 0, () -> segmentReader)`.
fn num_deletes_to_merge(policy: &dyn MergePolicy, leaf: &Arc<dyn LeafReader>) -> Result<i32> {
    let reader = segment_codec_reader(Arc::clone(leaf));
    let info = leaf
        .as_segment_reader()
        .expect("checked")
        .get_segment_info()
        .clone();
    policy.num_deletes_to_merge_with_reader(&info, 0, &mut || Ok(Arc::clone(&reader)))
}

/// The `i`-th leaf of `reader`.
fn leaf(reader: &Arc<dyn DirectoryReader>, i: usize) -> Result<Arc<dyn LeafReader>> {
    Ok(Arc::clone(reader).leaves()?[i].leaf_reader())
}

/// `LogDocMergePolicy`.
fn log_doc() -> Arc<dyn MergePolicy> {
    Arc::new(LogMergePolicy::by_doc_count())
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testForceMergeFullyDeleted`.
fn test_force_merge_fully_deleted() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let let_it_go = Arc::new(AtomicBool::new(false));
    let policy: Arc<dyn MergePolicy> = {
        let let_it_go = Arc::clone(&let_it_go);
        Arc::new(SoftDeletesRetentionMergePolicy::new(
            "soft_delete",
            supplier(move || {
                if let_it_go.load(Ordering::SeqCst) {
                    Arc::new(MatchNoDocsQuery::instance())
                } else {
                    Arc::new(MatchAllDocsQuery)
                }
            }),
            log_doc(),
        ))
    };
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_merge_policy(Some(Arc::clone(&policy)))?;
    index_writer_config.set_soft_deletes_field(Some("soft_delete".to_string()));
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;

    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    doc.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
    writer.add_document(&doc)?;
    writer.commit()?;
    let mut doc = Document::new();
    doc.add(stored_string("id", "2")?);
    doc.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
    writer.add_document(&doc)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    {
        assert_equals(&2, &Arc::clone(&reader).leaves()?.len());
        let segment_reader = leaf(&reader, 0)?;
        Assert::assert_true(keep_fully_deleted(
            &*policy,
            &segment_codec_reader(Arc::clone(&segment_reader)),
        )?);
        assert_equals(&0, &num_deletes_to_merge(&*policy, &segment_reader)?);
    }
    {
        let segment_reader = leaf(&reader, 1)?;
        Assert::assert_true(keep_fully_deleted(
            &*policy,
            &segment_codec_reader(Arc::clone(&segment_reader)),
        )?);
        assert_equals(&0, &num_deletes_to_merge(&*policy, &segment_reader)?);
        writer.force_merge(1)?;
        reader.close()?;
    }
    let reader = directory_reader::open_with_writer(&*writer)?;
    {
        assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
        let segment_reader = leaf(&reader, 0)?;
        assert_equals(&2, &reader.max_doc());
        Assert::assert_true(keep_fully_deleted(
            &*policy,
            &segment_codec_reader(Arc::clone(&segment_reader)),
        )?);
        assert_equals(&0, &num_deletes_to_merge(&*policy, &segment_reader)?);
    }
    writer.force_merge(1)?; // make sure we don't merge this
    assert_null(&directory_reader::open_if_changed(Arc::clone(&reader))?.map(|_| "reader"));

    writer.force_merge_deletes()?; // make sure we don't merge this
    assert_null(&directory_reader::open_if_changed(Arc::clone(&reader))?.map(|_| "reader"));
    let_it_go.store(true, Ordering::SeqCst);
    writer.force_merge_deletes()?; // make sure we don't merge this
    let directory_reader = directory_reader::open_if_changed(Arc::clone(&reader))?;
    assert_not_null(&directory_reader);
    let directory_reader = directory_reader.expect("checked by assertNotNull");
    assert_equals(&0, &directory_reader.num_deleted_docs());
    assert_equals(&0, &directory_reader.max_doc());
    IOUtils::close([
        IOUtils::closeable(|| directory_reader.close()),
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testKeepFullyDeletedSegments`.
fn test_keep_fully_deleted_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;

    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    doc.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
    writer.add_document(&doc)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
    let policy = SoftDeletesRetentionMergePolicy::new(
        "soft_delete",
        supplier(|| Arc::new(FieldExistsQuery::new("keep_around"))),
        Arc::new(NoMergePolicy::new()),
    );
    Assert::assert_false(keep_fully_deleted(
        &policy,
        &segment_codec_reader(leaf(&reader, 0)?),
    )?);
    reader.close()?;

    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    doc.add(Box::new(NumericDocValuesField::new("keep_around", 1)));
    doc.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
    writer.add_document(&doc)?;

    let reader1 = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&2, &Arc::clone(&reader1).leaves()?.len());
    Assert::assert_false(keep_fully_deleted(
        &policy,
        &segment_codec_reader(leaf(&reader1, 0)?),
    )?);

    Assert::assert_true(keep_fully_deleted(
        &policy,
        &segment_codec_reader(leaf(&reader1, 1)?),
    )?);

    IOUtils::close([
        IOUtils::closeable(|| reader1.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// `Instant.now().toEpochMilli()`.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A document with the stored strings `id` and `version` and the long point
/// `creation_date`.
fn dated_doc(version: &str, creation_date: i64) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    doc.add(stored_string("version", version)?);
    doc.add(Box::new(LongPoint::new("creation_date", &[creation_date])?));
    Ok(doc)
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testFieldBasedRetention`.
fn test_field_based_retention() -> Result<()> {
    const HOUR: i64 = 60 * 60 * 1000;
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    let now = now_millis();
    let time24_hours_ago = now - 24 * HOUR;
    let soft_deletes_field = "soft_delete";
    let docs_of_last24_hours = supplier(move || {
        LongPoint::new_range_query("creation_date", &[time24_hours_ago], &[now])
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))
    });
    index_writer_config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        soft_deletes_field,
        docs_of_last24_hours,
        log_doc(),
    ))))?;
    index_writer_config.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;

    let time28_hours_ago = now - 28 * HOUR;
    writer.add_document(&dated_doc("1", time28_hours_ago)?)?;

    writer.flush()?;
    let time26_hours_ago = now - 26 * HOUR;
    writer.soft_update_document(
        Term::from_text("id", "1"),
        &dated_doc("2", time26_hours_ago)?,
        &[ndv("soft_delete", 1)?],
    )?;

    if random().next_boolean() {
        writer.flush()?;
    }
    let time23_hours_ago = now - 23 * HOUR;
    writer.soft_update_document(
        Term::from_text("id", "1"),
        &dated_doc("3", time23_hours_ago)?,
        &[ndv("soft_delete", 1)?],
    )?;

    if random().next_boolean() {
        writer.flush()?;
    }
    let time12_hours_ago = now - 12 * HOUR;
    writer.soft_update_document(
        Term::from_text("id", "1"),
        &dated_doc("4", time12_hours_ago)?,
        &[ndv("soft_delete", 1)?],
    )?;

    if random().next_boolean() {
        writer.flush()?;
    }
    writer.soft_update_document(
        Term::from_text("id", "1"),
        &dated_doc("5", now)?,
        &[ndv("soft_delete", 1)?],
    )?;

    if random().next_boolean() {
        writer.flush()?;
    }
    writer.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&1, &reader.num_docs());
    assert_equals(&3, &reader.max_doc());
    let mut versions: JavaHashSet<String> = JavaHashSet::new();
    let fields = std::collections::HashSet::from(["version".to_string()]);
    for doc in 0..3 {
        versions.add(
            reader
                .stored_fields()?
                .document_fields(doc, &fields)?
                .get("version")
                .unwrap_or_else(|| crate::support::npe("version")),
        );
    }
    Assert::assert_true(versions.iter().any(|v| v == "5"));
    Assert::assert_true(versions.iter().any(|v| v == "4"));
    Assert::assert_true(versions.iter().any(|v| v == "3"));
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testKeepAllDocsAcrossMerges`.
fn test_keep_all_docs_across_merges() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "soft_delete",
        match_all(),
        log_doc(),
    ))))?;
    index_writer_config.set_soft_deletes_field(Some("soft_delete".to_string()));
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;

    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    writer.soft_update_document(Term::from_text("id", "1"), &doc, &[ndv("soft_delete", 1)?])?;

    writer.commit()?;
    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    writer.soft_update_document(Term::from_text("id", "1"), &doc, &[ndv("soft_delete", 1)?])?;

    writer.commit()?;
    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    doc.add(Box::new(NumericDocValuesField::new("soft_delete", 1))); // already deleted
    writer.soft_update_document(Term::from_text("id", "1"), &doc, &[ndv("soft_delete", 1)?])?;
    writer.commit()?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&0, &reader.num_docs());
    assert_equals(&3, &reader.max_doc());
    assert_equals(&0, &writer.get_doc_stats()?.num_docs);
    assert_equals(&3, &writer.get_doc_stats()?.max_doc);
    assert_equals(&3, &Arc::clone(&reader).leaves()?.len());
    reader.close()?;
    writer.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&0, &reader.num_docs());
    assert_equals(&3, &reader.max_doc());
    assert_equals(&0, &writer.get_doc_stats()?.num_docs);
    assert_equals(&3, &writer.get_doc_stats()?.max_doc);
    assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// The anonymous `FilterLeafReader` of `testSoftDeleteWithRetention`, which
/// hides no document.
struct NoLiveDocsLeafReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for NoLiveDocsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoLiveDocsLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for NoLiveDocsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for NoLiveDocsLeafReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, do_close, term_vectors, max_doc,
        stored_fields, get_numeric_doc_values, get_binary_doc_values, get_sorted_doc_values,
        get_sorted_numeric_doc_values, get_sorted_set_doc_values, get_norm_values,
        get_doc_values_skipper, get_float_vector_values, get_byte_vector_values,
        search_nearest_vectors, search_nearest_vectors_byte, get_field_infos,
        get_point_values, check_integrity, get_meta_data, terms);

    /// `leafReader.getCoreCacheHelper()`.
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }

    /// `leafReader.getReaderCacheHelper()`.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        None
    }

    fn num_docs(&self) -> i32 {
        LeafReader::max_doc(self)
    }
}

/// `ids.add(id)` on a `Collections.synchronizedSet(new HashSet<>())`.
type SyncIds = Arc<Mutex<JavaHashSet<String>>>;

/// Port of `TestSoftDeletesRetentionMergePolicy.testSoftDeleteWithRetention`.
///
/// Tests soft deletes that carry over deleted documents on merge for history
/// rentention.
fn test_soft_delete_with_retention() -> Result<()> {
    let seq_ids = Arc::new(AtomicI32::new(0));
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    {
        let seq_ids = Arc::clone(&seq_ids);
        index_writer_config.set_merge_policy(Some(Arc::new(
            SoftDeletesRetentionMergePolicy::new(
                "soft_delete",
                supplier(move || {
                    IntPoint::new_range_query(
                        "seq_id",
                        &[seq_ids.load(Ordering::SeqCst) - 50],
                        &[i32::MAX],
                    )
                    .unwrap_or_else(|e| {
                        rucene_test_framework::util::java_exceptions::rethrow_java(e)
                    })
                }),
                index_writer_config.merge_policy(),
            ),
        )))?;
    }
    index_writer_config.set_soft_deletes_field(Some("soft_delete".to_string()));
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;
    let num_threads = 2 + random().next_int_bound(3);
    let start_latch = Arc::new(CountDownLatch::new(1));
    let started = Arc::new(CountDownLatch::new(num_threads as i64));
    let update_several_docs = random().next_boolean();
    let ids: SyncIds = Arc::new(Mutex::new(JavaHashSet::new()));
    let mut threads = Vec::new();
    for _i in 0..num_threads {
        let (writer, seq_ids, start_latch, started, ids) = (
            Arc::clone(&writer),
            Arc::clone(&seq_ids),
            Arc::clone(&start_latch),
            Arc::clone(&started),
            Arc::clone(&ids),
        );
        let t = Thread::unnamed(move || {
            let run = || -> Result<()> {
                started.count_down();
                start_latch.await_();
                for _d in 0..100 {
                    let id = random().next_int_bound(10).to_string();
                    let seq_id = seq_ids.fetch_add(1, Ordering::SeqCst) + 1;
                    if update_several_docs {
                        // `Arrays.asList(doc, doc)`: the same document twice,
                        // here two documents built alike, as a slice holds
                        // its documents by value.
                        let make_doc = || -> Result<Document> {
                            let mut doc = Document::new();
                            doc.add(stored_string("id", &id)?);
                            doc.add(Box::new(IntPoint::new("seq_id", &[seq_id])?));
                            Ok(doc)
                        };
                        let docs = [make_doc()?, make_doc()?];
                        writer.soft_update_documents(
                            Term::from_text("id", &id),
                            &docs[..],
                            &[ndv("soft_delete", 1)?],
                        )?;
                    } else {
                        let mut doc = Document::new();
                        doc.add(stored_string("id", &id)?);
                        doc.add(Box::new(IntPoint::new("seq_id", &[seq_id])?));
                        writer.soft_update_document(
                            Term::from_text("id", &id),
                            &doc,
                            &[ndv("soft_delete", 1)?],
                        )?;
                    }
                    if LuceneTestCase::rarely() {
                        writer.flush()?;
                    }
                    ids.lock().unwrap_or_else(|e| e.into_inner()).add(id);
                }
                Ok(())
            };
            if let Err(e) = run() {
                Throwable::from_cause("java.lang.AssertionError", java_throwable(&e)).throw();
            }
        });
        t.start();
        threads.push(t);
    }
    started.await_();
    start_latch.count_down();

    for t in &threads {
        if let Err(p) = t.join() {
            std::panic::resume_unwind(p);
        }
    }
    let ids: Vec<String> = ids
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    let reader = directory_reader::open_with_writer(&*writer)?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    for id in &ids {
        let top_docs = searcher.search(Arc::new(TermQuery::new(Term::from_text("id", id))), 10)?;
        if update_several_docs {
            assert_equals(&2i64, &top_docs.total_hits.value());
            assert_equals(
                &(top_docs.score_docs[0].doc - top_docs.score_docs[1].doc).abs(),
                &1,
            );
        } else {
            assert_equals(&1i64, &top_docs.total_hits.value());
        }
    }
    writer.add_document(&Document::new())?; // add a dummy doc to trigger a segment here
    writer.flush()?;
    writer.force_merge(1)?;
    let old_reader = reader;
    let reader = match directory_reader::open_if_changed_with_writer(
        Arc::clone(&old_reader),
        &*writer,
        true,
    )? {
        Some(reader) => {
            old_reader.close()?;
            Assert::assert_false(std::ptr::addr_eq(
                Arc::as_ptr(&old_reader),
                Arc::as_ptr(&reader),
            ));
            reader
        }
        None => old_reader,
    };
    assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
    let leaf_reader = leaf(&reader, 0)?;
    let filtered: Arc<dyn LeafReader> = Arc::new(NoLiveDocsLeafReader {
        inner: FilterLeafReader::new(leaf_reader)?,
    });
    let searcher = IndexSearcher::new(filtered.as_index_reader())?;
    let seq_id = searcher.search(
        IntPoint::new_range_query(
            "seq_id",
            &[seq_ids.load(Ordering::SeqCst) - 50],
            &[i32::MAX],
        )?,
        10,
    )?;
    Assert::assert_true_msg(
        &format!("{} hits", seq_id.total_hits.value()),
        seq_id.total_hits.value() >= 50,
    );
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    for id in &ids {
        let hits = searcher
            .search(Arc::new(TermQuery::new(Term::from_text("id", id))), 10)?
            .total_hits
            .value();
        if update_several_docs {
            assert_equals(&2i64, &hits);
        } else {
            assert_equals(&1i64, &hits);
        }
    }
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testForceMergeDeletes`.
fn test_force_merge_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(Some("soft_delete".to_string()));
    config.set_merge_policy(Some(LuceneTestCase::new_merge_policy_include_mock(
        &*random(),
        false,
    )))?; // no mock MP it might not select segments for force merge
    if random().next_boolean() {
        config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
            "soft_delete",
            match_none(),
            config.merge_policy(),
        ))))?;
    }
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    // The first segment includes d1 and d2
    for i in 0..2 {
        let mut d = Document::new();
        d.add(stored_string("id", &i.to_string())?);
        writer.add_document(&d)?;
    }
    writer.flush()?;
    // The second segment includes only the tombstone
    let mut tombstone = Document::new();
    tombstone.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
    writer.soft_update_document(
        Term::from_text("id", "1"),
        &tombstone,
        &[ndv("soft_delete", 1)?],
    )?;
    writer.flush_with(false, true)?; // flush pending updates but don't trigger a merge, we run forceMergeDeletes below
                                     // Now we have have two segments - both having soft-deleted documents.
                                     // We expect any MP to merge these segments into one segment
                                     // when calling forceMergeDeletes.
    writer.force_merge_deletes_with(true)?;
    assert_equals(&1, &writer.clone_segment_infos()?.size());
    assert_equals(&1, &writer.get_doc_stats()?.num_docs);
    assert_equals(&1, &writer.get_doc_stats()?.max_doc);
    writer.close()?;
    dir.close()
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testDropFullySoftDeletedSegment`.
fn test_drop_fully_soft_deleted_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let soft_delete = if random().next_boolean() {
        None
    } else {
        Some("soft_delete")
    };
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(soft_delete.map(str::to_string));
    config.set_merge_policy(Some(LuceneTestCase::new_merge_policy_include_mock(
        &*random(),
        true,
    )))?;
    if let Some(soft_delete) = soft_delete {
        if random().next_boolean() {
            config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
                soft_delete,
                match_none(),
                config.merge_policy(),
            ))))?;
        }
    }
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    for i in 0..2 {
        let mut d = Document::new();
        d.add(stored_string("id", &i.to_string())?);
        writer.add_document(&d)?;
    }
    writer.flush()?;
    assert_equals(&1, &writer.clone_segment_infos()?.size());

    if let Some(soft_delete) = soft_delete {
        // the newly created segment should be dropped as it is fully deleted (i.e. only contains
        // deleted docs).
        if random().next_boolean() {
            let mut tombstone = Document::new();
            tombstone.add(Box::new(NumericDocValuesField::new(soft_delete, 1)));
            writer.soft_update_document(
                Term::from_text("id", "1"),
                &tombstone,
                &[ndv(soft_delete, 1)?],
            )?;
        } else {
            let mut doc = Document::new();
            doc.add(stored_string("id", "1")?);
            if random().next_boolean() {
                writer.soft_update_document(
                    Term::from_text("id", "1"),
                    &doc,
                    &[ndv(soft_delete, 1)?],
                )?;
            } else {
                writer.add_document(&doc)?;
            }
            writer.update_doc_values(Term::from_text("id", "1"), &[ndv(soft_delete, 1)?])?;
        }
    } else {
        let mut d = Document::new();
        d.add(stored_string("id", "1")?);
        writer.add_document(&d)?;
        writer.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
    }
    writer.commit()?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    assert_equals(&reader.num_docs(), &1);
    reader.close()?;
    assert_equals(&1, &writer.clone_segment_infos()?.size());

    writer.close()?;
    dir.close()
}

/// A `MergedSegmentWarmer` lambda (`sr -> { ... }`) over the writer, which it
/// holds weakly: Java's lambda captures the writer whose config holds it, a
/// cycle a garbage collector reclaims and reference counting would not.
struct WarmerFn<F: Fn(&IndexWriter) -> Result<()> + Send + Sync> {
    writer: Mutex<Weak<IndexWriter>>,
    body: F,
}

impl<F: Fn(&IndexWriter) -> Result<()> + Send + Sync> fmt::Debug for WarmerFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TestSoftDeletesRetentionMergePolicy$$Lambda")
    }
}

impl<F: Fn(&IndexWriter) -> Result<()> + Send + Sync> fmt::Display for WarmerFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl<F: Fn(&IndexWriter) -> Result<()> + Send + Sync> MergedSegmentWarmer for WarmerFn<F> {
    fn warm(&self, _reader: &Arc<dyn LeafReader>) -> Result<()> {
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade();
        if let Some(writer) = writer {
            (self.body)(&writer)?;
        }
        Ok(())
    }
}

/// `writer.getConfig().setMergedSegmentWarmer(sr -> body(writer))`.
fn set_warmer(
    writer: &Arc<IndexWriter>,
    body: impl Fn(&IndexWriter) -> Result<()> + Send + Sync + 'static,
) {
    writer
        .get_config()
        .set_merged_segment_warmer(Some(Arc::new(WarmerFn {
            writer: Mutex::new(Arc::downgrade(writer)),
            body,
        })));
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testSoftDeleteWhileMergeSurvives`.
fn test_soft_delete_while_merge_survives() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let soft_delete = "soft_delete";
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(Some(soft_delete.to_string()));
    let update = Arc::new(AtomicBool::new(true));
    config.set_reader_pooling(true);
    config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "soft_delete",
        supplier(|| Arc::new(FieldExistsQuery::new("keep"))),
        log_doc(),
    ))))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    {
        let update = Arc::clone(&update);
        set_warmer(&writer, move |writer| {
            if update
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let run = || -> Result<()> {
                    writer.soft_update_document(
                        Term::from_text("id", "0"),
                        &Document::new(),
                        &[ndv(soft_delete, 1)?, ndv("keep", 1)?],
                    )?;
                    writer.commit()?;
                    Ok(())
                };
                if let Err(e) = run() {
                    Throwable::from_cause("java.lang.AssertionError", java_throwable(&e)).throw();
                }
            }
            Ok(())
        });
    }

    let pre_existing_deletes = random().next_boolean();
    for i in 0..2 {
        let mut d = Document::new();
        d.add(stored_string("id", &i.to_string())?);
        if pre_existing_deletes && random().next_boolean() {
            writer.add_document(&d)?; // randomly add a preexisting hard-delete that we don't carry over
            writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
            d.add(Box::new(NumericDocValuesField::new("keep", 1)));
            writer.add_document(&d)?;
        } else {
            d.add(Box::new(NumericDocValuesField::new("keep", 1)));
            writer.add_document(&d)?;
        }
        writer.flush()?;
    }
    writer.force_merge(1)?;
    writer.commit()?;
    Assert::assert_false(update.load(Ordering::SeqCst));
    let open = directory_reader::open(dir.as_directory())?;
    assert_equals(&0, &open.num_deleted_docs());
    assert_equals(&3, &open.max_doc());
    IOUtils::close([
        IOUtils::closeable(|| open.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// This test is trying to hard-delete a particular document while the segment
/// is merged which is already soft-deleted. This requires special logic inside
/// IndexWriter#carryOverHardDeletes since docMaps are not created for this
/// document.
///
/// Port of `TestSoftDeletesRetentionMergePolicy.testDeleteDocWhileMergeThatIsSoftDeleted`.
fn test_delete_doc_while_merge_that_is_soft_deleted() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let soft_delete = "soft_delete";
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(Some(soft_delete.to_string()));
    let delete = Arc::new(AtomicBool::new(true));
    config.set_reader_pooling(true);
    config.set_merge_policy(Some(log_doc()))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let mut d = Document::new();
    d.add(stored_string("id", "0")?);
    writer.add_document(&d)?;
    let mut d = Document::new();
    d.add(stored_string("id", "1")?);
    writer.add_document(&d)?;
    if random().next_boolean() {
        // randomly run with a preexisting hard delete
        let mut d = Document::new();
        d.add(stored_string("id", "2")?);
        writer.add_document(&d)?;
        writer.delete_documents_by_terms(vec![Term::from_text("id", "2")])?;
    }

    writer.flush()?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.soft_update_document(
        Term::from_text("id", "0"),
        &Document::new(),
        &[ndv(soft_delete, 1)?],
    )?;
    writer.flush()?;
    {
        let delete = Arc::clone(&delete);
        let reader: Arc<dyn IndexReader> = Arc::clone(&reader) as _;
        set_warmer(&writer, move |writer| {
            if delete
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                match writer.try_delete_document(&reader, 0) {
                    Ok(seq_no) => Assert::assert_true_msg("seqId was -1", seq_no != -1),
                    Err(e) => Throwable::from_cause("java.lang.AssertionError", java_throwable(&e))
                        .throw(),
                }
            }
            Ok(())
        });
    }
    writer.force_merge(1)?;
    assert_equals(&2, &writer.get_doc_stats()?.num_docs);
    assert_equals(&2, &writer.get_doc_stats()?.max_doc);
    Assert::assert_false(delete.load(Ordering::SeqCst));
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// `try (DirectoryReader reader = DirectoryReader.open(writer)) {
/// assertEquals(maxDoc, reader.maxDoc()); assertEquals(numDocs,
/// reader.numDocs()); }`.
fn check_counts(writer: &IndexWriter, max_doc: i32, num_docs: i32) -> Result<()> {
    let reader = directory_reader::open_with_writer(writer)?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_equals(&max_doc, &reader.max_doc());
        assert_equals(&num_docs, &reader.num_docs());
    }));
    let closed = reader.close();
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
    closed
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testUndeleteDocument`.
fn test_undelete_document() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let soft_delete = "soft_delete";
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(Some(soft_delete.to_string()));
    config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "soft_delete",
        match_all(),
        log_doc(),
    ))))?;
    config.set_reader_pooling(true);
    config.set_merge_policy(Some(log_doc()))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let mut d = Document::new();
    d.add(stored_string("id", "0")?);
    d.add(stored_string("seq_id", "0")?);
    writer.add_document(&d)?;
    let mut d = Document::new();
    d.add(stored_string("id", "1")?);
    writer.add_document(&d)?;
    writer.update_doc_values(Term::from_text("id", "0"), &[ndv("soft_delete", 1)?])?;
    check_counts(&writer, 2, 1)?;
    do_update(
        &Term::from_text("id", "0"),
        &writer,
        &[ndv_null("soft_delete")?],
    )?;
    check_counts(&writer, 2, 2)?;
    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testMergeSoftDeleteAndHardDelete`.
fn test_merge_soft_delete_and_hard_delete() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let soft_delete = "soft_delete";
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(Some(soft_delete.to_string()));
    config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "soft_delete",
        match_all(),
        log_doc(),
    ))))?;
    config.set_reader_pooling(true);
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let mut d = Document::new();
    d.add(stored_string("id", "0")?);
    writer.add_document(&d)?;
    let mut d = Document::new();
    d.add(stored_string("id", "1")?);
    d.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
    writer.add_document(&d)?;
    check_counts(&writer, 2, 1)?;
    loop {
        let reader = directory_reader::open_with_writer(&*writer)?;
        let result = (|| -> Result<bool> {
            let top_docs =
                IndexSearcher::new(include_soft_deletes_wrapper(Arc::clone(&reader))? as _)?
                    .search(Arc::new(TermQuery::new(Term::from_text("id", "1"))), 1)?;
            assert_equals(&1i64, &top_docs.total_hits.value());
            Ok(writer.try_delete_document(
                &(Arc::clone(&reader) as Arc<dyn IndexReader>),
                top_docs.score_docs[0].doc,
            )? > 0)
        })();
        let closed = reader.close();
        if result.and_then(|done| closed.map(|()| done))? {
            break;
        }
    }
    writer.force_merge_deletes_with(true)?;
    assert_equals(&1, &writer.clone_segment_infos()?.size());
    let si = writer.clone_segment_infos()?.info(0).clone();
    assert_equals(&0, &si.get_soft_del_count()); // hard-delete should supersede the soft-delete
    assert_equals(&0, &si.get_del_count());
    assert_equals(&1, &si.info.max_doc()?);
    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testSoftDeleteWithTryUpdateDocValue`.
fn test_soft_delete_with_try_update_doc_value() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(Some("soft_delete".to_string()));
    config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "soft_delete",
        match_all(),
        Arc::new(LuceneTestCase::new_log_merge_policy()),
    ))))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let sm: SearcherManager = SearcherManager::from_writer(
        &*writer,
        true,
        false,
        Some(Arc::new(DefaultSearcherFactory)),
        None,
    )?;
    let mut d = Document::new();
    d.add(stored_string("id", "0")?);
    writer.add_document(&d)?;
    sm.maybe_refresh_blocking()?;
    do_update(
        &Term::from_text("id", "0"),
        &writer,
        &[ndv("soft_delete", 1)?, ndv("other-field", 1)?],
    )?;
    sm.maybe_refresh_blocking()?;
    assert_equals(&1, &writer.clone_segment_infos()?.size());
    let si = writer.clone_segment_infos()?.info(0).clone();
    assert_equals(&1, &si.get_soft_del_count());
    assert_equals(&1, &si.info.max_doc()?);
    IOUtils::close([
        IOUtils::closeable(|| sm.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testMixedSoftDeletesAndHardDeletes`.
fn test_mixed_soft_deletes_and_hard_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let soft_deletes_field = "soft-deletes";
    let config = LuceneTestCase::new_index_writer_config();
    config.set_max_buffered_docs(2 + random().next_int_bound(50))?;
    config.set_ram_buffer_size_mb(
        rucene::index::index_writer_config::LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64,
    )?;
    config.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        soft_deletes_field,
        match_all(),
        LuceneTestCase::new_merge_policy(),
    ))))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let num_docs = 10 + random().next_int_bound(100);
    let mut live_docs: JavaHashSet<String> = JavaHashSet::new();
    for i in 0..num_docs {
        let id = i.to_string();
        let mut doc = Document::new();
        doc.add(stored_string("id", &id)?);
        writer.add_document(&doc)?;
        live_docs.add(id);
    }
    for i in 0..num_docs {
        if random().next_boolean() {
            let id = i.to_string();
            if random().next_boolean() && live_docs.iter().any(|d| *d == id) {
                do_update(
                    &Term::from_text("id", &id),
                    &writer,
                    &[ndv(soft_deletes_field, 1)?],
                )?;
            } else {
                let mut doc = Document::new();
                doc.add(stored_string("id", &format!("v{id}"))?);
                writer.soft_update_document(
                    Term::from_text("id", &id),
                    &doc,
                    &[ndv(soft_deletes_field, 1)?],
                )?;
                live_docs.add(format!("v{id}"));
            }
        }
        if random().next_boolean() && live_docs.size() != 0 {
            let all: Vec<String> = live_docs.iter().cloned().collect();
            let del_id = RandomPicks::random_from(&*random(), &all).clone();
            if random().next_boolean() {
                do_delete(&Term::from_text("id", &del_id), &writer)?;
            } else {
                writer.delete_documents_by_terms(vec![Term::from_text("id", &del_id)])?;
            }
            live_docs.retain(|d| *d != del_id);
        }
    }
    {
        let unwrapped = directory_reader::open_with_writer(&*writer)?;
        let result = (|| -> Result<()> {
            let reader = include_soft_deletes_wrapper(Arc::clone(&unwrapped))?;
            assert_equals(&(live_docs.size() as i32), &reader.num_docs());
            Ok(())
        })();
        let closed = unwrapped.close();
        result.and(closed)?;
    }
    writer.commit()?;
    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSoftDeletesRetentionMergePolicy.testRewriteRetentionQuery`.
fn test_rewrite_retention_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_soft_deletes_field(Some("soft_deletes".to_string()));
    config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "soft_deletes",
        supplier(|| {
            Arc::new(
                PrefixQuery::new(Term::from_text("id", "foo")).unwrap_or_else(|e| {
                    rucene_test_framework::util::java_exceptions::rethrow_java(e)
                }),
            )
        }),
        LuceneTestCase::new_merge_policy(),
    ))))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;

    let mut d = Document::new();
    d.add(stored_string("id", "foo-1")?);
    writer.add_document(&d)?;
    let mut d = Document::new();
    d.add(stored_string("id", "foo-2")?);
    writer.soft_update_document(
        Term::from_text("id", "foo-1"),
        &d,
        &[ndv("soft_deletes", 1)?],
    )?;

    let mut d = Document::new();
    d.add(stored_string("id", "bar-1")?);
    writer.add_document(&d)?;
    d.add(stored_string("id", "bar-2")?);
    writer.soft_update_document(
        Term::from_text("id", "bar-1"),
        &d,
        &[ndv("soft_deletes", 1)?],
    )?;

    writer.force_merge(1)?;
    assert_equals(&2, &writer.get_doc_stats()?.num_docs); // foo-2, bar-2
    assert_equals(&3, &writer.get_doc_stats()?.max_doc); // foo-1, foo-2, bar-2
    IOUtils::close([
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// The document `doc` matches, searched through an `IncludeSoftDeletesWrapper`
/// over a fresh NRT reader, then `op(reader, theDoc)`, retried while it
/// answers -1 (a merge was just committing).
fn retry_on_doc(
    doc: &Term,
    writer: &IndexWriter,
    mut op: impl FnMut(&Arc<dyn IndexReader>, i32) -> Result<i64>,
) -> Result<()> {
    loop {
        // retry if we just committing a merge
        let reader = directory_reader::open_with_writer(writer)?;
        let result = (|| -> Result<i64> {
            let top_docs =
                IndexSearcher::new(include_soft_deletes_wrapper(Arc::clone(&reader))? as _)?
                    .search(Arc::new(TermQuery::new(doc.clone())), 10)?;
            assert_equals(&1i64, &top_docs.total_hits.value());
            let the_doc = top_docs.score_docs[0].doc;
            op(&(Arc::clone(&reader) as Arc<dyn IndexReader>), the_doc)
        })();
        let closed = reader.close();
        let seq_id = result.and_then(|s| closed.map(|()| s))?;
        if seq_id != -1 {
            return Ok(());
        }
    }
}

/// `static void doUpdate(Term doc, IndexWriter writer, Field... fields)`.
fn do_update(doc: &Term, writer: &IndexWriter, fields: &[Field]) -> Result<()> {
    retry_on_doc(doc, writer, |reader, the_doc| {
        writer.try_update_doc_value(reader, the_doc, fields)
    })
}

/// `static void doDelete(Term doc, IndexWriter writer)`.
fn do_delete(doc: &Term, writer: &IndexWriter) -> Result<()> {
    retry_on_doc(doc, writer, |reader, the_doc| {
        writer.try_delete_document(reader, the_doc)
    })
}

/// The anonymous `FilterLeafReader` of `IncludeSoftDeletesSubReaderWrapper`:
/// the segment's hard live docs only.
struct HardLiveDocsLeafReader {
    inner: FilterLeafReader,
    num_docs: i32,
}

impl HardLiveDocsLeafReader {
    /// The wrapped segment reader.
    fn segment(&self) -> &rucene::index::segment_reader::SegmentReader {
        self.inner
            .get_delegate()
            .as_segment_reader()
            .expect("checked when wrapped")
    }
}

impl fmt::Debug for HardLiveDocsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HardLiveDocsLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for HardLiveDocsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for HardLiveDocsLeafReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, do_close, term_vectors, max_doc,
        stored_fields, get_numeric_doc_values, get_binary_doc_values, get_sorted_doc_values,
        get_sorted_numeric_doc_values, get_sorted_set_doc_values, get_norm_values,
        get_doc_values_skipper, get_float_vector_values, get_byte_vector_values,
        search_nearest_vectors, search_nearest_vectors_byte, get_field_infos,
        get_point_values, check_integrity, get_meta_data, terms);

    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    /// `hardLiveDocs`.
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.segment().get_hard_live_docs()
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// `private static final class IncludeSoftDeletesSubReaderWrapper`.
struct IncludeSoftDeletesSubReaderWrapper;

impl SubReaderWrapper for IncludeSoftDeletesSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        // `while (reader instanceof FilterLeafReader) reader = getDelegate()`.
        let reader = FilterLeafReader::unwrap(&reader);
        let segment = reader.as_segment_reader().unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
        });
        let hard_live_docs = segment.get_hard_live_docs();
        let num_docs = match hard_live_docs {
            None => reader.max_doc(),
            Some(hard_live_docs) => {
                let mut bits = 0;
                for i in 0..hard_live_docs.length() {
                    if hard_live_docs.get(i) {
                        bits += 1;
                    }
                }
                bits
            }
        };
        Ok(Arc::new(HardLiveDocsLeafReader {
            inner: FilterLeafReader::new(reader)?,
            num_docs,
        }))
    }
}

/// `IncludeSoftDeletesWrapper.doWrapDirectoryReader`; its
/// `getReaderCacheHelper()` returns `null`, the default of a wrapper that
/// does not delegate the cache helper.
struct IncludeSoftDeletesWrapper;

impl DirectoryReaderWrapper for IncludeSoftDeletesWrapper {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        include_soft_deletes_wrapper(inner)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "IncludeSoftDeletesWrapper"
    }
}

/// `new IncludeSoftDeletesWrapper(in)`.
fn include_soft_deletes_wrapper(
    r#in: Arc<dyn DirectoryReader>,
) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(IncludeSoftDeletesSubReaderWrapper),
        Arc::new(IncludeSoftDeletesWrapper),
    )?))
}
