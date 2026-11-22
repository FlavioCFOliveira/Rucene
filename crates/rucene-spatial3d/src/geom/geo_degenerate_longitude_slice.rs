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


//! The types declared in `GeoDegenerateLongitudeSlice.java`, of the Java package
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
use super::geo_b_box::GeoBBox;
use super::geo_b_box_factory::GeoBBoxFactory;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::geo_sizeable::GeoSizeable;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::{d2s, iae, java_min};
use super::geo_area::{self, GeoArea};

/// Degenerate longitude slice.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegenerateLongitudeSlice`.
pub(crate) struct GeoDegenerateLongitudeSlice {
    planet_model: Arc<PlanetModel>,
    longitude: f64,
    bounding_plane: SidedPlane,
    plane: Plane,
    interior_point: GeoPoint,
    edge_points: [GeoPoint; 1],
    plane_points: [GeoPoint; 2],
}

impl GeoDegenerateLongitudeSlice {
    /// `new GeoDegenerateLongitudeSlice(PlanetModel, double longitude)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the longitude is out of range.
    pub fn new(planet_model: &Arc<PlanetModel>, longitude: f64) -> Result<Self> {
        let pm = planet_model;
        if longitude < -PI || longitude > PI {
            return Err(iae("Longitude out of range"));
        }
        let sin_longitude = longitude.sin();
        let cos_longitude = longitude.cos();
        let plane = Plane::new_vertical(cos_longitude, sin_longitude);
        let interior_point = GeoPoint::new_sin_cos(pm, 0.0, sin_longitude, 1.0, cos_longitude);
        let bounding_plane = SidedPlane::new_vertical(&interior_point, -sin_longitude, cos_longitude)?;
        Ok(Self {
            planet_model: pm.clone(),
            longitude,
            bounding_plane,
            plane,
            edge_points: [interior_point.clone()],
            interior_point,
            plane_points: [pm.north_pole.clone(), pm.south_pole.clone()],
        })
    }

    /// `new GeoDegenerateLongitudeSlice(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        Self::new(planet_model, a)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.longitude)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let pm = &self.planet_model;
        let distance = ds.compute_plane_distance_xyz(pm, &self.plane, x, y, z, &[&self.bounding_plane]);
        let north_distance = ds.compute_distance_xyz(&pm.north_pole, x, y, z);
        let south_distance = ds.compute_distance_xyz(&pm.south_pole, x, y, z);
        Ok(java_min(distance, java_min(north_distance, south_distance)))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model) && other.longitude == self.longitude
    }

    fn hash_code_impl(&self) -> i32 {
        let result = BasePlanetObject::hash_code(&self.planet_model);
        result.wrapping_mul(31).wrapping_add(super::dhash(self.longitude))
    }
}

impl_geo_base_area_shape_common!(GeoDegenerateLongitudeSlice, "org.apache.lucene.spatial3d.geom.GeoDegenerateLongitudeSlice", geo_b_box);

impl GeoArea for GeoDegenerateLongitudeSlice {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        if self.intersects_shape(path) {
            return Ok(geo_area::OVERLAPS);
        }
        if path.is_within(&self.interior_point) {
            return Ok(geo_area::CONTAINS);
        }
        Ok(geo_area::DISJOINT)
    }
}

impl Membership for GeoDegenerateLongitudeSlice {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.plane.evaluate_is_zero_xyz(x, y, z) && self.bounding_plane.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoDegenerateLongitudeSlice {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .add_vertical_plane(pm, self.longitude, &self.plane, &[&self.bounding_plane])
            .add_point(&pm.north_pole)
            .add_point(&pm.south_pole);
    }
}

impl GeoShape for GeoDegenerateLongitudeSlice {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        p.intersects(&self.planet_model, &self.plane, notable_points, &self.plane_points, bounds, &[&self.bounding_plane])
    }
}

impl GeoAreaShape for GeoDegenerateLongitudeSlice {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(&self.plane, &self.plane_points, &[&self.bounding_plane])
    }
}

impl GeoSizeable for GeoDegenerateLongitudeSlice {
    fn get_radius(&self) -> f64 {
        PI * 0.5
    }

    fn get_center(&self) -> GeoPoint {
        self.interior_point.clone()
    }
}

impl GeoBBox for GeoDegenerateLongitudeSlice {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let mut new_left_lon = self.longitude - angle;
        let mut new_right_lon = self.longitude + angle;
        let current_lon_span = 2.0 * angle;
        if current_lon_span + 2.0 * angle >= PI * 2.0 {
            new_left_lon = -PI;
            new_right_lon = PI;
        }
        GeoBBoxFactory::make_geo_bbox(&self.planet_model, PI * 0.5, -PI * 0.5, new_left_lon, new_right_lon)
    }
}

impl fmt::Display for GeoDegenerateLongitudeSlice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoDegenerateLongitudeSlice: {{planetmodel={}, longitude={}({})}}",
            self.planet_model,
            d2s(self.longitude),
            d2s(self.longitude * 180.0 / PI)
        )
    }
}
