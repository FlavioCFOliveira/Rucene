/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `RectangleImpl.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::fmt;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::util::java_fmt::double_to_string;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};
use crate::distance::distance_utils::DistanceUtils;
use crate::jdk::{double_compare_ne, hash_double, AtomicF64};
use crate::shape::base_shape::BaseShape;
use crate::shape::point::Point;
use crate::shape::r#impl::point_impl::PointImpl;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.impl.RectangleImpl`
/// (`public class RectangleImpl extends BaseShape<SpatialContext> implements
/// Rectangle`): a simple rectangle, which crosses the dateline when `minX >
/// maxX` in a geo context.
#[derive(Debug)]
pub struct RectangleImpl {
    me: Weak<RectangleImpl>,
    base: BaseShape,
    min_x: AtomicF64,
    max_x: AtomicF64,
    min_y: AtomicF64,
    max_y: AtomicF64,
}

impl RectangleImpl {
    /// `RectangleImpl(double minX, double maxX, double minY, double maxY,
    /// SpatialContext ctx)`.
    pub fn new(
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        ctx: Option<SpatialContextRef>,
    ) -> Arc<RectangleImpl> {
        let rect = Arc::new_cyclic(|me| RectangleImpl {
            me: me.clone(),
            base: BaseShape::new(ctx),
            min_x: AtomicF64::new(0.0),
            max_x: AtomicF64::new(0.0),
            min_y: AtomicF64::new(0.0),
            max_y: AtomicF64::new(0.0),
        });
        rect.reset(min_x, max_x, min_y, max_y);
        rect
    }

    /// `RectangleImpl(Point lowerLeft, Point upperRight, SpatialContext ctx)`.
    pub fn from_points(
        lower_left: &dyn Point,
        upper_right: &dyn Point,
        ctx: Option<SpatialContextRef>,
    ) -> Arc<RectangleImpl> {
        Self::new(
            lower_left.get_x(),
            upper_right.get_x(),
            lower_left.get_y(),
            upper_right.get_y(),
            ctx,
        )
    }

    /// `RectangleImpl(Rectangle r, SpatialContext ctx)`.
    pub fn from_rect(r: &dyn Rectangle, ctx: Option<SpatialContextRef>) -> Arc<RectangleImpl> {
        Self::new(
            r.get_min_x(),
            r.get_max_x(),
            r.get_min_y(),
            r.get_max_y(),
            ctx,
        )
    }

    fn ctx(&self) -> Arc<SpatialContext> {
        self.base.ctx()
    }

    /// `relate(Point point)`.
    pub fn relate_point(&self, point: &dyn Point) -> SpatialRelation {
        if point.get_y() > self.get_max_y() || point.get_y() < self.get_min_y() {
            return SpatialRelation::Disjoint;
        }
        let min_x = self.min_x.get();
        let mut max_x = self.max_x.get();
        let mut p_x = point.get_x();
        if self.ctx().is_geo() {
            let raw_width = max_x - min_x;
            if raw_width < 0.0 {
                max_x = min_x + (raw_width + 360.0);
            }
            if p_x < min_x {
                p_x += 360.0;
            } else if p_x > max_x {
                p_x -= 360.0;
            } else {
                return SpatialRelation::Contains; //short-circuit
            }
        }
        if p_x < min_x || p_x > max_x {
            return SpatialRelation::Disjoint;
        }
        SpatialRelation::Contains
    }

    /// `relate(Rectangle rect)`.
    pub fn relate_rect(&self, rect: &dyn Rectangle) -> SpatialRelation {
        let y_intersect = self.relate_y_range(rect.get_min_y(), rect.get_max_y());
        if y_intersect == SpatialRelation::Disjoint {
            return SpatialRelation::Disjoint;
        }
        let x_intersect = self.relate_x_range(rect.get_min_x(), rect.get_max_x());
        if x_intersect == SpatialRelation::Disjoint {
            return SpatialRelation::Disjoint;
        }
        if x_intersect == y_intersect {
            //in agreement
            return x_intersect;
        }
        if self.get_min_y() == rect.get_min_y() && self.get_max_y() == rect.get_max_y() {
            return x_intersect;
        }
        if (self.get_min_x() == rect.get_min_x() && self.get_max_x() == rect.get_max_x())
            || (self.ctx().is_geo() && Self::vertical_at_dateline(self, rect))
        {
            return y_intersect;
        }
        SpatialRelation::Intersects
    }

    fn vertical_at_dateline(rect1: &RectangleImpl, rect2: &dyn Rectangle) -> bool {
        if rect1.get_min_x() == rect1.get_max_x() && rect2.get_min_x() == rect2.get_max_x() {
            if rect1.get_min_x() == -180.0 {
                return rect2.get_min_x() == 180.0;
            } else if rect1.get_min_x() == 180.0 {
                return rect2.get_min_x() == -180.0;
            }
        }
        false
    }

    fn relate_range(int_min: f64, int_max: f64, ext_min: f64, ext_max: f64) -> SpatialRelation {
        if ext_min > int_max || ext_max < int_min {
            return SpatialRelation::Disjoint;
        }
        if ext_min >= int_min && ext_max <= int_max {
            return SpatialRelation::Contains;
        }
        if ext_min <= int_min && ext_max >= int_max {
            return SpatialRelation::Within;
        }
        SpatialRelation::Intersects
    }

    /// `equals(Rectangle thiz, Object o)`: the `Rectangle` equality every
    /// rectangle implementation shares.
    ///
    /// Java casts `o` to `RectangleImpl` after the `instanceof Rectangle`
    /// test, so a `Rectangle` of another class throws `ClassCastException`;
    /// this panics with that exception's text.
    pub fn equals_rect(thiz: &dyn Rectangle, o: &dyn Shape) -> bool {
        if thiz.as_shape().is_same(o) {
            return true;
        }
        if o.as_rectangle().is_none() {
            return false;
        }
        let Some(rectangle) = o.as_any().downcast_ref::<RectangleImpl>() else {
            panic!(
                "java.lang.ClassCastException: class {} cannot be cast to class \
                 org.locationtech.spatial4j.shape.impl.RectangleImpl",
                o.java_class_name()
            );
        };
        if double_compare_ne(rectangle.get_max_x(), thiz.get_max_x()) {
            return false;
        }
        if double_compare_ne(rectangle.get_max_y(), thiz.get_max_y()) {
            return false;
        }
        if double_compare_ne(rectangle.get_min_x(), thiz.get_min_x()) {
            return false;
        }
        if double_compare_ne(rectangle.get_min_y(), thiz.get_min_y()) {
            return false;
        }
        true
    }

    /// `hashCode(Rectangle thiz)`.
    pub fn hash_code_rect(thiz: &dyn Rectangle) -> i32 {
        let mut result = hash_double(thiz.get_min_x());
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(hash_double(thiz.get_max_x()));
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(hash_double(thiz.get_min_y()));
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(hash_double(thiz.get_max_y()));
        result
    }
}

impl Rectangle for RectangleImpl {
    fn reset(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) {
        debug_assert!(!self.is_empty());
        self.min_x.set(min_x);
        self.max_x.set(max_x);
        self.min_y.set(min_y);
        self.max_y.set(max_y);
        debug_assert!(
            min_y <= max_y || min_y.is_nan(),
            "minY, maxY: {}, {}",
            double_to_string(min_y),
            double_to_string(max_y)
        );
    }

    fn get_width(&self) -> f64 {
        let mut w = self.max_x.get() - self.min_x.get();
        if w < 0.0 {
            //only true when minX > maxX (WGS84 assumed)
            w += 360.0;
            debug_assert!(w >= 0.0);
        }
        w
    }

    fn get_height(&self) -> f64 {
        self.max_y.get() - self.min_y.get()
    }

    fn get_min_x(&self) -> f64 {
        self.min_x.get()
    }

    fn get_min_y(&self) -> f64 {
        self.min_y.get()
    }

    fn get_max_x(&self) -> f64 {
        self.max_x.get()
    }

    fn get_max_y(&self) -> f64 {
        self.max_y.get()
    }

    fn get_crosses_date_line(&self) -> bool {
        self.min_x.get() > self.max_x.get()
    }

    fn relate_y_range(&self, ext_min_y: f64, ext_max_y: f64) -> SpatialRelation {
        Self::relate_range(self.min_y.get(), self.max_y.get(), ext_min_y, ext_max_y)
    }

    fn relate_x_range(&self, mut ext_min_x: f64, mut ext_max_x: f64) -> SpatialRelation {
        let mut min_x = self.min_x.get();
        let mut max_x = self.max_x.get();
        if self.ctx().is_geo() {
            let raw_width = max_x - min_x;
            if raw_width == 360.0 {
                return SpatialRelation::Contains;
            }
            if raw_width < 0.0 {
                max_x = min_x + (raw_width + 360.0);
            }
            let ext_raw_width = ext_max_x - ext_min_x;
            if ext_raw_width == 360.0 {
                return SpatialRelation::Within;
            }
            if ext_raw_width < 0.0 {
                ext_max_x = ext_min_x + (ext_raw_width + 360.0);
            }
            if max_x < ext_min_x {
                min_x += 360.0;
                max_x += 360.0;
            } else if ext_max_x < min_x {
                ext_min_x += 360.0;
                ext_max_x += 360.0;
            }
        }
        Self::relate_range(min_x, max_x, ext_min_x, ext_max_x)
    }
}

impl Shape for RectangleImpl {
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        if self.is_empty() || other.is_empty() {
            return SpatialRelation::Disjoint;
        }
        if let Some(point) = other.as_point() {
            return self.relate_point(point);
        }
        if let Some(rect) = other.as_rectangle() {
            return self.relate_rect(rect);
        }
        other.relate(self).transpose()
    }

    /// `getBoundingBox()`: `this`.
    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.me.upgrade().expect("a RectangleImpl lives in an Arc")
    }

    fn has_area(&self) -> bool {
        self.max_x.get() != self.min_x.get() && self.max_y.get() != self.min_y.get()
    }

    fn get_area(&self, ctx: Option<&SpatialContext>) -> f64 {
        match ctx {
            None => self.get_width() * self.get_height(),
            Some(ctx) => ctx.get_dist_calc().area_rect(self),
        }
    }

    fn get_center(&self) -> Arc<dyn Point> {
        let min_x = self.min_x.get();
        if min_x.is_nan() {
            return self
                .ctx()
                .make_point(f64::NAN, f64::NAN)
                .unwrap_or_else(|e| panic!("{e}"));
        }
        let y = self.get_height() / 2.0 + self.min_y.get();
        let mut x = self.get_width() / 2.0 + min_x;
        if min_x > self.max_x.get() {
            //WGS84
            x = DistanceUtils::norm_lon_deg(x); //in case falls outside the standard range
        }
        PointImpl::new(x, y, self.base.ctx.clone())
    }

    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let (min_x, max_x, min_y, max_y) = (
            self.min_x.get(),
            self.max_x.get(),
            self.min_y.get(),
            self.max_y.get(),
        );
        let rect = if ctx.is_geo() {
            if max_y + distance >= 90.0 {
                ctx.make_rectangle(-180.0, 180.0, (-90.0f64).max(min_y - distance), 90.0)?
            } else if min_y - distance <= -90.0 {
                ctx.make_rectangle(-180.0, 180.0, -90.0, 90.0f64.min(max_y + distance))?
            } else {
                let lat_distance = distance;
                let closest_to_pole_y = if max_y.abs() > min_y.abs() {
                    max_y
                } else {
                    min_y
                };
                let lon_distance = DistanceUtils::calc_box_by_dist_from_pt_delta_lon_deg(
                    closest_to_pole_y,
                    min_x,
                    distance,
                ); //lat,lon order
                if lon_distance * 2.0 + self.get_width() >= 360.0 {
                    ctx.make_rectangle(-180.0, 180.0, min_y - lat_distance, max_y + lat_distance)?
                } else {
                    ctx.make_rectangle(
                        DistanceUtils::norm_lon_deg(min_x - lon_distance),
                        DistanceUtils::norm_lon_deg(max_x + lon_distance),
                        min_y - lat_distance,
                        max_y + lat_distance,
                    )?
                }
            }
        } else {
            let world_bounds = ctx.get_world_bounds();
            let new_min_x = world_bounds.get_min_x().max(min_x - distance);
            let new_max_x = world_bounds.get_max_x().min(max_x + distance);
            let new_min_y = world_bounds.get_min_y().max(min_y - distance);
            let new_max_y = world_bounds.get_max_y().min(max_y + distance);
            ctx.make_rectangle(new_min_x, new_max_x, new_min_y, new_max_y)?
        };
        Ok(rect)
    }

    fn is_empty(&self) -> bool {
        self.min_x.get().is_nan()
    }

    fn equals(&self, other: &dyn Shape) -> bool {
        Self::equals_rect(self, other)
    }

    fn hash_code(&self) -> i32 {
        Self::hash_code_rect(self)
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.base.get_context()
    }

    fn as_rectangle(&self) -> Option<&dyn Rectangle> {
        Some(self)
    }

    crate::spatial4j_shape_boilerplate!("org.locationtech.spatial4j.shape.impl.RectangleImpl");
}

impl fmt::Display for RectangleImpl {
    /// `toString()`: `Rect(minX=..,maxX=..,minY=..,maxY=..)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Rect(minX={},maxX={},minY={},maxY={})",
            double_to_string(self.min_x.get()),
            double_to_string(self.max_x.get()),
            double_to_string(self.min_y.get()),
            double_to_string(self.max_y.get())
        )
    }
}
