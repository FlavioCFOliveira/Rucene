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

//! The types declared in `GeoComplexPolygon.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::collections::HashSet;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use super::base_planet_object::BasePlanetObject;
use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::DistanceStyle;
use super::geo_area_shape::GeoAreaShape;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_point::GeoPoint;
use super::geo_polygon::GeoPolygon;
use super::geo_shape::GeoShape;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::vector::Vector;
use super::xyz_bounds::XYZBounds;
use super::{d2s, iae, ise, java_max, list_hash_code, npe, unchecked_panic};

/// GeoComplexPolygon objects are structures designed to handle very large
/// numbers of edges. They perform very well in this case compared to the
/// alternatives, which all have O(N) evaluation and O(N^2) setup times.
/// Complex polygons have O(N) setup times and best case O(log(N))
/// evaluation times.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon`. Java's
/// linked edges and tree nodes are kept in arenas and linked by index; an
/// iterator's identity set of edges is a set of edge indexes.
///
/// **Divergence.** Java's `isWithin` can throw the
/// `IllegalArgumentException`s "No dual-plane travel strategies were found"
/// and "Exhausted all traversal strategies", and the unchecked exceptions of
/// the crossing iterators (`GeoComplexPolygon.java` `isInSet`). The port's
/// `Membership::is_within_xyz` returns `bool`, as every other membership
/// does; [`GeoComplexPolygon::is_in_set_result`] returns those exceptions
/// as `Err`, and `is_within_xyz` panics with the exception's class and
/// message where Java would have thrown. `intersects` and `intersects_shape`
/// do the same for the exceptions of the tree traversal.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub struct GeoComplexPolygon {
    planet_model: Arc<PlanetModel>,
    edges: Vec<Edge>,
    x_tree: Tree,
    y_tree: Tree,
    z_tree: Tree,
    points_list: Vec<Vec<GeoPoint>>,
    test_point1_in_set: bool,
    test_point1: GeoPoint,
    test_point1_fixed_y_plane: Plane,
    test_point1_fixed_y_above_plane: Option<Plane>,
    test_point1_fixed_y_below_plane: Option<Plane>,
    test_point1_fixed_x_plane: Plane,
    test_point1_fixed_x_above_plane: Option<Plane>,
    test_point1_fixed_x_below_plane: Option<Plane>,
    test_point1_fixed_z_plane: Plane,
    test_point1_fixed_z_above_plane: Option<Plane>,
    test_point1_fixed_z_below_plane: Option<Plane>,
    edge_points: Vec<GeoPoint>,
    shape_start_edges: Vec<usize>,
}

/// This is the amount we go, roughly, in both directions, to find adjoining
/// points to test.
const NEAR_EDGE_CUTOFF: f64 = -Vector::MINIMUM_RESOLUTION * 10000.0;
/// Distance step for adjoining points.
const DELTA_DISTANCE: f64 = Vector::MINIMUM_RESOLUTION;
/// Maximum number of adjoining-point iterations.
const MAX_ITERATIONS: i32 = 100;
/// The amount off of the envelope plane that counts as "enough".
const OFF_PLANE_AMOUNT: f64 = Vector::MINIMUM_RESOLUTION * 0.1;

const HALF_PROPORTIONS: [f64; 1] = [0.5];

/// The plane above or below a fixed plane, `None` when too close to the
/// world's edge.
fn offset_plane(base: &Plane, above: bool, max: f64, min: f64) -> Option<Plane> {
    let p = Plane::new_offset(base, above);
    if -p.d - max > NEAR_EDGE_CUTOFF || min + p.d > NEAR_EDGE_CUTOFF {
        None
    } else {
        Some(p)
    }
}

/// `catch (IllegalArgumentException e)`: `Ok(None)` for an IAE, the other
/// errors propagate.
fn catch_iae<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.is_illegal_argument_exception() => Ok(None),
        Err(e) => Err(e),
    }
}

impl GeoComplexPolygon {
    /// `new GeoComplexPolygon(PlanetModel, List<List<GeoPoint>>, GeoPoint,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the test point is on an edge or an
    /// edge cannot be built.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        points_list: Vec<Vec<GeoPoint>>,
        test_point: GeoPoint,
        test_point_in_set: bool,
    ) -> Result<Self> {
        let pm = planet_model;
        debug_assert!(
            pm.point_on_surface_xyz(test_point.x, test_point.y, test_point.z),
            "Test point is not on the ellipsoid surface"
        );
        let mut edge_points = Vec::with_capacity(points_list.len());
        let mut shape_start_edges = Vec::with_capacity(points_list.len());
        let mut all_edges: Vec<Edge> = Vec::new();
        for shape_points in &points_list {
            all_edges.reserve(shape_points.len());
            let mut last_geo_point = shape_points.last().unwrap_or_else(|| {
                unchecked_panic(LuceneError::unchecked(
                    "java.lang.IndexOutOfBoundsException",
                    "Index -1 out of bounds for length 0",
                ))
            });
            edge_points.push(last_geo_point.clone());
            let mut last_edge: Option<usize> = None;
            let mut first_edge: Option<usize> = None;
            for this_geo_point in shape_points {
                debug_assert!(
                    pm.point_on_surface(this_geo_point),
                    "Polygon edge point must be on surface; {} is not",
                    this_geo_point
                );
                let edge = Edge::new(pm, last_geo_point.clone(), this_geo_point.clone())?;
                if edge.is_within(test_point.x, test_point.y, test_point.z) {
                    return Err(iae("Test point is on polygon edge: not allowed"));
                }
                let index = all_edges.len();
                all_edges.push(edge);
                if first_edge.is_none() {
                    first_edge = Some(index);
                }
                if let Some(le) = last_edge {
                    all_edges[le].next = index;
                }
                last_edge = Some(index);
                last_geo_point = this_geo_point;
            }
            let (Some(le), Some(fe)) = (last_edge, first_edge) else {
                npe()
            };
            all_edges[le].next = fe;
            shape_start_edges.push(fe);
        }
        let x_tree = XTree::create(&all_edges);
        let y_tree = YTree::create(&all_edges);
        let z_tree = ZTree::create(&all_edges);
        let test_point1 = test_point;
        let fixed_y = Plane::new(0.0, 1.0, 0.0, -test_point1.y);
        let fixed_x = Plane::new(1.0, 0.0, 0.0, -test_point1.x);
        let fixed_z = Plane::new(0.0, 0.0, 1.0, -test_point1.z);
        let (max_y, min_y) = (pm.get_maximum_y_value(), pm.get_minimum_y_value());
        let (max_x, min_x) = (pm.get_maximum_x_value(), pm.get_minimum_x_value());
        let (max_z, min_z) = (pm.get_maximum_z_value(), pm.get_minimum_z_value());
        Ok(Self {
            planet_model: pm.clone(),
            edges: all_edges,
            x_tree,
            y_tree,
            z_tree,
            points_list,
            test_point1_in_set: test_point_in_set,
            test_point1_fixed_y_above_plane: offset_plane(&fixed_y, true, max_y, min_y),
            test_point1_fixed_y_below_plane: offset_plane(&fixed_y, false, max_y, min_y),
            test_point1_fixed_x_above_plane: offset_plane(&fixed_x, true, max_x, min_x),
            test_point1_fixed_x_below_plane: offset_plane(&fixed_x, false, max_x, min_x),
            test_point1_fixed_z_above_plane: offset_plane(&fixed_z, true, max_z, min_z),
            test_point1_fixed_z_below_plane: offset_plane(&fixed_z, false, max_z, min_z),
            test_point1_fixed_y_plane: fixed_y,
            test_point1_fixed_x_plane: fixed_x,
            test_point1_fixed_z_plane: fixed_z,
            test_point1,
            edge_points,
            shape_start_edges,
        })
    }

    /// `new GeoComplexPolygon(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let points_list = Self::read_points_list(planet_model, input)?;
        let test_point = GeoPoint::from_stream_with_model(planet_model, input)?;
        let in_set = so::read_boolean(input)?;
        Self::new(planet_model, points_list, test_point, in_set)
    }

    fn read_points_list(
        planet_model: &Arc<PlanetModel>,
        input: &mut dyn Read,
    ) -> Result<Vec<Vec<GeoPoint>>> {
        let count = so::read_int(input)?;
        if count < 0 {
            return Err(iae(format!("Illegal Capacity: {}", count)));
        }
        let mut array = Vec::with_capacity((count as usize).min(1 << 16));
        for _ in 0..count {
            array.push(so::read_point_array(planet_model, input)?);
        }
        Ok(array)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_int(output, self.points_list.len() as i32)?;
        for points in &self.points_list {
            so::write_point_array(output, points)?;
        }
        so::SerializableObject::write(&self.test_point1, output)?;
        so::write_boolean(output, self.test_point1_in_set)
    }

    /// `isInSet(double, double, double, ...)` over the polygon's test point,
    /// with Java's unchecked exceptions as `Err`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when no traversal strategy works;
    /// `IllegalStateException` when a traversal intersection is missing.
    pub(crate) fn is_in_set_result(&self, x: f64, y: f64, z: f64) -> Result<bool> {
        self.is_in_set(
            x,
            y,
            z,
            &self.test_point1,
            self.test_point1_in_set,
            &self.test_point1_fixed_x_plane,
            self.test_point1_fixed_x_above_plane.as_ref(),
            self.test_point1_fixed_x_below_plane.as_ref(),
            &self.test_point1_fixed_y_plane,
            self.test_point1_fixed_y_above_plane.as_ref(),
            self.test_point1_fixed_y_below_plane.as_ref(),
            &self.test_point1_fixed_z_plane,
            self.test_point1_fixed_z_above_plane.as_ref(),
            self.test_point1_fixed_z_below_plane.as_ref(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn is_in_set(
        &self,
        x: f64,
        y: f64,
        z: f64,
        test_point: &GeoPoint,
        test_point_in_set: bool,
        test_point_fixed_x_plane: &Plane,
        test_point_fixed_x_above_plane: Option<&Plane>,
        test_point_fixed_x_below_plane: Option<&Plane>,
        test_point_fixed_y_plane: &Plane,
        test_point_fixed_y_above_plane: Option<&Plane>,
        test_point_fixed_y_below_plane: Option<&Plane>,
        test_point_fixed_z_plane: &Plane,
        test_point_fixed_z_above_plane: Option<&Plane>,
        test_point_fixed_z_below_plane: Option<&Plane>,
    ) -> Result<bool> {
        let pm = &*self.planet_model;
        if test_point.is_numerically_identical_xyz(x, y, z) {
            return Ok(test_point_in_set);
        }
        let parity = |it: &dyn CountingEdgeIterator, in_set: bool| {
            it.is_on_edge()
                || if (it.get_crossing_count() & 1) == 0 {
                    in_set
                } else {
                    !in_set
                }
        };
        if let (Some(above), Some(below), true) = (
            test_point_fixed_y_above_plane,
            test_point_fixed_y_below_plane,
            test_point_fixed_y_plane.evaluate_is_zero_xyz(x, y, z),
        ) {
            let mut it = self.create_linear_crossing_edge_iterator(
                test_point,
                test_point_fixed_y_plane,
                above,
                below,
                x,
                y,
                z,
            )?;
            self.y_tree
                .traverse_value(&self.edges, it.as_mut(), test_point.y)?;
            return Ok(parity(it.as_ref(), test_point_in_set));
        } else if let (Some(above), Some(below), true) = (
            test_point_fixed_x_above_plane,
            test_point_fixed_x_below_plane,
            test_point_fixed_x_plane.evaluate_is_zero_xyz(x, y, z),
        ) {
            let mut it = self.create_linear_crossing_edge_iterator(
                test_point,
                test_point_fixed_x_plane,
                above,
                below,
                x,
                y,
                z,
            )?;
            self.x_tree
                .traverse_value(&self.edges, it.as_mut(), test_point.x)?;
            return Ok(parity(it.as_ref(), test_point_in_set));
        } else if let (Some(above), Some(below), true) = (
            test_point_fixed_z_above_plane,
            test_point_fixed_z_below_plane,
            test_point_fixed_z_plane.evaluate_is_zero_xyz(x, y, z),
        ) {
            let mut it = self.create_linear_crossing_edge_iterator(
                test_point,
                test_point_fixed_z_plane,
                above,
                below,
                x,
                y,
                z,
            )?;
            self.z_tree
                .traverse_value(&self.edges, it.as_mut(), test_point.z)?;
            return Ok(parity(it.as_ref(), test_point_in_set));
        }
        let travel_plane_fixed_x = Plane::new(1.0, 0.0, 0.0, -x);
        let travel_plane_fixed_y = Plane::new(0.0, 1.0, 0.0, -y);
        let travel_plane_fixed_z = Plane::new(0.0, 0.0, 1.0, -z);
        let fixed_y_above_plane = offset_plane(
            &travel_plane_fixed_y,
            true,
            pm.get_maximum_y_value(),
            pm.get_minimum_y_value(),
        );
        let fixed_y_below_plane = offset_plane(
            &travel_plane_fixed_y,
            false,
            pm.get_maximum_y_value(),
            pm.get_minimum_y_value(),
        );
        let fixed_x_above_plane = offset_plane(
            &travel_plane_fixed_x,
            true,
            pm.get_maximum_x_value(),
            pm.get_minimum_x_value(),
        );
        let fixed_x_below_plane = offset_plane(
            &travel_plane_fixed_x,
            false,
            pm.get_maximum_x_value(),
            pm.get_minimum_x_value(),
        );
        let fixed_z_above_plane = offset_plane(
            &travel_plane_fixed_z,
            true,
            pm.get_maximum_z_value(),
            pm.get_minimum_z_value(),
        );
        let fixed_z_below_plane = offset_plane(
            &travel_plane_fixed_z,
            false,
            pm.get_maximum_z_value(),
            pm.get_minimum_z_value(),
        );
        let mut traversal_strategies: Vec<TraversalStrategy<'_>> = Vec::with_capacity(12);
        let ixy = pm.inverse_xy_scaling_squared;
        let iz = pm.inverse_z_scaling_squared;
        // Y test plane, X travel plane.
        if let (Some(tpa), Some(tpb), Some(fa), Some(fb)) = (
            test_point_fixed_y_above_plane,
            test_point_fixed_y_below_plane,
            fixed_x_above_plane.as_ref(),
            fixed_x_below_plane.as_ref(),
        ) {
            let check_above = 4.0 * (fa.d * fa.d * ixy + tpa.d * tpa.d * ixy - 1.0);
            let check_below = 4.0 * (fb.d * fb.d * ixy + tpb.d * tpb.d * ixy - 1.0);
            if check_above < Vector::MINIMUM_RESOLUTION_SQUARED
                && check_below < Vector::MINIMUM_RESOLUTION_SQUARED
            {
                let x_intersections_y = travel_plane_fixed_x
                    .find_intersections(pm, test_point_fixed_y_plane, &[])
                    .unwrap_or_else(|| npe());
                for p in x_intersections_y {
                    let tp_delta1 = test_point.x - p.x;
                    let tp_delta2 = test_point.z - p.z;
                    let cp_delta1 = y - p.y;
                    let cp_delta2 = z - p.z;
                    let new_distance = tp_delta1 * tp_delta1
                        + tp_delta2 * tp_delta2
                        + cp_delta1 * cp_delta1
                        + cp_delta2 * cp_delta2;
                    traversal_strategies.push(TraversalStrategy {
                        traversal_distance: new_distance,
                        first_leg_value: test_point.y,
                        second_leg_value: x,
                        first_leg_plane: test_point_fixed_y_plane,
                        first_leg_above_plane: tpa,
                        first_leg_below_plane: tpb,
                        second_leg_plane: &travel_plane_fixed_x,
                        second_leg_above_plane: fa,
                        second_leg_below_plane: fb,
                        first_leg_tree: &self.y_tree,
                        second_leg_tree: &self.x_tree,
                        intersection_point: p,
                    });
                }
            }
        }
        // Z test plane, X travel plane.
        if let (Some(tpa), Some(tpb), Some(fa), Some(fb)) = (
            test_point_fixed_z_above_plane,
            test_point_fixed_z_below_plane,
            fixed_x_above_plane.as_ref(),
            fixed_x_below_plane.as_ref(),
        ) {
            let check_above = 4.0 * (fa.d * fa.d * ixy + tpa.d * tpa.d * iz - 1.0);
            let check_below = 4.0 * (fb.d * fb.d * ixy + tpb.d * tpb.d * iz - 1.0);
            if check_above < Vector::MINIMUM_RESOLUTION_SQUARED
                && check_below < Vector::MINIMUM_RESOLUTION_SQUARED
            {
                let x_intersections_z = travel_plane_fixed_x
                    .find_intersections(pm, test_point_fixed_z_plane, &[])
                    .unwrap_or_else(|| npe());
                for p in x_intersections_z {
                    let tp_delta1 = test_point.x - p.x;
                    let tp_delta2 = test_point.y - p.y;
                    let cp_delta1 = y - p.y;
                    let cp_delta2 = z - p.z;
                    let new_distance = tp_delta1 * tp_delta1
                        + tp_delta2 * tp_delta2
                        + cp_delta1 * cp_delta1
                        + cp_delta2 * cp_delta2;
                    traversal_strategies.push(TraversalStrategy {
                        traversal_distance: new_distance,
                        first_leg_value: test_point.z,
                        second_leg_value: x,
                        first_leg_plane: test_point_fixed_z_plane,
                        first_leg_above_plane: tpa,
                        first_leg_below_plane: tpb,
                        second_leg_plane: &travel_plane_fixed_x,
                        second_leg_above_plane: fa,
                        second_leg_below_plane: fb,
                        first_leg_tree: &self.z_tree,
                        second_leg_tree: &self.x_tree,
                        intersection_point: p,
                    });
                }
            }
        }
        // X test plane, Y travel plane.
        if let (Some(tpa), Some(tpb), Some(fa), Some(fb)) = (
            test_point_fixed_x_above_plane,
            test_point_fixed_x_below_plane,
            fixed_y_above_plane.as_ref(),
            fixed_y_below_plane.as_ref(),
        ) {
            let check_above = 4.0 * (tpa.d * tpa.d * ixy + fa.d * fa.d * ixy - 1.0);
            let check_below = 4.0 * (tpb.d * tpb.d * ixy + fb.d * fb.d * ixy - 1.0);
            if check_above < Vector::MINIMUM_RESOLUTION_SQUARED
                && check_below < Vector::MINIMUM_RESOLUTION_SQUARED
            {
                let y_intersections_x = travel_plane_fixed_y
                    .find_intersections(pm, test_point_fixed_x_plane, &[])
                    .unwrap_or_else(|| npe());
                for p in y_intersections_x {
                    let tp_delta1 = test_point.y - p.y;
                    let tp_delta2 = test_point.z - p.z;
                    let cp_delta1 = x - p.x;
                    let cp_delta2 = z - p.z;
                    let new_distance = tp_delta1 * tp_delta1
                        + tp_delta2 * tp_delta2
                        + cp_delta1 * cp_delta1
                        + cp_delta2 * cp_delta2;
                    traversal_strategies.push(TraversalStrategy {
                        traversal_distance: new_distance,
                        first_leg_value: test_point.x,
                        second_leg_value: y,
                        first_leg_plane: test_point_fixed_x_plane,
                        first_leg_above_plane: tpa,
                        first_leg_below_plane: tpb,
                        second_leg_plane: &travel_plane_fixed_y,
                        second_leg_above_plane: fa,
                        second_leg_below_plane: fb,
                        first_leg_tree: &self.x_tree,
                        second_leg_tree: &self.y_tree,
                        intersection_point: p,
                    });
                }
            }
        }
        // Z test plane, Y travel plane.
        if let (Some(tpa), Some(tpb), Some(fa), Some(fb)) = (
            test_point_fixed_z_above_plane,
            test_point_fixed_z_below_plane,
            fixed_y_above_plane.as_ref(),
            fixed_y_below_plane.as_ref(),
        ) {
            let check_above = 4.0 * (tpa.d * tpa.d * iz + fa.d * fa.d * ixy - 1.0);
            let check_below = 4.0 * (tpb.d * tpb.d * iz + fb.d * fb.d * ixy - 1.0);
            if check_above < Vector::MINIMUM_RESOLUTION_SQUARED
                && check_below < Vector::MINIMUM_RESOLUTION_SQUARED
            {
                let y_intersections_z = travel_plane_fixed_y
                    .find_intersections(pm, test_point_fixed_z_plane, &[])
                    .unwrap_or_else(|| npe());
                for p in y_intersections_z {
                    let tp_delta1 = test_point.x - p.x;
                    let tp_delta2 = test_point.y - p.y;
                    let cp_delta1 = x - p.x;
                    let cp_delta2 = z - p.z;
                    let new_distance = tp_delta1 * tp_delta1
                        + tp_delta2 * tp_delta2
                        + cp_delta1 * cp_delta1
                        + cp_delta2 * cp_delta2;
                    traversal_strategies.push(TraversalStrategy {
                        traversal_distance: new_distance,
                        first_leg_value: test_point.z,
                        second_leg_value: y,
                        first_leg_plane: test_point_fixed_z_plane,
                        first_leg_above_plane: tpa,
                        first_leg_below_plane: tpb,
                        second_leg_plane: &travel_plane_fixed_y,
                        second_leg_above_plane: fa,
                        second_leg_below_plane: fb,
                        first_leg_tree: &self.z_tree,
                        second_leg_tree: &self.y_tree,
                        intersection_point: p,
                    });
                }
            }
        }
        // X test plane, Z travel plane.
        if let (Some(tpa), Some(tpb), Some(fa), Some(fb)) = (
            test_point_fixed_x_above_plane,
            test_point_fixed_x_below_plane,
            fixed_z_above_plane.as_ref(),
            fixed_z_below_plane.as_ref(),
        ) {
            let check_above = 4.0 * (tpa.d * tpa.d * ixy + fa.d * fa.d * iz - 1.0);
            let check_below = 4.0 * (tpb.d * tpb.d * ixy + fb.d * fb.d * iz - 1.0);
            if check_above < Vector::MINIMUM_RESOLUTION_SQUARED
                && check_below < Vector::MINIMUM_RESOLUTION_SQUARED
            {
                let z_intersections_x = travel_plane_fixed_z
                    .find_intersections(pm, test_point_fixed_x_plane, &[])
                    .unwrap_or_else(|| npe());
                for p in z_intersections_x {
                    let tp_delta1 = test_point.y - p.y;
                    let tp_delta2 = test_point.z - p.z;
                    let cp_delta1 = y - p.y;
                    let cp_delta2 = x - p.x;
                    let new_distance = tp_delta1 * tp_delta1
                        + tp_delta2 * tp_delta2
                        + cp_delta1 * cp_delta1
                        + cp_delta2 * cp_delta2;
                    traversal_strategies.push(TraversalStrategy {
                        traversal_distance: new_distance,
                        first_leg_value: test_point.x,
                        second_leg_value: z,
                        first_leg_plane: test_point_fixed_x_plane,
                        first_leg_above_plane: tpa,
                        first_leg_below_plane: tpb,
                        second_leg_plane: &travel_plane_fixed_z,
                        second_leg_above_plane: fa,
                        second_leg_below_plane: fb,
                        first_leg_tree: &self.x_tree,
                        second_leg_tree: &self.z_tree,
                        intersection_point: p,
                    });
                }
            }
        }
        // Y test plane, Z travel plane.
        if let (Some(tpa), Some(tpb), Some(fa), Some(fb)) = (
            test_point_fixed_y_above_plane,
            test_point_fixed_y_below_plane,
            fixed_z_above_plane.as_ref(),
            fixed_z_below_plane.as_ref(),
        ) {
            let check_above = 4.0 * (tpa.d * tpa.d * ixy + fa.d * fa.d * iz - 1.0);
            let check_below = 4.0 * (tpb.d * tpb.d * ixy + fb.d * fb.d * iz - 1.0);
            if check_above < Vector::MINIMUM_RESOLUTION_SQUARED
                && check_below < Vector::MINIMUM_RESOLUTION_SQUARED
            {
                let z_intersections_y = travel_plane_fixed_z
                    .find_intersections(pm, test_point_fixed_y_plane, &[])
                    .unwrap_or_else(|| npe());
                for p in z_intersections_y {
                    let tp_delta1 = test_point.x - p.x;
                    let tp_delta2 = test_point.z - p.z;
                    let cp_delta1 = y - p.y;
                    let cp_delta2 = x - p.x;
                    let new_distance = tp_delta1 * tp_delta1
                        + tp_delta2 * tp_delta2
                        + cp_delta1 * cp_delta1
                        + cp_delta2 * cp_delta2;
                    traversal_strategies.push(TraversalStrategy {
                        traversal_distance: new_distance,
                        first_leg_value: test_point.y,
                        second_leg_value: z,
                        first_leg_plane: test_point_fixed_y_plane,
                        first_leg_above_plane: tpa,
                        first_leg_below_plane: tpb,
                        second_leg_plane: &travel_plane_fixed_z,
                        second_leg_above_plane: fa,
                        second_leg_below_plane: fb,
                        first_leg_tree: &self.y_tree,
                        second_leg_tree: &self.z_tree,
                        intersection_point: p,
                    });
                }
            }
        }
        traversal_strategies.sort_by(|a, b| a.compare_to(b));
        if traversal_strategies.is_empty() {
            return Err(iae("No dual-plane travel strategies were found"));
        }
        for ts in &traversal_strategies {
            if let Some(result) = catch_iae(ts.apply(self, test_point, test_point_in_set, x, y, z))?
            {
                return Ok(result);
            }
        }
        Err(iae("Exhausted all traversal strategies"))
    }

    /// `createLinearCrossingEdgeIterator(...)`: a sector iterator, or a full
    /// one when the sector one is unreliable.
    #[allow(clippy::too_many_arguments)]
    fn create_linear_crossing_edge_iterator<'a>(
        &'a self,
        test_point: &GeoPoint,
        plane: &'a Plane,
        above_plane: &'a Plane,
        below_plane: &'a Plane,
        the_point_x: f64,
        the_point_y: f64,
        the_point_z: f64,
    ) -> Result<Box<dyn CountingEdgeIterator + 'a>> {
        if let Some(it) = catch_iae(SectorLinearCrossingEdgeIterator::new(
            self,
            test_point,
            plane,
            above_plane,
            below_plane,
            the_point_x,
            the_point_y,
            the_point_z,
        ))? {
            return Ok(Box::new(it));
        }
        Ok(Box::new(FullLinearCrossingEdgeIterator::new(
            self,
            test_point,
            plane,
            above_plane,
            below_plane,
            the_point_x,
            the_point_y,
            the_point_z,
        )?))
    }

    /// `findAdjoiningPoints(Plane, GeoPoint, Plane)`: `None` when no adjoining
    /// points are found.
    fn find_adjoining_points(
        &self,
        plane: &Plane,
        point_on_plane: &GeoPoint,
        envelope_plane: &Plane,
    ) -> Result<Option<[GeoPoint; 2]>> {
        let perpendicular = Vector::new_cross(plane, point_on_plane)?;
        let mut distance_factor = 0.0;
        for _ in 0..MAX_ITERATIONS {
            distance_factor += DELTA_DISTANCE;
            let point_a = self.planet_model.create_surface_point_xyz(
                point_on_plane.x + perpendicular.x * distance_factor,
                point_on_plane.y + perpendicular.y * distance_factor,
                point_on_plane.z + perpendicular.z * distance_factor,
            );
            let point_b = self.planet_model.create_surface_point_xyz(
                point_on_plane.x - perpendicular.x * distance_factor,
                point_on_plane.y - perpendicular.y * distance_factor,
                point_on_plane.z - perpendicular.z * distance_factor,
            );
            if envelope_plane.evaluate(&point_a).abs() > OFF_PLANE_AMOUNT
                && envelope_plane.evaluate(&point_b).abs() > OFF_PLANE_AMOUNT
            {
                return Ok(Some([point_a, point_b]));
            }
        }
        Ok(None)
    }

    fn compute_squared_distance(check_point: &GeoPoint, intersection_point: &GeoPoint) -> f64 {
        let distance_x = check_point.x - intersection_point.x;
        let distance_y = check_point.y - intersection_point.y;
        let distance_z = check_point.z - intersection_point.z;
        distance_x * distance_x + distance_y * distance_y + distance_z * distance_z
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let mut minimum_distance = f64::INFINITY;
        for &shape_start_edge in &self.shape_start_edges {
            let mut shape_edge = shape_start_edge;
            loop {
                let e = &self.edges[shape_edge];
                let new_dist = ds.compute_distance_xyz(&e.start_point, x, y, z);
                if new_dist < minimum_distance {
                    minimum_distance = new_dist;
                }
                let new_plane_dist = ds.compute_plane_distance_xyz(
                    &self.planet_model,
                    &e.plane,
                    x,
                    y,
                    z,
                    &[&e.start_plane, &e.end_plane],
                );
                if new_plane_dist < minimum_distance {
                    minimum_distance = new_plane_dist;
                }
                shape_edge = e.next;
                if shape_edge == shape_start_edge {
                    break;
                }
            }
        }
        Ok(minimum_distance)
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && self.test_point1_in_set == other.test_point1_in_set
            && self.test_point1 == other.test_point1
            && self.points_list == other.points_list
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(if self.test_point1_in_set { 1231 } else { 1237 });
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.test_point1.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(list_hash_code(&self.points_list, |l| {
                list_hash_code(l, |p| p.java_hash_code())
            }));
        result
    }

    fn fill_in_edge_description(&self, description: &mut String, start_edge: usize) {
        description.push_str(" {");
        let mut current_edge = start_edge;
        let mut edge_counter = 0;
        loop {
            if edge_counter > 0 {
                description.push_str(", ");
            }
            if edge_counter >= 20 {
                description.push_str("...");
                break;
            }
            description.push_str(&self.edges[current_edge].start_point.to_string());
            current_edge = self.edges[current_edge].next;
            if current_edge == start_edge {
                break;
            }
            edge_counter += 1;
        }
    }
}

impl_geo_base_area_shape_common!(
    GeoComplexPolygon,
    "org.apache.lucene.spatial3d.geom.GeoComplexPolygon",
    geo_polygon
);
impl_geo_base_area_relationship!(GeoComplexPolygon);

impl GeoPolygon for GeoComplexPolygon {}

impl Membership for GeoComplexPolygon {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        match self.is_in_set_result(x, y, z) {
            Ok(b) => b,
            Err(e) => unchecked_panic(e),
        }
    }
}

impl Bounded for GeoComplexPolygon {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(self, &self.planet_model, bounds);
        for &start_edge in &self.shape_start_edges {
            let mut current_edge = start_edge;
            loop {
                let e = &self.edges[current_edge];
                bounds.add_point(&e.start_point);
                bounds.add_plane(
                    &self.planet_model,
                    &e.plane,
                    &[&e.start_plane, &e.end_plane],
                );
                current_edge = e.next;
                if current_edge == start_edge {
                    break;
                }
            }
        }
    }
}

impl GeoShape for GeoComplexPolygon {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.clone()
    }

    fn intersects(
        &self,
        p: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        let mut intersector = IntersectorEdgeIterator {
            polygon: self,
            plane: p,
            notable_points,
            bounds,
        };
        let mut xyz_bounds = XYZBounds::new();
        p.record_bounds_xyz(&self.planet_model, &mut xyz_bounds, bounds);
        for point in notable_points {
            Bounds::add_point(&mut xyz_bounds, point);
        }
        let (Some(max_x), Some(min_x), Some(max_y), Some(min_y), Some(max_z), Some(min_z)) = (
            xyz_bounds.get_maximum_x(),
            xyz_bounds.get_minimum_x(),
            xyz_bounds.get_maximum_y(),
            xyz_bounds.get_minimum_y(),
            xyz_bounds.get_maximum_z(),
            xyz_bounds.get_minimum_z(),
        ) else {
            return false;
        };
        let x_delta = max_x - min_x;
        let y_delta = max_y - min_y;
        let z_delta = max_z - min_z;
        let r = if x_delta <= y_delta && x_delta <= z_delta {
            self.x_tree
                .traverse(&self.edges, &mut intersector, min_x, max_x)
        } else if y_delta <= x_delta && y_delta <= z_delta {
            self.y_tree
                .traverse(&self.edges, &mut intersector, min_y, max_y)
        } else if z_delta <= x_delta && z_delta <= y_delta {
            self.z_tree
                .traverse(&self.edges, &mut intersector, min_z, max_z)
        } else {
            return true;
        };
        match r {
            Ok(b) => !b,
            Err(e) => unchecked_panic(e),
        }
    }
}

impl GeoAreaShape for GeoComplexPolygon {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        let mut intersector = IntersectorShapeIterator { shape: geo_shape };
        let mut xyz_bounds = XYZBounds::new();
        geo_shape.get_bounds(&mut xyz_bounds);
        let ub = |v: Option<f64>| v.unwrap_or_else(|| npe());
        let x_delta = ub(xyz_bounds.get_maximum_x()) - ub(xyz_bounds.get_minimum_x());
        let y_delta = ub(xyz_bounds.get_maximum_y()) - ub(xyz_bounds.get_minimum_y());
        let z_delta = ub(xyz_bounds.get_maximum_z()) - ub(xyz_bounds.get_minimum_z());
        let r = if x_delta <= y_delta && x_delta <= z_delta {
            self.x_tree.traverse(
                &self.edges,
                &mut intersector,
                ub(xyz_bounds.get_minimum_x()),
                ub(xyz_bounds.get_maximum_x()),
            )
        } else if y_delta <= x_delta && y_delta <= z_delta {
            self.y_tree.traverse(
                &self.edges,
                &mut intersector,
                ub(xyz_bounds.get_minimum_y()),
                ub(xyz_bounds.get_maximum_y()),
            )
        } else if z_delta <= x_delta && z_delta <= y_delta {
            self.z_tree.traverse(
                &self.edges,
                &mut intersector,
                ub(xyz_bounds.get_minimum_z()),
                ub(xyz_bounds.get_maximum_z()),
            )
        } else {
            return true;
        };
        match r {
            Ok(b) => !b,
            Err(e) => unchecked_panic(e),
        }
    }
}

impl fmt::Display for GeoComplexPolygon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut edge_description = String::new();
        for &shape_start_edge in &self.shape_start_edges {
            self.fill_in_edge_description(&mut edge_description, shape_start_edge);
        }
        write!(
            f,
            "GeoComplexPolygon: {{planetmodel={}, number of shapes={}, address={:x}, testPoint={}, testPointInSet={}, shapes={{{}}}}}",
            self.planet_model,
            self.shape_start_edges.len(),
            self.hash_code_impl() as u32,
            self.test_point1,
            self.test_point1_in_set,
            edge_description
        )
    }
}

/// An instance of this class describes a single edge, and includes what is
/// necessary to reliably determine intersection in the context of the even/odd
/// algorithm used.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.Edge`;
/// `next` is the index of the next edge in the polygon's edge arena.
pub(crate) struct Edge {
    start_point: GeoPoint,
    end_point: GeoPoint,
    notable_points: [GeoPoint; 2],
    start_plane: SidedPlane,
    end_plane: SidedPlane,
    backing_plane: SidedPlane,
    plane: Plane,
    plane_bounds: XYZBounds,
    next: usize,
}

impl Edge {
    /// `new Edge(PlanetModel, GeoPoint startPoint, GeoPoint endPoint)`.
    fn new(pm: &PlanetModel, start_point: GeoPoint, end_point: GeoPoint) -> Result<Self> {
        let notable_points = [start_point.clone(), end_point.clone()];
        let plane = Plane::new_cross(&start_point, &end_point)?;
        let start_plane = SidedPlane::new_cross(&end_point, &plane, &start_point)?;
        let end_plane = SidedPlane::new_cross(&start_point, &plane, &end_point)?;
        let interpolation_point = plane
            .interpolate(pm, &start_point, &end_point, &HALF_PROPORTIONS)?
            .into_iter()
            .next()
            .unwrap_or_else(|| npe());
        let backing_plane =
            SidedPlane::new_vector_d(&interpolation_point, &interpolation_point, 0.0)?;
        let mut plane_bounds = XYZBounds::new();
        Bounds::add_point(&mut plane_bounds, &start_point);
        Bounds::add_point(&mut plane_bounds, &end_point);
        plane_bounds.add_plane(pm, &plane, &[&start_plane, &end_plane, &backing_plane]);
        Ok(Self {
            start_point,
            end_point,
            notable_points,
            start_plane,
            end_plane,
            backing_plane,
            plane,
            plane_bounds,
            next: usize::MAX,
        })
    }

    /// `isWithin(double, double, double)`.
    fn is_within(&self, the_point_x: f64, the_point_y: f64, the_point_z: f64) -> bool {
        self.plane
            .evaluate_is_zero_xyz(the_point_x, the_point_y, the_point_z)
            && self
                .start_plane
                .is_within_xyz(the_point_x, the_point_y, the_point_z)
            && self
                .end_plane
                .is_within_xyz(the_point_x, the_point_y, the_point_z)
            && self
                .backing_plane
                .is_within_xyz(the_point_x, the_point_y, the_point_z)
    }
}

/// Strategy class for describing traversals. Implements Comparable so that
/// these can be ordered by shortest distance.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.TraversalStrategy`.
pub(crate) struct TraversalStrategy<'a> {
    traversal_distance: f64,
    first_leg_value: f64,
    second_leg_value: f64,
    first_leg_plane: &'a Plane,
    first_leg_above_plane: &'a Plane,
    first_leg_below_plane: &'a Plane,
    second_leg_plane: &'a Plane,
    second_leg_above_plane: &'a Plane,
    second_leg_below_plane: &'a Plane,
    first_leg_tree: &'a Tree,
    second_leg_tree: &'a Tree,
    intersection_point: GeoPoint,
}

impl TraversalStrategy<'_> {
    /// `apply(GeoPoint, boolean, double, double, double)`.
    fn apply(
        &self,
        polygon: &GeoComplexPolygon,
        test_point: &GeoPoint,
        test_point_in_set: bool,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<bool> {
        let attempt = || -> Result<bool> {
            let mut test_point_edge_iterator = polygon.create_linear_crossing_edge_iterator(
                test_point,
                self.first_leg_plane,
                self.first_leg_above_plane,
                self.first_leg_below_plane,
                self.intersection_point.x,
                self.intersection_point.y,
                self.intersection_point.z,
            )?;
            self.first_leg_tree.traverse_value(
                &polygon.edges,
                test_point_edge_iterator.as_mut(),
                self.first_leg_value,
            )?;
            let intersection_point_on_edge = test_point_edge_iterator.is_on_edge();
            if intersection_point_on_edge {
                return Err(iae("Intersection point landed on an edge -- illegal path"));
            }
            let intersection_point_in_set = intersection_point_on_edge
                || if (test_point_edge_iterator.get_crossing_count() & 1) == 0 {
                    test_point_in_set
                } else {
                    !test_point_in_set
                };
            let mut travel_edge_iterator = polygon.create_linear_crossing_edge_iterator(
                &self.intersection_point,
                self.second_leg_plane,
                self.second_leg_above_plane,
                self.second_leg_below_plane,
                x,
                y,
                z,
            )?;
            self.second_leg_tree.traverse_value(
                &polygon.edges,
                travel_edge_iterator.as_mut(),
                self.second_leg_value,
            )?;
            Ok(travel_edge_iterator.is_on_edge()
                || if (travel_edge_iterator.get_crossing_count() & 1) == 0 {
                    intersection_point_in_set
                } else {
                    !intersection_point_in_set
                })
        };
        if let Some(rval) = catch_iae(attempt())? {
            return Ok(rval);
        }
        let mut edge_iterator = DualCrossingEdgeIterator::new(
            polygon,
            test_point,
            self.first_leg_plane,
            self.first_leg_above_plane,
            self.first_leg_below_plane,
            self.second_leg_plane,
            self.second_leg_above_plane,
            self.second_leg_below_plane,
            x,
            y,
            z,
            &self.intersection_point,
        )?;
        self.first_leg_tree.traverse_value(
            &polygon.edges,
            &mut edge_iterator,
            self.first_leg_value,
        )?;
        if edge_iterator.is_on_edge() {
            return Ok(true);
        }
        self.second_leg_tree.traverse_value(
            &polygon.edges,
            &mut edge_iterator,
            self.second_leg_value,
        )?;
        Ok(edge_iterator.is_on_edge()
            || if (edge_iterator.get_crossing_count() & 1) == 0 {
                test_point_in_set
            } else {
                !test_point_in_set
            })
    }

    /// `compareTo(TraversalStrategy)`.
    fn compare_to(&self, other: &Self) -> std::cmp::Ordering {
        if self.traversal_distance < other.traversal_distance {
            std::cmp::Ordering::Less
        } else if self.traversal_distance > other.traversal_distance {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }
}

impl fmt::Display for TraversalStrategy<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{firstLegValue={}; secondLegValue={}; firstLegPlane={}; secondLegPlane={}; intersectionPoint={}}}",
            d2s(self.first_leg_value),
            d2s(self.second_leg_value),
            self.first_leg_plane,
            self.second_leg_plane,
            self.intersection_point
        )
    }
}

/// Iterator execution interface, for tree traversal. Pass an object
/// implementing this interface into the traversal method of a tree, and each
/// edge that matches will cause this object to be called.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.EdgeIterator`;
/// `matches` receives the edge's arena index too, Java's object identity.
pub(crate) trait EdgeIterator {
    /// `matches(Edge)`: whether the traversal goes on.
    fn matches(&mut self, edge_index: usize, edge: &Edge) -> Result<bool>;
}

/// An instance of this class represents a known-good path of nodes that
/// counts crossings.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.CountingEdgeIterator`.
pub(crate) trait CountingEdgeIterator: EdgeIterator {
    /// `getCrossingCount()`.
    fn get_crossing_count(&self) -> i32;
    /// `isOnEdge()`.
    fn is_on_edge(&self) -> bool;
}

/// An instance of this class describes a single edge in a tree.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.Node`;
/// `edge`, `left` and `right` are arena indexes.
pub(crate) struct Node {
    edge: usize,
    low: f64,
    high: f64,
    left: Option<usize>,
    right: Option<usize>,
    max: f64,
}

/// An interval tree over edges.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.Tree`.
/// The abstract `getMinimum`/`getMaximum` of the subclasses are the
/// dimension functions [`XTree`], [`YTree`] and [`ZTree`] pass in.
pub(crate) struct Tree {
    nodes: Vec<Node>,
    root_node: Option<usize>,
}

impl Tree {
    /// `new Tree(List<Edge>)` with the subclass's dimension.
    fn new(
        all_edges: &[Edge],
        get_minimum: fn(&Edge) -> f64,
        get_maximum: fn(&Edge) -> f64,
    ) -> Self {
        let mut nodes: Vec<Node> = all_edges
            .iter()
            .enumerate()
            .map(|(i, edge)| {
                let maximum_value = get_maximum(edge);
                Node {
                    edge: i,
                    low: get_minimum(edge),
                    high: maximum_value,
                    left: None,
                    right: None,
                    max: maximum_value,
                }
            })
            .collect();
        nodes.sort_by(|left, right| {
            let ret = rucene::util::java_double_compare(left.low, right.low);
            if ret == std::cmp::Ordering::Equal {
                rucene::util::java_double_compare(left.max, right.max)
            } else {
                ret
            }
        });
        let len = nodes.len() as i64;
        let root_node = Self::create_tree(&mut nodes, 0, len - 1);
        Self { nodes, root_node }
    }

    fn create_tree(nodes: &mut [Node], low: i64, high: i64) -> Option<usize> {
        if low > high {
            return None;
        }
        let mid = ((low + high) as u64 >> 1) as i64;
        let left = Self::create_tree(nodes, low, mid - 1);
        let right = Self::create_tree(nodes, mid + 1, high);
        let m = mid as usize;
        nodes[m].left = left;
        nodes[m].right = right;
        if let Some(l) = left {
            nodes[m].max = java_max(nodes[m].max, nodes[l].max);
        }
        if let Some(r) = right {
            nodes[m].max = java_max(nodes[m].max, nodes[r].max);
        }
        Some(m)
    }

    /// `traverse(EdgeIterator, double)`.
    fn traverse_value(
        &self,
        edges: &[Edge],
        edge_iterator: &mut dyn EdgeIterator,
        value: f64,
    ) -> Result<bool> {
        self.traverse(edges, edge_iterator, value, value)
    }

    /// `traverse(EdgeIterator, double, double)`.
    fn traverse(
        &self,
        edges: &[Edge],
        edge_iterator: &mut dyn EdgeIterator,
        min_value: f64,
        max_value: f64,
    ) -> Result<bool> {
        match self.root_node {
            None => Ok(true),
            Some(root) => self.traverse_node(root, edges, edge_iterator, min_value, max_value),
        }
    }

    /// `Node.traverse(EdgeIterator, double, double)`.
    fn traverse_node(
        &self,
        node: usize,
        edges: &[Edge],
        edge_iterator: &mut dyn EdgeIterator,
        min_value: f64,
        max_value: f64,
    ) -> Result<bool> {
        let n = &self.nodes[node];
        if min_value <= n.max {
            if min_value <= n.high
                && max_value >= n.low
                && !edge_iterator.matches(n.edge, &edges[n.edge])?
            {
                return Ok(false);
            }
            if let Some(left) = n.left {
                if !self.traverse_node(left, edges, edge_iterator, min_value, max_value)? {
                    return Ok(false);
                }
            }
            if let Some(right) = n.right {
                if max_value >= n.low
                    && !self.traverse_node(right, edges, edge_iterator, min_value, max_value)?
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

fn ub(v: Option<f64>) -> f64 {
    v.unwrap_or_else(|| npe())
}

/// This is the z-tree.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.ZTree`.
pub(crate) struct ZTree;

impl ZTree {
    fn create(all_edges: &[Edge]) -> Tree {
        Tree::new(
            all_edges,
            |e| ub(e.plane_bounds.get_minimum_z()),
            |e| ub(e.plane_bounds.get_maximum_z()),
        )
    }
}

/// This is the y-tree.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.YTree`.
pub(crate) struct YTree;

impl YTree {
    fn create(all_edges: &[Edge]) -> Tree {
        Tree::new(
            all_edges,
            |e| ub(e.plane_bounds.get_minimum_y()),
            |e| ub(e.plane_bounds.get_maximum_y()),
        )
    }
}

/// This is the x-tree.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.XTree`.
pub(crate) struct XTree;

impl XTree {
    fn create(all_edges: &[Edge]) -> Tree {
        Tree::new(
            all_edges,
            |e| ub(e.plane_bounds.get_minimum_x()),
            |e| ub(e.plane_bounds.get_maximum_x()),
        )
    }
}

/// Assess whether edge intersects the provided plane plus bounds.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.IntersectorEdgeIterator`.
pub(crate) struct IntersectorEdgeIterator<'a> {
    polygon: &'a GeoComplexPolygon,
    plane: &'a Plane,
    notable_points: &'a [GeoPoint],
    bounds: &'a [&'a dyn Membership],
}

impl EdgeIterator for IntersectorEdgeIterator<'_> {
    fn matches(&mut self, _edge_index: usize, edge: &Edge) -> Result<bool> {
        Ok(!self.plane.intersects(
            &self.polygon.planet_model,
            &edge.plane,
            self.notable_points,
            &edge.notable_points,
            self.bounds,
            &[&edge.start_plane, &edge.end_plane],
        ))
    }
}

/// Assess whether edge intersects the provided shape.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.IntersectorShapeIterator`.
pub(crate) struct IntersectorShapeIterator<'a> {
    shape: &'a dyn GeoShape,
}

impl EdgeIterator for IntersectorShapeIterator<'_> {
    fn matches(&mut self, _edge_index: usize, edge: &Edge) -> Result<bool> {
        Ok(!self.shape.intersects(
            &edge.plane,
            &edge.notable_points,
            &[&edge.start_plane, &edge.end_plane],
        ))
    }
}

/// Count the number of verifiable edge crossings for a full 1/2 a world.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.FullLinearCrossingEdgeIterator`.
pub(crate) struct FullLinearCrossingEdgeIterator<'a> {
    polygon: &'a GeoComplexPolygon,
    plane: &'a Plane,
    above_plane: &'a Plane,
    below_plane: &'a Plane,
    bound: SidedPlane,
    the_point_x: f64,
    the_point_y: f64,
    the_point_z: f64,
    on_edge: bool,
    above_crossing_count: i32,
    below_crossing_count: i32,
}

impl<'a> FullLinearCrossingEdgeIterator<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        polygon: &'a GeoComplexPolygon,
        test_point: &GeoPoint,
        plane: &'a Plane,
        above_plane: &'a Plane,
        below_plane: &'a Plane,
        the_point_x: f64,
        the_point_y: f64,
        the_point_z: f64,
    ) -> Result<Self> {
        debug_assert!(
            plane.evaluate_is_zero_xyz(the_point_x, the_point_y, the_point_z),
            "Check point is not on travel plane"
        );
        debug_assert!(
            plane.evaluate_is_zero(test_point),
            "Test point is not on travel plane"
        );
        if plane.as_vector().is_numerically_identical(test_point) {
            return Err(iae("Plane vector identical to testpoint vector"));
        }
        let bound = SidedPlane::new_cross_positive(plane, test_point)?;
        Ok(Self {
            polygon,
            plane,
            above_plane,
            below_plane,
            bound,
            the_point_x,
            the_point_y,
            the_point_z,
            on_edge: false,
            above_crossing_count: 0,
            below_crossing_count: 0,
        })
    }

    fn count_crossings(
        &self,
        edge: &Edge,
        envelope_plane: &Plane,
        envelope_bound: &dyn Membership,
    ) -> Result<i32> {
        let intersections = edge.plane.find_intersections(
            &self.polygon.planet_model,
            envelope_plane,
            &[envelope_bound],
        );
        let mut crossings = 0;
        if let Some(intersections) = intersections {
            for intersection in &intersections {
                if edge.start_plane.strictly_within(intersection)
                    && edge.end_plane.strictly_within(intersection)
                {
                    crossings +=
                        if self.edge_crosses_envelope(&edge.plane, intersection, envelope_plane)? {
                            1
                        } else {
                            0
                        };
                }
            }
        }
        Ok(crossings)
    }

    fn edge_crosses_envelope(
        &self,
        edge_plane: &Plane,
        intersection_point: &GeoPoint,
        envelope_plane: &Plane,
    ) -> Result<bool> {
        let Some(adjoining_points) =
            self.polygon
                .find_adjoining_points(edge_plane, intersection_point, envelope_plane)?
        else {
            return Ok(true);
        };
        let mut within_count = 0;
        for adjoining in &adjoining_points {
            if self.plane.evaluate_is_zero(adjoining) && self.bound.is_within(adjoining) {
                within_count += 1;
            }
        }
        Ok((within_count & 1) != 0)
    }
}

impl EdgeIterator for FullLinearCrossingEdgeIterator<'_> {
    fn matches(&mut self, _edge_index: usize, edge: &Edge) -> Result<bool> {
        if edge.is_within(self.the_point_x, self.the_point_y, self.the_point_z) {
            self.on_edge = true;
            return Ok(false);
        }
        let plane_crossings = self.plane.find_intersections(
            &self.polygon.planet_model,
            &edge.plane,
            &[&self.bound, &edge.start_plane, &edge.end_plane],
        );
        if let Some(pc) = &plane_crossings {
            if pc.is_empty()
                && !self.plane.evaluate_is_zero(&edge.start_point)
                && !self.plane.evaluate_is_zero(&edge.end_point)
            {
                return Ok(true);
            }
        }
        let above_crossings = self.count_crossings(edge, self.above_plane, &self.bound)?;
        self.above_crossing_count += above_crossings;
        let below_crossings = self.count_crossings(edge, self.below_plane, &self.bound)?;
        self.below_crossing_count += below_crossings;
        Ok(true)
    }
}

impl CountingEdgeIterator for FullLinearCrossingEdgeIterator<'_> {
    fn get_crossing_count(&self) -> i32 {
        self.above_crossing_count.min(self.below_crossing_count)
    }

    fn is_on_edge(&self) -> bool {
        self.on_edge
    }
}

/// Count the number of verifiable edge crossings for less than 1/2 a world.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.SectorLinearCrossingEdgeIterator`.
pub(crate) struct SectorLinearCrossingEdgeIterator<'a> {
    polygon: &'a GeoComplexPolygon,
    plane: &'a Plane,
    above_plane: &'a Plane,
    below_plane: &'a Plane,
    bound1: SidedPlane,
    bound2: SidedPlane,
    the_point_x: f64,
    the_point_y: f64,
    the_point_z: f64,
    on_edge: bool,
    above_crossing_count: i32,
    below_crossing_count: i32,
}

impl<'a> SectorLinearCrossingEdgeIterator<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        polygon: &'a GeoComplexPolygon,
        test_point: &GeoPoint,
        plane: &'a Plane,
        above_plane: &'a Plane,
        below_plane: &'a Plane,
        the_point_x: f64,
        the_point_y: f64,
        the_point_z: f64,
    ) -> Result<Self> {
        debug_assert!(
            plane.evaluate_is_zero_xyz(the_point_x, the_point_y, the_point_z),
            "Check point is not on travel plane"
        );
        debug_assert!(
            plane.evaluate_is_zero(test_point),
            "Test point is not on travel plane"
        );
        let bound1_plane =
            SidedPlane::new_xyz_cross(the_point_x, the_point_y, the_point_z, plane, test_point)?;
        let bound2_plane =
            SidedPlane::new_cross_xyz(test_point, plane, the_point_x, the_point_y, the_point_z)?;
        if bound1_plane.is_numerically_identical(&bound2_plane) {
            return Err(iae(
                "Sector iterator unreliable when bounds planes are numerically identical",
            ));
        }
        Ok(Self {
            polygon,
            plane,
            above_plane,
            below_plane,
            bound1: bound1_plane,
            bound2: bound2_plane,
            the_point_x,
            the_point_y,
            the_point_z,
            on_edge: false,
            above_crossing_count: 0,
            below_crossing_count: 0,
        })
    }

    fn count_crossings(&self, edge: &Edge, envelope_plane: &Plane) -> Result<i32> {
        let intersections = edge.plane.find_intersections(
            &self.polygon.planet_model,
            envelope_plane,
            &[&self.bound1, &self.bound2],
        );
        let mut crossings = 0;
        if let Some(intersections) = intersections {
            for intersection in &intersections {
                if edge.start_plane.strictly_within(intersection)
                    && edge.end_plane.strictly_within(intersection)
                {
                    let counter =
                        if self.edge_crosses_envelope(&edge.plane, intersection, envelope_plane)? {
                            1
                        } else {
                            0
                        };
                    crossings += counter;
                }
            }
        }
        Ok(crossings)
    }

    fn edge_crosses_envelope(
        &self,
        edge_plane: &Plane,
        intersection_point: &GeoPoint,
        envelope_plane: &Plane,
    ) -> Result<bool> {
        let Some(adjoining_points) =
            self.polygon
                .find_adjoining_points(edge_plane, intersection_point, envelope_plane)?
        else {
            return Ok(true);
        };
        let mut within_count = 0;
        for adjoining in &adjoining_points {
            if self.plane.evaluate_is_zero(adjoining)
                && self.bound1.is_within(adjoining)
                && self.bound2.is_within(adjoining)
            {
                within_count += 1;
            }
        }
        Ok((within_count & 1) != 0)
    }
}

impl EdgeIterator for SectorLinearCrossingEdgeIterator<'_> {
    fn matches(&mut self, _edge_index: usize, edge: &Edge) -> Result<bool> {
        if edge.is_within(self.the_point_x, self.the_point_y, self.the_point_z) {
            self.on_edge = true;
            return Ok(false);
        }
        let plane_crossings = self.plane.find_intersections(
            &self.polygon.planet_model,
            &edge.plane,
            &[
                &self.bound1,
                &self.bound2,
                &edge.start_plane,
                &edge.end_plane,
            ],
        );
        if let Some(pc) = &plane_crossings {
            if pc.is_empty()
                && !self.plane.evaluate_is_zero(&edge.start_point)
                && !self.plane.evaluate_is_zero(&edge.end_point)
            {
                return Ok(true);
            }
        }
        let above_crossings = self.count_crossings(edge, self.above_plane)?;
        self.above_crossing_count += above_crossings;
        let below_crossings = self.count_crossings(edge, self.below_plane)?;
        self.below_crossing_count += below_crossings;
        Ok(true)
    }
}

impl CountingEdgeIterator for SectorLinearCrossingEdgeIterator<'_> {
    fn get_crossing_count(&self) -> i32 {
        self.above_crossing_count.min(self.below_crossing_count)
    }

    fn is_on_edge(&self) -> bool {
        self.on_edge
    }
}

/// Count the number of verifiable edge crossings for a dual-leg journey.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoComplexPolygon.DualCrossingEdgeIterator`.
pub(crate) struct DualCrossingEdgeIterator<'a> {
    polygon: &'a GeoComplexPolygon,
    /// Edges already seen, by arena index (Java's identity `HashSet<Edge>`).
    seen_edges: Option<HashSet<usize>>,
    test_point: GeoPoint,
    test_point_plane: &'a Plane,
    test_point_above_plane: &'a Plane,
    test_point_below_plane: &'a Plane,
    travel_plane: &'a Plane,
    travel_above_plane: &'a Plane,
    travel_below_plane: &'a Plane,
    the_point_x: f64,
    the_point_y: f64,
    the_point_z: f64,
    intersection_point: GeoPoint,
    test_point_cutoff_plane: SidedPlane,
    check_point_cutoff_plane: SidedPlane,
    test_point_other_cutoff_plane: SidedPlane,
    check_point_other_cutoff_plane: SidedPlane,
    computed_inside_outside: bool,
    test_point_inside_plane: Option<&'a Plane>,
    test_point_outside_plane: Option<&'a Plane>,
    travel_inside_plane: Option<&'a Plane>,
    travel_outside_plane: Option<&'a Plane>,
    inside_test_point_cutoff_plane: Option<SidedPlane>,
    inside_travel_cutoff_plane: Option<SidedPlane>,
    outside_test_point_cutoff_plane: Option<SidedPlane>,
    outside_travel_cutoff_plane: Option<SidedPlane>,
    on_edge: bool,
    inner_crossing_count: i32,
    outer_crossing_count: i32,
}

impl<'a> DualCrossingEdgeIterator<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        polygon: &'a GeoComplexPolygon,
        test_point: &GeoPoint,
        test_point_plane: &'a Plane,
        test_point_above_plane: &'a Plane,
        test_point_below_plane: &'a Plane,
        travel_plane: &'a Plane,
        travel_above_plane: &'a Plane,
        travel_below_plane: &'a Plane,
        the_point_x: f64,
        the_point_y: f64,
        the_point_z: f64,
        intersection_point: &GeoPoint,
    ) -> Result<Self> {
        debug_assert!(
            travel_plane.evaluate_is_zero(intersection_point),
            "intersection point must be on travel plane"
        );
        debug_assert!(
            test_point_plane.evaluate_is_zero(intersection_point),
            "intersection point must be on test point plane"
        );
        debug_assert!(
            !test_point.is_numerically_identical(intersection_point),
            "test point is the same as intersection point"
        );
        debug_assert!(
            !intersection_point.is_numerically_identical_xyz(the_point_x, the_point_y, the_point_z),
            "check point is same as intersection point"
        );
        let test_point_bound1 =
            SidedPlane::new_cross(intersection_point, test_point_plane, test_point)?;
        let test_point_bound2 =
            SidedPlane::new_cross(test_point, test_point_plane, intersection_point)?;
        if test_point_bound1.is_functionally_identical(&test_point_bound2) {
            return Err(iae(
                "Dual iterator unreliable when bounds planes are functionally identical",
            ));
        }
        let check_point_bound1 = SidedPlane::new_cross_xyz(
            intersection_point,
            travel_plane,
            the_point_x,
            the_point_y,
            the_point_z,
        )?;
        let check_point_bound2 = SidedPlane::new_xyz_cross(
            the_point_x,
            the_point_y,
            the_point_z,
            travel_plane,
            intersection_point,
        )?;
        if check_point_bound1.is_functionally_identical(&check_point_bound2) {
            return Err(iae(
                "Dual iterator unreliable when bounds planes are functionally identical",
            ));
        }
        debug_assert!(
            test_point_bound1.is_within(intersection_point),
            "intersection must be within testPointCutoffPlane"
        );
        debug_assert!(
            test_point_bound2.is_within(intersection_point),
            "intersection must be within testPointOtherCutoffPlane"
        );
        debug_assert!(
            check_point_bound1.is_within(intersection_point),
            "intersection must be within checkPointCutoffPlane"
        );
        debug_assert!(
            check_point_bound2.is_within(intersection_point),
            "intersection must be within checkPointOtherCutoffPlane"
        );
        Ok(Self {
            polygon,
            seen_edges: None,
            test_point: test_point.clone(),
            test_point_plane,
            test_point_above_plane,
            test_point_below_plane,
            travel_plane,
            travel_above_plane,
            travel_below_plane,
            the_point_x,
            the_point_y,
            the_point_z,
            intersection_point: intersection_point.clone(),
            test_point_cutoff_plane: test_point_bound1,
            check_point_cutoff_plane: check_point_bound1,
            test_point_other_cutoff_plane: test_point_bound2,
            check_point_other_cutoff_plane: check_point_bound2,
            computed_inside_outside: false,
            test_point_inside_plane: None,
            test_point_outside_plane: None,
            travel_inside_plane: None,
            travel_outside_plane: None,
            inside_test_point_cutoff_plane: None,
            inside_travel_cutoff_plane: None,
            outside_test_point_cutoff_plane: None,
            outside_travel_cutoff_plane: None,
            on_edge: false,
            inner_crossing_count: 0,
            outer_crossing_count: 0,
        })
    }

    /// `computeInsideOutside()`.
    // Java's late-assigned local.
    #[allow(clippy::needless_late_init)]
    fn compute_inside_outside(&mut self) -> Result<()> {
        if self.computed_inside_outside {
            return Ok(());
        }
        let pm = &*self.polygon.planet_model;
        let intersection_bound1 =
            SidedPlane::new_vector_d(&self.test_point, self.travel_plane, self.travel_plane.d)?;
        let intersection_bound2 = SidedPlane::new_xyz_vector_d(
            self.the_point_x,
            self.the_point_y,
            self.the_point_z,
            self.test_point_plane,
            self.test_point_plane.d,
        )?;
        debug_assert!(
            intersection_bound1.is_within(&self.intersection_point),
            "intersection must be within intersectionBound1"
        );
        debug_assert!(
            intersection_bound2.is_within(&self.intersection_point),
            "intersection must be within intersectionBound2"
        );
        let bounds: [&dyn Membership; 2] = [&intersection_bound1, &intersection_bound2];
        let above_above = self
            .travel_above_plane
            .find_intersections(pm, self.test_point_above_plane, &bounds)
            .unwrap_or_else(|| npe());
        let above_below = self
            .travel_above_plane
            .find_intersections(pm, self.test_point_below_plane, &bounds)
            .unwrap_or_else(|| npe());
        let below_below = self
            .travel_below_plane
            .find_intersections(pm, self.test_point_below_plane, &bounds)
            .unwrap_or_else(|| npe());
        let below_above = self
            .travel_below_plane
            .find_intersections(pm, self.test_point_above_plane, &bounds)
            .unwrap_or_else(|| npe());
        debug_assert!(
            (if !above_above.is_empty() { 1 } else { 0 })
                + (if !above_below.is_empty() { 1 } else { 0 })
                + (if !below_below.is_empty() { 1 } else { 0 })
                + (if !below_above.is_empty() { 1 } else { 0 })
                == 1,
            "Can be exactly one inside point, instead was: aa={} xyScaling={} bb={} ba={}",
            above_above.len(),
            above_below.len(),
            below_below.len(),
            below_above.len()
        );
        let inside_inside_points;
        if !above_above.is_empty() {
            self.travel_inside_plane = Some(self.travel_above_plane);
            self.test_point_inside_plane = Some(self.test_point_above_plane);
            self.travel_outside_plane = Some(self.travel_below_plane);
            self.test_point_outside_plane = Some(self.test_point_below_plane);
            inside_inside_points = above_above;
        } else if !above_below.is_empty() {
            self.travel_inside_plane = Some(self.travel_above_plane);
            self.test_point_inside_plane = Some(self.test_point_below_plane);
            self.travel_outside_plane = Some(self.travel_below_plane);
            self.test_point_outside_plane = Some(self.test_point_above_plane);
            inside_inside_points = above_below;
        } else if !below_below.is_empty() {
            self.travel_inside_plane = Some(self.travel_below_plane);
            self.test_point_inside_plane = Some(self.test_point_below_plane);
            self.travel_outside_plane = Some(self.travel_above_plane);
            self.test_point_outside_plane = Some(self.test_point_above_plane);
            inside_inside_points = below_below;
        } else if !below_above.is_empty() {
            self.travel_inside_plane = Some(self.travel_below_plane);
            self.test_point_inside_plane = Some(self.test_point_above_plane);
            self.travel_outside_plane = Some(self.travel_above_plane);
            self.test_point_outside_plane = Some(self.test_point_below_plane);
            inside_inside_points = below_above;
        } else {
            return Err(ise(format!(
                "Can't find traversal intersection among: {}, {}, {}, {}",
                self.travel_above_plane,
                self.test_point_above_plane,
                self.travel_below_plane,
                self.test_point_below_plane
            )));
        }
        let inside_inside_point = self.pick_proximate(&inside_inside_points)?;
        let test_point_outside_plane = self.test_point_outside_plane.unwrap_or_else(|| npe());
        let travel_outside_plane = self.travel_outside_plane.unwrap_or_else(|| npe());
        let travel_inside_plane = self.travel_inside_plane.unwrap_or_else(|| npe());
        let test_point_inside_plane = self.test_point_inside_plane.unwrap_or_else(|| npe());
        let outside_outside_points = test_point_outside_plane
            .find_intersections(pm, travel_outside_plane, &[])
            .unwrap_or_else(|| npe());
        let outside_outside_point = self.pick_proximate(&outside_outside_points)?;
        self.inside_travel_cutoff_plane = Some(SidedPlane::new_xyz_cross(
            self.the_point_x,
            self.the_point_y,
            self.the_point_z,
            travel_inside_plane,
            &inside_inside_point,
        )?);
        self.outside_travel_cutoff_plane = Some(SidedPlane::new_xyz_cross(
            self.the_point_x,
            self.the_point_y,
            self.the_point_z,
            travel_inside_plane,
            &outside_outside_point,
        )?);
        self.inside_test_point_cutoff_plane = Some(SidedPlane::new_cross(
            &self.test_point,
            test_point_inside_plane,
            &inside_inside_point,
        )?);
        self.outside_test_point_cutoff_plane = Some(SidedPlane::new_cross(
            &self.test_point,
            test_point_outside_plane,
            &outside_outside_point,
        )?);
        self.computed_inside_outside = true;
        Ok(())
    }

    fn pick_proximate(&self, points: &[GeoPoint]) -> Result<GeoPoint> {
        if points.is_empty() {
            Err(iae(
                "No off-plane intersection points were found; can't compute traversal",
            ))
        } else if points.len() == 1 {
            Ok(points[0].clone())
        } else {
            let p1dist =
                GeoComplexPolygon::compute_squared_distance(&points[0], &self.intersection_point);
            let p2dist =
                GeoComplexPolygon::compute_squared_distance(&points[1], &self.intersection_point);
            if p1dist < p2dist {
                Ok(points[0].clone())
            } else if p2dist < p1dist {
                Ok(points[1].clone())
            } else {
                Err(iae(format!(
                    "Neither off-plane intersection point matched intersection point; intersection = {}; offplane choice 0: {}; offplane choice 1: {}",
                    self.intersection_point, points[0], points[1]
                )))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn count_crossings(
        &self,
        edge: &Edge,
        travel_envelope_plane: &Plane,
        travel_envelope_bound1: &dyn Membership,
        travel_envelope_bound2: &dyn Membership,
        test_point_envelope_plane: &Plane,
        test_point_envelope_bound1: &dyn Membership,
        test_point_envelope_bound2: &dyn Membership,
    ) -> Result<i32> {
        let pm = &*self.polygon.planet_model;
        let travel_intersections = edge.plane.find_intersections(
            pm,
            travel_envelope_plane,
            &[travel_envelope_bound1, travel_envelope_bound2],
        );
        let test_point_intersections = edge.plane.find_intersections(
            pm,
            test_point_envelope_plane,
            &[test_point_envelope_bound1, test_point_envelope_bound2],
        );
        let mut crossings = 0;
        if let Some(travel_intersections) = &travel_intersections {
            for intersection in travel_intersections {
                if edge.start_plane.strictly_within(intersection)
                    && edge.end_plane.strictly_within(intersection)
                {
                    let mut not_dup = true;
                    if let Some(tpi) = &test_point_intersections {
                        for other_intersection in tpi {
                            if edge.start_plane.strictly_within(other_intersection)
                                && edge.end_plane.strictly_within(other_intersection)
                                && intersection.is_numerically_identical(other_intersection)
                            {
                                not_dup = false;
                                break;
                            }
                        }
                    }
                    if !not_dup {
                        continue;
                    }
                    crossings += if self.edge_crosses_envelope(
                        &edge.plane,
                        intersection,
                        travel_envelope_plane,
                    )? {
                        1
                    } else {
                        0
                    };
                }
            }
        }
        if let Some(tpi) = &test_point_intersections {
            for intersection in tpi {
                if edge.start_plane.strictly_within(intersection)
                    && edge.end_plane.strictly_within(intersection)
                {
                    crossings += if self.edge_crosses_envelope(
                        &edge.plane,
                        intersection,
                        test_point_envelope_plane,
                    )? {
                        1
                    } else {
                        0
                    };
                }
            }
        }
        Ok(crossings)
    }

    fn edge_crosses_envelope(
        &self,
        edge_plane: &Plane,
        intersection_point: &GeoPoint,
        envelope_plane: &Plane,
    ) -> Result<bool> {
        let Some(adjoining_points) =
            self.polygon
                .find_adjoining_points(edge_plane, intersection_point, envelope_plane)?
        else {
            return Ok(true);
        };
        let mut within_count = 0;
        for adjoining in &adjoining_points {
            if (self.travel_plane.evaluate_is_zero(adjoining)
                && self.check_point_cutoff_plane.is_within(adjoining)
                && self.check_point_other_cutoff_plane.is_within(adjoining))
                || (self.test_point_plane.evaluate_is_zero(adjoining)
                    && self.test_point_cutoff_plane.is_within(adjoining)
                    && self.test_point_other_cutoff_plane.is_within(adjoining))
            {
                within_count += 1;
            }
        }
        Ok((within_count & 1) != 0)
    }
}

impl EdgeIterator for DualCrossingEdgeIterator<'_> {
    fn matches(&mut self, edge_index: usize, edge: &Edge) -> Result<bool> {
        if edge.is_within(self.the_point_x, self.the_point_y, self.the_point_z) {
            self.on_edge = true;
            return Ok(false);
        }
        if let Some(seen) = &self.seen_edges {
            if seen.contains(&edge_index) {
                return Ok(true);
            }
        }
        self.seen_edges
            .get_or_insert_with(HashSet::new)
            .insert(edge_index);
        self.compute_inside_outside()?;
        let pm = &*self.polygon.planet_model;
        let travel_crossings = self.travel_plane.find_intersections(
            pm,
            &edge.plane,
            &[
                &self.check_point_cutoff_plane,
                &self.check_point_other_cutoff_plane,
                &edge.start_plane,
                &edge.end_plane,
            ],
        );
        if let Some(tc) = &travel_crossings {
            if tc.is_empty() {
                let test_point_crossings = self.test_point_plane.find_intersections(
                    pm,
                    &edge.plane,
                    &[
                        &self.test_point_cutoff_plane,
                        &self.test_point_other_cutoff_plane,
                        &edge.start_plane,
                        &edge.end_plane,
                    ],
                );
                if let Some(tpc) = &test_point_crossings {
                    if tpc.is_empty()
                        && !self.travel_plane.evaluate_is_zero(&edge.start_point)
                        && !self.travel_plane.evaluate_is_zero(&edge.end_point)
                        && !self.test_point_plane.evaluate_is_zero(&edge.start_point)
                        && !self.test_point_plane.evaluate_is_zero(&edge.end_point)
                    {
                        return Ok(true);
                    }
                }
            }
        }
        let travel_inside_plane = self.travel_inside_plane.unwrap_or_else(|| npe());
        let test_point_inside_plane = self.test_point_inside_plane.unwrap_or_else(|| npe());
        let travel_outside_plane = self.travel_outside_plane.unwrap_or_else(|| npe());
        let test_point_outside_plane = self.test_point_outside_plane.unwrap_or_else(|| npe());
        let inside_travel = self
            .inside_travel_cutoff_plane
            .as_ref()
            .unwrap_or_else(|| npe());
        let inside_test = self
            .inside_test_point_cutoff_plane
            .as_ref()
            .unwrap_or_else(|| npe());
        let outside_travel = self
            .outside_travel_cutoff_plane
            .as_ref()
            .unwrap_or_else(|| npe());
        let outside_test = self
            .outside_test_point_cutoff_plane
            .as_ref()
            .unwrap_or_else(|| npe());
        let inner = self.count_crossings(
            edge,
            travel_inside_plane,
            &self.check_point_cutoff_plane,
            inside_travel,
            test_point_inside_plane,
            &self.test_point_cutoff_plane,
            inside_test,
        )?;
        self.inner_crossing_count += inner;
        let outer = self.count_crossings(
            edge,
            travel_outside_plane,
            &self.check_point_cutoff_plane,
            outside_travel,
            test_point_outside_plane,
            &self.test_point_cutoff_plane,
            outside_test,
        )?;
        self.outer_crossing_count += outer;
        Ok(true)
    }
}

impl CountingEdgeIterator for DualCrossingEdgeIterator<'_> {
    fn get_crossing_count(&self) -> i32 {
        self.inner_crossing_count.min(self.outer_crossing_count)
    }

    fn is_on_edge(&self) -> bool {
        self.on_edge
    }
}
