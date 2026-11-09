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

//! Port of `org.apache.lucene.tests.geo.BaseXYPointTestCase`: abstract class
//! to do basic tests for a xy spatial impl (high level fields and queries).
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseXYPointTestCase`], whose required methods are the abstract methods
//! a subclass implements.
//!
//! **Adaptations** (declared, each confined to the member named):
//! * `String field` of the abstract query factories is `&str`: Rust cannot
//!   pass `null`, so `testBoxNull`, `testDistanceNull` and
//!   `testPolygonNullField` are assumption failures (see each).
//! * `XYGeometry` and `XYGeometry[]` are the port's [`XYGeometryValue`] and
//!   `Vec<XYGeometryValue>`; `XYPolygon...` is `Vec<XYPolygon>`.
//! * The anonymous `FilterCodec`/`PointsFormat` of `doRandomDistanceTest` is
//!   [`leaf_size_codec`].

// The float literals are Java's (`18.313694f`, `32.94823588839368f`, ...),
// kept digit for digit; the compiler rounds them to the same `float`.
#![allow(clippy::excessive_precision)]

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::document::spatial_query::XYGeometryValue;
use rucene::document::{
    Document, NumericDocValuesField, NumericValue, Store, StoredField, StringField,
};
use rucene::error::Result;
use rucene::geo::{XYPoint, XYPolygon, XYRectangle};
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
use rucene::search::TopDocs;
use rucene::util::java_fmt::{double_to_string, float_to_string};
use rucene::util::{Bits, BytesRef, FixedBitSet};

use super::base_geo_point_test_case::{decimal_format_grouped_2, index_order, leaf_size_codec};
use super::shape_test_util;
use crate::analysis::MockAnalyzer;
use crate::index::RandomIndexWriter;
use crate::jdk::bit_set::JavaBitSet;
use crate::search::FixedBitSetCollector;
use crate::util::java_exceptions::{expect_throws, rethrow_java};
use crate::util::lucene_test_case::{Nightly, TestDirectory, VERBOSE};
use crate::util::{LuceneTestCase, TestUtil};

/// `FIELD_NAME` (`protected static final String`).
pub const FIELD_NAME: &str = "point";

/// The declaration of `org.apache.lucene.tests.geo.BaseXYPointTestCase`
/// (`public abstract class BaseXYPointTestCase extends LuceneTestCase`) with
/// its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        fn t(name: &str, body: fn(&dyn BaseXYPointTestCase) -> Result<()>) -> Method {
            xy_point_test(name, body)
        }
        Class::builder("org.apache.lucene.tests.geo.BaseXYPointTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(t("testIndexExtremeValues", |c| {
                c.test_index_extreme_values()
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
            .method(t("testAllYEqual", |c| c.test_all_y_equal()))
            .method(t("testAllXEqual", |c| c.test_all_x_equal()))
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
            .method(t("testSmallSetRect2", |c| c.test_small_set_rect2()))
            .method(t("testSmallSetMultiValued", |c| {
                c.test_small_set_multi_valued()
            }))
            .method(t("testSmallSetWholeSpace", |c| {
                c.test_small_set_whole_space()
            }))
            .method(t("testSmallSetPoly", |c| c.test_small_set_poly()))
            .method(t("testSmallSetPolyWholeSpace", |c| {
                c.test_small_set_poly_whole_space()
            }))
            .method(t("testSmallSetDistance", |c| c.test_small_set_distance()))
            .method(t("testSmallSetTinyDistance", |c| {
                c.test_small_set_tiny_distance()
            }))
            .method(t("testSmallSetHugeDistance", |c| {
                c.test_small_set_huge_distance()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test method of a `BaseXYPointTestCase` subclass.
pub fn xy_point_test(
    name: &str,
    body: impl Fn(&dyn BaseXYPointTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut XYPointTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// The instance of a `BaseXYPointTestCase` subclass.
pub struct XYPointTestInstance {
    /// The subclass.
    pub case: Arc<dyn BaseXYPointTestCase>,
}

impl XYPointTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseXYPointTestCase>(case: T) -> Self {
        XYPointTestInstance {
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

/// `geometry.toString()` of an `XYGeometry`.
pub fn geometry_to_string(g: &XYGeometryValue) -> String {
    match g {
        XYGeometryValue::Point(p) => p.to_string(),
        XYGeometryValue::Rectangle(r) => r.to_string(),
        XYGeometryValue::Line(l) => l.to_string(),
        XYGeometryValue::Polygon(p) => p.to_string(),
        XYGeometryValue::Circle(c) => c.to_string(),
    }
}

/// `Arrays.toString(geometries)`.
fn geometries_to_string(geometries: &[XYGeometryValue]) -> String {
    let parts: Vec<String> = geometries.iter().map(geometry_to_string).collect();
    format!("[{}]", parts.join(", "))
}

/// `new XYPolygon(xs, ys, holes...)`.
fn polygon(xs: &[f32], ys: &[f32], holes: Vec<XYPolygon>) -> Result<XYPolygon> {
    XYPolygon::new(xs.to_vec(), ys.to_vec(), holes)
}

/// `IOUtils.close(r, dir)`: closes both, then throws the first failure.
fn close_all(r: Arc<dyn IndexReader>, dir: &mut TestDirectory) -> Result<()> {
    let first = r.close();
    drop(r);
    let second = dir.close();
    first.and(second)
}

/// The `floatValue()` of a stored field's `numericValue()`.
fn float_value(doc: &Document, name: &str) -> f32 {
    match doc.get_field(name).and_then(|f| f.numeric_value()) {
        Some(NumericValue::Int(v)) => v as f32,
        Some(NumericValue::Long(v)) => v as f32,
        Some(NumericValue::Float(v)) => v,
        Some(NumericValue::Double(v)) => v as f32,
        _ => randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.NullPointerException",
        ),
    }
}

/// `rectContainsPoint(XYRectangle rect, double x, double y)` (`static`).
pub fn rect_contains_point(rect: &XYRectangle, x: f64, y: f64) -> bool {
    if y < f64::from(rect.min_y()) || y > f64::from(rect.max_y()) {
        return false;
    }
    x >= f64::from(rect.min_x()) && x <= f64::from(rect.max_x())
}

/// `cartesianDistance(double x1, double y1, double x2, double y2)`
/// (`static`).
pub fn cartesian_distance(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let diff_x = x1 - x2;
    let diff_y = y1 - y2;
    (diff_x * diff_x + diff_y * diff_y).sqrt()
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

/// Port of the members of `BaseXYPointTestCase`.
pub trait BaseXYPointTestCase: Send + Sync + 'static {
    // TODO: remove these hooks once all subclasses can pass with new random!

    /// `nextX()` (`protected`).
    fn next_x(&self) -> f32 {
        shape_test_util::next_float(&*random())
    }

    /// `nextY()` (`protected`).
    fn next_y(&self) -> f32 {
        shape_test_util::next_float(&*random())
    }

    /// `nextBox()` (`protected`).
    fn next_box(&self) -> XYRectangle {
        shape_test_util::next_box(&*random())
    }

    /// `nextPolygon()` (`protected`).
    fn next_polygon(&self) -> XYPolygon {
        shape_test_util::next_polygon()
    }

    /// `nextGeometry()` (`protected`).
    ///
    /// # Errors
    ///
    /// What `new XYPoint(x, y)` throws.
    fn next_geometry(&self) -> Result<Vec<XYGeometryValue>> {
        let len = random().next_int_bound(4) + 1;
        let mut geometries = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let geometry = match random().next_int_bound(3) {
                0 => {
                    let x = self.next_x();
                    let y = self.next_y();
                    XYGeometryValue::Point(XYPoint::new(x, y)?)
                }
                1 => XYGeometryValue::Rectangle(self.next_box()),
                _ => XYGeometryValue::Polygon(self.next_polygon()),
            };
            geometries.push(geometry);
        }
        Ok(geometries)
    }

    /// `addPointToDoc(String field, Document doc, float x, float y)`
    /// (`protected abstract`).
    fn add_point_to_doc(&self, field: &str, doc: &mut Document, x: f32, y: f32) -> Result<()>;

    /// `newRectQuery(String field, float minX, float maxX, float minY, float
    /// maxY)` (`protected abstract`).
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>` in `newRectQuery`, `newDistanceQuery` and
    /// `newPolygonQuery`; `None` is Java's `null`.
    fn new_rect_query(
        &self,
        field: Option<&str>,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> Result<Arc<dyn Query>>;

    /// `newDistanceQuery(String field, float centerX, float centerY, float
    /// radius)` (`protected abstract`).
    fn new_distance_query(
        &self,
        field: Option<&str>,
        center_x: f32,
        center_y: f32,
        radius: f32,
    ) -> Result<Arc<dyn Query>>;

    /// `newPolygonQuery(String field, XYPolygon... polygon)` (`protected
    /// abstract`).
    fn new_polygon_query(
        &self,
        field: Option<&str>,
        polygon: Vec<XYPolygon>,
    ) -> Result<Arc<dyn Query>>;

    /// `newGeometryQuery(String field, XYGeometry... geometries)`
    /// (`protected abstract`).
    fn new_geometry_query(
        &self,
        field: &str,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<Arc<dyn Query>>;

    /// `testIndexExtremeValues()`: valid values that should not cause
    /// exception.
    fn test_index_extreme_values(&self) -> Result<()> {
        let mut document = Document::new();
        self.add_point_to_doc("foo", &mut document, f32::MAX, f32::MAX)?;
        self.add_point_to_doc("foo", &mut document, f32::MAX, -f32::MAX)?;
        self.add_point_to_doc("foo", &mut document, -f32::MAX, f32::MAX)?;
        self.add_point_to_doc("foo", &mut document, -f32::MAX, -f32::MAX)?;
        Ok(())
    }

    /// `testIndexNaNValues()`: NaN: illegal.
    fn test_index_nan_values(&self) -> Result<()> {
        let mut document = Document::new();

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, f32::NAN, 50.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid value"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 50.0, f32::NAN)
        });
        Assert::assert_true(message_contains(&expected, "invalid value"));
        Ok(())
    }

    /// `testIndexInfValues()`: Inf: illegal.
    fn test_index_inf_values(&self) -> Result<()> {
        let mut document = Document::new();

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, f32::INFINITY, 0.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid value"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, f32::NEG_INFINITY, 0.0)
        });
        Assert::assert_true(message_contains(&expected, "invalid value"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 0.0, f32::INFINITY)
        });
        Assert::assert_true(message_contains(&expected, "invalid value"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.add_point_to_doc("foo", &mut document, 0.0, f32::NEG_INFINITY)
        });
        Assert::assert_true(message_contains(&expected, "invalid value"));
        Ok(())
    }

    /// `testBoxBasics()`: add a single point and search for it in a box.
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

    /// `testBoxNull()`: null field name not allowed (`BaseXYPointTestCase.java:206-214`).
    fn test_box_null(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_rect_query(None, 18.0, 19.0, -66.0, -65.0)
        });
        Assert::assert_true(message_contains(&expected, "field must not be null"));
        Ok(())
    }

    /// `testBoxInvalidCoordinates()`: box should not accept invalid x/y.
    fn test_box_invalid_coordinates(&self) -> Result<()> {
        expect_throws("java.lang.Exception", || {
            self.new_rect_query(Some("field"), f32::NAN, f32::NAN, f32::NAN, f32::NAN)
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
            searcher.count(self.new_distance_query(Some("field"), 18.0, -65.0, 20.0)?)?,
        );

        drop(searcher);
        reader.close()?;
        drop(reader);
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testDistanceNull()`: null field name not allowed (`BaseXYPointTestCase.java:246-254`).
    fn test_distance_null(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(None, 18.0, -65.0, 50_000.0)
        });
        Assert::assert_true(message_contains(&expected, "field must not be null"));
        Ok(())
    }

    /// `testDistanceIllegal()`: distance query should not accept invalid x/y
    /// as origin.
    fn test_distance_illegal(&self) -> Result<()> {
        expect_throws("java.lang.Exception", || {
            self.new_distance_query(Some("field"), f32::NAN, f32::NAN, 120000.0)
        });
        Ok(())
    }

    /// `testDistanceNegative()`: negative distance queries are not allowed.
    fn test_distance_negative(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, -1.0)
        });
        Assert::assert_true(message_contains(&expected, "radius"));
        Ok(())
    }

    /// `testDistanceNaN()`: NaN distance queries are not allowed.
    fn test_distance_nan(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, f32::NAN)
        });
        Assert::assert_true(message_contains(&expected, "radius"));
        Assert::assert_true(message_contains(&expected, "NaN"));
        Ok(())
    }

    /// `testDistanceInf()`: Inf distance queries are not allowed.
    fn test_distance_inf(&self) -> Result<()> {
        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, f32::INFINITY)
        });
        Assert::assert_true_msg(&message(&expected), message_contains(&expected, "radius"));
        Assert::assert_true_msg(&message(&expected), message_contains(&expected, "finite"));

        let expected = expect_throws("java.lang.IllegalArgumentException", || {
            self.new_distance_query(Some("field"), 18.0, 19.0, f32::NEG_INFINITY)
        });
        Assert::assert_true_msg(&message(&expected), message_contains(&expected, "radius"));
        Assert::assert_true_msg(
            &message(&expected),
            message_contains(&expected, "bigger than 0"),
        );
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

    /// `testPolygonNullField()`: null field name not allowed (`BaseXYPointTestCase.java:416-427`).
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
        let the_x = self.next_x();
        let the_y = self.next_y();

        let xs = vec![the_x; num_points];
        let ys = vec![the_y; num_points];

        self.verify(&xs, &ys)
    }

    /// `testLowCardinality()`: a particularly tricky adversary for BKD tree.
    fn test_low_cardinality(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        let cardinality = TestUtil::next_int(&*random(), 2, 20);

        let mut diff_xs = vec![0f32; cardinality as usize];
        let mut diff_ys = vec![0f32; cardinality as usize];
        for i in 0..cardinality as usize {
            diff_xs[i] = self.next_x();
            diff_ys[i] = self.next_y();
        }

        let mut xs = vec![0f32; num_points];
        let mut ys = vec![0f32; num_points];
        for i in 0..num_points {
            let index = random().next_int_bound(cardinality) as usize;
            xs[i] = diff_xs[index];
            ys[i] = diff_ys[index];
        }
        self.verify(&xs, &ys)
    }

    /// `testAllYEqual()`.
    fn test_all_y_equal(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        let y = self.next_y();
        let mut xs = vec![0f32; num_points];
        let mut ys = vec![0f32; num_points];

        let mut have_real_doc = false;

        for doc_id in 0..num_points {
            let x = random().next_int_bound(20);
            if x == 17 {
                // Some docs don't have a point:
                ys[doc_id] = f32::NAN;
                if *VERBOSE {
                    println!("  doc={doc_id} is missing");
                }
                continue;
            }

            if doc_id > 0 && x == 14 && have_real_doc {
                let old_doc_id = loop {
                    let old_doc_id = random().next_int_bound(doc_id as i32) as usize;
                    if !ys[old_doc_id].is_nan() {
                        break old_doc_id;
                    }
                };

                // Fully identical point:
                ys[doc_id] = xs[old_doc_id];
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} y={} x={} (same x/y as doc={old_doc_id})",
                        float_to_string(y),
                        float_to_string(xs[doc_id])
                    );
                }
            } else {
                xs[doc_id] = self.next_x();
                have_real_doc = true;
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} y={} x={}",
                        float_to_string(y),
                        float_to_string(xs[doc_id])
                    );
                }
            }
            ys[doc_id] = y;
        }

        self.verify(&xs, &ys)
    }

    /// `testAllXEqual()`.
    fn test_all_x_equal(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        let the_x = self.next_x();
        let mut xs = vec![0f32; num_points];
        let mut ys = vec![0f32; num_points];

        let mut have_real_doc = false;

        for doc_id in 0..num_points {
            let x = random().next_int_bound(20);
            if x == 17 {
                // Some docs don't have a point:
                ys[doc_id] = f32::NAN;
                if *VERBOSE {
                    println!("  doc={doc_id} is missing");
                }
                continue;
            }

            if doc_id > 0 && x == 14 && have_real_doc {
                let old_doc_id = loop {
                    let old_doc_id = random().next_int_bound(doc_id as i32) as usize;
                    if !ys[old_doc_id].is_nan() {
                        break old_doc_id;
                    }
                };

                // Fully identical point:
                ys[doc_id] = ys[old_doc_id];
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} y={} x={} (same X/y as doc={old_doc_id})",
                        float_to_string(ys[doc_id]),
                        float_to_string(the_x)
                    );
                }
            } else {
                ys[doc_id] = self.next_y();
                have_real_doc = true;
                if *VERBOSE {
                    println!(
                        "  doc={doc_id} y={} x={}",
                        float_to_string(ys[doc_id]),
                        float_to_string(the_x)
                    );
                }
            }
            xs[doc_id] = the_x;
        }

        self.verify(&xs, &ys)
    }

    /// `testMultiValued()`.
    fn test_multi_valued(&self) -> Result<()> {
        let num_points = LuceneTestCase::at_least(1000) as usize;
        // Every doc has 2 points:
        let mut xs = vec![0f32; 2 * num_points];
        let mut ys = vec![0f32; 2 * num_points];
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = LuceneTestCase::new_index_writer_config();

        // We rely on docID order:
        iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        // and on seeds being able to reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

        for id in 0..num_points {
            let mut doc = Document::new();
            xs[2 * id] = self.next_x();
            ys[2 * id] = self.next_y();
            doc.add(Box::new(StringField::new(
                "id",
                id.to_string(),
                Store::YES,
            )?));
            self.add_point_to_doc(FIELD_NAME, &mut doc, xs[2 * id], ys[2 * id])?;
            xs[2 * id + 1] = self.next_x();
            ys[2 * id + 1] = self.next_y();
            self.add_point_to_doc(FIELD_NAME, &mut doc, xs[2 * id + 1], ys[2 * id + 1])?;

            if *VERBOSE {
                println!("id={id}");
                println!(
                    "  x={} y={}",
                    float_to_string(xs[2 * id]),
                    float_to_string(ys[2 * id])
                );
                println!(
                    "  x={} y={}",
                    float_to_string(xs[2 * id + 1]),
                    float_to_string(ys[2 * id + 1])
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
                rect.min_x(),
                rect.max_x(),
                rect.min_y(),
                rect.max_y(),
            )?;

            let hits = search_index(&s, query, r.max_doc())?;

            let mut fail = false;

            let stored_fields = s.stored_fields()?;
            for doc_id in 0..ys.len() / 2 {
                let y_doc1 = ys[2 * doc_id];
                let x_doc1 = xs[2 * doc_id];
                let y_doc2 = ys[2 * doc_id + 1];
                let x_doc2 = xs[2 * doc_id + 1];

                let result1 = rect_contains_point(&rect, f64::from(x_doc1), f64::from(y_doc1));
                let result2 = rect_contains_point(&rect, f64::from(x_doc2), f64::from(y_doc2));

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
                        "  x={} y={}\n  x={} y={}",
                        float_to_string(x_doc1),
                        float_to_string(y_doc1),
                        float_to_string(x_doc2),
                        float_to_string(y_doc2)
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

        let mut xs = vec![0f32; num_points];
        let mut ys = vec![0f32; num_points];

        let mut have_real_doc = false;

        for id in 0..num_points {
            let x = random().next_int_bound(20);
            if x == 17 {
                // Some docs don't have a point:
                ys[id] = f32::NAN;
                if *VERBOSE {
                    println!("  id={id} is missing");
                }
                continue;
            }

            if id > 0 && x < 3 && have_real_doc {
                let old_id = loop {
                    let old_id = random().next_int_bound(id as i32) as usize;
                    if !ys[old_id].is_nan() {
                        break old_id;
                    }
                };

                if x == 0 {
                    // Identical x to old point
                    ys[id] = ys[old_id];
                    xs[id] = self.next_x();
                    if *VERBOSE {
                        println!(
                            "  id={id} x={} y={} (same x as doc={old_id})",
                            float_to_string(xs[id]),
                            float_to_string(ys[id])
                        );
                    }
                } else if x == 1 {
                    // Identical y to old point
                    ys[id] = self.next_y();
                    xs[id] = xs[old_id];
                    if *VERBOSE {
                        println!(
                            "  id={id} x={} y={} (same y as doc={old_id})",
                            float_to_string(xs[id]),
                            float_to_string(ys[id])
                        );
                    }
                } else {
                    assert_eq!(x, 2);
                    // Fully identical point:
                    xs[id] = xs[old_id];
                    ys[id] = ys[old_id];
                    if *VERBOSE {
                        println!(
                            "  id={id} x={} y={} (same X/y as doc={old_id})",
                            float_to_string(xs[id]),
                            float_to_string(ys[id])
                        );
                    }
                }
            } else {
                xs[id] = self.next_x();
                ys[id] = self.next_y();
                have_real_doc = true;
                if *VERBOSE {
                    println!(
                        "  id={id} x={} y={}",
                        float_to_string(xs[id]),
                        float_to_string(ys[id])
                    );
                }
            }
        }

        self.verify(&xs, &ys)
    }

    /// `verify(float[] xs, float[] ys)` (`private`).
    fn verify(&self, xs: &[f32], ys: &[f32]) -> Result<()> {
        // NaN means missing for the doc!!!!!
        self.verify_random_rectangles(xs, ys)?;
        self.verify_random_distances(xs, ys)?;
        self.verify_random_polygons(xs, ys)?;
        self.verify_random_geometries(xs, ys)
    }

    /// The index setup the four `verifyRandom…` methods share: the
    /// `IndexWriterConfig`, the directory, the indexed points, the reader.
    #[doc(hidden)]
    fn open_verify_index(
        &self,
        xs: &[f32],
        ys: &[f32],
    ) -> Result<(TestDirectory, Arc<dyn DirectoryReader>)> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        // Else seeds may not reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        // Else we can get O(N^2) merging:
        let mbd = iwc.max_buffered_docs();
        if mbd != -1 && mbd < (xs.len() / 100) as i32 {
            iwc.set_max_buffered_docs((xs.len() / 100) as i32)?;
        }
        let dir = if xs.len() > 100000 {
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
        self.index_points(xs, ys, &mut deleted, &w)?;
        let r = directory_reader::open_with_writer(&*w)?;
        w.close()?;
        drop(w);
        Ok((dir, r))
    }

    /// `verifyRandomRectangles(float[] xs, float[] ys)` (`protected`).
    fn verify_random_rectangles(&self, xs: &[f32], ys: &[f32]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(xs, ys)?;
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
                rect.min_x(),
                rect.max_x(),
                rect.min_y(),
                rect.max_y(),
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
                } else if xs[id].is_nan() || ys[id].is_nan() {
                    false
                } else {
                    rect_contains_point(&rect, f64::from(xs[id]), f64::from(ys[id]))
                };

                if hits.get(doc_id as usize) != expected {
                    build_error(doc_id, expected, id, xs, ys, &*query, &live_docs, |b| {
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

    /// `verifyRandomDistances(float[] xs, float[] ys)` (`protected`).
    fn verify_random_distances(&self, xs: &[f32], ys: &[f32]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(xs, ys)?;
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
            let center_x = self.next_x();
            let center_y = self.next_y();

            let mut radius = 0f32;
            while radius == 0f32 {
                // So the query can cover at most 50% of the cartesian space:
                radius = random().next_float() * f32::MAX / 2.0;
            }

            if *VERBOSE {
                println!("  radius = {}", decimal_format_grouped_2(f64::from(radius)));
            }

            let query = self.new_distance_query(Some(FIELD_NAME), center_x, center_y, radius)?;

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
                } else if xs[id].is_nan() || ys[id].is_nan() {
                    false
                } else {
                    cartesian_distance(
                        f64::from(center_x),
                        f64::from(center_y),
                        f64::from(xs[id]),
                        f64::from(ys[id]),
                    ) <= f64::from(radius)
                };

                if hits.get(doc_id as usize) != expected {
                    let final_radius = radius;
                    let explain = |b: &mut String| {
                        if !f64::from(xs[id]).is_nan() {
                            let distance = cartesian_distance(
                                f64::from(center_x),
                                f64::from(center_y),
                                f64::from(xs[id]),
                                f64::from(ys[id]),
                            );
                            b.push_str(&format!(
                                "  centerX={} centerY={} distance={} vs radius={}",
                                float_to_string(center_x),
                                float_to_string(center_y),
                                double_to_string(distance),
                                float_to_string(final_radius)
                            ));
                        }
                    };
                    build_error(doc_id, expected, id, xs, ys, &*query, &live_docs, explain);
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

    /// `verifyRandomPolygons(float[] xs, float[] ys)` (`protected`).
    fn verify_random_polygons(&self, xs: &[f32], ys: &[f32]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(xs, ys)?;
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
                } else if xs[id].is_nan() || ys[id].is_nan() {
                    false
                } else {
                    shape_test_util::contains_slowly(
                        &polygon,
                        f64::from(xs[id]),
                        f64::from(ys[id]),
                    )?
                };

                if hits.get(doc_id as usize) != expected {
                    build_error(doc_id, expected, id, xs, ys, &*query, &live_docs, |b| {
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

    /// `verifyRandomGeometries(float[] xs, float[] ys)` (`protected`).
    fn verify_random_geometries(&self, xs: &[f32], ys: &[f32]) -> Result<()> {
        let (mut dir, dr) = self.open_verify_index(xs, ys)?;
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

            // geometries
            let geometries = self.next_geometry()?;
            let query = self.new_geometry_query(FIELD_NAME, geometries.clone())?;
            let component2d = XYGeometryValue::create(&geometries)?;

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
                } else if xs[id].is_nan() || ys[id].is_nan() {
                    false
                } else {
                    component2d.contains(f64::from(xs[id]), f64::from(ys[id]))
                };

                if hits.get(doc_id as usize) != expected {
                    build_error(doc_id, expected, id, xs, ys, &*query, &live_docs, |b| {
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

    /// `indexPoints(float[] xs, float[] ys, Set<Integer> deleted,
    /// IndexWriter w)` (`private`).
    fn index_points(
        &self,
        xs: &[f32],
        ys: &[f32],
        deleted: &mut HashSet<i32>,
        w: &IndexWriter,
    ) -> Result<()> {
        for id in 0..xs.len() {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
            doc.add(Box::new(NumericDocValuesField::new("id", id as i64)));
            if !xs[id].is_nan() && !ys[id].is_nan() {
                self.add_point_to_doc(FIELD_NAME, &mut doc, xs[id], ys[id])?;
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
        let rect = shape_test_util::next_box(&*random());
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = LuceneTestCase::new_index_writer_config();
        // Else seeds may not reproduce:
        iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
        for i in 0..3 {
            let y = if i == 0 {
                rect.min_y()
            } else if i == 1 {
                ((f64::from(rect.min_y()) + f64::from(rect.max_y())) / 2.0) as f32
            } else {
                rect.max_y()
            };
            for j in 0..3 {
                let x = if j == 0 {
                    rect.min_x()
                } else if j == 1 {
                    if i == 1 {
                        continue;
                    }
                    ((f64::from(rect.min_x()) + f64::from(rect.max_x())) / 2.0) as f32
                } else {
                    rect.max_x()
                };

                let mut doc = Document::new();
                self.add_point_to_doc(FIELD_NAME, &mut doc, x, y)?;
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
                rect.min_x(),
                rect.max_x(),
                rect.min_y(),
                rect.max_y(),
            )?)?,
        );
        // expand 1 ulp in each direction if possible and test a slightly larger box!
        if rect.min_x() != -f32::MAX {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_x().next_down(),
                    rect.max_x(),
                    rect.min_y(),
                    rect.max_y(),
                )?)?,
            );
        }
        if rect.max_x() != f32::MAX {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_x(),
                    rect.max_x().next_up(),
                    rect.min_y(),
                    rect.max_y(),
                )?)?,
            );
        }
        if rect.min_y() != -f32::MAX {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_x(),
                    rect.max_x(),
                    rect.min_y().next_down(),
                    rect.max_y(),
                )?)?,
            );
        }
        if rect.max_y() != f32::MAX {
            Assert::assert_equals(
                8,
                s.count(self.new_rect_query(
                    Some(field),
                    rect.min_x(),
                    rect.max_x(),
                    rect.min_y(),
                    rect.max_y().next_up(),
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
            let x = self.next_x();
            let y = self.next_y();
            // pre-normalize up front, so we can just use quantized value for testing and do simple
            // exact comparisons

            let mut doc = Document::new();
            self.add_point_to_doc("field", &mut doc, x, y)?;
            doc.add(Box::new(StoredField::new_number(
                "x",
                NumericValue::Float(x),
            )?));
            doc.add(Box::new(StoredField::new_number(
                "y",
                NumericValue::Float(y),
            )?));
            writer.add_document(&doc)?;
        }
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let stored_fields = reader.stored_fields()?;
        for _ in 0..num_queries {
            let circle = shape_test_util::next_circle();
            let x = circle.get_x();
            let y = circle.get_y();
            let radius = circle.get_radius();

            let mut expected = JavaBitSet::new();
            for doc in 0..reader.max_doc() {
                let stored = stored_fields.document(doc)?;
                let doc_x = float_value(&stored, "x");
                let doc_y = float_value(&stored, "y");
                let distance = cartesian_distance(
                    f64::from(x),
                    f64::from(y),
                    f64::from(doc_x),
                    f64::from(doc_y),
                );
                if distance <= f64::from(radius) {
                    expected.set(doc as usize);
                }
            }

            let top_docs = searcher.search_with_sort(
                self.new_distance_query(Some("field"), x, y, radius)?,
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
                    float_to_string(x),
                    float_to_string(y),
                    float_to_string(radius)
                );
                for doc in 0..reader.max_doc() {
                    let stored = stored_fields.document(doc)?;
                    let doc_x = float_value(&stored, "x");
                    let doc_y = float_value(&stored, "y");
                    let distance = cartesian_distance(
                        f64::from(x),
                        f64::from(y),
                        f64::from(doc_x),
                        f64::from(doc_y),
                    );
                    println!(
                        "{doc}: ({},{}), distance={}",
                        float_to_string(x),
                        float_to_string(y),
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
            rect.min_x(),
            rect.max_x(),
            rect.min_y(),
            rect.max_y(),
        )?;
        let q2 = self.new_rect_query(
            Some("field"),
            rect.min_x(),
            rect.max_x(),
            rect.min_y(),
            rect.max_y(),
        )?;
        Assert::assert_equals(QueryObject(q1), QueryObject(q2));

        let x = self.next_x();
        let y = self.next_y();
        let q1 = self.new_distance_query(Some("field"), x, y, 10000.0)?;
        let q2 = self.new_distance_query(Some("field"), x, y, 10000.0)?;
        Assert::assert_equals(QueryObject(Arc::clone(&q1)), QueryObject(q2));
        Assert::assert_false(q1.query_eq(&*self.new_distance_query(
            Some("field2"),
            x,
            y,
            10000.0,
        )?));

        let xs = [
            rect.min_x(),
            rect.max_x(),
            rect.max_x(),
            rect.min_x(),
            rect.min_x(),
        ];
        let ys = [
            rect.min_y(),
            rect.min_y(),
            rect.max_y(),
            rect.max_y(),
            rect.min_y(),
        ];
        let q1 = self.new_polygon_query(Some("field"), vec![polygon(&xs, &ys, Vec::new())?])?;
        let q2 = self.new_polygon_query(Some("field"), vec![polygon(&xs, &ys, Vec::new())?])?;
        Assert::assert_equals(QueryObject(Arc::clone(&q1)), QueryObject(q2));
        Assert::assert_false(q1.query_eq(
            &*self.new_polygon_query(Some("field2"), vec![polygon(&xs, &ys, Vec::new())?])?,
        ));
        Ok(())
    }

    /// `searchSmallSet(Query query, int size)` (`private`): return topdocs
    /// over a small set of points in field "point".
    fn search_small_set(&self, query: Arc<dyn Query>, size: i32) -> Result<TopDocs> {
        // this is a simple systematic test, indexing these points
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
            self.add_point_to_doc("point", &mut doc, p[0] as f32, p[1] as f32)?;
            writer.add_document(&doc)?;
        }

        // add explicit multi-valued docs
        for i in (0..pts.len()).step_by(2) {
            let mut doc = Document::new();
            self.add_point_to_doc("point", &mut doc, pts[i][0] as f32, pts[i][1] as f32)?;
            self.add_point_to_doc(
                "point",
                &mut doc,
                pts[i + 1][0] as f32,
                pts[i + 1][1] as f32,
            )?;
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

    /// `testSmallSetRect2()`.
    fn test_small_set_rect2(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_rect_query(Some("point"), -45.0, -44.0, -180.0, 180.0)?,
            20,
        )?;
        Assert::assert_equals(2, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetMultiValued()`.
    fn test_small_set_multi_valued(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_rect_query(Some("point"), 32.755, 32.776, -180.0, 180.770)?,
            20,
        )?;
        // 3 single valued docs + 2 multi-valued docs
        Assert::assert_equals(5, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetWholeSpace()`.
    fn test_small_set_whole_space(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_rect_query(Some("point"), -f32::MAX, f32::MAX, -f32::MAX, f32::MAX)?,
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

    /// `testSmallSetPolyWholeSpace()`.
    fn test_small_set_poly_whole_space(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_polygon_query(
                Some("point"),
                vec![polygon(
                    &[-f32::MAX, f32::MAX, f32::MAX, -f32::MAX, -f32::MAX],
                    &[-f32::MAX, -f32::MAX, f32::MAX, f32::MAX, -f32::MAX],
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
            self.new_distance_query(Some("point"), 32.94823588839368, -96.4538113027811, 6.0)?,
            20,
        )?;
        Assert::assert_equals(11, td.total_hits.value());
        Ok(())
    }

    /// `testSmallSetTinyDistance()`.
    fn test_small_set_tiny_distance(&self) -> Result<()> {
        let td = self.search_small_set(
            self.new_distance_query(Some("point"), 40.720611, -73.998776, 0.1)?,
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
                f32::MAX,
            )?,
            20,
        )?;
        Assert::assert_equals(24, td.total_hits.value());
        Ok(())
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

/// `buildError(int docID, boolean expected, int id, float[] xs, float[] ys,
/// Query query, Bits liveDocs, Consumer<StringBuilder> explain)`
/// (`private`).
#[allow(clippy::too_many_arguments)]
fn build_error(
    doc_id: i32,
    expected: bool,
    id: usize,
    xs: &[f32],
    ys: &[f32],
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
        "  x={} y={}\n",
        float_to_string(xs[id]),
        float_to_string(ys[id])
    ));
    b.push_str(&format!("  deleted?={}", is_deleted(live_docs, doc_id)));
    explain(&mut b);
    Assert::fail(&format!("wrong hit (first of possibly more):\n\n{b}"));
}
