//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLatLonPointDistanceFeatureQuery.java`.
//!
//! **Divergence from Lucene 10.5.0 (several methods).** Java adds one
//! `LatLonPoint` and one `LatLonDocValuesField` to a `Document` and calls
//! `setLocationValue` on them between `addDocument` calls. The port's
//! `Document` owns its fields, so each call indexes a document rebuilt with
//! the fields' current values (same fields, same order, same values).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::geo_fields::{LatLonDocValuesField, LatLonPoint};
use rucene::document::Document;
use rucene::error::Result;
use rucene::geo::{GeoEncodingUtils, GeoUtils};
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_reader::MultiReader;
use rucene::index::IndexReader;
use rucene::search::sort::{Sort, SortField};
use rucene::search::{
    IndexSearcher, MatchAllDocsQuery, Query, ScoreDoc, TopScoreDocCollectorManager,
};
use rucene::util::sloppy_math::SloppyMath;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::{CheckHits, QueryUtils};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestLatLonPointDistanceFeatureQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLatLonPointDistanceFeatureQuery",
        vec![
            test("testEqualsAndHashcode", test_equals_and_hashcode),
            test("testBasics", test_basics),
            test("testCrossesDateLine", test_crosses_date_line),
            test("testMissingField", test_missing_field),
            test("testMissingValue", test_missing_value),
            test("testMultiValued", test_multi_valued),
            test("testRandom", test_random),
            test("testCompareSorting", test_compare_sorting),
        ],
    )
}

fn q(field: &str, weight: f32, lat: f64, lon: f64, pivot: f64) -> Result<Arc<dyn Query>> {
    LatLonPoint::new_distance_feature_query(field, weight, lat, lon, pivot)
}

/// Port of `testEqualsAndHashcode`.
fn test_equals_and_hashcode() -> Result<()> {
    let q1 = q("foo", 3.0, 10.0, 10.0, 5.0)?;
    let q2 = q("foo", 3.0, 10.0, 10.0, 5.0)?;
    QueryUtils::check_equal(q1.as_ref(), q2.as_ref());

    QueryUtils::check_unequal(q1.as_ref(), q("bar", 3.0, 10.0, 10.0, 5.0)?.as_ref());
    QueryUtils::check_unequal(q1.as_ref(), q("foo", 4.0, 10.0, 10.0, 5.0)?.as_ref());
    QueryUtils::check_unequal(q1.as_ref(), q("foo", 3.0, 9.0, 10.0, 5.0)?.as_ref());
    QueryUtils::check_unequal(q1.as_ref(), q("foo", 3.0, 10.0, 9.0, 5.0)?.as_ref());
    QueryUtils::check_unequal(q1.as_ref(), q("foo", 3.0, 10.0, 10.0, 6.0)?.as_ref());
    Ok(())
}

/// `new RandomIndexWriter(random(), dir,
/// newIndexWriterConfig().setMergePolicy(newLogMergePolicy(random().nextBoolean())))`.
fn writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)
}

/// The document of `point` at `(lat, lon)` then `docValue` at
/// `(dv_lat, dv_lon)`.
fn point_doc(lat: f64, lon: f64, dv_lat: f64, dv_lon: f64) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(LatLonPoint::new("foo", lat, lon)?));
    doc.add(Box::new(LatLonDocValuesField::new("foo", dv_lat, dv_lon)?));
    Ok(doc)
}

/// `SloppyMath.haversinMeters(decodeLatitude(encodeLatitude(lat)),
/// decodeLongitude(encodeLongitude(lon)), originLat, originLon)`.
fn quantized_distance(lat: f64, lon: f64, origin_lat: f64, origin_lon: f64) -> Result<f64> {
    Ok(SloppyMath::haversin_meters(
        GeoEncodingUtils::decode_latitude(GeoEncodingUtils::encode_latitude(lat)?),
        GeoEncodingUtils::decode_longitude(GeoEncodingUtils::encode_longitude(lon)?),
        origin_lat,
        origin_lon,
    ))
}

/// `searcher.search(q, new TopScoreDocCollectorManager(n, 1)).scoreDocs`.
fn top(searcher: &IndexSearcher, q: &Arc<dyn Query>, n: i32) -> Result<Vec<ScoreDoc>> {
    let collector_manager = TopScoreDocCollectorManager::new(n, None, 1)?;
    Ok(searcher
        .search_with_manager(Arc::clone(q), &collector_manager)?
        .score_docs)
}

/// `(float) (3f * (pivot / (pivot + distance)))`.
fn score(pivot: f64, distance: f64) -> f32 {
    (3f32 as f64 * (pivot / (pivot + distance))) as f32
}

/// Port of `testBasics`.
fn test_basics() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let pivot_distance = 5000.0; // 5k

    for (lat, lon) in [
        (-7.0, -7.0),
        (9.0, 9.0),
        (8.0, 8.0),
        (4.0, 4.0),
        (-1.0, -1.0),
    ] {
        w.add_document(&point_doc(lat, lon, lat, lon)?)?;
    }

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, 10.0, 10.0, pivot_distance)?;
    let collector_manager = TopScoreDocCollectorManager::new(2, None, 1)?;
    let top_hits = searcher
        .search_with_manager(Arc::clone(&query), &collector_manager)?
        .score_docs;
    assert_equals(&2, &top_hits.len());

    let distance1 = quantized_distance(9.0, 9.0, 10.0, 10.0)?;
    let distance2 = quantized_distance(8.0, 8.0, 10.0, 10.0)?;

    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(1, score(pivot_distance, distance1)),
            ScoreDoc::new(2, score(pivot_distance, distance2)),
        ],
        &top_hits,
    );

    let distance1 = quantized_distance(9.0, 9.0, 9.0, 9.0)?;
    let distance2 = quantized_distance(8.0, 8.0, 9.0, 9.0)?;

    let query = q("foo", 3.0, 9.0, 9.0, pivot_distance)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_explanations(&query, Some(""), &searcher)?;

    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(1, score(pivot_distance, distance1)),
            ScoreDoc::new(2, score(pivot_distance, distance2)),
        ],
        &top_hits,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testCrossesDateLine`.
fn test_crosses_date_line() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let pivot_distance = 5000.0; // 5k

    w.add_document(&point_doc(0.0, -179.0, 0.0, -179.0)?)?;
    w.add_document(&point_doc(0.0, 176.0, 0.0, 176.0)?)?;
    w.add_document(&point_doc(0.0, -150.0, 0.0, -150.0)?)?;
    w.add_document(&point_doc(0.0, -140.0, 0.0, -140.0)?)?;
    // `point.setLocationValue(0, 140); docValue.setLocationValue(01, 140);`
    w.add_document(&point_doc(0.0, 140.0, 1.0, 140.0)?)?;

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, 0.0, 179.0, pivot_distance)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());

    let distance1 = quantized_distance(0.0, -179.0, 0.0, 179.0)?;
    let distance2 = quantized_distance(0.0, 176.0, 0.0, 179.0)?;

    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(0, score(pivot_distance, distance1)),
            ScoreDoc::new(1, score(pivot_distance, distance2)),
        ],
        &top_hits,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testMissingField`.
fn test_missing_field() -> Result<()> {
    let reader = Arc::new(MultiReader::new(Vec::new(), true)?);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query = q("foo", 3.0, 10.0, 10.0, 5000.0)?;
    let top_hits = searcher.search(query, 2)?;
    assert_equals(&0i64, &top_hits.total_hits.value());
    Ok(())
}

/// Port of `testMissingValue`.
fn test_missing_value() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    w.add_document(&point_doc(3.0, 3.0, 3.0, 3.0)?)?;
    w.add_document(&Document::new())?;
    w.add_document(&point_doc(7.0, 7.0, 7.0, 7.0)?)?;

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, 10.0, 10.0, 5.0)?;
    let top_hits = top(&searcher, &query, 3)?;
    assert_equals(&2, &top_hits.len());

    let distance1 = quantized_distance(7.0, 7.0, 10.0, 10.0)?;
    let distance2 = quantized_distance(3.0, 3.0, 10.0, 10.0)?;

    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(2, score(5.0, distance1)),
            ScoreDoc::new(0, score(5.0, distance2)),
        ],
        &top_hits,
    );

    CheckHits::check_explanations(&query, Some(""), &searcher)?;

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// A document of a `LatLonPoint` then a `LatLonDocValuesField` per point.
fn multi_doc(points: &[[f64; 2]]) -> Result<Document> {
    let mut doc = Document::new();
    for point in points {
        doc.add(Box::new(LatLonPoint::new("foo", point[0], point[1])?));
        doc.add(Box::new(LatLonDocValuesField::new(
            "foo", point[0], point[1],
        )?));
    }
    Ok(doc)
}

/// Port of `testMultiValued`.
fn test_multi_valued() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    w.add_document(&multi_doc(&[[0.0, 0.0], [30.0, 30.0], [60.0, 60.0]])?)?;
    w.add_document(&multi_doc(&[
        [45.0, 0.0],
        [-45.0, 0.0],
        [-90.0, 0.0],
        [90.0, 0.0],
    ])?)?;
    w.add_document(&multi_doc(&[
        [0.0, 90.0],
        [0.0, -90.0],
        [0.0, 180.0],
        [0.0, -180.0],
    ])?)?;
    w.add_document(&multi_doc(&[[3.0, 2.0]])?)?;
    w.add_document(&multi_doc(&[[45.0, 45.0], [-45.0, -45.0]])?)?;

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, 0.0, 0.0, 200.0)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());

    let distance1 = quantized_distance(0.0, 0.0, 0.0, 0.0)?;
    let distance2 = quantized_distance(3.0, 2.0, 0.0, 0.0)?;

    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(0, score(200.0, distance1)),
            ScoreDoc::new(3, score(200.0, distance2)),
        ],
        &top_hits,
    );

    let query = q("foo", 3.0, -90.0, 0.0, 10000.0)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_explanations(&query, Some(""), &searcher)?;

    let distance1 = quantized_distance(-90.0, 0.0, -90.0, 0.0)?;
    let distance2 = quantized_distance(-45.0, -45.0, -90.0, 0.0)?;

    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(1, score(10000.0, distance1)),
            ScoreDoc::new(4, score(10000.0, distance2)),
        ],
        &top_hits,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testRandom`.
fn test_random() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let num_docs = LuceneTestCase::at_least(1000);
    for _i in 0..num_docs {
        let lat = random().next_double() * 180.0 - 90.0;
        let lon = random().next_double() * 360.0 - 180.0;
        w.add_document(&point_doc(lat, lon, lat, lon)?)?;
    }

    let reader = rucene::index::directory_reader::open_with_writer(&*w)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let num_iters = LuceneTestCase::at_least(3);
    for _iter in 0..num_iters {
        let lat = random().next_double() * 180.0 - 90.0;
        let lon = random().next_double() * 360.0 - 180.0;
        let pivot_distance = random().next_double()
            * random().next_double()
            * std::f64::consts::PI
            * GeoUtils::EARTH_MEAN_RADIUS_METERS;
        let boost = (1 + random().next_int_bound(10)) as f32 / 3.0;
        let query = q("foo", boost, lat, lon, pivot_distance)?;
        CheckHits::check_top_scores(&*random(), &query, &searcher)?;
    }

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testCompareSorting`.
fn test_compare_sorting() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let num_docs = LuceneTestCase::at_least(10000);
    for _i in 0..num_docs {
        let lat = random().next_double() * 180.0 - 90.0;
        let lon = random().next_double() * 360.0 - 180.0;
        w.add_document(&point_doc(lat, lon, lat, lon)?)?;
    }

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let lat = random().next_double() * 180.0 - 90.0;
    let lon = random().next_double() * 360.0 - 180.0;
    let pivot_distance = random().next_double()
        * random().next_double()
        * GeoUtils::EARTH_MEAN_RADIUS_METERS
        * std::f64::consts::PI;
    let boost = (1 + random().next_int_bound(10)) as f32 / 3.0;

    let query1 = q("foo", boost, lat, lon, pivot_distance)?;
    let sort1 = Sort::new_fields(vec![
        SortField::FIELD_SCORE.clone(),
        LatLonDocValuesField::new_distance_sort("foo", lat, lon)?
            .sort_field()
            .clone(),
    ])?;

    let query2: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let sort2 = Sort::new_fields(vec![LatLonDocValuesField::new_distance_sort(
        "foo", lat, lon,
    )?
    .sort_field()
    .clone()])?;

    let top_docs1 = searcher.search_with_sort(query1, 10, &sort1)?;
    let top_docs2 = searcher.search_with_sort(query2, 10, &sort2)?;
    for i in 0..10 {
        Assert::assert_true(
            top_docs1.score_docs[i].score_doc.doc == top_docs2.score_docs[i].score_doc.doc,
        );
    }
    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}
