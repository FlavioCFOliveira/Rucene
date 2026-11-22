/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `GeoCircle.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::fmt;
use std::sync::{Arc, RwLock};

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::distance::distance_utils::DistanceUtils;
use rucene::util::java_fmt::format_fixed;

use crate::jdk::AtomicF64;
use crate::shape::circle::Circle;
use crate::shape::r#impl::circle_impl::{CircleImpl, CircleOverrides, CircleState};
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.impl.GeoCircle` (`public
/// class GeoCircle extends CircleImpl`): a circle on the surface of a sphere.
#[derive(Debug)]
pub struct GeoCircle {
    state: CircleState,
    /// `inverseCircle`: cached when the radius exceeds half the world.
    inverse_circle: RwLock<Option<Arc<GeoCircle>>>,
    /// `horizAxisY`.
    horiz_axis_y: AtomicF64,
}

/// `Math.ulp(double)`.
fn ulp(d: f64) -> f64 {
    let d = d.abs();
    if d.is_nan() {
        return f64::NAN;
    }
    if d.is_infinite() {
        return f64::INFINITY;
    }
    if d == f64::MAX {
        return 2f64.powi(971);
    }
    let next = f64::from_bits(d.to_bits() + 1);
    next - d
}

impl GeoCircle {
    /// `GeoCircle(Point p, double radiusDEG, SpatialContext ctx)`.
    pub fn new(p: Arc<dyn Point>, radius_deg: f64, ctx: &Arc<SpatialContext>) -> Arc<GeoCircle> {
        let circle = Arc::new(GeoCircle {
            state: CircleState::new(p, radius_deg, ctx),
            inverse_circle: RwLock::new(None),
            horiz_axis_y: AtomicF64::new(0.0),
        });
        debug_assert!(ctx.is_geo());
        circle.init();
        circle
    }

    fn inverse(&self) -> Option<Arc<GeoCircle>> {
        self.inverse_circle.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_inverse(&self, c: Option<Arc<GeoCircle>>) {
        *self.inverse_circle.write().unwrap_or_else(|e| e.into_inner()) = c;
    }

    fn init(&self) {
        let radius_deg = self.state.radius_deg.get();
        let enclosing_box = self.state.enclosing_box();
        let ctx = self.state.ctx();
        if radius_deg > 90.0 {
            debug_assert!(enclosing_box.get_width() == 360.0);
            let back_dist_deg = 180.0 - radius_deg;
            if back_dist_deg > 0.0 {
                let mut back_radius = 180.0 - radius_deg;
                let back_x = DistanceUtils::norm_lon_deg(self.get_center().get_x() + 180.0);
                let back_y = DistanceUtils::norm_lat_deg(self.get_center().get_y() + 180.0);
                back_radius -= ulp(back_y.abs() + back_radius).max(ulp(back_x.abs() + back_radius));
                if let Some(inverse) = self.inverse() {
                    inverse.reset(back_x, back_y, back_radius);
                } else {
                    let point = ctx.make_point(back_x, back_y).unwrap_or_else(|e| panic!("{e}"));
                    self.set_inverse(Some(GeoCircle::new(point, back_radius, &ctx)));
                }
            } else {
                self.set_inverse(None); //whole globe
            }
            self.horiz_axis_y.set(self.get_center().get_y()); //although probably not used
        } else {
            self.set_inverse(None);
            let horiz_axis_y = ctx.get_dist_calc().calc_box_by_dist_from_pt_y_horiz_axis_deg(
                &*self.get_center(),
                radius_deg,
                &ctx,
            );
            if horiz_axis_y > enclosing_box.get_max_y() {
                self.horiz_axis_y.set(enclosing_box.get_max_y());
            } else if horiz_axis_y < enclosing_box.get_min_y() {
                self.horiz_axis_y.set(enclosing_box.get_min_y());
            } else {
                self.horiz_axis_y.set(horiz_axis_y);
            }
        }
    }

    /// `contains(double x, double y)`.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.state.contains(x, y)
    }

    fn relate_rectangle_circle_wraps_pole(&self, r: &dyn Rectangle) -> SpatialRelation {
        let radius_deg = self.state.radius_deg.get();
        if radius_deg == 180.0 {
            //whole globe
            return SpatialRelation::Contains;
        }
        let y_top = self.get_center().get_y() + radius_deg;
        if y_top > 90.0 {
            let y_top_overlap = y_top - 90.0;
            debug_assert!(y_top_overlap <= 90.0);
            if r.get_min_y() >= 90.0 - y_top_overlap {
                return SpatialRelation::Contains;
            }
        } else {
            let y_bot = self.state.point.get_y() - radius_deg;
            if y_bot < -90.0 {
                let y_bot_overlap = -90.0 - y_bot;
                debug_assert!(y_bot_overlap <= 90.0);
                if r.get_max_y() <= -90.0 + y_bot_overlap {
                    return SpatialRelation::Contains;
                }
            } else {
                debug_assert!(y_top == 90.0 || y_bot == -90.0); //we simply touch a pole
            }
        }
        if r.get_width() == 360.0 {
            return SpatialRelation::Intersects;
        }
        let corners_intersect = self.num_corners_intersect(r);
        let front_x = self.get_center().get_x();
        if corners_intersect == 4 {
            //all
            let back_x = if front_x <= 0.0 { front_x + 180.0 } else { front_x - 180.0 };
            if r.relate_x_range(back_x, back_x).intersects() {
                SpatialRelation::Intersects
            } else {
                SpatialRelation::Contains
            }
        } else if corners_intersect == 0 {
            //none
            if r.relate_x_range(front_x, front_x).intersects() {
                SpatialRelation::Intersects
            } else {
                SpatialRelation::Disjoint
            }
        } else {
            //partial
            SpatialRelation::Intersects
        }
    }

    fn num_corners_intersect(&self, r: &dyn Rectangle) -> i32 {
        //if true then all corners intersect, if false then no corners intersect
        let b = self.contains(r.get_min_x(), r.get_min_y());
        if self.contains(r.get_min_x(), r.get_max_y()) {
            if !b {
                return 1; //partial
            }
        } else if b {
            return 1; //partial
        }
        if self.contains(r.get_max_x(), r.get_min_y()) {
            if !b {
                return 1; //partial
            }
        } else if b {
            return 1; //partial
        }
        if self.contains(r.get_max_x(), r.get_max_y()) {
            if !b {
                return 1; //partial
            }
        } else if b {
            return 1; //partial
        }
        if b {
            4
        } else {
            0
        }
    }
}

impl CircleOverrides for GeoCircle {
    fn state(&self) -> &CircleState {
        &self.state
    }

    /// `getYAxis()`: `horizAxisY`.
    fn get_y_axis(&self) -> f64 {
        self.horiz_axis_y.get()
    }

    fn relate_rectangle_phase2(&self, r: &dyn Rectangle, bbox_sect: SpatialRelation) -> SpatialRelation {
        if let Some(inverse) = self.inverse() {
            return inverse.relate(r.as_shape()).inverse();
        }
        let enclosing_box = self.state.enclosing_box();
        if enclosing_box.get_width() == 360.0 {
            return self.relate_rectangle_circle_wraps_pole(r);
        }
        if !enclosing_box.get_crosses_date_line() && !r.get_crosses_date_line() {
            return CircleImpl::relate_rectangle_phase2_impl(self, r, bbox_sect);
        }
        if r.get_width() == 360.0 {
            return SpatialRelation::Intersects;
        }
        let corners_intersect = self.num_corners_intersect(r);
        if corners_intersect == 4 {
            let x_intersect = r.relate_x_range(enclosing_box.get_min_x(), enclosing_box.get_max_x());
            if x_intersect == SpatialRelation::Within {
                return SpatialRelation::Contains;
            }
            return SpatialRelation::Intersects;
        }
        if corners_intersect > 0 {
            return SpatialRelation::Intersects;
        }
        if r.relate_y_range(self.get_y_axis(), self.get_y_axis()).intersects() // at y vertical
            && r.relate_x_range(enclosing_box.get_min_x(), enclosing_box.get_max_x()).intersects()
        {
            return SpatialRelation::Intersects;
        }
        if r.relate_x_range(self.get_x_axis(), self.get_x_axis()).intersects() {
            // at x horizontal
            let radius_deg = self.state.radius_deg.get();
            let y_top = self.get_center().get_y() + radius_deg;
            debug_assert!(y_top <= 90.0);
            let y_bot = self.get_center().get_y() - radius_deg;
            debug_assert!(y_bot >= -90.0);
            if r.relate_y_range(y_bot, y_top).intersects() {
                //back bottom
                return SpatialRelation::Intersects;
            }
        }
        SpatialRelation::Disjoint
    }
}

impl Circle for GeoCircle {
    fn reset(&self, x: f64, y: f64, radius_deg: f64) {
        self.state.reset(x, y, radius_deg);
        self.init();
    }

    fn get_radius(&self) -> f64 {
        self.state.radius_deg.get()
    }
}

impl Shape for GeoCircle {
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        CircleImpl::relate_impl(self, other)
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.state.enclosing_box()
    }

    fn has_area(&self) -> bool {
        self.state.radius_deg.get() > 0.0
    }

    fn get_area(&self, ctx: Option<&SpatialContext>) -> f64 {
        CircleImpl::get_area_impl(self, self.state.radius_deg.get(), ctx)
    }

    fn get_center(&self) -> Arc<dyn Point> {
        Arc::clone(&self.state.point)
    }

    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let c = ctx.make_circle_at(Arc::clone(&self.state.point), distance + self.state.radius_deg.get())?;
        Ok(c)
    }

    fn is_empty(&self) -> bool {
        self.state.point.is_empty()
    }

    fn equals(&self, other: &dyn Shape) -> bool {
        CircleImpl::equals_circle(self, other)
    }

    fn hash_code(&self) -> i32 {
        CircleImpl::hash_code_circle(self)
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.state.base.get_context()
    }

    fn as_circle(&self) -> Option<&dyn Circle> {
        Some(self)
    }

    crate::spatial4j_shape_boilerplate!("org.locationtech.spatial4j.shape.impl.GeoCircle");
}

impl fmt::Display for GeoCircle {
    /// `toString()`: `Circle(<point>, d=<deg>° <km>km)` with
    /// `String.format(Locale.ROOT, "%.1f° %.2fkm", ...)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let radius_deg = self.state.radius_deg.get();
        let dist_km = DistanceUtils::degrees2_dist(radius_deg, DistanceUtils::EARTH_MEAN_RADIUS_KM);
        write!(
            f,
            "Circle({}, d={}\u{00B0} {}km)",
            self.state.point,
            format_fixed(radius_deg, 1),
            format_fixed(dist_km, 2)
        )
    }
}
