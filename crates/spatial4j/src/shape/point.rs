/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `Point.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.shape.Point` (`public interface
/// Point extends Shape`): a point.
pub trait Point: Shape {
    /// `reset(double x, double y)`: changes this point in place.
    fn reset(&self, x: f64, y: f64);

    /// `getX()`.
    fn get_x(&self) -> f64;

    /// `getY()`.
    fn get_y(&self) -> f64;

    /// `getLat()`: `getY()` by default.
    fn get_lat(&self) -> f64 {
        self.get_y()
    }

    /// `getLon()`: `getX()` by default.
    fn get_lon(&self) -> f64 {
        self.get_x()
    }
}
