//! The `BaseGeoPointTestCase` and `BaseXYPointTestCase` suites (rmp #358):
//! the inherited cases run, under the ported runner and `LuceneTestCase`'s
//! rule chain, on the point fields Lucene's own subclasses bind
//! (`lucene/core/src/test/org/apache/lucene/search/`), each with its
//! abstract-method overrides:
//!
//! * `TestLatLonPointQueries` (`LatLonPoint`, with the quantizing
//!   `quantizeLat`/`quantizeLon`) and `TestLatLonDocValuesQueries`
//!   (`LatLonDocValuesField`) over `BaseGeoPointTestCase`;
//! * `TestXYPointQueries` (`XYPointField`) and `TestXYDocValuesQueries`
//!   (`XYDocValuesField`) over `BaseXYPointTestCase`.
//!
//! `TestLatLonPointQueries` also declares its own test method,
//! `testDistanceQueryWithInvertedIntersection` (rmp #392), bound here with
//! the class.
//!
//! **Adaptation** (`field == null`): the suites pass `None` for Java's
//! `null` field name; Rucene's query factories take a `&str`, which cannot
//! be `null`, so the bindings hand them `""` (see [`field_name`]).
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs each suite on the first `n` seeds of
//! `suite_seeds::SEEDS`; `test_geo_point_suite_draws_match_lucene` and
//! `test_xy_point_suite_draws_match_lucene` compare the seeded draws with
//! `BaseSuitesDrawsOracle`'s.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::geo_fields::{
    LatLonDocValuesField, LatLonPoint, XYDocValuesField, XYPointField,
};
use rucene::document::shape_field::QueryRelation;
use rucene::document::spatial_query::{LatLonGeometryValue, XYGeometryValue};
use rucene::document::Document;
use rucene::error::Result;
use rucene::geo::{GeoEncodingUtils, Polygon, XYPolygon};
use rucene::search::query::Query;
use rucene_test_framework::geo::base_geo_point_test_case::{
    self, BaseGeoPointTestCase, GeoPointTestInstance,
};
use rucene_test_framework::geo::base_xy_point_test_case::{
    self, BaseXYPointTestCase, XYPointTestInstance,
};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::LuceneTestCaseExecutor;
use rucene_test_framework::util::LuceneTestCase;

mod draw_hooks;
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// `GeoEncodingUtils.decodeLatitude(GeoEncodingUtils.encodeLatitude(latRaw))`.
fn quantize_lat(lat_raw: f64) -> f64 {
    GeoEncodingUtils::decode_latitude(
        GeoEncodingUtils::encode_latitude(lat_raw).unwrap_or_else(|e| rethrow_java(e)),
    )
}

/// `GeoEncodingUtils.decodeLongitude(GeoEncodingUtils.encodeLongitude(lonRaw))`.
fn quantize_lon(lon_raw: f64) -> f64 {
    GeoEncodingUtils::decode_longitude(
        GeoEncodingUtils::encode_longitude(lon_raw).unwrap_or_else(|e| rethrow_java(e)),
    )
}

// -----------------------------------------------------------------------------
// TestLatLonPointQueries
// -----------------------------------------------------------------------------

/// `TestLatLonPointQueries`' overrides.
struct TestLatLonPointQueries;

impl BaseGeoPointTestCase for TestLatLonPointQueries {
    fn add_point_to_doc(&self, field: &str, doc: &mut Document, lat: f64, lon: f64) -> Result<()> {
        doc.add(Box::new(LatLonPoint::new(field, lat, lon)?));
        Ok(())
    }

    fn new_rect_query(
        &self,
        field: Option<&str>,
        min_lat: f64,
        max_lat: f64,
        min_lon: f64,
        max_lon: f64,
    ) -> Result<Arc<dyn Query>> {
        LatLonPoint::new_box_query(field, min_lat, max_lat, min_lon, max_lon)
    }

    fn new_distance_query(
        &self,
        field: Option<&str>,
        center_lat: f64,
        center_lon: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn Query>> {
        LatLonPoint::new_distance_query(field, center_lat, center_lon, radius_meters)
    }

    fn new_polygon_query(
        &self,
        field: Option<&str>,
        polygons: Vec<Polygon>,
    ) -> Result<Arc<dyn Query>> {
        LatLonPoint::new_polygon_query(
            field,
            polygons
                .into_iter()
                .map(LatLonGeometryValue::Polygon)
                .collect(),
        )
    }

    fn new_geometry_query(
        &self,
        field: &str,
        geometry: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        LatLonPoint::new_geometry_query(Some(field), QueryRelation::Intersects, geometry)
    }

    fn quantize_lat(&self, lat_raw: f64) -> f64 {
        quantize_lat(lat_raw)
    }

    fn quantize_lon(&self, lon_raw: f64) -> f64 {
        quantize_lon(lon_raw)
    }
}

// -----------------------------------------------------------------------------
// TestLatLonDocValuesQueries
// -----------------------------------------------------------------------------

/// `TestLatLonDocValuesQueries`' overrides.
struct TestLatLonDocValuesQueries;

impl BaseGeoPointTestCase for TestLatLonDocValuesQueries {
    fn add_point_to_doc(&self, field: &str, doc: &mut Document, lat: f64, lon: f64) -> Result<()> {
        doc.add(Box::new(LatLonDocValuesField::new(field, lat, lon)?));
        Ok(())
    }

    fn new_rect_query(
        &self,
        field: Option<&str>,
        min_lat: f64,
        max_lat: f64,
        min_lon: f64,
        max_lon: f64,
    ) -> Result<Arc<dyn Query>> {
        LatLonDocValuesField::new_slow_box_query(field, min_lat, max_lat, min_lon, max_lon)
    }

    fn new_distance_query(
        &self,
        field: Option<&str>,
        center_lat: f64,
        center_lon: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn Query>> {
        LatLonDocValuesField::new_slow_distance_query(field, center_lat, center_lon, radius_meters)
    }

    fn new_polygon_query(
        &self,
        field: Option<&str>,
        polygons: Vec<Polygon>,
    ) -> Result<Arc<dyn Query>> {
        LatLonDocValuesField::new_slow_polygon_query(field, polygons)
    }

    fn new_geometry_query(
        &self,
        field: &str,
        geometry: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        LatLonDocValuesField::new_slow_geometry_query(
            Some(field),
            QueryRelation::Intersects,
            geometry,
        )
    }

    fn quantize_lat(&self, lat_raw: f64) -> f64 {
        quantize_lat(lat_raw)
    }

    fn quantize_lon(&self, lon_raw: f64) -> f64 {
        quantize_lon(lon_raw)
    }
}

// -----------------------------------------------------------------------------
// TestXYPointQueries
// -----------------------------------------------------------------------------

/// `TestXYPointQueries`' overrides.
struct TestXYPointQueries;

impl BaseXYPointTestCase for TestXYPointQueries {
    fn add_point_to_doc(&self, field: &str, doc: &mut Document, x: f32, y: f32) -> Result<()> {
        doc.add(Box::new(XYPointField::new(field, x, y)?));
        Ok(())
    }

    fn new_rect_query(
        &self,
        field: Option<&str>,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> Result<Arc<dyn Query>> {
        XYPointField::new_box_query(field, min_x, max_x, min_y, max_y)
    }

    fn new_distance_query(
        &self,
        field: Option<&str>,
        center_x: f32,
        center_y: f32,
        radius: f32,
    ) -> Result<Arc<dyn Query>> {
        XYPointField::new_distance_query(field, center_x, center_y, radius)
    }

    fn new_polygon_query(
        &self,
        field: Option<&str>,
        polygons: Vec<XYPolygon>,
    ) -> Result<Arc<dyn Query>> {
        XYPointField::new_polygon_query(field, polygons)
    }

    fn new_geometry_query(
        &self,
        field: &str,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        XYPointField::new_geometry_query(Some(field), geometries)
    }
}

// -----------------------------------------------------------------------------
// TestXYDocValuesQueries
// -----------------------------------------------------------------------------

/// `TestXYDocValuesQueries`' overrides.
struct TestXYDocValuesQueries;

impl BaseXYPointTestCase for TestXYDocValuesQueries {
    fn add_point_to_doc(&self, field: &str, doc: &mut Document, x: f32, y: f32) -> Result<()> {
        doc.add(Box::new(XYDocValuesField::new(field, x, y)?));
        Ok(())
    }

    fn new_rect_query(
        &self,
        field: Option<&str>,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> Result<Arc<dyn Query>> {
        XYDocValuesField::new_slow_box_query(field, min_x, max_x, min_y, max_y)
    }

    fn new_distance_query(
        &self,
        field: Option<&str>,
        center_x: f32,
        center_y: f32,
        radius: f32,
    ) -> Result<Arc<dyn Query>> {
        XYDocValuesField::new_slow_distance_query(field, center_x, center_y, radius)
    }

    fn new_polygon_query(
        &self,
        field: Option<&str>,
        polygons: Vec<XYPolygon>,
    ) -> Result<Arc<dyn Query>> {
        XYDocValuesField::new_slow_polygon_query(field, polygons)
    }

    fn new_geometry_query(
        &self,
        field: &str,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        XYDocValuesField::new_slow_geometry_query(Some(field), geometries)
    }
}

// -----------------------------------------------------------------------------
// The classes
// -----------------------------------------------------------------------------

fn lat_lon_point_queries_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestLatLonPointQueries")
        .extends(&base_geo_point_test_case::class())
        .constructor(Constructor::no_args(|| {
            GeoPointTestInstance::new(TestLatLonPointQueries)
        }))
        .method(
            Method::new("testDistanceQueryWithInvertedIntersection")
                .throws(&["java.io.IOException"])
                .body(|_f: &mut GeoPointTestInstance| {
                    if let Err(e) = test_distance_query_with_inverted_intersection() {
                        rethrow_java(e)
                    }
                }),
        )
        .build()
}

/// Port of `TestLatLonPointQueries.testDistanceQueryWithInvertedIntersection`.
fn test_distance_query_with_inverted_intersection() -> Result<()> {
    let case = TestLatLonPointQueries;
    let num_matching_docs = LuceneTestCase::at_least(
        10 * rucene::util::bkd::BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
    );

    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        {
            let w = rucene::index::index_writer::IndexWriter::new(
                dir.as_directory(),
                LuceneTestCase::new_index_writer_config(),
            )?;
            let written = (|| -> Result<()> {
                for _i in 0..num_matching_docs {
                    let mut doc = Document::new();
                    case.add_point_to_doc("field", &mut doc, 18.313694, -65.227444)?;
                    w.add_document(&doc)?;
                }

                // Add a handful of docs that don't match
                for _i in 0..11 {
                    let mut doc = Document::new();
                    case.add_point_to_doc("field", &mut doc, 10.0, -65.227444)?;
                    w.add_document(&doc)?;
                }
                w.force_merge(1)?;
                Ok(())
            })();
            w.close()?;
            written?;
        }

        let r = rucene::index::directory_reader::open(dir.as_directory())?;
        let checked = (|| -> Result<()> {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
            let count =
                searcher.count(case.new_distance_query(Some("field"), 18.0, -65.0, 50_000.0)?)?;
            if num_matching_docs != count {
                randomizedtesting::junit::Assert::fail(&format!(
                    "expected:<{num_matching_docs}> but was:<{count}>"
                ));
            }
            Ok(())
        })();
        r.close()?;
        checked
    })();
    dir.close()?;
    result
}

fn lat_lon_doc_values_queries_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestLatLonDocValuesQueries")
        .extends(&base_geo_point_test_case::class())
        .constructor(Constructor::no_args(|| {
            GeoPointTestInstance::new(TestLatLonDocValuesQueries)
        }))
        .build()
}

fn xy_point_queries_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestXYPointQueries")
        .extends(&base_xy_point_test_case::class())
        .constructor(Constructor::no_args(|| {
            XYPointTestInstance::new(TestXYPointQueries)
        }))
        .build()
}

fn xy_doc_values_queries_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestXYDocValuesQueries")
        .extends(&base_xy_point_test_case::class())
        .constructor(Constructor::no_args(|| {
            XYPointTestInstance::new(TestXYDocValuesQueries)
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestGeoPointSuiteDraws, TestXYPointSuiteDraws
// -----------------------------------------------------------------------------

static GEO_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static XY_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The shared executor `searchOnTestThread` sets aside, restored by
/// `recordClassDraw` (one draws class runs at a time, under `SUITES`).
static SHARED_EXECUTOR: Mutex<Option<Arc<LuceneTestCaseExecutor>>> = Mutex::new(None);

/// `@BeforeClass searchOnTestThread()` of both draws classes: sets
/// `LuceneTestCase`'s shared executor to `null`, so that every
/// `MAYBE_CACHE_POLICY` draw of a search happens on the test thread. With the
/// executor, the thread that runs each slice, and so the randomness drawn, is
/// a race in Java (three runs of the oracle gave three different fixtures).
fn search_on_test_thread() -> Method {
    Method::new("searchOnTestThread")
        .modifiers(modifier::PUBLIC | modifier::STATIC)
        .annotate(BeforeClass)
        .static_body(|| {
            *SHARED_EXECUTOR.lock().unwrap() = LuceneTestCase::replace_executor(None);
        })
}

/// `@AfterClass recordClassDraw()` of both draws classes: records the class
/// randomness's next draw, then restores the shared executor.
fn record_class_draw(draws: &'static Mutex<Vec<String>>) -> Method {
    Method::new("recordClassDraw")
        .modifiers(modifier::PUBLIC | modifier::STATIC)
        .annotate(AfterClass)
        .static_body(move || {
            draws.lock().unwrap().push(format!(
                "#class\tnext\t{}",
                LuceneTestCase::random().next_long()
            ));
            LuceneTestCase::replace_executor(SHARED_EXECUTOR.lock().unwrap().take());
        })
}

/// `TestGeoPointSuiteDraws` of `BaseSuitesDrawsOracle`: the cases on
/// `TestLatLonPointQueries`' overrides, with the shared executor set aside.
fn geo_draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.geo.TestGeoPointSuiteDraws")
        .extends(&base_geo_point_test_case::class())
        .constructor(Constructor::no_args(|| {
            GeoPointTestInstance::new(TestLatLonPointQueries)
        }))
        .method(search_on_test_thread())
        .method(draw_hooks::record_tear_down(
            &GEO_DRAWS,
            draw_hooks::lucene_tear_down::<GeoPointTestInstance>,
        ))
        .method(record_class_draw(&GEO_DRAWS))
        .build()
}

/// `TestXYPointSuiteDraws` of `BaseSuitesDrawsOracle`: the cases on
/// `TestXYPointQueries`' overrides, with the shared executor set aside.
fn xy_draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.geo.TestXYPointSuiteDraws")
        .extends(&base_xy_point_test_case::class())
        .constructor(Constructor::no_args(|| {
            XYPointTestInstance::new(TestXYPointQueries)
        }))
        .method(search_on_test_thread())
        .method(draw_hooks::record_tear_down(
            &XY_DRAWS,
            draw_hooks::lucene_tear_down::<XYPointTestInstance>,
        ))
        .method(record_class_draw(&XY_DRAWS))
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_geo_point_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_geo_point_draws.tsv", geo_draws_class, &GEO_DRAWS);
}

#[test]
fn test_xy_point_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_xy_point_draws.tsv", xy_draws_class, &XY_DRAWS);
}

#[test]
fn test_lat_lon_point_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lat_lon_point_queries_class);
}

#[test]
fn test_lat_lon_doc_values_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lat_lon_doc_values_queries_class);
}

#[test]
fn test_xy_point_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(xy_point_queries_class);
}

#[test]
fn test_xy_doc_values_queries() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(xy_doc_values_queries_class);
}
