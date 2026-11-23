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

//! The types declared in `GeoStandardPath.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::util::java_math;
use std::any::TypeId;
use std::collections::HashMap;
use std::f64::consts::PI;
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
use super::xyz_bounds::XYZBounds;
use super::{d2s, iae, ise, java_min, list_hash_code, list_to_string, npe, rte};

/// GeoShape representing a path across the surface of the globe, with a
/// specified half-width. Path is described by a series of points. Distances
/// are measured from the starting point along the path, and then at right
/// angles to the path.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath`. The path
/// components (segments, endpoints and tree nodes) are shared [`Arc`]s, as
/// Java's object references; there are no reference cycles.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub struct GeoStandardPath {
    planet_model: Arc<PlanetModel>,
    /// The cutoff angle (width).
    cutoff_angle: f64,
    /// Sine of cutoff angle.
    sin_angle: f64,
    /// Cosine of cutoff angle.
    cos_angle: f64,
    /// The original list of path points.
    points: Vec<GeoPoint>,
    /// The root of the path component tree.
    root_component: Option<Arc<dyn PathComponent>>,
    /// The edge points, `None` until `done()`; an element is `None` where
    /// Java stores a `null` point.
    edge_points: Option<Vec<Option<GeoPoint>>>,
    /// Set to true if `done()` has been called.
    is_done: bool,
}

impl GeoStandardPath {
    /// `new GeoStandardPath(PlanetModel, double maxCutoffAngle, GeoPoint[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal path.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        max_cutoff_angle: f64,
        path_points: &[GeoPoint],
    ) -> Result<Self> {
        let mut s = Self::new_empty(planet_model, max_cutoff_angle)?;
        s.points.extend_from_slice(path_points);
        s.done()?;
        Ok(s)
    }

    /// `new GeoStandardPath(PlanetModel, double maxCutoffAngle)`: a path to
    /// fill with [`add_point`](Self::add_point) and finish with
    /// [`done`](Self::done).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the cutoff angle is out of bounds.
    pub fn new_empty(planet_model: &Arc<PlanetModel>, max_cutoff_angle: f64) -> Result<Self> {
        if max_cutoff_angle <= 0.0 || max_cutoff_angle > PI * 0.5 {
            return Err(iae("Cutoff angle out of bounds"));
        }
        Ok(Self {
            planet_model: planet_model.clone(),
            cutoff_angle: max_cutoff_angle,
            cos_angle: java_math::cos(max_cutoff_angle),
            sin_angle: java_math::sin(max_cutoff_angle),
            points: Vec::new(),
            root_component: None,
            edge_points: None,
            is_done: false,
        })
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
        self.points
            .push(GeoPoint::new_lat_lon(&self.planet_model, lat, lon)?);
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
        let pm = &self.planet_model;
        let _ = self.cos_angle;
        let mut end_points: Vec<Arc<dyn PathComponent>> = Vec::with_capacity(self.points.len());
        let mut segments: Vec<Arc<PathSegment>> = Vec::with_capacity(self.points.len());
        let cutoff_offset = self.sin_angle * pm.get_minimum_magnitude();
        let mut last_point: Option<&GeoPoint> = None;
        let mut last_component: Option<Arc<dyn PathComponent>> = None;
        for end in &self.points {
            if let Some(lp) = last_point {
                let normalized_connecting_plane = Plane::new_cross(lp, end)?;
                let new_component = Arc::new(PathSegment::new(
                    pm,
                    last_component.clone(),
                    lp.clone(),
                    end.clone(),
                    normalized_connecting_plane,
                    cutoff_offset,
                )?);
                segments.push(new_component.clone());
                last_component = Some(new_component);
            }
            last_point = Some(end);
        }
        if segments.is_empty() {
            let lat = self.points[0].get_latitude();
            let lon = self.points[0].get_longitude();
            let mut upper_lat = lat + self.cutoff_angle;
            let mut upper_lon = lon;
            if upper_lat > PI * 0.5 {
                upper_lon += PI;
                if upper_lon > PI {
                    upper_lon -= 2.0 * PI;
                }
                upper_lat = PI - upper_lat;
            }
            let mut lower_lat = lat - self.cutoff_angle;
            let mut lower_lon = lon;
            if lower_lat < -PI * 0.5 {
                lower_lon += PI;
                if lower_lon > PI {
                    lower_lon -= 2.0 * PI;
                }
                lower_lat = -PI - lower_lat;
            }
            let upper_point = GeoPoint::new_lat_lon(pm, upper_lat, upper_lon)?;
            let lower_point = GeoPoint::new_lat_lon(pm, lower_lat, lower_lon)?;
            let point = self.points[0].clone();
            let normal_plane =
                Plane::construct_normalized_z_plane_points(&[&upper_point, &lower_point, &point]);
            let only_endpoint = CircleSegmentEndpoint::new_normal(
                pm,
                None,
                point,
                normal_plane.as_ref(),
                &upper_point,
                &lower_point,
            )?;
            let edge = only_endpoint
                .circle_plane
                .as_ref()
                .unwrap_or_else(|| npe())
                .get_sample_intersection_point(pm, normal_plane.as_ref().unwrap_or_else(|| npe()));
            end_points.push(Arc::new(only_endpoint));
            self.edge_points = Some(vec![edge]);
        } else {
            for i in 0..segments.len() {
                let current_segment = &segments[i];
                if i == 0 {
                    let start_endpoint = CutoffSingleCircleSegmentEndpoint::new(
                        pm,
                        None,
                        current_segment.start.clone(),
                        &current_segment.start_cutoff_plane,
                        &current_segment.ulhc,
                        &current_segment.llhc,
                    )?;
                    end_points.push(Arc::new(start_endpoint));
                    self.edge_points = Some(vec![Some(current_segment.ulhc.clone())]);
                    continue;
                }
                let prev_segment = &segments[i - 1];
                end_points.push(Arc::new(CutoffDualCircleSegmentEndpoint::new(
                    pm,
                    Some(prev_segment.clone()),
                    current_segment.start.clone(),
                    &prev_segment.end_cutoff_plane,
                    &current_segment.start_cutoff_plane,
                    &prev_segment.urhc,
                    &prev_segment.lrhc,
                    &current_segment.ulhc,
                    &current_segment.llhc,
                )?));
            }
            let last_segment = &segments[segments.len() - 1];
            end_points.push(Arc::new(CutoffSingleCircleSegmentEndpoint::new(
                pm,
                Some(last_segment.clone()),
                last_segment.end.clone(),
                &last_segment.end_cutoff_plane,
                &last_segment.urhc,
                &last_segment.lrhc,
            )?));
        }
        let mut tree_builder = TreeBuilder::new(segments.len() + end_points.len());
        tree_builder.add_component(end_points[0].clone());
        for i in 0..segments.len() {
            tree_builder.add_component(segments[i].clone());
            tree_builder.add_component(end_points[i + 1].clone());
        }
        self.root_component = tree_builder.get_root();
        Ok(())
    }

    /// `new GeoStandardPath(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` for an
    /// illegal path.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let cutoff = so::read_double(input)?;
        let points = so::read_point_array(planet_model, input)?;
        Self::new(planet_model, cutoff, &points)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.cutoff_angle)?;
        so::write_point_array(output, &self.points)
    }

    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let Some(root) = &self.root_component else {
            return Ok(f64::INFINITY);
        };
        Ok(ds.from_aggregation_form(root.distance(ds, x, y, z)?))
    }

    fn delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let Some(root) = &self.root_component else {
            return Ok(f64::INFINITY);
        };
        Ok(ds.from_aggregation_form(root.path_delta_distance(ds, x, y, z)?))
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
        let Some(root) = &self.root_component else {
            return Ok(f64::INFINITY);
        };
        Ok(ds.from_aggregation_form(root.outside_distance(ds, x, y, z)))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && self.cutoff_angle == other.cutoff_angle
            && self.points == other.points
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(super::dhash(self.cutoff_angle));
        result = result
            .wrapping_mul(31)
            .wrapping_add(list_hash_code(&self.points, |p| p.java_hash_code()));
        result
    }
}

impl_geo_base_area_shape_common!(
    GeoStandardPath,
    "org.apache.lucene.spatial3d.geom.GeoStandardPath"
);
impl_geo_base_area_relationship!(GeoStandardPath);
impl_geo_base_distance_shape!(GeoStandardPath);

impl GeoPath for GeoStandardPath {
    fn compute_path_center_distance_xyz(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64> {
        let Some(root) = &self.root_component else {
            return Ok(f64::INFINITY);
        };
        Ok(ds.from_aggregation_form(root.path_center_distance(ds, x, y, z)?))
    }

    fn compute_nearest_distance_xyz(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64> {
        let Some(root) = &self.root_component else {
            return Ok(f64::INFINITY);
        };
        let Some(distance_pair) = root.nearest_distance(ds, x, y, z)? else {
            return Ok(f64::INFINITY);
        };
        Ok(ds.from_aggregation_form(distance_pair.distance_along_path))
    }
}

impl Membership for GeoStandardPath {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        match &self.root_component {
            None => false,
            Some(root) => root.is_within_xyz(x, y, z),
        }
    }
}

impl Bounded for GeoStandardPath {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(self, &self.planet_model, bounds);
        if let Some(root) = &self.root_component {
            root.get_bounds(bounds);
        }
    }
}

impl GeoShape for GeoStandardPath {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points
            .as_ref()
            .unwrap_or_else(|| npe())
            .iter()
            .map(|p| p.clone().unwrap_or_else(|| npe()))
            .collect()
    }

    fn intersects(
        &self,
        plane: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        let Some(root) = &self.root_component else {
            return false;
        };
        let plane_bounds: Option<&XYZBounds> = None;
        root.intersects(plane, plane_bounds, notable_points, bounds)
    }
}

impl GeoAreaShape for GeoStandardPath {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        match &self.root_component {
            None => false,
            Some(root) => root.intersects_shape(geo_shape),
        }
    }
}

impl fmt::Display for GeoStandardPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoStandardPath: {{planetmodel={}, width={}({}), points={{{}}}}}",
            self.planet_model,
            d2s(self.cutoff_angle),
            d2s(self.cutoff_angle * 180.0 / PI),
            list_to_string(&self.points)
        )
    }
}

/// Algorithms that use this class.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.DistancePair`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DistancePair {
    /// The path center distance.
    pub path_center_distance: f64,
    /// The distance along the path.
    pub distance_along_path: f64,
}

impl fmt::Display for DistancePair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DistancePair: pathCenterDistance={},distanceAlongPath={}",
            d2s(self.path_center_distance),
            d2s(self.distance_along_path)
        )
    }
}

/// Path components consist of both path segments and segment endpoints.
/// This interface links their behavior without having to know anything else
/// about them.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.PathComponent`.
/// The distance methods return `Result` for the `RuntimeException`s Java's
/// path segments throw.
pub(crate) trait PathComponent: Send + Sync + fmt::Display {
    /// `isWithin(Vector)`.
    fn is_within(&self, point: &Vector) -> bool;
    /// `isWithin(double, double, double)`.
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool;
    /// `isWithinSection(Vector)`.
    #[allow(dead_code)] // Declared by Java's interface; no caller in Lucene.
    fn is_within_section(&self, point: &Vector) -> bool;
    /// `isWithinSection(double, double, double)`.
    fn is_within_section_xyz(&self, x: f64, y: f64, z: f64) -> bool;
    /// `getStartingDistance(DistanceStyle)`.
    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64;
    /// `fullPathDistance(DistanceStyle)`.
    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64;
    /// `distance(DistanceStyle, double, double, double)`.
    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64>;
    /// `nearestDistance(DistanceStyle, double, double, double)`.
    fn nearest_distance(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>>;
    /// `pathDistance(DistanceStyle, double, double, double)`.
    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64>;
    /// `pathDeltaDistance(DistanceStyle, double, double, double)`.
    fn path_delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64>;
    /// `nearestPathDistance(DistanceStyle, double, double, double)`.
    fn nearest_path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64>;
    /// `pathCenterDistance(DistanceStyle, double, double, double)`.
    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64>;
    /// `outsideDistance(DistanceStyle, double, double, double)`.
    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64;
    /// `intersects(Plane, XYZBounds, GeoPoint[], Membership[])`.
    fn intersects(
        &self,
        p: &Plane,
        plane_bounds: Option<&XYZBounds>,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool;
    /// `intersects(GeoShape)`.
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool;
    /// `getBounds(Bounds)`.
    fn get_bounds(&self, bounds: &mut dyn Bounds);
}

/// The membership of a path component, for `GeoBaseBounds.getBounds`.
struct ComponentMembership<'a>(&'a dyn PathComponent);

impl Membership for ComponentMembership<'_> {
    fn is_within(&self, point: &Vector) -> bool {
        self.0.is_within(point)
    }
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.0.is_within_xyz(x, y, z)
    }
}

fn ub(v: Option<f64>) -> f64 {
    v.unwrap_or_else(|| npe())
}

/// Path node: a binary tree node over two path components.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.PathNode`.
pub(crate) struct PathNode {
    child1: Arc<dyn PathComponent>,
    child2: Arc<dyn PathComponent>,
    bounds: XYZBounds,
}

impl PathNode {
    fn new(child1: Arc<dyn PathComponent>, child2: Arc<dyn PathComponent>) -> Self {
        let mut bounds = XYZBounds::new();
        child1.get_bounds(&mut bounds);
        child2.get_bounds(&mut bounds);
        Self {
            child1,
            child2,
            bounds,
        }
    }
}

impl PathComponent for PathNode {
    fn is_within(&self, point: &Vector) -> bool {
        self.is_within_xyz(point.x, point.y, point.z)
    }

    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        let b = &self.bounds;
        if x < ub(b.get_minimum_x())
            || x > ub(b.get_maximum_x())
            || y < ub(b.get_minimum_y())
            || y > ub(b.get_maximum_y())
            || z < ub(b.get_minimum_z())
            || z > ub(b.get_maximum_z())
        {
            return false;
        }
        self.child1.is_within_xyz(x, y, z) || self.child2.is_within_xyz(x, y, z)
    }

    fn is_within_section(&self, point: &Vector) -> bool {
        self.child1.is_within_section(point) || self.child2.is_within_section(point)
    }

    fn is_within_section_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.child1.is_within_section_xyz(x, y, z) || self.child2.is_within_section_xyz(x, y, z)
    }

    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        self.child1.get_starting_distance(ds)
    }

    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let child1_distance = self.child1.distance(ds, x, y, z)?;
        let child2_distance = self.child2.distance(ds, x, y, z)?;
        Ok(java_min(child1_distance, child2_distance))
    }

    fn nearest_distance(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>> {
        let first_child_distance = self.child1.nearest_distance(ds, x, y, z)?;
        let second_child_distance = self.child2.nearest_distance(ds, x, y, z)?;
        let (Some(first), Some(second)) = (first_child_distance, second_child_distance) else {
            return Ok(if first_child_distance.is_none() {
                second_child_distance
            } else {
                first_child_distance
            });
        };
        Ok(Some(
            if first.path_center_distance < second.path_center_distance {
                first
            } else if second.path_center_distance < first.path_center_distance {
                second
            } else if first.distance_along_path < second.distance_along_path {
                first
            } else {
                second
            },
        ))
    }

    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        ds.aggregate_distances(&[
            self.child1.full_path_distance(ds),
            self.child2.full_path_distance(ds),
        ])
    }

    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let a = self.child1.path_distance(ds, x, y, z)?;
        let full = self.child1.full_path_distance(ds);
        let b = self.child2.path_distance(ds, x, y, z)?;
        Ok(java_min(a, ds.aggregate_distances(&[full, b])))
    }

    fn path_delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let a = self.child1.path_delta_distance(ds, x, y, z)?;
        let b = self.child2.path_delta_distance(ds, x, y, z)?;
        Ok(java_min(a, b))
    }

    fn nearest_path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let a = self.child1.nearest_path_distance(ds, x, y, z)?;
        let b = self.child2.nearest_path_distance(ds, x, y, z)?;
        Ok(java_min(a, b))
    }

    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let a = self.child1.path_center_distance(ds, x, y, z)?;
        let b = self.child2.path_center_distance(ds, x, y, z)?;
        Ok(java_min(a, b))
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        java_min(
            self.child1.outside_distance(ds, x, y, z),
            self.child2.outside_distance(ds, x, y, z),
        )
    }

    fn intersects(
        &self,
        p: &Plane,
        plane_bounds: Option<&XYZBounds>,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        if let Some(pb) = plane_bounds {
            if !pb.overlaps(&self.bounds) {
                return false;
            }
        }
        self.child1
            .intersects(p, plane_bounds, notable_points, bounds)
            || self
                .child2
                .intersects(p, plane_bounds, notable_points, bounds)
    }

    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        self.child1.intersects_shape(geo_shape) || self.child2.intersects_shape(geo_shape)
    }

    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        if let Some(xyz) = bounds.as_any_mut().downcast_mut::<XYZBounds>() {
            self.bounds.add_bounds(xyz);
        } else {
            self.child1.get_bounds(bounds);
            self.child2.get_bounds(bounds);
        }
    }
}

impl fmt::Display for PathNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PathNode ({}) ({})", self.child1, self.child2)
    }
}

/// Internal interface describing segment endpoint implementations.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.SegmentEndpoint`.
#[allow(dead_code)] // Java's marker interface; nothing names it as a type.
pub(crate) trait SegmentEndpoint: PathComponent {}

/// Base implementation of SegmentEndpoint.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.BaseSegmentEndpoint`.
/// The subclasses embed it; its behaviour is the associated functions over
/// the subclass (`this`), Java's virtual dispatch.
pub(crate) struct BaseSegmentEndpoint {
    planet_model: Arc<PlanetModel>,
    /// The previous path element.
    previous: Option<Arc<dyn PathComponent>>,
    /// The center point of the endpoint.
    point: GeoPoint,
}

impl BaseSegmentEndpoint {
    fn new(
        planet_model: &Arc<PlanetModel>,
        previous: Option<Arc<dyn PathComponent>>,
        point: GeoPoint,
    ) -> Self {
        Self {
            planet_model: planet_model.clone(),
            previous,
            point,
        }
    }

    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        match &self.previous {
            None => ds.to_aggregation_form(0.0),
            Some(previous) => ds.aggregate_distances(&[
                previous.get_starting_distance(ds),
                previous.full_path_distance(ds),
            ]),
        }
    }

    fn distance(
        this: &dyn PathComponent,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64> {
        if !this.is_within_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let starting_distance = this.get_starting_distance(ds);
        let path_distance = this.path_distance(ds, x, y, z)?;
        Ok(ds.aggregate_distances(&[starting_distance, path_distance]))
    }

    fn nearest_distance(
        this: &dyn PathComponent,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>> {
        if !this.is_within_section_xyz(x, y, z) {
            return Ok(None);
        }
        let path_center_distance = this.path_center_distance(ds, x, y, z)?;
        let starting = this.get_starting_distance(ds);
        let nearest = this.nearest_path_distance(ds, x, y, z)?;
        Ok(Some(DistancePair {
            path_center_distance,
            distance_along_path: ds.aggregate_distances(&[starting, nearest]),
        }))
    }

    fn full_path_distance(ds: &dyn DistanceStyle) -> f64 {
        ds.to_aggregation_form(0.0)
    }

    fn path_delta_distance(
        &self,
        this: &dyn PathComponent,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        if !this.is_within_xyz(x, y, z) {
            return f64::INFINITY;
        }
        let the_distance = ds.to_aggregation_form(ds.compute_distance_xyz(&self.point, x, y, z));
        ds.aggregate_distances(&[the_distance, the_distance])
    }

    fn path_distance(
        &self,
        this: &dyn PathComponent,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        if !this.is_within_xyz(x, y, z) {
            return f64::INFINITY;
        }
        ds.to_aggregation_form(ds.compute_distance_xyz(&self.point, x, y, z))
    }

    fn nearest_path_distance(
        this: &dyn PathComponent,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        if !this.is_within_section_xyz(x, y, z) {
            return f64::INFINITY;
        }
        ds.to_aggregation_form(0.0)
    }

    fn path_center_distance(
        &self,
        this: &dyn PathComponent,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        if !this.is_within_section_xyz(x, y, z) {
            return f64::INFINITY;
        }
        ds.to_aggregation_form(ds.compute_distance_xyz(&self.point, x, y, z))
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        ds.to_aggregation_form(ds.compute_distance_xyz(&self.point, x, y, z))
    }

    fn get_bounds(&self, this: &dyn PathComponent, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(&ComponentMembership(this), &self.planet_model, bounds);
        bounds.add_point(&self.point);
    }

    /// `BaseSegmentEndpoint.equals(Object)`.
    #[allow(dead_code)]
    fn java_equals(&self, other: &BaseSegmentEndpoint) -> bool {
        self.point == other.point && *self.planet_model == *other.planet_model
    }

    /// `BaseSegmentEndpoint.hashCode()`.
    #[allow(dead_code)]
    fn java_hash_code(&self) -> i32 {
        self.point
            .java_hash_code()
            .wrapping_add(BasePlanetObject::hash_code(&self.planet_model))
    }
}

impl PathComponent for BaseSegmentEndpoint {
    fn is_within(&self, _point: &Vector) -> bool {
        false
    }
    fn is_within_xyz(&self, _x: f64, _y: f64, _z: f64) -> bool {
        false
    }
    fn is_within_section(&self, _point: &Vector) -> bool {
        true
    }
    fn is_within_section_xyz(&self, _x: f64, _y: f64, _z: f64) -> bool {
        true
    }
    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        BaseSegmentEndpoint::get_starting_distance(self, ds)
    }
    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        BaseSegmentEndpoint::full_path_distance(ds)
    }
    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        BaseSegmentEndpoint::distance(self, ds, x, y, z)
    }
    fn nearest_distance(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>> {
        BaseSegmentEndpoint::nearest_distance(self, ds, x, y, z)
    }
    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(BaseSegmentEndpoint::path_distance(self, self, ds, x, y, z))
    }
    fn path_delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(BaseSegmentEndpoint::path_delta_distance(
            self, self, ds, x, y, z,
        ))
    }
    fn nearest_path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(BaseSegmentEndpoint::nearest_path_distance(
            self, ds, x, y, z,
        ))
    }
    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(BaseSegmentEndpoint::path_center_distance(
            self, self, ds, x, y, z,
        ))
    }
    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        BaseSegmentEndpoint::outside_distance(self, ds, x, y, z)
    }
    fn intersects(
        &self,
        _p: &Plane,
        _plane_bounds: Option<&XYZBounds>,
        _notable_points: &[GeoPoint],
        _bounds: &[&dyn Membership],
    ) -> bool {
        false
    }
    fn intersects_shape(&self, _geo_shape: &dyn GeoShape) -> bool {
        false
    }
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        BaseSegmentEndpoint::get_bounds(self, self, bounds);
    }
}

impl SegmentEndpoint for BaseSegmentEndpoint {}

impl fmt::Display for BaseSegmentEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SegmentEndpoint ({})", self.point)
    }
}

/// Endpoint that's a simple circle.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.CircleSegmentEndpoint`;
/// `circle_plane` is `None` where Java stores `null`.
pub(crate) struct CircleSegmentEndpoint {
    base: BaseSegmentEndpoint,
    /// A plane describing the circle.
    circle_plane: Option<SidedPlane>,
}

impl CircleSegmentEndpoint {
    /// `new CircleSegmentEndpoint(PlanetModel, PathComponent, GeoPoint point,
    /// GeoPoint upperPoint, GeoPoint lowerPoint)`.
    fn new_two_points(
        planet_model: &Arc<PlanetModel>,
        previous: Option<Arc<dyn PathComponent>>,
        point: GeoPoint,
        upper_point: &GeoPoint,
        lower_point: &GeoPoint,
    ) -> Result<Self> {
        let circle_plane =
            SidedPlane::construct_sided_plane_from_two_points(&point, upper_point, lower_point)?;
        debug_assert!(circle_plane.is_within(&point));
        Ok(Self {
            base: BaseSegmentEndpoint::new(planet_model, previous, point),
            circle_plane: Some(circle_plane),
        })
    }

    /// `new CircleSegmentEndpoint(PlanetModel, PathComponent, GeoPoint point,
    /// Plane normalPlane, GeoPoint upperPoint, GeoPoint lowerPoint)`.
    fn new_normal(
        planet_model: &Arc<PlanetModel>,
        previous: Option<Arc<dyn PathComponent>>,
        point: GeoPoint,
        normal_plane: Option<&Plane>,
        upper_point: &GeoPoint,
        lower_point: &GeoPoint,
    ) -> Result<Self> {
        let circle_plane = SidedPlane::construct_normalized_perpendicular_sided_plane(
            &point,
            normal_plane.unwrap_or_else(|| npe()),
            upper_point,
            lower_point,
        )?;
        debug_assert!(circle_plane.as_ref().is_some_and(|c| c.is_within(&point)));
        Ok(Self {
            base: BaseSegmentEndpoint::new(planet_model, previous, point),
            circle_plane,
        })
    }

    fn plane(&self) -> &SidedPlane {
        self.circle_plane.as_ref().unwrap_or_else(|| npe())
    }

    fn circle_get_bounds(&self, this: &dyn PathComponent, bounds: &mut dyn Bounds) {
        self.base.get_bounds(this, bounds);
        bounds.add_plane(&self.base.planet_model, self.plane(), &[]);
    }
}

impl PathComponent for CircleSegmentEndpoint {
    fn is_within(&self, point: &Vector) -> bool {
        self.plane().is_within(point)
    }
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.plane().is_within_xyz(x, y, z)
    }
    fn is_within_section(&self, _point: &Vector) -> bool {
        true
    }
    fn is_within_section_xyz(&self, _x: f64, _y: f64, _z: f64) -> bool {
        true
    }
    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        self.base.get_starting_distance(ds)
    }
    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        BaseSegmentEndpoint::full_path_distance(ds)
    }
    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        BaseSegmentEndpoint::distance(self, ds, x, y, z)
    }
    fn nearest_distance(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>> {
        BaseSegmentEndpoint::nearest_distance(self, ds, x, y, z)
    }
    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(self.base.path_distance(self, ds, x, y, z))
    }
    fn path_delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(self.base.path_delta_distance(self, ds, x, y, z))
    }
    fn nearest_path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(BaseSegmentEndpoint::nearest_path_distance(
            self, ds, x, y, z,
        ))
    }
    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(self.base.path_center_distance(self, ds, x, y, z))
    }
    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        self.base.outside_distance(ds, x, y, z)
    }
    fn intersects(
        &self,
        p: &Plane,
        _plane_bounds: Option<&XYZBounds>,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        self.plane()
            .intersects(&self.base.planet_model, p, notable_points, &[], bounds, &[])
    }
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(self.plane(), &[], NO_BOUNDS)
    }
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        self.circle_get_bounds(self, bounds);
    }
}

impl SegmentEndpoint for CircleSegmentEndpoint {}

impl fmt::Display for CircleSegmentEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CircleSegmentEndpoint: {}", self.base)
    }
}

/// Endpoint that's a single circle with cutoff(s).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.CutoffSingleCircleSegmentEndpoint`.
pub(crate) struct CutoffSingleCircleSegmentEndpoint {
    circle: CircleSegmentEndpoint,
    /// Notable points for this segment endpoint.
    notable_points: [GeoPoint; 2],
    /// The cutoff plane (`cutoffPlanes` is the one-element array of it).
    cutoff_plane: SidedPlane,
}

impl CutoffSingleCircleSegmentEndpoint {
    fn new(
        planet_model: &Arc<PlanetModel>,
        previous: Option<Arc<dyn PathComponent>>,
        point: GeoPoint,
        cutoff_plane: &SidedPlane,
        top_edge_point: &GeoPoint,
        bottom_edge_point: &GeoPoint,
    ) -> Result<Self> {
        let circle = CircleSegmentEndpoint::new_two_points(
            planet_model,
            previous,
            point,
            top_edge_point,
            bottom_edge_point,
        )?;
        Ok(Self {
            circle,
            cutoff_plane: SidedPlane::new_inverse(cutoff_plane),
            notable_points: [top_edge_point.clone(), bottom_edge_point.clone()],
        })
    }
}

impl PathComponent for CutoffSingleCircleSegmentEndpoint {
    fn is_within(&self, point: &Vector) -> bool {
        self.cutoff_plane.is_within(point) && self.circle.is_within(point)
    }
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.cutoff_plane.is_within_xyz(x, y, z) && self.circle.is_within_xyz(x, y, z)
    }
    fn is_within_section(&self, point: &Vector) -> bool {
        self.cutoff_plane.is_within(point)
    }
    fn is_within_section_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.cutoff_plane.is_within_xyz(x, y, z)
    }
    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        self.circle.base.get_starting_distance(ds)
    }
    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        BaseSegmentEndpoint::full_path_distance(ds)
    }
    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        BaseSegmentEndpoint::distance(self, ds, x, y, z)
    }
    fn nearest_distance(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>> {
        BaseSegmentEndpoint::nearest_distance(self, ds, x, y, z)
    }
    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(self.circle.base.path_distance(self, ds, x, y, z))
    }
    fn path_delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(self.circle.base.path_delta_distance(self, ds, x, y, z))
    }
    fn nearest_path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_section_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        Ok(BaseSegmentEndpoint::nearest_path_distance(
            self, ds, x, y, z,
        ))
    }
    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_section_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        Ok(self.circle.base.path_center_distance(self, ds, x, y, z))
    }
    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        self.circle.base.outside_distance(ds, x, y, z)
    }
    fn intersects(
        &self,
        p: &Plane,
        _plane_bounds: Option<&XYZBounds>,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        self.circle.plane().intersects(
            &self.circle.base.planet_model,
            p,
            notable_points,
            &self.notable_points,
            bounds,
            &[&self.cutoff_plane],
        )
    }
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(
            self.circle.plane(),
            &self.notable_points,
            &[&self.cutoff_plane],
        )
    }
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        self.circle.circle_get_bounds(self, bounds);
        let pm = &self.circle.base.planet_model;
        let circle_plane = self.circle.plane();
        bounds.add_plane(pm, circle_plane, &[&self.cutoff_plane]);
        bounds.add_plane(pm, &self.cutoff_plane, &[circle_plane]);
        bounds.add_intersection(pm, circle_plane, &self.cutoff_plane, &[]);
        bounds.add_intersection(pm, &self.cutoff_plane, circle_plane, &[]);
    }
}

impl SegmentEndpoint for CutoffSingleCircleSegmentEndpoint {}

impl fmt::Display for CutoffSingleCircleSegmentEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CutoffSingleCircleSegmentEndpoint: {}", self.circle)
    }
}

/// Endpoint that's a dual circle with cutoff(s). This SegmentEndpoint is
/// used when we have two adjoining segments.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.CutoffDualCircleSegmentEndpoint`;
/// a circle plane is `None` where Java stores `null`.
pub(crate) struct CutoffDualCircleSegmentEndpoint {
    base: BaseSegmentEndpoint,
    /// First circle.
    circle_plane1: Option<SidedPlane>,
    /// Second circle.
    circle_plane2: Option<SidedPlane>,
    /// Notable points for first circle.
    notable_points1: Vec<GeoPoint>,
    /// Notable points for second circle.
    notable_points2: Vec<GeoPoint>,
    /// Both cutoff planes are included here (`cutoffPlanes`).
    boundary_plane1: SidedPlane,
    boundary_plane2: SidedPlane,
}

impl CutoffDualCircleSegmentEndpoint {
    #[allow(clippy::too_many_arguments)]
    // Java's late-assigned locals.
    #[allow(clippy::needless_late_init)]
    fn new(
        planet_model: &Arc<PlanetModel>,
        previous: Option<Arc<dyn PathComponent>>,
        point: GeoPoint,
        prev_cutoff_plane: &SidedPlane,
        next_cutoff_plane: &SidedPlane,
        prev_urhc: &GeoPoint,
        prev_lrhc: &GeoPoint,
        current_ulhc: &GeoPoint,
        current_llhc: &GeoPoint,
    ) -> Result<Self> {
        let circle_plane1;
        let notable_points1;
        if !prev_cutoff_plane.is_within(current_ulhc) {
            circle_plane1 = SidedPlane::construct_normalized_three_point_sided_plane(
                &point,
                prev_urhc,
                prev_lrhc,
                current_ulhc,
            );
            notable_points1 = vec![prev_urhc.clone(), prev_lrhc.clone(), current_ulhc.clone()];
        } else if !prev_cutoff_plane.is_within(current_llhc) {
            circle_plane1 = SidedPlane::construct_normalized_three_point_sided_plane(
                &point,
                prev_urhc,
                prev_lrhc,
                current_llhc,
            );
            notable_points1 = vec![prev_urhc.clone(), prev_lrhc.clone(), current_llhc.clone()];
        } else {
            circle_plane1 = Some(SidedPlane::construct_sided_plane_from_two_points(
                &point, prev_urhc, prev_lrhc,
            )?);
            notable_points1 = vec![prev_urhc.clone(), prev_lrhc.clone()];
        }
        let circle_plane2;
        let notable_points2;
        if !next_cutoff_plane.is_within(prev_urhc) {
            circle_plane2 = SidedPlane::construct_normalized_three_point_sided_plane(
                &point,
                current_ulhc,
                current_llhc,
                prev_urhc,
            );
            notable_points2 = vec![
                current_ulhc.clone(),
                current_llhc.clone(),
                prev_urhc.clone(),
            ];
        } else if !next_cutoff_plane.is_within(prev_lrhc) {
            circle_plane2 = SidedPlane::construct_normalized_three_point_sided_plane(
                &point,
                current_ulhc,
                current_llhc,
                prev_lrhc,
            );
            notable_points2 = vec![
                current_ulhc.clone(),
                current_llhc.clone(),
                prev_lrhc.clone(),
            ];
        } else {
            circle_plane2 = Some(SidedPlane::construct_sided_plane_from_two_points(
                &point,
                current_ulhc,
                current_llhc,
            )?);
            notable_points2 = vec![current_ulhc.clone(), current_llhc.clone()];
        }
        Ok(Self {
            base: BaseSegmentEndpoint::new(planet_model, previous, point),
            circle_plane1,
            circle_plane2,
            notable_points1,
            notable_points2,
            boundary_plane1: SidedPlane::new_inverse(prev_cutoff_plane),
            boundary_plane2: SidedPlane::new_inverse(next_cutoff_plane),
        })
    }

    fn c1(&self) -> &SidedPlane {
        self.circle_plane1.as_ref().unwrap_or_else(|| npe())
    }

    fn c2(&self) -> &SidedPlane {
        self.circle_plane2.as_ref().unwrap_or_else(|| npe())
    }
}

impl PathComponent for CutoffDualCircleSegmentEndpoint {
    fn is_within(&self, point: &Vector) -> bool {
        if !self.boundary_plane1.is_within(point) || !self.boundary_plane2.is_within(point) {
            return false;
        }
        self.c1().is_within(point) || self.c2().is_within(point)
    }
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        if !self.boundary_plane1.is_within_xyz(x, y, z)
            || !self.boundary_plane2.is_within_xyz(x, y, z)
        {
            return false;
        }
        self.c1().is_within_xyz(x, y, z) || self.c2().is_within_xyz(x, y, z)
    }
    fn is_within_section(&self, point: &Vector) -> bool {
        self.boundary_plane1.is_within(point) && self.boundary_plane2.is_within(point)
    }
    fn is_within_section_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.boundary_plane1.is_within_xyz(x, y, z) && self.boundary_plane2.is_within_xyz(x, y, z)
    }
    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        self.base.get_starting_distance(ds)
    }
    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        BaseSegmentEndpoint::full_path_distance(ds)
    }
    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        BaseSegmentEndpoint::distance(self, ds, x, y, z)
    }
    fn nearest_distance(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>> {
        BaseSegmentEndpoint::nearest_distance(self, ds, x, y, z)
    }
    fn path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(self.base.path_distance(self, ds, x, y, z))
    }
    fn path_delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(self.base.path_delta_distance(self, ds, x, y, z))
    }
    fn nearest_path_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_section_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        Ok(BaseSegmentEndpoint::nearest_path_distance(
            self, ds, x, y, z,
        ))
    }
    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_section_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        Ok(self.base.path_center_distance(self, ds, x, y, z))
    }
    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        self.base.outside_distance(ds, x, y, z)
    }
    fn intersects(
        &self,
        p: &Plane,
        _plane_bounds: Option<&XYZBounds>,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        let pm = &self.base.planet_model;
        let cutoff: [&dyn Membership; 2] = [&self.boundary_plane1, &self.boundary_plane2];
        self.c1().intersects(
            pm,
            p,
            notable_points,
            &self.notable_points1,
            bounds,
            &cutoff,
        ) || self.c2().intersects(
            pm,
            p,
            notable_points,
            &self.notable_points2,
            bounds,
            &cutoff,
        )
    }
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        let cutoff: [&dyn Membership; 2] = [&self.boundary_plane1, &self.boundary_plane2];
        geo_shape.intersects(self.c1(), &self.notable_points1, &cutoff)
            || geo_shape.intersects(self.c2(), &self.notable_points2, &cutoff)
    }
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        self.base.get_bounds(self, bounds);
        let pm = &self.base.planet_model;
        let (c1, c2, b1, b2) = (
            self.c1(),
            self.c2(),
            &self.boundary_plane1,
            &self.boundary_plane2,
        );
        bounds.add_plane(pm, c1, &[b1, b2]);
        bounds.add_plane(pm, c2, &[b1, b2]);
        bounds.add_plane(pm, b1, &[c1, b2]);
        bounds.add_plane(pm, b1, &[c2, b2]);
        bounds.add_plane(pm, b2, &[c1, b1]);
        bounds.add_plane(pm, b2, &[c2, b1]);
        bounds.add_intersection(pm, c1, b1, &[b2]);
        bounds.add_intersection(pm, c1, b2, &[b1]);
        bounds.add_intersection(pm, c2, b1, &[b2]);
        bounds.add_intersection(pm, c2, b2, &[b1]);
    }
}

impl SegmentEndpoint for CutoffDualCircleSegmentEndpoint {}

impl fmt::Display for CutoffDualCircleSegmentEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CutoffDualCircleSegmentEndpoint: {}", self.base)
    }
}

/// This is the pre-calculated data for a path segment.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.PathSegment`.
pub(crate) struct PathSegment {
    planet_model: Arc<PlanetModel>,
    /// Previous path component.
    previous: Option<Arc<dyn PathComponent>>,
    /// Starting point of the segment.
    start: GeoPoint,
    /// End point of the segment.
    end: GeoPoint,
    /// Place to keep any complete segment distances we've calculated so far.
    start_distance_cache: Mutex<HashMap<TypeId, f64>>,
    /// Normalized plane connecting the two points and going through world
    /// center.
    normalized_connecting_plane: Plane,
    /// Cutoff plane parallel to connecting plane representing one side of
    /// the path segment.
    upper_connecting_plane: SidedPlane,
    /// Cutoff plane parallel to connecting plane representing the other side
    /// of the path segment.
    lower_connecting_plane: SidedPlane,
    /// Plane going through the center and start point, marking the start
    /// edge of the segment.
    start_cutoff_plane: SidedPlane,
    /// Plane going through the center and end point, marking the end edge of
    /// the segment.
    end_cutoff_plane: SidedPlane,
    /// Upper right hand corner of segment.
    urhc: GeoPoint,
    /// Lower right hand corner of segment.
    lrhc: GeoPoint,
    /// Upper left hand corner of segment.
    ulhc: GeoPoint,
    /// Lower left hand corner of segment.
    llhc: GeoPoint,
    /// Notable points for the upper connecting plane.
    upper_connecting_plane_points: [GeoPoint; 2],
    /// Notable points for the lower connecting plane.
    lower_connecting_plane_points: [GeoPoint; 2],
}

impl PathSegment {
    fn new(
        planet_model: &Arc<PlanetModel>,
        previous: Option<Arc<dyn PathComponent>>,
        start: GeoPoint,
        end: GeoPoint,
        normalized_connecting_plane: Plane,
        plane_bounding_offset: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        let upper_connecting_plane =
            SidedPlane::new_vector_d(&start, &normalized_connecting_plane, -plane_bounding_offset)?;
        let lower_connecting_plane =
            SidedPlane::new_vector_d(&start, &normalized_connecting_plane, plane_bounding_offset)?;
        debug_assert!(upper_connecting_plane.is_within(&start));
        debug_assert!(upper_connecting_plane.is_within(&end));
        debug_assert!(lower_connecting_plane.is_within(&start));
        debug_assert!(lower_connecting_plane.is_within(&end));
        let start_cutoff_plane = SidedPlane::new_cross(&end, &normalized_connecting_plane, &start)?;
        debug_assert!(start_cutoff_plane.is_within(&end));
        debug_assert!(start_cutoff_plane.is_within(&start));
        let end_cutoff_plane = SidedPlane::new_cross(&start, &normalized_connecting_plane, &end)?;
        debug_assert!(end_cutoff_plane.is_within(&start));
        debug_assert!(end_cutoff_plane.is_within(&end));
        let upper_side: [&dyn Membership; 1] = [&upper_connecting_plane];
        let lower_side: [&dyn Membership; 1] = [&lower_connecting_plane];
        let start_side: [&dyn Membership; 1] = [&start_cutoff_plane];
        let end_side: [&dyn Membership; 1] = [&end_cutoff_plane];
        let one = |points: Vec<GeoPoint>| -> Result<GeoPoint> {
            if points.is_empty() {
                return Err(iae(
                    "Some segment boundary points are off the ellipsoid; path too wide",
                ));
            }
            if points.len() > 1 {
                return Err(iae("Ambiguous boundary points; path too short"));
            }
            Ok(points.into_iter().next().unwrap_or_else(|| npe()))
        };
        let ulhc = one(upper_connecting_plane.find_intersections_with(
            pm,
            &start_cutoff_plane,
            &lower_side,
            &end_side,
        ))?;
        let urhc = one(upper_connecting_plane.find_intersections_with(
            pm,
            &end_cutoff_plane,
            &lower_side,
            &start_side,
        ))?;
        let llhc = one(lower_connecting_plane.find_intersections_with(
            pm,
            &start_cutoff_plane,
            &upper_side,
            &end_side,
        ))?;
        let lrhc = one(lower_connecting_plane.find_intersections_with(
            pm,
            &end_cutoff_plane,
            &upper_side,
            &start_side,
        ))?;
        Ok(Self {
            planet_model: pm.clone(),
            previous,
            start,
            end,
            start_distance_cache: Mutex::new(HashMap::with_capacity(1)),
            normalized_connecting_plane,
            upper_connecting_plane,
            lower_connecting_plane,
            start_cutoff_plane,
            end_cutoff_plane,
            upper_connecting_plane_points: [ulhc.clone(), urhc.clone()],
            lower_connecting_plane_points: [llhc.clone(), lrhc.clone()],
            urhc,
            lrhc,
            ulhc,
            llhc,
        })
    }

    fn compute_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        match &self.previous {
            None => 0.0,
            Some(previous) => ds.aggregate_distances(&[
                previous.get_starting_distance(ds),
                previous.full_path_distance(ds),
            ]),
        }
    }

    fn perp(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64, f64) {
        let p = &self.normalized_connecting_plane;
        let perp_x = p.y * z - p.z * y;
        let perp_y = p.z * x - p.x * z;
        let perp_z = p.x * y - p.y * x;
        let magnitude = java_math::sqrt(perp_x * perp_x + perp_y * perp_y + perp_z * perp_z);
        (perp_x, perp_y, perp_z, magnitude)
    }

    /// The point of the connecting plane nearest the given point, shared by
    /// the distance methods.
    #[allow(clippy::too_many_arguments)]
    // Java's branches, kept as written.
    #[allow(clippy::if_same_then_else)]
    fn the_point(
        &self,
        x: f64,
        y: f64,
        z: f64,
        magnitude: f64,
        perp_x: f64,
        perp_y: f64,
        perp_z: f64,
    ) -> Result<GeoPoint> {
        let norm_factor = 1.0 / magnitude;
        let normalized_perp_plane = Plane::new(
            perp_x * norm_factor,
            perp_y * norm_factor,
            perp_z * norm_factor,
            0.0,
        );
        let intersection_points = self
            .normalized_connecting_plane
            .find_intersections(&self.planet_model, &normalized_perp_plane, NO_BOUNDS)
            .unwrap_or_else(|| npe());
        let world = || {
            rte(format!(
                "Can't find world intersection for point x={} y={} z={}",
                d2s(x),
                d2s(y),
                d2s(z)
            ))
        };
        if intersection_points.is_empty() {
            Err(world())
        } else if intersection_points.len() == 1 {
            Ok(intersection_points[0].clone())
        } else if self.start_cutoff_plane.is_within(&intersection_points[0])
            && self.end_cutoff_plane.is_within(&intersection_points[0])
        {
            Ok(intersection_points[0].clone())
        } else if self.start_cutoff_plane.is_within(&intersection_points[1])
            && self.end_cutoff_plane.is_within(&intersection_points[1])
        {
            Ok(intersection_points[1].clone())
        } else {
            Err(world())
        }
    }
}

impl PathComponent for PathSegment {
    fn full_path_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        ds.to_aggregation_form(ds.compute_distance_xyz(
            &self.start,
            self.end.x,
            self.end.y,
            self.end.z,
        ))
    }

    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.start_cutoff_plane.is_within_xyz(x, y, z)
            && self.end_cutoff_plane.is_within_xyz(x, y, z)
            && self.upper_connecting_plane.is_within_xyz(x, y, z)
            && self.lower_connecting_plane.is_within_xyz(x, y, z)
    }

    fn is_within(&self, v: &Vector) -> bool {
        self.is_within_xyz(v.x, v.y, v.z)
    }

    fn is_within_section(&self, point: &Vector) -> bool {
        self.start_cutoff_plane.is_within(point) && self.end_cutoff_plane.is_within(point)
    }

    fn is_within_section_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.start_cutoff_plane.is_within_xyz(x, y, z)
            && self.end_cutoff_plane.is_within_xyz(x, y, z)
    }

    fn get_starting_distance(&self, ds: &dyn DistanceStyle) -> f64 {
        let key = style_key(ds);
        let cached = self
            .start_distance_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .copied();
        match cached {
            Some(d) => d,
            None => {
                let dist = self.compute_starting_distance(ds);
                self.start_distance_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, dist);
                dist
            }
        }
    }

    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let starting_distance = self.get_starting_distance(ds);
        let path_distance = self.path_distance(ds, x, y, z)?;
        Ok(ds.from_aggregation_form(ds.aggregate_distances(&[starting_distance, path_distance])))
    }

    fn nearest_distance(
        &self,
        ds: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Option<DistancePair>> {
        if !self.is_within_section_xyz(x, y, z) {
            return Ok(None);
        }
        let path_center_distance = self.path_center_distance(ds, x, y, z)?;
        let starting = self.get_starting_distance(ds);
        let nearest = self.nearest_path_distance(ds, x, y, z)?;
        Ok(Some(DistancePair {
            path_center_distance,
            distance_along_path: ds.aggregate_distances(&[starting, nearest]),
        }))
    }

    fn path_center_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_section_xyz(x, y, z) {
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
        if !self.is_within_section_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let (perp_x, perp_y, perp_z, magnitude) = self.perp(x, y, z);
        if magnitude.abs() < Vector::MINIMUM_RESOLUTION {
            return Ok(ds.to_aggregation_form(0.0));
        }
        let the_point = self.the_point(x, y, z, magnitude, perp_x, perp_y, perp_z)?;
        Ok(ds.to_aggregation_form(ds.compute_distance_xyz(
            &self.start,
            the_point.x,
            the_point.y,
            the_point.z,
        )))
    }

    fn path_delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if !self.is_within_xyz(x, y, z) {
            return Ok(f64::INFINITY);
        }
        let (perp_x, perp_y, perp_z, magnitude) = self.perp(x, y, z);
        if magnitude.abs() < Vector::MINIMUM_RESOLUTION {
            let the_distance = ds.compute_distance_xyz(&self.start, x, y, z);
            return Ok(ds.aggregate_distances(&[the_distance, the_distance]));
        }
        let the_point = self.the_point(x, y, z, magnitude, perp_x, perp_y, perp_z)?;
        let the_distance = ds.to_aggregation_form(ds.compute_distance_xyz(&the_point, x, y, z));
        Ok(ds.aggregate_distances(&[the_distance, the_distance]))
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
            ds.to_aggregation_form(ds.compute_distance_xyz(
                &self.start,
                the_point.x,
                the_point.y,
                the_point.z,
            )),
        ]))
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> f64 {
        let pm = &self.planet_model;
        let upper_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.upper_connecting_plane,
            x,
            y,
            z,
            &[
                &self.lower_connecting_plane,
                &self.start_cutoff_plane,
                &self.end_cutoff_plane,
            ],
        );
        let lower_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.lower_connecting_plane,
            x,
            y,
            z,
            &[
                &self.upper_connecting_plane,
                &self.start_cutoff_plane,
                &self.end_cutoff_plane,
            ],
        );
        let start_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.start_cutoff_plane,
            x,
            y,
            z,
            &[
                &self.end_cutoff_plane,
                &self.lower_connecting_plane,
                &self.upper_connecting_plane,
            ],
        );
        let end_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.end_cutoff_plane,
            x,
            y,
            z,
            &[
                &self.start_cutoff_plane,
                &self.lower_connecting_plane,
                &self.upper_connecting_plane,
            ],
        );
        let ulhc_distance = ds.compute_distance_xyz(&self.ulhc, x, y, z);
        let urhc_distance = ds.compute_distance_xyz(&self.urhc, x, y, z);
        let llhc_distance = ds.compute_distance_xyz(&self.llhc, x, y, z);
        let lrhc_distance = ds.compute_distance_xyz(&self.lrhc, x, y, z);
        ds.to_aggregation_form(java_min(
            java_min(
                java_min(upper_distance, lower_distance),
                java_min(start_distance, end_distance),
            ),
            java_min(
                java_min(ulhc_distance, urhc_distance),
                java_min(llhc_distance, lrhc_distance),
            ),
        ))
    }

    fn intersects(
        &self,
        p: &Plane,
        _plane_bounds: Option<&XYZBounds>,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        let pm = &self.planet_model;
        self.upper_connecting_plane.intersects(
            pm,
            p,
            notable_points,
            &self.upper_connecting_plane_points,
            bounds,
            &[
                &self.lower_connecting_plane,
                &self.start_cutoff_plane,
                &self.end_cutoff_plane,
            ],
        ) || self.lower_connecting_plane.intersects(
            pm,
            p,
            notable_points,
            &self.lower_connecting_plane_points,
            bounds,
            &[
                &self.upper_connecting_plane,
                &self.start_cutoff_plane,
                &self.end_cutoff_plane,
            ],
        )
    }

    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(
            &self.upper_connecting_plane,
            &self.upper_connecting_plane_points,
            &[
                &self.lower_connecting_plane,
                &self.start_cutoff_plane,
                &self.end_cutoff_plane,
            ],
        ) || geo_shape.intersects(
            &self.lower_connecting_plane,
            &self.lower_connecting_plane_points,
            &[
                &self.upper_connecting_plane,
                &self.start_cutoff_plane,
                &self.end_cutoff_plane,
            ],
        )
    }

    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(&ComponentMembership(self), pm, bounds);
        let (up, lo, st, en) = (
            &self.upper_connecting_plane,
            &self.lower_connecting_plane,
            &self.start_cutoff_plane,
            &self.end_cutoff_plane,
        );
        bounds
            .add_point(&self.start)
            .add_point(&self.end)
            .add_point(&self.ulhc)
            .add_point(&self.urhc)
            .add_point(&self.lrhc)
            .add_point(&self.llhc)
            .add_plane(pm, up, &[lo, st, en])
            .add_plane(pm, lo, &[up, st, en])
            .add_plane(pm, st, &[en, up, lo])
            .add_plane(pm, en, &[st, up, lo])
            .add_intersection(pm, up, st, &[lo, en])
            .add_intersection(pm, st, lo, &[en, up])
            .add_intersection(pm, lo, en, &[up, st])
            .add_intersection(pm, en, up, &[st, lo]);
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PathSegment ({}, {}, {}, {})",
            self.ulhc, self.urhc, self.lrhc, self.llhc
        )
    }
}

/// This is a temporary object that is used to build a tree of path
/// components.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardPath.TreeBuilder`.
pub(crate) struct TreeBuilder {
    component_stack: Vec<Arc<dyn PathComponent>>,
    depth_stack: Vec<i32>,
}

impl TreeBuilder {
    fn new(max: usize) -> Self {
        Self {
            component_stack: Vec::with_capacity(max),
            depth_stack: Vec::with_capacity(max),
        }
    }

    fn add_component(&mut self, component: Arc<dyn PathComponent>) {
        self.component_stack.push(component);
        self.depth_stack.push(0);
        while self.depth_stack.len() >= 2 {
            let n = self.depth_stack.len();
            if self.depth_stack[n - 1] == self.depth_stack[n - 2] {
                self.merge_top();
            } else {
                break;
            }
        }
    }

    fn get_root(&mut self) -> Option<Arc<dyn PathComponent>> {
        if self.component_stack.is_empty() {
            return None;
        }
        while self.component_stack.len() > 1 {
            self.merge_top();
        }
        Some(self.component_stack[0].clone())
    }

    fn merge_top(&mut self) {
        self.depth_stack.pop();
        let second_component = self.component_stack.pop().unwrap_or_else(|| npe());
        let new_depth = self.depth_stack.pop().unwrap_or_else(|| npe()) + 1;
        let first_component = self.component_stack.pop().unwrap_or_else(|| npe());
        self.depth_stack.push(new_depth);
        self.component_stack
            .push(Arc::new(PathNode::new(first_component, second_component)));
    }
}
