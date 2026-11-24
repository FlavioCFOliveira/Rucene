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

//! Port of `ShapeRectRelationTestCase.java` (package
//! `org.apache.lucene.spatial.spatial4j`).
//!
//! * The fields `ctx` (of `RandomizedShapeTestCase`) and `maxRadius` are
//!   [`ShapeRectRelationFixture`], on each subclass's instance type.
//! * The inner class `AbstractRectIntersectionTestHelper` and its four
//!   anonymous subclasses are [`RectHelper`], one per [`HelperShape`].
//! * The `@Rule TestLog testLog` field is declared on each concrete class by
//!   [`shape_rect_relation_class`].

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{
    modifier, Class, ClassBuilder, Constructor, Field, Method, Value,
};
use randomizedtesting::junit::{Rule, Test, TestRule};
use rucene::error::{LuceneError, Result};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::randomized_shape_test::RandomizedShapeTest;
use spatial4j::shape::rect_intersection_test_helper::RectIntersectionTestHelper;
use spatial4j::shape::shape::Shape;
use spatial4j::test_log::TestLog;

use crate::spatial4j::randomized_shape_test_case::randomized_shape_test_case_class;
use crate::spatial_support::random;

/// `RADIANS_PER_DEGREE` (`protected static final`).
pub const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;

/// The fields of `ShapeRectRelationTestCase` (and its base's `ctx`).
pub struct ShapeRectRelationFixture {
    /// `ctx` (`protected SpatialContext ctx; // needs to be set ASAP`).
    pub ctx: Arc<SpatialContext>,
    /// `maxRadius` (`protected int maxRadius = 180`).
    pub max_radius: i32,
}

impl ShapeRectRelationFixture {
    /// `ShapeRectRelationTestCase()`: `super(SpatialContext.GEO)`.
    pub fn new() -> Self {
        Self {
            ctx: SpatialContext::geo(),
            max_radius: 180,
        }
    }
}

/// A concrete subclass of `ShapeRectRelationTestCase`.
pub trait ShapeRectRelationCase: Send + 'static {
    /// The inherited fields.
    fn srr(&self) -> &ShapeRectRelationFixture;
}

/// The abstract class `ShapeRectRelationTestCase extends RandomizedShapeTestCase`.
pub fn shape_rect_relation_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.spatial.spatial4j.ShapeRectRelationTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&randomized_shape_test_case_class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// An instance method of `T`; an `Err` is thrown as its Java exception.
pub fn tm<T: Send + 'static>(name: &str, body: fn(&mut T) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |t: &mut T| {
            if let Err(e) = body(t) {
                rethrow_java(e)
            }
        })
}

/// The concrete class `name`: the `@Rule public final TestLog testLog =
/// TestLog.instance` field, the four inherited tests (`overridden_nightly`
/// re-declares three of them `@Nightly`, as `TestGeo3dShapeWGS84ModelRectRelation`
/// does) and the class's own `methods`.
pub fn shape_rect_relation_class<T: ShapeRectRelationCase>(
    name: &str,
    constructor: Constructor,
    methods: Vec<Method>,
    annotate_inherited: impl Fn(&str, Method) -> Method,
) -> Arc<Class> {
    let mut builder: ClassBuilder = Class::builder(name)
        .extends(&shape_rect_relation_test_case_class())
        .constructor(constructor)
        .field(
            Field::new_instance_any("testLog", "org.locationtech.spatial4j.TestLog", || {
                Value::TestRule(Arc::new(TestLog::rule()) as Arc<dyn TestRule>)
            })
            .modifiers(modifier::PUBLIC | modifier::FINAL)
            .annotate(Rule),
        );
    for (m_name, body) in [
        (
            "testGeoCircleRect",
            test_geo_circle_rect::<T> as fn(&mut T) -> Result<()>,
        ),
        ("testGeoBBoxRect", test_geo_b_box_rect::<T>),
        ("testGeoPolygonRect", test_geo_polygon_rect::<T>),
        ("testGeoPathRect", test_geo_path_rect::<T>),
    ] {
        builder = builder.method(annotate_inherited(
            m_name,
            tm::<T>(m_name, body).annotate(Test::new()),
        ));
    }
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// The anonymous `AbstractRectIntersectionTestHelper` subclasses, by test.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HelperShape {
    /// `testGeoCircleRect`.
    Circle,
    /// `testGeoBBoxRect`.
    BBox,
    /// `testGeoPolygonRect`.
    Polygon,
    /// `testGeoPathRect`.
    Path,
}

/// `AbstractRectIntersectionTestHelper extends RectIntersectionTestHelper<Shape>`
/// (`public abstract class`, inner) and its anonymous subclasses.
pub struct RectHelper {
    base: RandomizedShapeTest,
    shape: HelperShape,
    /// The enclosing instance's `maxRadius`.
    max_radius: i32,
}

impl RectHelper {
    /// `new AbstractRectIntersectionTestHelper(ctx) {…}`.
    pub fn new(ctx: &Arc<SpatialContext>, shape: HelperShape, max_radius: i32) -> Self {
        Self {
            base: RandomizedShapeTest::new(Arc::clone(ctx)),
            shape,
            max_radius,
        }
    }
}

impl RectIntersectionTestHelper for RectHelper {
    fn base(&self) -> &RandomizedShapeTest {
        &self.base
    }

    fn is_random_shape_rectangular(&self) -> bool {
        self.shape == HelperShape::BBox
    }

    // 2 times each -- should be plenty

    fn get_contains_minimum(&self, _laps: i32) -> i32 {
        2
    }

    fn get_intersects_minimum(&self, _laps: i32) -> i32 {
        2
    }

    // producing "within" cases in Geo3D based on our random shapes doesn't happen often. It'd be
    // nice to increase this.
    fn get_within_minimum(&self, _laps: i32) -> i32 {
        match self.shape {
            // Long/thin so lets just find 1.
            HelperShape::Polygon | HelperShape::Path => 1,
            _ => 2,
        }
    }

    fn get_disjoint_minimum(&self, _laps: i32) -> i32 {
        2
    }

    fn get_bounding_minimum(&self, _laps: i32) -> i32 {
        2
    }

    fn generate_random_shape(&self, near_p: &Arc<dyn Point>) -> Result<Arc<dyn Shape>> {
        let ctx = self.base.ctx();
        match self.shape {
            HelperShape::Circle => {
                let circle_radius = self.max_radius - random().next_int_bound(self.max_radius); // no 0-radius
                Ok(ctx
                    .get_shape_factory()
                    .circle_at(Arc::clone(near_p), f64::from(circle_radius))?)
            }
            HelperShape::BBox => {
                let mut upper_right = self.base.random_point()?;
                let mut lower_left = self.base.random_point()?;
                if upper_right.get_y() < lower_left.get_y() {
                    // swap
                    std::mem::swap(&mut upper_right, &mut lower_left);
                }
                Ok(ctx
                    .get_shape_factory()
                    .rect_from_points(&*lower_left, &*upper_right)?)
            }
            HelperShape::Polygon => {
                let center_point = self.base.random_point()?;
                let max_distance = random().next_int_bound(self.max_radius - 20) + 20;
                let point_zone = ctx
                    .get_shape_factory()
                    .circle_at(center_point, f64::from(max_distance))?;
                let vertex_count = random().next_int_bound(3) + 3;
                loop {
                    let mut builder = ctx.get_shape_factory().polygon()?;
                    for _ in 0..vertex_count {
                        let point = self.base.random_point_in_circle(&*point_zone)?;
                        builder.point_xy(point.get_x(), point.get_y())?;
                    }
                    match builder.build() {
                        Ok(shape) => return Ok(shape),
                        Err(e) if e.is_illegal_argument_exception() => {
                            // This is what happens when we create a shape that is invalid.  Although it is
                            // conceivable that there are cases where
                            // the exception is thrown incorrectly, we aren't going to be able to do that in this
                            // random test.
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            HelperShape::Path => {
                let center_point = self.base.random_point()?;
                let max_distance = random().next_int_bound(self.max_radius - 20) + 20;
                let point_zone = ctx
                    .get_shape_factory()
                    .circle_at(center_point, f64::from(max_distance))?;
                let point_count = random().next_int_bound(5) + 1;
                let width =
                    f64::from(random().next_int_bound(89) + 1) * DistanceUtils::DEGREES_TO_RADIANS;
                let mut builder = ctx.get_shape_factory().line_string();
                loop {
                    for _ in 0..point_count {
                        let next_point = self.base.random_point_in_circle(&*point_zone)?;
                        builder.point_xy(next_point.get_x(), next_point.get_y())?;
                    }
                    builder.buffer(width);
                    match builder.build() {
                        Ok(shape) => return Ok(shape),
                        Err(e) if e.is_illegal_argument_exception() => {
                            // This is what happens when we create a shape that is invalid.  Although it is
                            // conceivable that there are cases where
                            // the exception is thrown incorrectly, we aren't going to be able to do that in this
                            // random test.
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }

    fn random_point_in_empty_shape(&self, shape: &Arc<dyn Shape>) -> Result<Arc<dyn Point>> {
        match self.shape {
            HelperShape::Circle | HelperShape::BBox => Ok(shape.get_center()),
            HelperShape::Polygon | HelperShape::Path => Err(LuceneError::unchecked(
                "java.lang.IllegalStateException",
                "unexpected; need to finish test code",
            )),
        }
    }
}

/// `testGeoCircleRect()`.
pub fn test_geo_circle_rect<T: ShapeRectRelationCase>(t: &mut T) -> Result<()> {
    RectHelper::new(&t.srr().ctx, HelperShape::Circle, t.srr().max_radius)
        .test_relate_with_rectangle()
}

/// `testGeoBBoxRect()`.
pub fn test_geo_b_box_rect<T: ShapeRectRelationCase>(t: &mut T) -> Result<()> {
    RectHelper::new(&t.srr().ctx, HelperShape::BBox, t.srr().max_radius)
        .test_relate_with_rectangle()
}

/// `testGeoPolygonRect()`: very slow, and test sources are not here, so no
/// clue how to fix.
pub fn test_geo_polygon_rect<T: ShapeRectRelationCase>(t: &mut T) -> Result<()> {
    RectHelper::new(&t.srr().ctx, HelperShape::Polygon, t.srr().max_radius)
        .test_relate_with_rectangle()
}

/// `testGeoPathRect()`.
pub fn test_geo_path_rect<T: ShapeRectRelationCase>(t: &mut T) -> Result<()> {
    RectHelper::new(&t.srr().ctx, HelperShape::Path, t.srr().max_radius)
        .test_relate_with_rectangle()
}
