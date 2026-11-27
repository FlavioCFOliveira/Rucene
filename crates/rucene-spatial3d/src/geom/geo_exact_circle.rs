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

//! The types declared in `GeoExactCircle.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::f64::consts::PI;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::DistanceStyle;
use super::geo_area_shape::GeoAreaShape;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_circle::GeoCircle;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::geo_sizeable::GeoSizeable;
use super::list_to_string;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::vector::Vector;
use super::{d2s, iae};

/// Circular area with a center and radius, built from plane slices to an
/// accuracy.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoExactCircle`.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub struct GeoExactCircle {
    planet_model: Arc<PlanetModel>,
    center: GeoPoint,
    radius: f64,
    actual_accuracy: f64,
    edge_points: [GeoPoint; 1],
    circle_slices: Vec<CircleSlice>,
}

impl GeoExactCircle {
    /// `new GeoExactCircle(PlanetModel, double lat, double lon, double radius,
    /// double accuracy)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for out-of-bounds arguments or a circle
    /// that cannot be sliced.
    // Java's late-assigned local.
    #[allow(clippy::needless_late_init)]
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        lat: f64,
        lon: f64,
        radius: f64,
        accuracy: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if lat < -PI * 0.5 || lat > PI * 0.5 {
            return Err(iae("Latitude out of bounds"));
        }
        if lon < -PI || lon > PI {
            return Err(iae("Longitude out of bounds"));
        }
        if radius < 0.0 {
            return Err(iae("Radius out of bounds"));
        }
        if radius < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Radius cannot be effectively zero"));
        }
        if pm.minimum_pole_distance - radius < Vector::MINIMUM_RESOLUTION {
            return Err(iae(format!(
                "Radius out of bounds. It cannot be bigger than {} for this planet model",
                d2s(pm.minimum_pole_distance)
            )));
        }
        let center = GeoPoint::new_lat_lon(pm, lat, lon)?;
        let actual_accuracy = if accuracy < Vector::MINIMUM_RESOLUTION {
            Vector::MINIMUM_RESOLUTION
        } else {
            accuracy
        };
        let mut slices: Vec<ApproximationSlice> = Vec::with_capacity(100);
        let north_point = pm.surface_point_on_bearing(&center, radius, 0.0)?;
        let south_point = pm.surface_point_on_bearing(&center, radius, PI)?;
        let east_point = pm.surface_point_on_bearing(&center, radius, PI * 0.5)?;
        let west_point = pm.surface_point_on_bearing(&center, radius, PI * 1.5)?;
        let edge_point;
        if pm.z_scaling > pm.xy_scaling {
            slices.push(ApproximationSlice::new(
                &center,
                east_point.clone(),
                PI * 0.5,
                west_point.clone(),
                PI * -0.5,
                north_point.clone(),
                0.0,
                true,
            )?);
            slices.push(ApproximationSlice::new(
                &center,
                west_point.clone(),
                PI * 1.5,
                east_point.clone(),
                PI * 0.5,
                south_point.clone(),
                PI,
                true,
            )?);
            edge_point = east_point;
        } else {
            slices.push(ApproximationSlice::new(
                &center,
                north_point.clone(),
                0.0,
                south_point.clone(),
                PI,
                east_point.clone(),
                PI * 0.5,
                true,
            )?);
            slices.push(ApproximationSlice::new(
                &center,
                south_point.clone(),
                PI,
                north_point.clone(),
                PI * 2.0,
                west_point.clone(),
                PI * 1.5,
                true,
            )?);
            edge_point = north_point;
        }
        let mut circle_slices = Vec::new();
        while let Some(this_slice) = slices.pop() {
            let interp_point1_bearing =
                (this_slice.point1_bearing + this_slice.middle_point_bearing) * 0.5;
            let interp_point1 =
                pm.surface_point_on_bearing(&center, radius, interp_point1_bearing)?;
            let interp_point2_bearing =
                (this_slice.point2_bearing + this_slice.middle_point_bearing) * 0.5;
            let interp_point2 =
                pm.surface_point_on_bearing(&center, radius, interp_point2_bearing)?;
            if !this_slice.must_split
                && this_slice.plane.evaluate(&interp_point1).abs() < actual_accuracy
                && this_slice.plane.evaluate(&interp_point2).abs() < actual_accuracy
            {
                circle_slices.push(CircleSlice::new(
                    this_slice.plane.clone(),
                    this_slice.end_point1.clone(),
                    this_slice.end_point2.clone(),
                    &center,
                    &this_slice.middle_point,
                )?);
            } else {
                slices.push(ApproximationSlice::new(
                    &center,
                    this_slice.end_point1.clone(),
                    this_slice.point1_bearing,
                    this_slice.middle_point.clone(),
                    this_slice.middle_point_bearing,
                    interp_point1,
                    interp_point1_bearing,
                    false,
                )?);
                slices.push(ApproximationSlice::new(
                    &center,
                    this_slice.middle_point.clone(),
                    this_slice.middle_point_bearing,
                    this_slice.end_point2.clone(),
                    this_slice.point2_bearing,
                    interp_point2,
                    interp_point2_bearing,
                    false,
                )?);
            }
        }
        Ok(Self {
            planet_model: pm.clone(),
            center,
            radius,
            actual_accuracy,
            edge_points: [edge_point],
            circle_slices,
        })
    }

    /// `new GeoExactCircle(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        let b = so::read_double(input)?;
        let c = so::read_double(input)?;
        let d = so::read_double(input)?;
        Self::new(planet_model, a, b, c, d)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.center.get_latitude())?;
        so::write_double(output, self.center.get_longitude())?;
        so::write_double(output, self.radius)?;
        so::write_double(output, self.actual_accuracy)
    }

    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(ds.compute_distance_xyz(&self.center, x, y, z))
    }

    fn delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        // Inherited `GeoBaseDistanceShape.deltaDistance`.
        super::geo_base_distance_shape::GeoBaseDistanceShape::delta_distance(
            self.distance(ds, x, y, z),
        )
    }

    fn distance_bounds(
        &self,
        bounds: &mut dyn Bounds,
        _ds: &dyn DistanceStyle,
        _distance_value: f64,
    ) -> Result<()> {
        self.get_bounds(bounds);
        Ok(())
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let mut outside_distance = f64::INFINITY;
        for slice in &self.circle_slices {
            let distance = ds.compute_plane_distance_xyz(
                &self.planet_model,
                &slice.circle_plane,
                x,
                y,
                z,
                &[&slice.plane1, &slice.plane2],
            );
            if distance < outside_distance {
                outside_distance = distance;
            }
        }
        Ok(outside_distance)
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.center == self.center
            && other.radius == self.radius
            && other.actual_accuracy == self.actual_accuracy
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.center.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(super::dhash(self.radius));
        result = result
            .wrapping_mul(31)
            .wrapping_add(super::dhash(self.actual_accuracy));
        result
    }
}

impl_geo_base_area_shape_common!(
    GeoExactCircle,
    "org.apache.lucene.spatial3d.geom.GeoExactCircle",
    geo_circle
);
impl_geo_base_area_relationship!(GeoExactCircle);
impl_geo_base_distance_shape!(GeoExactCircle);

impl Membership for GeoExactCircle {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        for slice in &self.circle_slices {
            if slice.circle_plane.is_within_xyz(x, y, z)
                && slice.plane1.is_within_xyz(x, y, z)
                && slice.plane2.is_within_xyz(x, y, z)
            {
                return true;
            }
        }
        false
    }
}

impl Bounded for GeoExactCircle {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(self, &self.planet_model, bounds);
        bounds.add_point(&self.center);
        for slice in &self.circle_slices {
            bounds.add_plane(
                &self.planet_model,
                &slice.circle_plane,
                &[&slice.plane1, &slice.plane2],
            );
            for point in &slice.notable_edge_points {
                bounds.add_point(point);
            }
        }
    }
}

impl GeoShape for GeoExactCircle {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(
        &self,
        p: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        for slice in &self.circle_slices {
            if slice.circle_plane.intersects(
                &self.planet_model,
                p,
                notable_points,
                &slice.notable_edge_points,
                bounds,
                &[&slice.plane1, &slice.plane2],
            ) {
                return true;
            }
        }
        false
    }
}

impl GeoAreaShape for GeoExactCircle {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        for slice in &self.circle_slices {
            if geo_shape.intersects(
                &slice.circle_plane,
                &slice.notable_edge_points,
                &[&slice.plane1, &slice.plane2],
            ) {
                return true;
            }
        }
        false
    }
}

impl GeoSizeable for GeoExactCircle {
    fn get_radius(&self) -> f64 {
        self.radius
    }

    fn get_center(&self) -> GeoPoint {
        self.center.clone()
    }
}

impl GeoCircle for GeoExactCircle {}

impl fmt::Display for GeoExactCircle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoExactCircle: {{planetmodel={}, center={}, radius={}({}), accuracy={}}}",
            self.planet_model,
            self.center,
            d2s(self.radius),
            d2s(self.radius * 180.0 / PI),
            d2s(self.actual_accuracy)
        )
    }
}

/// A temporary description of a section of circle.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoExactCircle.ApproximationSlice`.
pub struct ApproximationSlice {
    /// The plane of the slice.
    pub plane: SidedPlane,
    /// The first end point.
    pub end_point1: GeoPoint,
    /// The bearing of the first end point.
    pub point1_bearing: f64,
    /// The second end point.
    pub end_point2: GeoPoint,
    /// The bearing of the second end point.
    pub point2_bearing: f64,
    /// The middle point.
    pub middle_point: GeoPoint,
    /// The bearing of the middle point.
    pub middle_point_bearing: f64,
    /// Whether the slice must be split.
    pub must_split: bool,
}

impl ApproximationSlice {
    /// `new ApproximationSlice(GeoPoint center, GeoPoint endPoint1, double
    /// point1Bearing, GeoPoint endPoint2, double point2Bearing, GeoPoint
    /// middlePoint, double middlePointBearing, boolean mustSplit)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when no valid plane can be built.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        center: &GeoPoint,
        end_point1: GeoPoint,
        point1_bearing: f64,
        end_point2: GeoPoint,
        point2_bearing: f64,
        middle_point: GeoPoint,
        middle_point_bearing: f64,
        must_split: bool,
    ) -> Result<Self> {
        let Some(plane) = SidedPlane::construct_normalized_three_point_sided_plane(
            center,
            &end_point1,
            &end_point2,
            &middle_point,
        ) else {
            return Err(iae(format!(
                "Either circle is too small or accuracy is too high; could not construct a plane with endPoint1={} bearing {}, endPoint2={} bearing {}, middle={} bearing {}",
                end_point1,
                d2s(point1_bearing),
                end_point2,
                d2s(point2_bearing),
                middle_point,
                d2s(middle_point_bearing)
            )));
        };
        if plane.is_within_xyz(-center.x, -center.y, -center.z) {
            return Err(iae(format!(
                "Could not construct a valid plane for this planet model with endPoint1={} bearing {}, endPoint2={} bearing {}, middle={} bearing {}",
                end_point1,
                d2s(point1_bearing),
                end_point2,
                d2s(point2_bearing),
                middle_point,
                d2s(middle_point_bearing)
            )));
        }
        Ok(Self {
            plane,
            end_point1,
            point1_bearing,
            end_point2,
            point2_bearing,
            middle_point,
            middle_point_bearing,
            must_split,
        })
    }
}

impl fmt::Display for ApproximationSlice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{end point 1 = {} bearing 1 = {} end point 2 = {} bearing 2 = {} middle point = {} middle bearing = {}}}",
            self.end_point1,
            d2s(self.point1_bearing),
            self.end_point2,
            d2s(self.point2_bearing),
            self.middle_point,
            d2s(self.middle_point_bearing)
        )
    }
}

/// A description of a section of circle.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoExactCircle.CircleSlice`.
pub struct CircleSlice {
    notable_edge_points: [GeoPoint; 2],
    /// The plane of the circle section.
    pub circle_plane: SidedPlane,
    /// The first bounding plane.
    pub plane1: SidedPlane,
    /// The second bounding plane.
    pub plane2: SidedPlane,
}

impl CircleSlice {
    /// `new CircleSlice(SidedPlane circlePlane, GeoPoint endPoint1, GeoPoint
    /// endPoint2, GeoPoint center, GeoPoint check)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a bounding plane cannot be sided.
    pub fn new(
        circle_plane: SidedPlane,
        end_point1: GeoPoint,
        end_point2: GeoPoint,
        center: &GeoPoint,
        check: &GeoPoint,
    ) -> Result<Self> {
        let plane1 = SidedPlane::new_cross(check, &end_point1, center)?;
        let plane2 = SidedPlane::new_cross(check, &end_point2, center)?;
        Ok(Self {
            notable_edge_points: [end_point1, end_point2],
            circle_plane,
            plane1,
            plane2,
        })
    }
}

impl fmt::Display for CircleSlice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{circle plane = {} plane 1 = {} plane 2 = {} notable edge points = {}}}",
            self.circle_plane,
            self.plane1,
            self.plane2,
            list_to_string(&self.notable_edge_points)
        )
    }
}
