//! Port of `lucene/core/src/test/org/apache/lucene/search/TestRangeFieldsDocValuesQuery.java`.
//!
//! `XRangeDocValuesField extends BinaryRangeDocValuesField` and
//! `XRangeSlowRangeQuery extends BinaryRangeFieldRangeQuery` in Java; the
//! port's field and query types hold their base and hand it out
//! (`as_binary_range_doc_values_field`, `as_binary_range_field_range_query`),
//! which is what the documents index and the searcher runs.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    Document, DoubleRangeDocValuesField, FloatRangeDocValuesField, IntRangeDocValuesField,
    LongRangeDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::Query;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestRangeFieldsDocValuesQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestRangeFieldsDocValuesQuery",
        vec![
            test(
                "testDoubleRangeDocValuesIntersectsQuery",
                test_double_range_doc_values_intersects_query,
            ),
            test(
                "testIntRangeDocValuesIntersectsQuery",
                test_int_range_doc_values_intersects_query,
            ),
            test(
                "testLongRangeDocValuesIntersectQuery",
                test_long_range_doc_values_intersect_query,
            ),
            test(
                "testFloatRangeDocValuesIntersectQuery",
                test_float_range_doc_values_intersect_query,
            ),
            test("testToString", test_to_string),
            test("testNoData", test_no_data),
        ],
    )
}

macro_rules! dv_doc {
    ($field:ty, $min:expr, $max:expr) => {{
        let mut doc = Document::new();
        doc.add(Box::new(
            <$field>::new("dv", $min, $max)?
                .as_binary_range_doc_values_field()
                .clone(),
        ));
        doc
    }};
}

macro_rules! intersects {
    ($field:ty, $name:expr, $low:expr, $high:expr) => {{
        let q: Arc<dyn Query> = Arc::new(
            <$field>::new_slow_intersects_query($name, $low, $high)?
                .as_binary_range_field_range_query()
                .clone(),
        );
        q
    }};
}

fn count(searcher: &IndexSearcher, q: Arc<dyn Query>) -> Result<i32> {
    searcher.count(q)
}

/// Port of `TestRangeFieldsDocValuesQuery.testDoubleRangeDocValuesIntersectsQuery`.
fn test_double_range_doc_values_intersects_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let iters = LuceneTestCase::at_least(10);
    let min = [112.7, 296.0, 512.4];
    let max = [119.3, 314.8, 524.3];
    for _ in 0..iters {
        iw.add_document(&dv_doc!(DoubleRangeDocValuesField, &min, &max))?;
    }
    iw.commit()?;

    let non_matching_min = [256.7, 296.0, 532.4];
    let non_matching_max = [259.3, 364.8, 534.3];
    iw.add_document(&dv_doc!(
        DoubleRangeDocValuesField,
        &non_matching_min,
        &non_matching_max
    ))?;
    iw.commit()?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let low_range = [111.3, 294.4, 517.4];
    let high_range = [116.7, 319.4, 533.0];

    let query = intersects!(DoubleRangeDocValuesField, "dv", &low_range, &high_range);
    assert_equals(&count(&searcher, query)?, &iters);

    let low_range2 = [116.3, 299.3, 517.0];
    let high_range2 = [121.0, 317.1, 531.2];

    let query = intersects!(DoubleRangeDocValuesField, "dv", &low_range2, &high_range2);

    assert_equals(&count(&searcher, query)?, &iters);

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestRangeFieldsDocValuesQuery.testIntRangeDocValuesIntersectsQuery`.
fn test_int_range_doc_values_intersects_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let iters = LuceneTestCase::at_least(10);
    let min = [3, 11, 17];
    let max = [27, 35, 49];
    for _ in 0..iters {
        iw.add_document(&dv_doc!(IntRangeDocValuesField, &min, &max))?;
    }

    let min2 = [11, 19, 27];
    let max2 = [29, 38, 56];
    // Java builds this document and never adds it.
    let _doc = dv_doc!(IntRangeDocValuesField, &min2, &max2);

    iw.commit()?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let low_range = [6, 16, 19];
    let high_range = [29, 41, 42];

    let query = intersects!(IntRangeDocValuesField, "dv", &low_range, &high_range);

    assert_equals(&count(&searcher, query)?, &iters);

    let low_range2 = [2, 9, 18];
    let high_range2 = [25, 34, 41];

    let query = intersects!(IntRangeDocValuesField, "dv", &low_range2, &high_range2);

    assert_equals(&count(&searcher, query)?, &iters);

    let low_range3 = [101, 121, 153];
    let high_range3 = [156, 127, 176];

    let query = intersects!(IntRangeDocValuesField, "dv", &low_range3, &high_range3);

    assert_equals(&count(&searcher, query)?, &0);

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestRangeFieldsDocValuesQuery.testLongRangeDocValuesIntersectQuery`.
fn test_long_range_doc_values_intersect_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let iters = LuceneTestCase::at_least(10);
    let min = [31i64, 15, 2];
    let max = [95i64, 27, 4];
    for _ in 0..iters {
        iw.add_document(&dv_doc!(LongRangeDocValuesField, &min, &max))?;
    }

    let min2 = [101i64, 124, 137];
    let max2 = [138i64, 145, 156];
    // Java builds this document and never adds it.
    let _doc = dv_doc!(LongRangeDocValuesField, &min2, &max2);

    iw.commit()?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let low_range = [6i64, 12, 1];
    let high_range = [34i64, 24, 3];

    let query = intersects!(LongRangeDocValuesField, "dv", &low_range, &high_range);

    assert_equals(&count(&searcher, query)?, &iters);

    let low_range2 = [32i64, 18, 3];
    let high_range2 = [96i64, 29, 5];

    let query = intersects!(LongRangeDocValuesField, "dv", &low_range2, &high_range2);

    assert_equals(&count(&searcher, query)?, &iters);

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestRangeFieldsDocValuesQuery.testFloatRangeDocValuesIntersectQuery`.
fn test_float_range_doc_values_intersect_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let iters = LuceneTestCase::at_least(10);
    let min = [3.7f32, 11.0, 33.4];
    let max = [8.3f32, 21.6, 59.8];
    for _ in 0..iters {
        iw.add_document(&dv_doc!(FloatRangeDocValuesField, &min, &max))?;
    }

    let non_matching_min = [11.4f32, 29.7, 102.4];
    let non_matching_max = [17.6f32, 37.2, 160.2];
    iw.add_document(&dv_doc!(
        FloatRangeDocValuesField,
        &non_matching_min,
        &non_matching_max
    ))?;
    iw.commit()?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let low_range = [1.2f32, 8.3, 21.4];
    let high_range = [6.0f32, 17.6, 47.1];

    let query = intersects!(FloatRangeDocValuesField, "dv", &low_range, &high_range);

    assert_equals(&count(&searcher, query)?, &iters);

    let low_range2 = [6.1f32, 17.0, 31.3];
    let high_range2 = [14.2f32, 23.4, 61.1];

    let query = intersects!(FloatRangeDocValuesField, "dv", &low_range2, &high_range2);

    assert_equals(&count(&searcher, query)?, &iters);

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestRangeFieldsDocValuesQuery.testToString`; `q.toString()` is
/// `toString("")`.
fn test_to_string() -> Result<()> {
    let double_min = [112.7, 296.0, 512.4f32 as f64];
    let double_max = [119.3, 314.8, 524.3f32 as f64];
    let q1 = DoubleRangeDocValuesField::new_slow_intersects_query("foo", &double_min, &double_max)?;
    assert_equals(
        "foo:[[112.7, 296.0, 512.4000244140625] TO [119.3, 314.8, 524.2999877929688]]",
        q1.to_query_string("").as_str(),
    );

    let int_min = [3, 11, 17];
    let int_max = [27, 35, 49];
    let q2 = IntRangeDocValuesField::new_slow_intersects_query("foo", &int_min, &int_max)?;
    assert_equals(
        "foo:[[3, 11, 17] TO [27, 35, 49]]",
        q2.to_query_string("").as_str(),
    );

    let float_min = [3.7f32, 11.0, 33.4];
    let float_max = [8.3f32, 21.6, 59.8];
    let q3 = FloatRangeDocValuesField::new_slow_intersects_query("foo", &float_min, &float_max)?;
    assert_equals(
        "foo:[[3.7, 11.0, 33.4] TO [8.3, 21.6, 59.8]]",
        q3.to_query_string("").as_str(),
    );

    let long_min = [101i64, 124, 137];
    let long_max = [138i64, 145, 156];
    let q4 = LongRangeDocValuesField::new_slow_intersects_query("foo", &long_min, &long_max)?;
    assert_equals(
        "foo:[[101, 124, 137] TO [138, 145, 156]]",
        q4.to_query_string("").as_str(),
    );
    Ok(())
}

/// Port of `TestRangeFieldsDocValuesQuery.testNoData`.
fn test_no_data() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "abc".to_string(),
        Store::NO,
    )?));
    iw.add_document(&doc)?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    // test on field that doesn't exist
    let q1 = intersects!(LongRangeDocValuesField, "bar", &[20i64], &[27i64]);
    let r = searcher.search(q1, 10)?;
    assert_equals(&0, &r.total_hits.value());

    // test on field of wrong type
    let q2 = intersects!(LongRangeDocValuesField, "foo", &[20i64], &[27i64]);
    expect_throws("java.lang.IllegalStateException", || {
        searcher.search(Arc::clone(&q2), 10)
    });

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}
