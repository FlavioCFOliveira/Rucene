//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPointValues.java`.
//!
//! Test Indexing/IndexWriter with points.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs;
use rucene::document::{
    BinaryPoint, Document, DoublePoint, FloatPoint, IntPoint, LongPoint, Store, StringField,
};
use rucene::error::Result;
use rucene::index::check_index::Level;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::point_values::{
    self, IntersectVisitor, Relation, MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
use rucene::index::terms::Term;
use rucene::index::{CodecReader, IndexReader, LeafReader, MultiReader};
use rucene::store::{ByteBuffersDirectory, Directory, FSDirectory};
use rucene::util::{BytesRef, IOUtils};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_array_equals, assert_equals, assert_null, class, expect_throws_panic, random, test, IAE,
};
use crate::support::npe;

pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestPointValues",
        vec![
            test("testUpgradeFieldToPoints", test_upgrade_field_to_points),
            test(
                "testIllegalDimChangeOneDoc",
                test_illegal_dim_change_one_doc,
            ),
            test(
                "testIllegalDimChangeTwoDocs",
                test_illegal_dim_change_two_docs,
            ),
            test(
                "testIllegalDimChangeTwoSegments",
                test_illegal_dim_change_two_segments,
            ),
            test(
                "testIllegalDimChangeTwoWriters",
                test_illegal_dim_change_two_writers,
            ),
            test(
                "testIllegalDimChangeViaAddIndexesDirectory",
                test_illegal_dim_change_via_add_indexes_directory,
            ),
            test(
                "testIllegalDimChangeViaAddIndexesCodecReader",
                test_illegal_dim_change_via_add_indexes_codec_reader,
            ),
            test(
                "testIllegalDimChangeViaAddIndexesSlowCodecReader",
                test_illegal_dim_change_via_add_indexes_slow_codec_reader,
            ),
            test(
                "testIllegalNumBytesChangeOneDoc",
                test_illegal_num_bytes_change_one_doc,
            ),
            test(
                "testIllegalNumBytesChangeTwoDocs",
                test_illegal_num_bytes_change_two_docs,
            ),
            test(
                "testIllegalNumBytesChangeTwoSegments",
                test_illegal_num_bytes_change_two_segments,
            ),
            test(
                "testIllegalNumBytesChangeTwoWriters",
                test_illegal_num_bytes_change_two_writers,
            ),
            test(
                "testIllegalNumBytesChangeViaAddIndexesDirectory",
                test_illegal_num_bytes_change_via_add_indexes_directory,
            ),
            test(
                "testIllegalNumBytesChangeViaAddIndexesCodecReader",
                test_illegal_num_bytes_change_via_add_indexes_codec_reader,
            ),
            test(
                "testIllegalNumBytesChangeViaAddIndexesSlowCodecReader",
                test_illegal_num_bytes_change_via_add_indexes_slow_codec_reader,
            ),
            test("testIllegalTooManyBytes", test_illegal_too_many_bytes),
            test(
                "testIllegalTooManyDimensions",
                test_illegal_too_many_dimensions,
            ),
            test("testDifferentCodecs1", test_different_codecs1),
            test("testDifferentCodecs2", test_different_codecs2),
            test("testInvalidIntPointUsage", test_invalid_int_point_usage),
            test("testInvalidLongPointUsage", test_invalid_long_point_usage),
            test("testInvalidFloatPointUsage", test_invalid_float_point_usage),
            test(
                "testInvalidDoublePointUsage",
                test_invalid_double_point_usage,
            ),
            test("testTieBreakByDocID", test_tie_break_by_doc_id),
            test("testDeleteAllPointDocs", test_delete_all_point_docs),
            test(
                "testPointsFieldMissingFromOneSegment",
                test_points_field_missing_from_one_segment,
            ),
            test("testSparsePoints", test_sparse_points),
            test(
                "testCheckIndexIncludesPoints",
                test_check_index_includes_points,
            ),
            test("testMergedStatsEmptyReader", test_merged_stats_empty_reader),
            test(
                "testMergedStatsOneSegmentWithoutPoints",
                test_merged_stats_one_segment_without_points,
            ),
            test(
                "testMergedStatsAllPointsDeleted",
                test_merged_stats_all_points_deleted,
            ),
            test("testMergedStats", test_merged_stats),
        ],
    )
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new BinaryPoint("dim", new byte[n0], new byte[n1], ...)`.
fn bp(dims: &[usize]) -> Result<BinaryPoint> {
    let values: Vec<BytesRef> = dims.iter().map(|&n| BytesRef::new(vec![0u8; n])).collect();
    BinaryPoint::new("dim", &values)
}

/// `assertEquals(message, expected.getMessage())`.
fn assert_message(message: &str, expected: &Throwable) {
    assert_equals(&Some(message), &expected.message());
}

/// `(CodecReader) leaf` (a `ClassCastException` when it is not one).
fn as_codec_reader(leaf: Arc<dyn LeafReader>) -> Arc<dyn CodecReader> {
    leaf.as_codec_reader().unwrap_or_else(|| {
        Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
    })
}

/// Suddenly add points to an existing field.
///
/// Port of `TestPointValues.testUpgradeFieldToPoints`.
fn test_upgrade_field_to_points() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "dim",
        "foo",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.close()?;

    let iwc = LuceneTestCase::new_index_writer_config();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    doc.add(Box::new(bp(&[4])?));
    w.close()?;
    dir.close()
}

// Illegal schema change tests:

/// Port of `TestPointValues.testIllegalDimChangeOneDoc`.
fn test_illegal_dim_change_one_doc() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    doc.add(Box::new(bp(&[4, 4])?));
    let expected = expect_throws(IAE, || w.add_document(&doc));
    assert_message("Inconsistency of field data structures across documents for field [dim] of doc [0]. point dimension: expected '1', but it has '2'.", &expected);
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalDimChangeTwoDocs`.
fn test_illegal_dim_change_two_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;

    let mut doc2 = Document::new();
    doc2.add(Box::new(bp(&[4, 4])?));
    let expected = expect_throws(IAE, || w.add_document(&doc2));
    assert_message("Inconsistency of field data structures across documents for field [dim] of doc [1]. point dimension: expected '1', but it has '2'.", &expected);
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalDimChangeTwoSegments`.
fn test_illegal_dim_change_two_segments() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.commit()?;

    let mut doc2 = Document::new();
    doc2.add(Box::new(bp(&[4, 4])?));
    let expected = expect_throws(IAE, || w.add_document(&doc2));
    assert_message("cannot change field \"dim\" from points dimensionCount=1, indexDimensionCount=1, numBytes=4 to inconsistent dimensionCount=2, indexDimensionCount=2, numBytes=4", &expected);
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalDimChangeTwoWriters`.
fn test_illegal_dim_change_two_writers() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;
    let iwc = mock_iwc();

    let w2 = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc2 = Document::new();
    doc2.add(Box::new(bp(&[4, 4])?));
    let expected = expect_throws(IAE, || w2.add_document(&doc2));
    assert_message("cannot change field \"dim\" from points dimensionCount=1, indexDimensionCount=1, numBytes=4 to inconsistent dimensionCount=2, indexDimensionCount=2, numBytes=4", &expected);
    w2.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalDimChangeViaAddIndexesDirectory`.
fn test_illegal_dim_change_via_add_indexes_directory() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4, 4])?));
    w2.add_document(&doc)?;
    let expected = expect_throws(IAE, || w2.add_indexes_dirs(&[dir.as_directory()]));

    assert_message("cannot change field \"dim\" from points dimensionCount=2, indexDimensionCount=2, numBytes=4 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=4", &expected);
    let (dir_h, dir2_h) = (&mut dir, &mut dir2);
    IOUtils::close([
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir_h.close()),
        IOUtils::closeable(|| dir2_h.close()),
    ])
}

/// Port of `TestPointValues.testIllegalDimChangeViaAddIndexesCodecReader`.
fn test_illegal_dim_change_via_add_indexes_codec_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4, 4])?));
    w2.add_document(&doc)?;
    let r = directory_reader::open(dir.as_directory())?;
    let expected = expect_throws(IAE, || {
        w2.add_indexes_codec_readers(&[as_codec_reader(LuceneTestCase::get_only_leaf_reader(
            Arc::clone(&r) as _,
        )?)])
    });
    assert_message("cannot change field \"dim\" from points dimensionCount=2, indexDimensionCount=2, numBytes=4 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=4", &expected);
    let (dir_h, dir2_h) = (&mut dir, &mut dir2);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir_h.close()),
        IOUtils::closeable(|| dir2_h.close()),
    ])
}

/// Port of `TestPointValues.testIllegalDimChangeViaAddIndexesSlowCodecReader`.
fn test_illegal_dim_change_via_add_indexes_slow_codec_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4, 4])?));
    w2.add_document(&doc)?;
    let r = directory_reader::open(dir.as_directory())?;
    let expected = expect_throws(IAE, || TestUtil::add_indexes_slowly(&w2, &[Arc::clone(&r)]));
    assert_message("cannot change field \"dim\" from points dimensionCount=2, indexDimensionCount=2, numBytes=4 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=4", &expected);
    let (dir_h, dir2_h) = (&mut dir, &mut dir2);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir_h.close()),
        IOUtils::closeable(|| dir2_h.close()),
    ])
}

/// Port of `TestPointValues.testIllegalNumBytesChangeOneDoc`.
fn test_illegal_num_bytes_change_one_doc() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    doc.add(Box::new(bp(&[6])?));
    let expected = expect_throws(IAE, || w.add_document(&doc));
    assert_message("Inconsistency of field data structures across documents for field [dim] of doc [0]. point num bytes: expected '4', but it has '6'.", &expected);
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalNumBytesChangeTwoDocs`.
fn test_illegal_num_bytes_change_two_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;

    let mut doc2 = Document::new();
    doc2.add(Box::new(bp(&[6])?));
    let expected = expect_throws(IAE, || w.add_document(&doc2));
    assert_message("Inconsistency of field data structures across documents for field [dim] of doc [1]. point num bytes: expected '4', but it has '6'.", &expected);
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalNumBytesChangeTwoSegments`.
fn test_illegal_num_bytes_change_two_segments() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.commit()?;

    let mut doc2 = Document::new();
    doc2.add(Box::new(bp(&[6])?));
    let expected = expect_throws(IAE, || w.add_document(&doc2));
    assert_message("cannot change field \"dim\" from points dimensionCount=1, indexDimensionCount=1, numBytes=4 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=6", &expected);
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalNumBytesChangeTwoWriters`.
fn test_illegal_num_bytes_change_two_writers() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;
    let iwc = mock_iwc();

    let w2 = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc2 = Document::new();
    doc2.add(Box::new(bp(&[6])?));
    let expected = expect_throws(IAE, || w2.add_document(&doc2));
    assert_message("cannot change field \"dim\" from points dimensionCount=1, indexDimensionCount=1, numBytes=4 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=6", &expected);
    w2.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalNumBytesChangeViaAddIndexesDirectory`.
fn test_illegal_num_bytes_change_via_add_indexes_directory() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[6])?));
    w2.add_document(&doc)?;
    let expected = expect_throws(IAE, || w2.add_indexes_dirs(&[dir.as_directory()]));

    assert_message("cannot change field \"dim\" from points dimensionCount=1, indexDimensionCount=1, numBytes=6 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=4", &expected);
    let (dir_h, dir2_h) = (&mut dir, &mut dir2);
    IOUtils::close([
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir_h.close()),
        IOUtils::closeable(|| dir2_h.close()),
    ])
}

/// Port of `TestPointValues.testIllegalNumBytesChangeViaAddIndexesCodecReader`.
fn test_illegal_num_bytes_change_via_add_indexes_codec_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[6])?));
    w2.add_document(&doc)?;
    let r = directory_reader::open(dir.as_directory())?;
    let expected = expect_throws(IAE, || {
        w2.add_indexes_codec_readers(&[as_codec_reader(LuceneTestCase::get_only_leaf_reader(
            Arc::clone(&r) as _,
        )?)])
    });
    assert_message("cannot change field \"dim\" from points dimensionCount=1, indexDimensionCount=1, numBytes=6 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=4", &expected);
    let (dir_h, dir2_h) = (&mut dir, &mut dir2);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir_h.close()),
        IOUtils::closeable(|| dir2_h.close()),
    ])
}

/// Port of `TestPointValues.testIllegalNumBytesChangeViaAddIndexesSlowCodecReader`.
fn test_illegal_num_bytes_change_via_add_indexes_slow_codec_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[4])?));
    w.add_document(&doc)?;
    w.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(bp(&[6])?));
    w2.add_document(&doc)?;
    let r = directory_reader::open(dir.as_directory())?;
    let expected = expect_throws(IAE, || TestUtil::add_indexes_slowly(&w2, &[Arc::clone(&r)]));
    assert_message("cannot change field \"dim\" from points dimensionCount=1, indexDimensionCount=1, numBytes=6 to inconsistent dimensionCount=1, indexDimensionCount=1, numBytes=4", &expected);
    let (dir_h, dir2_h) = (&mut dir, &mut dir2);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir_h.close()),
        IOUtils::closeable(|| dir2_h.close()),
    ])
}

/// Port of `TestPointValues.testIllegalTooManyBytes`.
fn test_illegal_too_many_bytes() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    expect_throws(IAE, || -> Result<()> {
        doc.add(Box::new(bp(&[MAX_NUM_BYTES as usize + 1])?));
        Ok(())
    });

    let mut doc2 = Document::new();
    doc2.add(Box::new(IntPoint::new("dim", &[17])?));
    w.add_document(&doc2)?;
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testIllegalTooManyDimensions`.
fn test_illegal_too_many_dimensions() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let values = vec![4usize; MAX_INDEX_DIMENSIONS as usize + 1];
    expect_throws(IAE, || -> Result<()> {
        doc.add(Box::new(bp(&values)?));
        Ok(())
    });

    let mut doc2 = Document::new();
    doc2.add(Box::new(IntPoint::new("dim", &[17])?));
    w.add_document(&doc2)?;
    w.close()?;
    dir.close()
}

/// `Codec.forName(name)`.
fn codec_for_name(name: &str) -> Arc<dyn rucene::codecs::Codec> {
    codecs::for_name(name).unwrap_or_else(|| {
        Throwable::new(
            IAE,
            format!("An SPI class of type org.apache.lucene.codecs.Codec with name '{name}' does not exist."),
        )
        .throw()
    })
}

/// Write point values, one segment with Lucene84, another with SimpleText,
/// then forceMerge with SimpleText.
///
/// Port of `TestPointValues.testDifferentCodecs1`.
fn test_different_codecs1() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_codec(TestUtil::get_default_codec())?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[1])?));
    w.add_document(&doc)?;
    w.close()?;

    let iwc = mock_iwc();
    iwc.set_codec(codec_for_name("SimpleText"))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[1])?));
    w.add_document(&doc)?;

    w.force_merge(1)?;
    w.close()?;
    dir.close()
}

/// Write point values, one segment with Lucene84, another with SimpleText,
/// then forceMerge with Lucene84.
///
/// Port of `TestPointValues.testDifferentCodecs2`.
fn test_different_codecs2() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_codec(codec_for_name("SimpleText"))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[1])?));
    w.add_document(&doc)?;
    w.close()?;

    let iwc = mock_iwc();
    iwc.set_codec(TestUtil::get_default_codec())?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[1])?));
    w.add_document(&doc)?;

    w.force_merge(1)?;
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testInvalidIntPointUsage`.
fn test_invalid_int_point_usage() -> Result<()> {
    let field = IntPoint::new("field", &[17, 42])?;

    expect_throws(IAE, || field.set_int_value(14));

    expect_throws_panic("java.lang.IllegalStateException", || {
        rucene::index::IndexableField::numeric_value(&field);
    });
    Ok(())
}

/// Port of `TestPointValues.testInvalidLongPointUsage`.
fn test_invalid_long_point_usage() -> Result<()> {
    let field = LongPoint::new("field", &[17, 42])?;

    expect_throws(IAE, || field.set_long_value(14));

    expect_throws_panic("java.lang.IllegalStateException", || {
        rucene::index::IndexableField::numeric_value(&field);
    });
    Ok(())
}

/// Port of `TestPointValues.testInvalidFloatPointUsage`.
fn test_invalid_float_point_usage() -> Result<()> {
    let field = FloatPoint::new("field", &[17.0, 42.0])?;

    expect_throws(IAE, || field.set_float_value(14.0));

    expect_throws_panic("java.lang.IllegalStateException", || {
        rucene::index::IndexableField::numeric_value(&field);
    });
    Ok(())
}

/// Port of `TestPointValues.testInvalidDoublePointUsage`.
fn test_invalid_double_point_usage() -> Result<()> {
    let field = DoublePoint::new("field", &[17.0, 42.0])?;

    expect_throws(IAE, || field.set_double_value(14.0));

    expect_throws_panic("java.lang.IllegalStateException", || {
        rucene::index::IndexableField::numeric_value(&field);
    });
    Ok(())
}

/// The anonymous `IntersectVisitor` of `testTieBreakByDocID`.
struct TieBreakVisitor {
    last_doc_id: i32,
}

impl IntersectVisitor for TieBreakVisitor {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        if doc_id < self.last_doc_id {
            Assert::fail(&format!(
                "docs out of order: docID={doc_id} but lastDocID={}",
                self.last_doc_id
            ));
        }
        self.last_doc_id = doc_id;
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        self.visit(doc_id)
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        if random().next_boolean() {
            Ok(Relation::CellCrossesQuery)
        } else {
            Ok(Relation::CellInsideQuery)
        }
    }
}

/// Port of `TestPointValues.testTieBreakByDocID`.
fn test_tie_break_by_doc_id() -> Result<()> {
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let iwc = LuceneTestCase::new_index_writer_config();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[17])?));
    let num_docs = if *TEST_NIGHTLY { 300000 } else { 3000 };
    for _i in 0..num_docs {
        w.add_document(&doc)?;
        if random().next_int_bound(1000) == 17 {
            w.commit()?;
        }
    }

    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;

    for ctx in Arc::clone(&r).leaves()? {
        let points = ctx
            .leaf_reader()
            .get_point_values("int")?
            .unwrap_or_else(|| npe("getPointValues"));
        points.intersect(&mut TieBreakVisitor { last_doc_id: -1 })?;
    }

    r.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testDeleteAllPointDocs`.
fn test_delete_all_point_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "0".into(), Store::NO)?));
    doc.add(Box::new(IntPoint::new("int", &[17])?));
    w.add_document(&doc)?;
    w.add_document(&Document::new())?;
    w.commit()?;

    w.delete_documents_by_terms(vec![Term::from_text("id", "0")])?;

    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    assert_null(
        &Arc::clone(&r).leaves()?[0]
            .leaf_reader()
            .get_point_values("int")?
            .map(|_| "PointValues"),
    );
    w.close()?;
    r.close()?;
    dir.close()
}

/// Port of `TestPointValues.testPointsFieldMissingFromOneSegment`.
fn test_points_field_missing_from_one_segment() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(LuceneTestCase::create_temp_dir())?);
    let iwc = IndexWriterConfig::with_analyzer(None);
    let w = IndexWriter::new(Arc::clone(&dir), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "0".into(), Store::NO)?));
    doc.add(Box::new(IntPoint::new("int0", &[0])?));
    w.add_document(&doc)?;
    w.commit()?;

    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int1", &[17])?));
    w.add_document(&doc)?;
    w.force_merge(1)?;

    w.close()?;
    drop(w);
    close_shared(dir)
}

/// Port of `TestPointValues.testSparsePoints`.
fn test_sparse_points() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let num_docs = LuceneTestCase::at_least(1000);
    let num_fields = TestUtil::next_int(&*random(), 1, 10) as usize;
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut field_doc_counts = vec![0i32; num_fields];
    let mut field_sizes = vec![0i64; num_fields];
    for _i in 0..num_docs {
        let mut doc = Document::new();
        for field in 0..num_fields {
            let field_name = format!("int{field}");
            if random().next_int_bound(100) == 17 {
                doc.add(Box::new(IntPoint::new(
                    &field_name,
                    &[random().next_int()],
                )?));
                field_doc_counts[field] += 1;
                field_sizes[field] += 1;

                if random().next_int_bound(10) == 5 {
                    // add same field again!
                    doc.add(Box::new(IntPoint::new(
                        &field_name,
                        &[random().next_int()],
                    )?));
                    field_sizes[field] += 1;
                }
            }
        }
        w.add_document(&doc)?;
    }

    let r: Arc<dyn IndexReader> = w.get_reader()?;
    for field in 0..num_fields {
        let mut doc_count = 0;
        let mut size = 0;
        let field_name = format!("int{field}");
        for ctx in Arc::clone(&r).leaves()? {
            if let Some(points) = ctx.leaf_reader().get_point_values(&field_name)? {
                doc_count += points.doc_count()?;
                size += points.size()?;
            }
        }
        assert_equals(&field_doc_counts[field], &doc_count);
        assert_equals(&field_sizes[field], &size);
    }
    r.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestPointValues.testCheckIndexIncludesPoints`.
fn test_check_index_includes_points() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let w = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::with_analyzer(None))?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int1", &[17])?));
    w.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int1", &[44])?));
    doc.add(Box::new(IntPoint::new("int2", &[-17])?));
    w.add_document(&doc)?;
    w.close()?;

    let mut output: Vec<u8> = Vec::new();
    let status = TestUtil::check_index_with(
        Arc::clone(&dir),
        Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS,
        true,
        true,
        Some(&mut output),
    )?;
    assert_equals(&1, &status.segment_infos.len());
    let seg_status = &status.segment_infos[0];
    let points_status = seg_status
        .points_status
        .as_ref()
        .unwrap_or_else(|| npe("pointsStatus"));
    // total 3 point values were index:
    assert_equals(&3, &points_status.total_value_points);
    // ... across 2 fields:
    assert_equals(&2, &points_status.total_value_fields);

    // Make sure CheckIndex in fact declares that it is testing points!
    Assert::assert_true(String::from_utf8_lossy(&output).contains("test: points..."));
    drop(w);
    close_shared(dir)
}

/// `dir.close()` over a directory shared as an `Arc`.
fn close_shared(dir: Arc<dyn Directory>) -> Result<()> {
    dir.close()
}

/// Port of `TestPointValues.testMergedStatsEmptyReader`.
fn test_merged_stats_empty_reader() -> Result<()> {
    let reader: Arc<dyn IndexReader> = Arc::new(MultiReader::new(Vec::new(), true)?);
    assert_null(&point_values::min_packed_value(
        Arc::clone(&reader),
        "field",
    )?);
    assert_null(&point_values::max_packed_value(
        Arc::clone(&reader),
        "field",
    )?);
    assert_equals(&0, &point_values::doc_count(Arc::clone(&reader), "field")?);
    assert_equals(&0, &point_values::size(Arc::clone(&reader), "field")?);
    Ok(())
}

/// Port of `TestPointValues.testMergedStatsOneSegmentWithoutPoints`.
fn test_merged_stats_one_segment_without_points() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let iwc = IndexWriterConfig::with_analyzer(None);
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = IndexWriter::new(Arc::clone(&dir), iwc)?;
    w.add_document(&Document::new())?;
    directory_reader::open_with_writer(&*w)?.close()?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("field", &[i32::MIN])?));
    w.add_document(&doc)?;
    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;

    assert_array_equals(
        &[0u8; 4][..],
        &point_values::min_packed_value(Arc::clone(&reader), "field")?
            .unwrap_or_else(|| npe("min")),
    );
    assert_array_equals(
        &[0u8; 4][..],
        &point_values::max_packed_value(Arc::clone(&reader), "field")?
            .unwrap_or_else(|| npe("max")),
    );
    assert_equals(&1, &point_values::doc_count(Arc::clone(&reader), "field")?);
    assert_equals(&1, &point_values::size(Arc::clone(&reader), "field")?);

    assert_null(&point_values::min_packed_value(
        Arc::clone(&reader),
        "field2",
    )?);
    assert_null(&point_values::max_packed_value(
        Arc::clone(&reader),
        "field2",
    )?);
    assert_equals(&0, &point_values::doc_count(Arc::clone(&reader), "field2")?);
    assert_equals(&0, &point_values::size(Arc::clone(&reader), "field2")?);
    Ok(())
}

/// Port of `TestPointValues.testMergedStatsAllPointsDeleted`.
fn test_merged_stats_all_points_deleted() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let w = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::with_analyzer(None))?;
    w.add_document(&Document::new())?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("field", &[i32::MIN])?));
    doc.add(Box::new(StringField::new(
        "delete",
        "yes".into(),
        Store::NO,
    )?));
    w.add_document(&doc)?;
    w.force_merge(1)?;
    w.delete_documents_by_terms(vec![Term::from_text("delete", "yes")])?;
    w.add_document(&Document::new())?;
    w.force_merge(1)?;
    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;

    assert_null(&point_values::min_packed_value(
        Arc::clone(&reader),
        "field",
    )?);
    assert_null(&point_values::max_packed_value(
        Arc::clone(&reader),
        "field",
    )?);
    assert_equals(&0, &point_values::doc_count(Arc::clone(&reader), "field")?);
    assert_equals(&0, &point_values::size(Arc::clone(&reader), "field")?);
    Ok(())
}

/// Port of `TestPointValues.testMergedStats`.
fn test_merged_stats() -> Result<()> {
    let iters = LuceneTestCase::at_least(3);
    for _iter in 0..iters {
        do_test_merged_stats()?;
    }
    Ok(())
}

/// `private static byte[][] randomBinaryValue(int numDims, int numBytesPerDim)`.
fn random_binary_value(num_dims: usize, num_bytes_per_dim: usize) -> Vec<BytesRef> {
    let mut bytes = Vec::with_capacity(num_dims);
    for _i in 0..num_dims {
        let mut b = vec![0u8; num_bytes_per_dim];
        random().next_bytes(&mut b);
        bytes.push(BytesRef::new(b));
    }
    bytes
}

/// `private void doTestMergedStats()`.
fn do_test_merged_stats() -> Result<()> {
    let num_dims = TestUtil::next_int(&*random(), 1, 8) as usize;
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 1, 16) as usize;
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let w = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::with_analyzer(None))?;
    let num_docs = TestUtil::next_int(&*random(), 10, 20);
    for _i in 0..num_docs {
        let mut doc = Document::new();
        let num_points = random().next_int_bound(3);
        for _j in 0..num_points {
            doc.add(Box::new(BinaryPoint::new(
                "field",
                &random_binary_value(num_dims, num_bytes_per_dim),
            )?));
        }
        w.add_document(&doc)?;
        if random().next_boolean() {
            directory_reader::open_with_writer(&*w)?.close()?;
        }
    }

    let reader1: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    w.force_merge(1)?;
    let reader2: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    let expected =
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader2))?.get_point_values("field")?;
    match expected {
        None => {
            assert_null(&point_values::min_packed_value(
                Arc::clone(&reader1),
                "field",
            )?);
            assert_null(&point_values::max_packed_value(
                Arc::clone(&reader1),
                "field",
            )?);
            assert_equals(&0, &point_values::doc_count(Arc::clone(&reader1), "field")?);
            assert_equals(&0, &point_values::size(Arc::clone(&reader1), "field")?);
        }
        Some(expected) => {
            assert_equals(
                &expected.min_packed_value()?,
                &point_values::min_packed_value(Arc::clone(&reader1), "field")?,
            );
            assert_equals(
                &expected.max_packed_value()?,
                &point_values::max_packed_value(Arc::clone(&reader1), "field")?,
            );
            assert_equals(
                &expected.doc_count()?,
                &point_values::doc_count(Arc::clone(&reader1), "field")?,
            );
            assert_equals(
                &expected.size()?,
                &point_values::size(Arc::clone(&reader1), "field")?,
            );
        }
    }
    let closed = IOUtils::close([
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| reader1.close()),
        IOUtils::closeable(|| reader2.close()),
    ]);
    // The writer and the readers share `dir`; they are closed, so release them.
    drop((w, reader1, reader2));
    let dir_closed = close_shared(dir);
    closed.and(dir_closed)
}
