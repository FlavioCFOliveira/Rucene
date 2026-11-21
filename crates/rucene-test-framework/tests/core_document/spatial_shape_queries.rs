//! Ports of the concrete spatial query test classes of
//! `lucene/core/src/test/org/apache/lucene/document/` that extend the bases of
//! [`crate::base_spatial_test_case`]: the 13 `TestLatLon*Queries`/`*DVQueries`
//! and the 9 `TestXY*Queries`/`*DVQueries` classes.
//!
//! Each module is one Java class: its `SpatialCase` impl holds the class's
//! overrides, and its `class_def` lists the methods it inherits.
//!
//! Divergence (structure only): a Java constructor sets `VALIDATOR =
//! getValidator()` once the object exists; `new_instance` builds the object
//! with a placeholder validator and then replaces it with `get_validator()`,
//! which reads no other state.

use std::cell::RefCell;

use rucene::document::geo_fields::{LatLonDocValuesField, LatLonPoint};
use rucene::document::{LatLonShape, XYShape};
use rucene::error::Result;
use rucene::index::IndexableField;

use crate::base_spatial_test_case::{
    Encoder, Family, LatLonLineValidator, LatLonPointValidator, LatLonPolygonValidator,
    MultiContains, MultiValidator, Shape, ShapeType, SpatialCase, Validator, XYLineValidator,
    XYPointValidator, XYPolygonValidator, FIELD_NAME,
};
use crate::core_support::random;

/// Boxes each field of `fields`.
fn boxed<F: IndexableField + 'static>(fields: Vec<F>) -> Vec<Box<dyn IndexableField>> {
    fields
        .into_iter()
        .map(|f| Box::new(f) as Box<dyn IndexableField>)
        .collect()
}

/// Declares one concrete test class.
macro_rules! spatial_class {
    (
        $(#[$m:meta])*
        mod $module:ident, $ty:ident, $java:literal, $family:expr, $shape_type:expr,
        fields: |$this:ident, $field:ident, $shape:ident| $fields:expr,
        validator: |$enc:ident| $validator:expr,
        class: |$name:ident, $methods:ident| $class:expr
        $(, overrides: { $($over:tt)* })?
    ) => {
        $(#[$m])*
        pub mod $module {
            use super::*;

            /// The Java class.
            pub struct $ty {
                validator: RefCell<Box<dyn Validator>>,
            }

            impl SpatialCase for $ty {
                fn new_instance() -> Self {
                    let this = Self {
                        validator: RefCell::new(Box::new(LatLonPointValidator::new(Encoder::LatLon))),
                    };
                    let validator = this.get_validator();
                    *this.validator.borrow_mut() = validator;
                    this
                }

                fn family(&self) -> Family {
                    $family
                }

                fn get_shape_type(&self) -> ShapeType {
                    $shape_type
                }

                fn create_indexable_fields(
                    &self,
                    $field: &str,
                    $shape: &Shape,
                ) -> Result<Vec<Box<dyn IndexableField>>> {
                    let $this = self;
                    let _ = &$this;
                    let _ = &$field;
                    $fields
                }

                fn get_validator(&self) -> Box<dyn Validator> {
                    let $enc = self.encoder();
                    $validator
                }

                fn validator(&self) -> &RefCell<Box<dyn Validator>> {
                    &self.validator
                }

                $($($over)*)?
            }

            /// The declaration of the Java class.
            pub fn class_def() -> std::sync::Arc<randomizedtesting::jdk::reflect::Class> {
                let $name = $java;
                let $methods = crate::base_spatial_test_case::base_methods::<$ty>();
                $class
            }
        }
    };
}

/// `random().nextInt(4) + 1` shapes of one generator.
fn multi(mut next: impl FnMut() -> Result<Shape>) -> Result<Shape> {
    let n = random().next_int_bound(4) + 1;
    let mut shapes = Vec::with_capacity(n as usize);
    for _ in 0..n {
        shapes.push(next()?);
    }
    Ok(Shape::Array(shapes))
}

spatial_class!(
    /// Port of `TestLatLonPointShapeQueries`: random bounding box, line, and
    /// polygon query tests for random generated `latitude, longitude` points.
    mod test_lat_lon_point_shape_queries, TestLatLonPointShapeQueries,
    "org.apache.lucene.document.TestLatLonPointShapeQueries",
    Family::LatLonShape, ShapeType::Point,
    fields: |this, field, point| {
        let p = point.as_point();
        Ok(boxed(LatLonShape::create_indexable_fields_point(field, p.lat(), p.lon())?))
    },
    validator: |encoder| Box::new(LatLonPointValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn random_query_line(&self, shapes: &[Option<Shape>]) -> Result<Shape> {
            crate::base_spatial_test_case::lat_lon_random_query_line(self, shapes, false)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonLineShapeQueries`: random bounding box, line, and
    /// polygon query tests for random generated `Line` types.
    ///
    /// Java's `@SuppressWarnings("SimpleText")` is a compiler annotation with
    /// no runtime effect, so it is not declared.
    mod test_lat_lon_line_shape_queries, TestLatLonLineShapeQueries,
    "org.apache.lucene.document.TestLatLonLineShapeQueries",
    Family::LatLonShape, ShapeType::Line,
    fields: |this, field, line| {
        Ok(boxed(LatLonShape::create_indexable_fields_line(field, line.as_line())?))
    },
    validator: |encoder| Box::new(LatLonLineValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn random_query_line(&self, shapes: &[Option<Shape>]) -> Result<Shape> {
            crate::base_spatial_test_case::lat_lon_random_query_line(self, shapes, true)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonPolygonShapeQueries`: random bounding box, line, and
    /// polygon query tests for random indexed `Polygon` types.
    mod test_lat_lon_polygon_shape_queries, TestLatLonPolygonShapeQueries,
    "org.apache.lucene.document.TestLatLonPolygonShapeQueries",
    Family::LatLonShape, ShapeType::Polygon,
    fields: |this, field, polygon| {
        Ok(boxed(LatLonShape::create_indexable_fields_polygon(field, polygon.as_polygon())?))
    },
    validator: |encoder| Box::new(LatLonPolygonValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 25000)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonMultiPointShapeQueries`: random bounding box, line,
    /// and polygon query tests for random indexed arrays of `latitude,
    /// longitude` points.
    mod test_lat_lon_multi_point_shape_queries, TestLatLonMultiPointShapeQueries,
    "org.apache.lucene.document.TestLatLonMultiPointShapeQueries",
    Family::LatLonShape, ShapeType::Point,
    fields: |this, name, o| {
        let mut all_fields = Vec::new();
        for point in o.as_array() {
            let point = point.as_point();
            all_fields.extend(boxed(LatLonShape::create_indexable_fields_point(
                name,
                point.lat(),
                point.lon(),
            )?));
        }
        Ok(all_fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(LatLonPointValidator::new(encoder)),
        MultiContains::Loop,
    )),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            multi(|| ShapeType::Point.next_lat_lon_shape())
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonMultiLineShapeQueries`: random bounding box, line,
    /// and polygon query tests for random indexed arrays of `Line` types.
    mod test_lat_lon_multi_line_shape_queries, TestLatLonMultiLineShapeQueries,
    "org.apache.lucene.document.TestLatLonMultiLineShapeQueries",
    Family::LatLonShape, ShapeType::Line,
    fields: |this, name, o| {
        let mut all_fields = Vec::new();
        for line in o.as_array() {
            all_fields.extend(boxed(LatLonShape::create_indexable_fields_line(
                name,
                line.as_line(),
            )?));
        }
        Ok(all_fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(LatLonLineValidator::new(encoder)),
        MultiContains::Loop,
    )),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            multi(|| Ok(self.next_line()))
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonMultiPolygonShapeQueries`: random bounding box,
    /// line, and polygon query tests for random indexed arrays of `Polygon`
    /// types.
    mod test_lat_lon_multi_polygon_shape_queries, TestLatLonMultiPolygonShapeQueries,
    "org.apache.lucene.document.TestLatLonMultiPolygonShapeQueries",
    Family::LatLonShape, ShapeType::Polygon,
    fields: |this, name, o| {
        let mut all_fields = Vec::new();
        for polygon in o.as_array() {
            all_fields.extend(boxed(LatLonShape::create_indexable_fields_polygon(
                name,
                polygon.as_polygon(),
            )?));
        }
        Ok(all_fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(LatLonPolygonValidator::new(encoder)),
        MultiContains::LatLonPolygons,
    )),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            let n = random().next_int_bound(4) + 1;
            let mut polygons: Vec<Option<rucene::geo::geometry::Polygon>> = vec![None; n as usize];
            for i in 0..n as usize {
                let mut repetitions = 0;
                loop {
                    let p = self.get_shape_type().next_lat_lon_shape()?.as_polygon().clone();
                    if self.is_disjoint(&polygons, &p) {
                        polygons[i] = Some(p);
                        break;
                    }
                    repetitions += 1;
                    if repetitions > 50 {
                        return self.next_shape();
                    }
                }
            }
            Ok(Shape::Array(
                polygons
                    .into_iter()
                    .map(|p| Shape::Polygon(p.expect("filled")))
                    .collect(),
            ))
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

impl test_lat_lon_multi_polygon_shape_queries::TestLatLonMultiPolygonShapeQueries {
    /// The private `isDisjoint(Polygon[] polygons, Polygon check)`.
    fn is_disjoint(
        &self,
        polygons: &[Option<rucene::geo::geometry::Polygon>],
        check: &rucene::geo::geometry::Polygon,
    ) -> bool {
        let e = self.get_encoder();
        for polygon in polygons.iter().flatten() {
            if e.quantize_y(polygon.min_lat()) > e.quantize_y(check.max_lat())
                || e.quantize_y(polygon.max_lat()) < e.quantize_y(check.min_lat())
                || e.quantize_x(polygon.min_lon()) > e.quantize_x(check.max_lon())
                || e.quantize_x(polygon.max_lon()) < e.quantize_x(check.min_lon())
            {
                continue;
            }
            return false;
        }
        true
    }
}

spatial_class!(
    /// Port of `TestLatLonPointPointQueries`: random bounding box, line, and
    /// polygon query tests for random indexed arrays of `latitude, longitude`
    /// points.
    ///
    /// Java's nested `PointValidator` is never used (the class uses
    /// `TestLatLonPointShapeQueries.PointValidator`), and has the same body.
    mod test_lat_lon_point_point_queries, TestLatLonPointPointQueries,
    "org.apache.lucene.document.TestLatLonPointPointQueries",
    Family::LatLonPoint, ShapeType::Point,
    fields: |this, name, o| {
        let point = o.as_point();
        Ok(vec![Box::new(LatLonPoint::new(FIELD_NAME, point.lat(), point.lon())?)
            as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(LatLonPointValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonMultiPointPointQueries`: random bounding box, line,
    /// and polygon query tests for random indexed arrays of `latitude,
    /// longitude` points.
    mod test_lat_lon_multi_point_point_queries, TestLatLonMultiPointPointQueries,
    "org.apache.lucene.document.TestLatLonMultiPointPointQueries",
    Family::LatLonPoint, ShapeType::Point,
    fields: |this, name, o| {
        let mut fields = Vec::new();
        for point in o.as_array() {
            let point = point.as_point();
            fields.push(Box::new(LatLonPoint::new(FIELD_NAME, point.lat(), point.lon())?)
                as Box<dyn IndexableField>);
        }
        Ok(fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(LatLonPointValidator::new(encoder)),
        MultiContains::Loop,
    )),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            multi(|| ShapeType::Point.next_lat_lon_shape())
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonDocValuesPointPointQueries`: random bounding box,
    /// line, and polygon query tests for random indexed arrays of `latitude,
    /// longitude` points.
    ///
    /// Java's nested `PointValidator` is never used, and has the same body as
    /// `TestLatLonPointShapeQueries.PointValidator`.
    mod test_lat_lon_doc_values_point_point_queries, TestLatLonDocValuesPointPointQueries,
    "org.apache.lucene.document.TestLatLonDocValuesPointPointQueries",
    Family::LatLonDocValue, ShapeType::Point,
    fields: |this, name, o| {
        let point = o.as_point();
        Ok(vec![Box::new(LatLonDocValuesField::new(FIELD_NAME, point.lat(), point.lon())?)
            as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(LatLonPointValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonDocValuesMultiPointPointQueries`: random bounding
    /// box, line, and polygon query tests for random indexed arrays of
    /// `latitude, longitude` points.
    mod test_lat_lon_doc_values_multi_point_point_queries,
    TestLatLonDocValuesMultiPointPointQueries,
    "org.apache.lucene.document.TestLatLonDocValuesMultiPointPointQueries",
    Family::LatLonDocValue, ShapeType::Point,
    fields: |this, name, o| {
        let mut fields = Vec::new();
        for point in o.as_array() {
            let point = point.as_point();
            fields.push(Box::new(LatLonDocValuesField::new(FIELD_NAME, point.lat(), point.lon())?)
                as Box<dyn IndexableField>);
        }
        Ok(fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(LatLonPointValidator::new(encoder)),
        MultiContains::Loop,
    )),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            multi(|| ShapeType::Point.next_lat_lon_shape())
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestLatLonLineShapeDVQueries`: tests queries over geographic
    /// line shape doc values.
    mod test_lat_lon_line_shape_dv_queries, TestLatLonLineShapeDVQueries,
    "org.apache.lucene.document.TestLatLonLineShapeDVQueries",
    Family::LatLonShapeDocValue, ShapeType::Line,
    fields: |this, field, shape| {
        Ok(vec![Box::new(LatLonShape::create_doc_value_field_line(FIELD_NAME, shape.as_line())?)
            as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(LatLonLineValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods)
);

spatial_class!(
    /// Port of `TestLatLonPointShapeDVQueries`: tests queries over geographic
    /// point shape doc values.
    mod test_lat_lon_point_shape_dv_queries, TestLatLonPointShapeDVQueries,
    "org.apache.lucene.document.TestLatLonPointShapeDVQueries",
    Family::LatLonShapeDocValue, ShapeType::Point,
    fields: |this, field, shape| {
        let point = shape.as_point();
        Ok(vec![Box::new(LatLonShape::create_doc_value_field_point(
            FIELD_NAME,
            point.lat(),
            point.lon(),
        )?) as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(LatLonPointValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods)
);

spatial_class!(
    /// Port of `TestLatLonPolygonShapeDVQueries`: test queries over polygon
    /// `ShapeDocValues`.
    mod test_lat_lon_polygon_shape_dv_queries, TestLatLonPolygonShapeDVQueries,
    "org.apache.lucene.document.TestLatLonPolygonShapeDVQueries",
    Family::LatLonShapeDocValue, ShapeType::Polygon,
    fields: |this, field, shape| {
        Ok(vec![Box::new(LatLonShape::create_doc_value_field_polygon(
            FIELD_NAME,
            shape.as_polygon(),
        )?) as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(LatLonPolygonValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods)
);

spatial_class!(
    /// Port of `TestXYPointShapeQueries`: random cartesian bounding box, line,
    /// and polygon query tests for random generated `x, y` points.
    mod test_xy_point_shape_queries, TestXYPointShapeQueries,
    "org.apache.lucene.document.TestXYPointShapeQueries",
    Family::XYShape, ShapeType::Point,
    fields: |this, field, point| {
        let p = point.as_xy_point();
        Ok(boxed(XYShape::create_indexable_fields_point(field, p.get_x(), p.get_y())?))
    },
    validator: |encoder| Box::new(XYPointValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn random_query_line(&self, shapes: &[Option<Shape>]) -> Result<Shape> {
            crate::base_spatial_test_case::xy_random_query_line(self, shapes, false)
        }
    }
);

spatial_class!(
    /// Port of `TestXYLineShapeQueries`: random cartesian bounding box, line,
    /// and polygon query tests for random indexed `XYLine` types.
    mod test_xy_line_shape_queries, TestXYLineShapeQueries,
    "org.apache.lucene.document.TestXYLineShapeQueries",
    Family::XYShape, ShapeType::Line,
    fields: |this, field, line| {
        Ok(boxed(XYShape::create_indexable_fields_line(field, line.as_xy_line())?))
    },
    validator: |encoder| Box::new(XYLineValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn random_query_line(&self, shapes: &[Option<Shape>]) -> Result<Shape> {
            crate::base_spatial_test_case::xy_random_query_line(self, shapes, true)
        }
    }
);

spatial_class!(
    /// Port of `TestXYPolygonShapeQueries`: random cartesian bounding box,
    /// line, and polygon query tests for random indexed `XYPolygon` types.
    mod test_xy_polygon_shape_queries, TestXYPolygonShapeQueries,
    "org.apache.lucene.document.TestXYPolygonShapeQueries",
    Family::XYShape, ShapeType::Polygon,
    fields: |this, field, polygon| {
        Ok(boxed(XYShape::create_indexable_fields_polygon(field, polygon.as_xy_polygon())?))
    },
    validator: |encoder| Box::new(XYPolygonValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 25000)
        }
    }
);

spatial_class!(
    /// Port of `TestXYMultiPointShapeQueries`: random cartesian bounding box,
    /// line, and polygon query tests for random indexed arrays of `x, y`
    /// points.
    mod test_xy_multi_point_shape_queries, TestXYMultiPointShapeQueries,
    "org.apache.lucene.document.TestXYMultiPointShapeQueries",
    Family::XYShape, ShapeType::Point,
    fields: |this, name, o| {
        let mut all_fields = Vec::new();
        for point in o.as_array() {
            let point = point.as_xy_point();
            all_fields.extend(boxed(XYShape::create_indexable_fields_point(
                name,
                point.get_x(),
                point.get_y(),
            )?));
        }
        Ok(all_fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(XYPointValidator::new(encoder)),
        MultiContains::Loop,
    )),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            multi(|| {
                Ok(Shape::XYPoint(
                    rucene_test_framework::geo::shape_test_util::next_xy_point(),
                ))
            })
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestXYMultiLineShapeQueries`: random cartesian bounding box,
    /// line, and polygon query tests for random indexed arrays of cartesian
    /// `Line` types.
    mod test_xy_multi_line_shape_queries, TestXYMultiLineShapeQueries,
    "org.apache.lucene.document.TestXYMultiLineShapeQueries",
    Family::XYShape, ShapeType::Line,
    fields: |this, name, o| {
        let mut all_fields = Vec::new();
        for line in o.as_array() {
            all_fields.extend(boxed(XYShape::create_indexable_fields_line(
                name,
                line.as_xy_line(),
            )?));
        }
        Ok(all_fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(XYLineValidator::new(encoder)),
        MultiContains::Loop,
    )),
    class: |name, methods| crate::core_support::class(name, methods),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            multi(|| Ok(self.next_line()))
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestXYMultiPolygonShapeQueries`
    /// (`@LuceneTestCase.SuppressCodecs("SimpleText")`): random cartesian
    /// bounding box, line, and polygon query tests for random indexed arrays
    /// of cartesian polygon types.
    mod test_xy_multi_polygon_shape_queries, TestXYMultiPolygonShapeQueries,
    "org.apache.lucene.document.TestXYMultiPolygonShapeQueries",
    Family::XYShape, ShapeType::Polygon,
    fields: |this, name, o| {
        let mut all_fields = Vec::new();
        for polygon in o.as_array() {
            all_fields.extend(boxed(XYShape::create_indexable_fields_polygon(
                name,
                polygon.as_xy_polygon(),
            )?));
        }
        Ok(all_fields)
    },
    validator: |encoder| Box::new(MultiValidator::new(
        encoder,
        Box::new(XYPolygonValidator::new(encoder)),
        MultiContains::XYPolygons,
    )),
    class: |name, methods| crate::core_support::class_annotated(name, methods, |b| {
        b.annotate(rucene_test_framework::util::lucene_test_case::SuppressCodecs {
            value: vec!["SimpleText".to_string()],
        })
    }),
    overrides: {
        fn next_shape(&self) -> Result<Shape> {
            multi(|| self.get_shape_type().next_xy_shape())
        }
        fn test_random_big(&self) -> Result<()> {
            crate::base_spatial_test_case::do_test_random(self, 10000)
        }
    }
);

spatial_class!(
    /// Port of `TestXYLineShapeDVQueries`: tests queries over cartesian line
    /// shape doc values.
    mod test_xy_line_shape_dv_queries, TestXYLineShapeDVQueries,
    "org.apache.lucene.document.TestXYLineShapeDVQueries",
    Family::XYShapeDocValue, ShapeType::Line,
    fields: |this, field, shape| {
        Ok(vec![Box::new(XYShape::create_doc_value_field_line(FIELD_NAME, shape.as_xy_line())?)
            as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(XYLineValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods)
);

spatial_class!(
    /// Port of `TestXYPointShapeDVQueries`: tests queries over cartesian point
    /// shape doc values.
    mod test_xy_point_shape_dv_queries, TestXYPointShapeDVQueries,
    "org.apache.lucene.document.TestXYPointShapeDVQueries",
    Family::XYShapeDocValue, ShapeType::Point,
    fields: |this, field, shape| {
        let point = shape.as_xy_point();
        Ok(vec![Box::new(XYShape::create_doc_value_field_point(
            FIELD_NAME,
            point.get_x(),
            point.get_y(),
        )?) as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(XYPointValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods)
);

spatial_class!(
    /// Port of `TestXYPolygonShapeDVQueries`: test queries over `XYPolygon`
    /// doc value geometries.
    mod test_xy_polygon_shape_dv_queries, TestXYPolygonShapeDVQueries,
    "org.apache.lucene.document.TestXYPolygonShapeDVQueries",
    Family::XYShapeDocValue, ShapeType::Polygon,
    fields: |this, field, shape| {
        Ok(vec![Box::new(XYShape::create_doc_value_field_polygon(
            FIELD_NAME,
            shape.as_xy_polygon(),
        )?) as Box<dyn IndexableField>])
    },
    validator: |encoder| Box::new(XYPolygonValidator::new(encoder)),
    class: |name, methods| crate::core_support::class(name, methods)
);
