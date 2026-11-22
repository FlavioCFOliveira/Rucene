/*******************************************************************************
 * Copyright (c) 2015 MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `BufferedLine.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::double_to_string;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};
use crate::distance::distance_utils::DistanceUtils;
use crate::jdk::{double_compare_ne, hash_double};
use crate::shape::base_shape::BaseShape;
use crate::shape::r#impl::inf_buf_line::InfBufLine;
use crate::shape::r#impl::point_impl::PointImpl;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.impl.BufferedLine`
/// (`public class BufferedLine extends BaseShape<SpatialContext>`): a line
/// segment with a buffer, in a Euclidean space.
#[derive(Debug)]
pub struct BufferedLine {
    base: BaseShape,
    p_a: Arc<dyn Point>,
    p_b: Arc<dyn Point>,
    buf: f64,
    bbox: Arc<dyn Rectangle>,
    /// the primary line; passes through pA &amp; pB.
    line_primary: InfBufLine,
    /// perpendicular to the primary line, centered between pA &amp; pB.
    line_perp: InfBufLine,
}

impl BufferedLine {
    /// `BufferedLine(Point pA, Point pB, double buf, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makeRectangle` for the bounding
    /// box.
    pub fn new(p_a: Arc<dyn Point>, p_b: Arc<dyn Point>, buf: f64, ctx: &Arc<SpatialContext>) -> Result<Arc<BufferedLine>> {
        debug_assert!(buf >= 0.0); //TODO support buf=0 via another class ?
        let buf_extend = true;
        let delta_y = p_b.get_y() - p_a.get_y();
        let delta_x = p_b.get_x() - p_a.get_x();
        let center = PointImpl::new(p_a.get_x() + delta_x / 2.0, p_a.get_y() + delta_y / 2.0, None);
        let perp_extent = if buf_extend { buf } else { 0.0 };
        let (line_primary, line_perp) = if delta_x == 0.0 && delta_y == 0.0 {
            (InfBufLine::new(0.0, &*center, buf), InfBufLine::new(f64::INFINITY, &*center, buf))
        } else {
            let line_primary = InfBufLine::new(delta_y / delta_x, &*center, buf);
            let length = (delta_x * delta_x + delta_y * delta_y).sqrt();
            let line_perp = InfBufLine::new(-delta_x / delta_y, &*center, length / 2.0 + perp_extent);
            (line_primary, line_perp)
        };
        let (min_x, max_x, min_y, max_y);
        if delta_x == 0.0 {
            // vertical
            let (lo_y, hi_y) = if p_a.get_y() <= p_b.get_y() {
                (p_a.get_y(), p_b.get_y())
            } else {
                (p_b.get_y(), p_a.get_y())
            };
            min_x = p_a.get_x() - buf;
            max_x = p_a.get_x() + buf;
            min_y = lo_y - perp_extent;
            max_y = hi_y + perp_extent;
        } else {
            if !buf_extend {
                return Err(LuceneError::unsupported_operation("TODO"));
            }
            //solved via Wolfram Alpha
            let bbox_buf = buf * (1.0 + line_primary.get_slope().abs()) * line_primary.get_dist_denom_inv();
            debug_assert!(bbox_buf >= buf && bbox_buf <= buf * 1.5);
            if p_a.get_x() <= p_b.get_x() {
                min_x = p_a.get_x() - bbox_buf;
                max_x = p_b.get_x() + bbox_buf;
            } else {
                min_x = p_b.get_x() - bbox_buf;
                max_x = p_a.get_x() + bbox_buf;
            }
            if p_a.get_y() <= p_b.get_y() {
                min_y = p_a.get_y() - bbox_buf;
                max_y = p_b.get_y() + bbox_buf;
            } else {
                min_y = p_b.get_y() - bbox_buf;
                max_y = p_a.get_y() + bbox_buf;
            }
        }
        let bounds = ctx.get_world_bounds();
        let bbox = ctx.make_rectangle(
            bounds.get_min_x().max(min_x),
            bounds.get_max_x().min(max_x),
            bounds.get_min_y().max(min_y),
            bounds.get_max_y().min(max_y),
        )?;
        Ok(Arc::new(BufferedLine {
            base: BaseShape::new(Some(SpatialContextRef::from(ctx))),
            p_a,
            p_b,
            buf,
            bbox,
            line_primary,
            line_perp,
        }))
    }

    /// `expandBufForLongitudeSkew(Point pA, Point pB, double buf)`.
    pub fn expand_buf_for_longitude_skew(p_a: &dyn Point, p_b: &dyn Point, buf: f64) -> f64 {
        let abs_a = p_a.get_y().abs();
        let abs_b = p_b.get_y().abs();
        let max_lat = abs_a.max(abs_b);
        let new_buf = DistanceUtils::calc_lon_degrees_at_lat(max_lat, buf);
        debug_assert!(new_buf >= buf);
        new_buf
    }

    /// `relate(Rectangle r)`.
    pub fn relate_rect(&self, r: &dyn Rectangle) -> SpatialRelation {
        //Check BBox for disjoint & within.
        let bbox_r = self.bbox.relate(r.as_shape());
        if bbox_r == SpatialRelation::Disjoint || bbox_r == SpatialRelation::Within {
            return bbox_r;
        }
        //Either CONTAINS, INTERSECTS, or DISJOINT
        let scratch = PointImpl::new(0.0, 0.0, None);
        let pr_c = r.get_center();
        let result = self.line_primary.relate(r, &*pr_c, &*scratch);
        if result == SpatialRelation::Disjoint {
            return SpatialRelation::Disjoint;
        }
        let result_opp = self.line_perp.relate(r, &*pr_c, &*scratch);
        if result_opp == SpatialRelation::Disjoint {
            return SpatialRelation::Disjoint;
        }
        if result == result_opp {
            //either CONTAINS or INTERSECTS
            return result;
        }
        SpatialRelation::Intersects
    }

    /// `contains(Point p)`.
    pub fn contains(&self, p: &dyn Point) -> bool {
        self.line_primary.contains(p) && self.line_perp.contains(p)
    }

    /// `getA()`.
    pub fn get_a(&self) -> &Arc<dyn Point> {
        &self.p_a
    }

    /// `getB()`.
    pub fn get_b(&self) -> &Arc<dyn Point> {
        &self.p_b
    }

    /// `getBuf()`.
    pub fn get_buf(&self) -> f64 {
        self.buf
    }

    /// `getLinePrimary()`: the primary line, through pA &amp; pB.
    pub fn get_line_primary(&self) -> &InfBufLine {
        &self.line_primary
    }

    /// `getLinePerp()`: the line perpendicular to the primary one.
    pub fn get_line_perp(&self) -> &InfBufLine {
        &self.line_perp
    }
}

impl Shape for BufferedLine {
    /// `relate(Shape)`.
    ///
    /// Java throws `UnsupportedOperationException` for a shape that is
    /// neither a point nor a rectangle; this panics with that text.
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        if let Some(p) = other.as_point() {
            return if self.contains(p) { SpatialRelation::Contains } else { SpatialRelation::Disjoint };
        }
        if let Some(r) = other.as_rectangle() {
            return self.relate_rect(r);
        }
        panic!("java.lang.UnsupportedOperationException");
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        Arc::clone(&self.bbox)
    }

    fn has_area(&self) -> bool {
        self.buf > 0.0
    }

    fn get_area(&self, _ctx: Option<&SpatialContext>) -> f64 {
        self.line_primary.get_buf() * self.line_perp.get_buf() * 4.0
    }

    fn get_center(&self) -> Arc<dyn Point> {
        self.get_bounding_box().get_center()
    }

    fn get_buffered(&self, distance: f64, _ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let ctx = _ctx.self_arc();
        let line = BufferedLine::new(Arc::clone(&self.p_a), Arc::clone(&self.p_b), self.buf + distance, &ctx)?;
        Ok(line)
    }

    fn is_empty(&self) -> bool {
        self.p_a.is_empty()
    }

    fn equals(&self, o: &dyn Shape) -> bool {
        if self.as_shape().is_same(o) {
            return true;
        }
        if !self.as_shape().same_class(o) {
            return false;
        }
        let Some(that) = o.as_any().downcast_ref::<BufferedLine>() else {
            return false;
        };
        if double_compare_ne(that.buf, self.buf) {
            return false;
        }
        if !self.p_a.equals(that.p_a.as_shape()) {
            return false;
        }
        if !self.p_b.equals(that.p_b.as_shape()) {
            return false;
        }
        true
    }

    fn hash_code(&self) -> i32 {
        let mut result = self.p_a.hash_code();
        result = 31i32.wrapping_mul(result).wrapping_add(self.p_b.hash_code());
        31i32.wrapping_mul(result).wrapping_add(hash_double(self.buf))
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.base.get_context()
    }

    crate::spatial4j_shape_boilerplate!("org.locationtech.spatial4j.shape.impl.BufferedLine");
}

impl fmt::Display for BufferedLine {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BufferedLine({}, {} b={})", self.p_a, self.p_b, double_to_string(self.buf))
    }
}
