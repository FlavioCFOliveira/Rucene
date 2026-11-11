//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSizeBoundedForceMerge.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig};
use rucene::index::merge_policy::{LogMergePolicy, NoMergePolicy};
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, test};

/// `TestSizeBoundedForceMerge`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSizeBoundedForceMerge",
        vec![
            test("testByteSizeLimit", test_byte_size_limit),
            test("testNumDocsLimit", test_num_docs_limit),
            test("testLastSegmentTooLarge", test_last_segment_too_large),
            test("testFirstSegmentTooLarge", test_first_segment_too_large),
            test("testAllSegmentsSmall", test_all_segments_small),
            test("testAllSegmentsLarge", test_all_segments_large),
            test("testOneLargeOneSmall", test_one_large_one_small),
            test("testMergeFactor", test_merge_factor),
            test("testSingleMergeableSegment", test_single_mergeable_segment),
            test(
                "testSingleNonMergeableSegment",
                test_single_non_mergeable_segment,
            ),
            test(
                "testSingleMergeableTooLargeSegment",
                test_single_mergeable_too_large_segment,
            ),
        ],
    )
}

/// `private void addDocs(IndexWriter writer, int numDocs)`.
fn add_docs(writer: &IndexWriter, num_docs: i32) -> Result<()> {
    add_docs_with_id(writer, num_docs, false)
}

/// `private void addDocs(IndexWriter writer, int numDocs, boolean withID)`.
fn add_docs_with_id(writer: &IndexWriter, num_docs: i32, with_id: bool) -> Result<()> {
    for i in 0..num_docs {
        let mut doc = Document::new();
        if with_id {
            doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
        }
        writer.add_document(&doc)?;
    }
    writer.commit()?;
    Ok(())
}

/// `private static IndexWriterConfig newWriterConfig()`.
fn new_writer_config() -> Result<IndexWriterConfig> {
    let conf = LuceneTestCase::new_index_writer_config_analyzer(None);
    // Force using the default codec and not e.g. SimpleText which has a non-deterministic byte size
    // of its segment info due to escape characters.
    conf.set_codec(TestUtil::get_default_codec())?;
    conf.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB)?;
    // don't use compound files, because the overhead make size checks unreliable.
    conf.set_use_compound_file(false)?;
    // prevent any merges by default.
    conf.set_merge_policy(Some(NoMergePolicy::instance()))?;
    Ok(conf)
}

/// `new IndexWriter(dir, newWriterConfig())`, adding each of `num_docs` as a
/// commit (`addDocs(writer, n)`), then `writer.close()`.
fn prepare(dir: &Arc<dyn Directory>, num_docs: &[i32]) -> Result<()> {
    let conf = new_writer_config()?;
    let writer = IndexWriter::new(Arc::clone(dir), conf)?;
    for &n in num_docs {
        add_docs(&writer, n)?;
    }
    writer.close()
}

/// `conf = newWriterConfig(); LogMergePolicy lmp = new LogDocMergePolicy();
/// lmp.setMaxMergeDocs(maxMergeDocs); [lmp.setMergeFactor(mergeFactor);]
/// conf.setMergePolicy(lmp); writer = new IndexWriter(dir, conf);
/// writer.forceMerge(1); writer.close();`
fn force_merge_doc_limited(
    dir: &Arc<dyn Directory>,
    max_merge_docs: i32,
    merge_factor: Option<i32>,
) -> Result<()> {
    let conf = new_writer_config()?;
    let lmp = LogMergePolicy::by_doc_count();
    lmp.set_max_merge_docs(max_merge_docs);
    if let Some(merge_factor) = merge_factor {
        lmp.set_merge_factor(merge_factor)?;
    }
    conf.set_merge_policy(Some(Arc::new(lmp)))?;

    let writer = IndexWriter::new(Arc::clone(dir), conf)?;
    writer.force_merge(1)?;
    writer.close()
}

/// `new ByteBuffersDirectory()`.
fn new_dir() -> Arc<dyn Directory> {
    Arc::new(ByteBuffersDirectory::new())
}

/// Port of `TestSizeBoundedForceMerge.testByteSizeLimit`.
fn test_byte_size_limit() -> Result<()> {
    // tests that the max merge size constraint is applied during forceMerge.
    let dir = new_dir();

    // Prepare an index w/ several small segments and a large one.
    let conf = new_writer_config()?;
    let writer = IndexWriter::new(Arc::clone(&dir), conf)?;
    let num_segments = 15;
    for i in 0..num_segments {
        let num_docs = if i == 7 { 30 } else { 1 };
        add_docs(&writer, num_docs)?;
    }
    writer.close()?;

    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    let mut number_of_segments_of_minimum_size = 1;
    let size0 = sis.info(0).clone().size_in_bytes()?;
    for i in 1..sis.size() {
        if sis.info(i).clone().size_in_bytes()? == size0 {
            number_of_segments_of_minimum_size += 1;
        }
    }
    assert_equals(&(num_segments - 1), &number_of_segments_of_minimum_size);

    let min = sis.info(0).clone().size_in_bytes()? as f64;

    let conf = new_writer_config()?;
    let mut lmp = LogMergePolicy::by_byte_size();
    lmp.set_max_merge_mb_for_forced_merge(min / (1 << 20) as f64);
    conf.set_merge_policy(Some(Arc::new(lmp)))?;

    let writer = IndexWriter::new(Arc::clone(&dir), conf)?;
    writer.force_merge(1)?;
    writer.close()?;

    // Should only be 3 segments in the index, because one of them exceeds the size limit
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&3, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testNumDocsLimit`.
fn test_num_docs_limit() -> Result<()> {
    // tests that the max merge docs constraint is applied during forceMerge.
    let dir = new_dir();

    // Prepare an index w/ several small segments and a large one.
    prepare(&dir, &[3, 3, 5, 3, 3, 3, 3])?;

    force_merge_doc_limited(&dir, 3, None)?;

    // Should only be 3 segments in the index, because one of them exceeds the size limit
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&3, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testLastSegmentTooLarge`.
fn test_last_segment_too_large() -> Result<()> {
    let dir = new_dir();
    prepare(&dir, &[3, 3, 3, 5])?;
    force_merge_doc_limited(&dir, 3, None)?;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&2, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testFirstSegmentTooLarge`.
fn test_first_segment_too_large() -> Result<()> {
    let dir = new_dir();
    prepare(&dir, &[5, 3, 3, 3])?;
    force_merge_doc_limited(&dir, 3, None)?;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&2, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testAllSegmentsSmall`.
fn test_all_segments_small() -> Result<()> {
    let dir = new_dir();
    prepare(&dir, &[3, 3, 3, 3])?;
    force_merge_doc_limited(&dir, 3, None)?;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&1, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testAllSegmentsLarge`.
fn test_all_segments_large() -> Result<()> {
    let dir = new_dir();
    prepare(&dir, &[3, 3, 3])?;
    force_merge_doc_limited(&dir, 2, None)?;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&3, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testOneLargeOneSmall`.
fn test_one_large_one_small() -> Result<()> {
    let dir = new_dir();
    prepare(&dir, &[3, 5, 3, 5])?;
    force_merge_doc_limited(&dir, 3, None)?;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&4, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testMergeFactor`.
fn test_merge_factor() -> Result<()> {
    let dir = new_dir();
    prepare(&dir, &[3, 3, 3, 3, 5, 3, 3])?;
    force_merge_doc_limited(&dir, 3, Some(2))?;
    // Should only be 4 segments in the index, because of the merge factor and
    // max merge docs settings.
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&4, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testSingleMergeableSegment`.
fn test_single_mergeable_segment() -> Result<()> {
    let dir = new_dir();

    let conf = new_writer_config()?;
    let writer = IndexWriter::new(Arc::clone(&dir), conf)?;

    add_docs(&writer, 3)?;
    add_docs(&writer, 5)?;
    add_docs(&writer, 3)?;

    // delete the last document, so that the last segment is merged.
    writer.delete_documents_by_terms(vec![Term::from_text("id", "10")])?;
    writer.close()?;

    force_merge_doc_limited(&dir, 3, None)?;

    // Verify that the last segment does not have deletions.
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&3, &sis.size());
    Assert::assert_false(sis.info(2).has_deletions());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testSingleNonMergeableSegment`.
fn test_single_non_mergeable_segment() -> Result<()> {
    let dir = new_dir();

    let conf = new_writer_config()?;
    let writer = IndexWriter::new(Arc::clone(&dir), conf)?;

    add_docs_with_id(&writer, 3, true)?;

    writer.close()?;

    force_merge_doc_limited(&dir, 3, None)?;

    // Verify that the last segment does not have deletions.
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&1, &sis.size());
    Ok(())
}

/// Port of `TestSizeBoundedForceMerge.testSingleMergeableTooLargeSegment`.
fn test_single_mergeable_too_large_segment() -> Result<()> {
    let dir = new_dir();

    let conf = new_writer_config()?;
    let writer = IndexWriter::new(Arc::clone(&dir), conf)?;

    add_docs_with_id(&writer, 5, true)?;

    // delete the last document

    writer.delete_documents_by_terms(vec![Term::from_text("id", "4")])?;
    writer.close()?;

    force_merge_doc_limited(&dir, 2, None)?;

    // Verify that the last segment does not have deletions.
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
    assert_equals(&1, &sis.size());
    Assert::assert_true(sis.info(0).has_deletions());
    Ok(())
}
