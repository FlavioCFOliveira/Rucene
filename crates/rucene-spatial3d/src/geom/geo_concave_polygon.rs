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


//! The types declared in `GeoConcavePolygon.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::DistanceStyle;
use super::geo_area_shape::GeoAreaShape;
use super::geo_point::GeoPoint;
use super::geo_polygon::GeoPolygon;
use super::geo_shape::GeoShape;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::vector::Vector;
use super::{iae, ise, list_hash_code, list_to_string, npe, JavaBitSet};

/// GeoConcavePolygon objects are generic building blocks of more complex
/// structures. The only restrictions on these objects are: (1) they must be
/// concave; (2) they must have a maximum extent larger than PI. Violating
/// either one of these limits will cause the logic to fail.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoConcavePolygon`. The
/// arrays Java computes in `done()` (null until then) are `built`.
pub(crate) struct GeoConcavePolygon {
    planet_model: Arc<PlanetModel>,
    /// The list of polygon points.
    points: Vec<GeoPoint>,
    /// A bitset describing, for each edge, whether it is internal or not.
    is_internal_edges: JavaBitSet,
    /// The list of holes; `None` when there are none.
    holes: Option<Vec<Arc<dyn GeoPolygon>>>,
    /// Whether `done()` has been called.
    is_done: bool,
    built: Option<Built>,
}

/// The planes and points `done()` computes.
struct Built {
    edges: Vec<SidedPlane>,
    inverted_edges: Vec<SidedPlane>,
    start_bounds: Vec<SidedPlane>,
    end_bounds: Vec<SidedPlane>,
    notable_edge_points: Vec<[GeoPoint; 2]>,
    edge_points: Vec<GeoPoint>,
    /// `prevBrotherMap`, valued by edge index.
    prev_brother_map: HashMap<SidedPlane, usize>,
    /// `nextBrotherMap`, valued by edge index.
    next_brother_map: HashMap<SidedPlane, usize>,
}

fn normalize_holes(holes: Option<Vec<Arc<dyn GeoPolygon>>>) -> Option<Vec<Arc<dyn GeoPolygon>>> {
    match holes {
        Some(h) if h.is_empty() => None,
        other => other,
    }
}

impl GeoConcavePolygon {
    /// `new GeoConcavePolygon(PlanetModel, List<GeoPoint>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn new(planet_model: &Arc<PlanetModel>, point_list: Vec<GeoPoint>) -> Result<Self> {
        Self::new_with_holes(planet_model, point_list, None)
    }

    /// `new GeoConcavePolygon(PlanetModel, List<GeoPoint>, List<GeoPolygon>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn new_with_holes(
        planet_model: &Arc<PlanetModel>,
        point_list: Vec<GeoPoint>,
        holes: Option<Vec<Arc<dyn GeoPolygon>>>,
    ) -> Result<Self> {
        let mut s = Self {
            planet_model: planet_model.clone(),
            points: point_list,
            is_internal_edges: JavaBitSet::new(),
            holes: normalize_holes(holes),
            is_done: false,
            built: None,
        };
        s.done(false)?;
        Ok(s)
    }

    /// `new GeoConcavePolygon(PlanetModel, List<GeoPoint>, BitSet, boolean)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub(crate) fn new_with_flags(
        planet_model: &Arc<PlanetModel>,
        point_list: Vec<GeoPoint>,
        internal_edge_flags: JavaBitSet,
        return_edge_internal: bool,
    ) -> Result<Self> {
        Self::new_with_holes_and_flags(planet_model, point_list, None, internal_edge_flags, return_edge_internal)
    }

    /// `new GeoConcavePolygon(PlanetModel, List<GeoPoint>, List<GeoPolygon>,
    /// BitSet, boolean)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub(crate) fn new_with_holes_and_flags(
        planet_model: &Arc<PlanetModel>,
        point_list: Vec<GeoPoint>,
        holes: Option<Vec<Arc<dyn GeoPolygon>>>,
        internal_edge_flags: JavaBitSet,
        return_edge_internal: bool,
    ) -> Result<Self> {
        let mut s = Self {
            planet_model: planet_model.clone(),
            points: point_list,
            is_internal_edges: internal_edge_flags,
            holes: normalize_holes(holes),
            is_done: false,
            built: None,
        };
        s.done(return_edge_internal)?;
        Ok(s)
    }

    /// `new GeoConcavePolygon(PlanetModel, double startLatitude, double
    /// startLongitude)`: the start of an incremental construction.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an out-of-range start point.
    pub fn new_start(planet_model: &Arc<PlanetModel>, start_latitude: f64, start_longitude: f64) -> Result<Self> {
        Self::new_start_with_holes(planet_model, start_latitude, start_longitude, None)
    }

    /// `new GeoConcavePolygon(PlanetModel, double startLatitude, double
    /// startLongitude, List<GeoPolygon>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an out-of-range start point.
    pub fn new_start_with_holes(
        planet_model: &Arc<PlanetModel>,
        start_latitude: f64,
        start_longitude: f64,
        holes: Option<Vec<Arc<dyn GeoPolygon>>>,
    ) -> Result<Self> {
        let holes = normalize_holes(holes);
        let points = vec![GeoPoint::new_lat_lon(planet_model, start_latitude, start_longitude)?];
        Ok(Self {
            planet_model: planet_model.clone(),
            points,
            is_internal_edges: JavaBitSet::new(),
            holes,
            is_done: false,
            built: None,
        })
    }

    /// `addPoint(double latitude, double longitude, boolean isInternalEdge)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` after `done()`; `IllegalArgumentException` for
    /// an out-of-range point.
    pub fn add_point(&mut self, latitude: f64, longitude: f64, is_internal_edge: bool) -> Result<()> {
        if self.is_done {
            return Err(ise("Can't call addPoint() if done() already called"));
        }
        if is_internal_edge {
            self.is_internal_edges.set(self.points.len() - 1);
        }
        self.points.push(GeoPoint::new_lat_lon(&self.planet_model, latitude, longitude)?);
        Ok(())
    }

    /// `done(boolean isInternalReturnEdge)`: finishes the polygon.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when called twice; `IllegalArgumentException`
    /// for an illegal polygon.
    pub fn done(&mut self, is_internal_return_edge: bool) -> Result<()> {
        if self.is_done {
            return Err(ise("Can't call done() more than once"));
        }
        if self.points.len() < 3 {
            return Err(iae("Polygon needs at least three points."));
        }
        if is_internal_return_edge {
            self.is_internal_edges.set(self.points.len() - 1);
        }
        self.is_done = true;
        let n = self.points.len();
        let points = &self.points;
        let mut edges: Vec<SidedPlane> = Vec::with_capacity(n);
        let mut inverted_edges: Vec<SidedPlane> = Vec::new();
        let mut start_bounds = Vec::with_capacity(n);
        let mut end_bounds = Vec::with_capacity(n);
        let mut notable_edge_points = Vec::with_capacity(n);
        for i in 0..n {
            let start = &points[i];
            let end = &points[self.legal_index(i as i64 + 1)];
            let plane_to_find = Plane::new_cross(start, end)?;
            let mut end_point_index: i64 = -1;
            for j in 0..n {
                let index = self.legal_index(j as i64 + i as i64 + 2);
                if !plane_to_find.evaluate_is_zero(&points[index]) {
                    end_point_index = index as i64;
                    break;
                }
            }
            if end_point_index == -1 {
                return Err(iae("Polygon points are all coplanar"));
            }
            let check = &points[end_point_index as usize];
            let sp = SidedPlane::new_cross_on_side(check, false, start, end)?;
            start_bounds.push(SidedPlane::construct_sided_plane_from_one_point(end, &sp, start)?);
            end_bounds.push(SidedPlane::construct_sided_plane_from_one_point(start, &sp, end)?);
            notable_edge_points.push([start.clone(), end.clone()]);
            inverted_edges.push(SidedPlane::new_inverse(&sp));
            edges.push(sp);
        }
        let brothers = &inverted_edges;
        let mut prev_brother_map = HashMap::with_capacity(brothers.len());
        let mut next_brother_map = HashMap::with_capacity(brothers.len());
        for edge_index in 0..brothers.len() {
            let edge = &brothers[edge_index];
            let mut bound1_index = self.legal_index(edge_index as i64 + 1);
            while brothers[bound1_index].is_numerically_identical(edge) {
                if bound1_index == edge_index {
                    return Err(iae(format!("Constructed planes are all coplanar: {}", list_to_string(points))));
                }
                bound1_index = self.legal_index(bound1_index as i64 + 1);
            }
            let mut bound2_index = self.legal_index(edge_index as i64 - 1);
            while brothers[bound2_index].is_numerically_identical(edge) {
                if bound2_index == edge_index {
                    return Err(iae(format!("Constructed planes are all coplanar: {}", list_to_string(points))));
                }
                bound2_index = self.legal_index(bound2_index as i64 - 1);
            }
            let mut starting_index = bound2_index;
            loop {
                starting_index = self.legal_index(starting_index as i64 + 1);
                if starting_index == bound1_index {
                    break;
                }
                let interior_point = &points[starting_index];
                if !brothers[bound1_index].is_within(interior_point) || !brothers[bound2_index].is_within(interior_point) {
                    return Err(iae("Concave polygon has a side that is more than 180 degrees"));
                }
            }
            next_brother_map.insert(edge.clone(), bound1_index);
            prev_brother_map.insert(edge.clone(), bound2_index);
        }
        let mut edge_points = Vec::new();
        edge_points.push(points[0].clone());
        if let Some(holes) = &self.holes {
            for hole in holes {
                edge_points.extend(hole.get_edge_points());
            }
        }
        if self.is_within_holes(&points[0]) {
            return Err(iae("Polygon edge intersects a polygon hole; not allowed"));
        }
        self.built = Some(Built {
            edges,
            inverted_edges,
            start_bounds,
            end_bounds,
            notable_edge_points,
            edge_points,
            prev_brother_map,
            next_brother_map,
        });
        Ok(())
    }

    /// `isWithinHoles(GeoPoint)`.
    fn is_within_holes(&self, point: &GeoPoint) -> bool {
        if let Some(holes) = &self.holes {
            for hole in holes {
                if !hole.is_within(point) {
                    return true;
                }
            }
        }
        false
    }

    /// `legalIndex(int)`.
    fn legal_index(&self, mut index: i64) -> usize {
        let n = self.points.len() as i64;
        while index >= n {
            index -= n;
        }
        while index < 0 {
            index += n;
        }
        index as usize
    }

    /// `new GeoConcavePolygon(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` for an
    /// illegal polygon.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let points = so::read_point_array(planet_model, input)?;
        let holes = so::read_polygon_array(planet_model, input)?;
        let is_internal_edges = so::read_bit_set(input)?;
        let mut s = Self {
            planet_model: planet_model.clone(),
            points,
            is_internal_edges,
            holes: normalize_holes(Some(holes)),
            is_done: false,
            built: None,
        };
        if s.points.is_empty() {
            return Err(rucene::error::LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                "bitIndex < 0: -1",
            ));
        }
        let last = s.is_internal_edges.get(s.points.len() - 1);
        s.done(last)?;
        Ok(s)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_point_array(output, &self.points)?;
        so::write_polygon_array(output, self.holes.as_deref().unwrap_or(&[]))?;
        so::write_bit_set(output, &self.is_internal_edges)
    }

    fn built(&self) -> &Built {
        self.built.as_ref().unwrap_or_else(|| npe())
    }

    /// `localIsWithin(Vector)`.
    fn local_is_within(&self, v: &Vector) -> bool {
        self.local_is_within_xyz(v.x, v.y, v.z)
    }

    /// `localIsWithin(double, double, double)`.
    fn local_is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        let mut is_member = false;
        for edge in &self.built().edges {
            if edge.is_within_xyz(x, y, z) {
                is_member = true;
                break;
            }
        }
        is_member
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let b = self.built();
        let mut minimum_distance = f64::INFINITY;
        for edge_point in &self.points {
            let new_dist = ds.compute_distance_xyz(edge_point, x, y, z);
            if new_dist < minimum_distance {
                minimum_distance = new_dist;
            }
        }
        for edge_index in 0..b.edges.len() {
            let edge_plane = &b.edges[edge_index];
            let new_dist = ds.compute_plane_distance_xyz(
                &self.planet_model,
                edge_plane,
                x,
                y,
                z,
                &[&b.start_bounds[edge_index], &b.end_bounds[edge_index]],
            );
            if new_dist < minimum_distance {
                minimum_distance = new_dist;
            }
        }
        if let Some(holes) = &self.holes {
            for hole in holes {
                let hole_distance = hole.compute_outside_distance_xyz(ds, x, y, z)?;
                if hole_distance != 0.0 && hole_distance < minimum_distance {
                    minimum_distance = hole_distance;
                }
            }
        }
        Ok(minimum_distance)
    }

    fn equals_impl(&self, other: &Self) -> bool {
        if !BasePlanetObject::equals(&self.planet_model, &other.planet_model) {
            return false;
        }
        if !other.is_internal_edges.java_equals(&self.is_internal_edges) {
            return false;
        }
        if other.holes.is_some() || self.holes.is_some() {
            let (Some(oh), Some(sh)) = (&other.holes, &self.holes) else {
                return false;
            };
            if oh.len() != sh.len() || !oh.iter().zip(sh).all(|(a, b)| a.java_equals(b.as_any())) {
                return false;
            }
        }
        other.points == self.points
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(list_hash_code(&self.points, |p| p.java_hash_code()));
        if let Some(holes) = &self.holes {
            result = result
                .wrapping_mul(31)
                .wrapping_add(list_hash_code(holes, |h| h.java_hash_code()));
        }
        result
    }
}

impl_geo_base_area_shape_common!(GeoConcavePolygon, "org.apache.lucene.spatial3d.geom.GeoConcavePolygon", geo_polygon);
impl_geo_base_area_relationship!(GeoConcavePolygon);

impl GeoPolygon for GeoConcavePolygon {}

impl Membership for GeoConcavePolygon {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        if !self.local_is_within_xyz(x, y, z) {
            return false;
        }
        if let Some(holes) = &self.holes {
            for polygon in holes {
                if !polygon.is_within_xyz(x, y, z) {
                    return false;
                }
            }
        }
        true
    }
}

impl Bounded for GeoConcavePolygon {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        let b = self.built();
        if self.local_is_within(&pm.north_pole) {
            bounds.no_top_latitude_bound().no_longitude_bound().add_point(&pm.north_pole);
        }
        if self.local_is_within(&pm.south_pole) {
            bounds.no_bottom_latitude_bound().no_longitude_bound().add_point(&pm.south_pole);
        }
        if self.local_is_within(&pm.min_x_pole) {
            bounds.add_point(&pm.min_x_pole);
        }
        if self.local_is_within(&pm.max_x_pole) {
            bounds.add_point(&pm.max_x_pole);
        }
        if self.local_is_within(&pm.min_y_pole) {
            bounds.add_point(&pm.min_y_pole);
        }
        if self.local_is_within(&pm.max_y_pole) {
            bounds.add_point(&pm.max_y_pole);
        }
        bounds.is_wide();
        for point in &self.points {
            bounds.add_point(point);
        }
        for edge_index in 0..b.edges.len() {
            let edge = &b.edges[edge_index];
            bounds.add_plane(pm, edge, &[&b.start_bounds[edge_index], &b.end_bounds[edge_index]]);
        }
        for inverted_edge in &b.inverted_edges {
            let next_edge = &b.inverted_edges[*b.next_brother_map.get(inverted_edge).unwrap_or_else(|| npe())];
            bounds.add_intersection(
                pm,
                inverted_edge,
                next_edge,
                &[
                    &b.inverted_edges[*b.prev_brother_map.get(inverted_edge).unwrap_or_else(|| npe())],
                    &b.inverted_edges[*b.next_brother_map.get(next_edge).unwrap_or_else(|| npe())],
                ],
            );
        }
    }
}

impl GeoShape for GeoConcavePolygon {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.built().edge_points.clone()
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        let b = self.built();
        for edge_index in 0..b.edges.len() {
            let edge = &b.inverted_edges[edge_index];
            let points = &b.notable_edge_points[edge_index];
            if !self.is_internal_edges.get(edge_index)
                && edge.intersects(
                    &self.planet_model,
                    p,
                    notable_points,
                    points,
                    bounds,
                    &[&b.start_bounds[edge_index], &b.end_bounds[edge_index]],
                )
            {
                return true;
            }
        }
        if let Some(holes) = &self.holes {
            for hole in holes {
                if hole.intersects(p, notable_points, bounds) {
                    return true;
                }
            }
        }
        false
    }
}

impl GeoAreaShape for GeoConcavePolygon {
    fn intersects_shape(&self, shape: &dyn GeoShape) -> bool {
        let b = self.built();
        for edge_index in 0..b.edges.len() {
            let edge = &b.edges[edge_index];
            let points = &b.notable_edge_points[edge_index];
            if !self.is_internal_edges.get(edge_index)
                && shape.intersects(edge, points, &[&b.start_bounds[edge_index], &b.end_bounds[edge_index]])
            {
                return true;
            }
        }
        if let Some(holes) = &self.holes {
            for hole in holes {
                if hole.intersects_shape(shape) {
                    return true;
                }
            }
        }
        false
    }
}

impl fmt::Display for GeoConcavePolygon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoConcavePolygon: {{planetmodel={}, points={}, internalEdges={}{}}}",
            self.planet_model,
            list_to_string(&self.points),
            self.is_internal_edges,
            match &self.holes {
                None => String::new(),
                Some(h) => format!(", holes={}", list_to_string(h)),
            }
        )
    }
}
