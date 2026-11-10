//! Port of `lucene/core/src/test/org/apache/lucene/search/TestNearest.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::geo_fields::{LatLonDocValuesField, LatLonPoint};
use rucene::document::{Document, NumericValue, Store, StoredField, StringField};
use rucene::error::Result;
use rucene::geo::GeoEncodingUtils;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::search::field_comparator::SortValue;
use rucene::search::field_doc::FieldDoc;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::Sort;
use rucene::util::sloppy_math::SloppyMath;
use rucene_test_framework::geo::geo_test_util;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::term;
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test, verbose};

/// `TestNearest`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestNearest",
        vec![
            test(
                "testNearestNeighborWithDeletedDocs",
                test_nearest_neighbor_with_deleted_docs,
            ),
            test(
                "testNearestNeighborWithAllDeletedDocs",
                test_nearest_neighbor_with_all_deleted_docs,
            ),
            test("testTieBreakByDocID", test_tie_break_by_doc_id),
            test(
                "testNearestNeighborWithNoDocs",
                test_nearest_neighbor_with_no_docs,
            ),
            test("testNearestNeighborRandom", test_nearest_neighbor_random),
        ],
    )
}

fn point_doc(lat: f64, lon: f64, id: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(LatLonPoint::new("point", lat, lon)?));
    doc.add(Box::new(StringField::new(
        "id",
        id.to_string(),
        Store::YES,
    )?));
    Ok(doc)
}

/// `r.storedFields().document(doc).getField("id").stringValue()`.
fn stored_id(r: &Arc<dyn DirectoryReader>, doc: i32) -> Result<String> {
    let document = r.stored_fields()?.document(doc)?;
    Ok(document
        .get_field("id")
        .and_then(|f| f.string_value())
        .unwrap_or_default())
}

/// `LatLonPoint.nearest(s, "point", lat, lon, n)`.
fn nearest(s: &IndexSearcher, lat: f64, lon: f64, n: i32) -> Result<Vec<FieldDoc>> {
    Ok(LatLonPoint::nearest(s, Some("point"), lat, lon, n)?.score_docs)
}

/// `newSearcher(r, false)`: can't wrap because we require
/// Lucene60PointsFormat directly but e.g. ParallelReader wraps with its own
/// points impl.
fn unwrapped_searcher(r: &Arc<dyn DirectoryReader>) -> IndexSearcher {
    LuceneTestCase::new_searcher_wrap(Arc::clone(r), false)
}

/// Port of `TestNearest.testNearestNeighborWithDeletedDocs`.
fn test_nearest_neighbor_with_deleted_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        get_index_writer_config(),
    )?;
    w.add_document(&point_doc(40.0, 50.0, "0")?)?;
    w.add_document(&point_doc(45.0, 55.0, "1")?)?;

    let r = w.get_reader()?;
    let s = unwrapped_searcher(&r);
    let hit = &nearest(&s, 40.0, 50.0, 1)?[0];
    assert_equals(&"0".to_string(), &stored_id(&r, hit.score_doc.doc)?);
    drop(s);
    r.close()?;

    w.delete_documents(term("id", "0"))?;
    let r = w.get_reader()?;
    let s = unwrapped_searcher(&r);
    let hit = &nearest(&s, 40.0, 50.0, 1)?[0];
    assert_equals(&"1".to_string(), &stored_id(&r, hit.score_doc.doc)?);
    drop(s);
    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestNearest.testNearestNeighborWithAllDeletedDocs`.
fn test_nearest_neighbor_with_all_deleted_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        get_index_writer_config(),
    )?;
    w.add_document(&point_doc(40.0, 50.0, "0")?)?;
    w.add_document(&point_doc(45.0, 55.0, "1")?)?;

    let r = w.get_reader()?;
    let s = unwrapped_searcher(&r);
    let hit = &nearest(&s, 40.0, 50.0, 1)?[0];
    assert_equals(&"0".to_string(), &stored_id(&r, hit.score_doc.doc)?);
    drop(s);
    r.close()?;

    w.delete_documents(term("id", "0"))?;
    w.delete_documents(term("id", "1"))?;
    let r = w.get_reader()?;
    let s = unwrapped_searcher(&r);
    assert_equals(&0, &nearest(&s, 40.0, 50.0, 1)?.len());
    drop(s);
    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestNearest.testTieBreakByDocID`.
fn test_tie_break_by_doc_id() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), get_index_writer_config())?;
    w.add_document(&point_doc(40.0, 50.0, "0")?)?;
    w.add_document(&point_doc(40.0, 50.0, "1")?)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let hits = nearest(&unwrapped_searcher(&r), 45.0, 50.0, 2)?;
    assert_equals(&"0".to_string(), &stored_id(&r, hits[0].score_doc.doc)?);
    assert_equals(&"1".to_string(), &stored_id(&r, hits[1].score_doc.doc)?);

    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestNearest.testNearestNeighborWithNoDocs`.
fn test_nearest_neighbor_with_no_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        get_index_writer_config(),
    )?;
    let r = w.get_reader()?;
    assert_equals(&0, &nearest(&unwrapped_searcher(&r), 40.0, 50.0, 1)?.len());
    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn quantize_lat(lat_raw: f64) -> Result<f64> {
    Ok(GeoEncodingUtils::decode_latitude(
        GeoEncodingUtils::encode_latitude(lat_raw)?,
    ))
}

fn quantize_lon(lon_raw: f64) -> Result<f64> {
    Ok(GeoEncodingUtils::decode_longitude(
        GeoEncodingUtils::encode_longitude(lon_raw)?,
    ))
}

/// `((Double) fieldDoc.fields[0]).doubleValue()`.
fn distance_of(hit: &FieldDoc) -> f64 {
    match hit.fields.as_ref().and_then(|f| f.first()) {
        Some(SortValue::Double(v)) => *v,
        other => randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            format!("{other:?} cannot be cast to java.lang.Double"),
        )
        .throw(),
    }
}

/// Port of `TestNearest.testNearestNeighborRandom`.
fn test_nearest_neighbor_random() -> Result<()> {
    let num_points = LuceneTestCase::at_least(1000);
    let mut dir: TestDirectory = if num_points > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("TestNearest"))
    } else {
        LuceneTestCase::new_directory()
    };
    let mut lats = vec![0f64; num_points as usize];
    let mut lons = vec![0f64; num_points as usize];

    let iwc = get_index_writer_config();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    for id in 0..num_points as usize {
        lats[id] = quantize_lat(geo_test_util::next_latitude())?;
        lons[id] = quantize_lon(geo_test_util::next_longitude())?;
        let mut doc = Document::new();
        doc.add(Box::new(LatLonPoint::new("point", lats[id], lons[id])?));
        doc.add(Box::new(LatLonDocValuesField::new(
            "point", lats[id], lons[id],
        )?));
        doc.add(Box::new(StoredField::new_number(
            "id",
            NumericValue::Int(id as i32),
        )?));
        w.add_document(&doc)?;
    }

    if random().next_boolean() {
        w.force_merge(1)?;
    }

    let r = w.get_reader()?;
    if verbose() {
        println!("TEST: reader={r}");
    }
    // can't wrap because we require Lucene60PointsFormat directly but e.g.
    // ParallelReader wraps with its own points impl:
    let s = unwrapped_searcher(&r);
    let iters = LuceneTestCase::at_least(100);
    for iter in 0..iters {
        if verbose() {
            println!("\nTEST: iter={iter}");
        }
        let point_lat = geo_test_util::next_latitude();
        let point_lon = geo_test_util::next_longitude();

        // dumb brute force search to get the expected result:
        let mut expected_hits: Vec<(i32, f64)> = (0..lats.len())
            .map(|id| {
                let distance =
                    SloppyMath::haversin_meters(point_lat, point_lon, lats[id], lons[id]);
                (id as i32, distance)
            })
            .collect();

        expected_hits.sort_by(|a, b| {
            let cmp = a.1.total_cmp(&b.1);
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
            // tie break by smaller docID:
            a.0.cmp(&b.0)
        });

        let top_n = TestUtil::next_int(&*random(), 1, lats.len() as i32);

        if verbose() {
            println!("\nhits for pointLat={point_lat} pointLon={point_lon}");
        }

        // Also test with MatchAllDocsQuery, sorting by distance:
        let field_docs = s.search_with_sort(
            Arc::new(MatchAllDocsQuery::instance()),
            top_n,
            &Sort::new_fields(vec![LatLonDocValuesField::new_distance_sort(
                "point", point_lat, point_lon,
            )?
            .sort_field()
            .clone()])?,
        )?;

        let hits = nearest(&s, point_lat, point_lon, top_n)?;
        let stored_fields = r.stored_fields()?;
        for i in 0..top_n as usize {
            let expected = expected_hits[i];
            let expected2 = &field_docs.score_docs[i];
            let actual = &hits[i];
            let actual_doc = stored_fields.document(actual.score_doc.doc)?;

            if verbose() {
                println!("hit {i}");
                println!(
                    "  expected id={} lat={} lon={} distance={} meters",
                    expected.0, lats[expected.0 as usize], lons[expected.0 as usize], expected.1
                );
                println!(
                    "  actual id={:?} distance={:?} meters",
                    actual_doc.get_field("id").map(|f| f.numeric_value()),
                    actual.fields.as_ref().and_then(|f| f.first())
                );
            }

            assert_equals(&expected.0, &actual.score_doc.doc);
            assert_equals_delta(expected.1, distance_of(actual), 0.0);

            assert_equals(&expected2.score_doc.doc, &actual.score_doc.doc);
            assert_equals_delta(distance_of(expected2), distance_of(actual), 0.0);
        }
    }

    drop(s);
    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn get_index_writer_config() -> IndexWriterConfig {
    let iwc = LuceneTestCase::new_index_writer_config();
    crate::core_support::unchecked(iwc.set_codec(TestUtil::get_default_codec()).map(|_| ()));
    iwc
}
