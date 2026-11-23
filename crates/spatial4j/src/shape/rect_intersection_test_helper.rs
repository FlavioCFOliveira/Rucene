/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `RectIntersectionTestHelper.java`, of the Java
//! package `org.locationtech.spatial4j.shape` (spatial4j 0.8 test sources,
//! built with the `test-helpers` feature).

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;

use crate::shape::point::Point;
use crate::shape::r#impl::inf_buf_line::InfBufLine;
use crate::shape::r#impl::point_impl::PointImpl;
use crate::shape::randomized_shape_test::RandomizedShapeTest;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;
use crate::test_log::TestLog;

/// The payload of a panic caught by
/// [`test_relate_with_rectangle`](RectIntersectionTestHelper::test_relate_with_rectangle):
/// Java's caught `AssertionError`.
pub type AssertionPayload = Box<dyn std::any::Any + Send>;

/// Equivalent to `org.locationtech.spatial4j.shape.RectIntersectionTestHelper<S
/// extends Shape>` (`public abstract class … extends RandomizedShapeTest`).
///
/// A subclass is a type implementing this trait: the abstract methods are
/// the required ones, the overridable ones are provided, and
/// [`base`](Self::base) is the `RandomizedShapeTest` state it extends.
///
/// The type parameter `S` is erased to `Shape`, as Java's erasure does at run
/// time: its casts `(S) shape` are unchecked, and every use of the class in
/// Lucene 10.5.0 binds `S` to `Shape`.
pub trait RectIntersectionTestHelper {
    /// The `RandomizedShapeTest` this helper extends (its `ctx`).
    fn base(&self) -> &RandomizedShapeTest;

    /// `isRandomShapeRectangular()`: override to return true if
    /// `generateRandomShape` is essentially a Rectangle.
    fn is_random_shape_rectangular(&self) -> bool {
        false
    }

    /// `generateRandomShape(Point nearP)` (`protected abstract`).
    ///
    /// # Errors
    ///
    /// The errors of the shapes made.
    fn generate_random_shape(&self, near_p: &Arc<dyn Point>) -> Result<Arc<dyn Shape>>;

    /// `randomPointInEmptyShape(S shape)` (`protected abstract`): shape has
    /// no area; return a point in it.
    ///
    /// # Errors
    ///
    /// The errors of the shapes made.
    fn random_point_in_empty_shape(&self, shape: &Arc<dyn Shape>) -> Result<Arc<dyn Point>>;

    // Minimum distribution of relationships

    // Each shape has different characteristics, so we don't expect (for instance) shapes that
    // are likely to be long and thin to contain as many rectangles as those that
    // short and fat.

    /// `getContainsMinimum(int laps)`.
    fn get_contains_minimum(&self, laps: i32) -> i32 {
        laps / 1000
    }

    /// `getIntersectsMinimum(int laps)`.
    fn get_intersects_minimum(&self, laps: i32) -> i32 {
        laps / 1000
    }

    /// `getWithinMinimum(int laps)`.
    fn get_within_minimum(&self, laps: i32) -> i32 {
        laps / 1000
    }

    /// `getDisjointMinimum(int laps)`.
    fn get_disjoint_minimum(&self, laps: i32) -> i32 {
        laps / 1000
    }

    /// `getBoundingMinimum(int laps)`.
    fn get_bounding_minimum(&self, laps: i32) -> i32 {
        laps / 1000
    }

    /// `randomPointInOrNull(Shape shape)` (the override of
    /// `RandomizedShapeTest`'s).
    ///
    /// # Errors
    ///
    /// The errors of `randomPointInEmptyShape` and of the base method.
    fn random_point_in_or_null(&self, shape: &Arc<dyn Shape>) -> Result<Option<Arc<dyn Point>>> {
        if !shape.has_area() {
            let pt = self.random_point_in_empty_shape(shape)?;
            if cfg!(debug_assertions) && !shape.relate(pt.as_shape()).intersects() {
                Assert::fail("faulty randomPointInEmptyShape");
            }
            return Ok(Some(pt));
        }
        self.base().random_point_in_or_null(&**shape)
    }

    /// `testRelateWithRectangle()`.
    ///
    /// # Errors
    ///
    /// The errors of the shapes made.
    fn test_relate_with_rectangle(&self) -> Result<()> {
        let base = self.base();
        let ctx = base.ctx();
        // counters for the different intersection cases
        let (mut i_c, mut i_i, mut i_w, mut i_d, mut i_bbox_d) = (0, 0, 0, 0, 0);
        let mut laps = 0;
        let minlaps = RandomizedTest::scaled_random_int_between(20000, 200000);
        while i_c < self.get_contains_minimum(minlaps)
            || i_i < self.get_intersects_minimum(minlaps)
            || i_w < self.get_within_minimum(minlaps)
            || (!self.is_random_shape_rectangular() && i_d < self.get_disjoint_minimum(minlaps))
            || i_bbox_d < self.get_bounding_minimum(minlaps)
        {
            laps += 1;

            TestLog::clear();

            if laps > minlaps {
                Assert::fail(&format!(
                    "Did not find enough contains/within/intersection/disjoint/bounds cases in a reasonable number of random attempts. CWIDbD: {}({}),{}({}),{}({}),{}({}),{}({})  Laps exceeded {}",
                    i_c,
                    self.get_contains_minimum(minlaps),
                    i_w,
                    self.get_within_minimum(minlaps),
                    i_i,
                    self.get_intersects_minimum(minlaps),
                    i_d,
                    self.get_disjoint_minimum(minlaps),
                    i_bbox_d,
                    self.get_bounding_minimum(minlaps),
                    minlaps
                ));
            }

            let near_p = base.random_point_in_rect(&*ctx.get_world_bounds())?;

            let s = self.generate_random_shape(&near_p)?;

            let r = base.random_rectangle_near(Some(s.get_bounding_box().get_center()))?;

            let ic = s.relate(r.as_shape());

            TestLog::log(
                "S-R Rel: {}, Shape {}, Rectangle {}",
                vec![Arc::new(ic), Arc::new(Arc::clone(&s)), Arc::new(Arc::clone(&r))],
            );

            if ic != SpatialRelation::Disjoint {
                Assert::assert_true_msg(
                    "if not disjoint then the shape's bbox shouldn't be disjoint",
                    s.get_bounding_box().relate(r.as_shape()).intersects(),
                );
            }

            let body = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
                let mut max_tries = RandomizedTest::scaled_random_int_between(10, 100);
                match ic {
                    SpatialRelation::Contains => {
                        i_c += 1;
                        for _ in 0..max_tries {
                            let p = base.random_point_in_rect(&*r)?;
                            base.assert_relation_msg(
                                None,
                                SpatialRelation::Contains,
                                &*s,
                                p.as_shape(),
                            );
                        }
                    }

                    SpatialRelation::Within => {
                        i_w += 1;
                        for _ in 0..max_tries {
                            let p = match self.random_point_in_or_null(&s)? {
                                Some(p) => p,
                                // couldn't find a random point in shape
                                None => break,
                            };
                            base.assert_relation_msg(
                                None,
                                SpatialRelation::Contains,
                                r.as_shape(),
                                p.as_shape(),
                            );
                        }
                    }

                    SpatialRelation::Disjoint => {
                        if !s.get_bounding_box().relate(r.as_shape()).intersects() {
                            // bboxes are disjoint
                            i_bbox_d += 1;
                            if i_bbox_d >= self.get_bounding_minimum(minlaps) {
                                return Ok(());
                            }
                        } else {
                            i_d += 1;
                        }
                        for _ in 0..max_tries {
                            let p = base.random_point_in_rect(&*r)?;
                            base.assert_relation_msg(
                                None,
                                SpatialRelation::Disjoint,
                                &*s,
                                p.as_shape(),
                            );
                        }
                    }

                    SpatialRelation::Intersects => {
                        i_i += 1;
                        let mut point_r: Option<SpatialRelation> = None; // set once
                        let mut random_point_space: Option<Arc<dyn Rectangle>> = None;
                        max_tries = 1000; // give many attempts
                        for j in 0..max_tries {
                            let p: Arc<dyn Point> = if j < 4 {
                                let p: Arc<dyn Point> = PointImpl::with_ctx(0.0, 0.0, ctx);
                                InfBufLine::corner_by_quadrant(&*r, j + 1, &*p);
                                p
                            } else {
                                if random_point_space.is_none() {
                                    random_point_space = Some(
                                        if point_r == Some(SpatialRelation::Disjoint) {
                                            intersect_rects(base, &*r, &*s.get_bounding_box())?
                                        } else {
                                            // CONTAINS
                                            Arc::clone(&r)
                                        },
                                    );
                                }
                                base.random_point_in_rect(
                                    &**random_point_space.as_ref().expect("set above"),
                                )?
                            };
                            let point_r_new = s.relate(p.as_shape());
                            match point_r {
                                None => point_r = Some(point_r_new),
                                Some(pr) if pr != point_r_new => break,
                                Some(_) => {}
                            }
                        }
                    }
                }
                Ok(())
            }));
            match body {
                Ok(result) => result?,
                Err(payload) => {
                    let is_assertion_error = match payload.downcast_ref::<Throwable>() {
                        Some(t) => t.class_name() == "java.lang.AssertionError",
                        // a Rust panic is Java's AssertionError
                        None => true,
                    };
                    if !is_assertion_error {
                        resume_unwind(payload);
                    }
                    self.on_assert_fail(payload, &s, &r, ic);
                }
            }
        } // while loop

        system::out_println(&format!(
            "Laps: {laps} CWIDbD: {i_c},{i_w},{i_i},{i_d},{i_bbox_d}"
        ));
        Ok(())
    }

    /// `onAssertFail(AssertionError e, S s, Rectangle r, SpatialRelation ic)`:
    /// rethrows `e`.
    fn on_assert_fail(
        &self,
        e: AssertionPayload,
        _s: &Arc<dyn Shape>,
        _r: &Arc<dyn Rectangle>,
        _ic: SpatialRelation,
    ) {
        resume_unwind(e)
    }
}

/// `intersectRects(Rectangle r1, Rectangle r2)` (`private`).
fn intersect_rects(
    base: &RandomizedShapeTest,
    r1: &dyn Rectangle,
    r2: &dyn Rectangle,
) -> Result<Arc<dyn Rectangle>> {
    if cfg!(debug_assertions) && !r1.relate(r2.as_shape()).intersects() {
        Assert::fail_without_message();
    }
    let min_x = if r1.relate_x_range(r2.get_min_x(), r2.get_min_x()).intersects() {
        r2.get_min_x()
    } else {
        r1.get_min_x()
    };
    let max_x = if r1.relate_x_range(r2.get_max_x(), r2.get_max_x()).intersects() {
        r2.get_max_x()
    } else {
        r1.get_max_x()
    };
    let min_y = if r1.relate_y_range(r2.get_min_y(), r2.get_min_y()).intersects() {
        r2.get_min_y()
    } else {
        r1.get_min_y()
    };
    let max_y = if r1.relate_y_range(r2.get_max_y(), r2.get_max_y()).intersects() {
        r2.get_max_y()
    } else {
        r1.get_max_y()
    };
    base.ctx().make_rectangle(min_x, max_x, min_y, max_y)
}
