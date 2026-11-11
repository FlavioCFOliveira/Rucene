//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSoftDeletesDirectoryReaderWrapper.java`.
//!
//! `new SoftDeletesDirectoryReaderWrapper(in, field)` is
//! `SoftDeletesDirectoryReaderWrapper::new(field).wrap(in)` in Rucene, and
//! Java's `getSequentialSubReaders()` of a directory reader, the list of its
//! leaves, is read through `leaves()`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, NumericDocValuesField, NumericValue, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper, SoftDeletesFilterCodecReader,
    SoftDeletesFilterLeafReader,
};
use rucene::index::index_reader::{CacheKey, ClosedListener};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{
    MergePolicy, NoMergePolicy, RetentionQuerySupplier, SoftDeletesRetentionMergePolicy,
};
use rucene::index::terms::Term;
use rucene::index::{DirectoryReader, IndexReader, LeafReader};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::IndexSearcher;
use rucene::util::IOUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_null, class, random, test, JavaHashSet};

/// `TestSoftDeletesDirectoryReaderWrapper`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSoftDeletesDirectoryReaderWrapper",
        vec![
            test(
                "testDropFullyDeletedSegments",
                test_drop_fully_deleted_segments,
            ),
            test(
                "testReuseUnchangedLeafReader",
                test_reuse_unchanged_leaf_reader,
            ),
            test("testMixSoftAndHardDeletes", test_mix_soft_and_hard_deletes),
            test("testReaderCacheKey", test_reader_cache_key),
            test(
                "testAvoidWrappingReadersWithoutSoftDeletes",
                test_avoid_wrapping_readers_without_soft_deletes,
            ),
        ],
    )
}

/// `new SoftDeletesDirectoryReaderWrapper(in, field)`.
fn soft_wrap(r#in: Arc<dyn DirectoryReader>, field: &str) -> Result<Arc<dyn DirectoryReader>> {
    SoftDeletesDirectoryReaderWrapper::new(field).wrap(r#in)
}

/// `new NumericDocValuesField(name, value)`, as the `Field` the update APIs
/// take.
fn ndv_field(name: &str, value: i64) -> Result<Field> {
    Field::new_with_number(
        name,
        NumericValue::Long(value),
        NumericDocValuesField::type_().clone(),
    )
}

/// A document with the stored string fields `id` and `version`.
fn id_version_doc(id: &str, version: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", id.into(), Store::YES)?));
    doc.add(Box::new(StringField::new(
        "version",
        version.into(),
        Store::YES,
    )?));
    Ok(doc)
}

/// `MatchAllDocsQuery::new`.
fn match_all() -> RetentionQuerySupplier {
    Arc::new(|| Arc::new(MatchAllDocsQuery))
}

/// `MatchNoDocsQuery::new`.
fn match_none() -> RetentionQuerySupplier {
    Arc::new(|| Arc::new(MatchNoDocsQuery::instance()))
}

/// `reader.getSequentialSubReaders()` of a directory reader: its leaves.
fn sub_readers(reader: &Arc<dyn DirectoryReader>) -> Result<Vec<Arc<dyn LeafReader>>> {
    Ok(Arc::clone(reader)
        .leaves()?
        .iter()
        .map(|ctx| ctx.leaf_reader())
        .collect())
}

/// Identity of two readers (`==` on Java references).
fn same_reader<A: ?Sized, B: ?Sized>(a: &Arc<A>, b: &Arc<B>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// Port of `TestSoftDeletesDirectoryReaderWrapper.testDropFullyDeletedSegments`.
fn test_drop_fully_deleted_segments() -> Result<()> {
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    let soft_deletes_field = "soft_delete";
    index_writer_config.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    index_writer_config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        soft_deletes_field,
        match_all(),
        NoMergePolicy::instance(),
    ))))?;
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;
        let result = (|| -> Result<()> {
            writer.add_document(&id_version_doc("1", "1")?)?;
            writer.commit()?;
            writer.add_document(&id_version_doc("2", "1")?)?;
            writer.commit()?;

            {
                let reader = soft_wrap(
                    directory_reader::open(dir.as_directory())?,
                    soft_deletes_field,
                )?;
                assert_equals(&2, &Arc::clone(&reader).leaves()?.len());
                assert_equals(&2, &reader.num_docs());
                assert_equals(&2, &reader.max_doc());
                assert_equals(&0, &reader.num_deleted_docs());
                reader.close()?;
            }
            writer.update_doc_values(
                Term::from_text("id", "1"),
                &[ndv_field(soft_deletes_field, 1)?],
            )?;
            writer.commit()?;
            {
                let reader = soft_wrap(
                    directory_reader::open_with_writer(&*writer)?,
                    soft_deletes_field,
                )?;
                assert_equals(&1, &reader.num_docs());
                assert_equals(&1, &reader.max_doc());
                assert_equals(&0, &reader.num_deleted_docs());
                assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
                reader.close()?;
            }
            {
                let reader = soft_wrap(
                    directory_reader::open(dir.as_directory())?,
                    soft_deletes_field,
                )?;
                assert_equals(&1, &reader.num_docs());
                assert_equals(&1, &reader.max_doc());
                assert_equals(&0, &reader.num_deleted_docs());
                assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
                reader.close()?;
            }

            {
                let reader = directory_reader::open(dir.as_directory())?;
                assert_equals(&2, &reader.num_docs());
                assert_equals(&2, &reader.max_doc());
                assert_equals(&0, &reader.num_deleted_docs());
                assert_equals(&2, &Arc::clone(&reader).leaves()?.len());
                reader.close()?;
            }
            Ok(())
        })();
        let closed = writer.close();
        result.and(closed)
    })();
    let closed = dir.close();
    result.and(closed)
}

/// Port of `TestSoftDeletesDirectoryReaderWrapper.testReuseUnchangedLeafReader`.
fn test_reuse_unchanged_leaf_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    let soft_deletes_field = "soft_delete";
    index_writer_config.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    index_writer_config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;

    writer.add_document(&id_version_doc("1", "1")?)?;
    writer.add_document(&id_version_doc("2", "1")?)?;
    writer.commit()?;
    let reader = soft_wrap(
        directory_reader::open(dir.as_directory())?,
        soft_deletes_field,
    )?;
    assert_equals(&2, &reader.num_docs());
    assert_equals(&2, &reader.max_doc());
    assert_equals(&0, &reader.num_deleted_docs());

    writer.soft_update_document(
        Term::from_text("id", "1"),
        &id_version_doc("1", "2")?,
        &[ndv_field("soft_delete", 1)?],
    )?;

    writer.add_document(&id_version_doc("3", "1")?)?;
    writer.commit()?;

    let new_reader = directory_reader::open_if_changed(Arc::clone(&reader))?
        .unwrap_or_else(|| crate::support::npe("openIfChanged"));
    Assert::assert_false(same_reader(&new_reader, &reader));
    reader.close()?;
    let reader = new_reader;
    assert_equals(&3, &reader.num_docs());
    assert_equals(&4, &reader.max_doc());
    assert_equals(&1, &reader.num_deleted_docs());

    writer.soft_update_document(
        Term::from_text("id", "1"),
        &id_version_doc("1", "3")?,
        &[ndv_field("soft_delete", 1)?],
    )?;
    writer.commit()?;

    let new_reader = directory_reader::open_if_changed(Arc::clone(&reader))?
        .unwrap_or_else(|| crate::support::npe("openIfChanged"));
    Assert::assert_false(same_reader(&new_reader, &reader));
    let new_subs = sub_readers(&new_reader)?;
    let subs = sub_readers(&reader)?;
    assert_equals(&3, &new_subs.len());
    assert_equals(&2, &subs.len());
    Assert::assert_true(same_reader(&subs[0], &new_subs[0]));
    Assert::assert_false(same_reader(&subs[1], &new_subs[1]));
    Assert::assert_true(is_wrapped(&*subs[0]));
    // last one has no soft deletes
    Assert::assert_false(is_wrapped(&*subs[1]));

    Assert::assert_true(is_wrapped(&*new_subs[0]));
    Assert::assert_true(is_wrapped(&*new_subs[1]));
    // last one has no soft deletes
    Assert::assert_false(is_wrapped(&*new_subs[2]));
    reader.close()?;
    let reader = new_reader;
    assert_equals(&3, &reader.num_docs());
    assert_equals(&5, &reader.max_doc());
    assert_equals(&2, &reader.num_deleted_docs());
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// `private boolean isWrapped(LeafReader reader)`.
fn is_wrapped(reader: &dyn LeafReader) -> bool {
    reader.leaf_as_any().is_some_and(|a| {
        a.downcast_ref::<SoftDeletesFilterLeafReader>().is_some()
            || a.downcast_ref::<SoftDeletesFilterCodecReader>().is_some()
    })
}

/// Port of `TestSoftDeletesDirectoryReaderWrapper.testMixSoftAndHardDeletes`.
fn test_mix_soft_and_hard_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    let soft_deletes_field = "soft_delete";
    index_writer_config.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;
    let mut unique_docs: JavaHashSet<i32> = JavaHashSet::new();
    for _i in 0..100 {
        let doc_id = random().next_int_bound(5);
        unique_docs.add(doc_id);
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            doc_id.to_string(),
            Store::YES,
        )?));
        if doc_id % 2 == 0 {
            writer.update_document(Term::from_text("id", &doc_id.to_string()), &doc)?;
        } else {
            writer.soft_update_document(
                Term::from_text("id", &doc_id.to_string()),
                &doc,
                &[ndv_field(soft_deletes_field, 0)?],
            )?;
        }
    }

    writer.commit()?;
    writer.close()?;
    let reader = soft_wrap(
        directory_reader::open(dir.as_directory())?,
        soft_deletes_field,
    )?;
    assert_equals(&(unique_docs.size() as i32), &reader.num_docs());
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    for doc_id in unique_docs.iter() {
        assert_equals(
            &1,
            &searcher.count(Arc::new(TermQuery::new(Term::from_text(
                "id",
                &doc_id.to_string(),
            ))))?,
        );
    }

    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// The closed listeners of `testReaderCacheKey`: `counter.incrementAndGet()`,
/// then `assertSame(key, cacheHelper.getKey())`.
///
/// Java's lambda reads `cacheHelper.getKey()` when it runs; the listener holds
/// the key the helper answered when it was registered, the same key (a helper
/// answers one key for its life), so that the listener does not own the
/// helper whose listeners own it.
struct CountingListener {
    counter: Arc<AtomicI32>,
    helper_key: CacheKey,
}

impl ClosedListener for CountingListener {
    fn on_close(&self, key: CacheKey) -> Result<()> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        // `assertSame(key, cacheHelper.getKey())`.
        Assert::assert_true(key == self.helper_key);
        Ok(())
    }
}

/// Port of `TestSoftDeletesDirectoryReaderWrapper.testReaderCacheKey`.
fn test_reader_cache_key() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    let soft_deletes_field = "soft_delete";
    index_writer_config.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    index_writer_config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;

    writer.add_document(&id_version_doc("1", "1")?)?;
    writer.add_document(&id_version_doc("2", "1")?)?;
    writer.commit()?;
    let reader = soft_wrap(
        directory_reader::open(dir.as_directory())?,
        soft_deletes_field,
    )?;
    let reader_cache_helper = Arc::clone(&reader).leaves()?[0]
        .leaf_reader()
        .get_reader_cache_helper()
        .unwrap_or_else(|| crate::support::npe("getReaderCacheHelper"));
    let leaf_called = Arc::new(AtomicI32::new(0));
    let dir_called = Arc::new(AtomicI32::new(0));
    reader_cache_helper.add_closed_listener(Arc::new(CountingListener {
        counter: Arc::clone(&leaf_called),
        helper_key: reader_cache_helper.get_key().clone(),
    }));
    let dir_reader_cache_helper = reader
        .get_reader_cache_helper()
        .unwrap_or_else(|| crate::support::npe("getReaderCacheHelper"));
    dir_reader_cache_helper.add_closed_listener(Arc::new(CountingListener {
        counter: Arc::clone(&dir_called),
        helper_key: dir_reader_cache_helper.get_key().clone(),
    }));
    assert_equals(&2, &reader.num_docs());
    assert_equals(&2, &reader.max_doc());
    assert_equals(&0, &reader.num_deleted_docs());

    writer.soft_update_document(
        Term::from_text("id", "1"),
        &id_version_doc("1", "2")?,
        &[ndv_field("soft_delete", 1)?],
    )?;

    writer.add_document(&id_version_doc("3", "1")?)?;
    writer.commit()?;
    assert_equals(&0, &leaf_called.load(Ordering::SeqCst));
    assert_equals(&0, &dir_called.load(Ordering::SeqCst));
    let new_reader = directory_reader::open_if_changed(Arc::clone(&reader))?
        .unwrap_or_else(|| crate::support::npe("openIfChanged"));
    assert_equals(&0, &leaf_called.load(Ordering::SeqCst));
    assert_equals(&0, &dir_called.load(Ordering::SeqCst));
    // assertNotSame(newReader.getReaderCacheHelper().getKey(), reader.getReaderCacheHelper().getKey());
    let key_of = |r: &Arc<dyn DirectoryReader>| -> CacheKey {
        r.get_reader_cache_helper()
            .unwrap_or_else(|| crate::support::npe("getReaderCacheHelper"))
            .get_key()
            .clone()
    };
    Assert::assert_false(key_of(&new_reader) == key_of(&reader));
    Assert::assert_false(same_reader(&new_reader, &reader));
    reader.close()?;
    let reader = new_reader;
    assert_equals(&1, &dir_called.load(Ordering::SeqCst));
    assert_equals(&1, &leaf_called.load(Ordering::SeqCst));
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// `assertThat(reader, instanceOf(SegmentReader.class))`.
fn assert_segment_reader(reader: &dyn LeafReader) {
    if reader.as_segment_reader().is_none() {
        Assert::fail(&format!(
            "\nExpected: an instance of org.apache.lucene.index.SegmentReader\n     but: <{reader}> is not one"
        ))
    }
}

/// Port of `TestSoftDeletesDirectoryReaderWrapper.testAvoidWrappingReadersWithoutSoftDeletes`.
fn test_avoid_wrapping_readers_without_soft_deletes() -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();
    let soft_deletes_field = "soft_deletes";
    iwc.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    let merge_policy: Arc<dyn MergePolicy> = iwc.merge_policy();
    iwc.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        soft_deletes_field,
        match_all(),
        Arc::clone(&merge_policy),
    ))))?;
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        let result = (|| -> Result<()> {
            let num_docs = 1 + random().next_int_bound(10);
            for i in 0..num_docs {
                let mut doc = Document::new();
                let doc_id = i.to_string();
                doc.add(Box::new(StringField::new("id", doc_id, Store::YES)?));
                writer.add_document(&doc)?;
            }
            let num_deletes = 1 + random().next_int_bound(5);
            for _i in 0..num_deletes {
                let mut doc = Document::new();
                let doc_id = random().next_int_bound(num_docs).to_string();
                doc.add(Box::new(StringField::new(
                    "id",
                    doc_id.clone(),
                    Store::YES,
                )?));
                writer.soft_update_document(
                    Term::from_text("id", &doc_id),
                    &doc,
                    &[ndv_field(soft_deletes_field, 0)?],
                )?;
            }
            writer.flush()?;
            {
                let reader = directory_reader::open_with_writer(&*writer)?;
                let wrapped = soft_wrap(Arc::clone(&reader), soft_deletes_field)?;
                let mut expected_num_deletes = 0;
                let leaves = Arc::clone(&wrapped).leaves()?;
                for i in 0..leaves.len() {
                    expected_num_deletes +=
                        leaves[i].leaf_reader().as_index_reader().num_deleted_docs();
                }
                assert_equals(&num_docs, &wrapped.num_docs());
                assert_equals(&expected_num_deletes, &wrapped.num_deleted_docs());
                reader.close()?;
            }
            writer.get_config().set_merge_policy(Some(Arc::new(
                SoftDeletesRetentionMergePolicy::new(
                    soft_deletes_field,
                    match_none(),
                    Arc::clone(&merge_policy),
                ),
            )))?;
            writer.force_merge(1)?;
            {
                let reader = directory_reader::open_with_writer(&*writer)?;
                for leaf_context in Arc::clone(&reader).leaves()? {
                    let leaf = leaf_context.leaf_reader();
                    assert_segment_reader(&*leaf);
                    let segment_reader = leaf.as_segment_reader().expect("checked");
                    assert_null(&LeafReader::get_live_docs(segment_reader).map(|_| "Bits"));
                    assert_null(&segment_reader.get_hard_live_docs().map(|_| "Bits"));
                }
                let wrapped = soft_wrap(Arc::clone(&reader), soft_deletes_field)?;
                assert_equals(&num_docs, &wrapped.num_docs());
                assert_equals(&0, &wrapped.num_deleted_docs());
                for leaf in Arc::clone(&wrapped).leaves()? {
                    assert_segment_reader(&*leaf.leaf_reader());
                }
                reader.close()?;
            }
            Ok(())
        })();
        let closed = writer.close();
        result.and(closed)
    })();
    let closed = dir.close();
    result.and(closed)
}
