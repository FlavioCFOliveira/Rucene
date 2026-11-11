//! Port of `lucene/core/src/test/org/apache/lucene/search/TestXYPointDistanceSort.java`.
//!
//! Simple tests for `XYDocValuesField.newDistanceSort`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::geo_fields::XYDocValuesField;
use rucene::document::{Document, NumericDocValuesField, NumericValue, StoredField};
use rucene::error::Result;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::search::field_comparator::SortValue;
use rucene::search::field_doc::FieldDoc;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{MatchAllDocsQuery, Query};
use rucene_test_framework::geo::shape_test_util;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::java_number;
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestXYPointDistanceSort`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestXYPointDistanceSort",
        vec![
            test("testDistanceSort", test_distance_sort),
            test("testMissingLast", test_missing_last),
            test("testRandom", test_random),
            test("testRandomHuge", test_random_huge).annotate(Nightly),
        ],
    )
}

/// `private double cartesianDistance(double x1, double y1, double x2, double y2)`.
fn cartesian_distance(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let diff_x = x1 - x2;
    let diff_y = y1 - y2;
    (diff_x * diff_x + diff_y * diff_y).sqrt()
}

/// `cartesianDistance` of `float` arguments, widened as Java widens them.
fn cartesian_distance_f(x1: f32, y1: f32, x2: f32, y2: f32) -> f64 {
    cartesian_distance(f64::from(x1), f64::from(y1), f64::from(x2), f64::from(y2))
}

/// `(Double) d.fields[i]`.
fn double_field(d: &FieldDoc, i: usize) -> f64 {
    match &d.fields.as_ref().expect("fields")[i] {
        SortValue::Double(v) => *v,
        other => {
            panic!("java.lang.ClassCastException: {other:?} cannot be cast to java.lang.Double")
        }
    }
}

/// `(Integer) d.fields[i]`.
fn int_field(d: &FieldDoc, i: usize) -> i32 {
    match &d.fields.as_ref().expect("fields")[i] {
        SortValue::Int(v) => *v,
        other => {
            panic!("java.lang.ClassCastException: {other:?} cannot be cast to java.lang.Integer")
        }
    }
}

/// `new Sort(XYDocValuesField.newDistanceSort(field, x, y))`.
fn distance_sort(field: &str, x: f32, y: f32) -> Result<Sort> {
    Sort::new_fields(vec![XYDocValuesField::new_distance_sort(field, x, y)?
        .sort_field()
        .clone()])
}

/// Add three points and sort by distance
///
/// Port of `TestXYPointDistanceSort.testDistanceSort`.
fn test_distance_sort() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // add some docs
    let mut doc = Document::new();
    doc.add(Box::new(XYDocValuesField::new(
        "location",
        40.759011f32,
        -73.9844722f32,
    )?));
    iw.add_document(&doc)?;
    let d1 = cartesian_distance_f(40.759011, -73.9844722, 40.7143528, -74.0059731);

    let mut doc = Document::new();
    doc.add(Box::new(XYDocValuesField::new(
        "location",
        40.718266f32,
        -74.007819f32,
    )?));
    iw.add_document(&doc)?;
    let d2 = cartesian_distance_f(40.718266, -74.007819, 40.7143528, -74.0059731);

    let mut doc = Document::new();
    doc.add(Box::new(XYDocValuesField::new(
        "location",
        40.7051157f32,
        -74.0088305f32,
    )?));
    iw.add_document(&doc)?;
    let d3 = cartesian_distance_f(40.7051157, -74.0088305, 40.7143528, -74.0059731);

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let sort = distance_sort("location", 40.7143528f32, -74.0059731f32)?;
    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 3, &sort)?;

    let d = &td.score_docs[0];
    assert_equals_delta(d2, double_field(d, 0), 0.0);

    let d = &td.score_docs[1];
    assert_equals_delta(d3, double_field(d, 0), 0.0);

    let d = &td.score_docs[2];
    assert_equals_delta(d1, double_field(d, 0), 0.0);

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Add two points (one doc missing) and sort by distance
///
/// Port of `TestXYPointDistanceSort.testMissingLast`.
fn test_missing_last() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // missing
    let doc = Document::new();
    iw.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(XYDocValuesField::new(
        "location",
        40.718266f32,
        -74.007819f32,
    )?));
    iw.add_document(&doc)?;
    let d2 = cartesian_distance_f(40.718266, -74.007819, 40.7143528, -74.0059731);

    let mut doc = Document::new();
    doc.add(Box::new(XYDocValuesField::new(
        "location",
        40.7051157f32,
        -74.0088305f32,
    )?));
    iw.add_document(&doc)?;
    let d3 = cartesian_distance_f(40.7051157, -74.0088305, 40.7143528, -74.0059731);

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let sort = distance_sort("location", 40.7143528f32, -74.0059731f32)?;
    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 3, &sort)?;

    let d = &td.score_docs[0];
    assert_equals_delta(d2, double_field(d, 0), 0.0);

    let d = &td.score_docs[1];
    assert_equals_delta(d3, double_field(d, 0), 0.0);

    let d = &td.score_docs[2];
    assert_equals_delta(f64::INFINITY, double_field(d, 0), 0.0);

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Run a few iterations with just 10 docs, hopefully easy to debug
///
/// Port of `TestXYPointDistanceSort.testRandom`.
fn test_random() -> Result<()> {
    for _iters in 0..100 {
        do_random_test(10, 100)?;
    }
    Ok(())
}

/// Runs with thousands of docs
///
/// Port of `TestXYPointDistanceSort.testRandomHuge`.
fn test_random_huge() -> Result<()> {
    for _iters in 0..10 {
        do_random_test(2000, 100)?;
    }
    Ok(())
}

/// result class used for testing. holds an id+distance. we sort these with
/// Arrays.sort and compare with lucene's results
///
/// `static class Result implements Comparable<Result>`.
#[derive(Debug, Clone, Copy)]
struct TestResult {
    id: i32,
    distance: f64,
}

impl TestResult {
    fn new(id: i32, distance: f64) -> Self {
        TestResult { id, distance }
    }

    /// `compareTo(Result o)`.
    fn compare_to(&self, o: &TestResult) -> Ordering {
        // `Double.compare(distance, o.distance)`.
        let cmp = self.distance.total_cmp(&o.distance);
        if cmp == Ordering::Equal {
            return self.id.cmp(&o.id);
        }
        cmp
    }
}

impl PartialEq for TestResult {
    /// `equals(Object obj)`: `Double.doubleToLongBits` equality of the
    /// distances, then the ids.
    fn eq(&self, other: &Self) -> bool {
        let bits = |d: f64| {
            if d.is_nan() {
                f64::NAN.to_bits()
            } else {
                d.to_bits()
            }
        };
        bits(self.distance) == bits(other.distance) && self.id == other.id
    }
}

impl fmt::Display for TestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Result [id={}, distance={}]",
            self.id,
            rucene::util::java_fmt::double_to_string(self.distance)
        )
    }
}

/// `private void doRandomTest(int numDocs, int numQueries)`.
fn do_random_test(num_docs: i32, num_queries: i32) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    // else seeds may not to reproduce:
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StoredField::new_number(
            "id",
            NumericValue::Int(i),
        )?));
        doc.add(Box::new(NumericDocValuesField::new("id", i64::from(i))));
        if random().next_int_bound(10) > 7 {
            let x = shape_test_util::next_float(&*random());
            let y = shape_test_util::next_float(&*random());

            doc.add(Box::new(XYDocValuesField::new("field", x, y)?));
            doc.add(Box::new(StoredField::new_number(
                "x",
                NumericValue::Float(x),
            )?));
            doc.add(Box::new(StoredField::new_number(
                "y",
                NumericValue::Float(y),
            )?));
        } // otherwise "missing"
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;
    let stored_fields = reader.stored_fields()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    for _i in 0..num_queries {
        let x = shape_test_util::next_float(&*random());
        let y = shape_test_util::next_float(&*random());
        let missing_value = f64::INFINITY;

        let mut expected: Vec<TestResult> = Vec::with_capacity(reader.max_doc() as usize);

        for doc in 0..reader.max_doc() {
            let target_doc = stored_fields.document(doc)?;
            let distance = match target_doc.get_field("x") {
                None => missing_value, // missing
                Some(x_field) => {
                    let doc_x = f64::from(java_number::float_value(x_field.numeric_value()));
                    let doc_y = f64::from(java_number::float_value(
                        target_doc
                            .get_field("y")
                            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
                            .numeric_value(),
                    ));
                    cartesian_distance(f64::from(x), f64::from(y), doc_x, doc_y)
                }
            };
            let id = java_number::int_value(
                target_doc
                    .get_field("id")
                    .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
                    .numeric_value(),
            );
            expected.push(TestResult::new(id, distance));
        }

        expected.sort_by(|a, b| a.compare_to(b));

        // randomize the topN a bit
        let top_n = TestUtil::next_int(&*random(), 1, reader.max_doc());
        // sort by distance, then ID
        let distance_sort = XYDocValuesField::new_distance_sort("field", x, y)?;
        let sort = Sort::new_fields(vec![
            distance_sort.sort_field().clone(),
            SortField::new(Some("id".to_string()), SortFieldType::Int)?,
        ])?;

        let query = || -> Arc<dyn Query> { Arc::new(MatchAllDocsQuery::instance()) };
        let top_docs = searcher.search_with_sort(query(), top_n, &sort)?;
        for result_number in 0..top_n as usize {
            let field_doc = &top_docs.score_docs[result_number];
            let actual = TestResult::new(int_field(field_doc, 1), double_field(field_doc, 0));
            assert_equals(&expected[result_number], &actual);
        }

        // get page2 with searchAfter()
        if top_n < reader.max_doc() {
            let page2 = TestUtil::next_int(&*random(), 1, reader.max_doc() - top_n);
            let top_docs2 = searcher.search_after_with_sort(
                Some(top_docs.score_docs[top_n as usize - 1].clone()),
                query(),
                page2,
                &sort,
            )?;
            for result_number in 0..page2 as usize {
                let field_doc = &top_docs2.score_docs[result_number];
                let actual = TestResult::new(int_field(field_doc, 1), double_field(field_doc, 0));
                assert_equals(&expected[top_n as usize + result_number], &actual);
            }
        }
    }
    drop(searcher);
    drop(stored_fields);
    reader.close()?;
    writer.close()?;
    dir.close()
}
