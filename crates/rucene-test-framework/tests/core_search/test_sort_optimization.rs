//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSortOptimization.java`.
//!
//! The Java fields builders (`BiFunction<String, Integer, List<IndexableField>>`)
//! are plain functions returning the boxed fields of one document. Java
//! mutates a `SortField` after a `Sort` holds it; Rust's `Sort` owns a copy, so
//! each mutation is made before the `Sort` that must see it is built, which is
//! the same state at each search.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::doc_values::Lucene90DocValuesFormat;
use rucene::codecs::Codec;
use rucene::document::range_fields::IntRange;
use rucene::document::{
    Document, FloatDocValuesField, FloatPoint, IntPoint, KeywordField, LongField, LongPoint,
    NumericDocValuesField, SortedDocValuesField, Store, StoredField, StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{
    FieldInfo, FieldInfos, IndexOptions, IndexReader, IndexableField, LeafReader, Term,
    VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::field_comparator::SortValue;
use rucene::search::field_doc::FieldDoc;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::{
    BooleanQueryBuilder, MatchAllDocsQuery, Occur, Query, ScoreDoc, SortedNumericSelectorType,
    SortedSetSelectorType, TopDocs, TopFieldDocs,
};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::ScorerIndexSearcher;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::read_ahead_match_all_docs_query::ReadAheadMatchAllDocsQuery;
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test, IAE};

/// `TestSortOptimization`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSortOptimization",
        vec![
            test(
                "testLongSortOptimizationPointIndex",
                test_long_sort_optimization_point_index,
            ),
            test(
                "testLongSortOptimizationSkipperIndex",
                test_long_sort_optimization_skipper_index,
            ),
            test(
                "testLongSortOptimizationOnFieldNotIndexedWithPoints",
                test_long_sort_optimization_on_field_not_indexed_with_points,
            ),
            test(
                "testSortOptimizationWithMissingValuesPointIndex",
                test_sort_optimization_with_missing_values_point_index,
            ),
            test(
                "testSortOptimizationWithMissingValuesSkipperIndex",
                test_sort_optimization_with_missing_values_skipper_index,
            ),
            test(
                "testNumericDocValuesOptimizationWithMissingValuesPointIndex",
                test_numeric_doc_values_optimization_with_missing_values_point_index,
            ),
            test(
                "testNumericDocValuesOptimizationWithMissingValuesSkipperIndex",
                test_numeric_doc_values_optimization_with_missing_values_skipper_index,
            ),
            test(
                "testSortOptimizationEqualValuesPointIndex",
                test_sort_optimization_equal_values_point_index,
            ),
            test(
                "testSortOptimizationEqualValuesSkipperIndex",
                test_sort_optimization_equal_values_skipper_index,
            ),
            test(
                "testFloatSortOptimizationPointIndex",
                test_float_sort_optimization_point_index,
            ),
            test(
                "testFloatSortOptimizationSkpperIndex",
                test_float_sort_optimization_skpper_index,
            ),
            test(
                "testDocSortOptimizationMultipleIndices",
                test_doc_sort_optimization_multiple_indices,
            ),
            test(
                "testDocSortOptimizationWithAfter",
                test_doc_sort_optimization_with_after,
            ),
            test(
                "testDocSortOptimizationWithAfterCollectsAllDocs",
                test_doc_sort_optimization_with_after_collects_all_docs,
            ),
            test("testDocSortOptimization", test_doc_sort_optimization),
            test("testDocSort", test_doc_sort),
            test("testPointValidation", test_point_validation),
            test("testMaxDocVisited", test_max_doc_visited),
            test("testRandomLong", test_random_long),
            test(
                "testSortOptimizationOnSortedNumericField",
                test_sort_optimization_on_sorted_numeric_field,
            ),
            test(
                "testPointsPrunesOnSegmentEntry",
                test_points_prunes_on_segment_entry,
            ),
            test(
                "testDVSkipperPrunesOnSegmentEntry",
                test_dv_skipper_prunes_on_segment_entry,
            ),
            test(
                "testStringSortOptimizationBasedPostings",
                test_string_sort_optimization_based_postings,
            ),
            test(
                "testStringSortOptimizationBasedDVSkipper",
                test_string_sort_optimization_based_dv_skipper,
            ),
            test(
                "testStringSortOptimizationWithMissingValuesBasedPostings",
                test_string_sort_optimization_with_missing_values_based_postings,
            ),
            test(
                "testStringSortOptimizationWithMissingValuesBasedDVSkipper",
                test_string_sort_optimization_with_missing_values_based_dv_skipper,
            ),
            test(
                "testStringSortOptimizationFieldMissingInSegmentBasedPostings",
                test_string_sort_optimization_field_missing_in_segment_based_postings,
            ),
            test(
                "testStringSortOptimizationFieldMissingInSegmentBasedDVSkipper",
                test_string_sort_optimization_field_missing_in_segment_based_dv_skipper,
            ),
        ],
    )
}

/// `BiFunction<String, Integer, List<IndexableField>>`.
type FieldsBuilder = fn(&str, i32) -> Result<Vec<Box<dyn IndexableField>>>;

/// `BiFunction<String, Float, List<IndexableField>>`.
type FloatFieldsBuilder = fn(&str, f32) -> Result<Vec<Box<dyn IndexableField>>>;

/// `BiFunction<String, BytesRef, IndexableField>`.
type StringFieldBuilder = fn(&str, BytesRef) -> Result<Box<dyn IndexableField>>;

/// `List.of(new LongPoint(field, value), new NumericDocValuesField(field, value))`.
fn long_point_fields(field: &str, value: i32) -> Result<Vec<Box<dyn IndexableField>>> {
    Ok(vec![
        Box::new(LongPoint::new(field, &[i64::from(value)])?),
        Box::new(NumericDocValuesField::new(field, i64::from(value))),
    ])
}

/// `List.of(new IntPoint(field, value), new NumericDocValuesField(field, value))`.
fn int_point_fields(field: &str, value: i32) -> Result<Vec<Box<dyn IndexableField>>> {
    Ok(vec![
        Box::new(IntPoint::new(field, &[value])?),
        Box::new(NumericDocValuesField::new(field, i64::from(value))),
    ])
}

/// `List.of(NumericDocValuesField.indexedField(field, value))`.
fn skipper_fields(field: &str, value: i32) -> Result<Vec<Box<dyn IndexableField>>> {
    Ok(vec![Box::new(NumericDocValuesField::indexed_field(
        field,
        i64::from(value),
    ))])
}

/// `new KeywordField(field, value, Field.Store.NO)`.
fn keyword_field(field: &str, value: BytesRef) -> Result<Box<dyn IndexableField>> {
    Ok(Box::new(KeywordField::new_with_bytes(
        field,
        value,
        Store::NO,
    )?))
}

/// `SortedDocValuesField::indexedField`.
fn sorted_indexed_field(field: &str, value: BytesRef) -> Result<Box<dyn IndexableField>> {
    Ok(Box::new(SortedDocValuesField::indexed_field(field, value)))
}

/// `fieldsBuilder.apply(field, value).forEach(doc::add)`.
fn add_all(doc: &mut Document, fields: Vec<Box<dyn IndexableField>>) {
    for f in fields {
        doc.add(f);
    }
}

/// `new IndexWriterConfig().setCodec(TestUtil.getDefaultCodec())`.
fn default_codec_config() -> Result<IndexWriterConfig> {
    let config = IndexWriterConfig::new();
    // Make sure to use the default codec, otherwise some random points formats
    // that have large values for maxPointsPerLeaf might not enable skipping
    // with only 10k docs
    config.set_codec(TestUtil::get_default_codec())?;
    Ok(config)
}

/// `new Sort(fields...)`.
fn sort_of(fields: Vec<SortField>) -> Result<Sort> {
    Sort::new_fields(fields)
}

/// `new SortField(field, type)`.
fn sf(field: &str, t: SortFieldType) -> Result<SortField> {
    SortField::new(Some(field.to_string()), t)
}

/// `new SortField(field, type, reverse, missingValue)`.
fn sf_missing(
    field: &str,
    t: SortFieldType,
    reverse: bool,
    missing: MissingValue,
) -> Result<SortField> {
    SortField::new_with_missing(Some(field.to_string()), t, reverse, Some(missing))
}

/// `KeywordField.newSortField(field, reverse, SortedSetSelector.Type.MIN, missingValue)`.
fn keyword_sort(field: &str, reverse: bool, missing: MissingValue) -> Result<SortField> {
    Ok(KeywordField::new_sort_field_with_missing(
        Some(field),
        reverse,
        Some(SortedSetSelectorType::MIN),
        Some(missing),
    )?
    .into_sort_field())
}

/// `fieldDoc.fields[i]`.
fn field(fd: &FieldDoc, i: usize) -> &SortValue {
    &fd.fields.as_ref().expect("FieldDoc.fields")[i]
}

/// `((Long) value).intValue()`.
fn long_int_value(value: &SortValue) -> i32 {
    match value {
        SortValue::Long(v) => *v as i32,
        other => panic!("java.lang.ClassCastException: {other:?} cannot be cast to java.lang.Long"),
    }
}

/// `(float) value`.
fn float_value(value: &SortValue) -> f32 {
    match value {
        SortValue::Float(v) => *v,
        other => {
            panic!("java.lang.ClassCastException: {other:?} cannot be cast to java.lang.Float")
        }
    }
}

/// `new FieldDoc(doc, Float.NaN, fields)`.
fn after_doc(doc: i32, fields: Vec<SortValue>) -> FieldDoc {
    FieldDoc::with_fields(doc, f32::NAN, fields)
}

/// `reader.leaves().size()`.
fn leaves_size(reader: &Arc<dyn DirectoryReader>) -> Result<usize> {
    let r: Arc<dyn IndexReader> = Arc::clone(reader) as Arc<dyn IndexReader>;
    Ok(r.leaves()?.len())
}

/// `TopFieldDocs.scoreDocs` as the `ScoreDoc[]` `CheckHits.checkEqual` reads.
fn score_docs(top_docs: &TopFieldDocs) -> Vec<ScoreDoc> {
    top_docs.score_docs.iter().map(|fd| fd.score_doc).collect()
}

/// `final DirectoryReader reader = DirectoryReader.open(writer); writer.close();`.
fn open_and_close(writer: &Arc<IndexWriter>) -> Result<Arc<dyn DirectoryReader>> {
    let reader = directory_reader::open_with_writer(&**writer)?;
    writer.close()?;
    Ok(reader)
}

/// `reader.close(); dir.close();`.
fn close_all(reader: &Arc<dyn DirectoryReader>, dir: TestDirectory) -> Result<()> {
    reader.close()?;
    dir.close()
}

/// Port of `TestSortOptimization.testLongSortOptimizationPointIndex`.
fn test_long_sort_optimization_point_index() -> Result<()> {
    long_sort_optimization(long_point_fields)
}

/// Port of `TestSortOptimization.testLongSortOptimizationSkipperIndex`.
fn test_long_sort_optimization_skipper_index() -> Result<()> {
    long_sort_optimization(skipper_fields)
}

/// `private void testLongSortOptimization(BiFunction<...> fieldsBuilder)`.
fn long_sort_optimization(fields_builder: FieldsBuilder) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = default_codec_config()?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let num_docs = LuceneTestCase::at_least(10000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        add_all(&mut doc, fields_builder("my_field", i)?);
        writer.add_document(&doc)?;
        if i == 7000 {
            writer.flush()?; // two segments
        }
    }
    let reader = open_and_close(&writer)?;
    let sort_field = sf("my_field", SortFieldType::Long)?;
    let sort = sort_of(vec![sort_field.clone()])?;
    let num_hits = 3;

    {
        // simple sort
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;

        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits {
            let field_doc = &top_docs.score_docs[i as usize];
            assert_equals(&i, &long_int_value(field(field_doc, 0)));
        }
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), i64::from(num_docs));
    }

    {
        // paging sort with after
        let after_value: i64 = 2;
        let after = after_doc(2, vec![SortValue::Long(after_value)]);
        let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits {
            let field_doc = &top_docs.score_docs[i as usize];
            assert_equals(
                &SortValue::Long(after_value + 1 + i64::from(i)),
                field(field_doc, 0),
            );
        }
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), i64::from(num_docs));
    }

    {
        // test that if there is the secondary sort on _score, scores are filled correctly
        let sort2 = sort_of(vec![sort_field.clone(), SortField::FIELD_SCORE])?;
        let top_docs = assert_search_hits(&reader, &sort2, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits {
            let field_doc = &top_docs.score_docs[i as usize];
            assert_equals(&i, &long_int_value(field(field_doc, 0)));
            let score = float_value(field(field_doc, 1));
            assert_equals_delta(1.0, f64::from(score), 0.001);
        }
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), i64::from(num_docs));
    }

    {
        // test that if numeric field is a secondary sort, no optimization is run
        let sort2 = sort_of(vec![SortField::FIELD_SCORE, sort_field.clone()])?;
        let top_docs = assert_search_hits(&reader, &sort2, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        assert_equals(&top_docs.total_hits.value(), &i64::from(num_docs)); // assert that all documents were collected => optimization was not run
    }

    close_all(&reader, dir)
}

/// test that even if a field is not indexed with points, optimized sort still
/// works as expected, although no optimization will be run
///
/// Port of `TestSortOptimization.testLongSortOptimizationOnFieldNotIndexedWithPoints`.
fn test_long_sort_optimization_on_field_not_indexed_with_points() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(100);
    // my_field is not indexed with points
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new(
            "my_field",
            i64::from(i),
        )));
        writer.add_document(&doc)?;
    }
    let reader = open_and_close(&writer)?;
    let sort_field = sf("my_field", SortFieldType::Long)?;
    let sort = sort_of(vec![sort_field])?;
    let num_hits = 3;

    let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
    assert_equals(&top_docs.score_docs.len(), &(num_hits as usize)); // sort still works and returns expected number of docs
    for i in 0..num_hits {
        let field_doc = &top_docs.score_docs[i as usize];
        assert_equals(&i, &long_int_value(field(field_doc, 0))); // returns expected values
    }
    assert_equals(&top_docs.total_hits.value(), &i64::from(num_docs)); // assert that all documents were collected => optimization was not run

    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testSortOptimizationWithMissingValuesPointIndex`.
fn test_sort_optimization_with_missing_values_point_index() -> Result<()> {
    sort_optimization_with_missing_values(long_point_fields)
}

/// Port of `TestSortOptimization.testSortOptimizationWithMissingValuesSkipperIndex`.
fn test_sort_optimization_with_missing_values_skipper_index() -> Result<()> {
    sort_optimization_with_missing_values(skipper_fields)
}

/// `private void testSortOptimizationWithMissingValues(BiFunction<...> fieldsBuilder)`.
fn sort_optimization_with_missing_values(fields_builder: FieldsBuilder) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = default_codec_config()?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let num_docs = LuceneTestCase::at_least(10000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        if (i % 500) != 0 {
            // miss values on every 500th document
            add_all(&mut doc, fields_builder("my_field", i)?);
        }
        writer.add_document(&doc)?;
        if i == 7000 {
            writer.flush()?; // two segments
        }
    }
    let reader = open_and_close(&writer)?;
    let num_hits = 3;
    let num_docs = i64::from(num_docs);

    {
        // test that optimization is run when missing value setting of SortField is competitive with
        // Pruning.GREATER_THAN_OR_EQUAL_TO
        let sort_field = sf_missing(
            "my_field",
            SortFieldType::Long,
            false,
            MissingValue::Long(0),
        )?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }
    {
        // test that optimization is not run when missing value setting of SortField is competitive
        // with Pruning.SKIP
        let sort_field1 = sf_missing(
            "my_field1",
            SortFieldType::Long,
            false,
            MissingValue::Long(0),
        )?;
        let sort_field2 = sf_missing(
            "my_field2",
            SortFieldType::Long,
            false,
            MissingValue::Long(0),
        )?;
        let sort = sort_of(vec![sort_field1, sort_field2])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        assert_equals(&top_docs.total_hits.value(), &num_docs); // assert that all documents were collected => optimization was not run
    }
    {
        // test that optimization is run when missing value setting of SortField is NOT competitive
        let sort_field = sf_missing(
            "my_field",
            SortFieldType::Long,
            false,
            MissingValue::Long(100),
        )?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // test that optimization is not run when missing value setting of SortField is competitive
        // with after on asc order
        let after_value = i64::MAX;
        let after_doc_id = 10 + random().next_int_bound(1000);
        let after = after_doc(after_doc_id, vec![SortValue::Long(after_value)]);
        let sort_field = sf_missing(
            "my_field",
            SortFieldType::Long,
            false,
            MissingValue::Long(i64::MAX),
        )?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // test that optimization is not run when missing value setting of SortField is competitive
        // with after on desc order
        let after_value = i64::MAX;
        let after_doc_id = 10 + random().next_int_bound(1000);
        let after = after_doc(after_doc_id, vec![SortValue::Long(after_value)]);
        let sort_field = sf_missing(
            "my_field",
            SortFieldType::Long,
            true,
            MissingValue::Long(i64::MAX),
        )?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // test that optimization is run when missing value setting of SortField is NOT competitive
        // with after on asc order
        let after_value: i64 = 3;
        let after = after_doc(3, vec![SortValue::Long(after_value)]);
        let sort_field = sf_missing(
            "my_field",
            SortFieldType::Long,
            false,
            MissingValue::Long(2),
        )?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits {
            let field_doc = &top_docs.score_docs[i as usize];
            assert_equals(
                &SortValue::Long(after_value + 1 + i64::from(i)),
                field(field_doc, 0),
            );
        }
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testNumericDocValuesOptimizationWithMissingValuesPointIndex`.
fn test_numeric_doc_values_optimization_with_missing_values_point_index() -> Result<()> {
    numeric_doc_values_optimization_with_missing_values(long_point_fields)
}

/// Port of `TestSortOptimization.testNumericDocValuesOptimizationWithMissingValuesSkipperIndex`.
fn test_numeric_doc_values_optimization_with_missing_values_skipper_index() -> Result<()> {
    numeric_doc_values_optimization_with_missing_values(skipper_fields)
}

/// `private void testNumericDocValuesOptimizationWithMissingValues(BiFunction<...> fieldsBuilder)`.
fn numeric_doc_values_optimization_with_missing_values(
    fields_builder: FieldsBuilder,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = default_codec_config()?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let num_docs = LuceneTestCase::at_least(10000);
    let miss_values_num_docs = num_docs / 2;
    for i in 0..num_docs {
        let mut doc = Document::new();
        if i <= miss_values_num_docs { // missing value document
        } else {
            add_all(&mut doc, fields_builder("my_field", i)?);
        }
        writer.add_document(&doc)?;
    }
    let reader = open_and_close(&writer)?;
    let num_hits = 3;
    let num_docs = i64::from(num_docs);
    let top_docs1;
    let top_docs2;

    {
        // Test that optimization is run with NumericDocValues when missing value is NOT competitive
        let sort_field = sf_missing("my_field", SortFieldType::Long, true, MissingValue::Long(0))?;
        let sort = sort_of(vec![sort_field])?;
        top_docs1 = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_non_competitive_hits_are_skipped(top_docs1.total_hits.value(), num_docs);
    }
    {
        // Test that sort on sorted numeric field without sort optimization and with sort optimization
        // produce the same results
        let mut sort_field =
            sf_missing("my_field", SortFieldType::Long, true, MissingValue::Long(0))?;
        sort_field.set_optimize_sort_with_points(false);
        let sort = sort_of(vec![sort_field])?;
        top_docs2 = assert_search_hits(&reader, &sort, num_hits, None)?;
        // assert that the resulting hits are the same
        assert_equals(&top_docs1.score_docs.len(), &top_docs2.score_docs.len());
        assert_equals(&top_docs1.score_docs.len(), &(num_hits as usize));
        let score_docs1 = &top_docs1.score_docs;
        let score_docs2 = &top_docs2.score_docs;
        for i in 0..num_hits as usize {
            let field_doc = &score_docs1[i];
            let field_doc2 = &score_docs2[i];
            assert_equals(field(field_doc, 0), field(field_doc2, 0));
            assert_equals(&field_doc.doc(), &field_doc2.doc());
        }
        Assert::assert_true(top_docs1.total_hits.value() < top_docs2.total_hits.value());
    }

    {
        // Test that we can't do optimization via NumericDocValues when there are multiple comparators
        // missing values are not competitive
        let sort_field1 = sf_missing("my_field", SortFieldType::Long, true, MissingValue::Long(0))?;
        let sort_field2 = sf_missing("other", SortFieldType::Long, true, MissingValue::Long(0))?;
        let multi_sorts = sort_of(vec![sort_field1, sort_field2])?;
        let top_docs = assert_search_hits(&reader, &multi_sorts, num_hits, None)?;
        // can't optimization with NumericDocValues when there are multiple comparators
        assert_equals(&top_docs.total_hits.value(), &num_docs);
    }

    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testSortOptimizationEqualValuesPointIndex`.
fn test_sort_optimization_equal_values_point_index() -> Result<()> {
    sort_optimization_equal_values(int_point_fields)
}

/// Port of `TestSortOptimization.testSortOptimizationEqualValuesSkipperIndex`.
fn test_sort_optimization_equal_values_skipper_index() -> Result<()> {
    sort_optimization_equal_values(skipper_fields)
}

/// `private void testSortOptimizationEqualValues(BiFunction<...> fieldsBuilder)`.
fn sort_optimization_equal_values(fields_builder: FieldsBuilder) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = default_codec_config()?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let num_docs = LuceneTestCase::at_least(if *TEST_NIGHTLY { 50_000 } else { 10_000 });
    for i in 1..=num_docs {
        let mut doc = Document::new();
        add_all(&mut doc, fields_builder("my_field1", 100)?);
        doc.add(Box::new(NumericDocValuesField::new(
            "my_field2",
            i64::from(num_docs - i),
        ))); // diff values for the field my_field2
        writer.add_document(&doc)?;
        // if there is only one segment, we could test that totalHits must always equal (numHits + 1)
        if i == 7000 && random().next_boolean() {
            writer.flush()?; // two segments
        }
    }
    let reader = open_and_close(&writer)?;
    let num_hits = 3;
    let num_docs_l = i64::from(num_docs);

    {
        // test that sorting on a single field with equal values uses the optimization with
        // GREATER_THAN_OR_EQUAL_TO
        let sort_field = sf("my_field1", SortFieldType::Int)?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits as usize {
            let field_doc = &top_docs.score_docs[i];
            assert_equals(&SortValue::Int(100), field(field_doc, 0));
        }
        if leaves_size(&reader)? == 1 {
            // if segment size equals one, totalHits should always equals numHits plus 1
            assert_equals(&top_docs.total_hits.value(), &i64::from(num_hits + 1));
        }
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs_l);
    }

    {
        // test that sorting on a single field with equal values and after parameter
        // use the optimization with GREATER_THAN_OR_EQUAL_TO
        let after_value = 100;
        let after_doc_id = 10 + random().next_int_bound(1000);
        let sort_field = sf("my_field1", SortFieldType::Int)?;
        let sort = sort_of(vec![sort_field])?;
        let after = after_doc(after_doc_id, vec![SortValue::Int(after_value)]);
        let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits as usize {
            let field_doc = &top_docs.score_docs[i];
            assert_equals(&SortValue::Int(100), field(field_doc, 0));
            Assert::assert_true(field_doc.doc() > after_doc_id);
        }
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs_l);
    }

    {
        // test that sorting on main field with equal values + another field for tie breaks doesn't
        // use optimization with Pruning.GREATER_THAN
        let sort_field1 = sf("my_field1", SortFieldType::Int)?;
        let sort_field2 = sf("my_field2", SortFieldType::Int)?;
        let sort = sort_of(vec![sort_field1, sort_field2])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits {
            let field_doc = &top_docs.score_docs[i as usize];
            assert_equals(&SortValue::Int(100), field(field_doc, 0)); // sort on 1st field as expected
            assert_equals(&SortValue::Int(i), field(field_doc, 1)); // sort on 2nd field as expected
        }
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        assert_equals(&top_docs.total_hits.value(), &num_docs_l); // assert that all documents were collected => optimization was not run
    }

    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testFloatSortOptimizationPointIndex`.
fn test_float_sort_optimization_point_index() -> Result<()> {
    float_sort_optimization(|field, value| {
        Ok(vec![
            Box::new(FloatPoint::new(field, &[value])?),
            Box::new(FloatDocValuesField::new(field, value)),
        ])
    })
}

/// Port of `TestSortOptimization.testFloatSortOptimizationSkpperIndex`.
fn test_float_sort_optimization_skpper_index() -> Result<()> {
    // `FloatDocValuesField.indexedField` is the static
    // `NumericDocValuesField.indexedField` it inherits.
    float_sort_optimization(|field, value| {
        Ok(vec![Box::new(NumericDocValuesField::indexed_field(
            field,
            i64::from(value.to_bits() as i32),
        ))])
    })
}

/// `private void testFloatSortOptimization(BiFunction<...> fieldsBuilder)`.
fn float_sort_optimization(fields_builder: FloatFieldsBuilder) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(10000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        add_all(&mut doc, fields_builder("my_field", 1f32 * i as f32)?);
        writer.add_document(&doc)?;
    }
    let reader = open_and_close(&writer)?;
    let sort_field = sf("my_field", SortFieldType::Float)?;
    let sort = sort_of(vec![sort_field])?;
    let num_hits = 3;

    {
        // simple sort
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_equals(&top_docs.score_docs.len(), &(num_hits as usize));
        for i in 0..num_hits {
            let field_doc = &top_docs.score_docs[i as usize];
            assert_equals(&SortValue::Float(1f32 * i as f32), field(field_doc, 0));
        }
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), i64::from(num_docs));
    }

    close_all(&reader, dir)
}

/// Test that a search with sort on [_doc, other fields] across multiple
/// indices doesn't miss any documents.
///
/// Port of `TestSortOptimization.testDocSortOptimizationMultipleIndices`.
fn test_doc_sort_optimization_multiple_indices() -> Result<()> {
    let num_indices = 3;
    let num_docs_in_index = LuceneTestCase::at_least(50);
    let mut dirs: Vec<TestDirectory> = Vec::with_capacity(num_indices);
    let mut readers: Vec<Arc<dyn DirectoryReader>> = Vec::with_capacity(num_indices);
    for i in 0..num_indices {
        dirs.push(LuceneTestCase::new_directory());
        let config = default_codec_config()?;
        {
            let writer = IndexWriter::new(dirs[i].as_directory(), config)?;
            let body = (|| -> Result<()> {
                for doc_id in 0..num_docs_in_index {
                    let mut doc = Document::new();
                    doc.add(Box::new(NumericDocValuesField::new(
                        "my_field",
                        i64::from(doc_id * num_indices as i32 + i as i32),
                    )));
                    writer.add_document(&doc)?;
                }
                writer.flush()
            })();
            let closed = writer.close();
            body.and(closed)?;
        }
        readers.push(directory_reader::open(dirs[i].as_directory())?);
    }

    let size = 7;
    let sort = sort_of(vec![
        SortField::FIELD_DOC,
        sf("my_field", SortFieldType::Long)?,
    ])?;
    let mut top_docs: Vec<TopFieldDocs> = Vec::with_capacity(num_indices);
    let mut cur_num_hits;
    let mut after: Option<FieldDoc> = None;
    let mut collected_docs: i64 = 0;
    let mut total_docs: i64 = 0;
    let mut num_hits = 0;
    loop {
        top_docs.clear();
        for i in 0..num_indices {
            top_docs.push(assert_search_hits(&readers[i], &sort, size, after.clone())?);
            for doc_id in 0..top_docs[i].score_docs.len() {
                top_docs[i].score_docs[doc_id].score_doc.shard_index = i as i32;
            }
            collected_docs += top_docs[i].total_hits.value();
            total_docs += i64::from(num_docs_in_index);
        }
        let merged_top_dcs = TopDocs::merge_sorted(&sort, size as usize, &top_docs)?;
        cur_num_hits = merged_top_dcs.score_docs.len();
        num_hits += cur_num_hits as i32;
        if cur_num_hits > 0 {
            after = Some(merged_top_dcs.score_docs[cur_num_hits - 1].clone());
        }
        if cur_num_hits == 0 {
            break;
        }
    }

    for i in 0..num_indices {
        readers[i].close()?;
        dirs[i].close()?;
    }

    let expected_num_hits = num_docs_in_index * num_indices as i32;
    assert_equals(&expected_num_hits, &num_hits);
    assert_non_competitive_hits_are_skipped(collected_docs, total_docs);
    Ok(())
}

/// Port of `TestSortOptimization.testDocSortOptimizationWithAfter`.
fn test_doc_sort_optimization_with_after() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(150);
    for i in 0..num_docs {
        let doc = Document::new();
        writer.add_document(&doc)?;
        if (i > 0) && (i % 50 == 0) {
            writer.flush()?;
        }
    }

    let reader = open_and_close(&writer)?;
    let num_hits = 10;
    let search_afters = [3, 10, num_docs - 10];
    for search_after in search_afters {
        // sort by _doc with search after should trigger optimization
        {
            let sort = sort_of(vec![SortField::FIELD_DOC])?;
            let after = after_doc(search_after, vec![SortValue::Int(search_after)]);
            let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
            let exp_num_hits = if search_after >= (num_docs - num_hits) {
                num_docs - search_after - 1
            } else {
                num_hits
            };
            assert_equals(&(exp_num_hits as usize), &top_docs.score_docs.len());
            for i in 0..top_docs.score_docs.len() {
                let expected_doc_id = search_after + 1 + i as i32;
                assert_equals(&expected_doc_id, &top_docs.score_docs[i].doc());
            }
            assert_equals(
                &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                &top_docs.total_hits.relation(),
            );
            assert_non_competitive_hits_are_skipped(
                top_docs.total_hits.value(),
                i64::from(num_docs),
            );
        }

        // sort by _doc + _score with search after should trigger optimization
        {
            let sort = sort_of(vec![SortField::FIELD_DOC, SortField::FIELD_SCORE])?;
            let after = after_doc(
                search_after,
                vec![SortValue::Int(search_after), SortValue::Float(1.0f32)],
            );
            let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
            let exp_num_hits = if search_after >= (num_docs - num_hits) {
                num_docs - search_after - 1
            } else {
                num_hits
            };
            assert_equals(&(exp_num_hits as usize), &top_docs.score_docs.len());
            for i in 0..top_docs.score_docs.len() {
                let expected_doc_id = search_after + 1 + i as i32;
                assert_equals(&expected_doc_id, &top_docs.score_docs[i].doc());
            }
            assert_equals(
                &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                &top_docs.total_hits.relation(),
            );
            assert_non_competitive_hits_are_skipped(
                top_docs.total_hits.value(),
                i64::from(num_docs),
            );
        }

        // sort by _doc desc should not trigger optimization
        {
            let sort = sort_of(vec![SortField::new_reverse(
                None,
                SortFieldType::Doc,
                true,
            )?])?;
            let after = after_doc(search_after, vec![SortValue::Int(search_after)]);
            let top_docs = assert_search_hits(&reader, &sort, num_hits, Some(after))?;
            let exp_num_hits = if search_after < num_hits {
                search_after
            } else {
                num_hits
            };
            assert_equals(&(exp_num_hits as usize), &top_docs.score_docs.len());
            for i in 0..top_docs.score_docs.len() {
                let expected_doc_id = search_after - 1 - i as i32;
                assert_equals(&expected_doc_id, &top_docs.score_docs[i].doc());
            }
            // assert that all documents were collected
            assert_equals(&i64::from(num_docs), &top_docs.total_hits.value());
        }
    }

    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testDocSortOptimizationWithAfterCollectsAllDocs`.
fn test_doc_sort_optimization_with_after_collects_all_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(if *TEST_NIGHTLY { 50_000 } else { 5_000 });
    let multiple_segments = random().next_boolean();
    let num_docs_in_segment = num_docs / 10 + random().next_int_bound(num_docs / 10);

    for i in 1..=num_docs {
        let doc = Document::new();
        writer.add_document(&doc)?;
        if multiple_segments && (i % num_docs_in_segment == 0) {
            writer.flush()?;
        }
    }
    writer.flush()?;

    let reader = directory_reader::open_with_writer(&*writer)?;
    let mut visited_hits = 0;
    let mut after: Option<FieldDoc> = None;
    while visited_hits < num_docs {
        let batch = 1 + random().next_int_bound(500);
        let top_docs = assert_search_hits(
            &reader,
            &sort_of(vec![SortField::FIELD_DOC])?,
            batch,
            after.clone(),
        )?;
        let expected_hits = (num_docs - visited_hits).min(batch);
        assert_equals(&(expected_hits as usize), &top_docs.score_docs.len());
        after = Some(top_docs.score_docs[expected_hits as usize - 1].clone());
        for i in 0..top_docs.score_docs.len() {
            assert_equals(&visited_hits, &top_docs.score_docs[i].doc());
            visited_hits += 1;
        }
    }
    assert_equals(&visited_hits, &num_docs);
    // `IOUtils.close(writer, reader, dir)`.
    let r1 = writer.close();
    let r2 = reader.close();
    let r3 = dir.close();
    r1.and(r2).and(r3)
}

/// Port of `TestSortOptimization.testDocSortOptimization`.
fn test_doc_sort_optimization() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(100);
    let mut seg = 1;
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LongPoint::new("lf", &[i64::from(i)])?));
        doc.add(Box::new(StoredField::new_number(
            "slf",
            rucene::document::NumericValue::Int(i),
        )?));
        doc.add(Box::new(StringField::new(
            "tf",
            format!("seg{seg}"),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
        if (i > 0) && (i % 50 == 0) {
            writer.flush()?;
            seg += 1;
        }
    }
    let reader = open_and_close(&writer)?;

    let num_hits = 3;
    let sort = sort_of(vec![SortField::FIELD_DOC])?;

    // sort by _doc should skip all non-competitive documents
    {
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_equals(&(num_hits as usize), &top_docs.score_docs.len());
        for i in 0..num_hits {
            assert_equals(&i, &top_docs.score_docs[i as usize].doc());
        }
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), 10);
    }

    // sort by _doc with a bool query should skip all non-competitive documents
    {
        let lower_range = 40;
        let mut bq = BooleanQueryBuilder::new();
        bq.add(
            LongPoint::new_range_query("lf", &[lower_range], &[i64::MAX])?,
            Occur::MUST,
        )?;
        bq.add(
            Arc::new(TermQuery::new(Term::from_text("tf", "seg1"))),
            Occur::MUST,
        )?;

        let top_docs =
            assert_search_hits_query(&reader, Arc::new(bq.build()), &sort, num_hits, None)?;
        assert_equals(&(num_hits as usize), &top_docs.score_docs.len());
        let stored_fields = reader.stored_fields()?;
        for i in 0..num_hits {
            let d = stored_fields.document(top_docs.score_docs[i as usize].doc())?;
            assert_equals(
                &Some((i64::from(i) + lower_range).to_string()),
                &d.get("slf"),
            );
            assert_equals(&Some("seg1".to_string()), &d.get("tf"));
        }
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), 10);
    }

    close_all(&reader, dir)
}

/// Test that sorting on _doc works correctly. This test goes through
/// DefaultBulkSorter::scoreRange, where scorerIterator is BitSetIterator. As a
/// conjunction of this BitSetIterator with DocComparator's iterator, we get
/// BitSetConjunctionDISI. BitSetConjuctionDISI advances based on the
/// DocComparator's iterator, and doesn't consider that its BitSetIterator may
/// have advanced passed a certain doc.
///
/// Port of `TestSortOptimization.testDocSort`.
fn test_doc_sort() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = 4;
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            format!("id{i}"),
            Store::NO,
        )?));
        if i < 2 {
            doc.add(Box::new(LongPoint::new("lf", &[1])?));
        }
        writer.add_document(&doc)?;
    }
    let reader = open_and_close(&writer)?;

    let mut searcher = LuceneTestCase::new_searcher_assert(
        Arc::clone(&reader),
        random().next_boolean(),
        random().next_boolean(),
    );
    searcher.set_query_cache(None);
    let num_hits = 10;
    let total_hits_threshold = 10;
    let sort = sort_of(vec![SortField::FIELD_DOC])?;

    {
        let collector_manager =
            TopFieldCollectorManager::with_num_hits(sort, num_hits, total_hits_threshold)?;
        let mut bq = BooleanQueryBuilder::new();
        bq.add(LongPoint::new_exact_query("lf", 1)?, Occur::MUST)?;
        bq.add(
            Arc::new(TermQuery::new(Term::from_text("id", "id3"))),
            Occur::MUST_NOT,
        )?;
        let top_docs = searcher.search_with_manager(Arc::new(bq.build()), &collector_manager)?;
        assert_equals(&2, &top_docs.score_docs.len());
    }

    drop(searcher);
    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testPointValidation`.
fn test_point_validation() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();

    doc.add(Box::new(IntPoint::new("intField", &[4])?));
    doc.add(Box::new(NumericDocValuesField::new("intField", 4)));

    doc.add(Box::new(LongPoint::new("longField", &[42])?));
    doc.add(Box::new(NumericDocValuesField::new("longField", 42)));

    doc.add(Box::new(IntRange::new("intRange", &[1], &[10])?));
    doc.add(Box::new(NumericDocValuesField::new("intRange", 4)));

    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher_assert(
        Arc::clone(&reader),
        random().next_boolean(),
        random().next_boolean(),
    );
    let match_all = || -> Arc<dyn Query> { Arc::new(MatchAllDocsQuery::instance()) };

    let mut long_sort_on_int_field = sf("intField", SortFieldType::Long)?;
    expect_throws(IAE, || {
        searcher.search_with_sort(
            match_all(),
            1,
            &sort_of(vec![long_sort_on_int_field.clone()])?,
        )
    });
    // assert that when sort optimization is disabled we can use LONG sort on int field
    long_sort_on_int_field.set_optimize_sort_with_indexed_data(false);
    searcher.search_with_sort(match_all(), 1, &sort_of(vec![long_sort_on_int_field])?)?;

    let mut int_sort_on_long_field = sf("longField", SortFieldType::Int)?;
    expect_throws(IAE, || {
        searcher.search_with_sort(
            match_all(),
            1,
            &sort_of(vec![int_sort_on_long_field.clone()])?,
        )
    });
    // assert that when sort optimization is disabled we can use INT sort on long field
    int_sort_on_long_field.set_optimize_sort_with_indexed_data(false);
    searcher.search_with_sort(match_all(), 1, &sort_of(vec![int_sort_on_long_field])?)?;

    let mut int_sort_on_int_range_field = sf("intRange", SortFieldType::Int)?;
    expect_throws(IAE, || {
        searcher.search_with_sort(
            match_all(),
            1,
            &sort_of(vec![int_sort_on_int_range_field.clone()])?,
        )
    });
    // assert that when sort optimization is disabled we can use INT sort on intRange field
    int_sort_on_int_range_field.set_optimize_sort_with_indexed_data(false);
    searcher.search_with_sort(match_all(), 1, &sort_of(vec![int_sort_on_int_range_field])?)?;

    drop(searcher);
    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testMaxDocVisited`.
fn test_max_doc_visited() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(10000);
    let offset = i64::from(100 + random().next_int_bound(100));
    let smallest_value = i64::from(50 + random().next_int_bound(50));
    let mut flushed = false;
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new(
            "my_field",
            i64::from(i) + offset,
        )));
        doc.add(Box::new(LongPoint::new(
            "my_field",
            &[i64::from(i) + offset],
        )?));
        writer.add_document(&doc)?;
        if i >= 5000 && !flushed {
            flushed = true;
            writer.flush()?;
            // Index the smallest value to the first slot of the second segment
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::new(
                "my_field",
                smallest_value,
            )));
            doc.add(Box::new(LongPoint::new("my_field", &[smallest_value])?));
            writer.add_document(&doc)?;
        }
    }
    let reader = open_and_close(&writer)?;
    let sort_field = sf("my_field", SortFieldType::Long)?;
    let top_docs = assert_search_hits(
        &reader,
        &sort_of(vec![sort_field])?,
        1 + random().next_int_bound(100),
        None,
    )?;
    let field_doc = &top_docs.score_docs[0];
    assert_equals(
        &(smallest_value as i32),
        &long_int_value(field(field_doc, 0)),
    );
    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testRandomLong`.
fn test_random_long() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let mut seq_nos: Vec<i64> = Vec::new();
    let limit = if *TEST_NIGHTLY { 10000 } else { 1000 };
    let iterations = limit + random().next_int_bound(limit);
    let mut seq_no_generator = i64::from(random().next_int_bound(1000));
    for _i in 0..i64::from(iterations) {
        let copies = if random().next_int_bound(100) <= 5 {
            1
        } else {
            1 + random().next_int_bound(5)
        };
        for _j in 0..copies {
            seq_nos.push(seq_no_generator);
        }
        seq_nos.push(seq_no_generator);
        seq_no_generator += 1;
        if random().next_int_bound(100) <= 5 {
            seq_no_generator += i64::from(random().next_int_bound(10));
        }
    }

    shuffle(&mut seq_nos, &*random());
    let mut pending_docs = 0;
    for &seq_no in &seq_nos {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("seq_no", seq_no)));
        doc.add(Box::new(LongPoint::new("seq_no", &[seq_no])?));
        writer.add_document(&doc)?;
        pending_docs += 1;
        if pending_docs > 500 && random().next_int_bound(100) <= 5 {
            pending_docs = 0;
            writer.flush()?;
        }
    }
    let reverse = random().next_boolean();
    writer.flush()?;
    if !reverse {
        seq_nos.sort();
    } else {
        seq_nos.sort_by(|a, b| b.cmp(a));
    }
    let reader = open_and_close(&writer)?;
    let sort_field =
        SortField::new_reverse(Some("seq_no".to_string()), SortFieldType::Long, reverse)?;
    let mut visited_hits = 0usize;
    let mut after: Option<FieldDoc> = None;
    // test page search
    while visited_hits < seq_nos.len() {
        let batch = 1 + random().next_int_bound(100);
        let query: Arc<dyn Query> = if random().next_boolean() {
            Arc::new(MatchAllDocsQuery::instance())
        } else {
            LongPoint::new_range_query("seq_no", &[0], &[i64::MAX])?
        };
        let top_docs = assert_search_hits_query(
            &reader,
            query,
            &sort_of(vec![sort_field.clone()])?,
            batch,
            after.clone(),
        )?;
        let expected_hits = (seq_nos.len() - visited_hits).min(batch as usize);
        assert_equals(&expected_hits, &top_docs.score_docs.len());
        after = Some(top_docs.score_docs[expected_hits - 1].clone());
        for i in 0..top_docs.score_docs.len() {
            let field_doc = &top_docs.score_docs[i];
            let expected_seq_no = seq_nos[visited_hits];
            assert_equals(
                &expected_seq_no,
                &i64::from(long_int_value(field(field_doc, 0))),
            );
            visited_hits += 1;
        }
    }

    // test search
    let num_hits = 1 + random().next_int_bound(100);
    let top_docs = assert_search_hits(&reader, &sort_of(vec![sort_field])?, num_hits, after)?;
    for i in 0..top_docs.score_docs.len() {
        let expected_seq_no = seq_nos[i];
        let field_doc = &top_docs.score_docs[i];
        assert_equals(
            &expected_seq_no,
            &i64::from(long_int_value(field(field_doc, 0))),
        );
    }
    close_all(&reader, dir)
}

/// Test that sort on sorted numeric field without sort optimization and with
/// sort optimization produce the same results
///
/// Port of `TestSortOptimization.testSortOptimizationOnSortedNumericField`.
fn test_sort_optimization_on_sorted_numeric_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(5000);
    for _i in 0..num_docs {
        let value = random().next_int();
        let value2 = random().next_int();
        let mut doc = Document::new();
        doc.add(Box::new(LongField::new(
            "my_field",
            i64::from(value),
            Store::NO,
        )));
        doc.add(Box::new(LongField::new(
            "my_field",
            i64::from(value2),
            Store::NO,
        )));
        writer.add_document(&doc)?;
    }
    let reader = open_and_close(&writer)?;

    let selector_type = *RandomPicks::random_from(
        &*random(),
        &[
            SortedNumericSelectorType::MIN,
            SortedNumericSelectorType::MAX,
        ],
    );
    let reverse = random().next_boolean();
    let mut sort_field =
        LongField::new_sort_field("my_field", reverse, selector_type)?.into_sort_field();
    sort_field.set_optimize_sort_with_indexed_data(false);
    let sort = sort_of(vec![sort_field])?; // sort without sort optimization
    let sort_field2 =
        LongField::new_sort_field("my_field", reverse, selector_type)?.into_sort_field();
    let sort2 = sort_of(vec![sort_field2])?; // sort with sort optimization

    let mut expected_collected_hits: i64 = 0;
    let mut collected_hits: i64 = 0;
    let mut collected_hits2: i64 = 0;
    let mut visited_hits = 0;
    let mut after: Option<FieldDoc> = None;
    while visited_hits < num_docs {
        let batch = 1 + random().next_int_bound(100);
        let expected_hits = (num_docs - visited_hits).min(batch);

        let top_docs = assert_search_hits(&reader, &sort, batch, after.clone())?;
        let score_docs = &top_docs.score_docs;

        let top_docs2 = assert_search_hits(&reader, &sort2, batch, after.clone())?;
        let score_docs2 = &top_docs2.score_docs;

        // assert that the resulting hits are the same
        assert_equals(&(expected_hits as usize), &top_docs.score_docs.len());
        assert_equals(&top_docs.score_docs.len(), &top_docs2.score_docs.len());
        for i in 0..score_docs.len() {
            let field_doc = &score_docs[i];
            let field_doc2 = &score_docs2[i];
            assert_equals(field(field_doc, 0), field(field_doc2, 0));
            assert_equals(&field_doc.doc(), &field_doc2.doc());
            visited_hits += 1;
        }

        expected_collected_hits += i64::from(num_docs);
        collected_hits += top_docs.total_hits.value();
        collected_hits2 += top_docs2.total_hits.value();
        after = Some(score_docs[expected_hits as usize - 1].clone());
    }
    assert_equals(&visited_hits, &num_docs);
    assert_equals(&expected_collected_hits, &collected_hits);
    // assert that the second sort with optimization collected less or equal hits
    Assert::assert_true(collected_hits >= collected_hits2);

    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testPointsPrunesOnSegmentEntry`.
fn test_points_prunes_on_segment_entry() -> Result<()> {
    prunes_on_segment_entry(TestUtil::get_default_codec(), 2048, long_point_fields)
}

/// Port of `TestSortOptimization.testDVSkipperPrunesOnSegmentEntry`.
fn test_dv_skipper_prunes_on_segment_entry() -> Result<()> {
    prunes_on_segment_entry(
        TestUtil::always_doc_values_format(Arc::new(Lucene90DocValuesFormat::with_interval(16))),
        16,
        skipper_fields,
    )
}

/// Test that CompetitiveDISIBuilder enables pruning from the very start of a
/// subsequent segment.
///
/// `private void testPrunesOnSegmentEntry(Codec codec, int blockSize, BiFunction<...> fieldsBuilder)`.
fn prunes_on_segment_entry(
    codec: Arc<dyn Codec>,
    block_size: i32,
    fields_builder: FieldsBuilder,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = IndexWriterConfig::new();
    config.set_codec(codec)?;
    config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;

    let num_hits = 5;
    let docs_in_first_segment = num_hits * 2;
    for i in 0..docs_in_first_segment {
        let mut doc = Document::new();
        add_all(&mut doc, fields_builder("my_field", i)?);
        writer.add_document(&doc)?;
    }
    writer.flush()?;

    // Many non-competitive values followed by one block of competitive values, so that pruning
    // must be established on segment entry to skip the non-competitive docs.
    let non_competitive_blocks = 20;
    let docs_in_non_competitive_blocks = non_competitive_blocks * block_size;
    for i in 0..docs_in_non_competitive_blocks {
        let mut doc = Document::new();
        add_all(&mut doc, fields_builder("my_field", 1000 + i)?);
        writer.add_document(&doc)?;
    }
    for i in 0..block_size {
        let mut doc = Document::new();
        add_all(&mut doc, fields_builder("my_field", i)?);
        writer.add_document(&doc)?;
    }
    writer.flush()?;

    let reader = open_and_close(&writer)?;
    assert_equals(&2, &leaves_size(&reader)?);

    let sort_field = sf("my_field", SortFieldType::Long)?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let top_docs = searcher.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &TopFieldCollectorManager::with_num_hits(sort_of(vec![sort_field])?, num_hits, 1)?,
    )?;

    assert_equals(&(num_hits as usize), &top_docs.score_docs.len());
    let total_docs = docs_in_first_segment + docs_in_non_competitive_blocks + block_size;
    assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), i64::from(total_docs));

    drop(searcher);
    close_all(&reader, dir)
}

/// `private void assertNonCompetitiveHitsAreSkipped(long collectedHits, long numDocs)`.
fn assert_non_competitive_hits_are_skipped(collected_hits: i64, num_docs: i64) {
    if collected_hits >= num_docs {
        Assert::fail(&format!(
            "Expected some non-competitive hits are skipped; got collected_hits={collected_hits} num_docs={num_docs}"
        ));
    }
}

/// Port of `TestSortOptimization.testStringSortOptimizationBasedPostings`.
fn test_string_sort_optimization_based_postings() -> Result<()> {
    string_sort_optimization(keyword_field)
}

/// Port of `TestSortOptimization.testStringSortOptimizationBasedDVSkipper`.
fn test_string_sort_optimization_based_dv_skipper() -> Result<()> {
    string_sort_optimization(sorted_indexed_field)
}

/// `new BytesRef(Integer.toString(random().nextInt(i, i + 20)))`.
fn random_correlated_value(i: i32) -> BytesRef {
    BytesRef::from_char_sequence(&random().next_int_range(i, i + 20).to_string())
}

/// `private void testStringSortOptimization(BiFunction<...> fieldsBuilder)`.
fn string_sort_optimization(fields_builder: StringFieldBuilder) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let num_docs = LuceneTestCase::at_least(10000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        // Random values that roughly correlate with index order, to make skipper implementation
        // useful
        let value = random_correlated_value(i);
        doc.add(fields_builder("my_field", value)?);
        writer.add_document(&doc)?;
        if i == 7000 {
            writer.flush()?; // multiple segments
        }
    }
    let reader = open_and_close(&writer)?;
    do_test_string_sort_optimization(&reader)?;
    do_test_string_sort_optimization_disabled(&reader)?;
    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testStringSortOptimizationWithMissingValuesBasedPostings`.
fn test_string_sort_optimization_with_missing_values_based_postings() -> Result<()> {
    string_sort_optimization_with_missing_values(keyword_field)
}

/// Port of `TestSortOptimization.testStringSortOptimizationWithMissingValuesBasedDVSkipper`.
fn test_string_sort_optimization_with_missing_values_based_dv_skipper() -> Result<()> {
    string_sort_optimization_with_missing_values(sorted_indexed_field)
}

/// `private void testStringSortOptimizationWithMissingValues(BiFunction<...> fieldsBuilder)`.
fn string_sort_optimization_with_missing_values(fields_builder: StringFieldBuilder) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = IndexWriterConfig::new();
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    // Larger number of documents as some of them are missing values and we still want
    // to have multiple skipper blocks of 4096 entries.
    let num_docs = LuceneTestCase::at_least(30000);
    // one segment with all values missing to start with
    writer.add_document(&Document::new())?;
    for i in 0..num_docs - 2 {
        if i == 7000 {
            writer.flush()?; // multiple segments
        }
        let mut doc = Document::new();
        if random().next_int_bound(2) == 0 {
            // Random values that roughly correlate with index order, to make skipper implementation
            // useful
            let value = random_correlated_value(i);
            doc.add(fields_builder("my_field", value)?);
        }
        writer.add_document(&doc)?;
    }
    writer.flush()?;
    // And one empty segment with all values missing to finish with
    writer.add_document(&Document::new())?;
    let reader = open_and_close(&writer)?;
    do_test_string_sort_optimization(&reader)?;
    close_all(&reader, dir)
}

/// Port of `TestSortOptimization.testStringSortOptimizationFieldMissingInSegmentBasedPostings`.
fn test_string_sort_optimization_field_missing_in_segment_based_postings() -> Result<()> {
    string_sort_optimization_field_missing_in_segment(keyword_field)
}

/// Port of `TestSortOptimization.testStringSortOptimizationFieldMissingInSegmentBasedDVSkipper`.
fn test_string_sort_optimization_field_missing_in_segment_based_dv_skipper() -> Result<()> {
    string_sort_optimization_field_missing_in_segment(sorted_indexed_field)
}

/// Test that when a segment doesn't contain the sort field at all (fieldInfo
/// == null), the optimization still works. All docs in such a segment have
/// missing values, and when missing values are non-competitive the entire
/// segment should be skippable.
///
/// `private void testStringSortOptimizationFieldMissingInSegment(BiFunction<...> fieldsBuilder)`.
fn string_sort_optimization_field_missing_in_segment(
    fields_builder: StringFieldBuilder,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    // Use NoMergePolicy to ensure we have deterministic segment geometry
    let config = IndexWriterConfig::new();
    config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;

    // First segment: a small number of docs with the keyword field, enough to fill the top-N queue.
    let docs_with_field = 20;
    for i in 0..docs_with_field {
        let mut doc = Document::new();
        doc.add(fields_builder(
            "my_field",
            BytesRef::from_char_sequence(&i.to_string()),
        )?);
        writer.add_document(&doc)?;
    }
    writer.flush()?;

    // Second segment: many docs WITHOUT the keyword field. The field doesn't exist in this
    // segment's FieldInfos at all, so every doc has a missing value for the sort field.
    let docs_without_field = LuceneTestCase::at_least(10000);
    for _i in 0..docs_without_field {
        writer.add_document(&Document::new())?;
    }

    let reader = open_and_close(&writer)?;

    let num_docs = i64::from(docs_with_field + docs_without_field);
    let num_hits = 5;
    let match_all = MatchAllDocsQuery::instance();

    {
        // ascending sort with missing-last: once the queue fills from the first segment,
        // all docs in the second segment have non-competitive missing values and should be skipped
        let sort_field = keyword_sort("my_field", false, MissingValue::StringLast)?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // descending sort with missing-last
        let mut sort_field = keyword_sort("my_field", true, MissingValue::StringLast)?;
        let sort = sort_of(vec![sort_field.clone()])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;

        sort_field.set_optimize_sort_with_indexed_data(false);
        let sort = sort_of(vec![sort_field])?;
        let unpruned = assert_search_hits(&reader, &sort, num_hits, None)?;

        CheckHits::check_equal(&match_all, &score_docs(&top_docs), &score_docs(&unpruned));
    }

    {
        // descending sort with missing-first: once the queue fills from the first segment,
        // all docs in the second segment have non-competitive missing values and should be skipped
        let sort_field = keyword_sort("my_field", true, MissingValue::StringFirst)?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // ascending sort with missing-first
        let mut sort_field = keyword_sort("my_field", false, MissingValue::StringFirst)?;
        let sort = sort_of(vec![sort_field.clone()])?;
        let top_docs = assert_search_hits(&reader, &sort, num_hits, None)?;

        sort_field.set_optimize_sort_with_indexed_data(false);
        let sort = sort_of(vec![sort_field])?;
        let unpruned = assert_search_hits(&reader, &sort, num_hits, None)?;

        CheckHits::check_equal(&match_all, &score_docs(&top_docs), &score_docs(&unpruned));
    }

    close_all(&reader, dir)
}

/// `private void doTestStringSortOptimization(DirectoryReader reader)`.
fn do_test_string_sort_optimization(reader: &Arc<dyn DirectoryReader>) -> Result<()> {
    let num_docs = i64::from(reader.num_docs());
    let num_hits = 5;

    {
        // simple ascending sort
        let sort_field = keyword_sort("my_field", false, MissingValue::StringLast)?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_sort(reader, &sort, num_hits, None)?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // simple descending sort
        let sort_field = keyword_sort("my_field", true, MissingValue::StringFirst)?;
        let sort = sort_of(vec![sort_field])?;
        let top_docs = assert_sort(reader, &sort, num_hits, None)?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // ascending sort that returns missing values first
        let sort_field = keyword_sort("my_field", false, MissingValue::StringFirst)?;
        let sort = sort_of(vec![sort_field])?;
        assert_sort(reader, &sort, num_hits, None)?;
    }

    {
        // descending sort that returns missing values last
        let sort_field = keyword_sort("my_field", true, MissingValue::StringLast)?;
        let sort = sort_of(vec![sort_field])?;
        assert_sort(reader, &sort, num_hits, None)?;
    }

    {
        // paging ascending sort with after
        let sort_field = keyword_sort("my_field", false, MissingValue::StringLast)?;
        let sort = sort_of(vec![sort_field])?;
        let after_value = BytesRef::from_char_sequence(if random().next_boolean() {
            "23"
        } else {
            "230000000"
        });
        let after = after_doc(2, vec![SortValue::Bytes(after_value)]);
        let top_docs = assert_sort(reader, &sort, num_hits, Some(after))?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // paging descending sort with after
        let sort_field = keyword_sort("my_field", true, MissingValue::StringFirst)?;
        let sort = sort_of(vec![sort_field])?;
        let after_value = BytesRef::from_char_sequence(if random().next_boolean() {
            "17"
        } else {
            "170000000"
        });
        let after = after_doc(2, vec![SortValue::Bytes(after_value)]);
        let top_docs = assert_sort(reader, &sort, num_hits, Some(after))?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // paging ascending sort with after that returns missing values first
        let sort_field = keyword_sort("my_field", false, MissingValue::StringFirst)?;
        let sort = sort_of(vec![sort_field])?;
        let after_value = BytesRef::from_char_sequence(if random().next_boolean() {
            "23"
        } else {
            "230000000"
        });
        let after = after_doc(2, vec![SortValue::Bytes(after_value)]);
        let top_docs = assert_sort(reader, &sort, num_hits, Some(after))?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // paging descending sort with after that returns missing values first
        let sort_field = keyword_sort("my_field", true, MissingValue::StringLast)?;
        let sort = sort_of(vec![sort_field])?;
        let after_value = BytesRef::from_char_sequence(if random().next_boolean() {
            "17"
        } else {
            "170000000"
        });
        let after = after_doc(2, vec![SortValue::Bytes(after_value)]);
        let top_docs = assert_sort(reader, &sort, num_hits, Some(after))?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // test that if there is the secondary sort on _score, hits are still skipped
        let sort_field = keyword_sort("my_field", false, MissingValue::StringLast)?;
        let sort = sort_of(vec![sort_field, SortField::FIELD_SCORE])?;
        let top_docs = assert_sort(reader, &sort, num_hits, None)?;
        assert_non_competitive_hits_are_skipped(top_docs.total_hits.value(), num_docs);
    }

    {
        // test that if string field is a secondary sort, no optimization is run
        let sort_field = keyword_sort("my_field", false, MissingValue::StringLast)?;
        let sort = sort_of(vec![SortField::FIELD_SCORE, sort_field])?;
        let top_docs = assert_sort(reader, &sort, num_hits, None)?;
        assert_equals(&top_docs.total_hits.value(), &num_docs); // assert that all documents were collected => optimization was not run
    }
    Ok(())
}

/// `public void doTestStringSortOptimizationDisabled(DirectoryReader reader)`.
fn do_test_string_sort_optimization_disabled(reader: &Arc<dyn DirectoryReader>) -> Result<()> {
    let mut sort_field = keyword_sort("my_field", false, MissingValue::StringLast)?;
    sort_field.set_optimize_sort_with_indexed_data(false);

    let sort = sort_of(vec![sort_field])?;
    let num_docs = i64::from(reader.num_docs());
    let num_hits = 5;

    let top_docs = assert_search_hits(reader, &sort, num_hits, None)?;
    assert_equals(&num_docs, &top_docs.total_hits.value());
    Ok(())
}

/// `private TopDocs assertSort(DirectoryReader reader, Sort sort, int n, FieldDoc after)`.
fn assert_sort(
    reader: &Arc<dyn DirectoryReader>,
    sort: &Sort,
    n: i32,
    after: Option<FieldDoc>,
) -> Result<TopFieldDocs> {
    let top_docs = assert_search_hits(reader, sort, n, after.clone())?;
    let mut sort_field2: Vec<SortField> = sort.fields().to_vec();
    // A secondary sort on reverse doc ID is the best way to catch bugs if the comparator filters
    // too aggressively
    sort_field2.push(SortField::new_reverse(None, SortFieldType::Doc, true)?);
    let mut after2 = None;
    if let Some(after) = after {
        let mut after_fields2 = after.fields.clone().expect("after.fields");
        after_fields2.push(SortValue::Int(i32::MAX));
        after2 = Some(FieldDoc::with_fields(
            after.doc(),
            after.score(),
            after_fields2,
        ));
    }
    assert_search_hits(reader, &sort_of(sort_field2)?, n, after2)?;
    Ok(top_docs)
}

/// `private TopFieldDocs assertSearchHits(DirectoryReader reader, Sort sort, int n, FieldDoc after)`.
fn assert_search_hits(
    reader: &Arc<dyn DirectoryReader>,
    sort: &Sort,
    n: i32,
    after: Option<FieldDoc>,
) -> Result<TopFieldDocs> {
    assert_search_hits_query(
        reader,
        Arc::new(MatchAllDocsQuery::instance()),
        sort,
        n,
        after,
    )
}

/// `private TopFieldDocs assertSearchHits(DirectoryReader reader, Query query, Sort sort, int n, FieldDoc after)`.
fn assert_search_hits_query(
    reader: &Arc<dyn DirectoryReader>,
    query: Arc<dyn Query>,
    sort: &Sort,
    n: i32,
    after: Option<FieldDoc>,
) -> Result<TopFieldDocs> {
    // single threaded and no bulk-scoring optimizations so that the total hit count is
    // deterministic and can be reasoned about
    let mut searcher = ScorerIndexSearcher::new(Arc::clone(reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None);

    let optimized_top_docs = searcher.search_with_manager(
        Arc::clone(&query),
        &TopFieldCollectorManager::new(sort.clone(), n, after.clone(), n)?,
    )?;

    let is_match_all = query.as_any().is::<MatchAllDocsQuery>();
    if is_match_all {
        // Searcher that hides index structures to force a linear scan of the sort fields, and make
        // sure that the same hits are returned
        // We can only do that on a MatchAllDocsQuery, otherwise the query won't have data structures
        // to operate on :)
        let mut unoptimized_searcher = LuceneTestCase::new_searcher_threads(
            no_index_directory_reader(Arc::clone(reader))?,
            true,
            true,
            false,
        );
        unoptimized_searcher.set_query_cache(None);
        let unoptimized_top_docs = unoptimized_searcher.search_with_manager(
            Arc::clone(&query),
            &TopFieldCollectorManager::new(sort.clone(), n, after.clone(), n)?,
        )?;
        CheckHits::check_equal(
            &*query,
            &score_docs(&unoptimized_top_docs),
            &score_docs(&optimized_top_docs),
        );
    }

    // Use the random searcher in combination with DummyMatchAllDocsQuery to make sure we test the
    // behavior when the bulk scorer reads ahead
    let random_query: Arc<dyn Query> = if is_match_all {
        Arc::new(ReadAheadMatchAllDocsQuery::new())
    } else {
        Arc::clone(&query)
    };
    // Random IndexSearcher to make sure that enabling threading and bulk-scoring optimizations
    // doesn't affect the returned hits
    let mut random_searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
    random_searcher.set_query_cache(None);
    let random_top_docs = random_searcher.search_with_manager(
        random_query,
        &TopFieldCollectorManager::new(sort.clone(), n, after, n)?,
    )?;
    CheckHits::check_equal(
        &*query,
        &score_docs(&optimized_top_docs),
        &score_docs(&random_top_docs),
    );

    Ok(optimized_top_docs)
}

// -----------------------------------------------------------------------------
// NoIndexDirectoryReader
// -----------------------------------------------------------------------------

/// `new NoIndexDirectoryReader(in)`: `private static final class
/// NoIndexDirectoryReader extends FilterDirectoryReader`.
fn no_index_directory_reader(r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(NoIndexSubReaderWrapper),
        Arc::new(NoIndexDirectoryReaderWrapper),
    )?))
}

/// The anonymous `SubReaderWrapper` of `NoIndexDirectoryReader`.
struct NoIndexSubReaderWrapper;

impl SubReaderWrapper for NoIndexSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(NoIndexLeafReader {
            inner: FilterLeafReader::new(reader)?,
        }))
    }
}

/// `NoIndexDirectoryReader.doWrapDirectoryReader`, which throws, and its
/// `getReaderCacheHelper()`, which returns `null`.
struct NoIndexDirectoryReaderWrapper;

impl DirectoryReaderWrapper for NoIndexDirectoryReaderWrapper {
    fn wrap(&self, _inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "NoIndexDirectoryReader"
    }

    fn reader_cache_helper_is_null(&self) -> bool {
        true
    }
}

/// `private static final class NoIndexLeafReader extends FilterLeafReader`.
struct NoIndexLeafReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for NoIndexLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoIndexLeafReader").finish_non_exhaustive()
    }
}

impl fmt::Display for NoIndexLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for NoIndexLeafReader {
    fn core(&self) -> &rucene::index::IndexReaderCore {
        LeafReader::core(&self.inner)
    }
    fn filter_leaf_reader_delegate(&self) -> Option<&Arc<dyn LeafReader>> {
        LeafReader::filter_leaf_reader_delegate(&self.inner)
    }
    fn term_vectors(&self) -> Result<Box<dyn rucene::index::TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }
    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }
    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }
    fn stored_fields(&self) -> Result<Box<dyn rucene::index::StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }
    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn terms(&self, _field: &str) -> Result<Option<Box<dyn rucene::index::Terms>>> {
        Ok(None)
    }
    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }
    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }
    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }
    fn get_norm_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }
    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }
    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }
    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }
    fn get_field_infos(&self) -> FieldInfos {
        let infos = self.inner.get_field_infos();
        let mut new_infos: Vec<FieldInfo> = Vec::with_capacity(infos.size());
        for fi in infos.iter() {
            let no_index_fi = FieldInfo::new_full(
                fi.name.clone(),
                fi.number,
                false,
                false,
                false,
                IndexOptions::NONE,
                fi.get_doc_values_type(),
                fi.doc_values_skip_index_type(),
                fi.get_doc_values_gen(),
                fi.attributes(),
                0,
                0,
                0,
                0,
                VectorEncoding::FLOAT32,
                VectorSimilarityFunction::DOT_PRODUCT,
                fi.is_soft_deletes_field(),
                fi.is_parent_field(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
            new_infos.push(no_index_fi);
        }
        FieldInfos::new(new_infos).unwrap_or_else(|e| panic!("{e}"))
    }
    fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
        LeafReader::get_live_docs(&self.inner)
    }
    fn get_point_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn rucene::index::PointValues>>> {
        Ok(None)
    }
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }
    fn get_meta_data(&self) -> rucene::index::LeafMetaData {
        self.inner.get_meta_data()
    }
}
