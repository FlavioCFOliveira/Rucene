/*******************************************************************************
 * Copyright (c) 2015 MITRE and VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `AbstractDistanceCalculator.java`, of the Java
//! package `org.locationtech.spatial4j.distance`.

/// Equivalent to
/// `org.locationtech.spatial4j.distance.AbstractDistanceCalculator` (`public
/// abstract class AbstractDistanceCalculator implements
/// DistanceCalculator`).
///
/// Its `distance(Point, Point)` and `within(...)` are the default methods of
/// [`DistanceCalculator`](crate::distance::distance_calculator::DistanceCalculator);
/// its `toString()`, `getClass().getSimpleName()`, is
/// [`to_string`](Self::to_string), which each calculator's `Display` uses.
pub struct AbstractDistanceCalculator;

impl AbstractDistanceCalculator {
    /// `toString()`: the simple class name.
    pub fn to_string(simple_class_name: &str) -> String {
        simple_class_name.to_string()
    }
}
