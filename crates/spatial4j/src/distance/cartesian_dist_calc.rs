/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `CartesianDistCalc.java`, of the Java package
//! `org.locationtech.spatial4j.distance`.

use rucene::util::java_math;
use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::distance::distance_calculator::DistanceCalculator;
use crate::distance::distance_utils::DistanceUtils;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;

/// Equivalent to `org.locationtech.spatial4j.distance.CartesianDistCalc`
/// (`public class CartesianDistCalc extends AbstractDistanceCalculator`):
/// Euclidean distance, optionally squared.
#[derive(Clone, Copy, Debug, Default)]
pub struct CartesianDistCalc {
    squared: bool,
}

impl CartesianDistCalc {
    /// `INSTANCE`.
    pub const INSTANCE: CartesianDistCalc = CartesianDistCalc { squared: false };
    /// `INSTANCE_SQUARED`.
    pub const INSTANCE_SQUARED: CartesianDistCalc = CartesianDistCalc { squared: true };

    /// `CartesianDistCalc()`.
    pub fn new() -> Self {
        Self { squared: false }
    }

    /// `CartesianDistCalc(boolean squared)`.
    pub fn with_squared(squared: bool) -> Self {
        Self { squared }
    }

    fn distance_squared(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> f64 {
        let delta_x = from_x - to_x;
        let delta_y = from_y - to_y;
        delta_x * delta_x + delta_y * delta_y
    }

    /// `distanceToLineSegment(Point point, double vX, double vY, double wX,
    /// double wY)`.
    pub fn distance_to_line_segment(
        &self,
        point: &dyn Point,
        v_x: f64,
        v_y: f64,
        w_x: f64,
        w_y: f64,
    ) -> f64 {
        // Translated from: http://bl.ocks.org/mbostock/4218871
        let d = Self::distance_squared(v_x, v_y, w_x, w_y);
        let (to_x, to_y);
        if d <= 0.0 {
            to_x = v_x;
            to_y = v_y;
        } else {
            // t = ((point[0] - v[0]) * (w[0] - v[0]) + (point[1] - v[1]) * (w[1] - v[1])) / d
            let t = ((point.get_x() - v_x) * (w_x - v_x) + (point.get_y() - v_y) * (w_y - v_y)) / d;
            if t < 0.0 {
                to_x = v_x;
                to_y = v_y;
            } else if t > 1.0 {
                to_x = w_x;
                to_y = w_y;
            } else {
                to_x = v_x + t * (w_x - v_x);
                to_y = v_y + t * (w_y - v_y);
            }
        }
        self.distance_xy(point, to_x, to_y)
    }
}

impl DistanceCalculator for CartesianDistCalc {
    fn distance_xy(&self, from: &dyn Point, to_x: f64, to_y: f64) -> f64 {
        let x_squared_plus_y_squared =
            Self::distance_squared(from.get_x(), from.get_y(), to_x, to_y);
        if self.squared {
            return x_squared_plus_y_squared;
        }
        java_math::sqrt(x_squared_plus_y_squared)
    }

    fn within(&self, from: &dyn Point, to_x: f64, to_y: f64, distance: f64) -> bool {
        let delta_x = from.get_x() - to_x;
        let delta_y = from.get_y() - to_y;
        delta_x * delta_x + delta_y * delta_y <= distance * distance
    }

    fn point_on_bearing(
        &self,
        from: &Arc<dyn Point>,
        dist_deg: f64,
        bearing_deg: f64,
        ctx: &SpatialContext,
        reuse: Option<Arc<dyn Point>>,
    ) -> Result<Arc<dyn Point>> {
        if dist_deg == 0.0 {
            let Some(reuse) = reuse else {
                return Ok(Arc::clone(from));
            };
            reuse.reset(from.get_x(), from.get_y());
            return Ok(reuse);
        }
        let bearing_rad = DistanceUtils::to_radians(bearing_deg);
        let x = from.get_x() + java_math::sin(bearing_rad) * dist_deg;
        let y = from.get_y() + java_math::cos(bearing_rad) * dist_deg;
        match reuse {
            None => ctx.make_point(x, y),
            Some(reuse) => {
                reuse.reset(x, y);
                Ok(reuse)
            }
        }
    }

    fn calc_box_by_dist_from_pt(
        &self,
        from: &dyn Point,
        dist_deg: f64,
        ctx: &SpatialContext,
        reuse: Option<Arc<dyn Rectangle>>,
    ) -> Result<Arc<dyn Rectangle>> {
        let min_x = from.get_x() - dist_deg;
        let max_x = from.get_x() + dist_deg;
        let min_y = from.get_y() - dist_deg;
        let max_y = from.get_y() + dist_deg;
        match reuse {
            None => ctx.make_rectangle(min_x, max_x, min_y, max_y),
            Some(reuse) => {
                reuse.reset(min_x, max_x, min_y, max_y);
                Ok(reuse)
            }
        }
    }

    fn calc_box_by_dist_from_pt_y_horiz_axis_deg(
        &self,
        from: &dyn Point,
        _dist_deg: f64,
        _ctx: &SpatialContext,
    ) -> f64 {
        from.get_y()
    }

    fn area_rect(&self, rect: &dyn Rectangle) -> f64 {
        rect.get_area(None)
    }

    fn area_circle(&self, circle: &dyn Circle) -> f64 {
        circle.get_area(None)
    }

    fn equals(&self, o: &dyn DistanceCalculator) -> bool {
        match o.as_any().downcast_ref::<CartesianDistCalc>() {
            Some(that) => self.squared == that.squared,
            None => false,
        }
    }

    fn hash_code(&self) -> i32 {
        i32::from(self.squared)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl fmt::Display for CartesianDistCalc {
    /// `toString()`: `getClass().getSimpleName()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CartesianDistCalc")
    }
}
