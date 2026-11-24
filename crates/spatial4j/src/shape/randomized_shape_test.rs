/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `RandomizedShapeTest.java`, of the Java package
//! `org.locationtech.spatial4j.shape` (spatial4j 0.8 test sources, built with
//! the `test-helpers` feature).

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::math;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::distance::distance_utils::DistanceUtils;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.RandomizedShapeTest`
/// (`public abstract class RandomizedShapeTest extends RandomizedTest`): a
/// base test class with utility methods to help test shapes. The random
/// draws are `RandomizedTest`'s, on the current randomized context.
///
/// Java's subclasses override `randomPointInOrNull`; that override point is
/// the [`RectIntersectionTestHelper`](super::rect_intersection_test_helper::RectIntersectionTestHelper)
/// trait, which reaches this base through `base()`.
#[derive(Clone, Debug)]
pub struct RandomizedShapeTest {
    /// `ctx` (`protected SpatialContext ctx; //needs to be set ASAP`);
    /// `None` stands for `null`.
    pub ctx: Option<Arc<SpatialContext>>,
}

/// `EPS` (`protected static final double`).
pub const EPS: f64 = 10e-9;

/// `DIVISIBLE` (`protected final double`): used to reduce the space of
/// numbers to increase the likelihood that random numbers become
/// equivalent, and thus trigger different code paths. Also makes some random
/// shapes easier to manually examine.
pub const DIVISIBLE: f64 = 2.0; // even coordinates; (not always used)

impl RandomizedShapeTest {
    /// `RandomizedShapeTest()` (`protected`): `ctx` stays `null`.
    pub fn new_without_context() -> Self {
        Self { ctx: None }
    }

    /// `RandomizedShapeTest(SpatialContext ctx)`.
    pub fn new(ctx: Arc<SpatialContext>) -> Self {
        Self { ctx: Some(ctx) }
    }

    /// `ctx`, dereferenced: Java's `NullPointerException` when unset.
    pub fn ctx(&self) -> &Arc<SpatialContext> {
        self.ctx
            .as_ref()
            .unwrap_or_else(|| Throwable::new("java.lang.NullPointerException", "ctx").throw())
    }

    /// `checkShapesImplementEquals(Class<?>[] classes)` (`public static`):
    /// each class must declare `equals` and `hashCode`.
    pub fn check_shapes_implement_equals(classes: &[Arc<Class>]) {
        for clazz in classes {
            let declares = |name: &str| clazz.declared_methods().iter().any(|m| m.name() == name);
            if !declares("equals") {
                Assert::fail(&format!(
                    "Shape needs to define 'equals' : {}",
                    clazz.name()
                ));
            }
            if !declares("hashCode") {
                Assert::fail(&format!(
                    "Shape needs to define 'hashCode' : {}",
                    clazz.name()
                ));
            }
        }
    }

    // These few norm methods normalize the arguments for creating a shape to
    // account for the dateline. Some tests loop past the dateline or have offsets
    // that go past it and it's easier to have them coded that way and correct for
    // it here.  These norm methods should be used when needed, not frivolously.

    /// `normX(double x)`.
    pub fn norm_x(&self, x: f64) -> f64 {
        if self.ctx().is_geo() {
            DistanceUtils::norm_lon_deg(x)
        } else {
            x
        }
    }

    /// `normY(double y)`.
    pub fn norm_y(&self, y: f64) -> f64 {
        if self.ctx().is_geo() {
            DistanceUtils::norm_lat_deg(y)
        } else {
            y
        }
    }

    /// `makeNormRect(double minX, double maxX, double minY, double maxY)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makeRectangle`.
    pub fn make_norm_rect(
        &self,
        mut min_x: f64,
        mut max_x: f64,
        mut min_y: f64,
        mut max_y: f64,
    ) -> Result<Arc<dyn Rectangle>> {
        let ctx = self.ctx();
        if ctx.is_geo() {
            if (max_x - min_x).abs() >= 360.0 {
                min_x = -180.0;
                max_x = 180.0;
            } else {
                min_x = DistanceUtils::norm_lon_deg(min_x);
                max_x = DistanceUtils::norm_lon_deg(max_x);
            }
        } else {
            if max_x < min_x {
                std::mem::swap(&mut min_x, &mut max_x);
            }
            min_x = bound_x(min_x, &*ctx.get_world_bounds());
            max_x = bound_x(max_x, &*ctx.get_world_bounds());
        }
        if max_y < min_y {
            std::mem::swap(&mut min_y, &mut max_y);
        }
        min_y = bound_y(min_y, &*ctx.get_world_bounds());
        max_y = bound_y(max_y, &*ctx.get_world_bounds());
        ctx.make_rectangle(min_x, max_x, min_y, max_y)
    }

    /// `divisible(double v, double divisible)` (`public static`):
    /// `(int) (Math.round(v / divisible) * divisible)`.
    pub fn divisible_by(v: f64, divisible: f64) -> f64 {
        f64::from(((math::round(v / divisible) as f64) * divisible) as i32)
    }

    /// `divisible(double v)`.
    pub fn divisible(&self, v: f64) -> f64 {
        Self::divisible_by(v, DIVISIBLE)
    }

    /// `divisible(Point p)`: `reset()`'s p, and confines to world bounds.
    /// Might not be divisible if the world bound isn't divisible too.
    pub fn divisible_point(&self, p: Arc<dyn Point>) -> Arc<dyn Point> {
        let bounds = self.ctx().get_world_bounds();
        let new_x = bound_x(self.divisible(p.get_x()), &*bounds);
        let new_y = bound_y(self.divisible(p.get_y()), &*bounds);
        p.reset(new_x, new_y);
        p
    }

    /// `assertRelation(SpatialRelation expected, Shape a, Shape b)`.
    pub fn assert_relation(&self, expected: SpatialRelation, a: &dyn Shape, b: &dyn Shape) {
        self.assert_relation_msg(None, expected, a, b);
    }

    /// `assertRelation(String msg, SpatialRelation expected, Shape a, Shape b)`.
    pub fn assert_relation_msg(
        &self,
        msg: Option<&str>,
        expected: SpatialRelation,
        a: &dyn Shape,
        b: &dyn Shape,
    ) {
        self.assert_intersect(msg, expected, a, b);
        // check flipped a & b w/ transpose(), while we're at it
        self.assert_intersect(msg, expected.transpose(), b, a);
    }

    /// `_assertIntersect(String msg, SpatialRelation expected, Shape a, Shape b)`.
    fn assert_intersect(
        &self,
        msg: Option<&str>,
        expected: SpatialRelation,
        a: &dyn Shape,
        b: &dyn Shape,
    ) {
        let sect = a.relate(b);
        if sect == expected {
            return;
        }
        let msg = format!(
            "{}{} intersect {}",
            msg.map_or_else(String::new, |m| format!("{m}\r")),
            a,
            b
        );
        if expected == SpatialRelation::Within || expected == SpatialRelation::Contains {
            if a.as_any().type_id() == b.as_any().type_id() {
                // they are the same shape type
                assert_equals_shape(Some(&msg), a, b);
            } else {
                // they are effectively points or lines that are the same location
                Assert::assert_true_msg(&msg, !a.has_area());
                Assert::assert_true_msg(&msg, !b.has_area());

                let a_bbox = a.get_bounding_box();
                let b_bbox = b.get_bounding_box();
                if a_bbox.get_height() == 0.0
                    && b_bbox.get_height() == 0.0
                    && ((a_bbox.get_max_y() == 90.0 && b_bbox.get_max_y() == 90.0)
                        || (a_bbox.get_min_y() == -90.0 && b_bbox.get_min_y() == -90.0))
                {
                    // == a point at the pole
                } else {
                    assert_equals_shape(Some(&msg), a_bbox.as_shape(), b_bbox.as_shape());
                }
            }
        } else {
            Assert::assert_equals_msg(Some(&msg), expected, sect); // always fails
        }
    }

    /// `assertEqualsRatio(String msg, double expected, double actual)`.
    pub fn assert_equals_ratio(&self, msg: &str, expected: f64, actual: f64) {
        let delta = (actual - expected).abs();
        let base = actual.min(expected);
        let delta_ratio = if base == 0.0 {
            delta
        } else {
            delta.min(delta / base)
        };
        Assert::assert_equals_double_msg(Some(msg), 0.0, delta_ratio, EPS);
    }

    /// `randomIntBetweenDivisible(int start, int end)`.
    pub fn random_int_between_divisible(&self, start: i32, end: i32) -> i32 {
        self.random_int_between_divisible_by(start, end, DIVISIBLE as i32)
    }

    /// `randomIntBetweenDivisible(int start, int end, int divisible)`:
    /// returns a random integer between `[start, end]`. Integers between
    /// must be divisible by the 3rd argument.
    pub fn random_int_between_divisible_by(&self, start: i32, end: i32, divisible: i32) -> i32 {
        // DWS: I tested this
        let divis_start = (f64::from(start + 1) / f64::from(divisible)).ceil() as i32;
        let divis_end = (f64::from(end - 1) / f64::from(divisible)).floor() as i32;
        let divis_range = 0.max(divis_end - divis_start + 1);
        // spatial4j calls `randomInt(int)`, which randomizedtesting deprecates.
        #[allow(deprecated)]
        let r = RandomizedTest::random_int_max(1 + divis_range); // remember that '0' is counted
        if r == 0 {
            return start;
        }
        if r == 1 {
            return end;
        }
        (r - 2 + divis_start) * divisible
    }

    /// `randomRectangle(Point nearP)`; `None` stands for `null`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shapes made.
    pub fn random_rectangle_near(
        &self,
        near_p: Option<Arc<dyn Point>>,
    ) -> Result<Arc<dyn Rectangle>> {
        let bounds = self.ctx().get_world_bounds();
        let near_p = match near_p {
            Some(p) => p,
            None => self.random_point_in_rect(&*bounds)?,
        };

        let world_x_range = [bounds.get_min_x(), bounds.get_max_x()];
        let world_y_range = [bounds.get_min_y(), bounds.get_max_y()];
        let x_near = if RandomizedTest::rarely() {
            0.0
        } else {
            near_p.get_x()
        };
        let x_range = random_range(x_near, &world_x_range);
        let y_near = if RandomizedTest::rarely() {
            0.0
        } else {
            near_p.get_y()
        };
        let y_range = random_range(y_near, &world_y_range);

        self.make_norm_rect(
            self.divisible(x_range[0]),
            self.divisible(x_range[1]),
            self.divisible(y_range[0]),
            self.divisible(y_range[1]),
        )
    }

    /// `randomGaussianZeroTo(double max)` (`private`; unused by spatial4j).
    #[allow(dead_code)] // Java's private method has no caller either.
    fn random_gaussian_zero_to(max: f64) -> f64 {
        if max == 0.0 {
            return max;
        }
        debug_assert!(max > 0.0);
        loop {
            let r = RandomizedTest::random_gaussian().abs() * (max * 0.50);
            if r <= max {
                return r;
            }
        }
    }

    /// `randomRectangle(int divisible)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `makeNormRect`.
    pub fn random_rectangle_divisible(&self, divisible: i32) -> Result<Arc<dyn Rectangle>> {
        let mut r_x = f64::from(self.random_int_between_divisible_by(-180, 180, divisible));
        let r_w = f64::from(self.random_int_between_divisible_by(0, 360, divisible));
        let r_y1 = f64::from(self.random_int_between_divisible_by(-90, 90, divisible));
        let r_y2 = f64::from(self.random_int_between_divisible_by(-90, 90, divisible));
        let r_ymin = r_y1.min(r_y2);
        let r_ymax = r_y1.max(r_y2);
        if r_w > 0.0 && r_x == 180.0 {
            r_x = -180.0;
        }
        self.make_norm_rect(r_x, r_x + r_w, r_ymin, r_ymax)
    }

    /// `randomPoint()`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makePoint`.
    pub fn random_point(&self) -> Result<Arc<dyn Point>> {
        self.random_point_in_rect(&*self.ctx().get_world_bounds())
    }

    /// `randomPointIn(Circle c)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `pointOnBearing`.
    pub fn random_point_in_circle(&self, c: &dyn Circle) -> Result<Arc<dyn Point>> {
        let d = c.get_radius() * RandomizedTest::random_double();
        let angle_deg = 360.0 * RandomizedTest::random_double();
        let ctx = self.ctx();
        let p = ctx
            .get_dist_calc()
            .point_on_bearing(&c.get_center(), d, angle_deg, ctx, None)?;
        Assert::assert_equals(SpatialRelation::Contains, c.relate(p.as_shape()));
        Ok(p)
    }

    /// `randomPointIn(Rectangle r)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makePoint`.
    pub fn random_point_in_rect(&self, r: &dyn Rectangle) -> Result<Arc<dyn Point>> {
        let mut x = r.get_min_x() + RandomizedTest::random_double() * r.get_width();
        let mut y = r.get_min_y() + RandomizedTest::random_double() * r.get_height();
        x = self.norm_x(x);
        y = self.norm_y(y);
        let p = self.ctx().make_point(x, y)?;
        Assert::assert_equals(SpatialRelation::Contains, r.relate(p.as_shape()));
        Ok(p)
    }

    /// `randomPointIn(Shape shape)`.
    ///
    /// # Errors
    ///
    /// Java's `UnsupportedOperationException` (`Need area to define
    /// shape!`) for a shape without area; the errors of `randomPointIn`.
    pub fn random_point_in_shape(&self, shape: &dyn Shape) -> Result<Arc<dyn Point>> {
        if !shape.has_area() {
            // or try the center?
            return Err(unsupported("Need area to define shape!"));
        }
        let bbox = shape.get_bounding_box();
        loop {
            let p = self.random_point_in_rect(&*bbox)?;
            if shape.relate(p.as_shape()).intersects() {
                return Ok(p);
            }
        }
    }

    /// `randomPointInOrNull(Shape shape)`; `None` stands for `null`.
    ///
    /// # Errors
    ///
    /// As [`random_point_in_shape`](Self::random_point_in_shape).
    pub fn random_point_in_or_null(&self, shape: &dyn Shape) -> Result<Option<Arc<dyn Point>>> {
        if !shape.has_area() {
            // or try the center?
            return Err(unsupported("Need area to define shape!"));
        }
        let bbox = shape.get_bounding_box();
        for _ in 0..1000 {
            let p = self.random_point_in_rect(&*bbox)?;
            if shape.relate(p.as_shape()).intersects() {
                return Ok(Some(p));
            }
        }
        Ok(None) // tried too many times and failed
    }

    /// `assertGreaterOrEqual(double left, double right, double delta)`
    /// (`public static`): tests that `left >= right`, but may be less if
    /// within some tolerance.
    pub fn assert_greater_or_equal(left: f64, right: f64, delta: f64) {
        if left > right {
            return;
        }
        Assert::assert_equals_double_msg(None, left, right, delta);
    }
}

/// `boundX(double i, Rectangle bounds)` (package-private `static`).
pub fn bound_x(i: f64, bounds: &dyn Rectangle) -> f64 {
    bound(i, bounds.get_min_x(), bounds.get_max_x())
}

/// `boundY(double i, Rectangle bounds)` (package-private `static`).
pub fn bound_y(i: f64, bounds: &dyn Rectangle) -> f64 {
    bound(i, bounds.get_min_y(), bounds.get_max_y())
}

/// `bound(double i, double min, double max)` (package-private `static`).
pub fn bound(i: f64, min: f64, max: f64) -> f64 {
    if i < min {
        return min;
    }
    if i > max {
        return max;
    }
    i
}

/// `randomRange(double near, double[] bounds)` (`private`).
fn random_range(near: f64, bounds: &[f64; 2]) -> [f64; 2] {
    let bounds_width = bounds[1] - bounds[0];
    let mid = near + RandomizedTest::random_gaussian() * bounds_width / 6.0;
    let width = RandomizedTest::random_gaussian().abs() * bounds_width / 6.0; // 1/3rd
    [mid - width / 2.0, mid + width / 2.0]
}

/// Java's `UnsupportedOperationException(message)`.
fn unsupported(message: &str) -> rucene::error::LuceneError {
    rucene::error::LuceneError::unchecked("java.lang.UnsupportedOperationException", message)
}

/// JUnit's `assertEquals(String message, Object expected, Object actual)`
/// over two shapes: `expected.equals(actual)`, failing with
/// `[message ]expected:<…> but was:<…>`.
pub fn assert_equals_shape(message: Option<&str>, expected: &dyn Shape, actual: &dyn Shape) {
    if !expected.equals(actual) {
        let prefix = match message {
            Some(m) if !m.is_empty() => format!("{m} "),
            _ => String::new(),
        };
        let (e, a) = (expected.to_string(), actual.to_string());
        if e == a {
            // JUnit's failNotEquals for equal strings: the class names.
            Assert::fail(&format!(
                "{prefix}expected: {}<{e}> but was: {}<{a}>",
                expected.java_class_name(),
                actual.java_class_name()
            ));
        }
        Assert::fail(&format!("{prefix}expected:<{e}> but was:<{a}>"));
    }
}
