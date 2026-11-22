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


//! The types declared in `GeoDegeneratePath.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::{style_key, DistanceStyle};
use super::geo_area_shape::GeoAreaShape;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_path::GeoPath;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::membership::Membership;
use super::plane::{Plane, NO_BOUNDS};
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::vector::Vector;
use super::{d2s, iae, ise, java_min, list_hash_code, list_to_string, npe, rte};

/// GeoShape representing a path across the surface of the globe, with a
/// specified half-width. Path is described by a series of points. Distances
/// are measured from the starting point along the path, and then at right
/// angles to the path.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegeneratePath`.
pub(crate) struct GeoDegeneratePath {
    planet_model: Arc<PlanetModel>,
    /// The original list of path points.
    points: Vec<GeoPoint>,
    /// The list of path endpoints, `None` until `done()`.
    end_points: Option<Vec<SegmentEndpoint>>,
    /// The list of path segments, `None` until `done()`.
    segments: Option<Vec<PathSegment>>,
    /// The edge points, `None` until `done()`.
    edge_points: Option<Vec<GeoPoint>>,
    /// Set to true if `done()` has been called.
    is_done: bool,
}

impl GeoDegeneratePath {
    /// `new GeoDegeneratePath(PlanetModel, GeoPoint[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal path.
    pub fn new(planet_model: &Arc<PlanetModel>, path_points: &[GeoPoint]) -> Result<Self> {
        let mut s = Self::new_empty(planet_model);
        s.points.extend_from_slice(path_points);
        s.done()?;
        Ok(s)
    }

    /// `new GeoDegeneratePath(PlanetModel)`: a path to fill with
    /// [`add_point`](Self::add_point) and finish with [`done`](Self::done).
    pub fn new_empty(planet_model: &Arc<PlanetModel>) -> Self {
        Self {
            planet_model: planet_model.clone(),
            points: Vec::new(),
            end_points: None,
            segments: None,
            edge_points: None,
            is_done: false,
        }
    }

    /// `addPoint(double lat, double lon)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` after `done()`; `IllegalArgumentException` for
    /// an out-of-range point.
    pub fn add_point(&mut self, lat: f64, lon: f64) -> Result<()> {
        if self.is_done {
            return Err(ise("Can't call addPoint() if done() already called"));
        }
        self.points.push(GeoPoint::new_lat_lon(&self.planet_model, lat, lon)?);
        Ok(())
    }

    /// `done()`: completes the path.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when called twice; `IllegalArgumentException`
    /// for an empty or illegal path.
    pub fn done(&mut self) -> Result<()> {
        if self.is_done {
            return Err(ise("Can't call done() twice"));
        }
        if self.points.is_empty() {
            return Err(iae("Path must have at least one point"));
        }
        self.is_done = true;
        let pm = self.planet_model.clone();
        self.end_points = Some(Vec::with_capacity(self.points.len()));
        self.segments = Some(Vec::with_capacity(self.points.len()));
        let mut last_point: Option<&GeoPoint> = None;
        for end in &self.points {
            if let Some(lp) = last_point {
                let normalized_connecting_plane = Plane::new_cross(lp, end)?;
                let seg = PathSegment::new(&pm, lp.clone(), end.clone(), normalized_connecting_plane)?;
                self.segments.as_mut().unwrap_or_else(|| npe()).push(seg);
            }
            last_point = Some(end);
        }
        let segments = self.segments.as_ref().unwrap_or_else(|| npe());
        let end_points = self.end_points.as_mut().unwrap_or_else(|| npe());
        if segments.is_empty() {
            let point = self.points[0].clone();
            end_points.push(SegmentEndpoint::new(&pm, point.clone()));
            self.edge_points = Some(vec![point]);
            return Ok(());
        }
        for i in 0..segments.len() {
            let current_segment = &segments[i];
            if i == 0 {
                end_points.push(SegmentEndpoint::new_one(&pm, current_segment.start.clone(), &current_segment.start_cutoff_plane));
                self.edge_points = Some(vec![current_segment.start.clone()]);
                continue;
            }
            end_points.push(SegmentEndpoint::new_two(
                &pm,
                current_segment.start.clone(),
                &segments[i - 1].end_cutoff_plane,
                &current_segment.start_cutoff_plane,
            ));
        }
        let last_segment = &segments[segments.len() - 1];
        end_points.push(SegmentEndpoint::new_one(&pm, last_segment.end.clone(), &last_segment.end_cutoff_plane));
        Ok(())
    }

    /// `new GeoDegeneratePath(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` for an
    /// illegal path.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let points = so::read_point_array(planet_model, input)?;
        Self::new(planet_model, &points)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_point_array(output, &self.points)
    }

    fn segs(&self) -> &[PathSegment] {
        self.segments.as_deref().unwrap_or_else(|| npe())
    }

    fn eps(&self) -> &[SegmentEndpoint] {
        self.end_points.as_deref().unwrap_or_else(|| npe())
    }

    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let mut current_distance = 0.0;
        for segment in self.segs() {
            let distance = segment.path_distance(ds, x, y, z)?;
            if distance != f64::INFINITY {
                return Ok(ds.from_aggregation_form(ds.aggregate_distances(&[current_distance, distance])));
            }
            current_distance = ds.aggregate_distances(&[current_distance, segment.full_path_distance(ds)]);
        }
        let mut segment_index = 0;
        current_distance = 0.0;
        for endpoint in self.eps() {
            let distance = endpoint.path_distance(ds, x, y, z);
            if distance != f64::INFINITY {
                return Ok(ds.from_aggregation_form(ds.aggregate_distances(&[current_distance, distance])));
            }
            if segment_index < self.segs().len() {
                current_distance =
                    ds.aggregate_distances(&[current_distance, self.segs()[segment_index].full_path_distance(ds)]);
                segment_index += 1;
            }
        }
        Ok(f64::INFINITY)
    }

    fn delta_distance(&self, _ds: &dyn DistanceStyle, _x: f64, _y: f64, _z: f64) -> Result<f64> {
        Ok(0.0)
    }

    fn distance_bounds(&self, bounds: &mut dyn Bounds, _ds: &dyn DistanceStyle, _distance_value: f64) -> Result<()> {
        self.get_bounds(bounds);
        Ok(())
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let mut min_distance = f64::INFINITY;
        for endpoint in self.eps() {
            let new_distance = endpoint.outside_distance(ds, x, y, z);
            if new_distance < min_distance {
                min_distance = new_distance;
            }
        }
        for segment in self.segs() {
            let new_distance = segment.outside_distance(ds, x, y, z);
            if new_distance < min_distance {
                min_distance = new_distance;
            }
        }
        Ok(ds.from_aggregation_form(min_distance))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model) && self.points == other.points
    }

    fn hash_code_impl(&self) -> i32 {
        let result = BasePlanetObject::hash_code(&self.planet_model);
        result
            .wrapping_mul(31)
            .wrapping_add(list_hash_code(&self.points, |p| p.java_hash_code()))
    }
}

impl_geo_base_area_shape_common!(GeoDegeneratePath, "org.apache.lucene.spatial3d.geom.GeoDegeneratePath");
impl_geo_base_area_relationship!(GeoDegeneratePath);
impl_geo_base_distance_shape!(GeoDegeneratePath);

impl GeoPath for GeoDegeneratePath {
    fn compute_path_center_distance_xyz(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let mut closest_distance = f64::INFINITY;
        for segment in self.segs() {
            let segment_distance = segment.path_center_distance(ds, x, y, z)?;
            if segment_distance < closest_distance {
                closest_distance = segment_distance;
            }
        }
        for endpoint in self.eps() {
            let endpoint_distance = endpoint.path_center_distance(ds, x, y, z);
            if endpoint_distance < closest_distance {
                closest_distance = endpoint_distance;
            }
        }
        Ok(closest_distance)
    }

    fn compute_nearest_distance_xyz(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let mut current_distance = 0.0;
        let mut min_path_center_distance = f64::INFINITY;
        let mut best_distance = f64::INFINITY;
        let mut segment_index = 0;
        let segments = self.segs();
        for endpoint in self.eps() {
            let endpoint_path_center_distance = endpoint.path_center_distance(ds, x, y, z);
            if endpoint_path_center_distance < min_path_center_distance {
                min_path_center_distance = endpoint_path_center_distance;
                best_distance = current_distance;
            }
            if segment_index < segments.len() {
                let segment = &segments[segment_index];
                segment_index += 1;
                let segment_path_center_distance = segment.path_center_distance(ds, x, y, z)?;
                if segment_path_center_distance < min_path_center_distance {
                    min_path_center_distance = segment_path_center_distance;
                    best_distance = ds.aggregate_distances(&[current_distance, segment.nearest_path_distance(ds, x, y, z)?]);
                }
                current_distance = ds.aggregate_distances(&[current_distance, segment.full_path_distance(ds)]);
            }
        }
        Ok(ds.from_aggregation_form(best_distance))
    }
}

impl Membership for GeoDegeneratePath {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        for path_point in self.eps() {
            if path_point.is_within_xyz(x, y, z) {
                return true;
            }
        }
        for path_segment in self.segs() {
            if path_segment.is_within_xyz(x, y, z) {
                return true;
            }
        }
        false
    }
}

impl Bounded for GeoDegeneratePath {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(self, &self.planet_model, bounds);
        for path_segment in self.segs() {
            path_segment.get_bounds(bounds);
        }
        if self.eps().len() == 1 {
            self.eps()[0].get_bounds(bounds);
        }
    }
}

impl GeoShape for GeoDegeneratePath {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.clone().unwrap_or_else(|| npe())
    }

    fn intersects(&self, plane: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        if self.eps().len() == 1 {
            return self.eps()[0].intersects(plane, notable_points, bounds);
        }
        for path_segment in self.segs() {
            if path_segment.intersects(plane, notable_points, bounds) {
                return true;
            }
        }
        false
    }
}

impl GeoAreaShape for GeoDegeneratePath {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        if self.eps().len() == 1 {
            return self.eps()[0].intersects_shape(geo_shape);
        }
        for path_segment in self.segs() {
            if path_segment.intersects_shape(geo_shape) {
                return true;
            }
        }
        false
    }
}

impl fmt::Display for GeoDegeneratePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoDegeneratePath: {{planetmodel={}, points={{{}}}}}",
            self.planet_model,
            list_to_string(&self.points)
        )
    }
}

/// This is precalculated data for segment endpoint.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegeneratePath.SegmentEndpoint`.
/// Java's `extends GeoBaseBounds` is its `Membership` and its `getBounds`;
/// the private class is never serialized or used as a shape.
pub(crate) struct SegmentEndpoint {
    planet_model: Arc<PlanetModel>,
    /// The center point of the endpoint.
    point: GeoPoint,
    /// Pertinent cutoff planes from adjoining segments.
    cutoff_planes: Vec<SidedPlane>,
}

impl SegmentEndpoint {
    fn new(planet_model: &Arc<PlanetModel>, point: GeoPoint) -> Self {
        Self {
            planet_model: planet_model.clone(),
            point,
            cutoff_planes: Vec::new(),
        }
    }

    fn new_one(planet_model: &Arc<PlanetModel>, point: GeoPoint, cutoff_plane: &SidedPlane) -> Self {
        Self {
            planet_model: planet_model.clone(),
            point,
            cutoff_planes: vec![SidedPlane::new_inverse(cutoff_plane)],
        }
    }

    fn new_two(planet_model: &Arc<PlanetModel>, point: GeoPoint, cutoff_plane1: &SidedPlane, cutoff_plane2: &SidedPlane) -> Self {
        Self {
            planet_model: planet_model.clone(),
            point,
            cutoff_planes: vec![SidedPlane::new_inverse(cutoff_plane1), SidedPlane::new_inverse(cutoff_plane2)],
        }
    }

    /// `isWithinSection(double, double, double)`.
    fn is_within_section(&self, x: f64, y: f64, z: f64) -> bool {
        for m in &self.cutoff_planes {
            if !m.is_within_xyz(x, y, z) {
                return false;
            }
        }
        true
    }

    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        if !self.is_within_xyz(x, y, z) {
            return f64::INFINITY;
        }
        ds.to_aggregation_form(ds.compute_distance_xyz(&self.point, x, y, z))
    }

    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        if !self.is_within_section(x, y, z) {
            return f64::INFINITY;
        }
        ds.to_aggregation_form(ds.compute_distance_xyz(&self.point, x, y, z))
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        ds.to_aggregation_form(ds.compute_distance_xyz(&self.point, x, y, z))
    }

    fn intersects(&self, p: &Plane, _notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        if !p.evaluate_is_zero(&self.point) {
            return false;
        }
        for m in bounds {
            if !m.is_within(&self.point) {
                return false;
            }
        }
        true
    }

    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.is_within(&self.point)
    }

    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        bounds.add_point(&self.point);
    }

    /// `SegmentEndpoint.equals(Object)`.
    #[allow(dead_code)]
    fn java_equals(&self, other: &SegmentEndpoint) -> bool {
        self.point == other.point
    }

    /// `SegmentEndpoint.hashCode()`.
    #[allow(dead_code)]
    fn java_hash_code(&self) -> i32 {
        self.point.java_hash_code()
    }
}

impl Membership for SegmentEndpoint {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.point.is_identical_xyz(x, y, z)
    }
}

impl fmt::Display for SegmentEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = &self.planet_model;
        write!(f, "SegmentEndpoint: {}", self.point)
    }
}

/// This is the pre-calculated data for a path segment.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegeneratePath.PathSegment`.
/// Java's `extends GeoBaseBounds` is its `Membership` and its `getBounds`.
pub(crate) struct PathSegment {
    planet_model: Arc<PlanetModel>,
    /// Starting point of the segment.
    start: GeoPoint,
    /// End point of the segment.
    end: GeoPoint,
    /// Place to keep any complete segment distances we've calculated so far.
    full_distance_cache: Mutex<HashMap<TypeId, f64>>,
    /// Normalized plane connecting the two points and going through world
    /// center.
    normalized_connecting_plane: Plane,
    /// Plane going through the center and start point, marking the start
    /// edge of the segment.
    start_cutoff_plane: SidedPlane,
    /// Plane going through the center and end point, marking the end edge of
    /// the segment.
    end_cutoff_plane: SidedPlane,
    /// Notable points for the connecting plane.
    connecting_plane_points: [GeoPoint; 2],
}

impl PathSegment {
    fn new(planet_model: &Arc<PlanetModel>, start: GeoPoint, end: GeoPoint, normalized_connecting_plane: Plane) -> Result<Self> {
        let start_cutoff_plane = SidedPlane::new_cross(&end, &normalized_connecting_plane, &start)?;
        debug_assert!(start_cutoff_plane.is_within(&end));
        debug_assert!(start_cutoff_plane.is_within(&start));
        let end_cutoff_plane = SidedPlane::new_cross(&start, &normalized_connecting_plane, &end)?;
        debug_assert!(end_cutoff_plane.is_within(&start));
        debug_assert!(end_cutoff_plane.is_within(&end));
        Ok(Self {
            planet_model: planet_model.clone(),
            connecting_plane_points: [start.clone(), end.clone()],
            start,
            end,
            full_distance_cache: Mutex::new(HashMap::with_capacity(1)),
            normalized_connecting_plane,
            start_cutoff_plane,
            end_cutoff_plane,
        })
    }

    /// `fullPathDistance(DistanceStyle)`.
    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        let key = style_key(ds);
        let cached = self.full_distance_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key).copied();
        match cached {
            Some(d) => d,
            None => {
                let dist = ds.to_aggregation_form(ds.compute_distance_xyz(&self.start, self.end.x, self.end.y, self.end.z));
                self.full_distance_cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, dist);
                dist
            }
        }
    }

    /// `isWithinSection(double, double, double)`.
    fn is_within_section(&self, x: f64, y: f64, z: f64) -> bool {
        self.start_cutoff_plane.is_within_xyz(x, y, z) && self.end_cutoff_plane.is_within_xyz(x, y, z)
    }

    /// The point of the connecting plane nearest the given point, shared by
    /// the three distance methods.
    fn the_point(&self, x: f64, y: f64, z: f64, magnitude: f64, perp_x: f64, perp_y: f64, perp_z: f64) -> Result<GeoPoint> {
        let norm_factor = 1.0 / magnitude;
        let normalized_perp_plane = Plane::new(perp_x * norm_factor, perp_y * norm_factor, perp_z * norm_factor, 0.0);
        let intersection_points = self
            .normalized_connecting_plane
            .find_intersections(&self.planet_model, &normalized_perp_plane, NO_BOUNDS)
            .unwrap_or_else(|| npe());
        let world = || rte(format!("Can't find world intersection for point x={} y={} z={}", d2s(x), d2s(y), d2s(z)));
        if intersection_points.is_empty() {
            Err(world())
        } else if intersection_points.len() == 1 {
            Ok(intersection_points[0].clone())
        } else if self.start_cutoff_plane.is_within(&intersection_points[0]) && self.end_cutoff_plane.is_within(&intersection_points[0]) {
            Ok(intersection_points[0].clone())
        } else if self.start_cutoff_plane.is_within(&intersection_points[1]) && self.end_cutoff_plane.is_within(&intersection_points[1]) {
            Ok(intersection_points[1].clone())
        } else {
            Err(world())
        }
    }

    fn perp(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64, f64) {
        let p = &self.normalized_connecting_plane;
        let perp_x = p.y * z - p.z * y;
        let perp_y = p.z * x - p.x * z;
        let perp_z = p.x * y - p.y * x;
        let magnitude = (perp_x * perp_x + perp_y * perp_y + perp_z * perp_z).sqrt();
        (perp_x, perp_y, perp_z, magnitude)
    }

    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_section(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let (perp_x, perp_y, perp_z, magnitude) = self.perp(x, y, z);
        if magnitude.abs() < Vector::MINIMUM_RESOLUTION {
            return Ok(ds.compute_distance_xyz(&self.start, x, y, z));
        }
        let the_point = self.the_point(x, y, z, magnitude, perp_x, perp_y, perp_z)?;
        Ok(ds.to_aggregation_form(ds.compute_distance_xyz(&the_point, x, y, z)))
    }

    fn nearest_path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_section(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let (perp_x, perp_y, perp_z, magnitude) = self.perp(x, y, z);
        if magnitude.abs() < Vector::MINIMUM_RESOLUTION {
            return Ok(ds.to_aggregation_form(0.0));
        }
        let the_point = self.the_point(x, y, z, magnitude, perp_x, perp_y, perp_z)?;
        Ok(ds.to_aggregation_form(ds.compute_distance_xyz(&self.start, the_point.x, the_point.y, the_point.z)))
    }

    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let (perp_x, perp_y, perp_z, magnitude) = self.perp(x, y, z);
        if magnitude.abs() < Vector::MINIMUM_RESOLUTION {
            return Ok(ds.to_aggregation_form(ds.compute_distance_xyz(&self.start, x, y, z)));
        }
        let the_point = self.the_point(x, y, z, magnitude, perp_x, perp_y, perp_z)?;
        Ok(ds.aggregate_distances(&[
            ds.to_aggregation_form(ds.compute_distance_xyz(&the_point, x, y, z)),
            ds.to_aggregation_form(ds.compute_distance_xyz(&self.start, the_point.x, the_point.y, the_point.z)),
        ]))
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        let distance = ds.compute_plane_distance_xyz(
            &self.planet_model,
            &self.normalized_connecting_plane,
            x,
            y,
            z,
            &[&self.start_cutoff_plane, &self.end_cutoff_plane],
        );
        let start_distance = ds.compute_distance_xyz(&self.start, x, y, z);
        let end_distance = ds.compute_distance_xyz(&self.end, x, y, z);
        java_min(java_min(start_distance, end_distance), distance)
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        self.normalized_connecting_plane.intersects(
            &self.planet_model,
            p,
            &self.connecting_plane_points,
            notable_points,
            bounds,
            &[&self.start_cutoff_plane, &self.end_cutoff_plane],
        )
    }

    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(
            &self.normalized_connecting_plane,
            &self.connecting_plane_points,
            &[&self.start_cutoff_plane, &self.end_cutoff_plane],
        )
    }

    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(self, &self.planet_model, bounds);
        bounds
            .add_point(&self.start)
            .add_point(&self.end)
            .add_plane(&self.planet_model, &self.normalized_connecting_plane, &[&self.start_cutoff_plane, &self.end_cutoff_plane]);
    }
}

impl Membership for PathSegment {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.start_cutoff_plane.is_within_xyz(x, y, z)
            && self.end_cutoff_plane.is_within_xyz(x, y, z)
            && self.normalized_connecting_plane.evaluate_is_zero_xyz(x, y, z)
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PathSegment: {} to {}", self.start, self.end)
    }
}
