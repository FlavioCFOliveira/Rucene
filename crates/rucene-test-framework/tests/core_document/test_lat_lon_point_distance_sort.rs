//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLatLonPointDistanceSort.java`:
//! simple tests for `LatLonDocValuesField#newDistanceSort`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::geo_fields::LatLonDocValuesField;
use rucene::document::{Document, NumericDocValuesField, NumericValue, StoredField};
use rucene::error::Result;
use rucene::geo::GeoEncodingUtils;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::IndexReader;
use rucene::search::field_comparator::SortValue;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{FieldDoc, MatchAllDocsQuery, Query};
use rucene::util::sloppy_math::SloppyMath;
use rucene_test_framework::geo::geo_test_util;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::java_number::{double_value, int_value};
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestLatLonPointDistanceSort`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLatLonPointDistanceSort",
        vec![
            test("testDistanceSort", test_distance_sort),
            test("testMissingLast", test_missing_last),
            test("testRandom", test_random),
            test("testRandomHuge", test_random_huge).annotate(Nightly),
        ],
    )
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

/// `(Double) fieldDoc.fields[i]`.
fn double_field(d: &FieldDoc, i: usize) -> f64 {
    match &d.fields.as_ref().expect("fields")[i] {
        SortValue::Double(v) => *v,
        other => panic!("ClassCastException: {other:?} cannot be cast to java.lang.Double"),
    }
}

/// `(Integer) fieldDoc.fields[i]`.
fn int_field(d: &FieldDoc, i: usize) -> i32 {
    match &d.fields.as_ref().expect("fields")[i] {
        SortValue::Int(v) => *v,
        other => panic!("ClassCastException: {other:?} cannot be cast to java.lang.Integer"),
    }
}

/// A document of one `LatLonDocValuesField("location", lat, lon)`.
fn location(lat: f64, lon: f64) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(LatLonDocValuesField::new("location", lat, lon)?));
    Ok(doc)
}

/// `new Sort(LatLonDocValuesField.newDistanceSort("location", 40.7143528, -74.0059731))`.
fn location_sort() -> Result<Sort> {
    Sort::new_fields(vec![LatLonDocValuesField::new_distance_sort(
        "location",
        40.7143528,
        -74.0059731,
    )?
    .sort_field()
    .clone()])
}

/// Port of `testDistanceSort`: add three points and sort by distance.
fn test_distance_sort() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // add some docs
    iw.add_document(&location(40.759011, -73.9844722)?)?;
    iw.add_document(&location(40.718266, -74.007819)?)?;
    iw.add_document(&location(40.7051157, -74.0088305)?)?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let td = searcher.search_with_sort(match_all(), 3, &location_sort()?)?;

    assert_equals_delta(462.1028401330431, double_field(&td.score_docs[0], 0), 0.0);
    assert_equals_delta(1054.9842850974826, double_field(&td.score_docs[1], 0), 0.0);
    assert_equals_delta(5285.881528419706, double_field(&td.score_docs[2], 0), 0.0);

    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testMissingLast`: add two points (one doc missing) and sort by
/// distance.
fn test_missing_last() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // missing
    iw.add_document(&Document::new())?;
    iw.add_document(&location(40.718266, -74.007819)?)?;
    iw.add_document(&location(40.7051157, -74.0088305)?)?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let td = searcher.search_with_sort(match_all(), 3, &location_sort()?)?;

    assert_equals_delta(462.1028401330431, double_field(&td.score_docs[0], 0), 0.0);
    assert_equals_delta(1054.9842850974826, double_field(&td.score_docs[1], 0), 0.0);
    assert_equals_delta(f64::INFINITY, double_field(&td.score_docs[2], 0), 0.0);

    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testRandom`: a few iterations with just 10 docs, hopefully easy
/// to debug.
fn test_random() -> Result<()> {
    for _iters in 0..100 {
        do_random_test(10, 100)?;
    }
    Ok(())
}

/// Port of `testRandomHuge` (`@Nightly`): runs with thousands of docs.
fn test_random_huge() -> Result<()> {
    for _iters in 0..10 {
        do_random_test(2000, 100)?;
    }
    Ok(())
}

/// `Result`: an id and a distance, ordered by distance then id
/// (`Result.compareTo`) and equal by `doubleToLongBits(distance)` and id.
#[derive(Debug, Clone, Copy)]
struct IdDistance {
    id: i32,
    distance: f64,
}

impl PartialEq for IdDistance {
    fn eq(&self, other: &Self) -> bool {
        // `Double.doubleToLongBits` collapses every NaN to one value.
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

impl IdDistance {
    /// `Result.compareTo(o)`: `Double.compare` on the distance, then the id.
    fn compare_to(&self, o: &Self) -> std::cmp::Ordering {
        let cmp = double_compare(self.distance, o.distance);
        if cmp == std::cmp::Ordering::Equal {
            return self.id.cmp(&o.id);
        }
        cmp
    }
}

/// `Double.compare(a, b)`: numeric order, then `doubleToLongBits` (so
/// `-0.0 < 0.0` and NaN is greatest).
fn double_compare(a: f64, b: f64) -> std::cmp::Ordering {
    if a < b {
        return std::cmp::Ordering::Less;
    }
    if a > b {
        return std::cmp::Ordering::Greater;
    }
    let bits = |d: f64| {
        if d.is_nan() {
            f64::NAN.to_bits() as i64
        } else {
            d.to_bits() as i64
        }
    };
    bits(a).cmp(&bits(b))
}

/// `doRandomTest(numDocs, numQueries)`.
fn do_random_test(num_docs: i32, num_queries: i32) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    // else seeds may not to reproduce:
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StoredField::new_number(
            "id",
            NumericValue::Int(i),
        )?));
        doc.add(Box::new(NumericDocValuesField::new("id", i as i64)));
        if random().next_int_bound(10) > 7 {
            let lat_raw = geo_test_util::next_latitude();
            let lon_raw = geo_test_util::next_longitude();
            // pre-normalize up front, so we can just use quantized value for testing and do simple
            // exact comparisons
            let lat =
                GeoEncodingUtils::decode_latitude(GeoEncodingUtils::encode_latitude(lat_raw)?);
            let lon =
                GeoEncodingUtils::decode_longitude(GeoEncodingUtils::encode_longitude(lon_raw)?);

            doc.add(Box::new(LatLonDocValuesField::new("field", lat, lon)?));
            doc.add(Box::new(StoredField::new_number(
                "lat",
                NumericValue::Double(lat),
            )?));
            doc.add(Box::new(StoredField::new_number(
                "lon",
                NumericValue::Double(lon),
            )?));
        } // otherwise "missing"
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let stored_fields = reader.stored_fields()?;

    for _i in 0..num_queries {
        let lat = geo_test_util::next_latitude();
        let lon = geo_test_util::next_longitude();
        let missing_value = f64::INFINITY;

        let max_doc = reader.max_doc();
        let mut expected = Vec::with_capacity(max_doc as usize);

        for doc in 0..max_doc {
            let target_doc = stored_fields.document(doc)?;
            let distance = match target_doc.get_field("lat") {
                None => missing_value, // missing
                Some(lat_field) => {
                    let doc_latitude = double_value(lat_field.numeric_value());
                    let doc_longitude =
                        double_value(target_doc.get_field("lon").expect("lon").numeric_value());
                    SloppyMath::haversin_meters(lat, lon, doc_latitude, doc_longitude)
                }
            };
            let id = int_value(target_doc.get_field("id").expect("id").numeric_value());
            expected.push(IdDistance { id, distance });
        }

        // `Arrays.sort(expected)`.
        expected.sort_by(IdDistance::compare_to);

        // randomize the topN a bit
        let top_n = TestUtil::next_int(&*random(), 1, max_doc);
        // sort by distance, then ID
        let distance_sort = LatLonDocValuesField::new_distance_sort("field", lat, lon)?
            .sort_field()
            .clone();
        let sort = Sort::new_fields(vec![
            distance_sort,
            SortField::new(Some("id".to_string()), SortFieldType::Int)?,
        ])?;

        let top_docs = searcher.search_with_sort(match_all(), top_n, &sort)?;
        for result_number in 0..top_n as usize {
            let field_doc = &top_docs.score_docs[result_number];
            let actual = IdDistance {
                id: int_field(field_doc, 1),
                distance: double_field(field_doc, 0),
            };
            assert_equals(&expected[result_number], &actual);
        }

        // get page2 with searchAfter()
        if top_n < max_doc {
            let page2 = TestUtil::next_int(&*random(), 1, max_doc - top_n);
            let after = top_docs.score_docs[top_n as usize - 1].clone();
            let top_docs2 =
                searcher.search_after_with_sort(Some(after), match_all(), page2, &sort)?;
            for result_number in 0..page2 as usize {
                let field_doc = &top_docs2.score_docs[result_number];
                let actual = IdDistance {
                    id: int_field(field_doc, 1),
                    distance: double_field(field_doc, 0),
                };
                assert_equals(&expected[top_n as usize + result_number], &actual);
            }
        }
    }
    (reader as Arc<dyn IndexReader>).close()?;
    writer.close()?;
    dir.close()
}
