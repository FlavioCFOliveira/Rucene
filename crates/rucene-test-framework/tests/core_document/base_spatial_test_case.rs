//! Port of the spatial test bases of `lucene/core/src/test/org/apache/lucene/document/`:
//! `BaseSpatialTestCase`, `BaseLatLonSpatialTestCase`, `BaseLatLonShapeTestCase`,
//! `BaseLatLonPointTestCase`, `BaseLatLonDocValueTestCase`,
//! `BaseLatLonShapeDocValueTestCase`, `BaseXYShapeTestCase` and
//! `BaseXYShapeDocValueTestCase`.
//!
//! **Divergences from the Java test sources (structure only).**
//! - Java's six abstract classes form an inheritance chain. Rust has no
//!   implementation inheritance, so the chain becomes one trait,
//!   [`SpatialCase`]: its abstract methods are the methods every concrete class
//!   implements, and the intermediate classes' overrides are its default
//!   methods, selected by [`SpatialCase::family`] (which intermediate base the
//!   Java class extends). A concrete class overrides a default exactly where its
//!   Java class overrides the inherited method.
//! - Java passes shapes as `Object` and casts them. Here they are the closed
//!   list [`Shape`] (the declared list of the classes Java casts to); a failed
//!   cast throws Java's `ClassCastException`.
//! - Java's `Field[]` of `createIndexableFields` becomes a vector of boxed
//!   [`IndexableField`]s; the validators, which only read the fields
//!   `LatLonShape`/`XYShape.createIndexableFields` build, take the
//!   [`Triangle`]s directly.

// Some Java members (`ShapeType.MIXED`, `Encoder.quantizeXCeil`, ...) are
// declared by the bases and used by no concrete class.
#![allow(dead_code, clippy::too_many_arguments)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::generators::{RandomNumbers, RandomPicks};
use randomizedtesting::jdk::exception::throw;
use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::Assert;
use rucene::document::geo_fields::{LatLonDocValuesField, LatLonPoint};
use rucene::document::shape_field::{QueryRelation, ShapeField, Triangle, TriangleType};
use rucene::document::spatial_query::{LatLonGeometryValue, XYGeometryValue};
use rucene::document::{
    Document, LatLonShape, NumericDocValuesField, ShapeDocValuesField, Store, StringField, XYShape,
};
use rucene::error::{LuceneError, Result};
use rucene::geo::component2d::{Component2D, WithinRelation};
use rucene::geo::encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
use rucene::geo::geometry::{
    Circle, Line, Point, Polygon, Rectangle, XYCircle, XYLine, XYPoint, XYPolygon, XYRectangle,
};
use rucene::geo::tessellator::Tessellator;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, IndexableField};
use rucene::search::{IndexSearcher, Query};
use rucene::util::{BytesRef, FixedBitSet};
use rucene_test_framework::geo::{geo_test_util, shape_test_util};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::{FixedBitSetCollector, QueryUtils};
use rucene_test_framework::util::lucene_test_case::{Nightly, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, random, test_closure, unchecked, verbose};

/// `FIELD_NAME`: name of the `LatLonShape` indexed field.
pub const FIELD_NAME: &str = "shape";

/// `QueryRelation.values()`, in declaration order.
pub const QUERY_RELATIONS: [QueryRelation; 4] = [
    QueryRelation::Intersects,
    QueryRelation::Within,
    QueryRelation::Disjoint,
    QueryRelation::Contains,
];

/// `POINT_LINE_RELATIONS`.
pub const POINT_LINE_RELATIONS: [QueryRelation; 3] = [
    QueryRelation::Intersects,
    QueryRelation::Disjoint,
    QueryRelation::Contains,
];

/// The shape-doc-values bases' `getSupportedQueryRelations()`.
pub const DOC_VALUE_RELATIONS: [QueryRelation; 3] = [
    QueryRelation::Intersects,
    QueryRelation::Within,
    QueryRelation::Disjoint,
];

// -----------------------------------------------------------------------------
// Shapes
// -----------------------------------------------------------------------------

/// A shape the tests pass as `Object`: the declared list of the classes the
/// Java tests cast to.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// `org.apache.lucene.geo.Point`.
    Point(Point),
    /// `org.apache.lucene.geo.Line`.
    Line(Line),
    /// `org.apache.lucene.geo.Polygon`.
    Polygon(Polygon),
    /// `org.apache.lucene.geo.Circle`.
    Circle(Circle),
    /// `org.apache.lucene.geo.Rectangle`.
    Rectangle(Rectangle),
    /// `org.apache.lucene.geo.XYPoint`.
    XYPoint(XYPoint),
    /// `org.apache.lucene.geo.XYLine`.
    XYLine(XYLine),
    /// `org.apache.lucene.geo.XYPolygon`.
    XYPolygon(XYPolygon),
    /// `org.apache.lucene.geo.XYCircle`.
    XYCircle(XYCircle),
    /// `org.apache.lucene.geo.XYRectangle`.
    XYRectangle(XYRectangle),
    /// A `double[] {lat, lon}`.
    Doubles([f64; 2]),
    /// A `float[] {x, y}`.
    Floats([f32; 2]),
    /// An `Object[]` (`Point[]`, `Line[]`, `Polygon[]`, ... of one class).
    Array(Vec<Shape>),
}

impl fmt::Display for Shape {
    /// `toString()`; an array prints as `Arrays.toString`.
    ///
    /// Divergence: Java prints a `double[]`/`float[]` as its identity
    /// (`[D@1b6d3586`); there is no identity here, so the values are printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Shape::Point(s) => write!(f, "{s}"),
            Shape::Line(s) => write!(f, "{s}"),
            Shape::Polygon(s) => write!(f, "{s}"),
            Shape::Circle(s) => write!(f, "{s}"),
            Shape::Rectangle(s) => write!(f, "{s}"),
            Shape::XYPoint(s) => write!(f, "{s}"),
            Shape::XYLine(s) => write!(f, "{s:?}"),
            Shape::XYPolygon(s) => write!(f, "{s:?}"),
            Shape::XYCircle(s) => write!(f, "{s:?}"),
            Shape::XYRectangle(s) => write!(f, "{s}"),
            Shape::Doubles(d) => write!(f, "{d:?}"),
            Shape::Floats(d) => write!(f, "{d:?}"),
            Shape::Array(a) => {
                write!(f, "[")?;
                for (i, s) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{s}")?;
                }
                write!(f, "]")
            }
        }
    }
}

/// Throws Java's `ClassCastException` for a cast of `shape` to `class`.
fn class_cast(shape: &Shape, class: &str) -> ! {
    let from = match shape {
        Shape::Point(_) => "org.apache.lucene.geo.Point",
        Shape::Line(_) => "org.apache.lucene.geo.Line",
        Shape::Polygon(_) => "org.apache.lucene.geo.Polygon",
        Shape::Circle(_) => "org.apache.lucene.geo.Circle",
        Shape::Rectangle(_) => "org.apache.lucene.geo.Rectangle",
        Shape::XYPoint(_) => "org.apache.lucene.geo.XYPoint",
        Shape::XYLine(_) => "org.apache.lucene.geo.XYLine",
        Shape::XYPolygon(_) => "org.apache.lucene.geo.XYPolygon",
        Shape::XYCircle(_) => "org.apache.lucene.geo.XYCircle",
        Shape::XYRectangle(_) => "org.apache.lucene.geo.XYRectangle",
        Shape::Doubles(_) => "[D",
        Shape::Floats(_) => "[F",
        Shape::Array(_) => "[Ljava.lang.Object;",
    };
    throw(
        "java.lang.ClassCastException",
        format!("class {from} cannot be cast to class {class}"),
    )
}

macro_rules! shape_cast {
    ($fn:ident, $variant:ident, $ty:ty, $class:literal) => {
        /// The Java cast `(Class) shape`.
        pub fn $fn(&self) -> &$ty {
            match self {
                Shape::$variant(s) => s,
                other => class_cast(other, $class),
            }
        }
    };
}

impl Shape {
    shape_cast!(as_point, Point, Point, "org.apache.lucene.geo.Point");
    shape_cast!(as_line, Line, Line, "org.apache.lucene.geo.Line");
    shape_cast!(
        as_polygon,
        Polygon,
        Polygon,
        "org.apache.lucene.geo.Polygon"
    );
    shape_cast!(as_circle, Circle, Circle, "org.apache.lucene.geo.Circle");
    shape_cast!(
        as_rectangle,
        Rectangle,
        Rectangle,
        "org.apache.lucene.geo.Rectangle"
    );
    shape_cast!(
        as_xy_point,
        XYPoint,
        XYPoint,
        "org.apache.lucene.geo.XYPoint"
    );
    shape_cast!(as_xy_line, XYLine, XYLine, "org.apache.lucene.geo.XYLine");
    shape_cast!(
        as_xy_polygon,
        XYPolygon,
        XYPolygon,
        "org.apache.lucene.geo.XYPolygon"
    );
    shape_cast!(
        as_xy_circle,
        XYCircle,
        XYCircle,
        "org.apache.lucene.geo.XYCircle"
    );
    shape_cast!(
        as_xy_rectangle,
        XYRectangle,
        XYRectangle,
        "org.apache.lucene.geo.XYRectangle"
    );
    shape_cast!(as_doubles, Doubles, [f64; 2], "[D");
    shape_cast!(as_floats, Floats, [f32; 2], "[F");
    shape_cast!(as_array, Array, Vec<Shape>, "[Ljava.lang.Object;");

    /// `shape instanceof Object[]`.
    pub fn is_array(&self) -> bool {
        matches!(self, Shape::Array(_))
    }
}

/// `Arrays.toString(Object[])` over the shapes.
pub fn shapes_to_string(shapes: &[Shape]) -> String {
    Shape::Array(shapes.to_vec()).to_string()
}

// -----------------------------------------------------------------------------
// Encoder
// -----------------------------------------------------------------------------

/// `BaseSpatialTestCase.Encoder`: the anonymous subclasses of
/// `BaseLatLonSpatialTestCase.getEncoder()` and
/// `BaseXYShapeTestCase.getEncoder()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// `BaseLatLonSpatialTestCase`'s encoder.
    LatLon,
    /// `BaseXYShapeTestCase`'s encoder.
    XY,
}

impl Encoder {
    /// `decodeX(encoded)`.
    pub fn decode_x(self, encoded: i32) -> f64 {
        match self {
            Encoder::LatLon => GeoEncodingUtils::decode_longitude(encoded),
            Encoder::XY => f64::from(XYEncodingUtils::decode(encoded)),
        }
    }

    /// `decodeY(encoded)`.
    pub fn decode_y(self, encoded: i32) -> f64 {
        match self {
            Encoder::LatLon => GeoEncodingUtils::decode_latitude(encoded),
            Encoder::XY => f64::from(XYEncodingUtils::decode(encoded)),
        }
    }

    /// `quantizeX(raw)`.
    pub fn quantize_x(self, raw: f64) -> f64 {
        match self {
            Encoder::LatLon => GeoEncodingUtils::decode_longitude(unchecked(
                GeoEncodingUtils::encode_longitude(raw),
            )),
            Encoder::XY => f64::from(XYEncodingUtils::decode(unchecked(XYEncodingUtils::encode(
                raw as f32,
            )))),
        }
    }

    /// `quantizeXCeil(raw)`.
    pub fn quantize_x_ceil(self, raw: f64) -> f64 {
        match self {
            Encoder::LatLon => GeoEncodingUtils::decode_longitude(unchecked(
                GeoEncodingUtils::encode_longitude_ceil(raw),
            )),
            Encoder::XY => self.quantize_x(raw),
        }
    }

    /// `quantizeY(raw)`.
    pub fn quantize_y(self, raw: f64) -> f64 {
        match self {
            Encoder::LatLon => {
                GeoEncodingUtils::decode_latitude(unchecked(GeoEncodingUtils::encode_latitude(raw)))
            }
            Encoder::XY => self.quantize_x(raw),
        }
    }

    /// `quantizeYCeil(raw)`.
    pub fn quantize_y_ceil(self, raw: f64) -> f64 {
        match self {
            Encoder::LatLon => GeoEncodingUtils::decode_latitude(unchecked(
                GeoEncodingUtils::encode_latitude_ceil(raw),
            )),
            Encoder::XY => self.quantize_x(raw),
        }
    }
}

// -----------------------------------------------------------------------------
// Validator
// -----------------------------------------------------------------------------

/// The state of `BaseSpatialTestCase.Validator`: its `encoder` and its
/// `queryRelation` (initially `INTERSECTS`).
#[derive(Clone, Copy, Debug)]
pub struct ValidatorBase {
    /// `encoder`.
    pub encoder: Encoder,
    /// `queryRelation`.
    pub query_relation: QueryRelation,
}

impl ValidatorBase {
    /// `Validator(Encoder encoder)`.
    pub fn new(encoder: Encoder) -> Self {
        Self {
            encoder,
            query_relation: QueryRelation::Intersects,
        }
    }

    /// `testComponentQuery(Component2D query, Field[] fields)`.
    pub fn test_component_query_fields(
        &self,
        query: &dyn Component2D,
        fields: &[Triangle],
    ) -> Result<bool> {
        let encoder = self.encoder;
        for field in fields {
            let intersects;
            let contains;
            let decoded = ShapeField::decode_triangle(field_bytes(field))?;
            match decoded.triangle_type {
                TriangleType::Point => {
                    let y = encoder.decode_y(decoded.a_y);
                    let x = encoder.decode_x(decoded.a_x);
                    intersects = query.contains(x, y);
                    contains = intersects;
                }
                TriangleType::Line => {
                    let a_y = encoder.decode_y(decoded.a_y);
                    let a_x = encoder.decode_x(decoded.a_x);
                    let b_y = encoder.decode_y(decoded.b_y);
                    let b_x = encoder.decode_x(decoded.b_x);
                    intersects = query.intersects_line_bbox(a_x, a_y, b_x, b_y);
                    contains = query.contains_line_bbox(a_x, a_y, b_x, b_y);
                }
                TriangleType::Triangle => {
                    let a_y = encoder.decode_y(decoded.a_y);
                    let a_x = encoder.decode_x(decoded.a_x);
                    let b_y = encoder.decode_y(decoded.b_y);
                    let b_x = encoder.decode_x(decoded.b_x);
                    let c_y = encoder.decode_y(decoded.c_y);
                    let c_x = encoder.decode_x(decoded.c_x);
                    intersects = query.intersects_triangle_bbox(a_x, a_y, b_x, b_y, c_x, c_y);
                    contains = query.contains_triangle_bbox(a_x, a_y, b_x, b_y, c_x, c_y);
                }
            }
            Assert::assert_true(contains == intersects || (!contains && intersects));
            if self.query_relation == QueryRelation::Disjoint && intersects {
                return Ok(false);
            } else if self.query_relation == QueryRelation::Within && !contains {
                return Ok(false);
            } else if self.query_relation == QueryRelation::Intersects && intersects {
                return Ok(true);
            }
        }
        Ok(self.query_relation != QueryRelation::Intersects)
    }

    /// `testWithinQuery(Component2D query, Field[] fields)`.
    pub fn test_within_query(
        &self,
        query: &dyn Component2D,
        fields: &[Triangle],
    ) -> Result<WithinRelation> {
        let encoder = self.encoder;
        let mut answer = WithinRelation::Disjoint;
        for field in fields {
            let decoded = ShapeField::decode_triangle(field_bytes(field))?;
            let relation = match decoded.triangle_type {
                TriangleType::Point => {
                    let y = encoder.decode_y(decoded.a_y);
                    let x = encoder.decode_x(decoded.a_x);
                    query.within_point(x, y)
                }
                TriangleType::Line => {
                    let a_y = encoder.decode_y(decoded.a_y);
                    let a_x = encoder.decode_x(decoded.a_x);
                    let b_y = encoder.decode_y(decoded.b_y);
                    let b_x = encoder.decode_x(decoded.b_x);
                    query.within_line_bbox(a_x, a_y, decoded.ab, b_x, b_y)
                }
                TriangleType::Triangle => {
                    let a_y = encoder.decode_y(decoded.a_y);
                    let a_x = encoder.decode_x(decoded.a_x);
                    let b_y = encoder.decode_y(decoded.b_y);
                    let b_x = encoder.decode_x(decoded.b_x);
                    let c_y = encoder.decode_y(decoded.c_y);
                    let c_x = encoder.decode_x(decoded.c_x);
                    query.within_triangle_bbox(
                        a_x, a_y, decoded.ab, b_x, b_y, decoded.bc, c_x, c_y, decoded.ca,
                    )
                }
            };
            if relation == WithinRelation::NotWithin {
                return Ok(relation);
            } else if relation == WithinRelation::Candidate {
                answer = WithinRelation::Candidate;
            }
        }
        Ok(answer)
    }
}

/// `field.binaryValue().bytes` of a triangle field.
fn field_bytes(field: &Triangle) -> &[u8] {
    let bytes: &BytesRef = field.binary_value().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    bytes.bytes.as_slice()
}

/// `BaseSpatialTestCase.Validator`.
pub trait Validator {
    /// The inherited `encoder` and `queryRelation` fields.
    fn base(&self) -> &ValidatorBase;
    /// The inherited fields, mutably.
    fn base_mut(&mut self) -> &mut ValidatorBase;

    /// `setRelation(relation)` (returns `this` in Java).
    fn set_relation(&mut self, relation: QueryRelation) {
        self.base_mut().query_relation = relation;
    }

    /// `testComponentQuery(Component2D query, Object shape)`.
    fn test_component_query(&self, query: &dyn Component2D, shape: &Shape) -> Result<bool>;
}

/// Declares a validator whose whole state is [`ValidatorBase`].
macro_rules! simple_validator {
    ($(#[$m:meta])* $name:ident, |$this:ident, $query:ident, $shape:ident| $body:expr) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug)]
        pub struct $name(pub ValidatorBase);

        impl $name {
            /// The constructor `(Encoder encoder)`.
            pub fn new(encoder: Encoder) -> Self {
                Self(ValidatorBase::new(encoder))
            }
        }

        impl Validator for $name {
            fn base(&self) -> &ValidatorBase {
                &self.0
            }
            fn base_mut(&mut self) -> &mut ValidatorBase {
                &mut self.0
            }
            fn test_component_query(
                &self,
                $query: &dyn Component2D,
                $shape: &Shape,
            ) -> Result<bool> {
                let $this = &self.0;
                $body
            }
        }
    };
}

simple_validator!(
    /// `TestLatLonPointShapeQueries.PointValidator` (also the identical
    /// `PointValidator`s of `TestLatLonPointPointQueries` and
    /// `TestLatLonDocValuesPointPointQueries`).
    LatLonPointValidator,
    |base, query, shape| {
        let p = shape.as_point();
        let fields = LatLonShape::create_indexable_fields_point("dummy", p.lat(), p.lon())?;
        if base.query_relation == QueryRelation::Contains {
            return Ok(base.test_within_query(query, &fields)? == WithinRelation::Candidate);
        }
        base.test_component_query_fields(query, &fields)
    }
);

simple_validator!(
    /// `TestLatLonLineShapeQueries.LineValidator`.
    LatLonLineValidator,
    |base, query, shape| {
        let line = shape.as_line();
        let fields = LatLonShape::create_indexable_fields_line("dummy", line)?;
        if base.query_relation == QueryRelation::Contains {
            return Ok(base.test_within_query(query, &fields)? == WithinRelation::Candidate);
        }
        base.test_component_query_fields(query, &fields)
    }
);

simple_validator!(
    /// `TestLatLonPolygonShapeQueries.PolygonValidator`.
    LatLonPolygonValidator,
    |base, query, shape| {
        let polygon = shape.as_polygon();
        let fields = LatLonShape::create_indexable_fields_polygon("dummy", polygon)?;
        if base.query_relation == QueryRelation::Contains {
            return Ok(base.test_within_query(query, &fields)? == WithinRelation::Candidate);
        }
        base.test_component_query_fields(query, &fields)
    }
);

impl LatLonPolygonValidator {
    /// `testWithinPolygon(Component2D query, Polygon polygon)`.
    pub fn test_within_polygon(
        &self,
        query: &dyn Component2D,
        polygon: &Polygon,
    ) -> Result<WithinRelation> {
        self.0.test_within_query(
            query,
            &LatLonShape::create_indexable_fields_polygon("dummy", polygon)?,
        )
    }
}

simple_validator!(
    /// `TestXYPointShapeQueries.PointValidator`.
    XYPointValidator,
    |base, query, shape| {
        let point = shape.as_xy_point();
        let fields = XYShape::create_indexable_fields_point("dummy", point.get_x(), point.get_y())?;
        if base.query_relation == QueryRelation::Contains {
            return Ok(base.test_within_query(query, &fields)? == WithinRelation::Candidate);
        }
        base.test_component_query_fields(query, &fields)
    }
);

simple_validator!(
    /// `TestXYLineShapeQueries.LineValidator`.
    XYLineValidator,
    |base, query, shape| {
        let line = shape.as_xy_line();
        let fields = XYShape::create_indexable_fields_line("dummy", line)?;
        if base.query_relation == QueryRelation::Contains {
            return Ok(base.test_within_query(query, &fields)? == WithinRelation::Candidate);
        }
        base.test_component_query_fields(query, &fields)
    }
);

simple_validator!(
    /// `TestXYPolygonShapeQueries.PolygonValidator`.
    XYPolygonValidator,
    |base, query, shape| {
        let polygon = shape.as_xy_polygon();
        let fields = XYShape::create_indexable_fields_polygon("dummy", polygon)?;
        if base.query_relation == QueryRelation::Contains {
            return Ok(base.test_within_query(query, &fields)? == WithinRelation::Candidate);
        }
        base.test_component_query_fields(query, &fields)
    }
);

/// How a multi-shape validator answers `CONTAINS` before looping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiContains {
    /// `MultiPointValidator`/`MultiLineValidator`: no special case.
    Loop,
    /// `TestLatLonMultiPolygonShapeQueries.MultiPolygonValidator`:
    /// `testWithinPolygon` over `LatLonShape` polygons.
    LatLonPolygons,
    /// `TestXYMultiPolygonShapeQueries.MultiPolygonValidator`:
    /// `testWithinPolygon` over `XYShape` polygons.
    XYPolygons,
}

/// The multi-shape validators (`MultiPointValidator`, `MultiLineValidator`,
/// `MultiPolygonValidator` of the `*Multi*Queries` classes): one inner
/// validator per element of the indexed `Object[]`.
///
/// Divergence: Java declares one nested class per test class with the same
/// body; they share this one struct, parameterised by the inner validator and
/// by [`MultiContains`].
pub struct MultiValidator {
    base: ValidatorBase,
    inner: Box<dyn Validator>,
    contains: MultiContains,
}

impl MultiValidator {
    /// The constructor `(Encoder encoder)`, which creates the inner validator.
    pub fn new(encoder: Encoder, inner: Box<dyn Validator>, contains: MultiContains) -> Self {
        Self {
            base: ValidatorBase::new(encoder),
            inner,
            contains,
        }
    }

    /// The private `testWithinPolygon(Component2D query, Polygon[] polygons)`.
    fn test_within_polygons(&self, query: &dyn Component2D, shapes: &[Shape]) -> Result<bool> {
        let mut answer = WithinRelation::Disjoint;
        for p in shapes {
            let relation = match self.contains {
                MultiContains::LatLonPolygons => self.inner.base().test_within_query(
                    query,
                    &LatLonShape::create_indexable_fields_polygon("dummy", p.as_polygon())?,
                )?,
                _ => self.inner.base().test_within_query(
                    query,
                    &XYShape::create_indexable_fields_polygon("dummy", p.as_xy_polygon())?,
                )?,
            };
            if relation == WithinRelation::NotWithin {
                return Ok(false);
            } else if relation == WithinRelation::Candidate {
                answer = relation;
            }
        }
        Ok(answer == WithinRelation::Candidate)
    }
}

impl Validator for MultiValidator {
    fn base(&self) -> &ValidatorBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut ValidatorBase {
        &mut self.base
    }

    fn set_relation(&mut self, relation: QueryRelation) {
        self.base.query_relation = relation;
        self.inner.base_mut().query_relation = relation;
    }

    fn test_component_query(&self, query: &dyn Component2D, shape: &Shape) -> Result<bool> {
        let shapes = shape.as_array();
        let query_relation = self.base.query_relation;
        if self.contains != MultiContains::Loop && query_relation == QueryRelation::Contains {
            return self.test_within_polygons(query, shapes);
        }
        for p in shapes {
            let b = self.inner.test_component_query(query, p)?;
            if b && query_relation == QueryRelation::Intersects {
                return Ok(true);
            } else if b && query_relation == QueryRelation::Contains {
                return Ok(true);
            } else if !b && query_relation == QueryRelation::Disjoint {
                return Ok(false);
            } else if !b && query_relation == QueryRelation::Within {
                return Ok(false);
            }
        }
        Ok(
            query_relation != QueryRelation::Intersects
                && query_relation != QueryRelation::Contains,
        )
    }
}

// -----------------------------------------------------------------------------
// Shape types
// -----------------------------------------------------------------------------

/// `ShapeType`, the nested enum of `BaseLatLonSpatialTestCase` and of
/// `BaseXYShapeTestCase` (same constants, different generators).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeType {
    /// `POINT`.
    Point,
    /// `LINE`.
    Line,
    /// `POLYGON`.
    Polygon,
    /// `MIXED`.
    Mixed,
}

/// `ShapeType.subList`.
const SUB_LIST: [ShapeType; 3] = [ShapeType::Point, ShapeType::Line, ShapeType::Polygon];

impl ShapeType {
    /// `BaseLatLonSpatialTestCase.ShapeType.nextShape()`.
    pub fn next_lat_lon_shape(self) -> Result<Shape> {
        Ok(match self {
            ShapeType::Point => Shape::Point(geo_test_util::next_point()),
            ShapeType::Line => Shape::Line(geo_test_util::next_line()),
            ShapeType::Polygon => loop {
                let p = geo_test_util::next_polygon();
                match Tessellator::tessellate(&p, random().next_boolean()) {
                    Ok(_) => break Shape::Polygon(p),
                    // if we can't tessellate; then random polygon generator
                    // created a malformed shape
                    Err(LuceneError::IllegalArgument(_)) => {}
                    Err(e) => return Err(e),
                }
            },
            ShapeType::Mixed => {
                RandomPicks::random_from(&*random(), &SUB_LIST).next_lat_lon_shape()?
            }
        })
    }

    /// `BaseXYShapeTestCase.ShapeType.nextShape()`.
    pub fn next_xy_shape(self) -> Result<Shape> {
        Ok(match self {
            ShapeType::Point => Shape::XYPoint(shape_test_util::next_xy_point()),
            ShapeType::Line => Shape::XYLine(shape_test_util::next_line()),
            ShapeType::Polygon => loop {
                let p = shape_test_util::next_polygon();
                match Tessellator::tessellate_xy(&p, true) {
                    Ok(_) => break Shape::XYPolygon(p),
                    // if we can't tessellate; then random polygon generator
                    // created a malformed shape
                    Err(LuceneError::IllegalArgument(_)) => {}
                    Err(e) => return Err(e),
                }
            },
            ShapeType::Mixed => RandomPicks::random_from(&*random(), &SUB_LIST).next_xy_shape()?,
        })
    }
}

// -----------------------------------------------------------------------------
// The case trait
// -----------------------------------------------------------------------------

/// Which intermediate abstract base a concrete Java test class extends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// `BaseLatLonShapeTestCase`.
    LatLonShape,
    /// `BaseLatLonPointTestCase`.
    LatLonPoint,
    /// `BaseLatLonDocValueTestCase`.
    LatLonDocValue,
    /// `BaseLatLonShapeDocValueTestCase`.
    LatLonShapeDocValue,
    /// `BaseXYShapeTestCase`.
    XYShape,
    /// `BaseXYShapeDocValueTestCase`.
    XYShapeDocValue,
}

impl Family {
    /// Whether the family extends `BaseXYShapeTestCase` (otherwise
    /// `BaseLatLonSpatialTestCase`).
    pub fn is_xy(self) -> bool {
        matches!(self, Family::XYShape | Family::XYShapeDocValue)
    }
}

/// Converts the varargs `Object...` of lat/lon geometries.
fn lat_lon_geometries(shapes: &[Shape]) -> Vec<LatLonGeometryValue> {
    shapes
        .iter()
        .map(|s| match s {
            Shape::Line(l) => LatLonGeometryValue::Line(l.clone()),
            Shape::Polygon(p) => LatLonGeometryValue::Polygon(p.clone()),
            Shape::Circle(c) => LatLonGeometryValue::Circle(c.clone()),
            Shape::Rectangle(r) => LatLonGeometryValue::Rectangle(r.clone()),
            Shape::Point(p) => LatLonGeometryValue::Point(p.clone()),
            other => class_cast(other, "org.apache.lucene.geo.LatLonGeometry"),
        })
        .collect()
}

/// Converts the varargs `Object...` of cartesian geometries.
fn xy_geometries(shapes: &[Shape]) -> Vec<XYGeometryValue> {
    shapes
        .iter()
        .map(|s| match s {
            Shape::XYLine(l) => XYGeometryValue::Line(l.clone()),
            Shape::XYPolygon(p) => XYGeometryValue::Polygon(p.clone()),
            Shape::XYCircle(c) => XYGeometryValue::Circle(c.clone()),
            Shape::XYRectangle(r) => XYGeometryValue::Rectangle(r.clone()),
            Shape::XYPoint(p) => XYGeometryValue::Point(p.clone()),
            other => class_cast(other, "org.apache.lucene.geo.XYGeometry"),
        })
        .collect()
}

/// `new Point(point[0], point[1])` for each `double[]` of `points`.
fn lat_lon_points(points: &[Shape]) -> Result<Vec<LatLonGeometryValue>> {
    points
        .iter()
        .map(|p| {
            let point = p.as_doubles();
            Ok(LatLonGeometryValue::Point(Point::new(point[0], point[1])?))
        })
        .collect()
}

/// `ShapeDocValuesField.newGeometryQuery(field, ...)`, which always throws.
fn shape_doc_values_geometry_query(field: &str) -> Result<Arc<dyn Query>> {
    ShapeDocValuesField::new_geometry_query(field)?;
    unreachable!("ShapeDocValuesField.newGeometryQuery always throws IllegalStateException")
}

/// `BaseSpatialTestCase` and its intermediate subclasses, as one trait.
///
/// The concrete Java class's instance is a fresh value per test method, as
/// JUnit creates one instance per method; [`SpatialCase::validator`] is its
/// `VALIDATOR` field.
pub trait SpatialCase: Sized + 'static {
    /// A new instance (Java's constructor): `ENCODER = getEncoder();
    /// VALIDATOR = getValidator();`.
    fn new_instance() -> Self;

    /// Which intermediate base the Java class extends.
    fn family(&self) -> Family;

    /// `getShapeType()`.
    fn get_shape_type(&self) -> ShapeType;

    /// `createIndexableFields(field, shape)`.
    fn create_indexable_fields(
        &self,
        field: &str,
        shape: &Shape,
    ) -> Result<Vec<Box<dyn IndexableField>>>;

    /// `getValidator()`.
    fn get_validator(&self) -> Box<dyn Validator>;

    /// The `VALIDATOR` field.
    fn validator(&self) -> &RefCell<Box<dyn Validator>>;

    /// `ENCODER`.
    fn encoder(&self) -> Encoder {
        self.get_encoder()
    }

    /// `getEncoder()`.
    fn get_encoder(&self) -> Encoder {
        if self.family().is_xy() {
            Encoder::XY
        } else {
            Encoder::LatLon
        }
    }

    /// `nextShape()`.
    fn next_shape(&self) -> Result<Shape> {
        if self.family().is_xy() {
            self.get_shape_type().next_xy_shape()
        } else {
            self.get_shape_type().next_lat_lon_shape()
        }
    }

    /// `nextLine()`.
    fn next_line(&self) -> Shape {
        if self.family().is_xy() {
            Shape::XYLine(shape_test_util::next_line())
        } else {
            Shape::Line(geo_test_util::next_line())
        }
    }

    /// `nextPolygon()`.
    fn next_polygon(&self) -> Shape {
        if self.family().is_xy() {
            Shape::XYPolygon(shape_test_util::next_polygon())
        } else {
            Shape::Polygon(geo_test_util::next_polygon())
        }
    }

    /// `randomQueryBox()`.
    fn random_query_box(&self) -> Shape {
        if self.family().is_xy() {
            Shape::XYRectangle(shape_test_util::next_box(&*random()))
        } else {
            Shape::Rectangle(geo_test_util::next_box())
        }
    }

    /// `nextPoints()`.
    fn next_points(&self) -> Vec<Shape> {
        if self.family().is_xy() {
            let random = random();
            let num_points = TestUtil::next_int(&*random, 1, 20);
            let mut points = Vec::with_capacity(num_points as usize);
            for _ in 0..num_points {
                let x = shape_test_util::next_float(&*random);
                let y = shape_test_util::next_float(&*random);
                points.push(Shape::Floats([x, y]));
            }
            points
        } else {
            let num_points = TestUtil::next_int(&*random(), 1, 20);
            let mut points = Vec::with_capacity(num_points as usize);
            for _ in 0..num_points {
                let lat = geo_test_util::next_latitude();
                let lon = geo_test_util::next_longitude();
                points.push(Shape::Doubles([lat, lon]));
            }
            points
        }
    }

    /// `nextCircle()`.
    fn next_circle(&self) -> Result<Shape> {
        if self.family().is_xy() {
            Ok(Shape::XYCircle(shape_test_util::next_circle()))
        } else {
            let radius_meters =
                random().next_double() * GeoUtils::EARTH_MEAN_RADIUS_METERS * std::f64::consts::PI
                    / 2.0
                    + 1.0;
            Ok(Shape::Circle(Circle::new(
                geo_test_util::next_latitude(),
                geo_test_util::next_longitude(),
                radius_meters,
            )?))
        }
    }

    /// `rectMinX(rect)`.
    fn rect_min_x(&self, rect: &Shape) -> f64 {
        if self.family().is_xy() {
            f64::from(rect.as_xy_rectangle().min_x())
        } else {
            rect.as_rectangle().min_lon()
        }
    }

    /// `rectMaxX(rect)`.
    fn rect_max_x(&self, rect: &Shape) -> f64 {
        if self.family().is_xy() {
            f64::from(rect.as_xy_rectangle().max_x())
        } else {
            rect.as_rectangle().max_lon()
        }
    }

    /// `rectMinY(rect)`.
    fn rect_min_y(&self, rect: &Shape) -> f64 {
        if self.family().is_xy() {
            f64::from(rect.as_xy_rectangle().min_y())
        } else {
            rect.as_rectangle().min_lat()
        }
    }

    /// `rectMaxY(rect)`.
    fn rect_max_y(&self, rect: &Shape) -> f64 {
        if self.family().is_xy() {
            f64::from(rect.as_xy_rectangle().max_y())
        } else {
            rect.as_rectangle().max_lat()
        }
    }

    /// `rectCrossesDateline(rect)`.
    fn rect_crosses_dateline(&self, rect: &Shape) -> bool {
        if self.family().is_xy() {
            false
        } else {
            rect.as_rectangle().crosses_dateline()
        }
    }

    /// `getSupportedQueryRelations()`.
    fn get_supported_query_relations(&self) -> &'static [QueryRelation] {
        match self.family() {
            Family::LatLonShapeDocValue | Family::XYShapeDocValue => &DOC_VALUE_RELATIONS,
            _ => &QUERY_RELATIONS,
        }
    }

    /// `randomQueryLine(Object... shapes)`.
    fn random_query_line(&self, _shapes: &[Option<Shape>]) -> Result<Shape> {
        Ok(self.next_line())
    }

    /// `randomQueryPolygon()`.
    fn random_query_polygon(&self) -> Shape {
        self.next_polygon()
    }

    /// `randomQueryCircle()`.
    fn random_query_circle(&self) -> Result<Shape> {
        self.next_circle()
    }

    /// `newRectQuery(field, queryRelation, minX, maxX, minY, maxY)`.
    fn new_rect_query(
        &self,
        field: &str,
        query_relation: QueryRelation,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
    ) -> Result<Arc<dyn Query>> {
        match self.family() {
            Family::LatLonShape => {
                LatLonShape::new_box_query(field, query_relation, min_y, max_y, min_x, max_x)?
                    .into_query()
            }
            Family::LatLonPoint => LatLonPoint::new_geometry_query(
                Some(field),
                query_relation,
                vec![LatLonGeometryValue::Rectangle(Rectangle::new(
                    min_y, max_y, min_x, max_x,
                )?)],
            ),
            Family::LatLonDocValue => LatLonDocValuesField::new_slow_geometry_query(
                Some(field),
                query_relation,
                vec![LatLonGeometryValue::Rectangle(Rectangle::new(
                    min_y, max_y, min_x, max_x,
                )?)],
            ),
            Family::LatLonShapeDocValue => LatLonShape::new_slow_doc_values_box_query(
                field,
                query_relation,
                min_y,
                max_y,
                min_x,
                max_x,
            )?
            .into_query(),
            Family::XYShape => XYShape::new_box_query(
                field,
                query_relation,
                min_x as f32,
                max_x as f32,
                min_y as f32,
                max_y as f32,
            )?
            .into_query(),
            Family::XYShapeDocValue => Ok(Arc::new(XYShape::new_slow_doc_values_box_query(
                field,
                query_relation,
                min_x as f32,
                max_x as f32,
                min_y as f32,
                max_y as f32,
            )?)),
        }
    }

    /// `newLineQuery(field, queryRelation, Object... lines)`.
    fn new_line_query(
        &self,
        field: &str,
        query_relation: QueryRelation,
        lines: &[Shape],
    ) -> Result<Arc<dyn Query>> {
        match self.family() {
            Family::LatLonShape => LatLonShape::new_line_query(
                field,
                query_relation,
                lines.iter().map(|l| l.as_line().clone()).collect(),
            )?
            .into_query(),
            Family::LatLonPoint => {
                let lines = lines
                    .iter()
                    .map(|l| l.as_line().clone())
                    .collect::<Vec<_>>();
                LatLonPoint::new_geometry_query(
                    Some(field),
                    query_relation,
                    lines.into_iter().map(LatLonGeometryValue::Line).collect(),
                )
            }
            Family::LatLonDocValue => {
                let lines = lines
                    .iter()
                    .map(|l| l.as_line().clone())
                    .collect::<Vec<_>>();
                LatLonDocValuesField::new_slow_geometry_query(
                    Some(field),
                    query_relation,
                    lines.into_iter().map(LatLonGeometryValue::Line).collect(),
                )
            }
            Family::LatLonShapeDocValue => {
                lines.iter().for_each(|l| {
                    l.as_line();
                });
                shape_doc_values_geometry_query(field)
            }
            Family::XYShape => XYShape::new_line_query(
                field,
                query_relation,
                lines.iter().map(|l| l.as_xy_line().clone()).collect(),
            )?
            .into_query(),
            Family::XYShapeDocValue => {
                lines.iter().for_each(|l| {
                    l.as_xy_line();
                });
                shape_doc_values_geometry_query(field)
            }
        }
    }

    /// `newPolygonQuery(field, queryRelation, Object... polygons)`.
    fn new_polygon_query(
        &self,
        field: &str,
        query_relation: QueryRelation,
        polygons: &[Shape],
    ) -> Result<Arc<dyn Query>> {
        match self.family() {
            Family::LatLonShape => LatLonShape::new_polygon_query(
                field,
                query_relation,
                polygons.iter().map(|p| p.as_polygon().clone()).collect(),
            )?
            .into_query(),
            Family::LatLonPoint => {
                let polygons = polygons
                    .iter()
                    .map(|p| p.as_polygon().clone())
                    .collect::<Vec<_>>();
                LatLonPoint::new_geometry_query(
                    Some(field),
                    query_relation,
                    polygons
                        .into_iter()
                        .map(LatLonGeometryValue::Polygon)
                        .collect(),
                )
            }
            Family::LatLonDocValue => {
                let polygons = polygons
                    .iter()
                    .map(|p| p.as_polygon().clone())
                    .collect::<Vec<_>>();
                LatLonDocValuesField::new_slow_geometry_query(
                    Some(field),
                    query_relation,
                    polygons
                        .into_iter()
                        .map(LatLonGeometryValue::Polygon)
                        .collect(),
                )
            }
            Family::LatLonShapeDocValue => {
                polygons.iter().for_each(|p| {
                    p.as_polygon();
                });
                shape_doc_values_geometry_query(field)
            }
            Family::XYShape => XYShape::new_polygon_query(
                field,
                query_relation,
                polygons.iter().map(|p| p.as_xy_polygon().clone()).collect(),
            )?
            .into_query(),
            Family::XYShapeDocValue => {
                polygons.iter().for_each(|p| {
                    p.as_xy_polygon();
                });
                shape_doc_values_geometry_query(field)
            }
        }
    }

    /// `newPointsQuery(field, queryRelation, Object... points)`.
    fn new_points_query(
        &self,
        field: &str,
        query_relation: QueryRelation,
        points: &[Shape],
    ) -> Result<Arc<dyn Query>> {
        match self.family() {
            Family::LatLonShape => LatLonShape::new_point_query(
                field,
                query_relation,
                &points.iter().map(|p| *p.as_doubles()).collect::<Vec<_>>(),
            )?
            .into_query(),
            Family::LatLonPoint => LatLonPoint::new_geometry_query(
                Some(field),
                query_relation,
                lat_lon_points(points)?,
            ),
            Family::LatLonDocValue => LatLonDocValuesField::new_slow_geometry_query(
                Some(field),
                query_relation,
                lat_lon_points(points)?,
            ),
            Family::LatLonShapeDocValue => {
                points.iter().for_each(|p| {
                    p.as_doubles();
                });
                shape_doc_values_geometry_query(field)
            }
            Family::XYShape => XYShape::new_point_query(
                field,
                query_relation,
                &points.iter().map(|p| *p.as_floats()).collect::<Vec<_>>(),
            )?
            .into_query(),
            Family::XYShapeDocValue => {
                points.iter().for_each(|p| {
                    p.as_floats();
                });
                shape_doc_values_geometry_query(field)
            }
        }
    }

    /// `newDistanceQuery(field, queryRelation, Object circle)`.
    fn new_distance_query(
        &self,
        field: &str,
        query_relation: QueryRelation,
        circle: &Shape,
    ) -> Result<Arc<dyn Query>> {
        match self.family() {
            // `BaseXYShapeDocValueTestCase` casts its `XYCircle` to `Circle`
            // here, as Java does: a `ClassCastException`.
            Family::LatLonShape | Family::LatLonShapeDocValue | Family::XYShapeDocValue => {
                LatLonShape::new_distance_query(
                    field,
                    query_relation,
                    vec![circle.as_circle().clone()],
                )?
                .into_query()
            }
            Family::LatLonPoint => LatLonPoint::new_geometry_query(
                Some(field),
                query_relation,
                vec![LatLonGeometryValue::Circle(circle.as_circle().clone())],
            ),
            Family::LatLonDocValue => LatLonDocValuesField::new_slow_geometry_query(
                Some(field),
                query_relation,
                vec![LatLonGeometryValue::Circle(circle.as_circle().clone())],
            ),
            Family::XYShape => XYShape::new_distance_query(
                field,
                query_relation,
                vec![circle.as_xy_circle().clone()],
            )?
            .into_query(),
        }
    }

    /// `toLine2D(Object... line)`.
    fn to_line_2d(&self, lines: &[Shape]) -> Result<Arc<dyn Component2D>> {
        if self.family().is_xy() {
            lines.iter().for_each(|l| {
                l.as_xy_line();
            });
            XYGeometryValue::create(&xy_geometries(lines))
        } else {
            lines.iter().for_each(|l| {
                l.as_line();
            });
            LatLonGeometryValue::create(&lat_lon_geometries(lines))
        }
    }

    /// `toPolygon2D(Object... polygon)`.
    fn to_polygon_2d(&self, polygons: &[Shape]) -> Result<Arc<dyn Component2D>> {
        if self.family().is_xy() {
            polygons.iter().for_each(|p| {
                p.as_xy_polygon();
            });
            XYGeometryValue::create(&xy_geometries(polygons))
        } else {
            polygons.iter().for_each(|p| {
                p.as_polygon();
            });
            LatLonGeometryValue::create(&lat_lon_geometries(polygons))
        }
    }

    /// `toPoint2D(Object... points)`.
    fn to_point_2d(&self, points: &[Shape]) -> Result<Arc<dyn Component2D>> {
        if self.family().is_xy() {
            let mut point_array = Vec::with_capacity(points.len());
            for p in points {
                let p = p.as_floats();
                point_array.push(XYGeometryValue::Point(XYPoint::new(p[0], p[1])?));
            }
            XYGeometryValue::create(&point_array)
        } else {
            LatLonGeometryValue::create(&lat_lon_points(points)?)
        }
    }

    /// `toCircle2D(Object circle)`.
    fn to_circle_2d(&self, circle: &Shape) -> Result<Arc<dyn Component2D>> {
        if self.family().is_xy() {
            XYGeometryValue::create(&[XYGeometryValue::Circle(circle.as_xy_circle().clone())])
        } else {
            LatLonGeometryValue::create(&[LatLonGeometryValue::Circle(circle.as_circle().clone())])
        }
    }

    /// `toRectangle2D(minX, maxX, minY, maxY)`.
    fn to_rectangle_2d(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
    ) -> Result<Arc<dyn Component2D>> {
        if self.family().is_xy() {
            XYGeometryValue::create(&[XYGeometryValue::Rectangle(XYRectangle::new(
                min_x as f32,
                max_x as f32,
                min_y as f32,
                max_y as f32,
            )?)])
        } else {
            LatLonGeometryValue::create(&[LatLonGeometryValue::Rectangle(Rectangle::new(
                min_y, max_y, min_x, max_x,
            )?)])
        }
    }

    /// `testRandomBig()` (`@Nightly`): its body, which subclasses override.
    fn test_random_big(&self) -> Result<()> {
        do_test_random(self, 20000)
    }

    /// `verifyRandomLineQueries(reader, shapes)`.
    fn verify_random_line_queries(
        &self,
        reader: &Arc<dyn DirectoryReader>,
        shapes: &[Option<Shape>],
    ) -> Result<()> {
        if matches!(
            self.family(),
            Family::LatLonShapeDocValue | Family::XYShapeDocValue
        ) {
            // NOT IMPLEMENTED YET (the `*ShapeDVQueries` override)
            return Ok(());
        }
        base_verify_random_line_queries(self, reader, shapes)
    }

    /// `verifyRandomPolygonQueries(reader, shapes)`.
    fn verify_random_polygon_queries(
        &self,
        reader: &Arc<dyn DirectoryReader>,
        shapes: &[Option<Shape>],
    ) -> Result<()> {
        if matches!(
            self.family(),
            Family::LatLonShapeDocValue | Family::XYShapeDocValue
        ) {
            // NOT IMPLEMENTED YET (the `*ShapeDVQueries` override)
            return Ok(());
        }
        base_verify_random_polygon_queries(self, reader, shapes)
    }

    /// `verifyRandomPointQueries(reader, shapes)`.
    fn verify_random_point_queries(
        &self,
        reader: &Arc<dyn DirectoryReader>,
        shapes: &[Option<Shape>],
    ) -> Result<()> {
        if matches!(
            self.family(),
            Family::LatLonShapeDocValue | Family::XYShapeDocValue
        ) {
            // NOT IMPLEMENTED YET (the `*ShapeDVQueries` override)
            return Ok(());
        }
        base_verify_random_point_queries(self, reader, shapes)
    }

    /// `verifyRandomDistanceQueries(reader, shapes)`.
    fn verify_random_distance_queries(
        &self,
        reader: &Arc<dyn DirectoryReader>,
        shapes: &[Option<Shape>],
    ) -> Result<()> {
        if matches!(
            self.family(),
            Family::LatLonShapeDocValue | Family::XYShapeDocValue
        ) {
            // NOT IMPLEMENTED YET (the `*ShapeDVQueries` override)
            return Ok(());
        }
        base_verify_random_distance_queries(self, reader, shapes)
    }
}

/// `VALIDATOR.setRelation(queryRelation).testComponentQuery(query, shape)`.
fn validate<C: SpatialCase>(
    case: &C,
    relation: QueryRelation,
    query: &dyn Component2D,
    shape: &Shape,
) -> Result<bool> {
    let mut validator = case.validator().borrow_mut();
    validator.set_relation(relation);
    validator.test_component_query(query, shape)
}

// -----------------------------------------------------------------------------
// The inherited test methods
// -----------------------------------------------------------------------------

/// The test methods every concrete class inherits from `BaseSpatialTestCase`,
/// plus those of its intermediate base (`testBoundingBoxQueriesEquivalence`,
/// `testBoxQueryEqualsAndHashcode`, ...), bound to `C`.
pub fn base_methods<C: SpatialCase>() -> Vec<Method> {
    fn t<C: SpatialCase>(f: fn(&C) -> Result<()>) -> impl Fn() -> Result<()> + Send + Sync {
        move || f(&C::new_instance())
    }
    let mut methods = vec![
        test_closure("testSameShapeManyTimes", t::<C>(test_same_shape_many_times)),
        test_closure(
            "testLowCardinalityShapeManyTimes",
            t::<C>(test_low_cardinality_shape_many_times),
        ),
        test_closure("testRandomTiny", t::<C>(test_random_tiny)),
        test_closure("testRandomMedium", t::<C>(test_random_medium)),
        test_closure("testRandomBig", t::<C>(|c: &C| c.test_random_big())).annotate(Nightly),
    ];
    match C::new_instance().family() {
        Family::LatLonShape => {
            methods.push(test_closure(
                "testBoundingBoxQueriesEquivalence",
                t::<C>(lat_lon_shape_test_bounding_box_queries_equivalence),
            ));
            methods.push(test_closure(
                "testBoxQueryEqualsAndHashcode",
                t::<C>(test_box_query_equals_and_hashcode),
            ));
            methods.push(test_closure(
                "testLineQueryEqualsAndHashcode",
                t::<C>(test_line_query_equals_and_hashcode),
            ));
            methods.push(test_closure(
                "testPolygonQueryEqualsAndHashcode",
                t::<C>(test_polygon_query_equals_and_hashcode),
            ));
        }
        Family::LatLonPoint => {
            methods.push(test_closure(
                "testBoundingBoxQueriesEquivalence",
                t::<C>(lat_lon_point_test_bounding_box_queries_equivalence),
            ));
            methods.push(test_closure(
                "testQueryEqualsAndHashcode",
                t::<C>(test_query_equals_and_hashcode),
            ));
        }
        _ => {}
    }
    methods
}

/// `testSameShapeManyTimes()`: a particularly tricky adversary for BKD tree.
pub fn test_same_shape_many_times<C: SpatialCase>(case: &C) -> Result<()> {
    let num_shapes = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(50)
    } else {
        LuceneTestCase::at_least(3)
    };

    // Every doc has 2 points:
    let the_shape = case.next_shape()?;

    let shapes = vec![Some(the_shape); num_shapes as usize];

    verify(case, &shapes)
}

/// `testLowCardinalityShapeManyTimes()`: force low cardinality leaves.
pub fn test_low_cardinality_shape_many_times<C: SpatialCase>(case: &C) -> Result<()> {
    let num_shapes = LuceneTestCase::at_least(20);
    let cardinality = TestUtil::next_int(&*random(), 2, 20);

    let mut diff_shapes = Vec::with_capacity(cardinality as usize);
    for _ in 0..cardinality {
        diff_shapes.push(case.next_shape()?);
    }

    let mut shapes = Vec::with_capacity(num_shapes as usize);
    for _ in 0..num_shapes {
        shapes.push(Some(
            diff_shapes[random().next_int_bound(cardinality) as usize].clone(),
        ));
    }

    verify(case, &shapes)
}

/// `testRandomTiny()`: make sure single-leaf-node case is OK.
pub fn test_random_tiny<C: SpatialCase>(case: &C) -> Result<()> {
    do_test_random(case, 10)
}

/// `testRandomMedium()`.
pub fn test_random_medium<C: SpatialCase>(case: &C) -> Result<()> {
    do_test_random(case, LuceneTestCase::at_least(20))
}

/// `doTestRandom(count)`.
pub fn do_test_random<C: SpatialCase>(case: &C, count: i32) -> Result<()> {
    let num_shapes = LuceneTestCase::at_least(count);

    if verbose() {
        println!(
            "TEST: number of {:?} shapes={num_shapes}",
            case.get_shape_type()
        );
    }

    let mut shapes = Vec::with_capacity(num_shapes as usize);
    for id in 0..num_shapes {
        // `RandomizedTest.randomIntBetween(0, 20)`.
        let x = RandomNumbers::random_int_between(&*random(), 0, 20);
        if x == 17 {
            shapes.push(None);
            if verbose() {
                println!("  id={id} is missing");
            }
        } else {
            // create a new shape
            shapes.push(Some(case.next_shape()?));
        }
    }
    verify(case, &shapes)
}

/// The private `addShapeToDoc(field, doc, shape)`.
fn add_shape_to_doc<C: SpatialCase>(
    case: &C,
    field: &str,
    doc: &mut Document,
    shape: &Shape,
) -> Result<()> {
    for f in case.create_indexable_fields(field, shape)? {
        doc.add(f);
    }
    Ok(())
}

/// The private `verify(Object... shapes)`.
fn verify<C: SpatialCase>(case: &C, shapes: &[Option<Shape>]) -> Result<()> {
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
    let mbd = iwc.max_buffered_docs();
    if mbd != -1 && mbd < (shapes.len() / 100) as i32 {
        iwc.set_max_buffered_docs((shapes.len() / 100) as i32)?;
    }
    let mut dir = if shapes.len() > 1000 {
        let simple_name = LuceneTestCase::get_test_class().simple_name().to_string();
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(&simple_name))
    } else {
        LuceneTestCase::new_directory()
    };
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    // index random polygons
    index_random_shapes(case, &w, shapes)?;

    // query testing
    let reader = directory_reader::open_with_writer(&*w)?;
    // test random bbox queries
    verify_random_queries(case, &reader, shapes)?;
    w.close()?;
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// `indexRandomShapes(w, Object... shapes)`.
pub fn index_random_shapes<C: SpatialCase>(
    case: &C,
    w: &IndexWriter,
    shapes: &[Option<Shape>],
) -> Result<()> {
    let mut deleted: HashSet<i32> = HashSet::new();
    for (id, shape) in shapes.iter().enumerate() {
        let id = id as i32;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
        doc.add(Box::new(NumericDocValuesField::new("id", i64::from(id))));
        if let Some(shape) = shape {
            add_shape_to_doc(case, FIELD_NAME, &mut doc, shape)?;
        }
        w.add_document(&doc)?;
        if id > 0 && random().next_int_bound(100) == 42 {
            let id_to_delete = random().next_int_bound(id);
            w.delete_documents_by_terms(vec![Term::new(
                "id",
                BytesRef::from_char_sequence(&id_to_delete.to_string()),
            )])?;
            deleted.insert(id_to_delete);
            if verbose() {
                println!("   delete id={id_to_delete}");
            }
        }
    }

    // `RandomizedTest.randomBoolean()`.
    if random().next_boolean() {
        w.force_merge(1)?;
    }
    Ok(())
}

/// `verifyRandomQueries(reader, Object... shapes)`.
pub fn verify_random_queries<C: SpatialCase>(
    case: &C,
    reader: &Arc<dyn DirectoryReader>,
    shapes: &[Option<Shape>],
) -> Result<()> {
    // test random bbox queries
    verify_random_bbox_queries(case, reader, shapes)?;
    // test random line queries
    case.verify_random_line_queries(reader, shapes)?;
    // test random polygon queries
    case.verify_random_polygon_queries(reader, shapes)?;
    // test random point queries
    case.verify_random_point_queries(reader, shapes)?;
    // test random distance queries
    case.verify_random_distance_queries(reader, shapes)
}

/// The shared header of a `FAIL:` message.
fn fail_message(
    expected: bool,
    id: i32,
    query_relation: QueryRelation,
    query: &dyn Query,
    doc_id: i32,
    shape: &str,
    deleted: bool,
) -> String {
    let mut b = String::new();
    if expected {
        b.push_str(&format!("FAIL: id={id} should match but did not\n"));
    } else {
        b.push_str(&format!("FAIL: id={id} should not match but did\n"));
    }
    b.push_str(&format!("  relation={query_relation:?}\n"));
    b.push_str(&format!(
        "  query={} docID={doc_id}\n",
        rucene::search::query::query_to_string(query)
    ));
    b.push_str(&format!("  shape={shape}\n"));
    b.push_str(&format!("  deleted?={deleted}"));
    b
}

/// The verification loop the five `verifyRandom*Queries` share: compares the
/// hits of `query` with `expected_of(shape)` for every document.
fn check_hits(
    reader: &Arc<dyn DirectoryReader>,
    shapes: &[Option<Shape>],
    hits: &FixedBitSet,
    live_docs: &Option<Box<dyn rucene::util::Bits>>,
    max_doc: i32,
    query_relation: QueryRelation,
    query: &dyn Query,
    mut expected_of: impl FnMut(&Shape) -> Result<bool>,
    suffix: &dyn Fn() -> String,
) -> Result<()> {
    let r: Arc<dyn IndexReader> = Arc::clone(reader) as Arc<dyn IndexReader>;
    let mut doc_id_to_id = MultiDocValues::get_numeric_values(&r, "id")?.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    for doc_id in 0..max_doc {
        assert_equals(&doc_id, &doc_id_to_id.next_doc()?);
        let id = doc_id_to_id.long_value()? as i32;
        let deleted = live_docs.as_ref().is_some_and(|l| !l.get(doc_id as usize));
        let expected = if deleted {
            // document is deleted
            false
        } else if let Some(shape) = &shapes[id as usize] {
            expected_of(shape)?
        } else {
            false
        };

        if hits.get(doc_id as usize) != expected {
            let shape = shapes[id as usize]
                .clone()
                .map_or_else(|| "null".to_string(), |s| s.to_string());
            // `shapes[id]` prints with `Arrays.toString` when it is an array.
            let mut b = fail_message(expected, id, query_relation, query, doc_id, &shape, deleted);
            b.push_str(&suffix());
            Assert::fail(&format!("wrong hit (first of possibly more):\n\n{b}"));
        }
    }
    Ok(())
}

/// `scaledIterationCount(shapes)`.
fn scaled_iteration_count(shapes: usize) -> i32 {
    if shapes < 500 {
        LuceneTestCase::at_least(50)
    } else if shapes < 5000 {
        LuceneTestCase::at_least(25)
    } else if shapes < 25000 {
        LuceneTestCase::at_least(5)
    } else {
        LuceneTestCase::at_least(2)
    }
}

/// The private `searchIndex(s, query, maxDoc)`.
fn search_index(s: &IndexSearcher, query: Arc<dyn Query>, max_doc: i32) -> Result<FixedBitSet> {
    s.search_with_manager(query, &FixedBitSetCollector::create_manager(max_doc))
}

/// `verifyRandomBBoxQueries(reader, Object... shapes)`: test random generated
/// bounding boxes.
pub fn verify_random_bbox_queries<C: SpatialCase>(
    case: &C,
    reader: &Arc<dyn DirectoryReader>,
    shapes: &[Option<Shape>],
) -> Result<()> {
    let s = LuceneTestCase::new_searcher(Arc::clone(reader));

    let iters = scaled_iteration_count(shapes.len());

    let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
    let max_doc = s.get_index_reader().max_doc();

    for iter in 0..iters {
        if verbose() {
            println!("\nTEST: iter={} of {iters} s={s:?}", iter + 1);
        }

        // BBox
        let rect = case.random_query_box();
        let query_relation =
            *RandomPicks::random_from(&*random(), case.get_supported_query_relations());
        let query = case.new_rect_query(
            FIELD_NAME,
            query_relation,
            case.rect_min_x(&rect),
            case.rect_max_x(&rect),
            case.rect_min_y(&rect),
            case.rect_max_y(&rect),
        )?;

        if verbose() {
            println!(
                "  query={}, relation={query_relation:?}",
                rucene::search::query::query_to_string(&*query)
            );
        }

        let hits = search_index(&s, Arc::clone(&query), max_doc)?;

        let min_x = case.rect_min_x(&rect);
        let max_x = case.rect_max_x(&rect);
        let min_y = case.rect_min_y(&rect);
        let max_y = case.rect_max_y(&rect);
        let encoder = case.encoder();
        check_hits(
            reader,
            shapes,
            &hits,
            &live_docs,
            max_doc,
            query_relation,
            &*query,
            |shape| {
                if query_relation == QueryRelation::Contains && case.rect_crosses_dateline(&rect) {
                    // For contains we need to call the validator for each section.
                    // It is only expected if both sides are contained.
                    let left = case.to_rectangle_2d(min_x, GeoUtils::MAX_LON_INCL, min_y, max_y)?;
                    let right =
                        case.to_rectangle_2d(GeoUtils::MIN_LON_INCL, max_x, min_y, max_y)?;
                    Ok(validate(case, query_relation, &*left, shape)?
                        && validate(case, query_relation, &*right, shape)?)
                } else {
                    let component2d = case.to_rectangle_2d(min_x, max_x, min_y, max_y)?;
                    validate(case, query_relation, &*component2d, shape)
                }
            },
            &|| {
                format!(
                    "  rect=Rectangle(lat={} TO {} lon={} TO {})\n",
                    encoder.quantize_y_ceil(case.rect_min_y(&rect)),
                    encoder.quantize_y(case.rect_max_y(&rect)),
                    min_x,
                    encoder.quantize_x(case.rect_max_x(&rect))
                )
            },
        )?;
    }
    Ok(())
}

/// `verifyRandomLineQueries(reader, Object... shapes)` of
/// `BaseSpatialTestCase`: test random generated lines.
pub fn base_verify_random_line_queries<C: SpatialCase>(
    case: &C,
    reader: &Arc<dyn DirectoryReader>,
    shapes: &[Option<Shape>],
) -> Result<()> {
    let s = LuceneTestCase::new_searcher(Arc::clone(reader));

    let iters = scaled_iteration_count(shapes.len());

    let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
    let max_doc = s.get_index_reader().max_doc();

    for iter in 0..iters {
        if verbose() {
            println!("\nTEST: iter={} of {iters} s={s:?}", iter + 1);
        }

        // line
        let query_line = case.random_query_line(shapes)?;
        let query_line_2d = case.to_line_2d(std::slice::from_ref(&query_line))?;
        let query_relation = *RandomPicks::random_from(&*random(), &POINT_LINE_RELATIONS);
        let query = case.new_line_query(
            FIELD_NAME,
            query_relation,
            std::slice::from_ref(&query_line),
        )?;

        if verbose() {
            println!(
                "  query={}, relation={query_relation:?}",
                rucene::search::query::query_to_string(&*query)
            );
        }

        let hits = search_index(&s, Arc::clone(&query), max_doc)?;

        check_hits(
            reader,
            shapes,
            &hits,
            &live_docs,
            max_doc,
            query_relation,
            &*query,
            |shape| validate(case, query_relation, &*query_line_2d, shape),
            &|| format!("  queryPolygon={query_line}"),
        )?;
    }
    Ok(())
}

/// `verifyRandomPolygonQueries(reader, Object... shapes)` of
/// `BaseSpatialTestCase`: test random generated polygons.
pub fn base_verify_random_polygon_queries<C: SpatialCase>(
    case: &C,
    reader: &Arc<dyn DirectoryReader>,
    shapes: &[Option<Shape>],
) -> Result<()> {
    let s = LuceneTestCase::new_searcher(Arc::clone(reader));

    let iters = scaled_iteration_count(shapes.len());

    let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
    let max_doc = s.get_index_reader().max_doc();

    for iter in 0..iters {
        if verbose() {
            println!("\nTEST: iter={} of {iters} s={s:?}", iter + 1);
        }

        // Polygon
        let query_polygon = case.random_query_polygon();
        let query_poly_2d = case.to_polygon_2d(std::slice::from_ref(&query_polygon))?;
        let query_relation = *RandomPicks::random_from(&*random(), &QUERY_RELATIONS);
        let query = case.new_polygon_query(
            FIELD_NAME,
            query_relation,
            std::slice::from_ref(&query_polygon),
        )?;

        if verbose() {
            println!(
                "  query={}, relation={query_relation:?}",
                rucene::search::query::query_to_string(&*query)
            );
        }

        let hits = search_index(&s, Arc::clone(&query), max_doc)?;

        check_hits(
            reader,
            shapes,
            &hits,
            &live_docs,
            max_doc,
            query_relation,
            &*query,
            |shape| validate(case, query_relation, &*query_poly_2d, shape),
            &|| format!("  queryPolygon={query_polygon}"),
        )?;
    }
    Ok(())
}

/// `verifyRandomPointQueries(reader, Object... shapes)` of
/// `BaseSpatialTestCase`: test random generated point queries.
pub fn base_verify_random_point_queries<C: SpatialCase>(
    case: &C,
    reader: &Arc<dyn DirectoryReader>,
    shapes: &[Option<Shape>],
) -> Result<()> {
    let s = LuceneTestCase::new_searcher(Arc::clone(reader));

    let iters = scaled_iteration_count(shapes.len());

    let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
    let max_doc = s.get_index_reader().max_doc();

    for iter in 0..iters {
        if verbose() {
            println!("\nTEST: iter={} of {iters} s={s:?}", iter + 1);
        }

        let query_points = case.next_points();
        let query_relation = *RandomPicks::random_from(&*random(), &QUERY_RELATIONS);
        let (query_poly_2d, query) = if query_relation == QueryRelation::Contains {
            (
                case.to_point_2d(&query_points[..1])?,
                case.new_points_query(FIELD_NAME, query_relation, &query_points[..1])?,
            )
        } else {
            (
                case.to_point_2d(&query_points)?,
                case.new_points_query(FIELD_NAME, query_relation, &query_points)?,
            )
        };

        if verbose() {
            println!(
                "  query={}, relation={query_relation:?}",
                rucene::search::query::query_to_string(&*query)
            );
        }

        let hits = search_index(&s, Arc::clone(&query), max_doc)?;

        check_hits(
            reader,
            shapes,
            &hits,
            &live_docs,
            max_doc,
            query_relation,
            &*query,
            |shape| validate(case, query_relation, &*query_poly_2d, shape),
            &|| format!("  rect=Points({})\n", shapes_to_string(&query_points)),
        )?;
    }
    Ok(())
}

/// `verifyRandomDistanceQueries(reader, Object... shapes)` of
/// `BaseSpatialTestCase`: test random generated circles.
pub fn base_verify_random_distance_queries<C: SpatialCase>(
    case: &C,
    reader: &Arc<dyn DirectoryReader>,
    shapes: &[Option<Shape>],
) -> Result<()> {
    let s = LuceneTestCase::new_searcher(Arc::clone(reader));

    let iters = scaled_iteration_count(shapes.len());

    let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
    let max_doc = s.get_index_reader().max_doc();

    for iter in 0..iters {
        if verbose() {
            println!("\nTEST: iter={} of {iters} s={s:?}", iter + 1);
        }

        // Polygon
        let query_circle = case.random_query_circle()?;
        let query_circle_2d = case.to_circle_2d(&query_circle)?;
        let query_relation = *RandomPicks::random_from(&*random(), &QUERY_RELATIONS);
        let query = case.new_distance_query(FIELD_NAME, query_relation, &query_circle)?;

        if verbose() {
            println!(
                "  query={}, relation={query_relation:?}",
                rucene::search::query::query_to_string(&*query)
            );
        }

        let hits = search_index(&s, Arc::clone(&query), max_doc)?;

        check_hits(
            reader,
            shapes,
            &hits,
            &live_docs,
            max_doc,
            query_relation,
            &*query,
            |shape| validate(case, query_relation, &*query_circle_2d, shape),
            &|| format!("  distanceQuery={query_circle}"),
        )?;
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Methods of the intermediate bases
// -----------------------------------------------------------------------------

/// Indexes `atLeast(20)` random shapes through a `RandomIndexWriter` and
/// returns the searcher, the reader, the writer and the directory, as the
/// first half of both `testBoundingBoxQueriesEquivalence` methods.
fn index_for_equivalence<C: SpatialCase>(
    case: &C,
) -> Result<(
    IndexSearcher,
    Arc<dyn DirectoryReader>,
    RandomIndexWriter,
    rucene_test_framework::util::lucene_test_case::TestDirectory,
)> {
    let num_shapes = LuceneTestCase::at_least(20);
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    for _ in 0..num_shapes {
        index_random_shapes(case, w.w(), &[Some(case.next_shape()?)])?;
    }
    if random().next_boolean() {
        w.force_merge(1)?;
    }

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    Ok((searcher, reader, w, dir))
}

/// `BaseLatLonShapeTestCase.testBoundingBoxQueriesEquivalence()`.
pub fn lat_lon_shape_test_bounding_box_queries_equivalence<C: SpatialCase>(case: &C) -> Result<()> {
    use rucene::document::spatial_query::LatLonShapeQuery;
    let (searcher, reader, w, mut dir) = index_for_equivalence(case)?;

    let b = geo_test_util::next_box();
    let rect = || vec![LatLonGeometryValue::Rectangle(b.clone())];
    let box_query = |relation| {
        LatLonShape::new_box_query(
            FIELD_NAME,
            relation,
            b.min_lat(),
            b.max_lat(),
            b.min_lon(),
            b.max_lon(),
        )?
        .into_query()
    };

    let q1 = box_query(QueryRelation::Intersects)?;
    let q2: Arc<dyn Query> = Arc::new(LatLonShapeQuery::new(
        FIELD_NAME,
        QueryRelation::Intersects,
        rect(),
    )?);
    assert_equals(&searcher.count(q1)?, &searcher.count(q2)?);

    let q1 = box_query(QueryRelation::Within)?;
    let q2: Arc<dyn Query> = Arc::new(LatLonShapeQuery::new(
        FIELD_NAME,
        QueryRelation::Within,
        rect(),
    )?);
    assert_equals(&searcher.count(q1)?, &searcher.count(q2)?);

    let q1 = box_query(QueryRelation::Contains)?;
    let q2: Arc<dyn Query> = if b.crosses_dateline() {
        LatLonShape::new_geometry_query(FIELD_NAME, QueryRelation::Contains, rect())?
            .into_query()?
    } else {
        Arc::new(LatLonShapeQuery::new(
            FIELD_NAME,
            QueryRelation::Contains,
            rect(),
        )?)
    };
    assert_equals(&searcher.count(q1)?, &searcher.count(q2)?);

    let q1 = box_query(QueryRelation::Disjoint)?;
    let q2: Arc<dyn Query> = Arc::new(LatLonShapeQuery::new(
        FIELD_NAME,
        QueryRelation::Disjoint,
        rect(),
    )?);
    assert_equals(&searcher.count(q1)?, &searcher.count(q2)?);

    w.close()?;
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// `BaseLatLonPointTestCase.testBoundingBoxQueriesEquivalence()`.
pub fn lat_lon_point_test_bounding_box_queries_equivalence<C: SpatialCase>(case: &C) -> Result<()> {
    use rucene::document::point_queries::LatLonPointQuery;
    let (searcher, reader, w, mut dir) = index_for_equivalence(case)?;

    let b = geo_test_util::next_box();
    let q1 = LatLonPoint::new_box_query(
        Some(FIELD_NAME),
        b.min_lat(),
        b.max_lat(),
        b.min_lon(),
        b.max_lon(),
    )?;
    let q2: Arc<dyn Query> = Arc::new(LatLonPointQuery::new(
        FIELD_NAME,
        QueryRelation::Intersects,
        vec![LatLonGeometryValue::Rectangle(b)],
    )?);
    assert_equals(&searcher.count(q1)?, &searcher.count(q2)?);

    w.close()?;
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// `BaseLatLonShapeTestCase.testBoxQueryEqualsAndHashcode()`.
pub fn test_box_query_equals_and_hashcode<C: SpatialCase>(case: &C) -> Result<()> {
    let rectangle = geo_test_util::next_box();
    let query_relation = *RandomPicks::random_from(&*random(), &QUERY_RELATIONS);
    let field_name = "foo";
    let rect_query = |field: &str, relation, r: &Rectangle| {
        case.new_rect_query(
            field,
            relation,
            r.min_lon(),
            r.max_lon(),
            r.min_lat(),
            r.max_lat(),
        )
    };
    let q1 = rect_query(field_name, query_relation, &rectangle)?;
    let q2 = rect_query(field_name, query_relation, &rectangle)?;
    QueryUtils::check_equal(&*q1, &*q2);
    // different field name
    let q3 = rect_query("bar", query_relation, &rectangle)?;
    QueryUtils::check_unequal(&*q1, &*q3);
    // different query relation
    let new_query_relation = *RandomPicks::random_from(&*random(), &QUERY_RELATIONS);
    let q4 = rect_query(field_name, new_query_relation, &rectangle)?;
    if query_relation == new_query_relation {
        QueryUtils::check_equal(&*q1, &*q4);
    } else {
        QueryUtils::check_unequal(&*q1, &*q4);
    }
    // different shape
    let new_rectangle = geo_test_util::next_box();
    let q5 = rect_query(field_name, query_relation, &new_rectangle)?;
    if rectangle == new_rectangle {
        QueryUtils::check_equal(&*q1, &*q5);
    } else {
        QueryUtils::check_unequal(&*q1, &*q5);
    }
    Ok(())
}

/// `BaseLatLonShapeTestCase.testLineQueryEqualsAndHashcode()`.
pub fn test_line_query_equals_and_hashcode<C: SpatialCase>(case: &C) -> Result<()> {
    let line = case.next_line();
    let query_relation = *RandomPicks::random_from(&*random(), &POINT_LINE_RELATIONS);
    let field_name = "foo";
    let q1 = case.new_line_query(field_name, query_relation, std::slice::from_ref(&line))?;
    let q2 = case.new_line_query(field_name, query_relation, std::slice::from_ref(&line))?;
    QueryUtils::check_equal(&*q1, &*q2);
    // different field name
    let q3 = case.new_line_query("bar", query_relation, std::slice::from_ref(&line))?;
    QueryUtils::check_unequal(&*q1, &*q3);
    // different query relation
    let new_query_relation = *RandomPicks::random_from(&*random(), &POINT_LINE_RELATIONS);
    let q4 = case.new_line_query(field_name, new_query_relation, std::slice::from_ref(&line))?;
    if query_relation == new_query_relation {
        QueryUtils::check_equal(&*q1, &*q4);
    } else {
        QueryUtils::check_unequal(&*q1, &*q4);
    }
    // different shape
    let new_line = case.next_line();
    let q5 = case.new_line_query(field_name, query_relation, std::slice::from_ref(&new_line))?;
    if line == new_line {
        QueryUtils::check_equal(&*q1, &*q5);
    } else {
        QueryUtils::check_unequal(&*q1, &*q5);
    }
    Ok(())
}

/// `BaseLatLonShapeTestCase.testPolygonQueryEqualsAndHashcode()`.
pub fn test_polygon_query_equals_and_hashcode<C: SpatialCase>(case: &C) -> Result<()> {
    polygon_query_equals_and_hashcode(case, &QUERY_RELATIONS)
}

/// `BaseLatLonPointTestCase.testQueryEqualsAndHashcode()`.
pub fn test_query_equals_and_hashcode<C: SpatialCase>(case: &C) -> Result<()> {
    polygon_query_equals_and_hashcode(case, &[QueryRelation::Intersects, QueryRelation::Disjoint])
}

/// The common body of the two polygon equality tests, which differ only in
/// the relations they draw from.
fn polygon_query_equals_and_hashcode<C: SpatialCase>(
    case: &C,
    relations: &[QueryRelation],
) -> Result<()> {
    let polygon = Shape::Polygon(geo_test_util::next_polygon());
    let query_relation = *RandomPicks::random_from(&*random(), relations);
    let field_name = "foo";
    let q1 = case.new_polygon_query(field_name, query_relation, std::slice::from_ref(&polygon))?;
    let q2 = case.new_polygon_query(field_name, query_relation, std::slice::from_ref(&polygon))?;
    QueryUtils::check_equal(&*q1, &*q2);
    // different field name
    let q3 = case.new_polygon_query("bar", query_relation, std::slice::from_ref(&polygon))?;
    QueryUtils::check_unequal(&*q1, &*q3);
    // different query relation
    let new_query_relation = *RandomPicks::random_from(&*random(), relations);
    let q4 = case.new_polygon_query(
        field_name,
        new_query_relation,
        std::slice::from_ref(&polygon),
    )?;
    if query_relation == new_query_relation {
        QueryUtils::check_equal(&*q1, &*q4);
    } else {
        QueryUtils::check_unequal(&*q1, &*q4);
    }
    // different shape
    let new_polygon = Shape::Polygon(geo_test_util::next_polygon());
    let q5 = case.new_polygon_query(
        field_name,
        query_relation,
        std::slice::from_ref(&new_polygon),
    )?;
    if polygon == new_polygon {
        QueryUtils::check_equal(&*q1, &*q5);
    } else {
        QueryUtils::check_unequal(&*q1, &*q5);
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// `randomQueryLine` overrides
// -----------------------------------------------------------------------------

/// The `randomQueryLine(Object... shapes)` override of
/// `TestLatLonPointShapeQueries` (`vertex == None`) and
/// `TestLatLonLineShapeQueries` (picks a line vertex).
pub fn lat_lon_random_query_line<C: SpatialCase>(
    case: &C,
    shapes: &[Option<Shape>],
    from_line: bool,
) -> Result<Shape> {
    if random().next_int_bound(100) == 42 {
        // we want to ensure some cross, so randomly generate lines that share
        // vertices with the indexed point set
        let mut max_bound = (shapes.len() as f64 * 0.1).floor() as i32;
        if max_bound < 2 {
            max_bound = shapes.len() as i32;
        }
        let n = RandomNumbers::random_int_between(&*random(), 2, max_bound) as usize;
        let mut lats = vec![0f64; n];
        let mut lons = vec![0f64; n];
        let (mut i, mut j) = (0usize, 0usize);
        while j < lats.len() && i < shapes.len() {
            let take = random().next_boolean();
            match (&shapes[i], take) {
                (Some(shape), true) => {
                    if from_line {
                        let l = shape.as_line();
                        let v = random().next_int_bound(l.num_points() as i32 - 1) as usize;
                        lats[j] = l.get_lat(v);
                        lons[j] = l.get_lon(v);
                    } else {
                        let p = shape.as_point();
                        lats[j] = p.lat();
                        lons[j] = p.lon();
                    }
                }
                (shape, _) => {
                    // the Java cast `(Point) shapes[i]` happens before the test
                    if let Some(shape) = shape {
                        if from_line {
                            shape.as_line();
                        } else {
                            shape.as_point();
                        }
                    }
                    lats[j] = geo_test_util::next_latitude();
                    lons[j] = geo_test_util::next_longitude();
                }
            }
            i += 1;
            j += 1;
        }
        return Ok(Shape::Line(Line::new(lats, lons)?));
    }
    Ok(case.next_line())
}

/// The `randomQueryLine(Object... shapes)` override of
/// `TestXYPointShapeQueries` and `TestXYLineShapeQueries`.
pub fn xy_random_query_line<C: SpatialCase>(
    case: &C,
    shapes: &[Option<Shape>],
    from_line: bool,
) -> Result<Shape> {
    let random = random();
    if random.next_int_bound(100) == 42 {
        let mut max_bound = (shapes.len() as f64 * 0.1).floor() as i32;
        if max_bound < 2 {
            max_bound = shapes.len() as i32;
        }
        let n = RandomNumbers::random_int_between(&*random, 2, max_bound) as usize;
        let mut x = vec![0f32; n];
        let mut y = vec![0f32; n];
        let (mut i, mut j) = (0usize, 0usize);
        while j < x.len() && i < shapes.len() {
            if let Some(shape) = &shapes[i] {
                if from_line {
                    shape.as_xy_line();
                } else {
                    shape.as_xy_point();
                }
            }
            if random.next_boolean() && shapes[i].is_some() {
                let shape = shapes[i].as_ref().expect("checked");
                if from_line {
                    let l = shape.as_xy_line();
                    let v = random.next_int_bound(l.num_points() as i32 - 1) as usize;
                    x[j] = l.get_x_at(v);
                    y[j] = l.get_y_at(v);
                } else {
                    let p = shape.as_xy_point();
                    x[j] = p.get_x();
                    y[j] = p.get_y();
                }
            } else {
                x[j] = shape_test_util::next_float(&*random);
                y[j] = shape_test_util::next_float(&*random);
            }
            i += 1;
            j += 1;
        }
        return Ok(Shape::XYLine(XYLine::new(x, y)?));
    }
    Ok(case.next_line())
}
