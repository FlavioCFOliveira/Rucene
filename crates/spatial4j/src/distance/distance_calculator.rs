/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `DistanceCalculator.java`, of the Java package
//! `org.locationtech.spatial4j.distance`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;

/// Equivalent to `org.locationtech.spatial4j.distance.DistanceCalculator`
/// (`public interface`): distance calculations, in the context's units.
///
/// `equals(Object)`, `hashCode()` and `toString()` are
/// [`equals`](Self::equals), [`hash_code`](Self::hash_code) and the
/// [`fmt::Display`] implementation.
pub trait DistanceCalculator: fmt::Display + fmt::Debug + Send + Sync + Any {
    /// `distance(Point from, Point to)`.
    fn distance(&self, from: &dyn Point, to: &dyn Point) -> f64 {
        self.distance_xy(from, to.get_x(), to.get_y())
    }

    /// `distance(Point from, double toX, double toY)`.
    fn distance_xy(&self, from: &dyn Point, to_x: f64, to_y: f64) -> f64;

    /// `within(Point from, double toX, double toY, double distance)`.
    fn within(&self, from: &dyn Point, to_x: f64, to_y: f64, distance: f64) -> bool {
        self.distance_xy(from, to_x, to_y) <= distance
    }

    /// `pointOnBearing(Point from, double distDEG, double bearingDEG,
    /// SpatialContext ctx, Point reuse)`: `reuse` (reset) when given, else a
    /// new point; `from` itself when `distDEG` is 0 and there is no `reuse`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makePoint`.
    fn point_on_bearing(
        &self,
        from: &Arc<dyn Point>,
        dist_deg: f64,
        bearing_deg: f64,
        ctx: &SpatialContext,
        reuse: Option<Arc<dyn Point>>,
    ) -> Result<Arc<dyn Point>>;

    /// `calcBoxByDistFromPt(Point from, double distDEG, SpatialContext ctx,
    /// Rectangle reuse)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makeRectangle`.
    fn calc_box_by_dist_from_pt(
        &self,
        from: &dyn Point,
        dist_deg: f64,
        ctx: &SpatialContext,
        reuse: Option<Arc<dyn Rectangle>>,
    ) -> Result<Arc<dyn Rectangle>>;

    /// `calcBoxByDistFromPt_yHorizAxisDEG(Point from, double distDEG,
    /// SpatialContext ctx)`.
    fn calc_box_by_dist_from_pt_y_horiz_axis_deg(
        &self,
        from: &dyn Point,
        dist_deg: f64,
        ctx: &SpatialContext,
    ) -> f64;

    /// `area(Rectangle rect)`.
    fn area_rect(&self, rect: &dyn Rectangle) -> f64;

    /// `area(Circle circle)`.
    fn area_circle(&self, circle: &dyn Circle) -> f64;

    /// `equals(Object)`.
    fn equals(&self, other: &dyn DistanceCalculator) -> bool;

    /// `hashCode()`.
    fn hash_code(&self) -> i32;

    /// This calculator as `Any`, for `getClass()` and casts.
    fn as_any(&self) -> &dyn Any;
}
