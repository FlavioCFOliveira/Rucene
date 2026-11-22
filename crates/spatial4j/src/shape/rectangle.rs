/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `Rectangle.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.Rectangle` (`public
/// interface Rectangle extends Shape`): an axis-aligned rectangle, which may
/// cross the dateline in a geo context.
pub trait Rectangle: Shape {
    /// `reset(double minX, double maxX, double minY, double maxY)`: changes
    /// this rectangle in place.
    fn reset(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64);

    /// `getWidth()`.
    fn get_width(&self) -> f64;

    /// `getHeight()`.
    fn get_height(&self) -> f64;

    /// `getMinX()`.
    fn get_min_x(&self) -> f64;

    /// `getMinY()`.
    fn get_min_y(&self) -> f64;

    /// `getMaxX()`.
    fn get_max_x(&self) -> f64;

    /// `getMaxY()`.
    fn get_max_y(&self) -> f64;

    /// `getCrossesDateLine()`.
    fn get_crosses_date_line(&self) -> bool;

    /// `relateYRange(double minY, double maxY)`.
    fn relate_y_range(&self, min_y: f64, max_y: f64) -> SpatialRelation;

    /// `relateXRange(double minX, double maxX)`.
    fn relate_x_range(&self, min_x: f64, max_x: f64) -> SpatialRelation;
}
