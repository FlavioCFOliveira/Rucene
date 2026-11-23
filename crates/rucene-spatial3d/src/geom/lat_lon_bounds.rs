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

//! The types declared in `LatLonBounds.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::any::Any;
use std::f64::consts::PI;
use std::fmt;

use super::bounds::Bounds;
use super::geo_point::GeoPoint;
use super::membership::Membership;
use super::od2s;
use super::plane::Plane;
use super::planet_model::PlanetModel;

/// An object for accumulating latitude/longitude bounds information.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.LatLonBounds`; Java's
/// nullable `Double` fields are `Option<f64>`.
#[derive(Clone, Debug, Default)]
pub struct LatLonBounds {
    no_longitude_bound: bool,
    no_top_latitude_bound: bool,
    no_bottom_latitude_bound: bool,
    min_latitude: Option<f64>,
    max_latitude: Option<f64>,
    left_longitude: Option<f64>,
    right_longitude: Option<f64>,
}

impl LatLonBounds {
    /// `new LatLonBounds()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getMaxLatitude()`.
    pub fn get_max_latitude(&self) -> Option<f64> {
        self.max_latitude
    }

    /// `getMinLatitude()`.
    pub fn get_min_latitude(&self) -> Option<f64> {
        self.min_latitude
    }

    /// `getLeftLongitude()`.
    pub fn get_left_longitude(&self) -> Option<f64> {
        self.left_longitude
    }

    /// `getRightLongitude()`.
    pub fn get_right_longitude(&self) -> Option<f64> {
        self.right_longitude
    }

    /// `checkNoLongitudeBound()`.
    pub fn check_no_longitude_bound(&self) -> bool {
        self.no_longitude_bound
    }

    /// `checkNoTopLatitudeBound()`.
    pub fn check_no_top_latitude_bound(&self) -> bool {
        self.no_top_latitude_bound
    }

    /// `checkNoBottomLatitudeBound()`.
    pub fn check_no_bottom_latitude_bound(&self) -> bool {
        self.no_bottom_latitude_bound
    }

    fn add_latitude_bound(&mut self, latitude: f64) {
        if !self.no_top_latitude_bound && self.max_latitude.is_none_or(|m| latitude > m) {
            self.max_latitude = Some(latitude);
        }
        if !self.no_bottom_latitude_bound && self.min_latitude.is_none_or(|m| latitude < m) {
            self.min_latitude = Some(latitude);
        }
    }

    fn add_longitude_bound(&mut self, mut longitude: f64) {
        if self.left_longitude.is_none() && self.right_longitude.is_none() {
            self.left_longitude = Some(longitude);
            self.right_longitude = Some(longitude);
        } else {
            let left = self.left_longitude.unwrap_or_else(|| super::npe());
            let right = self.right_longitude.unwrap_or_else(|| super::npe());
            let current_left_longitude = left;
            let mut current_right_longitude = right;
            if current_right_longitude < current_left_longitude {
                current_right_longitude += 2.0 * PI;
            }
            if longitude < current_left_longitude {
                longitude += 2.0 * PI;
            }
            if longitude < current_left_longitude || longitude > current_right_longitude {
                let left_extension_amt = if longitude < current_left_longitude {
                    current_left_longitude - longitude
                } else {
                    current_left_longitude + 2.0 * PI - longitude
                };
                let right_extension_amt = if longitude > current_right_longitude {
                    longitude - current_right_longitude
                } else {
                    longitude + 2.0 * PI - current_right_longitude
                };
                if left_extension_amt < right_extension_amt {
                    let mut new_left = left - left_extension_amt;
                    while new_left <= -PI {
                        new_left += 2.0 * PI;
                    }
                    self.left_longitude = Some(new_left);
                } else {
                    let mut new_right = right + right_extension_amt;
                    while new_right > PI {
                        new_right -= 2.0 * PI;
                    }
                    self.right_longitude = Some(new_right);
                }
            }
        }
        let left = self.left_longitude.unwrap_or_else(|| super::npe());
        let mut test_right_longitude = self.right_longitude.unwrap_or_else(|| super::npe());
        if test_right_longitude < left {
            test_right_longitude += PI * 2.0;
        }
        if test_right_longitude - left >= PI {
            self.no_longitude_bound = true;
            self.left_longitude = None;
            self.right_longitude = None;
        }
    }
}

impl Bounds for LatLonBounds {
    fn add_plane(
        &mut self,
        planet_model: &PlanetModel,
        plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        plane.record_bounds_lat_lon(planet_model, self, bounds);
        self
    }

    fn add_horizontal_plane(
        &mut self,
        _planet_model: &PlanetModel,
        latitude: f64,
        _horizontal_plane: &Plane,
        _bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        if !self.no_top_latitude_bound || !self.no_bottom_latitude_bound {
            self.add_latitude_bound(latitude);
        }
        self
    }

    fn add_vertical_plane(
        &mut self,
        _planet_model: &PlanetModel,
        longitude: f64,
        _vertical_plane: &Plane,
        _bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        if !self.no_longitude_bound {
            self.add_longitude_bound(longitude);
        }
        self
    }

    fn is_wide(&mut self) -> &mut dyn Bounds {
        self.no_longitude_bound()
    }

    fn add_x_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        if !self.no_longitude_bound {
            self.add_longitude_bound(point.get_longitude());
        }
        self
    }

    fn add_y_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        if !self.no_longitude_bound {
            self.add_longitude_bound(point.get_longitude());
        }
        self
    }

    fn add_z_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        if !self.no_top_latitude_bound || !self.no_bottom_latitude_bound {
            let latitude = point.get_latitude();
            self.add_latitude_bound(latitude);
        }
        self
    }

    fn add_intersection(
        &mut self,
        planet_model: &PlanetModel,
        plane1: &Plane,
        plane2: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds {
        plane1.record_bounds_intersection_lat_lon(planet_model, self, plane2, bounds);
        self
    }

    fn add_point(&mut self, point: &GeoPoint) -> &mut dyn Bounds {
        if !self.no_longitude_bound {
            self.add_longitude_bound(point.get_longitude());
        }
        if !self.no_top_latitude_bound || !self.no_bottom_latitude_bound {
            self.add_latitude_bound(point.get_latitude());
        }
        self
    }

    fn no_longitude_bound(&mut self) -> &mut dyn Bounds {
        self.no_longitude_bound = true;
        self.left_longitude = None;
        self.right_longitude = None;
        self
    }

    fn no_top_latitude_bound(&mut self) -> &mut dyn Bounds {
        self.no_top_latitude_bound = true;
        self.max_latitude = None;
        self
    }

    fn no_bottom_latitude_bound(&mut self) -> &mut dyn Bounds {
        self.no_bottom_latitude_bound = true;
        self.min_latitude = None;
        self
    }

    fn no_bound(&mut self, _planet_model: &PlanetModel) -> &mut dyn Bounds {
        self.no_longitude_bound();
        self.no_top_latitude_bound();
        self.no_bottom_latitude_bound()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl fmt::Display for LatLonBounds {
    /// `LatLonBounds.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nb = |no: bool, v: Option<f64>| {
            if no {
                "no bound".to_string()
            } else {
                od2s(v)
            }
        };
        write!(
            f,
            "LatLonBounds [minLat={}, maxLat={}, leftLon={}, rightLon={}]",
            nb(self.no_bottom_latitude_bound, self.min_latitude),
            nb(self.no_top_latitude_bound, self.max_latitude),
            nb(self.no_longitude_bound, self.left_longitude),
            nb(self.no_longitude_bound, self.right_longitude)
        )
    }
}
