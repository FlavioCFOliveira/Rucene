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


//! The types declared in `GeoPolygonFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use super::geo_complex_polygon::GeoComplexPolygon;
use super::geo_composite_polygon::GeoCompositePolygon;
use super::geo_concave_polygon::GeoConcavePolygon;
use super::geo_convex_polygon::GeoConvexPolygon;
use super::geo_point::GeoPoint;
use super::geo_polygon::GeoPolygon;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::sided_plane::SidedPlane;
use super::vector::Vector;
use rucene::util::java_random::JavaRandom;

use super::{iae, iae_message, list_to_string, npe, rte, JavaBitSet};

/// Class which constructs a GeoMembershipShape representing an arbitrary
/// polygon.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory`. A
/// `null` polygon result is `None`. The factory compares points by identity
/// (`==`) as Java does: internally points are shared [`Arc`]s compared with
/// [`Arc::ptr_eq`], one per input list element.
pub struct GeoPolygonFactory;

/// Use this class to specify a polygon with associated holes.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.PolygonDescription`.
#[derive(Clone, Debug)]
pub struct PolygonDescription {
    /// The list of points.
    pub points: Vec<GeoPoint>,
    /// The list of holes.
    pub holes: Vec<PolygonDescription>,
}

impl PolygonDescription {
    /// `new PolygonDescription(List<? extends GeoPoint>)`.
    pub fn new(points: Vec<GeoPoint>) -> Self {
        Self::with_holes(points, Vec::new())
    }

    /// `new PolygonDescription(List<? extends GeoPoint>, List<? extends
    /// PolygonDescription>)`.
    pub fn with_holes(points: Vec<GeoPoint>, holes: Vec<PolygonDescription>) -> Self {
        Self { points, holes }
    }
}

type Pt = Arc<GeoPoint>;
type Holes = Option<Vec<Arc<dyn GeoPolygon>>>;

/// A `TileException` or an unchecked exception.
enum FactoryError {
    Tile(TileException),
    Other(LuceneError),
}

impl From<LuceneError> for FactoryError {
    fn from(e: LuceneError) -> Self {
        FactoryError::Other(e)
    }
}

impl From<TileException> for FactoryError {
    fn from(e: TileException) -> Self {
        FactoryError::Tile(e)
    }
}

type FResult<T> = std::result::Result<T, FactoryError>;

fn to_points(pts: &[Pt]) -> Vec<GeoPoint> {
    pts.iter().map(|p| (**p).clone()).collect()
}

fn holes_size(holes: &Holes) -> usize {
    holes.as_ref().map_or(0, |h| h.len())
}

/// The point list of a polygon as `List.toString()` prints it.
fn points_to_string(pts: &[Pt]) -> String {
    list_to_string(&to_points(pts))
}

impl GeoPolygonFactory {
    /// Minimum point count to consider a polygon "large".
    const SMALL_POLYGON_CUTOFF_EDGES: usize = 100;

    /// `makeGeoConcavePolygon(PlanetModel, List<GeoPoint>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_concave_polygon(planet_model: &Arc<PlanetModel>, point_list: Vec<GeoPoint>) -> Result<Arc<dyn GeoPolygon>> {
        Ok(Arc::new(GeoConcavePolygon::new(planet_model, point_list)?))
    }

    /// `makeGeoConvexPolygon(PlanetModel, List<GeoPoint>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_convex_polygon(planet_model: &Arc<PlanetModel>, point_list: Vec<GeoPoint>) -> Result<Arc<dyn GeoPolygon>> {
        Ok(Arc::new(GeoConvexPolygon::new(planet_model, point_list)?))
    }

    /// `makeGeoConcavePolygon(PlanetModel, List<GeoPoint>, List<GeoPolygon>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_concave_polygon_with_holes(
        planet_model: &Arc<PlanetModel>,
        point_list: Vec<GeoPoint>,
        holes: Option<Vec<Arc<dyn GeoPolygon>>>,
    ) -> Result<Arc<dyn GeoPolygon>> {
        Ok(Arc::new(GeoConcavePolygon::new_with_holes(planet_model, point_list, holes)?))
    }

    /// `makeGeoConvexPolygon(PlanetModel, List<GeoPoint>, List<GeoPolygon>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_convex_polygon_with_holes(
        planet_model: &Arc<PlanetModel>,
        point_list: Vec<GeoPoint>,
        holes: Option<Vec<Arc<dyn GeoPolygon>>>,
    ) -> Result<Arc<dyn GeoPolygon>> {
        Ok(Arc::new(GeoConvexPolygon::new_with_holes(planet_model, point_list, holes)?))
    }

    /// `makeGeoPolygon(PlanetModel, PolygonDescription)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_polygon_from_description(
        planet_model: &Arc<PlanetModel>,
        description: &PolygonDescription,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        Self::make_geo_polygon_from_description_with_leniency(planet_model, description, 0.0)
    }

    /// `makeGeoPolygon(PlanetModel, PolygonDescription, double leniencyValue)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_polygon_from_description_with_leniency(
        planet_model: &Arc<PlanetModel>,
        description: &PolygonDescription,
        leniency_value: f64,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let holes: Holes = if !description.holes.is_empty() {
            let mut holes: Vec<Arc<dyn GeoPolygon>> = Vec::with_capacity(description.holes.len());
            for hole_description in &description.holes {
                let Some(gp) = Self::make_geo_polygon_from_description_with_leniency(planet_model, hole_description, leniency_value)? else {
                    return Ok(None);
                };
                holes.push(gp);
            }
            Some(holes)
        } else {
            None
        };
        if description.points.len() <= Self::SMALL_POLYGON_CUTOFF_EDGES {
            let input: Vec<Pt> = description.points.iter().cloned().map(Arc::new).collect();
            let Some(first_filtered_point_list) = Self::filter_points(&input) else {
                return Ok(None);
            };
            let Some(filtered_point_list) = Self::filter_edges(&first_filtered_point_list, leniency_value)? else {
                return Ok(None);
            };
            match Self::find_and_generate(planet_model, &filtered_point_list, &holes) {
                Ok(p) => return Ok(Some(p)),
                Err(FactoryError::Other(e)) => return Err(e),
                Err(FactoryError::Tile(_)) => {}
            }
        }
        let pd = vec![description.clone()];
        Ok(Some(Self::make_large_geo_polygon(planet_model, &pd)?))
    }

    /// The body shared by the two `makeGeoPolygon` forms: center of mass, then
    /// random poles, then `IllegalArgumentException`.
    fn find_and_generate(planet_model: &Arc<PlanetModel>, filtered_point_list: &[Pt], holes: &Holes) -> FResult<Arc<dyn GeoPolygon>> {
        let center_of_mass = Arc::new(Self::get_center_of_mass(planet_model, filtered_point_list));
        if let Some(is_center_of_mass_inside) = Self::is_inside_polygon(&center_of_mass, filtered_point_list) {
            return Self::generate_geo_polygon(planet_model, filtered_point_list, holes, &center_of_mass, is_center_of_mass_inside);
        }
        let mut generator = JavaRandom::new(1234);
        for _ in 0..1_000_000 {
            let pole = Arc::new(Self::pick_pole(&mut generator, planet_model, filtered_point_list)?);
            if let Some(is_pole_inside) = Self::is_inside_polygon(&pole, filtered_point_list) {
                return Self::generate_geo_polygon(planet_model, filtered_point_list, holes, &pole, is_pole_inside);
            }
        }
        Err(FactoryError::Other(iae(format!(
            "cannot find a point that is inside the polygon {}",
            points_to_string(filtered_point_list)
        ))))
    }

    /// `makeGeoPolygon(PlanetModel, List<GeoPoint>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_polygon(planet_model: &Arc<PlanetModel>, point_list: Vec<GeoPoint>) -> Result<Option<Arc<dyn GeoPolygon>>> {
        Self::make_geo_polygon_with_holes(planet_model, point_list, None)
    }

    /// `makeGeoPolygon(PlanetModel, List<GeoPoint>, List<GeoPolygon>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_polygon_with_holes(
        planet_model: &Arc<PlanetModel>,
        point_list: Vec<GeoPoint>,
        holes: Option<Vec<Arc<dyn GeoPolygon>>>,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        Self::make_geo_polygon_with_leniency(planet_model, point_list, holes, 0.0)
    }

    /// `makeGeoPolygon(PlanetModel, List<GeoPoint>, List<GeoPolygon>, double
    /// leniencyValue)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal polygon.
    pub fn make_geo_polygon_with_leniency(
        planet_model: &Arc<PlanetModel>,
        point_list: Vec<GeoPoint>,
        holes: Option<Vec<Arc<dyn GeoPolygon>>>,
        leniency_value: f64,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let input: Vec<Pt> = point_list.iter().cloned().map(Arc::new).collect();
        let Some(first_filtered_point_list) = Self::filter_points(&input) else {
            return Ok(None);
        };
        let Some(filtered_point_list) = Self::filter_edges(&first_filtered_point_list, leniency_value)? else {
            return Ok(None);
        };
        match Self::find_and_generate(planet_model, &filtered_point_list, &holes) {
            Ok(p) => Ok(Some(p)),
            Err(FactoryError::Other(e)) => Err(e),
            Err(FactoryError::Tile(e)) => {
                if holes_size(&holes) > 0 {
                    return Err(iae(e.message));
                }
                let description = vec![PolygonDescription::new(point_list)];
                Ok(Some(Self::make_large_geo_polygon(planet_model, &description)?))
            }
        }
    }

    fn get_center_of_mass(planet_model: &PlanetModel, points: &[Pt]) -> GeoPoint {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut z = 0.0;
        for point in points {
            x += point.x;
            y += point.y;
            z += point.z;
        }
        planet_model.create_surface_point_xyz(x, y, z)
    }

    /// `makeLargeGeoPolygon(PlanetModel, List<PolygonDescription>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when no in-set determination is possible.
    pub fn make_large_geo_polygon(planet_model: &Arc<PlanetModel>, shapes_list: &[PolygonDescription]) -> Result<Arc<dyn GeoPolygon>> {
        let mut points_list: Vec<Vec<Pt>> = Vec::new();
        let mut test_point_shape: Option<BestShape> = None;
        for shape in shapes_list {
            test_point_shape = Self::convert_polygon(&mut points_list, shape, test_point_shape, true);
        }
        let Some(test_point_shape) = test_point_shape else {
            return Err(iae("couldn't find a non-degenerate polygon for in-set determination"));
        };
        let plain_points_list: Vec<Vec<GeoPoint>> = points_list.iter().map(|l| to_points(l)).collect();
        let center_of_mass = Self::get_center_of_mass(planet_model, &test_point_shape.points);
        if let Some(com_rval) = test_point_shape.create_geo_complex_polygon(planet_model, &plain_points_list, &center_of_mass)? {
            return Ok(Arc::new(com_rval));
        }
        let mut generator = JavaRandom::new(1234);
        for _ in 0..1_000_000 {
            let pole = Self::pick_pole(&mut generator, planet_model, &test_point_shape.points)?;
            if let Some(rval) = test_point_shape.create_geo_complex_polygon(planet_model, &plain_points_list, &pole)? {
                return Ok(Arc::new(rval));
            }
        }
        Err(iae(format!("cannot find a point that is inside the polygon {}", test_point_shape)))
    }

    fn convert_polygon(
        points_list: &mut Vec<Vec<Pt>>,
        shape: &PolygonDescription,
        mut test_point_shape: Option<BestShape>,
        must_be_inside: bool,
    ) -> Option<BestShape> {
        let input: Vec<Pt> = shape.points.iter().cloned().map(Arc::new).collect();
        let Some(filtered_points) = Self::filter_points(&input) else {
            return test_point_shape;
        };
        if shape.holes.is_empty() && test_point_shape.as_ref().is_none_or(|t| t.points.len() > filtered_points.len()) {
            test_point_shape = Some(BestShape::new(filtered_points.clone(), must_be_inside));
        }
        points_list.push(filtered_points);
        for hole in &shape.holes {
            test_point_shape = Self::convert_polygon(points_list, hole, test_point_shape, !must_be_inside);
        }
        test_point_shape
    }

    /// `generateGeoPolygon(PlanetModel, List<GeoPoint>, List<GeoPolygon>,
    /// GeoPoint, boolean)`.
    fn generate_geo_polygon(
        planet_model: &Arc<PlanetModel>,
        filtered_point_list: &[Pt],
        holes: &Holes,
        test_point: &Pt,
        test_point_inside: bool,
    ) -> FResult<Arc<dyn GeoPolygon>> {
        let initial_plane = SidedPlane::new_cross(test_point, &filtered_point_list[0], &filtered_point_list[1])?;
        let mut rval = GeoCompositePolygon::new(planet_model);
        let mut seen_concave = MutableBoolean::default();
        if !Self::build_polygon_shape(
            &mut rval,
            &mut seen_concave,
            planet_model,
            filtered_point_list,
            &JavaBitSet::new(),
            0,
            1,
            initial_plane.clone(),
            holes,
            Some(test_point),
        )? {
            if test_point_inside {
                let mut rval = GeoCompositePolygon::new(planet_model);
                let mut seen_concave = MutableBoolean::default();
                Self::build_polygon_shape(
                    &mut rval,
                    &mut seen_concave,
                    planet_model,
                    filtered_point_list,
                    &JavaBitSet::new(),
                    0,
                    1,
                    initial_plane,
                    holes,
                    None,
                )?;
                return Ok(Arc::new(rval));
            }
            let mut rval = GeoCompositePolygon::new(planet_model);
            let mut seen_concave = MutableBoolean::default();
            Self::build_polygon_shape(
                &mut rval,
                &mut seen_concave,
                planet_model,
                filtered_point_list,
                &JavaBitSet::new(),
                0,
                1,
                SidedPlane::new_inverse(&initial_plane),
                holes,
                None,
            )?;
            Ok(Arc::new(rval))
        } else {
            if !test_point_inside {
                return Ok(Arc::new(rval));
            }
            let mut rval = GeoCompositePolygon::new(planet_model);
            let mut seen_concave = MutableBoolean::default();
            Self::build_polygon_shape(
                &mut rval,
                &mut seen_concave,
                planet_model,
                filtered_point_list,
                &JavaBitSet::new(),
                0,
                1,
                SidedPlane::new_inverse(&initial_plane),
                holes,
                None,
            )?;
            Ok(Arc::new(rval))
        }
    }

    /// `filterPoints(List<? extends GeoPoint>)`: `None` when the polygon is
    /// degenerate.
    fn filter_points(input: &[Pt]) -> Option<Vec<Pt>> {
        let mut no_identical_points = Vec::with_capacity(input.len());
        let mut start_index: i64 = -1;
        let compare_point = input.first().unwrap_or_else(|| {
            super::unchecked_panic(LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                "Index 0 out of bounds for length 0",
            ))
        });
        let size = input.len() as i64;
        let mut i: i64 = 0;
        while i < size - 1 {
            let the_point = &input[Self::get_legal_index(-i - 1, size) as usize];
            if !the_point.is_numerically_identical(compare_point) {
                start_index = Self::get_legal_index(-i, size);
                break;
            }
            i += 1;
        }
        if start_index == -1 {
            return None;
        }
        let mut current_index = start_index;
        loop {
            let current_point = &input[current_index as usize];
            no_identical_points.push(current_point.clone());
            loop {
                current_index = Self::get_legal_index(current_index + 1, size);
                if current_index == start_index {
                    break;
                }
                let next_non_identical_point = &input[current_index as usize];
                if !next_non_identical_point.is_numerically_identical(current_point) {
                    break;
                }
            }
            if current_index == start_index {
                break;
            }
        }
        if no_identical_points.len() < 3 {
            return None;
        }
        Some(no_identical_points)
    }

    /// `filterEdges(List<GeoPoint>, double)`: `None` when the polygon is
    /// degenerate.
    fn filter_edges(no_identical_points: &[Pt], leniency_value: f64) -> Result<Option<Vec<Pt>>> {
        for i in 0..no_identical_points.len() {
            if let Some(result_path) = Self::find_safe_path(no_identical_points, i as i64, leniency_value)? {
                if result_path.previous.is_some() {
                    let mut rval = Vec::with_capacity(no_identical_points.len());
                    result_path.fill_in_list(&mut rval);
                    return Ok(Some(rval));
                }
            }
        }
        Ok(None)
    }

    fn find_safe_path(points: &[Pt], start_index: i64, _leniency_value: f64) -> Result<Option<Box<SafePath>>> {
        let size = points.len() as i64;
        let mut safe_path: Option<Box<SafePath>> = None;
        let mut i = start_index;
        while i < start_index + size {
            let start_point_index = Self::get_legal_index(i - 1, size);
            let start_point = &points[start_point_index as usize];
            let mut end_point_index = Self::get_legal_index(i, size);
            let mut end_point = &points[end_point_index as usize];
            if start_point.is_numerically_identical(end_point) {
                i += 1;
                continue;
            }
            loop {
                let next_point_index = Self::get_legal_index(end_point_index + 1, size);
                let next_point = &points[next_point_index as usize];
                if start_point.is_numerically_identical(next_point) {
                    return Ok(None);
                }
                if !Plane::are_points_coplanar(start_point, end_point, next_point)? {
                    break;
                }
                if end_point_index == start_index {
                    return Ok(None);
                }
                end_point_index = next_point_index;
                end_point = next_point;
                i += 1;
            }
            if safe_path.is_some() && end_point_index == start_index {
                break;
            }
            let current_plane = Plane::new_cross(start_point, end_point)?;
            safe_path = Some(Box::new(SafePath::new(safe_path, end_point.clone(), end_point_index, current_plane)));
            i += 1;
        }
        Ok(safe_path)
    }

    fn pick_pole(generator: &mut JavaRandom, planet_model: &PlanetModel, points: &[Pt]) -> Result<GeoPoint> {
        // `generator.nextInt(points.size())`: Java's `Random.nextInt(int)`
        // throws `IllegalArgumentException` for a bound that is not positive.
        let bound = points.len() as i32;
        if bound <= 0 {
            return Err(iae("bound must be positive"));
        }
        let point_index = generator.next_int_bound(bound);
        let close_point = &points[point_index as usize];
        let angle = generator.next_double() * PI * 2.0 - PI;
        let mut max_arc_distance = points[0].arc_distance(&points[1]);
        let trial_arc_distance = points[0].arc_distance(&points[2]);
        if trial_arc_distance > max_arc_distance {
            max_arc_distance = trial_arc_distance;
        }
        let arc_distance = max_arc_distance - generator.next_double() * max_arc_distance;
        let x = arc_distance.cos();
        let sin_arc_distance = arc_distance.sin();
        let y = angle.cos() * sin_arc_distance;
        let z = angle.sin() * sin_arc_distance;
        let sin_latitude = close_point.get_latitude().sin();
        let cos_latitude = close_point.get_latitude().cos();
        let sin_longitude = close_point.get_longitude().sin();
        let cos_longitude = close_point.get_longitude().cos();
        let x1 = x * cos_latitude - z * sin_latitude;
        let y1 = y;
        let z1 = x * sin_latitude + z * cos_latitude;
        let x2 = x1 * cos_longitude - y1 * sin_longitude;
        let y2 = x1 * sin_longitude + y1 * cos_longitude;
        let z2 = z1;
        Ok(planet_model.create_surface_point_xyz(x2, y2, z2))
    }

    /// `isInsidePolygon(GeoPoint, List<GeoPoint>)`: `None` when undetermined.
    fn is_inside_polygon(point: &GeoPoint, poly_points: &[Pt]) -> Option<bool> {
        let latitude = point.get_latitude();
        let longitude = point.get_longitude();
        let sin_latitude = latitude.sin();
        let cos_latitude = latitude.cos();
        let sin_longitude = longitude.sin();
        let cos_longitude = longitude.cos();
        let mut arc_distance = 0.0;
        let mut prev_angle: Option<f64> = None;
        for poly_point in poly_points {
            let angle = Self::compute_angle(poly_point, sin_latitude, cos_latitude, sin_longitude, cos_longitude)?;
            if let Some(prev) = prev_angle {
                let mut angle_delta = angle - prev;
                if angle_delta < -PI {
                    angle_delta += PI * 2.0;
                }
                if angle_delta > PI {
                    angle_delta -= PI * 2.0;
                }
                if (angle_delta - PI).abs() < Vector::MINIMUM_ANGULAR_RESOLUTION {
                    return None;
                }
                arc_distance += angle_delta;
            }
            prev_angle = Some(angle);
        }
        if let Some(prev) = prev_angle {
            let last_angle = Self::compute_angle(&poly_points[0], sin_latitude, cos_latitude, sin_longitude, cos_longitude)?;
            let mut angle_delta = last_angle - prev;
            if angle_delta < -PI {
                angle_delta += PI * 2.0;
            }
            if angle_delta > PI {
                angle_delta -= PI * 2.0;
            }
            if (angle_delta - PI).abs() < Vector::MINIMUM_ANGULAR_RESOLUTION {
                return None;
            }
            arc_distance += angle_delta;
        }
        if f64::abs(arc_distance) < Vector::MINIMUM_ANGULAR_RESOLUTION {
            return None;
        }
        Some(arc_distance > 0.0)
    }

    fn compute_angle(point: &GeoPoint, sin_latitude: f64, cos_latitude: f64, sin_longitude: f64, cos_longitude: f64) -> Option<f64> {
        let x1 = point.x * cos_longitude + point.y * sin_longitude;
        let y1 = -point.x * sin_longitude + point.y * cos_longitude;
        let z1 = point.z;
        let y2 = y1;
        let z2 = -x1 * sin_latitude + z1 * cos_latitude;
        if (y2 * y2 + z2 * z2).sqrt() < Vector::MINIMUM_RESOLUTION {
            return None;
        }
        Some(z2.atan2(y2))
    }

    /// `buildPolygonShape(...)`.
    #[allow(clippy::too_many_arguments)]
    fn build_polygon_shape(
        rval: &mut GeoCompositePolygon,
        seen_concave: &mut MutableBoolean,
        planet_model: &Arc<PlanetModel>,
        points_list: &[Pt],
        internal_edges: &JavaBitSet,
        start_point_index: usize,
        end_point_index: usize,
        starting_edge: SidedPlane,
        holes: &Holes,
        test_point: Option<&Pt>,
    ) -> FResult<bool> {
        let mut edge_buffer = EdgeBuffer::new(points_list, internal_edges, start_point_index, end_point_index, starting_edge)?;
        let mut stopping_point = edge_buffer.pick_one();
        let mut current_edge = stopping_point;
        while let Some(ce) = current_edge {
            let Some(found_it) = Self::find_convex_polygon(planet_model, ce, rval, &mut edge_buffer, holes, test_point)? else {
                return Ok(false);
            };
            if found_it {
                stopping_point = edge_buffer.pick_one();
                current_edge = stopping_point;
                continue;
            }
            current_edge = edge_buffer.get_next(ce);
            if current_edge == stopping_point {
                break;
            }
        }
        let mut found_bad_edge = false;
        let check_edges: Vec<usize> = edge_buffer.iterator().collect();
        for check_edge in check_edges {
            let ce = edge_buffer.edge(check_edge).clone();
            let flipped_plane = SidedPlane::new_inverse(&ce.plane);
            let confirm_edges: Vec<usize> = edge_buffer.iterator().collect();
            for confirm_edge in confirm_edges {
                if confirm_edge == check_edge {
                    continue;
                }
                let cf = edge_buffer.edge(confirm_edge);
                let the_point: Option<Pt> = if !Arc::ptr_eq(&ce.start_point, &cf.start_point)
                    && !Arc::ptr_eq(&ce.end_point, &cf.start_point)
                    && !flipped_plane.is_within(&cf.start_point)
                {
                    Some(cf.start_point.clone())
                } else if !Arc::ptr_eq(&ce.start_point, &cf.end_point)
                    && !Arc::ptr_eq(&ce.end_point, &cf.end_point)
                    && !flipped_plane.is_within(&cf.end_point)
                {
                    Some(cf.end_point.clone())
                } else {
                    None
                };
                let Some(the_point) = the_point else {
                    continue;
                };
                found_bad_edge = true;
                if Plane::are_points_coplanar(&ce.start_point, &ce.end_point, &the_point)? {
                    continue;
                }
                let third_part_points = vec![ce.start_point.clone(), ce.end_point.clone(), the_point.clone()];
                let mut third_part_internal = JavaBitSet::new();
                third_part_internal.set_value(0, ce.is_internal);
                third_part_internal.set_value(1, true);
                debug_assert!(
                    ce.plane.is_within(&the_point),
                    "Point was on wrong side of complementary plane, so must be on the right side of the non-complementary plane!"
                );
                let convex_part = GeoConvexPolygon::new_with_holes_and_flags(
                    planet_model,
                    to_points(&third_part_points),
                    holes.clone(),
                    third_part_internal,
                    true,
                )?;
                rval.add_shape(Arc::new(convex_part))?;
                let mut loop_edge = edge_buffer.get_previous(check_edge).unwrap_or_else(|| npe());
                let mut first_part_points: Vec<Pt> = Vec::new();
                let mut first_part_internal = JavaBitSet::new();
                let mut i = 0;
                loop {
                    let le = edge_buffer.edge(loop_edge);
                    first_part_points.push(le.end_point.clone());
                    if Arc::ptr_eq(&le.end_point, &the_point) {
                        break;
                    }
                    first_part_internal.set_value(i, le.is_internal);
                    i += 1;
                    loop_edge = edge_buffer.get_previous(loop_edge).unwrap_or_else(|| npe());
                }
                first_part_internal.set_value(i, true);
                let first_len = first_part_points.len();
                if !Self::build_polygon_shape(
                    rval,
                    seen_concave,
                    planet_model,
                    &first_part_points,
                    &first_part_internal,
                    first_len - 1,
                    0,
                    SidedPlane::new_cross_on_side(&ce.end_point, false, &ce.start_point, &the_point)?,
                    holes,
                    test_point,
                )? {
                    return Ok(false);
                }
                let mut second_part_points: Vec<Pt> = Vec::new();
                let mut second_part_internal = JavaBitSet::new();
                loop_edge = edge_buffer.get_next(check_edge).unwrap_or_else(|| npe());
                i = 0;
                loop {
                    let le = edge_buffer.edge(loop_edge);
                    second_part_points.push(le.start_point.clone());
                    if Arc::ptr_eq(&le.start_point, &the_point) {
                        break;
                    }
                    second_part_internal.set_value(i, le.is_internal);
                    i += 1;
                    loop_edge = edge_buffer.get_next(loop_edge).unwrap_or_else(|| npe());
                }
                second_part_internal.set_value(i, true);
                let second_len = second_part_points.len();
                if !Self::build_polygon_shape(
                    rval,
                    seen_concave,
                    planet_model,
                    &second_part_points,
                    &second_part_internal,
                    second_len - 1,
                    0,
                    SidedPlane::new_cross_on_side(&ce.start_point, false, &ce.end_point, &the_point)?,
                    holes,
                    test_point,
                )? {
                    return Ok(false);
                }
                return Ok(true);
            }
        }
        if found_bad_edge {
            return Err(TileException::new("Could not tile polygon; found a pathological coplanarity that couldn't be addressed").into());
        }
        if !Self::make_concave_polygon(planet_model, rval, seen_concave, &edge_buffer, holes, test_point)? {
            return Ok(false);
        }
        Ok(true)
    }

    fn make_concave_polygon(
        planet_model: &Arc<PlanetModel>,
        rval: &mut GeoCompositePolygon,
        seen_concave: &mut MutableBoolean,
        edge_buffer: &EdgeBuffer,
        holes: &Holes,
        test_point: Option<&Pt>,
    ) -> FResult<bool> {
        if edge_buffer.size() == 0 {
            return Ok(true);
        }
        if seen_concave.value {
            return Err(iae("Illegal polygon; polygon edges intersect each other").into());
        }
        seen_concave.value = true;
        if edge_buffer.size() < 3 {
            return Err(iae("Illegal polygon; polygon edges intersect each other").into());
        }
        let mut points: Vec<GeoPoint> = Vec::with_capacity(edge_buffer.size());
        let mut internal_edges = JavaBitSet::new();
        let mut edge = edge_buffer.pick_one().unwrap_or_else(|| npe());
        let mut is_internal = false;
        for i in 0..edge_buffer.size() {
            let e = edge_buffer.edge(edge);
            points.push((*e.start_point).clone());
            if i < edge_buffer.size() - 1 {
                internal_edges.set_value(i, e.is_internal);
            } else {
                is_internal = e.is_internal;
            }
            edge = edge_buffer.get_next(edge).unwrap_or_else(|| npe());
        }
        let mut attempt = || -> Result<bool> {
            if let (Some(tp), true) = (test_point, holes_size(holes) > 0) {
                let test_polygon = GeoConcavePolygon::new_with_holes_and_flags(planet_model, points.clone(), None, internal_edges.clone(), is_internal)?;
                if test_polygon.is_within(tp) {
                    return Ok(false);
                }
            }
            let real_polygon = GeoConcavePolygon::new_with_holes_and_flags(planet_model, points.clone(), holes.clone(), internal_edges.clone(), is_internal)?;
            if let (Some(tp), true) = (test_point, holes_size(holes) == 0) {
                if real_polygon.is_within(tp) {
                    return Ok(false);
                }
            }
            rval.add_shape(Arc::new(real_polygon))?;
            Ok(true)
        };
        match attempt() {
            Ok(b) => Ok(b),
            Err(e) => match iae_message(&e) {
                Some(m) => Err(TileException::new(m).into()),
                None => Err(e.into()),
            },
        }
    }

    fn find_convex_polygon(
        planet_model: &Arc<PlanetModel>,
        current_edge: usize,
        rval: &mut GeoCompositePolygon,
        edge_buffer: &mut EdgeBuffer,
        holes: &Holes,
        test_point: Option<&Pt>,
    ) -> FResult<Option<bool>> {
        let mut included_edges: HashSet<usize> = HashSet::new();
        included_edges.insert(current_edge);
        let mut first_edge = current_edge;
        let mut last_edge = current_edge;
        let parallel = "Two adjacent edge planes are effectively parallel despite filtering; give up on tiling";
        loop {
            let fe = edge_buffer.edge(first_edge).clone();
            let le = edge_buffer.edge(last_edge).clone();
            if Arc::ptr_eq(&fe.start_point, &le.end_point) {
                break;
            }
            let new_last_edge = edge_buffer.get_next(last_edge).unwrap_or_else(|| npe());
            let nle = edge_buffer.edge(new_last_edge).clone();
            if Plane::are_points_coplanar(&le.start_point, &le.end_point, &nle.end_point)? {
                break;
            }
            if le.plane.is_functionally_identical(&nle.plane) {
                return Err(TileException::new(parallel).into());
            }
            if Self::is_within_set(&nle.end_point, &included_edges, edge_buffer) {
                let return_boundary = if !Arc::ptr_eq(&fe.start_point, &nle.end_point) {
                    if Plane::are_points_coplanar(&fe.end_point, &fe.start_point, &nle.end_point)?
                        || Plane::are_points_coplanar(&fe.start_point, &nle.end_point, &nle.start_point)?
                    {
                        break;
                    }
                    Some(SidedPlane::new_cross(&fe.end_point, &fe.start_point, &nle.end_point)?)
                } else {
                    None
                };
                let mut found_point_inside = false;
                for edge in edge_buffer.iterator() {
                    if !included_edges.contains(&edge) && edge != new_last_edge {
                        let e = edge_buffer.edge(edge);
                        if !Arc::ptr_eq(&e.start_point, &nle.end_point)
                            && Self::is_within_ext(&e.start_point, &included_edges, edge_buffer, new_last_edge, return_boundary.as_ref())
                        {
                            found_point_inside = true;
                            break;
                        }
                        if !Arc::ptr_eq(&e.end_point, &fe.start_point)
                            && Self::is_within_ext(&e.end_point, &included_edges, edge_buffer, new_last_edge, return_boundary.as_ref())
                        {
                            found_point_inside = true;
                            break;
                        }
                    }
                }
                if !found_point_inside {
                    included_edges.insert(new_last_edge);
                    last_edge = new_last_edge;
                    continue;
                }
            }
            break;
        }
        loop {
            let fe = edge_buffer.edge(first_edge).clone();
            let le = edge_buffer.edge(last_edge).clone();
            if Arc::ptr_eq(&fe.start_point, &le.end_point) {
                break;
            }
            let new_first_edge = edge_buffer.get_previous(first_edge).unwrap_or_else(|| npe());
            let nfe = edge_buffer.edge(new_first_edge).clone();
            if Plane::are_points_coplanar(&nfe.start_point, &nfe.end_point, &fe.end_point)? {
                break;
            }
            if fe.plane.is_functionally_identical(&nfe.plane) {
                return Err(TileException::new(parallel).into());
            }
            if Self::is_within_set(&nfe.start_point, &included_edges, edge_buffer) {
                let return_boundary = if !Arc::ptr_eq(&nfe.start_point, &le.end_point) {
                    if Plane::are_points_coplanar(&le.start_point, &le.end_point, &nfe.start_point)?
                        || Plane::are_points_coplanar(&le.end_point, &nfe.start_point, &nfe.end_point)?
                    {
                        break;
                    }
                    Some(SidedPlane::new_cross(&le.start_point, &le.end_point, &nfe.start_point)?)
                } else {
                    None
                };
                let mut found_point_inside = false;
                for edge in edge_buffer.iterator() {
                    if !included_edges.contains(&edge) && edge != new_first_edge {
                        let e = edge_buffer.edge(edge);
                        if !Arc::ptr_eq(&e.start_point, &le.end_point)
                            && Self::is_within_ext(&e.start_point, &included_edges, edge_buffer, new_first_edge, return_boundary.as_ref())
                        {
                            found_point_inside = true;
                            break;
                        }
                        if !Arc::ptr_eq(&e.end_point, &nfe.start_point)
                            && Self::is_within_ext(&e.end_point, &included_edges, edge_buffer, new_first_edge, return_boundary.as_ref())
                        {
                            found_point_inside = true;
                            break;
                        }
                    }
                }
                if !found_point_inside {
                    included_edges.insert(new_first_edge);
                    first_edge = new_first_edge;
                    continue;
                }
            }
            break;
        }
        if included_edges.len() < 2 {
            return Ok(Some(false));
        }
        let mut points: Vec<GeoPoint> = Vec::with_capacity(included_edges.len() + 1);
        let mut internal_edges = JavaBitSet::new();
        let return_is_internal;
        let fe = edge_buffer.edge(first_edge).clone();
        let le = edge_buffer.edge(last_edge).clone();
        if Arc::ptr_eq(&fe.start_point, &le.end_point) {
            if included_edges.len() < 3 {
                return Ok(Some(false));
            }
            if fe.plane.is_functionally_identical(&le.plane) {
                return Err(TileException::new(parallel).into());
            }
            let mut edge = first_edge;
            points.push((*edge_buffer.edge(edge).start_point).clone());
            let mut k = 0;
            loop {
                if edge == last_edge {
                    break;
                }
                let e = edge_buffer.edge(edge);
                points.push((*e.end_point).clone());
                internal_edges.set_value(k, e.is_internal);
                k += 1;
                edge = edge_buffer.get_next(edge).unwrap_or_else(|| npe());
            }
            return_is_internal = le.is_internal;
            edge_buffer.clear();
        } else {
            let return_sided_plane = SidedPlane::new_cross_on_side(&fe.end_point, false, &fe.start_point, &le.end_point)?;
            let return_edge = Edge::new(fe.start_point.clone(), le.end_point.clone(), return_sided_plane, true);
            if return_edge.plane.is_functionally_identical(&le.plane) || return_edge.plane.is_functionally_identical(&fe.plane) {
                return Err(TileException::new(parallel).into());
            }
            let mut edges: Vec<usize> = Vec::with_capacity(included_edges.len());
            return_is_internal = true;
            let mut edge = first_edge;
            points.push((*edge_buffer.edge(edge).start_point).clone());
            let mut k = 0;
            loop {
                let e = edge_buffer.edge(edge);
                points.push((*e.end_point).clone());
                internal_edges.set_value(k, e.is_internal);
                k += 1;
                edges.push(edge);
                if edge == last_edge {
                    break;
                }
                edge = edge_buffer.get_next(edge).unwrap_or_else(|| npe());
            }
            edge_buffer.replace(&edges, return_edge);
        }
        let mut attempt = || -> Result<Option<bool>> {
            if let (Some(tp), true) = (test_point, holes_size(holes) > 0) {
                let test_polygon = GeoConvexPolygon::new_with_holes_and_flags(planet_model, points.clone(), None, internal_edges.clone(), return_is_internal)?;
                if test_polygon.is_within(tp) {
                    return Ok(None);
                }
            }
            let real_polygon = GeoConvexPolygon::new_with_holes_and_flags(planet_model, points.clone(), holes.clone(), internal_edges.clone(), return_is_internal)?;
            if let (Some(tp), true) = (test_point, holes_size(holes) == 0) {
                if real_polygon.is_within(tp) {
                    return Ok(None);
                }
            }
            rval.add_shape(Arc::new(real_polygon))?;
            Ok(Some(true))
        };
        match attempt() {
            Ok(b) => Ok(b),
            Err(e) => match iae_message(&e) {
                Some(m) => Err(TileException::new(m).into()),
                None => Err(e.into()),
            },
        }
    }

    /// `isWithin(GeoPoint, Set<Edge>, Edge, SidedPlane)`.
    fn is_within_ext(
        point: &GeoPoint,
        edge_set: &HashSet<usize>,
        edge_buffer: &EdgeBuffer,
        extension: usize,
        return_boundary: Option<&SidedPlane>,
    ) -> bool {
        if !edge_buffer.edge(extension).plane.is_within(point) {
            return false;
        }
        if let Some(rb) = return_boundary {
            if !rb.is_within(point) {
                return false;
            }
        }
        Self::is_within_set(point, edge_set, edge_buffer)
    }

    /// `isWithin(GeoPoint, Set<Edge>)`.
    fn is_within_set(point: &GeoPoint, edge_set: &HashSet<usize>, edge_buffer: &EdgeBuffer) -> bool {
        for &edge in edge_set {
            if !edge_buffer.edge(edge).plane.is_within(point) {
                return false;
            }
        }
        true
    }

    fn get_legal_index(mut index: i64, size: i64) -> i64 {
        while index < 0 {
            index += size;
        }
        while index >= size {
            index -= size;
        }
        index
    }
}

/// Class for tracking the best shape for finding a pole, and whether or not
/// the pole must be inside or outside of the shape.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.BestShape`.
pub(crate) struct BestShape {
    points: Vec<Pt>,
    pole_must_be_inside: bool,
}

impl BestShape {
    fn new(points: Vec<Pt>, pole_must_be_inside: bool) -> Self {
        Self { points, pole_must_be_inside }
    }

    /// `createGeoComplexPolygon(PlanetModel, List<List<GeoPoint>>, GeoPoint)`:
    /// `None` when the test point is not usable.
    fn create_geo_complex_polygon(
        &self,
        planet_model: &Arc<PlanetModel>,
        points_list: &[Vec<GeoPoint>],
        test_point: &GeoPoint,
    ) -> Result<Option<GeoComplexPolygon>> {
        let Some(is_test_point_inside) = GeoPolygonFactory::is_inside_polygon(test_point, &self.points) else {
            return Ok(None);
        };
        let r = if is_test_point_inside == self.pole_must_be_inside {
            GeoComplexPolygon::new(planet_model, points_list.to_vec(), test_point.clone(), is_test_point_inside)
        } else {
            GeoComplexPolygon::new(
                planet_model,
                points_list.to_vec(),
                GeoPoint::new(-test_point.x, -test_point.y, -test_point.z),
                !is_test_point_inside,
            )
        };
        match r {
            Ok(p) => Ok(Some(p)),
            Err(e) if e.is_illegal_argument_exception() => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl fmt::Display for BestShape {
    /// `Object.toString()`: the class name and the identity hash code, whose
    /// stand-in is the project's stable-name hash.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "org.apache.lucene.spatial3d.geom.GeoPolygonFactory$BestShape@{:x}",
            rucene::util::java_collections::java_identity_hash_code_stand_in("org.apache.lucene.spatial3d.geom.GeoPolygonFactory$BestShape") as u32
        )
    }
}

/// An edge of the polygon being tiled; compared by identity.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.Edge`.
#[derive(Clone)]
pub(crate) struct Edge {
    start_point: Pt,
    end_point: Pt,
    plane: SidedPlane,
    is_internal: bool,
}

impl Edge {
    fn new(start_point: Pt, end_point: Pt, plane: SidedPlane, is_internal: bool) -> Self {
        Self {
            start_point,
            end_point,
            plane,
            is_internal,
        }
    }
}

/// Iterator execution interface, for tree traversal, over the edges of an
/// [`EdgeBuffer`] in ring order.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.EdgeBufferIterator`.
pub(crate) struct EdgeBufferIterator<'a> {
    edge_buffer: &'a EdgeBuffer,
    first_edge: Option<usize>,
    current_edge: Option<usize>,
}

impl<'a> EdgeBufferIterator<'a> {
    fn new(edge_buffer: &'a EdgeBuffer) -> Self {
        let current_edge = edge_buffer.pick_one();
        Self {
            edge_buffer,
            first_edge: current_edge,
            current_edge,
        }
    }

    /// `remove()`: unsupported.
    ///
    /// # Errors
    ///
    /// Always `RuntimeException("Unsupported operation")`.
    #[allow(dead_code)]
    fn remove(&mut self) -> Result<()> {
        Err(rte("Unsupported operation"))
    }
}

impl Iterator for EdgeBufferIterator<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let rval = self.current_edge;
        if let Some(ce) = self.current_edge {
            self.current_edge = self.edge_buffer.get_next(ce);
            if self.current_edge == self.first_edge {
                self.current_edge = None;
            }
        }
        rval
    }
}

/// Class representing a pool of edges, linked in a ring.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.EdgeBuffer`.
/// Edges live in an arena and are linked by index (Java's identity maps).
pub(crate) struct EdgeBuffer {
    arena: Vec<Edge>,
    one_edge: Option<usize>,
    edges: HashSet<usize>,
    previous_edges: HashMap<usize, usize>,
    next_edges: HashMap<usize, usize>,
}

impl EdgeBuffer {
    fn new(
        point_list: &[Pt],
        internal_edges: &JavaBitSet,
        start_plane_start_index: usize,
        start_plane_end_index: usize,
        start_plane: SidedPlane,
    ) -> Result<Self> {
        let mut s = Self {
            arena: Vec::new(),
            one_edge: None,
            edges: HashSet::new(),
            previous_edges: HashMap::new(),
            next_edges: HashMap::new(),
        };
        let start_edge = s.add(Edge::new(
            point_list[start_plane_start_index].clone(),
            point_list[start_plane_end_index].clone(),
            start_plane,
            internal_edges.get(start_plane_start_index),
        ));
        let mut current_edge = start_edge;
        let mut start_index;
        let mut end_index = start_plane_end_index;
        loop {
            if Arc::ptr_eq(&s.arena[current_edge].end_point, &s.arena[start_edge].start_point) {
                s.previous_edges.insert(start_edge, current_edge);
                s.next_edges.insert(current_edge, start_edge);
                s.edges.insert(start_edge);
                break;
            }
            start_index = end_index;
            end_index += 1;
            if end_index >= point_list.len() {
                end_index -= point_list.len();
            }
            let new_point = &point_list[end_index];
            let is_new_point_within = s.arena[current_edge].plane.is_within(new_point);
            let point_to_present = s.arena[current_edge].start_point.clone();
            let new_plane = SidedPlane::new_cross_on_side(&point_to_present, is_new_point_within, &point_list[start_index], new_point)?;
            let new_edge = s.add(Edge::new(
                point_list[start_index].clone(),
                point_list[end_index].clone(),
                new_plane,
                internal_edges.get(start_index),
            ));
            s.previous_edges.insert(new_edge, current_edge);
            s.next_edges.insert(current_edge, new_edge);
            s.edges.insert(new_edge);
            current_edge = new_edge;
        }
        s.one_edge = Some(start_edge);
        Ok(s)
    }

    fn add(&mut self, edge: Edge) -> usize {
        self.arena.push(edge);
        self.arena.len() - 1
    }

    fn edge(&self, index: usize) -> &Edge {
        &self.arena[index]
    }

    fn get_previous(&self, current_edge: usize) -> Option<usize> {
        self.previous_edges.get(&current_edge).copied()
    }

    fn get_next(&self, current_edge: usize) -> Option<usize> {
        self.next_edges.get(&current_edge).copied()
    }

    fn replace(&mut self, remove_list: &[usize], new_edge: Edge) {
        let new_edge = self.add(new_edge);
        let previous = self.previous_edges.get(&remove_list[0]).copied().unwrap_or_else(|| npe());
        let next = self.next_edges.get(&remove_list[remove_list.len() - 1]).copied().unwrap_or_else(|| npe());
        self.edges.insert(new_edge);
        self.previous_edges.insert(new_edge, previous);
        self.next_edges.insert(previous, new_edge);
        self.previous_edges.insert(next, new_edge);
        self.next_edges.insert(new_edge, next);
        for &edge in remove_list {
            if Some(edge) == self.one_edge {
                self.one_edge = Some(new_edge);
            }
            self.edges.remove(&edge);
            self.previous_edges.remove(&edge);
            self.next_edges.remove(&edge);
        }
    }

    fn clear(&mut self) {
        self.edges.clear();
        self.previous_edges.clear();
        self.next_edges.clear();
        self.one_edge = None;
    }

    fn size(&self) -> usize {
        self.edges.len()
    }

    fn iterator(&self) -> EdgeBufferIterator<'_> {
        EdgeBufferIterator::new(self)
    }

    fn pick_one(&self) -> Option<usize> {
        self.one_edge
    }
}

/// An instance of this class represents a known-good path of nodes.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.SafePath`.
pub(crate) struct SafePath {
    last_point: Pt,
    previous: Option<Box<SafePath>>,
}

impl SafePath {
    fn new(previous: Option<Box<SafePath>>, last_point: Pt, _last_point_index: i64, _last_plane: Plane) -> Self {
        Self { last_point, previous }
    }

    /// `fillInList(List<GeoPoint>)`.
    fn fill_in_list(&self, point_list: &mut Vec<Pt>) {
        let mut safe_path = self;
        while let Some(prev) = &safe_path.previous {
            point_list.push(safe_path.last_point.clone());
            safe_path = prev;
        }
        point_list.push(safe_path.last_point.clone());
        point_list.reverse();
    }
}

/// Class used to track a boolean that can be modified.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.MutableBoolean`.
#[derive(Default)]
pub(crate) struct MutableBoolean {
    /// The value.
    pub value: bool,
}

/// Exception we throw when we can't tile a polygon due to numerical precision
/// issues.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPolygonFactory.TileException`.
#[derive(Debug)]
pub(crate) struct TileException {
    message: String,
}

impl TileException {
    fn new(msg: impl Into<String>) -> Self {
        Self { message: msg.into() }
    }
}

impl fmt::Display for TileException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "org.apache.lucene.spatial3d.geom.GeoPolygonFactory$TileException: {}", self.message)
    }
}
