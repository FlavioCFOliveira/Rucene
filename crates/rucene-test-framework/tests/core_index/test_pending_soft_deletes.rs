//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPendingSoftDeletes.java`.
//!
//! It extends `TestPendingDeletes`, whose tests it inherits over its
//! `newPendingDeletes` (`test_pending_deletes`). The anonymous
//! `DocValuesFieldUpdates` of `singleUpdate` is a numeric
//! `DocValuesFieldUpdates` holding the same documents, each with the value
//! `1` (`hasValue`) or reset (`!hasValue`), at `delGen` 0: its iterator
//! returns what the anonymous iterator returns. `assertSame` on two `Bits`
//! compares the bits they share (`Bits::identity`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, NumericDocValuesField, NumericValue, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values_field_updates::{merged_iterator, DocValuesFieldUpdates};
use rucene::index::field_infos::FieldInfo;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::terms::Term;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexReader, PendingSoftDeletes,
    SegmentCommitInfo, VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::java_collections::{JavaHashMap, JavaStringMap};
use rucene::util::Bits;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};
use crate::support::npe;
use crate::test_pending_deletes::{self as base, AnyPendingDeletes};

/// `TestPendingSoftDeletes`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestPendingSoftDeletes")
        .extends(&base_class())
        .constructor(randomizedtesting::jdk::reflect::Constructor::no_args(|| ()))
        .method(test(
            "testHardDeleteSoftDeleted",
            test_hard_delete_soft_deleted,
        ))
        .method(test("testDeleteSoft", test_delete_soft))
        .method(test("testApplyUpdates", test_apply_updates))
        .method(test(
            "testUpdateAppliedOnlyOnce",
            test_update_applied_only_once,
        ))
        .method(test("testResetOnUpdate", test_reset_on_update))
        .build()
}

/// `TestPendingDeletes` as the superclass: its tests over this class's
/// `newPendingDeletes`.
fn base_class() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestPendingDeletes",
        vec![
            test("testDeleteDoc", || {
                base::test_delete_doc(&new_any_pending_deletes)
            }),
            test("testWriteLiveDocs", || {
                base::test_write_live_docs(&new_any_pending_deletes)
            }),
            test("testIsFullyDeleted", || {
                base::test_is_fully_deleted(&new_any_pending_deletes)
            }),
        ],
    )
}

/// `newPendingDeletes(commitInfo)`, the override.
fn new_pending_deletes(commit_info: &SegmentCommitInfo) -> PendingSoftDeletes {
    PendingSoftDeletes::new("_soft_deletes", commit_info)
}

/// The override, as the superclass's factory sees it.
fn new_any_pending_deletes(commit_info: &SegmentCommitInfo) -> AnyPendingDeletes {
    AnyPendingDeletes::Soft(new_pending_deletes(commit_info))
}

/// The writer of the soft-delete tests: `setSoftDeletesField("_soft_deletes")`,
/// `setMaxBufferedDocs(max_buffered_docs)`, `NoMergePolicy.INSTANCE` and
/// `setRAMBufferSizeMB(DISABLE_AUTO_FLUSH)`.
fn soft_writer(dir: &Arc<dyn Directory>, max_buffered_docs: i32) -> Result<Arc<IndexWriter>> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_soft_deletes_field(Some("_soft_deletes".to_string()));
    iwc.set_max_buffered_docs(max_buffered_docs)?;
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    iwc.set_ram_buffer_size_mb(f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH))?;
    IndexWriter::new(Arc::clone(dir), iwc)
}

/// The three soft updates every writer test starts with.
fn soft_update_three(writer: &IndexWriter) -> Result<()> {
    for id in ["1", "2", "2"] {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            id.to_string(),
            Store::YES,
        )?));
        writer.soft_update_document(
            Term::from_text("id", id),
            &doc,
            // `new NumericDocValuesField("_soft_deletes", 1)`.
            &[Field::new_with_number(
                "_soft_deletes",
                NumericValue::Long(1),
                NumericDocValuesField::type_().clone(),
            )?],
        )?;
    }
    Ok(())
}

/// `(SegmentReader) reader.leaves().get(0).reader()` and its segment.
fn only_segment(
    reader: &Arc<dyn DirectoryReader>,
) -> Result<(Arc<dyn rucene::index::LeafReader>, SegmentCommitInfo)> {
    let leaves = (Arc::clone(reader) as Arc<dyn IndexReader>).leaves()?;
    assert_equals(&1, &leaves.len());
    let leaf = leaves[0].leaf_reader();
    let info = segment_reader(&leaf).get_segment_info().clone();
    Ok((leaf, info))
}

/// `(SegmentReader) leaf`.
fn segment_reader(leaf: &Arc<dyn rucene::index::LeafReader>) -> &SegmentReader {
    leaf.as_segment_reader().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            "not a SegmentReader",
        )
        .throw()
    })
}

/// `bits.get(doc)` of non-null bits.
fn get(bits: Option<Box<dyn Bits>>, doc: usize) -> bool {
    bits.unwrap_or_else(|| npe("liveDocs")).get(doc)
}

/// The identity of non-null bits, for `assertSame`.
fn identity(bits: &Option<Box<dyn Bits>>) -> *const () {
    bits.as_ref().unwrap_or_else(|| npe("liveDocs")).identity()
}

/// `IOUtils.close(reader, writer, dir)`.
fn close_all(
    reader: Arc<dyn DirectoryReader>,
    writer: Arc<IndexWriter>,
    dir: &mut rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<()> {
    let outcomes = [reader.close(), writer.close()];
    drop((reader, writer));
    let closed = dir.close();
    for outcome in outcomes {
        outcome?;
    }
    closed
}

/// Port of `TestPendingSoftDeletes.testHardDeleteSoftDeleted`.
fn test_hard_delete_soft_deleted() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = soft_writer(&dir.as_directory(), 10)?;
    soft_update_three(&writer)?;
    writer.commit()?;
    let reader = directory_reader::open(dir.as_directory())?;
    let (leaf, segment_info) = only_segment(&reader)?;
    let segment_reader = segment_reader(&leaf);
    let mut pending_soft_deletes = new_pending_deletes(&segment_info);
    pending_soft_deletes.on_new_reader(segment_reader, &segment_info)?;
    assert_equals(&0, &pending_soft_deletes.num_pending_deletes());
    assert_equals(&1, &pending_soft_deletes.get_del_count(&segment_info));
    Assert::assert_true(get(pending_soft_deletes.get_live_docs(), 0));
    Assert::assert_false(get(pending_soft_deletes.get_live_docs(), 1));
    Assert::assert_true(get(pending_soft_deletes.get_live_docs(), 2));
    Assert::assert_true(pending_soft_deletes.get_hard_live_docs().is_none());
    Assert::assert_true(pending_soft_deletes.delete(1, &segment_info)?);
    assert_equals(&0, &pending_soft_deletes.num_pending_deletes());
    assert_equals(&-1, &pending_soft_deletes.pending_delete_count()); // transferred the delete
    assert_equals(&1, &pending_soft_deletes.get_del_count(&segment_info));
    drop(leaf);
    close_all(reader, writer, &mut dir)
}

/// Port of `TestPendingSoftDeletes.testDeleteSoft`.
fn test_delete_soft() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = soft_writer(&dir.as_directory(), 10)?;
    soft_update_three(&writer)?;
    writer.commit()?;
    let reader = directory_reader::open(dir.as_directory())?;
    let (leaf, segment_info) = only_segment(&reader)?;
    let mut pending_soft_deletes = new_pending_deletes(&segment_info);
    pending_soft_deletes.on_new_reader(segment_reader(&leaf), &segment_info)?;
    assert_equals(&0, &pending_soft_deletes.num_pending_deletes());
    assert_equals(&1, &pending_soft_deletes.get_del_count(&segment_info));
    Assert::assert_true(get(pending_soft_deletes.get_live_docs(), 0));
    Assert::assert_false(get(pending_soft_deletes.get_live_docs(), 1));
    Assert::assert_true(get(pending_soft_deletes.get_live_docs(), 2));
    Assert::assert_true(pending_soft_deletes.get_hard_live_docs().is_none());
    // pass reader again
    let live_docs = pending_soft_deletes.get_live_docs();
    pending_soft_deletes.on_new_reader(segment_reader(&leaf), &segment_info)?;
    assert_equals(&0, &pending_soft_deletes.num_pending_deletes());
    assert_equals(&1, &pending_soft_deletes.get_del_count(&segment_info));
    Assert::assert_true(identity(&live_docs) == identity(&pending_soft_deletes.get_live_docs()));

    // now apply a hard delete
    writer.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
    writer.commit()?;
    drop(leaf);
    reader.close()?;
    let reader = directory_reader::open(dir.as_directory())?;
    let (leaf, segment_info) = only_segment(&reader)?;
    let mut pending_soft_deletes = new_pending_deletes(&segment_info);
    pending_soft_deletes.on_new_reader(segment_reader(&leaf), &segment_info)?;
    assert_equals(&0, &pending_soft_deletes.num_pending_deletes());
    assert_equals(&2, &pending_soft_deletes.get_del_count(&segment_info));
    Assert::assert_false(get(pending_soft_deletes.get_live_docs(), 0));
    Assert::assert_false(get(pending_soft_deletes.get_live_docs(), 1));
    Assert::assert_true(get(pending_soft_deletes.get_live_docs(), 2));
    Assert::assert_true(pending_soft_deletes.get_hard_live_docs().is_some());
    Assert::assert_false(get(pending_soft_deletes.get_hard_live_docs(), 0));
    Assert::assert_true(get(pending_soft_deletes.get_hard_live_docs(), 1));
    Assert::assert_true(get(pending_soft_deletes.get_hard_live_docs(), 2));
    drop(leaf);
    close_all(reader, writer, &mut dir)
}

/// `new FieldInfo("_soft_deletes", 1, false, false, false, IndexOptions.NONE,
/// DocValuesType.NUMERIC, DocValuesSkipIndexType.NONE, dv_gen, Collections.emptyMap(), 0, 0, 0,
/// 0, VectorEncoding.FLOAT32, VectorSimilarityFunction.EUCLIDEAN, true, false)`.
fn soft_deletes_field_info(dv_gen: i64) -> Result<FieldInfo> {
    FieldInfo::new_full(
        "_soft_deletes",
        1,
        false,
        false,
        false,
        IndexOptions::NONE,
        DocValuesType::NUMERIC,
        DocValuesSkipIndexType::NONE,
        dv_gen,
        JavaStringMap::Hash(JavaHashMap::new()),
        0,
        0,
        0,
        0,
        VectorEncoding::FLOAT32,
        VectorSimilarityFunction::EUCLIDEAN,
        true,
        false,
    )
}

/// `deletes.onDocValuesUpdate(fieldInfo, update.iterator())` for each update.
fn apply(
    deletes: &mut PendingSoftDeletes,
    field_info: &FieldInfo,
    updates: &[DocValuesFieldUpdates],
    info: &mut SegmentCommitInfo,
) -> Result<()> {
    for update in updates {
        let mut iterator = merged_iterator(vec![update.iterator()]);
        deletes.on_doc_values_update(field_info, &mut iterator, info)?;
    }
    Ok(())
}

/// Port of `TestPendingSoftDeletes.testApplyUpdates`.
fn test_apply_updates() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let si = base::test_segment_info(&dir, 10)?;
    let mut commit_info = base::test_commit_info(si)?;
    let writer = IndexWriter::new(Arc::clone(&dir), LuceneTestCase::new_index_writer_config())?;
    for _ in 0..commit_info.info.max_doc()? {
        writer.add_document(&Document::new())?;
    }
    writer.force_merge(1)?;
    writer.commit()?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    assert_equals(&1, &leaves.len());
    let leaf = leaves[0].leaf_reader();
    let mut deletes = new_pending_deletes(&commit_info);
    deletes.on_new_reader(segment_reader(&leaf), &commit_info)?;
    drop((leaf, leaves));
    reader.close()?;
    writer.close()?;
    let field_info = soft_deletes_field_info(0)?;
    let docs_deleted = [1, 3, 7, 8, NO_MORE_DOCS];
    let updates = [single_update(&docs_deleted, 10, true)?];
    apply(&mut deletes, &field_info, &updates, &mut commit_info)?;
    assert_equals(&0, &deletes.num_pending_deletes());
    assert_equals(&4, &deletes.get_del_count(&commit_info));
    let expected = [
        true, false, true, false, true, true, true, false, false, true,
    ];
    for (doc, &live) in expected.iter().enumerate() {
        assert_equals(&live, &get(deletes.get_live_docs(), doc));
    }

    let docs_deleted = [1, 2, NO_MORE_DOCS];
    let updates = [single_update(&docs_deleted, 10, true)?];
    let field_info = soft_deletes_field_info(1)?;
    apply(&mut deletes, &field_info, &updates, &mut commit_info)?;
    assert_equals(&0, &deletes.num_pending_deletes());
    assert_equals(&5, &deletes.get_del_count(&commit_info));
    let expected = [
        true, false, false, false, true, true, true, false, false, true,
    ];
    for (doc, &live) in expected.iter().enumerate() {
        assert_equals(&live, &get(deletes.get_live_docs(), doc));
    }
    Ok(())
}

/// Port of `TestPendingSoftDeletes.testUpdateAppliedOnlyOnce`.
fn test_update_applied_only_once() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = soft_writer(&dir.as_directory(), 3)?; // make sure we write one segment
    soft_update_three(&writer)?;
    writer.commit()?;
    let reader = directory_reader::open(dir.as_directory())?;
    let (leaf, mut segment_info) = only_segment(&reader)?;
    let mut deletes = new_pending_deletes(&segment_info);
    deletes.on_new_reader(segment_reader(&leaf), &segment_info)?;
    let field_info = soft_deletes_field_info(segment_info.get_next_doc_values_gen())?;
    let docs_deleted = [1, NO_MORE_DOCS];
    let updates = [single_update(&docs_deleted, 3, true)?];
    apply(&mut deletes, &field_info, &updates, &mut segment_info)?;
    assert_equals(&0, &deletes.num_pending_deletes());
    assert_equals(&1, &deletes.get_del_count(&segment_info));
    Assert::assert_true(get(deletes.get_live_docs(), 0));
    Assert::assert_false(get(deletes.get_live_docs(), 1));
    Assert::assert_true(get(deletes.get_live_docs(), 2));
    let live_docs = deletes.get_live_docs();
    deletes.on_new_reader(segment_reader(&leaf), &segment_info)?;
    // no changes we don't apply updates twice
    Assert::assert_true(identity(&live_docs) == identity(&deletes.get_live_docs()));
    Assert::assert_true(get(deletes.get_live_docs(), 0));
    Assert::assert_false(get(deletes.get_live_docs(), 1));
    Assert::assert_true(get(deletes.get_live_docs(), 2));
    assert_equals(&0, &deletes.num_pending_deletes());
    assert_equals(&1, &deletes.get_del_count(&segment_info));
    drop(leaf);
    close_all(reader, writer, &mut dir)
}

/// Port of `TestPendingSoftDeletes.testResetOnUpdate`.
fn test_reset_on_update() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = soft_writer(&dir.as_directory(), 3)?; // make sure we write one segment
    soft_update_three(&writer)?;
    writer.commit()?;
    let reader = directory_reader::open(dir.as_directory())?;
    let (leaf, mut segment_info) = only_segment(&reader)?;
    let mut deletes = new_pending_deletes(&segment_info);
    deletes.on_new_reader(segment_reader(&leaf), &segment_info)?;
    let field_info = soft_deletes_field_info(segment_info.get_next_doc_values_gen())?;
    let updates = [single_update(&[0, 1, NO_MORE_DOCS], 3, false)?];
    apply(&mut deletes, &field_info, &updates, &mut segment_info)?;
    assert_equals(&0, &deletes.num_pending_deletes());
    Assert::assert_true(get(deletes.get_live_docs(), 0));
    Assert::assert_true(get(deletes.get_live_docs(), 1));
    Assert::assert_true(get(deletes.get_live_docs(), 2));
    let live_docs = deletes.get_live_docs();
    deletes.on_new_reader(segment_reader(&leaf), &segment_info)?;
    // no changes we keep this update
    Assert::assert_true(identity(&live_docs) == identity(&deletes.get_live_docs()));
    Assert::assert_true(get(deletes.get_live_docs(), 0));
    Assert::assert_true(get(deletes.get_live_docs(), 1));
    Assert::assert_true(get(deletes.get_live_docs(), 2));
    assert_equals(&0, &deletes.num_pending_deletes());

    segment_info.advance_doc_values_gen();
    let field_info = soft_deletes_field_info(segment_info.get_next_doc_values_gen())?;
    let updates = [single_update(&[1, NO_MORE_DOCS], 3, true)?];
    apply(&mut deletes, &field_info, &updates, &mut segment_info)?;
    // no changes we keep this update
    Assert::assert_true(identity(&live_docs) != identity(&deletes.get_live_docs()));
    Assert::assert_true(get(deletes.get_live_docs(), 0));
    Assert::assert_false(get(deletes.get_live_docs(), 1));
    Assert::assert_true(get(deletes.get_live_docs(), 2));
    assert_equals(&0, &deletes.num_pending_deletes());
    assert_equals(&1, &deletes.get_del_count(&segment_info));
    drop(leaf);
    close_all(reader, writer, &mut dir)
}

/// `private DocValuesFieldUpdates singleUpdate(List<Integer> docsChanged, int maxDoc, boolean
/// hasValue)`: the documents of `docs_changed` up to `NO_MORE_DOCS`, with the value `1` or
/// reset.
fn single_update(
    docs_changed: &[i32],
    max_doc: i32,
    has_value: bool,
) -> Result<DocValuesFieldUpdates> {
    let mut updates =
        DocValuesFieldUpdates::new(max_doc, 0, "_soft_deletes", DocValuesType::NUMERIC);
    for &doc in docs_changed {
        if doc == NO_MORE_DOCS {
            break;
        }
        if has_value {
            updates.add_long(doc, 1);
        } else {
            updates.reset(doc);
        }
    }
    updates.finish()?;
    Ok(updates)
}
