/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `Circle.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.shape.Circle` (`public interface
/// Circle extends Shape`): a circle, its radius in the context's units.
pub trait Circle: Shape {
    /// `reset(double x, double y, double radiusDEG)`: changes this circle in
    /// place.
    fn reset(&self, x: f64, y: f64, radius_deg: f64);

    /// `getRadius()`.
    fn get_radius(&self) -> f64;
}
