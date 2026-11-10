//! Port of `lucene/core/src/test/org/apache/lucene/search/TestIndexSortSortedNumericDocValuesRangeQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    Document, IntPoint, LongPoint, SortedNumericDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::search::field_exists_query::FieldExistsQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::index_sort_sorted_numeric_doc_values_range_query::IndexSortSortedNumericDocValuesRangeQuery;
use rucene::search::sort::MissingValue;
use rucene::search::{
    query_to_string, BooleanQueryBuilder, MatchNoDocsQuery, Query, ScoreMode, Sort, SortField,
    SortFieldType, SortedNumericSelectorType, SortedNumericSortField, Weight,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::dummy_total_hit_count_collector::DummyTotalHitCountCollector;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::SuppressCodecs;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::assert_query_not_equals;
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_null, assert_null, class_annotated, random, test,
};

/// `TestIndexSortSortedNumericDocValuesRangeQuery`
/// (`@LuceneTestCase.SuppressCodecs(value = "SimpleText")`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.search.TestIndexSortSortedNumericDocValuesRangeQuery",
        vec![
            test(
                "testSameHitsAsPointRangeQuery",
                test_same_hits_as_point_range_query,
            ),
            test(
                "testSameHitsAsPointRangeQueryIntSort",
                test_same_hits_as_point_range_query_int_sort,
            ),
            test("testEquals", test_equals),
            test("testToString", test_to_string),
            test(
                "testIndexSortDocValuesWithEvenLength",
                test_index_sort_doc_values_with_even_length,
            ),
            test(
                "testIndexSortDocValuesWithOddLength",
                test_index_sort_doc_values_with_odd_length,
            ),
            test(
                "testIndexSortDocValuesWithSingleValue",
                test_index_sort_doc_values_with_single_value,
            ),
            test("testIndexSortMissingValues", test_index_sort_missing_values),
            test("testNoDocuments", test_no_documents),
            test("testRewriteExhaustiveRange", test_rewrite_exhaustive_range),
            test("testRewriteFallbackQuery", test_rewrite_fallback_query),
            test("testNoIndexSort", test_no_index_sort),
            test("testIndexSortOnWrongField", test_index_sort_on_wrong_field),
            test("testOtherSortTypes", test_other_sort_types),
            test("testMultiDocValues", test_multi_doc_values),
            test("testFallbackCount", test_fallback_count),
            test("testCompareCount", test_compare_count),
            test("testCountBoundary", test_count_boundary),
            test("testCountWithBkdAsc", test_count_with_bkd_asc),
            test("testCountWithBkdDesc", test_count_with_bkd_desc),
            test("testRandomCountWithBkdAsc", test_random_count_with_bkd_asc),
            test(
                "testRandomCountWithBkdDesc",
                test_random_count_with_bkd_desc,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())))
}

/// `new Sort(sortField)`.
fn sort_of(sort_field: SortedNumericSortField) -> Result<Sort> {
    Sort::new_fields(vec![sort_field.into_sort_field()])
}

/// `new RandomIndexWriter(random(), dir, iwc)` with `iwc.setIndexSort(sort)`.
fn sorted_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    sort: Sort,
) -> Result<RandomIndexWriter> {
    let mut iwc = mock_iwc();
    iwc.set_index_sort(sort)?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)
}

fn test_same_hits_as_point_range_query() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _iter in 0..iters {
        let mut dir = LuceneTestCase::new_directory();

        let mut iwc = mock_iwc();
        let reverse = random().next_boolean();
        let enable_missing_value = random().next_boolean();
        let mut missing_value = None;
        if enable_missing_value {
            missing_value = Some(MissingValue::Long(if random().next_boolean() {
                TestUtil::next_long(&*random(), -100, 10000)
            } else if random().next_boolean() {
                i64::MIN
            } else {
                i64::MAX
            }));
        }
        let sort_field = SortedNumericSortField::with_missing_value(
            "dv",
            SortFieldType::Long,
            reverse,
            SortedNumericSelectorType::MIN,
            missing_value,
        )?;
        iwc.set_index_sort(sort_of(sort_field)?)?;

        let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

        let num_docs = LuceneTestCase::at_least(100);
        for _i in 0..num_docs {
            let mut doc = Document::new();
            let num_values = TestUtil::next_int(&*random(), 0, 1);
            for _j in 0..num_values {
                let value = TestUtil::next_long(&*random(), -100, 10000);
                doc.add(Box::new(SortedNumericDocValuesField::new("dv", value)));
                doc.add(Box::new(LongPoint::new("idx", &[value])?));
            }
            iw.add_document(&doc)?;
        }
        if random().next_boolean() {
            iw.delete_documents_by_query(LongPoint::new_range_query("idx", &[0], &[10])?)?;
        }
        let reader = iw.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        iw.close()?;

        for _i in 0..100 {
            let min = if random().next_boolean() {
                i64::MIN
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let max = if random().next_boolean() {
                i64::MAX
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let q1 = LongPoint::new_range_query("idx", &[min], &[max])?;
            let q2 = create_query("dv", min, max);
            assert_same_hits(&searcher, q1, q2, false)?;
        }

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_same_hits_as_point_range_query_int_sort() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _iter in 0..iters {
        let mut dir = LuceneTestCase::new_directory();

        let mut iwc = mock_iwc();
        let reverse = random().next_boolean();
        let enable_missing_value = random().next_boolean();
        let mut missing_value = None;
        if enable_missing_value {
            missing_value = Some(MissingValue::Int(if random().next_boolean() {
                TestUtil::next_int(&*random(), -100, 10000)
            } else if random().next_boolean() {
                i32::MIN
            } else {
                i32::MAX
            }));
        }
        let sort_field = SortedNumericSortField::with_missing_value(
            "dv",
            SortFieldType::Int,
            reverse,
            SortedNumericSelectorType::MIN,
            missing_value,
        )?;
        iwc.set_index_sort(sort_of(sort_field)?)?;

        let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

        let num_docs = LuceneTestCase::at_least(100);
        for _i in 0..num_docs {
            let mut doc = Document::new();
            let num_values = TestUtil::next_int(&*random(), 0, 1);
            for _j in 0..num_values {
                let value = TestUtil::next_int(&*random(), -100, 10000);
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "dv",
                    i64::from(value),
                )));
                doc.add(Box::new(IntPoint::new("idx", &[value])?));
            }
            iw.add_document(&doc)?;
        }
        if random().next_boolean() {
            iw.delete_documents_by_query(IntPoint::new_range_query("idx", &[0], &[10])?)?;
        }
        let reader = iw.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        iw.close()?;

        for _i in 0..100 {
            let min = if random().next_boolean() {
                i32::MIN
            } else {
                TestUtil::next_int(&*random(), -100, 10000)
            };
            let max = if random().next_boolean() {
                i32::MAX
            } else {
                TestUtil::next_int(&*random(), -100, 10000)
            };
            let q1 = IntPoint::new_range_query("idx", &[min], &[max])?;
            let q2 = create_query("dv", i64::from(min), i64::from(max));
            assert_same_hits(&searcher, q1, q2, false)?;
        }

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

/// `Sort.INDEXORDER`.
fn index_order() -> Sort {
    Sort::new_fields(vec![SortField::FIELD_DOC.clone()]).expect("one sort field")
}

fn assert_same_hits(
    searcher: &IndexSearcher,
    q1: Arc<dyn Query>,
    q2: Arc<dyn Query>,
    scores: bool,
) -> Result<()> {
    let max_doc = searcher.get_index_reader().max_doc();
    let sort = if scores { Sort::new() } else { index_order() };
    let td1 = searcher.search_with_sort(q1, max_doc, &sort)?;
    let td2 = searcher.search_with_sort(q2, max_doc, &sort)?;
    assert_equals(&td1.total_hits.value(), &td2.total_hits.value());
    for i in 0..td1.score_docs.len() {
        assert_equals(
            &td1.score_docs[i].score_doc.doc,
            &td2.score_docs[i].score_doc.doc,
        );
        if scores {
            assert_equals_delta(
                f64::from(td1.score_docs[i].score_doc.score),
                f64::from(td2.score_docs[i].score_doc.score),
                10e-7,
            );
        }
    }
    Ok(())
}

fn test_equals() -> Result<()> {
    let q1 = create_query("foo", 3, 5);
    QueryUtils::check_equal(&*q1, &*create_query("foo", 3, 5));
    QueryUtils::check_unequal(&*q1, &*create_query("foo", 3, 6));
    QueryUtils::check_unequal(&*q1, &*create_query("foo", 4, 5));
    QueryUtils::check_unequal(&*q1, &*create_query("bar", 3, 5));
    Ok(())
}

fn test_to_string() -> Result<()> {
    let q1 = create_query("foo", 3, 5);
    assert_equals("foo:[3 TO 5]", query_to_string(&*q1).as_str());
    assert_equals("[3 TO 5]", q1.to_query_string("foo").as_str());
    assert_equals("foo:[3 TO 5]", q1.to_query_string("bar").as_str());
    Ok(())
}

fn test_index_sort_doc_values_with_even_length() -> Result<()> {
    for ty in [SortFieldType::Int, SortFieldType::Long] {
        index_sort_doc_values_with_even_length(true, ty)?;
        index_sort_doc_values_with_even_length(false, ty)?;
    }
    Ok(())
}

/// `testIndexSortDocValuesWithEvenLength(boolean reverse, SortField.Type type)`.
fn index_sort_doc_values_with_even_length(reverse: bool, ty: SortFieldType) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let index_sort = sort_of(SortedNumericSortField::with_reverse("field", ty, reverse)?)?;
    let writer = sorted_writer(&dir, index_sort)?;

    writer.add_document(&create_document("field", -80))?;
    writer.add_document(&create_document("field", -5))?;
    writer.add_document(&create_document("field", 0))?;
    writer.add_document(&create_document("field", 0))?;
    writer.add_document(&create_document("field", 30))?;
    writer.add_document(&create_document("field", 35))?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // Test ranges consisting of one value.
    assert_number_of_hits(&searcher, create_query("field", -80, -80), 1)?;
    assert_number_of_hits(&searcher, create_query("field", -5, -5), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 0, 0), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 30, 30), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 35, 35), 1)?;

    assert_number_of_hits(&searcher, create_query("field", -90, -90), 0)?;
    assert_number_of_hits(&searcher, create_query("field", 5, 5), 0)?;
    assert_number_of_hits(&searcher, create_query("field", 40, 40), 0)?;

    // Test the lower end of the document value range.
    assert_number_of_hits(&searcher, create_query("field", -90, -4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", -80, -4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", -70, -4), 1)?;
    assert_number_of_hits(&searcher, create_query("field", -80, -5), 2)?;

    // Test the upper end of the document value range.
    assert_number_of_hits(&searcher, create_query("field", 25, 34), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 25, 35), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 25, 36), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 30, 35), 2)?;

    // Test multiple occurrences of the same value.
    assert_number_of_hits(&searcher, create_query("field", -4, 4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", -4, 0), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 0, 4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 0, 30), 3)?;

    // Test ranges that span all documents.
    assert_number_of_hits(&searcher, create_query("field", -80, 35), 6)?;
    assert_number_of_hits(&searcher, create_query("field", -90, 40), 6)?;

    writer.close()?;
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn assert_number_of_hits(
    searcher: &IndexSearcher,
    query: Arc<dyn Query>,
    number_of_hits: i32,
) -> Result<()> {
    assert_equals(
        &number_of_hits,
        &searcher.search_with_manager(
            Arc::clone(&query),
            &DummyTotalHitCountCollector::create_manager(),
        )?,
    );
    assert_equals(&number_of_hits, &searcher.count(query)?);
    Ok(())
}

fn test_index_sort_doc_values_with_odd_length() -> Result<()> {
    index_sort_doc_values_with_odd_length(false)?;
    index_sort_doc_values_with_odd_length(true)
}

/// `testIndexSortDocValuesWithOddLength(boolean reverse)`.
fn index_sort_doc_values_with_odd_length(reverse: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let index_sort = sort_of(SortedNumericSortField::with_reverse(
        "field",
        SortFieldType::Long,
        reverse,
    )?)?;
    let writer = sorted_writer(&dir, index_sort)?;

    writer.add_document(&create_document("field", -80))?;
    writer.add_document(&create_document("field", -5))?;
    writer.add_document(&create_document("field", 0))?;
    writer.add_document(&create_document("field", 0))?;
    writer.add_document(&create_document("field", 5))?;
    writer.add_document(&create_document("field", 30))?;
    writer.add_document(&create_document("field", 35))?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // Test ranges consisting of one value.
    assert_number_of_hits(&searcher, create_query("field", -80, -80), 1)?;
    assert_number_of_hits(&searcher, create_query("field", -5, -5), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 0, 0), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 5, 5), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 30, 30), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 35, 35), 1)?;

    assert_number_of_hits(&searcher, create_query("field", -90, -90), 0)?;
    assert_number_of_hits(&searcher, create_query("field", 6, 6), 0)?;
    assert_number_of_hits(&searcher, create_query("field", 40, 40), 0)?;

    // Test the lower end of the document value range.
    assert_number_of_hits(&searcher, create_query("field", -90, -4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", -80, -4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", -70, -4), 1)?;
    assert_number_of_hits(&searcher, create_query("field", -80, -5), 2)?;

    // Test the upper end of the document value range.
    assert_number_of_hits(&searcher, create_query("field", 25, 34), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 25, 35), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 25, 36), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 30, 35), 2)?;

    // Test multiple occurrences of the same value.
    assert_number_of_hits(&searcher, create_query("field", -4, 4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", -4, 0), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 0, 4), 2)?;
    assert_number_of_hits(&searcher, create_query("field", 0, 30), 4)?;

    // Test ranges that span all documents.
    assert_number_of_hits(&searcher, create_query("field", -80, 35), 7)?;
    assert_number_of_hits(&searcher, create_query("field", -90, 40), 7)?;

    writer.close()?;
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_index_sort_doc_values_with_single_value() -> Result<()> {
    index_sort_doc_values_with_single_value(false)?;
    index_sort_doc_values_with_single_value(true)
}

fn index_sort_doc_values_with_single_value(reverse: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let index_sort = sort_of(SortedNumericSortField::with_reverse(
        "field",
        SortFieldType::Long,
        reverse,
    )?)?;
    let writer = sorted_writer(&dir, index_sort)?;

    writer.add_document(&create_document("field", 42))?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    assert_number_of_hits(&searcher, create_query("field", 42, 43), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 42, 42), 1)?;
    assert_number_of_hits(&searcher, create_query("field", 41, 41), 0)?;
    assert_number_of_hits(&searcher, create_query("field", 43, 43), 0)?;

    writer.close()?;
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_index_sort_missing_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let mut iwc = mock_iwc();
    let sort_field = SortedNumericSortField::with_missing_value(
        "field",
        SortFieldType::Long,
        false,
        SortedNumericSelectorType::MIN,
        Some(MissingValue::Long(random().next_long())),
    )?;
    iwc.set_index_sort(sort_of(sort_field)?)?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    writer.add_document(&create_document("field", -80))?;
    writer.add_document(&create_document("field", -5))?;
    writer.add_document(&create_document("field", 0))?;
    writer.add_document(&create_document("field", 35))?;

    writer.add_document(&create_document("other-field", 0))?;
    writer.add_document(&create_document("other-field", 10))?;
    writer.add_document(&create_document("other-field", 20))?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    assert_number_of_hits(&searcher, create_query("field", -70, 0), 2)?;
    assert_number_of_hits(&searcher, create_query("field", -2, 35), 2)?;

    assert_number_of_hits(&searcher, create_query("field", -80, 35), 4)?;
    assert_number_of_hits(&searcher, create_query("field", i64::MIN, i64::MAX), 4)?;

    writer.close()?;
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_no_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    writer.add_document(&Document::new())?;
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query = create_query("foo", 2, 4);
    let w = searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    assert_null(&w.scorer(&leaves[0])?.map(|_| ()));

    writer.close()?;
    drop((w, searcher));
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `query.rewrite(searcher)`: Rucene's `None` is Java's `this`.
fn rewrite_once(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<Arc<dyn Query>> {
    Ok(query
        .rewrite(searcher)?
        .unwrap_or_else(|| Arc::clone(query)))
}

fn test_rewrite_exhaustive_range() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    writer.add_document(&Document::new())?;
    let reader = writer.get_reader()?;

    let query = create_query("field", i64::MIN, i64::MAX);
    let rewritten_query = rewrite_once(&query, &LuceneTestCase::new_searcher(Arc::clone(&reader)))?;
    let expected: Arc<dyn Query> = Arc::new(FieldExistsQuery::new("field"));
    assert_query_equals(&expected, &rewritten_query);

    writer.close()?;
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_rewrite_fallback_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    writer.add_document(&Document::new())?;
    let reader = writer.get_reader()?;

    // Create an (unrealistic) fallback query that is sure to be rewritten.
    let fallback_query: Arc<dyn Query> = Arc::new(BooleanQueryBuilder::new().build());
    let query: Arc<dyn Query> = Arc::new(IndexSortSortedNumericDocValuesRangeQuery::new(
        "field",
        1,
        42,
        fallback_query,
    ));

    let rewritten_query = rewrite_once(&query, &LuceneTestCase::new_searcher(Arc::clone(&reader)))?;
    assert_query_not_equals(&query, &rewritten_query);
    // MatcherAssert.assertThat(rewrittenQuery, instanceOf(IndexSortSortedNumericDocValuesRangeQuery.class))
    let range_query = rewritten_query
        .as_any()
        .downcast_ref::<IndexSortSortedNumericDocValuesRangeQuery>();
    randomizedtesting::junit::Assert::assert_true(range_query.is_some());

    let range_query = range_query.expect("instanceOf checked");
    let expected: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
    assert_query_equals(&expected, range_query.get_fallback_query());

    writer.close()?;
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Test that the index sort optimization not activated if there is no index sort.
fn test_no_index_sort() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    writer.add_document(&create_document("field", 0))?;

    index_sort_optimization_deactivated(&writer)?;

    writer.close()?;
    dir.close()?;
    Ok(())
}

/// Test that the index sort optimization is not activated when the sort is on the wrong field.
fn test_index_sort_on_wrong_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let index_sort = sort_of(SortedNumericSortField::new(
        "other-field",
        SortFieldType::Long,
    )?)?;
    let writer = sorted_writer(&dir, index_sort)?;
    writer.add_document(&create_document("field", 0))?;

    index_sort_optimization_deactivated(&writer)?;

    writer.close()?;
    dir.close()?;
    Ok(())
}

fn test_other_sort_types() -> Result<()> {
    for ty in [SortFieldType::Float, SortFieldType::Double] {
        let mut dir = LuceneTestCase::new_directory();

        let index_sort = sort_of(SortedNumericSortField::new("field", ty)?)?;
        let writer = sorted_writer(&dir, index_sort)?;
        writer.add_document(&create_document("field", 0))?;

        index_sort_optimization_deactivated(&writer)?;

        writer.close()?;
        dir.close()?;
    }
    Ok(())
}

/// Test that the index sort optimization is not activated when some documents have multiple
/// values.
fn test_multi_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let index_sort = sort_of(SortedNumericSortField::new("field", SortFieldType::Long)?)?;
    let writer = sorted_writer(&dir, index_sort)?;

    let mut doc = Document::new();
    doc.add(Box::new(SortedNumericDocValuesField::new("field", 0)));
    doc.add(Box::new(SortedNumericDocValuesField::new("field", 10)));
    writer.add_document(&doc)?;

    index_sort_optimization_deactivated(&writer)?;

    writer.close()?;
    dir.close()?;
    Ok(())
}

/// `testIndexSortOptimizationDeactivated(RandomIndexWriter writer)`.
fn index_sort_optimization_deactivated(writer: &RandomIndexWriter) -> Result<()> {
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = create_query("field", 0, 0);
    let weight = query.create_weight(&searcher, ScoreMode::TOP_SCORES, 1.0)?;

    // Check that the two-phase iterator is not null, indicating that we've fallen
    // back to SortedNumericDocValuesField.newSlowRangeQuery.
    for context in &Arc::clone(searcher.get_index_reader()).leaves()? {
        let mut scorer = weight.scorer(context)?.expect("scorer");
        assert_not_null(&scorer.two_phase_iterator().map(|_| ()));
    }

    drop((weight, searcher));
    reader.close()?;
    Ok(())
}

fn test_fallback_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_sort = sort_of(SortedNumericSortField::new("field", SortFieldType::Long)?)?;
    let writer = sorted_writer(&dir, index_sort)?;
    let mut doc = Document::new();
    doc.add(Box::new(SortedNumericDocValuesField::new("field", 10)));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // we use an unrealistic query that exposes its own Weight#count
    let fallback_query: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
    // the index is not sorted on this field, the fallback query is used
    let query = IndexSortSortedNumericDocValuesRangeQuery::new("another", 1, 42, fallback_query);
    let weight = query.create_weight(&searcher, ScoreMode::COMPLETE, 1.0)?;
    for context in searcher.get_leaf_contexts() {
        assert_equals(&0, &weight.count(context)?);
    }

    writer.close()?;
    drop((weight, searcher));
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_compare_count() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _iter in 0..iters {
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = mock_iwc();
        let enable_missing_value = random().next_boolean();
        let mut missing_value = None;
        if enable_missing_value {
            missing_value = Some(MissingValue::Long(if random().next_boolean() {
                TestUtil::next_long(&*random(), -100, 10000)
            } else if random().next_boolean() {
                i64::MIN
            } else {
                i64::MAX
            }));
        }
        let sort_field = SortedNumericSortField::with_missing_value(
            "field",
            SortFieldType::Long,
            false,
            SortedNumericSelectorType::MIN,
            missing_value,
        )?;
        iwc.set_index_sort(sort_of(sort_field)?)?;

        let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

        let num_docs = LuceneTestCase::at_least(100);
        for _i in 0..num_docs {
            let mut doc = Document::new();
            let num_values = TestUtil::next_int(&*random(), 0, 1);
            for _j in 0..num_values {
                let value = TestUtil::next_long(&*random(), -100, 10000);
                doc = create_sndv_and_point_document("field", value)?;
            }
            writer.add_document(&doc)?;
        }

        if random().next_boolean() {
            writer.delete_documents_by_query(LongPoint::new_range_query("field", &[0], &[10])?)?;
        }

        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        writer.close()?;

        for _i in 0..100 {
            let min = if random().next_boolean() {
                i64::MIN
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let max = if random().next_boolean() {
                i64::MAX
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let q1 = LongPoint::new_range_query("field", &[min], &[max])?;

            let fallback_query = LongPoint::new_range_query("field", &[min], &[max])?;
            let q2 =
                IndexSortSortedNumericDocValuesRangeQuery::new("field", min, max, fallback_query);
            let weight1 = q1.create_weight(&searcher, ScoreMode::COMPLETE, 1.0)?;
            let weight2 = q2.create_weight(&searcher, ScoreMode::COMPLETE, 1.0)?;
            assert_same_count(&*weight1, &*weight2, &searcher)?;
        }

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn assert_same_count(
    weight1: &dyn Weight,
    weight2: &dyn Weight,
    searcher: &IndexSearcher,
) -> Result<()> {
    for context in searcher.get_leaf_contexts() {
        assert_equals(&weight1.count(context)?, &weight2.count(context)?);
    }
    Ok(())
}

fn test_count_boundary() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = mock_iwc();
    let use_lower = random().next_boolean();
    let lower_value: i64 = 1;
    let upper_value: i64 = 100;
    let sort_field = SortedNumericSortField::with_missing_value(
        "field",
        SortFieldType::Long,
        false,
        SortedNumericSelectorType::MIN,
        Some(MissingValue::Long(if use_lower {
            lower_value
        } else {
            upper_value
        })),
    )?;
    iwc.set_index_sort(sort_of(sort_field)?)?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    writer.add_document(&create_sndv_and_point_document(
        "field",
        random().next_long_range(lower_value, upper_value),
    )?)?;
    writer.add_document(&create_sndv_and_point_document(
        "field",
        random().next_long_range(lower_value, upper_value),
    )?)?;
    // missingValue
    writer.add_document(&create_missing_value_document()?)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let fallback_query = LongPoint::new_range_query("field", &[lower_value], &[upper_value])?;
    let query = IndexSortSortedNumericDocValuesRangeQuery::new(
        "field",
        lower_value,
        upper_value,
        fallback_query,
    );
    let weight = query.create_weight(&searcher, ScoreMode::COMPLETE, 1.0)?;
    let mut count = 0;
    for context in searcher.get_leaf_contexts() {
        count += weight.count(context)?;
    }
    assert_equals(&2, &count);

    writer.close()?;
    drop((weight, searcher));
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn create_missing_value_document() -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("foo", "fox".into(), Store::YES)?));
    Ok(doc)
}

fn create_sndv_and_point_document(field: &str, value: i64) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(SortedNumericDocValuesField::new(field, value)));
    doc.add(Box::new(LongPoint::new(field, &[value])?));
    Ok(doc)
}

fn create_document(field: &str, value: i64) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(SortedNumericDocValuesField::new(field, value)));
    doc
}

fn create_query(field: &str, lower_value: i64, upper_value: i64) -> Arc<dyn Query> {
    let fallback_query =
        SortedNumericDocValuesField::new_slow_range_query(field, lower_value, upper_value);
    Arc::new(IndexSortSortedNumericDocValuesRangeQuery::new(
        field,
        lower_value,
        upper_value,
        fallback_query,
    ))
}

fn test_count_with_bkd_asc() -> Result<()> {
    do_test_count_with_bkd(false)
}

fn test_count_with_bkd_desc() -> Result<()> {
    do_test_count_with_bkd(true)
}

/// `new IndexSortSortedNumericDocValuesRangeQuery(field, lo, hi, LongPoint.newRangeQuery(field, lo, hi))`
/// weighted with `COMPLETE`, then each leaf's count equals `expected`.
fn assert_bkd_count(
    searcher: &IndexSearcher,
    filed_name: &str,
    lo: i64,
    hi: i64,
    expected: i32,
) -> Result<()> {
    let fallback_query = LongPoint::new_range_query(filed_name, &[lo], &[hi])?;
    let query = IndexSortSortedNumericDocValuesRangeQuery::new(filed_name, lo, hi, fallback_query);
    let weight = query.create_weight(searcher, ScoreMode::COMPLETE, 1.0)?;
    for context in searcher.get_leaf_contexts() {
        assert_equals(&expected, &weight.count(context)?);
    }
    Ok(())
}

/// `doTestCountWithBkd(boolean reverse)`.
fn do_test_count_with_bkd(reverse: bool) -> Result<()> {
    let filed_name = "field";
    let mut dir = LuceneTestCase::new_directory();
    let index_sort = sort_of(SortedNumericSortField::with_reverse(
        filed_name,
        SortFieldType::Long,
        reverse,
    )?)?;
    let writer = sorted_writer(&dir, index_sort)?;
    add_doc_with_bkd(&writer, filed_name, 7, 500)?;
    add_doc_with_bkd(&writer, filed_name, 5, 600)?;
    add_doc_with_bkd(&writer, filed_name, 11, 700)?;
    add_doc_with_bkd(&writer, filed_name, 13, 800)?;
    add_doc_with_bkd(&writer, filed_name, 9, 900)?;
    writer.flush()?;
    writer.force_merge(1)?;
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // Both bounds exist in the dataset
    assert_bkd_count(&searcher, filed_name, 7, 9, 1400)?;

    // Both bounds do not exist in the dataset
    assert_bkd_count(&searcher, filed_name, 6, 10, 1400)?;

    // Min bound exists in the dataset, not the max
    assert_bkd_count(&searcher, filed_name, 7, 10, 1400)?;

    // Min bound doesn't exist in the dataset, max does
    assert_bkd_count(&searcher, filed_name, 6, 9, 1400)?;

    // Min bound is the min value of the dataset
    assert_bkd_count(&searcher, filed_name, 5, 8, 1100)?;

    // Min bound is less than min value of the dataset
    assert_bkd_count(&searcher, filed_name, 4, 8, 1100)?;

    // Max bound is the max value of the dataset
    assert_bkd_count(&searcher, filed_name, 10, 13, 1500)?;

    // Max bound is greater than max value of the dataset
    assert_bkd_count(&searcher, filed_name, 10, 14, 1500)?;

    // Everything matches
    assert_bkd_count(&searcher, filed_name, 2, 14, 3500)?;

    // Bounds equal to min/max values of the dataset, everything matches
    assert_bkd_count(&searcher, filed_name, 2, 14, 3500)?;

    // Bounds are less than the min value of the dataset
    assert_bkd_count(&searcher, filed_name, 2, 3, 0)?;

    // Bounds are greater than the max value of the dataset
    assert_bkd_count(&searcher, filed_name, 14, 15, 0)?;

    writer.close()?;
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_random_count_with_bkd_asc() -> Result<()> {
    do_test_random_count_with_bkd(false)
}

fn test_random_count_with_bkd_desc() -> Result<()> {
    do_test_random_count_with_bkd(true)
}

fn do_test_random_count_with_bkd(reverse: bool) -> Result<()> {
    let filed_name = "field";
    let mut dir = LuceneTestCase::new_directory();
    let index_sort = sort_of(SortedNumericSortField::with_reverse(
        filed_name,
        SortFieldType::Long,
        reverse,
    )?)?;
    let writer = sorted_writer(&dir, index_sort)?;
    let random = random();
    for _i in 0..100 {
        let value = i64::from(random.next_int_bound(1000));
        let repeat = random.next_int_bound(1000);
        add_doc_with_bkd(&writer, filed_name, value, repeat)?;
    }
    writer.flush()?;
    writer.force_merge(1)?;
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    for _i in 0..100 {
        let random1 = random.next_int_bound(1100);
        let random2 = random.next_int_bound(1100);
        let low = i64::from(random1.min(random2));
        let upper = i64::from(random1.max(random2));
        let range_query = LongPoint::new_range_query(filed_name, &[low], &[upper])?;
        let index_sort_range_query = IndexSortSortedNumericDocValuesRangeQuery::new(
            filed_name,
            low,
            upper,
            Arc::clone(&range_query),
        );
        let index_sort_range_query_weight =
            index_sort_range_query.create_weight(&searcher, ScoreMode::COMPLETE, 1.0)?;
        let range_query_weight = range_query.create_weight(&searcher, ScoreMode::COMPLETE, 1.0)?;
        for context in searcher.get_leaf_contexts() {
            assert_equals(
                &range_query_weight.count(context)?,
                &index_sort_range_query_weight.count(context)?,
            );
        }
    }

    writer.close()?;
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn add_doc_with_bkd(
    index_writer: &RandomIndexWriter,
    field: &str,
    value: i64,
    repeat: i32,
) -> Result<()> {
    for _i in 0..repeat {
        let mut doc = Document::new();
        doc.add(Box::new(SortedNumericDocValuesField::new(field, value)));
        doc.add(Box::new(LongPoint::new(field, &[value])?));
        index_writer.add_document(&doc)?;
    }
    Ok(())
}
