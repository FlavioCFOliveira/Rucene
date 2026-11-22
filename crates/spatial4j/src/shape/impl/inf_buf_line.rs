/*******************************************************************************
 * Copyright (c) 2015 MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `InfBufLine.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::fmt;

use rucene::util::java_fmt::double_to_string;

use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.impl.InfBufLine` (`public
/// class`): an infinite line with a buffer distance, an internal helper of
/// `BufferedLine`.
#[derive(Clone, Copy, Debug)]
pub struct InfBufLine {
    slope: f64, //can be infinite for vertical line
    intercept: f64,
    buf: f64,
    dist_denom_inv: f64, //cached: 1 / Math.sqrt(slope * slope + 1)
}

const EPS: f64 = 10e-14;
const OPPOSITE_QUAD: [i32; 5] = [-1, 3, 4, 1, 2];

impl InfBufLine {
    /// `InfBufLine(double slope, Point point, double buf)` (package-private).
    pub(crate) fn new(slope: f64, point: &dyn Point, buf: f64) -> Self {
        debug_assert!(!slope.is_nan());
        let (intercept, dist_denom_inv) = if slope.is_infinite() {
            (point.get_x(), f64::NAN)
        } else {
            (point.get_y() - slope * point.get_x(), 1.0 / (slope * slope + 1.0).sqrt())
        };
        Self { slope, intercept, buf, dist_denom_inv }
    }

    /// `relate(Rectangle r, Point prC, Point scratch)` (package-private).
    pub(crate) fn relate(&self, r: &dyn Rectangle, pr_c: &dyn Point, scratch: &dyn Point) -> SpatialRelation {
        debug_assert!(r.get_center().equals(pr_c.as_shape()));
        let c_quad = self.quadrant(pr_c);
        let nearest_p = scratch;
        Self::corner_by_quadrant(r, OPPOSITE_QUAD[c_quad as usize], nearest_p);
        let nearest_contains = self.contains(nearest_p);
        if nearest_contains {
            let farthest_p = scratch;
            Self::corner_by_quadrant(r, c_quad, farthest_p);
            let farthest_contains = self.contains(farthest_p);
            if farthest_contains {
                return SpatialRelation::Contains;
            }
            SpatialRelation::Intersects
        } else {
            // not nearestContains
            if self.quadrant(nearest_p) == c_quad {
                return SpatialRelation::Disjoint; //out of buffer on same side as center
            }
            SpatialRelation::Intersects //nearest & farthest points straddle the line
        }
    }

    /// `contains(Point p)` (package-private).
    pub(crate) fn contains(&self, p: &dyn Point) -> bool {
        self.distance_unbuffered(p) <= self.buf + EPS
    }

    /// `distanceUnbuffered(Point c)`.
    pub fn distance_unbuffered(&self, c: &dyn Point) -> f64 {
        if self.slope.is_infinite() {
            return (c.get_x() - self.intercept).abs();
        }
        let num = (c.get_y() - self.slope * c.get_x() - self.intercept).abs();
        num * self.dist_denom_inv
    }

    /// `quadrant(Point c)`.
    pub fn quadrant(&self, c: &dyn Point) -> i32 {
        if self.slope.is_infinite() {
            return if c.get_x() > self.intercept { 1 } else { 2 }; //4 : 3 would work too
        }
        let y_at_cin_line = self.slope * c.get_x() + self.intercept;
        let above = c.get_y() >= y_at_cin_line;
        if self.slope > 0.0 {
            if above {
                2
            } else {
                4
            }
        } else if above {
            1
        } else {
            3
        }
    }

    /// `cornerByQuadrant(Rectangle r, int cornerQuad, Point out)`.
    pub fn corner_by_quadrant(r: &dyn Rectangle, corner_quad: i32, out: &dyn Point) {
        let x = if corner_quad == 1 || corner_quad == 4 { r.get_max_x() } else { r.get_min_x() };
        let y = if corner_quad == 1 || corner_quad == 2 { r.get_max_y() } else { r.get_min_y() };
        out.reset(x, y);
    }

    /// `getSlope()`.
    pub fn get_slope(&self) -> f64 {
        self.slope
    }

    /// `getIntercept()`.
    pub fn get_intercept(&self) -> f64 {
        self.intercept
    }

    /// `getBuf()`.
    pub fn get_buf(&self) -> f64 {
        self.buf
    }

    /// `getDistDenomInv()`.
    pub fn get_dist_denom_inv(&self) -> f64 {
        self.dist_denom_inv
    }
}

impl fmt::Display for InfBufLine {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InfBufLine{{buf={}, intercept={}, slope={}}}",
            double_to_string(self.buf),
            double_to_string(self.intercept),
            double_to_string(self.slope)
        )
    }
}
