/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `XYZBounds.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::any::Any;
use std::fmt;

use super::bounds::Bounds;
use super::geo_point::GeoPoint;
use super::membership::Membership;
use super::od2s;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::vector::Vector;

/// An object for accumulating XYZ bounds information.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.XYZBounds`; Java's
/// nullable `Double` fields are `Option<f64>`, and unboxing a `null` is the
/// `NullPointerException` Java raises.
#[derive(Clone, Debug, Default)]
pub struct XYZBounds {
    min_x: Option<f64>,
    max_x: Option<f64>,
    min_y: Option<f64>,
    max_y: Option<f64>,
    min_z: Option<f64>,
    max_z: Option<f64>,
}

/// Unboxes a `Double`.
#[inline]
fn ub(v: Option<f64>) -> f64 {
    v.unwrap_or_else(|| super::npe())
}

impl XYZBounds {
    /// A 1,000 multiple of `MINIMUM_RESOLUTION` added to the bounds.
    const FUDGE_FACTOR: f64 = Vector::MINIMUM_RESOLUTION * 1e3;

    /// `new XYZBounds()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getMinimumX()`.
    pub fn get_minimum_x(&self) -> Option<f64> {
        self.min_x
    }

    /// `getMaximumX()`.
    pub fn get_maximum_x(&self) -> Option<f64> {
        self.max_x
    }

    /// `getMinimumY()`.
    pub fn get_minimum_y(&self) -> Option<f64> {
        self.min_y
    }

    /// `getMaximumY()`.
    pub fn get_maximum_y(&self) -> Option<f64> {
        self.max_y
    }

    /// `getMinimumZ()`.
    pub fn get_minimum_z(&self) -> Option<f64> {
        self.min_z
    }

    /// `getMaximumZ()`.
    pub fn get_maximum_z(&self) -> Option<f64> {
        self.max_z
    }

    /// `isSmallestMinX(PlanetModel)`.
    pub fn is_smallest_min_x(&self, planet_model: &PlanetModel) -> bool {
        match self.min_x {
            None => false,
            Some(v) => v - planet_model.get_minimum_x_value() < Vector::MINIMUM_RESOLUTION,
        }
    }

    /// `isLargestMaxX(PlanetModel)`.
    pub fn is_largest_max_x(&self, planet_model: &PlanetModel) -> bool {
        match self.max_x {
            None => false,
            Some(v) => planet_model.get_maximum_x_value() - v < Vector::MINIMUM_RESOLUTION,
        }
    }

    /// `isSmallestMinY(PlanetModel)`.
    pub fn is_smallest_min_y(&self, planet_model: &PlanetModel) -> bool {
        match self.min_y {
            None => false,
            Some(v) => v - planet_model.get_minimum_y_value() < Vector::MINIMUM_RESOLUTION,
        }
    }

    /// `isLargestMaxY(PlanetModel)`.
    pub fn is_largest_max_y(&self, planet_model: &PlanetModel) -> bool {
        match self.max_y {
            None => false,
            Some(v) => planet_model.get_maximum_y_value() - v < Vector::MINIMUM_RESOLUTION,
        }
    }

    /// `isSmallestMinZ(PlanetModel)`.
    pub fn is_smallest_min_z(&self, planet_model: &PlanetModel) -> bool {
        match self.min_z {
            None => false,
            Some(v) => v - planet_model.get_minimum_z_value() < Vector::MINIMUM_RESOLUTION,
        }
    }

    /// `isLargestMaxZ(PlanetModel)`.
    pub fn is_largest_max_z(&self, planet_model: &PlanetModel) -> bool {
        match self.max_z {
            None => false,
            Some(v) => planet_model.get_maximum_z_value() - v < Vector::MINIMUM_RESOLUTION,
        }
    }

    /// `overlaps(XYZBounds)`.
    pub fn overlaps(&self, bounds: &XYZBounds) -> bool {
        Self::is_corner_inside(self, bounds) || Self::is_corner_inside(bounds, self)
    }

    fn is_corner_inside(one: &XYZBounds, other: &XYZBounds) -> bool {
        let (
            Some(one_min_x),
            Some(one_max_x),
            Some(one_min_y),
            Some(one_max_y),
            Some(one_min_z),
            Some(one_max_z),
        ) = (
            one.min_x, one.max_x, one.min_y, one.max_y, one.min_z, one.max_z,
        )
        else {
            return false;
        };
        if other.min_x.is_none()
            || other.max_x.is_none()
            || other.min_y.is_none()
            || other.max_y.is_none()
            || other.min_z.is_none()
            || other.max_z.is_none()
        {
            return false;
        }
        Self::is_point_inside(other, one_min_x, one_min_y, one_min_z)
            || Self::is_point_inside(other, one_max_x, one_min_y, one_min_z)
            || Self::is_point_inside(other, one_min_x, one_max_y, one_min_z)
            || Self::is_point_inside(other, one_max_x, one_max_y, one_min_z)
            || Self::is_point_inside(other, one_min_x, one_min_y, one_max_z)
            || Self::is_point_inside(other, one_max_x, one_min_y, one_max_z)
            || Self::is_point_inside(other, one_min_x, one_max_y, one_max_z)
            || Self::is_point_inside(other, one_max_x, one_max_y, one_max_z)
    }

    fn is_point_inside(other: &XYZBounds, x: f64, y: f64, z: f64) -> bool {
        ub(other.min_x) <= x
            && ub(other.max_x) >= x
            && ub(other.min_y) <= y
            && ub(other.max_y) >= y
            && ub(other.min_z) <= z
            && ub(other.max_z) >= z
    }

    /// `addBounds(XYZBounds)`: widens `bounds` by these bounds.
    pub fn add_bounds(&self, bounds: &mut XYZBounds) {
        if bounds.max_x.is_none() || ub(self.max_x) > ub(bounds.max_x) {
            bounds.max_x = self.max_x;
        }
        if bounds.min_x.is_none() || ub(self.min_x) < ub(bounds.min_x) {
            bounds.min_x = self.min_x;
        }
        if bounds.max_y.is_none() || ub(self.max_y) > ub(bounds.max_y) {
            bounds.max_y = self.max_y;
        }
        if bounds.min_y.is_none() || ub(self.min_y) < ub(bounds.min_y) {
            bounds.min_y = self.min_y;
        }
        if bounds.max_z.is_none() || ub(self.max_z) > ub(bounds.max_z) {
            bounds.max_z = self.max_z;
        }
        if bounds.min_z.is_none() || ub(self.min_z) < ub(bounds.min_z) {
            bounds.min_z = self.min_z;
        }
    }

    /// `addXValue(double)`.
    pub fn add_x_value_f64(&mut self, x: f64) -> &mut dyn Bounds {
        let small = x - Self::FUDGE_FACTOR;
        if self.min_x.is_none_or(|m| m > small) {
            self.min_x = Some(small);
        }
        let large = x + Self::FUDGE_FACTOR;
        if self.max_x.is_none_or(|m| m < large) {
            self.max_x = Some(large);
        }
        self
    }

    /// `addYValue(double)`.
    pub fn add_y_value_f64(&mut self, y: f64) -> &mut dyn Bounds {
        let small = y - Self::FUDGE_FACTOR;
        if self.min_y.is_none_or(|m| m > small) {
            self.min_y = Some(small);
        }
        let large = y + Self::FUDGE_FACTOR;
        if self.max_y.is_none_or(|m| m < large) {
            self.max_y = Some(large);
        }
        self
    }

    /// `addZValue(double)`.
    pub fn add_z_value_f64(&mut self, z: f64) -> &mut dyn Bounds {
        let small = z - Self::FUDGE_FACTOR;
        if self.min_z.is_none_or(|m| m > small) {
            self.min_z = Some(small);
        }
        let large = z + Self::FUDGE_FACTOR;
        if self.max_z.is_none_or(|m| m < large) {
            self.max_z = Some(large);
        }
        self
    }

    /// `isWithin(Vector)`.
    pub fn is_within(&self, v: &Vector) -> bool {
        self.is_within_xyz(v.x, v.y, v.z)
    }

    /// `isWithin(double, double, double)`.
    pub fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.min_x.is_some_and(|v| x >= v)
            && self.max_x.is_some_and(|v| x <= v)
            && self.min_y.is_some_and(|v| y >= v)
            && self.max_y.is_some_and(|v| y <= v)
            && self.min_z.is_some_and(|v| z >= v)
            && self.max_z.is_some_and(|v| z <= v)
    }
}

impl Bounds for XYZBounds {
    fn add_plane(
        &mut self,
        planet_model: &PlanetModel,
        plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        plane.record_bounds_xyz(planet_model, self, bounds);
        self
    }

    fn add_horizontal_plane(
        &mut self,
        planet_model: &PlanetModel,
        _latitude: f64,
        horizontal_plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        self.add_plane(planet_model, horizontal_plane, bounds)
    }

    fn add_vertical_plane(
        &mut self,
        planet_model: &PlanetModel,
        _longitude: f64,
        vertical_plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        self.add_plane(planet_model, vertical_plane, bounds)
    }

    fn add_x_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        self.add_x_value_f64(point.x)
    }

    fn add_y_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        self.add_y_value_f64(point.y)
    }

    fn add_z_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        self.add_z_value_f64(point.z)
    }

    fn add_intersection(
        &mut self,
        planet_model: &PlanetModel,
        plane1: &Plane,
        plane2: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        plane1.record_bounds_intersection_xyz(planet_model, self, plane2, bounds);
        self
    }

    fn add_point(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        self.add_x_value(point);
        self.add_y_value(point);
        self.add_z_value(point)
    }

    fn is_wide(&mut self) -> &mut dyn Bounds {
        self
    }

    fn no_longitude_bound(&mut self) -> &mut dyn Bounds {
        self
    }

    fn no_top_latitude_bound(&mut self) -> &mut dyn Bounds {
        self
    }

    fn no_bottom_latitude_bound(&mut self) -> &mut dyn Bounds {
        self
    }

    fn no_bound(&mut self, planet_model: &PlanetModel) -> &mut dyn Bounds {
        self.min_x = Some(planet_model.get_minimum_x_value());
        self.max_x = Some(planet_model.get_maximum_x_value());
        self.min_y = Some(planet_model.get_minimum_y_value());
        self.max_y = Some(planet_model.get_maximum_y_value());
        self.min_z = Some(planet_model.get_minimum_z_value());
        self.max_z = Some(planet_model.get_maximum_z_value());
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl fmt::Display for XYZBounds {
    /// `XYZBounds.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "XYZBounds: [xmin={} xmax={} ymin={} ymax={} zmin={} zmax={}]",
            od2s(self.min_x),
            od2s(self.max_x),
            od2s(self.min_y),
            od2s(self.max_y),
            od2s(self.min_z),
            od2s(self.max_z)
        )
    }
}
