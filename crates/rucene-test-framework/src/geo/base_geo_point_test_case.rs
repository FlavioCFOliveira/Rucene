/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.geo.BaseGeoPointTestCase`: abstract
//! class to do basic tests for a geospatial impl (high level fields and
//! queries).
//!
//! NOTE: This test focuses on geospatial (distance queries, polygon queries,
//! etc) indexing and search, not any underlying storage format or encoding:
//! it merely supplies two hooks for the encoding so that tests can be exact.
//! The \[stretch\] goal is for this test to be so thorough in testing a new
//! geo impl that if this test passes, then all Lucene tests should also
//! pass. Ie, if there is some bug in a given geo impl that this test fails
//! to catch then this test needs to be improved!
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseGeoPointTestCase`], whose required methods are the abstract
//! methods a subclass implements.
//!
//! **Adaptations** (declared, each confined to the member named):
//! * `String field` of the abstract query factories is `&str`: Rust cannot
//!   pass `null`, so `testBoxNull`, `testDistanceNull` and
//!   `testPolygonNullField` are assumption failures (see each).
//! * `LatLonGeometry` and `LatLonGeometry[]` are the port's
//!   [`LatLonGeometryValue`] and `Vec<LatLonGeometryValue>`; `Polygon...`
//!   is `Vec<Polygon>`.
//! * The anonymous `FilterCodec`/`PointsFormat` of `doRandomDistanceTest` is
//!   [`LeafSizePointsFormat`] in a [`FilterCodec`].

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::points::{
    Lucene90PointsReader, Lucene90PointsWriter, DEFAULT_MAX_MB_SORT_IN_HEAP, VERSION_CURRENT,
};
use rucene::codecs::points::{PointsFormat, PointsReader, PointsWriter};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{Codec, FilterCodec};
use rucene::document::spatial_query::LatLonGeometryValue;
use rucene::document::{
    Document, NumericDocValuesField, NumericValue, Store, StoredField, StringField,
};
use rucene::error::Result;
use rucene::geo::{Circle, GeoEncodingUtils, GeoUtils, Polygon, Rectangle};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::terms::Term;
use rucene::index::{DirectoryReader, IndexReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::{MatchNoDocsQuery, Sort, SortField, TopDocs};
use rucene::util::java_fmt::double_to_string;
use rucene::util::sloppy_math::SloppyMath;
use rucene::util::{Bits, BytesRef, FixedBitSet};

use super::geo_test_util;
use crate::analysis::MockAnalyzer;
use crate::index::RandomIndexWriter;
use crate::jdk::bit_set::JavaBitSet;
use crate::search::FixedBitSetCollector;
use crate::util::java_exceptions::{expect_throws, rethrow_java};
use crate::util::lucene_test_case::{Nightly, TestDirectory, VERBOSE};
use crate::util::{LuceneTestCase, TestUtil};

/// `FIELD_NAME` (`protected static final String`).
pub const FIELD_NAME: &str = "point";

/// The declaration of `org.apache.lucene.tests.geo.BaseGeoPointTestCase`
/// (`public abstract class BaseGeoPointTestCase extends LuceneTestCase`)
/// with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        fn t(name: &str, body: fn(&dyn BaseGeoPointTestCase) -> Result<()>) -> Method {
            geo_point_test(name, body)
        }
        Class::builder("org.apache.lucene.tests.geo.BaseGeoPointTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(t("testIndexExtremeValues", |c| {
                c.test_index_extreme_values()
            }))
            .method(t("testIndexOutOfRangeValues", |c| {
                c.test_index_out_of_range_values()
            }))
            .method(t("testIndexNaNValues", |c| c.test_index_nan_values()))
            .method(t("testIndexInfValues", |c| c.test_index_inf_values()))
            .method(t("testBoxBasics", |c| c.test_box_basics()))
            .method(t("testBoxNull", |c| c.test_box_null()))
            .method(t("testBoxInvalidCoordinates", |c| {
                c.test_box_invalid_coordinates()
            }))
            .method(t("testDistanceBasics", |c| c.test_distance_basics()))
            .method(t("testDistanceNull", |c| c.test_distance_null()))
            .method(t("testDistanceIllegal", |c| c.test_distance_illegal()))
            .method(t("testDistanceNegative", |c| c.test_distance_negative()))
            .method(t("testDistanceNaN", |c| c.test_distance_nan()))
            .method(t("testDistanceInf", |c| c.test_distance_inf()))
            .method(t("testPolygonBasics", |c| c.test_polygon_basics()))
            .method(t("testPolygonHole", |c| c.test_polygon_hole()))
            .method(t("testPolygonHoleExcludes", |c| {
                c.test_polygon_hole_excludes()
            }))
            .method(t("testMultiPolygonBasics", |c| {
                c.test_multi_polygon_basics()
            }))
            .method(t("testPolygonNullField", |c| c.test_polygon_null_field()))
            .method(t("testSamePointManyTimes", |c| {
                c.test_same_point_many_times()
            }))
            .method(t("testLowCardinality", |c| c.test_low_cardinality()))
            .method(t("testAllLatEqual", |c| c.test_all_lat_equal()))
            .method(t("testAllLonEqual", |c| c.test_all_lon_equal()))
            .method(t("testMultiValued", |c| c.test_multi_valued()))
            .method(t("testRandomTiny", |c| c.test_random_tiny()))
            .method(t("testRandomMedium", |c| c.test_random_medium()))
            .method(t("testRandomBig", |c| c.test_random_big()).annotate(Nightly))
            .method(t("testRectBoundariesAreInclusive", |c| {
                c.test_rect_boundaries_are_inclusive()
            }))
            .method(t("testRandomDistance", |c| c.test_random_distance()))
            .method(
                t("testRandomDistanceHuge", |c| c.test_random_distance_huge()).annotate(Nightly),
            )
            .method(t("testEquals", |c| c.test_equals()))
            .method(t("testSmallSetRect", |c| c.test_small_set_rect()))
            .method(t("testSmallSetDateline", |c| c.test_small_set_dateline()))
            .method(t("testSmallSetMultiValued", |c| {
                c.test_small_set_multi_valued()
            }))
            .method(t("testSmallSetWholeMap", |c| c.test_small_set_whole_map()))
            .method(t("testSmallSetPoly", |c| c.test_small_set_poly()))
            .method(t("testSmallSetPolyWholeMap", |c| {
                c.test_small_set_poly_whole_map()
            }))
            .method(t("testSmallSetDistance", |c| c.test_small_set_distance()))
            .method(t("testSmallSetTinyDistance", |c| {
                c.test_small_set_tiny_distance()
            }))
            .method(t("testSmallSetDistanceNotEmpty", |c| {
                c.test_small_set_distance_not_empty()
            }))
            .method(t("testSmallSetHugeDistance", |c| {
                c.test_small_set_huge_distance()
            }))
            .method(t("testSmallSetDistanceDateline", |c| {
                c.test_small_set_distance_dateline()
            }))
            .method(t("testNarrowPolygonCloseToNorthPole", |c| {
                c.test_narrow_polygon_close_to_north_pole()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test method of a `BaseGeoPointTestCase` subclass.
pub fn geo_point_test(
    name: &str,
    body: impl Fn(&dyn BaseGeoPointTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut GeoPointTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// The instance of a `BaseGeoPointTestCase` subclass.
pub struct GeoPointTestInstance {
    /// The subclass.
    pub case: Arc<dyn BaseGeoPointTestCase>,
}

impl GeoPointTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseGeoPointTestCase>(case: T) -> Self {
        GeoPointTestInstance {
            case: Arc::new(case),
        }
    }
}

/// `random()`.
fn random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    LuceneTestCase::random()
}

/// The message of a caught exception (`getMessage()`), `"null"` when it has
/// none, as string concatenation prints it.
fn message(t: &randomizedtesting::jdk::exception::Throwable) -> String {
    t.message().unwrap_or("null").to_string()
}

/// `expected.getMessage().contains(s)`: a `null` message throws
/// `NullPointerException` in Java.
fn message_contains(t: &randomizedtesting::jdk::exception::Throwable, s: &str) -> bool {
    match t.message() {
        Some(m) => m.contains(s),
        None => randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.NullPointerException",
        ),
    }
}

/// A query compared with `Query.equals` and printed with `toString()`, for
/// `assertEquals(q1, q2)`.
struct QueryObject(Arc<dyn Query>);

impl PartialEq for QueryObject {
    fn eq(&self, other: &Self) -> bool {
        self.0.query_eq(&*other.0)
    }
}

impl std::fmt::Display for QueryObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.to_query_string(""))
    }
}

/// `geometry.toString()` of a `LatLonGeometry`.
pub fn geometry_to_string(g: &LatLonGeometryValue) -> String {
    match g {
        LatLonGeometryValue::Point(p) => p.to_string(),
        LatLonGeometryValue::Rectangle(r) => r.to_string(),
        LatLonGeometryValue::Line(l) => l.to_string(),
        LatLonGeometryValue::Polygon(p) => p.to_string(),
        LatLonGeometryValue::Circle(c) => c.to_string(),
    }
}

/// `Arrays.toString(geometries)`.
fn geometries_to_string(geometries: &[LatLonGeometryValue]) -> String {
    let parts: Vec<String> = geometries.iter().map(geometry_to_string).collect();
    format!("[{}]", parts.join(", "))
}

/// `new Polygon(lats, lons, holes...)`.
fn polygon(lats: &[f64], lons: &[f64], holes: Vec<Polygon>) -> Result<Polygon> {
    Polygon::new(lats.to_vec(), lons.to_vec(), holes)
}

/// `IOUtils.close(r, dir)`: closes both, then throws the first failure.
fn close_all(r: Arc<dyn IndexReader>, dir: &mut TestDirectory) -> Result<()> {
    let first = r.close();
    drop(r);
    let second = dir.close();
    first.and(second)
}

/// The `doubleValue()` of a stored field's `numericValue()`.
fn double_value(doc: &Document, name: &str) -> f64 {
    match doc.get_field(name).and_then(|f| f.numeric_value()) {
        Some(NumericValue::Int(v)) => f64::from(v),
        Some(NumericValue::Long(v)) => v as f64,
        Some(NumericValue::Float(v)) => f64::from(v),
        Some(NumericValue::Double(v)) => v,
        _ => randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.NullPointerException",
        ),
    }
}

/// `Sort.INDEXORDER`: sort by index order.
pub fn index_order() -> Sort {
    Sort::new_fields(vec![SortField::FIELD_DOC.clone()])
        .expect("INVARIANT: one sort field is a valid sort")
}

/// `rectContainsPoint(Rectangle rect, double pointLat, double pointLon)`
/// (`static final`).
pub fn rect_contains_point(rect: &Rectangle, point_lat: f64, point_lon: f64) -> bool {
    assert!(!point_lat.is_nan());

    if point_lat < rect.min_lat() || point_lat > rect.max_lat() {
        return false;
    }

    if rect.min_lon() <= rect.max_lon() {
        point_lon >= rect.min_lon() && point_lon <= rect.max_lon()
    } else {
        // Rect crosses dateline:
        point_lon <= rect.max_lon() || point_lon >= rect.min_lon()
    }
}

/// The anonymous `PointsFormat` of `doRandomDistanceTest`: a
/// `Lucene90PointsWriter` with `pointsInLeaf` points per leaf and
/// `BKDWriter.DEFAULT_MAX_MB_SORT_IN_HEAP` (16), and the stock reader.
#[derive(Debug)]
pub struct LeafSizePointsFormat {
    /// `pointsInLeaf`.
    pub points_in_leaf: i32,
}

impl PointsFormat for LeafSizePointsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_writer(&self, write_state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(Lucene90PointsWriter::with_config(
            write_state,
            self.points_in_leaf,
            DEFAULT_MAX_MB_SORT_IN_HEAP,
            VERSION_CURRENT,
        )?))
    }

    fn fields_reader(&self, read_state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Lucene90PointsReader::new(read_state)?))
    }
}

/// `new FilterCodec(in.getName(), in) { pointsFormat() }` over
/// `TestUtil.getDefaultCodec()` with [`LeafSizePointsFormat`].
pub fn leaf_size_codec(points_in_leaf: i32) -> Arc<dyn Codec> {
    let r#in = TestUtil::get_default_codec();
    Arc::new(
        FilterCodec::new(r#in.name().to_string(), Arc::clone(&r#in))
            .with_points_format(LeafSizePointsFormat { points_in_leaf }),
    )
}

/// The points of `searchSmallSet`.
const SMALL_SET_POINTS: [[f64; 2]; 16] = [
    [32.763420, -96.774],
    [32.7559529921407, -96.7759895324707],
    [32.77866942010977, -96.77701950073242],
    [32.7756745755423, -96.7706036567688],
    [27.703618681345585, -139.73458170890808],
    [32.94823588839368, -96.4538113027811],
    [33.06047141970814, -96.65084838867188],
    [32.778650, -96.7772],
    [-88.56029371730983, -177.23537676036358],
    [33.541429799076354, -26.779373834241003],
    [26.774024500421728, -77.35379276106497],
    [-90.0, -14.796283808944777],
    [32.94823588839368, -178.8538113027811],
    [32.94823588839368, 178.8538113027811],
    [40.720611, -73.998776],
    [-44.5, -179.5],
];

/// Port of the members of `BaseGeoPointTestCase`.
pub trait BaseGeoPointTestCase: Send + Sync + 'static {
    // TODO: remove these hooks once all subclasses can pass with new random!

    /// `nextLongitude()` (`protected`).
    fn next_longitude(&self) -> f64 {
        geo_test_util::next_longitude()
    }

    /// `nextLatitude()` (`protected`).
    fn next_latitude(&self) -> f64 {
        geo_test_util::next_latitude()
    }

    /// `nextBox()` (`protected`).
    fn next_box(&self) -> Rectangle {
        geo_test_util::next_box()
    }

    /// `nextCircle()` (`protected`).
    fn next_circle(&self) -> Circle {
        geo_test_util::next_circle()
    }

    /// `nextPolygon()` (`protected`).
    fn next_polygon(&self) -> Polygon {
        geo_test_util::next_polygon()
    }

    /// `nextGeometry()` (`protected`).
    fn next_geometry(&self) -> Vec<LatLonGeometryValue> {
        let length = random().next_int_bound(4) + 1;
        let mut geometries = Vec::with_capacity(length as usize);
        for _ in 0..length {
            let geometry = match random().next_int_bound(3) {
                0 => LatLonGeometryValue::Rectangle(self.next_box()),
                1 => LatLonGeometryValue::Circle(self.next_circle()),
                _ => LatLonGeometryValue::Polygon(self.next_polygon()),
            };
            geometries.push(geometry);
        }
        geometries
    }

    /// `quantizeLat(double lat)` (`protected`): override this to quantize
    /// randomly generated lat, so the test won't fail due to quantization
    /// errors, which are 1) annoying to debug, and 2) should never affect
    /// "real" usage terribly.
    fn quantize_lat(&self, lat: f64) -> f64 {
        lat
    }

    /// `quantizeLon(double lon)` (`protected`): override this to quantize
    /// randomly generated lon, so the test won't fail due to quantization
    /// errors, which are 1) annoying to debug, and 2) should never affect
    /// "real" usage terribly.
    fn quantize_lon(&self, lon: f64) -> f64 {
        lon
    }

    /// `addPointToDoc(String field, Document doc, double lat, double lon)`
    /// (`protected abstract`).
    fn add_point_to_doc(&self, field: &str, doc: &mut Document, lat: f64, lon: f64) -> Result<()>;

    /// `newRectQuery(String field, double minLat, double maxLat, double
    /// minLon, double maxLon)` (`protected abstract`).
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>` in `newRectQuery`, `newDistanceQuery` and
    /// `newPolygonQuery`; `None` is Java's `null`.
    fn new_rect_query(
        &self,
        field: Option<&str>,
        min_lat: f64,
        max_lat: f64,
        min_lon: f64,
        max_lon: f64,
    ) -> Result<Arc<dyn Query>>;

    /// `newDistanceQuery(String field, double centerLat, double centerLon,
    /// double radiusMeters)` (`protected abstract`).
    fn new_distance_query(
        &self,
        field: Option<&str>,
        center_lat: f64,
        center_lon: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn Query>>;

    /// `newPolygonQuery(String field, Polygon... polygon)` (`protected
    /// abstract`).
    fn new_polygon_query(
        &self,
        field: Option<&str>,
        polygon: Vec<Polygon>,
    ) -> Result<Arc<dyn Query>>;

    /// `newGeometryQuery(String field, LatLonGeometry... geometry)`
    /// (`protected abstract`).
    fn new_geometry_query(
        &self,
        field: &str,
        geometry: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>>;

    /// `testIndexExtremeValues()`: valid values that should not cause
    /// exception.
    fn test_index_extreme_values(&self) -> Result<()> {
        let mut document = Document::new();
        self.add_point_to_doc("foo", &mut document, 90.0, 180.0)?;
        self.add_point_to_doc("foo", &mut document, 90.0, -180.0)?;
        self.add_point_to_doc("foo", &mut document, -90.0, 180.0)?;
        self.add_point_to_doc("foo", &mut document, -90.0, -180.0)?;
        Ok(())
    }

    /// `testIndexOutOfRangeValues()`: invalid values.
    fn test_index_out_of_range_values(&self) -> Result<()> {
        let mut document = Document::new();

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 90.0f64.next_up(), 50.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid latitude"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, (-90.0f64).next_down(), 50.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid latitude"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 90.0, 180.0f64.next_up())
        });
        Assert::assert_true(message_contains(&expected, "invalid longitude"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 90.0, (-180.0f64).next_down())
        });
        Assert::assert_true(message_contains(&expected, "invalid longitude"));
        Ok(())
    }

    /// `testIndexNaNValues()`: NaN: illegal.
    fn test_index_nan_values(&self) -> Result<()> {
        let mut document = Document::new();

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, f64::NAN, 50.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid latitude"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 50.0, f64::NAN)
        });
        Assert::assert_true(message_contains(&expected, "invalid longitude"));
        Ok(())
    }

    /// `testIndexInfValues()`: Inf: illegal.
    fn test_index_inf_values(&self) -> Result<()> {
        let mut document = Document::new();

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, f64::INFINITY, 50.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid latitude"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, f64::NEG_INFINITY, 50.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid latitude"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 50.0, f64::INFINITY)
        });
        Assert::assert_true(message_contains(&expected, "invalid longitude"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 50.0, f64::NEG_INFINITY)
        });
        Assert::assert_true(message_contains(&expected, "invalid longitude"));
        Ok(())
    }

    /// `testBoxBasics()`: add a single point and search for it in a box.
    // NOTE: we don't currently supply an exact search, only ranges, because of the lossiness...
    fn test_box_basics(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

        // add a doc with a point
        let mut document = Document::new();
        self.add_point_to_doc("field", &mut document, 18.313694, -65.227444)?;
        writer.add_document(&document)?;

        // search and verify we found our doc
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        Assert::assert_equals(
            1,
            searcher.count(self.new_rect_query(Some("field"), 18.0, 19.0, -66.0, -65.0)?)?,
        );

        drop(searcher);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testBoxNull()`: null field name not allowed (`BaseGeoPointTestCase.java:261-269`).
    fn test_box_null(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_rect_query(None, 18.0, 19.0, -66.0, -65.0)
        });
        Assert::assert_true(message_contains(&expected, "field must not be null"));
        Ok(())
    }

    /// `testBoxInvalidCoordinates()`: box should not accept invalid lat/lon.
    fn test_box_invalid_coordinates(&self) -> Result<()> {
        expect_throws("java.lang.Exception", || {
            self.new_rect_query(Some("field"), -92.0, -91.0, 179.0, 181.0)
        });
        Ok(())
    }

    /// `testDistanceBasics()`: test we can search for a point.
    fn test_distance_basics(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

        // add a doc with a location
        let mut document = Document::new();
        self.add_point_to_doc("field", &mut document, 18.313694, -65.227444)?;
        writer.add_document(&document)?;

        // search within 50km and verify we found our doc
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        Assert::assert_equals(
            1,
            searcher.count(self.new_distance_query(Some("field"), 18.0, -65.0, 50_000.0)?)?,
        );

        drop(searcher);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testDistanceNull()`: null field name not allowed (`BaseGeoPointTestCase.java:301-309`).
    fn test_distance_null(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(None, 18.0, -65.0, 50_000.0)
        });
        Assert::assert_true(message_contains(&expected, "field must not be null"));
        Ok(())
    }

    /// `testDistanceIllegal()`: distance query should not accept invalid
    /// lat/lon as origin.
    fn test_distance_illegal(&self) -> Result<()> {
        expect_throws("java.lang.Exception", || {
            self.new_distance_query(Some("field"), 92.0, 181.0, 120000.0)
        });
        Ok(())
    }

    /// `testDistanceNegative()`: negative distance queries are not allowed.
    fn test_distance_negative(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, -1.0)
        });
        Assert::assert_true(message_contains(&expected, "radiusMeters"));
        Assert::assert_true(message_contains(&expected, "invalid"));
        Ok(())
    }

    /// `testDistanceNaN()`: NaN distance queries are not allowed.
    fn test_distance_nan(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, f64::NAN)
        });
        Assert::assert_true(message_contains(&expected, "radiusMeters"));
        Assert::assert_true(message_contains(&expected, "invalid"));
        Ok(())
    }

    /// `testDistanceInf()`: Inf distance queries are not allowed.
    fn test_distance_inf(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, f64::INFINITY)
        });
        Assert::assert_true(message_contains(&expected, "radiusMeters"));
        Assert::assert_true(message_contains(&expected, "invalid"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, f64::NEG_INFINITY)
        });
        Assert::assert_true_msg(
            &message(&expected),
            message_contains(&expected, "radiusMeters"),
        );
        Assert::assert_true(message_contains(&expected, "invalid"));
        Ok(())
    }

    /// `testPolygonBasics()`: test we can search for a polygon.
    fn test_polygon_basics(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

        // add a doc with a point
        let mut document = Document::new();
        self.add_point_to_doc("field", &mut document, 18.313694, -65.227444)?;
        writer.add_document(&document)?;

        // search and verify we found our doc
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        Assert::assert_equals(
            1,
            searcher.count(self.new_polygon_query(
                Some("field"),
                vec![polygon(
                    &[18.0, 18.0, 19.0, 19.0, 18.0],
                    &[-66.0, -65.0, -65.0, -66.0, -66.0],
                    Vec::new(),
                )?],
            )?)?,
        );

        drop(searcher);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testPolygonHole()`: test we can search for a polygon with a hole
    /// (but still includes the doc).
    fn test_polygon_hole(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

        // add a doc with a point
        let mut document = Document::new();
        self.add_point_to_doc("field", &mut document, 18.313694, -65.227444)?;
        writer.add_document(&document)?;

        // search and verify we found our doc
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let inner = polygon(
            &[18.5, 18.5, 18.7, 18.7, 18.5],
            &[-65.7, -65.4, -65.4, -65.7, -65.7],
            Vec::new(),
        )?;
        let outer = polygon(
            &[18.0, 18.0, 19.0, 19.0, 18.0],
            &[-66.0, -65.0, -65.0, -66.0, -66.0],
            vec![inner],
        )?;
        Assert::assert_equals(
            1,
            searcher.count(self.new_polygon_query(Some("field"), vec![outer])?)?,
        );

        drop(searcher);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testPolygonHoleExcludes()`: test we can search for a polygon with a
    /// hole (that excludes the doc).
    fn test_polygon_hole_excludes(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

        // add a doc with a point
        let mut document = Document::new();
        self.add_point_to_doc("field", &mut document, 18.313694, -65.227444)?;
        writer.add_document(&document)?;

        // search and verify we found our doc
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let inner = polygon(
            &[18.2, 18.2, 18.4, 18.4, 18.2],
            &[-65.3, -65.2, -65.2, -65.3, -65.3],
            Vec::new(),
        )?;
        let outer = polygon(
            &[18.0, 18.0, 19.0, 19.0, 18.0],
            &[-66.0, -65.0, -65.0, -66.0, -66.0],
            vec![inner],
        )?;
        Assert::assert_equals(
            0,
            searcher.count(self.new_polygon_query(Some("field"), vec![outer])?)?,
        );

        drop(searcher);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testMultiPolygonBasics()`: test we can search for a multi-polygon.
    fn test_multi_polygon_basics(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

        // add a doc with a point
        let mut document = Document::new();
        self.add_point_to_doc("field", &mut document, 18.313694, -65.227444)?;
        writer.add_document(&document)?;

        // search and verify we found our doc
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let a = polygon(
            &[28.0, 28.0, 29.0, 29.0, 28.0],
            &[-56.0, -55.0, -55.0, -56.0, -56.0],
            Vec::new(),
        )?;
        let b = polygon(
            &[18.0, 18.0, 19.0, 19.0, 18.0],
            &[-66.0, -65.0, -65.0, -66.0, -66.0],
            Vec::new(),
        )?;
        Assert::assert_equals(
            1,
            searcher.count(self.new_polygon_query(Some("field"), vec![a, b])?)?,
        );

        drop(searcher);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testPolygonNullField()`: null field name not allowed (`BaseGeoPointTestCase.java:472-483`).
    fn test_polygon_null_field(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_polygon_query(
                None,
                vec![polygon(
                    &[18.0, 18.0, 19.0, 19.0, 18.0],
                    &[-66.0, -65.0, -65.0, -66.0, -66.0],
                    Vec::new(),
                )?],
            )
        });
        Assert::assert_true(message_contains(&expected, "field must not be null"));
        Ok(())
    }

    /// `testSamePointManyTimes()`: a particularly tricky adversary for BKD
    /// tree.
    fn test_same_point_many_times(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;

        // Every doc has 2 points:
        let the_lat = self.next_latitude();
        let the_lon = self.next_longitude();

        let mut lats = vec![the_lat; num_points];
        let mut lons = vec![the_lon; num_points];

        self.verify(&mut lats, &mut lons)
    }

    /// `testLowCardinality()`: a particularly tricky adversary for BKD tree.
    fn test_low_cardinality(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        let cardinality = TestUtil::next_int(&*random(), 2, 20);

        let mut diff_lons = vec![0.0; cardinality as usize];
        let mut diff_lats = vec![0.0; cardinality as usize];
        for i in 0..cardinality as usize {
            diff_lats[i] = self.next_latitude();
            diff_lons[i] = self.next_longitude();
        }

        let mut lats = vec![0.0; num_points];
        let mut lons = vec![0.0; num_points];
        for i in 0..num_points {
            let index = random().next_int_bound(cardinality) as usize;
            lats[i] = diff_lats[index];
            lons[i] = diff_lons[index];
        }

        self.verify(&mut lats, &mut lons)
    }

    /// `testAllLatEqual()`.
    fn test_all_lat_equal(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        let lat = self.next_latitude();
        let mut lats = vec![0.0; num_points];
        let mut lons = vec![0.0; num_points];

        let mut have_real_doc = false;

        for doc_id in 0..num_points {
            let x = random().next_int_bound(20);
            if x == 17 {
                // Some docs don't have a point:
                lats[doc_id] = f64::NAN;
                if *VERBOSE {
                    println!("  doc={doc_id} is missing");
                }
                continue;
            }

            if doc_id > 0 && x == 14 && have_real_doc {
                let old_doc_id = loop {
                    let old_doc_id = random().next_int_bound(doc_id as i32) as usize;
                    if !lats[old_doc_id].is_nan() {
                        break old_doc_id;
                    }
                };

                // Fully identical point:
                lons[doc_id] = lons[old_doc_id];
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} lat={} lon={} (same lat/lon as doc={old_doc_id})",
                        double_to_string(lat),
                        double_to_string(lons[doc_id])
                    );
                }
            } else {
                lons[doc_id] = self.next_longitude();
                have_real_doc = true;
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} lat={} lon={}",
                        double_to_string(lat),
                        double_to_string(lons[doc_id])
                    );
                }
            }
            lats[doc_id] = lat;
        }

        self.verify(&mut lats, &mut lons)
    }

    /// `testAllLonEqual()`.
    fn test_all_lon_equal(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        let the_lon = self.next_longitude();
        let mut lats = vec![0.0; num_points];
        let mut lons = vec![0.0; num_points];

        let mut have_real_doc = false;

        // System.out.println("theLon=" + theLon);

        for doc_id in 0..num_points {
            let x = random().next_int_bound(20);
            if x == 17 {
                // Some docs don't have a point:
                lats[doc_id] = f64::NAN;
                if *VERBOSE {
                    println!("  doc={doc_id} is missing");
                }
                continue;
            }

            if doc_id > 0 && x == 14 && have_real_doc {
                let old_doc_id = loop {
                    let old_doc_id = random().next_int_bound(doc_id as i32) as usize;
                    if !lats[old_doc_id].is_nan() {
                        break old_doc_id;
                    }
                };

                // Fully identical point:
                lats[doc_id] = lats[old_doc_id];
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} lat={} lon={} (same lat/lon as doc={old_doc_id})",
                        double_to_string(lats[doc_id]),
                        double_to_string(the_lon)
                    );
                }
            } else {
                lats[doc_id] = self.next_latitude();
                have_real_doc = true;
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} lat={} lon={}",
                        double_to_string(lats[doc_id]),
                        double_to_string(the_lon)
                    );
                }
            }
            lons[doc_id] = the_lon;
        }

        self.verify(&mut lats, &mut lons)
    }

    /// `testMultiValued()`.
    fn test_multi_valued(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        // Every doc has 2 points:
        let mut lats = vec![0.0; 2 * num_points];
        let mut lons = vec![0.0; 2 * num_points];
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = LuceneTestCase::new_index_writer_config();

        // We rely on docID order:
        iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        // and on seeds being able to reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

        for id in 0..num_points {
            let mut doc = Document::new();
            lats[2 * id] = self.quantize_lat(self.next_latitude());
            lons[2 * id] = self.quantize_lon(self.next_longitude());
            doc.add(Box::new(StringField::new(
                "id",
                id.to_string(),
                Store::YES,
            )?));
            self.add_point_to_doc(FIELD_NAME, &mut doc, lats[2 * id], lons[2 * id])?;
            lats[2 * id + 1] = self.quantize_lat(self.next_latitude());
            lons[2 * id + 1] = self.quantize_lon(self.next_longitude());
            self.add_point_to_doc(FIELD_NAME, &mut doc, lats[2 * id + 1], lons[2 * id + 1])?;

            if *VERBOSE {
                println!("id={id}");
                println!(
                    "  lat={} lon={}",
                    double_to_string(lats[2 * id]),
                    double_to_string(lons[2 * id])
                );
                println!(
                    "  lat={} lon={}",
                    double_to_string(lats[2 * id + 1]),
                    double_to_string(lons[2 * id + 1])
                );
            }
            w.add_document(&doc)?;
        }

        // TODO: share w/ verify; just need parallel array of the expected ids
        if random().next_boolean() {
            w.force_merge(1)?;
        }
        let dr = w.get_reader()?;
        let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;
        w.close()?;
        drop(w);

        let s = LuceneTestCase::new_searcher(Arc::clone(&dr));

        let iters = LuceneTestCase::at_least(25);
        for iter in 0..iters {
            let rect = self.next_box();

            if *VERBOSE {
                println!("\nTEST: iter={iter} rect={rect}");
            }

            let query = self.new_rect_query(
                Some(FIELD_NAME),
                rect.min_lat(),
                rect.max_lat(),
                rect.min_lon(),
                rect.max_lon(),
            )?;

            let hits = search_index(&s, query, r.max_doc())?;

            let mut fail = false;

            let stored_fields = s.stored_fields()?;
            for doc_id in 0..lats.len() / 2 {
                let lat_doc1 = lats[2 * doc_id];
                let lon_doc1 = lons[2 * doc_id];
                let lat_doc2 = lats[2 * doc_id + 1];
                let lon_doc2 = lons[2 * doc_id + 1];

                let result1 = rect_contains_point(&rect, lat_doc1, lon_doc1);
                let result2 = rect_contains_point(&rect, lat_doc2, lon_doc2);

                let expected = result1 || result2;

                if hits.get(doc_id) != expected {
                    let id = stored_fields
                        .document(doc_id as i32)?
                        .get_field("id")
                        .and_then(|f| f.string_value())
                        .unwrap_or_else(|| "null".to_string());
                    if expected {
                        println!("TEST: id={id} docID={doc_id} should match but did not");
                    } else {
                        println!("TEST: id={id} docID={doc_id} should not match but did");
                    }
                    println!("  rect={rect}");
                    println!(
                        "  lat={} lon={}\n  lat={} lon={}",
                        double_to_string(lat_doc1),
                        double_to_string(lon_doc1),
                        double_to_string(lat_doc2),
                        double_to_string(lon_doc2)
                    );
                    println!("  result1={result1} result2={result2}");
                    fail = true;
                }
            }

            if fail {
                Assert::fail("some hits were wrong");
            }
        }
        drop(s);
        drop(dr);
        r.close()?;
        drop(r);
        dir.close()
    }

    /// `testRandomTiny()`.
    fn test_random_tiny(&self) -> Result<()> {
        // Make sure single-leaf-node case is OK:
        self.do_test_random(10)
    }

    /// `testRandomMedium()`.
    fn test_random_medium(&self) -> Result<()> {
        self.do_test_random(1000)
    }

    /// `testRandomBig()` (`@Nightly`).
    fn test_random_big(&self) -> Result<()> {
        LuceneTestCase::assume_false(
            "Direct codec can OOME on this test",
            TestUtil::get_doc_values_format(FIELD_NAME) == "Direct",
        );
        self.do_test_random(200000)
    }

    /// `doTestRandom(int count)` (`private`).
    fn do_test_random(&self, count: i32) -> Result<()> {
        let num_points = LuceneTestCase::at_least(count) as usize;

        if *VERBOSE {
            println!("TEST: numPoints={num_points}");
        }

        let mut lats = vec![0.0; num_points];
        let mut lons = vec![0.0; num_points];

        let mut have_real_doc = false;

        for id in 0..num_points {
            let x = random().next_int_bound(20);
            if x == 17 {
                // Some docs don't have a point:
                lats[id] = f64::NAN;
                if *VERBOSE {
                    println!("  id={id} is missing");
                }
                continue;
            }

            if id > 0 && x < 3 && have_real_doc {
                let old_id = loop {
                    let old_id = random().next_int_bound(id as i32) as usize;
                    if !lats[old_id].is_nan() {
                        break old_id;
                    }
                };

                if x == 0 {
                    // Identical lat to old point
                    lats[id] = lats[old_id];
                    lons[id] = self.next_longitude();
                    if *VERBOSE {
                        println!(
                            "  id={id} lat={} lon={} (same lat as doc={old_id})",
                            double_to_string(lats[id]),
                            double_to_string(lons[id])
                        );
                    }
                } else if x == 1 {
                    // Identical lon to old point
                    lats[id] = self.next_latitude();
                    lons[id] = lons[old_id];
                    if *VERBOSE {
                        println!(
                            "  id={id} lat={} lon={} (same lon as doc={old_id})",
                            double_to_string(lats[id]),
                            double_to_string(lons[id])
                        );
                    }
                } else {
                    assert_eq!(x, 2);
                    // Fully identical point:
                    lats[id] = lats[old_id];
                    lons[id] = lons[old_id];
                    if *VERBOSE {
                        println!(
                            "  id={id} lat={} lon={} (same lat/lon as doc={old_id})",
                            double_to_string(lats[id]),
                            double_to_string(lons[id])
                        );
                    }
                }
            } else {
                lats[id] = self.next_latitude();
                lons[id] = self.next_longitude();
                have_real_doc = true;
                if *VERBOSE {
                    println!(
                        "  id={id} lat={} lon={}",
                        double_to_string(lats[id]),
                        double_to_string(lons[id])
                    );
                }
            }
        }

        self.verify(&mut lats, &mut lons)
    }

    /// `verify(double[] lats, double[] lons)` (`private`).
    fn verify(&self, lats: &mut [f64], lons: &mut [f64]) -> Result<()> {
        // quantize each value the same way the index does
        // NaN means missing for the doc!!!!!
        for lat in lats.iter_mut() {
            if !lat.is_nan() {
                *lat = self.quantize_lat(*lat);
            }
        }
        for lon in lons.iter_mut() {
            if !lon.is_nan() {
                *lon = self.quantize_lon(*lon);
            }
        }
        self.verify_random_rectangles(lats, lons)?;
        self.verify_random_distances(lats, lons)?;
        self.verify_random_polygons(lats, lons)?;
        self.verify_random_geometries(lats, lons)
    }

    /// The index setup the four `verifyRandom…` methods share: the
    /// `IndexWriterConfig`, the directory, the indexed points, the reader.
    #[doc(hidden)]
    fn open_verify_index(
        &self,
        lats: &[f64],
        lons: &[f64],
    ) -> Result<(TestDirectory, Arc<dyn DirectoryReader>)> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        // Else seeds may not reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        // Else we can get O(N^2) merging:
        let mbd = iwc.max_buffered_docs();
        if mbd != -1 && mbd < (lats.len() / 100) as i32 {
            iwc.set_max_buffered_docs((lats.len() / 100) as i32)?;
        }
        let dir = if lats.len() > 100000 {
            // Avoid slow codecs like SimpleText
            iwc.set_codec(TestUtil::get_default_codec())?;
            let simple_name = LuceneTestCase::get_test_class().simple_name().to_string();
            LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(&simple_name))
        } else {
            LuceneTestCase::new_directory()
        };

        let mut deleted: HashSet<i32> = HashSet::new();
        // RandomIndexWriter is too slow here:
        let w = IndexWriter::new(dir.as_directory(), iwc)?;
        self.index_points(lats, lons, &mut deleted, &w)?;

        let r = directory_reader::open_with_writer(&*w)?;
        w.close()?;
        drop(w);
        Ok((dir, r))
    }

    /// `verifyRandomRectangles(double[] lats, double[] lons)` (`protected`).
    fn verify_random_rectangles(&self, lats: &[f64], lons: &[f64]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(lats, lons)?;
        let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;

        let s = LuceneTestCase::new_searcher(Arc::clone(&dr));

        let iters = LuceneTestCase::at_least(25);

        let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
        let max_doc = s.get_index_reader().max_doc();

        for iter in 0..iters {
            if *VERBOSE {
                println!("\nTEST: iter={iter} s={s:?}");
            }

            let rect = self.next_box();

            let query = self.new_rect_query(
                Some(FIELD_NAME),
                rect.min_lat(),
                rect.max_lat(),
                rect.min_lon(),
                rect.max_lon(),
            )?;

            if *VERBOSE {
                println!("  query={}", query.to_query_string(""));
            }

            let hits = search_index(&s, Arc::clone(&query), max_doc)?;

            let mut fail = false;
            let mut doc_id_to_id = numeric_ids(&r)?;
            for doc_id in 0..max_doc {
                Assert::assert_equals(doc_id, doc_id_to_id.next_doc()?);
                let id = doc_id_to_id.long_value()? as i32 as usize;
                let expected = if is_deleted(&live_docs, doc_id) {
                    // document is deleted
                    false
                } else if lats[id].is_nan() {
                    false
                } else {
                    rect_contains_point(&rect, lats[id], lons[id])
                };

                if hits.get(doc_id as usize) != expected {
                    build_error(doc_id, expected, id, lats, lons, &*query, &live_docs, |b| {
                        b.push_str(&format!("  rect={rect}"))
                    });
                    fail = true;
                }
            }
            if fail {
                Assert::fail("some hits were wrong");
            }
        }

        drop(s);
        drop(live_docs);
        drop(dr);
        close_all(r, &mut dir)
    }

    /// `verifyRandomDistances(double[] lats, double[] lons)` (`protected`).
    fn verify_random_distances(&self, lats: &[f64], lons: &[f64]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(lats, lons)?;
        let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;

        let s = LuceneTestCase::new_searcher(Arc::clone(&dr));

        let iters = LuceneTestCase::at_least(25);

        let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
        let max_doc = s.get_index_reader().max_doc();

        for iter in 0..iters {
            if *VERBOSE {
                println!("\nTEST: iter={iter} s={s:?}");
            }

            // Distance
            let center_lat = self.next_latitude();
            let center_lon = self.next_longitude();

            // So the query can cover at most 50% of the earth's surface:
            let radius_meters =
                random().next_double() * GeoUtils::EARTH_MEAN_RADIUS_METERS * std::f64::consts::PI
                    / 2.0
                    + 1.0;

            if *VERBOSE {
                println!(
                    "  radiusMeters = {}",
                    decimal_format_grouped_2(radius_meters)
                );
            }

            let query =
                self.new_distance_query(Some(FIELD_NAME), center_lat, center_lon, radius_meters)?;

            if *VERBOSE {
                println!("  query={}", query.to_query_string(""));
            }

            let hits = search_index(&s, Arc::clone(&query), max_doc)?;

            let mut fail = false;
            let mut doc_id_to_id = numeric_ids(&r)?;
            for doc_id in 0..max_doc {
                Assert::assert_equals(doc_id, doc_id_to_id.next_doc()?);
                let id = doc_id_to_id.long_value()? as i32 as usize;
                let expected = if is_deleted(&live_docs, doc_id) {
                    // document is deleted
                    false
                } else if lats[id].is_nan() {
                    false
                } else {
                    SloppyMath::haversin_meters(center_lat, center_lon, lats[id], lons[id])
                        <= radius_meters
                };

                if hits.get(doc_id as usize) != expected {
                    let explain = |b: &mut String| {
                        if !lats[id].is_nan() {
                            let distance_meters = SloppyMath::haversin_meters(
                                center_lat, center_lon, lats[id], lons[id],
                            );
                            b.push_str(&format!(
                                "  centerLat={} centerLon={} distanceMeters={} vs radiusMeters={}",
                                double_to_string(center_lat),
                                double_to_string(center_lon),
                                double_to_string(distance_meters),
                                double_to_string(radius_meters)
                            ));
                        }
                    };
                    build_error(
                        doc_id, expected, id, lats, lons, &*query, &live_docs, explain,
                    );
                    fail = true;
                }
            }
            if fail {
                Assert::fail("some hits were wrong");
            }
        }

        drop(s);
        drop(live_docs);
        drop(dr);
        close_all(r, &mut dir)
    }

    /// `verifyRandomPolygons(double[] lats, double[] lons)` (`protected`).
    fn verify_random_polygons(&self, lats: &[f64], lons: &[f64]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(lats, lons)?;
        let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;

        // We can't wrap with "exotic" readers because points needs to work:
        let s = LuceneTestCase::new_searcher(Arc::clone(&dr));

        let iters = LuceneTestCase::at_least(75);

        let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
        let max_doc = s.get_index_reader().max_doc();

        for iter in 0..iters {
            if *VERBOSE {
                println!("\nTEST: iter={iter} s={s:?}");
            }

            // Polygon
            let polygon = self.next_polygon();
            let query = self.new_polygon_query(Some(FIELD_NAME), vec![polygon.clone()])?;

            if *VERBOSE {
                println!("  query={}", query.to_query_string(""));
            }

            let hits = search_index(&s, Arc::clone(&query), max_doc)?;

            let mut fail = false;
            let mut doc_id_to_id = numeric_ids(&r)?;
            for doc_id in 0..max_doc {
                Assert::assert_equals(doc_id, doc_id_to_id.next_doc()?);
                let id = doc_id_to_id.long_value()? as i32 as usize;
                let expected = if is_deleted(&live_docs, doc_id) {
                    // document is deleted
                    false
                } else if lats[id].is_nan() {
                    false
                } else {
                    geo_test_util::contains_slowly(&polygon, lats[id], lons[id])?
                };

                if hits.get(doc_id as usize) != expected {
                    build_error(doc_id, expected, id, lats, lons, &*query, &live_docs, |b| {
                        b.push_str(&format!("  polygon={polygon}"))
                    });
                    fail = true;
                }
            }
            if fail {
                Assert::fail("some hits were wrong");
            }
        }

        drop(s);
        drop(live_docs);
        drop(dr);
        close_all(r, &mut dir)
    }

    /// `verifyRandomGeometries(double[] lats, double[] lons)` (`protected`).
    fn verify_random_geometries(&self, lats: &[f64], lons: &[f64]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(lats, lons)?;
        let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;

        // We can't wrap with "exotic" readers because points needs to work:
        let s = LuceneTestCase::new_searcher(Arc::clone(&dr));

        let iters = LuceneTestCase::at_least(75);

        let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
        let max_doc = s.get_index_reader().max_doc();

        for iter in 0..iters {
            if *VERBOSE {
                println!("\nTEST: iter={iter} s={s:?}");
            }

            // Polygon
            let geometries = self.next_geometry();
            let query = self.new_geometry_query(FIELD_NAME, geometries.clone())?;

            if *VERBOSE {
                println!("  query={}", query.to_query_string(""));
            }

            let hits = search_index(&s, Arc::clone(&query), max_doc)?;

            let component2d = LatLonGeometryValue::create(&geometries)?;

            let mut fail = false;
            let mut doc_id_to_id = numeric_ids(&r)?;
            for doc_id in 0..max_doc {
                Assert::assert_equals(doc_id, doc_id_to_id.next_doc()?);
                let id = doc_id_to_id.long_value()? as i32 as usize;
                let expected = if is_deleted(&live_docs, doc_id) {
                    // document is deleted
                    false
                } else if lats[id].is_nan() {
                    false
                } else {
                    component2d.contains(self.quantize_lon(lons[id]), self.quantize_lat(lats[id]))
                };

                if hits.get(doc_id as usize) != expected {
                    build_error(doc_id, expected, id, lats, lons, &*query, &live_docs, |b| {
                        b.push_str(&format!("  geometry={}", geometries_to_string(&geometries)))
                    });
                    fail = true;
                }
            }
            if fail {
                Assert::fail("some hits were wrong");
            }
        }

        drop(s);
        drop(live_docs);
        drop(dr);
        close_all(r, &mut dir)
    }

    /// `indexPoints(double[] lats, double[] lons, Set<Integer> deleted,
    /// IndexWriter w)` (`private`).
    fn index_points(
        &self,
        lats: &[f64],
        lons: &[f64],
        deleted: &mut HashSet<i32>,
        w: &IndexWriter,
    ) -> Result<()> {
        for id in 0..lats.len() {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
            doc.add(Box::new(NumericDocValuesField::new("id", id as i64)));
            if !lats[id].is_nan() {
                self.add_point_to_doc(FIELD_NAME, &mut doc, lats[id], lons[id])?;
            }
            w.add_document(&doc)?;
            if id > 0 && random().next_int_bound(100) == 42 {
                let id_to_delete = random().next_int_bound(id as i32);
                w.delete_documents_by_terms(vec![Term::new(
                    "id",
                    BytesRef::from_char_sequence(&id_to_delete.to_string()),
                )])?;
                deleted.insert(id_to_delete);
                if *VERBOSE {
                    println!("  delete id={id_to_delete}");
                }
            }
        }

        if random().next_boolean() {
            w.force_merge(1)?;
        }
        Ok(())
    }

    /// `testRectBoundariesAreInclusive()`.
    fn test_rect_boundaries_are_inclusive(&self) -> Result<()> {
        // TODO: why this dateline leniency???
        let rect = loop {
            let rect = self.next_box();
            if !rect.crosses_dateline() {
                break rect;
            }
        };
        // this test works in quantized space: for testing inclusiveness of exact edges it must be
        // aware of index-time quantization!
        let rect = Rectangle::new(
            self.quantize_lat(rect.min_lat()),
            self.quantize_lat(rect.max_lat()),
            self.quantize_lon(rect.min_lon()),
            self.quantize_lon(rect.max_lon()),
        )?;
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = LuceneTestCase::new_index_writer_config();
        // Else seeds may not reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
        for x in 0..3 {
            let lat = if x == 0 {
                rect.min_lat()
            } else if x == 1 {
                self.quantize_lat((rect.min_lat() + rect.max_lat()) / 2.0)
            } else {
                rect.max_lat()
            };
            for y in 0..3 {
                let lon = if y == 0 {
                    rect.min_lon()
                } else if y == 1 {
                    if x == 1 {
                        continue;
                    }
                    self.quantize_lon((rect.min_lon() + rect.max_lon()) / 2.0)
                } else {
                    rect.max_lon()
                };

                let mut doc = Document::new();
                self.add_point_to_doc(FIELD_NAME, &mut doc, lat, lon)?;
                w.add_document(&doc)?;
            }
        }
        let r = w.get_reader()?;
        let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
        let field = FIELD_NAME;
        // exact edge cases
        Assert::assert_equals(
            8,
            s.count(self.new_rect_query(
                Some(field),
                rect.min_lat(),
                rect.max_lat(),
                rect.min_lon(),
                rect.max_lon(),
            )?)?,
        );

        // expand 1 ulp in each direction if possible and test a slightly larger box!
        if rect.min_lat() != -90.0 {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_lat().next_down(),
                    rect.max_lat(),
                    rect.min_lon(),
                    rect.max_lon(),
                )?)?,
            );
        }
        if rect.max_lat() != 90.0 {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_lat(),
                    rect.max_lat().next_up(),
                    rect.min_lon(),
                    rect.max_lon(),
                )?)?,
            );
        }
        if rect.min_lon() != -180.0 {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_lat(),
                    rect.max_lat(),
                    rect.min_lon().next_down(),
                    rect.max_lon(),
                )?)?,
            );
        }
        if rect.max_lon() != 180.0 {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_lat(),
                    rect.max_lat(),
                    rect.min_lon(),
                    rect.max_lon().next_up(),
                )?)?,
            );
        }

        // now shrink 1 ulp in each direction if possible: it should not include bogus stuff
        // we can't shrink if values are already at extremes, and
        // we can't do this if rectangle is actually a line or we will create a cross-dateline query
        if rect.min_lat() != 90.0
            && rect.max_lat() != -90.0
            && rect.min_lon() != 80.0
            && rect.max_lon() != -180.0
            && rect.min_lon() != rect.max_lon()
        {
            // note we put points on "sides" not just "corners" so we just shrink all 4 at once for
            // now: it should exclude all points!
            Assert::assert_equals(
                0,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_lat().next_up(),
                    rect.max_lat().next_down(),
                    rect.min_lon().next_up(),
                    rect.max_lon().next_down(),
                )?)?,
            );
        }

        drop(s);
        r.close()?;
        drop(r);
        w.close()?;
        drop(w);
        dir.close()
    }

    /// `testRandomDistance()`: run a few iterations with just 10 docs,
    /// hopefully easy to debug.
    fn test_random_distance(&self) -> Result<()> {
        let num_iters = LuceneTestCase::at_least(1);
        for _ in 0..num_iters {
            self.do_random_distance_test(10, 100)?;
        }
        Ok(())
    }

    /// `testRandomDistanceHuge()` (`@Nightly`): runs with thousands of docs.
    fn test_random_distance_huge(&self) -> Result<()> {
        for _ in 0..10 {
            self.do_random_distance_test(2000, 100)?;
        }
        Ok(())
    }

    /// `doRandomDistanceTest(int numDocs, int numQueries)` (`private`).
    fn do_random_distance_test(&self, num_docs: i32, num_queries: i32) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = LuceneTestCase::new_index_writer_config();
        // Else seeds may not reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        let points_in_leaf = 2 + random().next_int_bound(4);
        iwc.set_codec(leaf_size_codec(points_in_leaf))?;
        let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

        for _ in 0..num_docs {
            let lat_raw = self.next_latitude();
            let lon_raw = self.next_longitude();
            // pre-normalize up front, so we can just use quantized value for testing and do simple
            // exact comparisons
            let lat = self.quantize_lat(lat_raw);
            let lon = self.quantize_lon(lon_raw);
            let mut doc = Document::new();
            self.add_point_to_doc("field", &mut doc, lat, lon)?;
            doc.add(Box::new(StoredField::new_number(
                "lat",
                NumericValue::Double(lat),
            )?));
            doc.add(Box::new(StoredField::new_number(
                "lon",
                NumericValue::Double(lon),
            )?));
            writer.add_document(&doc)?;
        }
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let stored_fields = reader.stored_fields()?;
        for _ in 0..num_queries {
            let lat = self.next_latitude();
            let lon = self.next_longitude();
            let radius = 50000000f64 * random().next_double();

            let mut expected = JavaBitSet::new();
            for doc in 0..reader.max_doc() {
                let stored = stored_fields.document(doc)?;
                let doc_latitude = double_value(&stored, "lat");
                let doc_longitude = double_value(&stored, "lon");
                let distance = SloppyMath::haversin_meters(lat, lon, doc_latitude, doc_longitude);
                if distance <= radius {
                    expected.set(doc as usize);
                }
            }

            let top_docs = searcher.search_with_sort(
                self.new_distance_query(Some("field"), lat, lon, radius)?,
                reader.max_doc(),
                &index_order(),
            )?;
            let mut actual = JavaBitSet::new();
            for doc in &top_docs.score_docs {
                actual.set(doc.score_doc.doc as usize);
            }

            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Assert::assert_equals(&expected, &actual);
            }));
            if let Err(payload) = outcome {
                println!(
                    "center: ({},{}), radius={}",
                    double_to_string(lat),
                    double_to_string(lon),
                    double_to_string(radius)
                );
                for doc in 0..reader.max_doc() {
                    let stored = stored_fields.document(doc)?;
                    let doc_latitude = double_value(&stored, "lat");
                    let doc_longitude = double_value(&stored, "lon");
                    let distance =
                        SloppyMath::haversin_meters(lat, lon, doc_latitude, doc_longitude);
                    println!(
                        "{doc}: ({},{}), distance={}",
                        double_to_string(doc_latitude),
                        double_to_string(doc_longitude),
                        double_to_string(distance)
                    );
                }
                std::panic::resume_unwind(payload);
            }
        }
        drop(searcher);
        drop(stored_fields);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testEquals()`.
    fn test_equals(&self) -> Result<()> {
        let rect = self.next_box();

        let q1 = self.new_rect_query(
            Some("field"),
            rect.min_lat(),
            rect.max_lat(),
            rect.min_lon(),
            rect.max_lon(),
        )?;
        let q2 = self.new_rect_query(
            Some("field"),
            rect.min_lat(),
            rect.max_lat(),
            rect.min_lon(),
            rect.max_lon(),
        )?;
        Assert::assert_equals(QueryObject(Arc::clone(&q1)), QueryObject(q2));
        // for "impossible" ranges LatLonPoint.newBoxQuery will return MatchNoDocsQuery
        // changing the field is unrelated to that.
        if q1.as_any().downcast_ref::<MatchNoDocsQuery>().is_none() {
            Assert::assert_false(q1.query_eq(&*self.new_rect_query(
                Some("field2"),
                rect.min_lat(),
                rect.max_lat(),
                rect.min_lon(),
                rect.max_lon(),
            )?));
        }

        let lat = self.next_latitude();
        let lon = self.next_longitude();
        let q1 = self.new_distance_query(Some("field"), lat, lon, 10000.0)?;
        let q2 = self.new_distance_query(Some("field"), lat, lon, 10000.0)?;
        Assert::assert_equals(QueryObject(Arc::clone(&q1)), QueryObject(q2));
        Assert::assert_false(q1.query_eq(&*self.new_distance_query(
            Some("field2"),
            lat,
            lon,
            10000.0,
        )?));

        let lats = [
            rect.min_lat(),
            rect.max_lat(),
            rect.max_lat(),
            rect.min_lat(),
            rect.min_lat(),
        ];
        let lons = [
            rect.min_lon(),
            rect.min_lon(),
            rect.max_lon(),
            rect.max_lon(),
            rect.min_lon(),
        ];
        let q1 = self.new_polygon_query(Some("field"), vec![polygon(&lats, &lons, Vec::new())?])?;
        let q2 = self.new_polygon_query(Some("field"), vec![polygon(&lats, &lons, Vec::new())?])?;
        Assert::assert_equals(QueryObject(Arc::clone(&q1)), QueryObject(q2));
        Assert::assert_false(q1.query_eq(
            &*self.new_polygon_query(Some("field2"), vec![polygon(&lats, &lons, Vec::new())?])?,
        ));
        Ok(())
    }

    /// `searchSmallSet(Query query, int size)` (`private`): return topdocs
    /// over a small set of points in field "point".
    fn search_small_set(&self, query: Arc<dyn Query>, size: i32) -> Result<TopDocs> {
        // this is a simple systematic test, indexing these points
        // TODO: fragile: does not understand quantization in any way yet uses extremely high
        // precision!
        let pts = &SMALL_SET_POINTS;

        let mut directory = LuceneTestCase::new_directory();

        // TODO: must these simple tests really rely on docid order?
        let mut iwc: IndexWriterConfig = LuceneTestCase::new_index_writer_config_analyzer(
            Arc::new(MockAnalyzer::new(&*random())),
        );
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 100, 1000))?;
        iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        // Else seeds may not reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwc)?;

        for p in pts {
            let mut doc = Document::new();
            self.add_point_to_doc("point", &mut doc, p[0], p[1])?;
            writer.add_document(&doc)?;
        }

        // add explicit multi-valued docs
        for i in (0..pts.len()).step_by(2) {
            let mut doc = Document::new();
            self.add_point_to_doc("point", &mut doc, pts[i][0], pts[i][1])?;
            self.add_point_to_doc("point", &mut doc, pts[i + 1][0], pts[i + 1][1])?;
            writer.add_document(&doc)?;
        }

        // index random string documents
        let mut i = 0;
        while i < random().next_int_bound(10) {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "string",
                i.to_string(),
                Store::NO,
            )?));
            writer.add_document(&doc)?;
            i += 1;
        }

        let reader = writer.get_reader()?;
        writer.close()?;
        drop(writer);

        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let top_docs = searcher.search(query, size)?;
        drop(searcher);
        reader.close()?;
        drop(reader);
        directory.close()?;
        Ok(top_docs)
    }

    /// `testSmallSetRect()`.
    fn test_small_set_rect(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_rect_query(Some("point"), 32.778, 32.779, -96.778, -96.777)?,
            5,
        )?;
        Assert::assert_equals(4, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetDateline()`.
    fn test_small_set_dateline(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_rect_query(Some("point"), -45.0, -44.0, 179.0, -179.0)?,
            20,
        )?;
        Assert::assert_equals(2, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetMultiValued()`.
    fn test_small_set_multi_valued(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_rect_query(Some("point"), 32.755, 32.776, -96.454, -96.770)?,
            20,
        )?;
        // 3 single valued docs + 2 multi-valued docs
        Assert::assert_equals(5, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetWholeMap()`.
    fn test_small_set_whole_map(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_rect_query(
                Some("point"),
                GeoUtils::MIN_LAT_INCL,
                GeoUtils::MAX_LAT_INCL,
                GeoUtils::MIN_LON_INCL,
                GeoUtils::MAX_LON_INCL,
            )?,
            20,
        )?;
        Assert::assert_equals(24, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetPoly()`.
    fn test_small_set_poly(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_polygon_query(
                Some("point"),
                vec![polygon(
                    &[
                        33.073130, 32.9942669, 32.938386, 33.0374494, 33.1369762, 33.1162747,
                        33.073130, 33.073130,
                    ],
                    &[
                        -96.7682647,
                        -96.8280029,
                        -96.6288757,
                        -96.4929199,
                        -96.6041564,
                        -96.7449188,
                        -96.76826477,
                        -96.7682647,
                    ],
                    Vec::new(),
                )?],
            )?,
            5,
        )?;
        Assert::assert_equals(2, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetPolyWholeMap()`.
    fn test_small_set_poly_whole_map(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_polygon_query(
                Some("point"),
                vec![polygon(
                    &[
                        GeoUtils::MIN_LAT_INCL,
                        GeoUtils::MAX_LAT_INCL,
                        GeoUtils::MAX_LAT_INCL,
                        GeoUtils::MIN_LAT_INCL,
                        GeoUtils::MIN_LAT_INCL,
                    ],
                    &[
                        GeoUtils::MIN_LON_INCL,
                        GeoUtils::MIN_LON_INCL,
                        GeoUtils::MAX_LON_INCL,
                        GeoUtils::MAX_LON_INCL,
                        GeoUtils::MIN_LON_INCL,
                    ],
                    Vec::new(),
                )?],
            )?,
            20,
        )?;
        Assert::assert_equals_msg(Some("testWholeMap failed"), 24, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetDistance()`.
    fn test_small_set_distance(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_distance_query(Some("point"), 32.94823588839368, -96.4538113027811, 6000.0)?,
            20,
        )?;
        Assert::assert_equals(2, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetTinyDistance()`.
    fn test_small_set_tiny_distance(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_distance_query(Some("point"), 40.720611, -73.998776, 1.0)?,
            20,
        )?;
        Assert::assert_equals(2, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetDistanceNotEmpty()`: see
    /// <https://issues.apache.org/jira/browse/LUCENE-6905>.
    fn test_small_set_distance_not_empty(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_distance_query(
                Some("point"),
                -88.56029371730983,
                -177.23537676036358,
                7757.999232959935,
            )?,
            20,
        )?;
        Assert::assert_equals(2, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetHugeDistance()`: explicitly large.
    fn test_small_set_huge_distance(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_distance_query(
                Some("point"),
                32.94823588839368,
                -96.4538113027811,
                6000000.0,
            )?,
            20,
        )?;
        Assert::assert_equals(16, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetDistanceDateline()`.
    fn test_small_set_distance_dateline(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_distance_query(
                Some("point"),
                32.94823588839368,
                -179.9538113027811,
                120000.0,
            )?,
            20,
        )?;
        Assert::assert_equals(3, td.total_hits.value());
        Ok(())
    }

    /// `testNarrowPolygonCloseToNorthPole()`.
    fn test_narrow_polygon_close_to_north_pole(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        let mut dir = LuceneTestCase::new_directory();
        let w = IndexWriter::new(dir.as_directory(), iwc)?;

        // index point closes to Lat 90
        let mut doc = Document::new();
        let base = i32::MAX;
        self.add_point_to_doc(
            FIELD_NAME,
            &mut doc,
            GeoEncodingUtils::decode_latitude(base - 2),
            GeoEncodingUtils::decode_longitude(base - 2),
        )?;
        w.add_document(&doc)?;
        w.flush()?;

        // query testing
        let reader = directory_reader::open_with_writer(&*w)?;
        let s = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let min_lat = GeoEncodingUtils::decode_latitude(base - 3);
        let max_lat = GeoEncodingUtils::decode_latitude(base);
        let min_lon = GeoEncodingUtils::decode_longitude(base - 3);
        let max_lon = GeoEncodingUtils::decode_longitude(base);

        let query = self.new_polygon_query(
            Some(FIELD_NAME),
            vec![polygon(
                &[min_lat, min_lat, max_lat, max_lat, min_lat],
                &[min_lon, max_lon, max_lon, min_lon, min_lon],
                Vec::new(),
            )?],
        )?;

        Assert::assert_equals(1, s.count(query)?);
        drop(s);
        // IOUtils.close(w, reader, dir)
        let first = w.close();
        drop(w);
        let second = reader.close();
        drop(reader);
        let third = dir.close();
        first.and(second).and(third)
    }
}

/// `searchIndex(IndexSearcher s, Query query, int maxDoc)` (`private`).
fn search_index(s: &IndexSearcher, query: Arc<dyn Query>, max_doc: i32) -> Result<FixedBitSet> {
    s.search_with_manager(query, &FixedBitSetCollector::create_manager(max_doc))
}

/// `MultiDocValues.getNumericValues(r, "id")`, `NullPointerException` on
/// its use when absent.
fn numeric_ids(
    r: &Arc<dyn IndexReader>,
) -> Result<Box<dyn rucene::index::doc_values::NumericDocValues>> {
    Ok(
        MultiDocValues::get_numeric_values(r, "id")?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        }),
    )
}

/// `liveDocs != null && liveDocs.get(docID) == false`.
fn is_deleted(live_docs: &Option<Box<dyn Bits>>, doc_id: i32) -> bool {
    live_docs.as_ref().is_some_and(|l| !l.get(doc_id as usize))
}

/// `buildError(int docID, boolean expected, int id, double[] lats, double[]
/// lons, Query query, Bits liveDocs, Consumer<StringBuilder> explain)`
/// (`private`).
#[allow(clippy::too_many_arguments)]
fn build_error(
    doc_id: i32,
    expected: bool,
    id: usize,
    lats: &[f64],
    lons: &[f64],
    query: &dyn Query,
    live_docs: &Option<Box<dyn Bits>>,
    explain: impl FnOnce(&mut String),
) {
    let mut b = String::new();
    if expected {
        b.push_str(&format!("FAIL: id={id} should match but did not\n"));
    } else {
        b.push_str(&format!("FAIL: id={id} should not match but did\n"));
    }
    b.push_str(&format!(
        "  query={} docID={doc_id}\n",
        query.to_query_string("")
    ));
    b.push_str(&format!(
        "  lat={} lon={}\n",
        double_to_string(lats[id]),
        double_to_string(lons[id])
    ));
    b.push_str(&format!("  deleted?={}", is_deleted(live_docs, doc_id)));
    explain(&mut b);
    Assert::fail(&format!("wrong hit (first of possibly more):\n\n{b}"));
}

/// `new DecimalFormat("#,###.00", DecimalFormatSymbols.getInstance(
/// Locale.ENGLISH)).format(value)` for a finite, non-negative value:
/// grouping by thousands, exactly two fraction digits, `HALF_EVEN`
/// rounding (`DecimalFormat`'s default) of the value's exact binary
/// expansion.
pub fn decimal_format_grouped_2(value: f64) -> String {
    // `{:.2}` rounds the exact binary value half-to-even, as DecimalFormat
    // does for a double since JDK 8.
    let fixed = format!("{value:.2}");
    let (int_part, frac_part) = fixed.split_once('.').unwrap_or((&fixed, "00"));
    let digits: Vec<char> = int_part.chars().collect();
    let mut grouped = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(*c);
    }
    // "#,###" prints no integer digit for a value below one.
    if grouped == "0" {
        grouped.clear();
    }
    format!("{grouped}.{frac_part}")
}
