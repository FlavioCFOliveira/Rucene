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

//! The types declared in `RandomGeo3dShapeGenerator.java`, of the Java package
//! `org.apache.lucene.spatial3d.tests`.
//!
//! # Exceptions and `null`
//!
//! The Java methods throw only unchecked exceptions; their ports return
//! [`Result`], whose `Err` is the exception Java lets escape. A Java
//! `catch (IllegalArgumentException e) { continue; }` is a match on
//! [`LuceneError::is_illegal_argument_exception`] that continues the loop and
//! propagates every other error, as the JVM does. A Java `null` result is
//! `None`.
//!
//! # Randomness
//!
//! `random()` forks a private `java.util.Random` ([`JdkRandom`]) from one
//! `nextLong()` of the current [`RandomizedContext`] random on every call, and
//! `RandomizedTest.randomDouble()` draws from that context random directly, so
//! the port consumes the context random in exactly Java's sequence.

use rucene::util::java_math;
use std::any::Any;
use std::f64::consts::PI;
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::{RandomizedContext, RandomizedTest};
use rucene::error::{LuceneError, Result};
use rucene::util::java_double_compare;
use rucene_spatial3d::geom::geo_area::{CONTAINS, DISJOINT, OVERLAPS, WITHIN};
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_b_box::GeoBBox;
use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
use rucene_spatial3d::geom::geo_circle::GeoCircle;
use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
use rucene_spatial3d::geom::geo_composite_area_shape::GeoCompositeAreaShape;
use rucene_spatial3d::geom::geo_composite_membership_shape::GeoCompositeMembershipShape;
use rucene_spatial3d::geom::geo_path::GeoPath;
use rucene_spatial3d::geom::geo_path_factory::GeoPathFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::geo_point_shape::GeoPointShape;
use rucene_spatial3d::geom::geo_point_shape_factory::GeoPointShapeFactory;
use rucene_spatial3d::geom::geo_polygon::GeoPolygon;
use rucene_spatial3d::geom::geo_polygon_factory::{GeoPolygonFactory, PolygonDescription};
use rucene_spatial3d::geom::geo_shape::GeoShape;
use rucene_spatial3d::geom::planet_model::{self, PlanetModel};

/// Runs `$body` (a Java `try` block producing `Result`); an
/// `IllegalArgumentException` continues the enclosing loop, any other error
/// is returned from the enclosing function.
macro_rules! try_or_continue {
    ($body:expr) => {
        match $body {
            Ok(value) => value,
            Err(e) if e.is_illegal_argument_exception() => continue,
            Err(e) => return Err(e),
        }
    };
}

/// Class for generating random Geo3dShapes. They can be generated under given
/// constraints which are expressed as a shape and a relationship.
///
/// Note that convexity for polygons is defined as polygons that contains
/// antipodal points, otherwise they are convex. Internally they can be created
/// using GeoConvexPolygons and GeoConcavePolygons.
///
/// Equivalent to `org.apache.lucene.spatial3d.tests.RandomGeo3dShapeGenerator`.
/// Java's class has static methods only (and a private constructor); they are
/// associated functions of this uninhabitable type.
pub enum RandomGeo3dShapeGenerator {}

/// Max num of iterations to find right shape under given constrains.
const MAX_SHAPE_ITERATIONS: i32 = 20;
/// Max num of iterations to find right point under given constrains.
const MAX_POINT_ITERATIONS: i32 = 1000;

impl RandomGeo3dShapeGenerator {
    /// Supported shape `CONVEX_POLYGON`.
    pub const CONVEX_POLYGON: i32 = 0;
    /// Supported shape `CONVEX_POLYGON_WITH_HOLES`.
    pub const CONVEX_POLYGON_WITH_HOLES: i32 = 1;
    /// Supported shape `CONCAVE_POLYGON`.
    pub const CONCAVE_POLYGON: i32 = 2;
    /// Supported shape `CONCAVE_POLYGON_WITH_HOLES`.
    pub const CONCAVE_POLYGON_WITH_HOLES: i32 = 3;
    /// Supported shape `COMPLEX_POLYGON`.
    pub const COMPLEX_POLYGON: i32 = 4;
    /// Supported shape `CIRCLE`.
    pub const CIRCLE: i32 = 5;
    /// Supported shape `RECTANGLE`.
    pub const RECTANGLE: i32 = 6;
    /// Supported shape `PATH`.
    pub const PATH: i32 = 7;
    /// Supported shape `COLLECTION`.
    pub const COLLECTION: i32 = 8;
    /// Supported shape `POINT`.
    pub const POINT: i32 = 9;
    /// Supported shape `LINE`.
    pub const LINE: i32 = 10;
    /// Supported shape `EXACT_CIRCLE`.
    pub const EXACT_CIRCLE: i32 = 11;

    /// Helper shape for generating constraints: a three sided convex polygon.
    pub const CONVEX_SIMPLE_POLYGON: i32 = 500;
    /// Helper shape for generating constraints: a three sided concave polygon.
    pub const CONCAVE_SIMPLE_POLYGON: i32 = 501;

    /// `random()`: a private-use random forked from the current
    /// [`RandomizedContext`].
    fn random() -> JdkRandom {
        JdkRandom::new(RandomizedContext::current().get_random().next_long())
    }

    /// `randomPlanetModel()`: a random planet model among the supported ones,
    /// currently `SPHERE` and `WGS84`.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` (`Unexpected planet model`) on a draw outside
    /// `[0, 2)`, which `nextInt(2)` never returns.
    pub fn random_planet_model() -> Arc<PlanetModel> {
        let shape_type = Self::random().next_int_bound(2);
        match shape_type {
            0 => planet_model::SPHERE.clone(),
            1 => planet_model::WGS84.clone(),
            _ => panic!("java.lang.IllegalStateException: Unexpected planet model"),
        }
    }

    /// `randomShapeType()`: a random shape code from all supported shapes.
    pub fn random_shape_type() -> i32 {
        Self::random().next_int_bound(12)
    }

    /// `randomGeoAreaShapeType()`: a random GeoAreaShape code from all
    /// supported GeoAreaShapes.
    ///
    /// The Java javadoc says collections are removed because it is difficult
    /// to create shapes with properties in some cases; the code draws from all
    /// 12 codes nevertheless, as this port does.
    pub fn random_geo_area_shape_type() -> i32 {
        Self::random().next_int_bound(12)
    }

    /// `randomConvexShapeType()`: a random shape code from all convex
    /// supported shapes.
    pub fn random_convex_shape_type() -> i32 {
        let mut shape_type = Self::random_shape_type();
        while Self::is_concave(shape_type) {
            shape_type = Self::random_shape_type();
        }
        shape_type
    }

    /// `randomConcaveShapeType()`: a random shape code from all concave
    /// supported shapes.
    pub fn random_concave_shape_type() -> i32 {
        let mut shape_type = Self::random_shape_type();
        while !Self::is_concave(shape_type) {
            shape_type = Self::random_shape_type();
        }
        shape_type
    }

    /// `isConcave(int)`: whether the shape code represents a concave shape.
    pub fn is_concave(shape_type: i32) -> bool {
        shape_type == Self::CONCAVE_POLYGON
    }

    /// `getEmptyConstraint()`: an empty [`Constraints`] object.
    pub fn get_empty_constraint() -> Constraints {
        Constraints::new()
    }

    /// `randomGeoPoint(PlanetModel)`: a random GeoPoint.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of `new GeoPoint(PlanetModel, double,
    /// double)`.
    pub fn random_geo_point(planet_model: &Arc<PlanetModel>) -> Result<GeoPoint> {
        let mut point: Option<GeoPoint> = None;
        while point.is_none() {
            point = Self::random_geo_point_with_constraints(
                planet_model,
                &Self::get_empty_constraint(),
            )?;
        }
        Ok(point.expect("the loop exits on a point"))
    }

    /// `randomGeoPoint(PlanetModel, Constraints)`: a random GeoPoint under the
    /// given constraints, `None` (Java `null`) when none is found.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of `new GeoPoint(PlanetModel, double,
    /// double)`.
    pub fn random_geo_point_with_constraints(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<GeoPoint>> {
        let mut iterations = 0;
        while iterations < MAX_POINT_ITERATIONS {
            let mut lat = RandomizedTest::random_double() * PI / 2.0;
            if Self::random().next_boolean() {
                lat = -lat;
            }
            let mut lon = RandomizedTest::random_double() * PI;
            if Self::random().next_boolean() {
                lon = -lon;
            }
            iterations += 1;
            let point = GeoPoint::new_lat_lon(planet_model, lat, lon)?;
            if constraints.is_within(&point) {
                return Ok(Some(point));
            }
        }
        Ok(None)
    }

    /// `randomGeoAreaShape(int, PlanetModel)`: a random GeoAreaShape.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Unexpected shape type`) for an unknown shape
    /// code; the unchecked exceptions the shape construction lets escape.
    pub fn random_geo_area_shape(
        shape_type: i32,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Arc<dyn GeoAreaShape>> {
        let mut geo_area_shape: Option<Arc<dyn GeoAreaShape>> = None;
        while geo_area_shape.is_none() {
            geo_area_shape = Self::random_geo_area_shape_with_constraints(
                shape_type,
                planet_model,
                &Constraints::new(),
            )?;
        }
        Ok(geo_area_shape.expect("the loop exits on a shape"))
    }

    /// `randomGeoAreaShape(int, PlanetModel, Constraints)`: a random
    /// GeoAreaShape under the given constraints, `None` (Java `null`) when it
    /// cannot be built.
    ///
    /// Java casts the `GeoShape` of `randomGeoShape` to `GeoAreaShape`; every
    /// shape that method builds is statically a `GeoAreaShape`, so the cast
    /// cannot fail and the port keeps the static type instead.
    ///
    /// # Errors
    ///
    /// As [`Self::random_geo_area_shape`].
    pub fn random_geo_area_shape_with_constraints(
        shape_type: i32,
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoAreaShape>>> {
        Self::random_shape(shape_type, planet_model, constraints)
    }

    /// `randomGeoShape(int, PlanetModel)`: a random GeoShape.
    ///
    /// # Errors
    ///
    /// As [`Self::random_geo_area_shape`].
    pub fn random_geo_shape(
        shape_type: i32,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Arc<dyn GeoShape>> {
        let mut geo_shape: Option<Arc<dyn GeoShape>> = None;
        while geo_shape.is_none() {
            geo_shape = Self::random_geo_shape_with_constraints(
                shape_type,
                planet_model,
                &Constraints::new(),
            )?;
        }
        Ok(geo_shape.expect("the loop exits on a shape"))
    }

    /// `randomGeoShape(int, PlanetModel, Constraints)`: a random GeoShape
    /// under the given constraints, `None` (Java `null`) when it cannot be
    /// built.
    ///
    /// # Errors
    ///
    /// As [`Self::random_geo_area_shape`].
    pub fn random_geo_shape_with_constraints(
        shape_type: i32,
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoShape>>> {
        Ok(Self::random_shape(shape_type, planet_model, constraints)?
            .map(|shape| -> Arc<dyn GeoShape> { shape }))
    }

    /// The `switch` of `randomGeoShape(int, PlanetModel, Constraints)`, at the
    /// static type every branch shares.
    fn random_shape(
        shape_type: i32,
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoAreaShape>>> {
        Ok(match shape_type {
            Self::CONVEX_POLYGON => {
                Self::convex_polygon(planet_model, constraints)?.map(area_of_polygon)
            }
            Self::CONVEX_POLYGON_WITH_HOLES => {
                Self::convex_polygon_with_holes(planet_model, constraints)?.map(area_of_polygon)
            }
            Self::CONCAVE_POLYGON => {
                Self::concave_polygon(planet_model, constraints)?.map(area_of_polygon)
            }
            Self::CONCAVE_POLYGON_WITH_HOLES => {
                Self::concave_polygon_with_holes(planet_model, constraints)?.map(area_of_polygon)
            }
            Self::COMPLEX_POLYGON => {
                Self::complex_polygon(planet_model, constraints)?.map(area_of_polygon)
            }
            Self::CIRCLE => {
                Self::circle(planet_model, constraints)?.map(|s| -> Arc<dyn GeoAreaShape> { s })
            }
            Self::RECTANGLE => {
                Self::rectangle(planet_model, constraints)?.map(|s| -> Arc<dyn GeoAreaShape> { s })
            }
            Self::PATH => {
                Self::path(planet_model, constraints)?.map(|s| -> Arc<dyn GeoAreaShape> { s })
            }
            Self::COLLECTION => {
                Self::collection(planet_model, constraints)?.map(|s| -> Arc<dyn GeoAreaShape> { s })
            }
            Self::POINT => {
                Self::point(planet_model, constraints)?.map(|s| -> Arc<dyn GeoAreaShape> { s })
            }
            Self::LINE => {
                Self::line(planet_model, constraints)?.map(|s| -> Arc<dyn GeoAreaShape> { s })
            }
            Self::CONVEX_SIMPLE_POLYGON => {
                Self::simple_convex_polygon(planet_model, constraints)?.map(area_of_polygon)
            }
            Self::CONCAVE_SIMPLE_POLYGON => {
                Self::concave_simple_polygon(planet_model, constraints)?.map(area_of_polygon)
            }
            Self::EXACT_CIRCLE => Self::exact_circle(planet_model, constraints)?
                .map(|s| -> Arc<dyn GeoAreaShape> { s }),
            _ => return Err(LuceneError::illegal_state("Unexpected shape type")),
        })
    }

    /// `point(PlanetModel, Constraints)`: a random GeoPointShape under the
    /// given constraints, `None` when it cannot be built.
    fn point(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPointShape>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let Some(point) = Self::random_geo_point_with_constraints(planet_model, constraints)?
            else {
                continue;
            };
            let point_shape = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPointShape>>> {
                let point_shape = GeoPointShapeFactory::make_geo_point_shape(
                    planet_model,
                    point.get_latitude(),
                    point.get_longitude(),
                )?;
                if !constraints.valid(Some(point_shape.as_geo_shape()))? {
                    return Ok(None);
                }
                Ok(Some(point_shape))
            })());
            match point_shape {
                None => continue,
                Some(point_shape) => return Ok(Some(point_shape)),
            }
        }
        Ok(None)
    }

    /// `circle(PlanetModel, Constraints)`: a random GeoCircle under the given
    /// constraints, `None` when it cannot be built.
    fn circle(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoCircle>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let Some(center) = Self::random_geo_point_with_constraints(planet_model, constraints)?
            else {
                continue;
            };
            let radius = Self::random_cutoff_angle();
            let circle = try_or_continue!((|| -> Result<Option<Arc<dyn GeoCircle>>> {
                let circle = GeoCircleFactory::make_geo_circle(
                    planet_model,
                    center.get_latitude(),
                    center.get_longitude(),
                    radius,
                )?;
                if !constraints.valid(Some(circle.as_geo_shape()))? {
                    return Ok(None);
                }
                Ok(Some(circle))
            })());
            match circle {
                None => continue,
                Some(circle) => return Ok(Some(circle)),
            }
        }
        Ok(None)
    }

    /// `exactCircle(PlanetModel, Constraints)`: a random exact GeoCircle under
    /// the given constraints, `None` when it cannot be built.
    fn exact_circle(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoCircle>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let Some(center) = Self::random_geo_point_with_constraints(planet_model, constraints)?
            else {
                continue;
            };
            let radius = Self::random_cutoff_angle();
            let pow = Self::random().next_int_bound(10) + 3;
            let accuracy = Self::random().next_double() * java_math::pow(10.0, f64::from(-pow));
            let circle = try_or_continue!((|| -> Result<Option<Arc<dyn GeoCircle>>> {
                let circle = GeoCircleFactory::make_exact_geo_circle(
                    planet_model,
                    center.get_latitude(),
                    center.get_longitude(),
                    radius,
                    accuracy,
                )?;
                if !constraints.valid(Some(circle.as_geo_shape()))? {
                    return Ok(None);
                }
                Ok(Some(circle))
            })());
            match circle {
                None => continue,
                Some(circle) => return Ok(Some(circle)),
            }
        }
        Ok(None)
    }

    /// `rectangle(PlanetModel, Constraints)`: a random GeoBBox under the given
    /// constraints, `None` when it cannot be built.
    fn rectangle(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoBBox>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let Some(point1) = Self::random_geo_point_with_constraints(planet_model, constraints)?
            else {
                continue;
            };
            let Some(point2) = Self::random_geo_point_with_constraints(planet_model, constraints)?
            else {
                continue;
            };

            let min_lat = java_min(point1.get_latitude(), point2.get_latitude());
            let max_lat = java_max(point1.get_latitude(), point2.get_latitude());
            let min_lon = java_min(point1.get_longitude(), point2.get_longitude());
            let max_lon = java_max(point1.get_longitude(), point2.get_longitude());

            let bbox = try_or_continue!((|| -> Result<Option<Arc<dyn GeoBBox>>> {
                let bbox = GeoBBoxFactory::make_geo_bbox(
                    planet_model,
                    max_lat,
                    min_lat,
                    min_lon,
                    max_lon,
                )?;
                if !constraints.valid(Some(bbox.as_geo_shape()))? {
                    return Ok(None);
                }
                Ok(Some(bbox))
            })());
            match bbox {
                None => continue,
                Some(bbox) => return Ok(Some(bbox)),
            }
        }
        Ok(None)
    }

    /// `line(PlanetModel, Constraints)`: a random degenerate GeoPath under the
    /// given constraints, `None` when it cannot be built.
    fn line(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPath>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let vertex_count = Self::random().next_int_bound(2) + 2;
            let geo_points = Self::points(vertex_count, planet_model, constraints)?;
            if geo_points.len() < 2 {
                continue;
            }
            let path = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPath>>> {
                let path = GeoPathFactory::make_geo_path(planet_model, 0.0, &geo_points)?;
                if !constraints.valid(Some(path.as_geo_shape()))? {
                    return Ok(None);
                }
                Ok(Some(path))
            })());
            match path {
                None => continue,
                Some(path) => return Ok(Some(path)),
            }
        }
        Ok(None)
    }

    /// `path(PlanetModel, Constraints)`: a random GeoPath under the given
    /// constraints, `None` when it cannot be built.
    fn path(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPath>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let vertex_count = Self::random().next_int_bound(2) + 2;
            let geo_points = Self::points(vertex_count, planet_model, constraints)?;
            if geo_points.len() < 2 {
                continue;
            }
            let width = Self::random_cutoff_angle();
            let path = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPath>>> {
                let path = GeoPathFactory::make_geo_path(planet_model, width, &geo_points)?;
                if !constraints.valid(Some(path.as_geo_shape()))? {
                    return Ok(None);
                }
                Ok(Some(path))
            })());
            match path {
                None => continue,
                Some(path) => return Ok(Some(path)),
            }
        }
        Ok(None)
    }

    /// `collection(PlanetModel, Constraints)`: a random
    /// GeoCompositeAreaShape of convex polygons under the given constraints,
    /// `None` when it cannot be built.
    fn collection(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<GeoCompositeAreaShape>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let number_shapes = Self::random().next_int_bound(3) + 2;
            let mut collection = GeoCompositeAreaShape::new(planet_model);
            for _ in 0..number_shapes {
                let member = Self::convex_polygon(planet_model, constraints)?;
                if let Some(member) = member {
                    collection.add_shape(area_of_polygon(member))?;
                }
            }
            if collection.size() == 0 {
                continue;
            }
            return Ok(Some(Arc::new(collection)));
        }
        Ok(None)
    }

    /// `convexPolygon(PlanetModel, Constraints)`: a random convex GeoPolygon
    /// under the given constraints, `None` when it cannot be built.
    fn convex_polygon(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let vertex_count = Self::random().next_int_bound(4) + 3;
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let geo_points = Self::points(vertex_count, planet_model, constraints)?;
            if geo_points.len() < 3 {
                continue;
            }
            let ordered_geo_points = Self::order_points(&geo_points);
            let polygon = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let polygon =
                    GeoPolygonFactory::make_geo_polygon(planet_model, ordered_geo_points.clone())?;
                if !constraints.valid(polygon_shape(&polygon))?
                    || Self::is_concave_polygon(planet_model, polygon_ref(&polygon))
                {
                    return Ok(None);
                }
                Ok(polygon)
            })());
            match polygon {
                None => continue,
                Some(polygon) => return Ok(Some(polygon)),
            }
        }
        Ok(None)
    }

    /// `convexPolygonWithHoles(PlanetModel, Constraints)`: a random convex
    /// GeoPolygon with holes under the given constraints, `None` when it
    /// cannot be built.
    fn convex_polygon_with_holes(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let vertex_count = Self::random().next_int_bound(4) + 3;
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let geo_points = Self::points(vertex_count, planet_model, constraints)?;
            if geo_points.len() < 3 {
                continue;
            }
            let ordered_geo_points = Self::order_points(&geo_points);
            let polygon = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let polygon =
                    GeoPolygonFactory::make_geo_polygon(planet_model, ordered_geo_points.clone())?;
                // polygon should comply with all constraints except disjoint as we have holes
                let mut polygon_constraints = Constraints::new();
                polygon_constraints.put_all(&constraints.get_contains());
                polygon_constraints.put_all(&constraints.get_within());
                polygon_constraints.put_all(&constraints.get_disjoint());
                if !polygon_constraints.valid(polygon_shape(&polygon))?
                    || Self::is_concave_polygon(planet_model, polygon_ref(&polygon))
                {
                    return Ok(None);
                }
                let polygon = polygon.expect("valid(null) is false");
                // hole must overlap with polygon and comply with any CONTAINS constraint.
                let mut hole_constraints = Constraints::new();
                hole_constraints.put_all(&constraints.get_contains());
                hole_constraints.put(area_of_polygon(polygon.clone()), OVERLAPS);
                // Points must be with in the polygon and must comply
                // CONTAINS and DISJOINT constraints
                let mut points_constraints = Constraints::new();
                points_constraints.put(area_of_polygon(polygon.clone()), WITHIN);
                points_constraints.put_all(&constraints.get_contains());
                points_constraints.put_all(&constraints.get_disjoint());
                let holes = Self::concave_polygon_holes(
                    planet_model,
                    &polygon,
                    &hole_constraints,
                    &mut points_constraints,
                )?;
                // we should have at least one hole
                if holes.is_empty() {
                    return Ok(None);
                }
                let polygon = GeoPolygonFactory::make_geo_polygon_with_holes(
                    planet_model,
                    ordered_geo_points.clone(),
                    Some(holes),
                )?;
                if !constraints.valid(polygon_shape(&polygon))?
                    || Self::is_concave_polygon(planet_model, polygon_ref(&polygon))
                {
                    return Ok(None);
                }
                Ok(polygon)
            })());
            match polygon {
                None => continue,
                Some(polygon) => return Ok(Some(polygon)),
            }
        }
        Ok(None)
    }

    /// `concavePolygonHoles(PlanetModel, GeoPolygon, Constraints,
    /// Constraints)`: a random list of concave GeoPolygons, the holes of
    /// `polygon`. Holes and points obey different constraints; each accepted
    /// hole is added to `point_constraints` as `DISJOINT`, as Java mutates the
    /// map it is given.
    fn concave_polygon_holes(
        planet_model: &Arc<PlanetModel>,
        polygon: &Arc<dyn GeoPolygon>,
        hole_constraints: &Constraints,
        point_constraints: &mut Constraints,
    ) -> Result<Vec<Arc<dyn GeoPolygon>>> {
        let mut iterations = 0;
        let holes_count = Self::random().next_int_bound(3) + 1;
        let mut holes: Vec<Arc<dyn GeoPolygon>> = Vec::new();
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let vertex_count = Self::random().next_int_bound(3) + 3;
            let geo_points = Self::points(vertex_count, planet_model, point_constraints)?;
            if geo_points.len() < 3 {
                continue;
            }
            let mut geo_points = Self::order_points(&geo_points);
            let inverse_polygon =
                GeoPolygonFactory::make_geo_polygon(planet_model, geo_points.clone())?;
            // The convex polygon must be within
            match &inverse_polygon {
                None => continue,
                Some(inverse_polygon) => {
                    if polygon.get_relationship(inverse_polygon.as_geo_shape())? != WITHIN {
                        continue;
                    }
                }
            }
            // make it concave
            geo_points.reverse();
            let hole = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let hole = GeoPolygonFactory::make_geo_polygon(planet_model, geo_points.clone())?;
                if !hole_constraints.valid(polygon_shape(&hole))?
                    || Self::is_convex_polygon(planet_model, polygon_ref(&hole))
                {
                    return Ok(None);
                }
                Ok(hole)
            })());
            let Some(hole) = hole else {
                continue;
            };
            holes.push(hole.clone());
            if holes.len() == holes_count as usize {
                return Ok(holes);
            }
            point_constraints.put(area_of_polygon(hole), DISJOINT);
        }
        Ok(holes)
    }

    /// `concavePolygon(PlanetModel, Constraints)`: a random concave GeoPolygon
    /// under the given constraints, `None` when it cannot be built.
    fn concave_polygon(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let vertex_count = Self::random().next_int_bound(4) + 3;
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let geo_points = Self::points(vertex_count, planet_model, constraints)?;
            if geo_points.len() < 3 {
                continue;
            }
            let mut ordered_geo_points = Self::order_points(&geo_points);
            ordered_geo_points.reverse();
            let polygon = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let polygon =
                    GeoPolygonFactory::make_geo_polygon(planet_model, ordered_geo_points.clone())?;
                if !constraints.valid(polygon_shape(&polygon))?
                    || Self::is_convex_polygon(planet_model, polygon_ref(&polygon))
                {
                    return Ok(None);
                }
                Ok(polygon)
            })());
            match polygon {
                None => continue,
                Some(polygon) => return Ok(Some(polygon)),
            }
        }
        Ok(None)
    }

    /// `concavePolygonWithHoles(PlanetModel, Constraints)`: a random concave
    /// GeoPolygon with holes under the given constraints, `None` when it
    /// cannot be built. The final polygon is convex, as the hole wraps the
    /// convex polygon.
    fn concave_polygon_with_holes(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let vertex_count = Self::random().next_int_bound(4) + 3;
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            // we first build the hole. We consider all constraints except
            // disjoint as we have a hole
            let mut hole_constraints = Constraints::new();
            hole_constraints.put_all(&constraints.get_contains());
            hole_constraints.put_all(&constraints.get_within());
            hole_constraints.put_all(&constraints.get_overlaps());
            let Some(hole) = Self::convex_polygon(planet_model, &hole_constraints)? else {
                continue;
            };
            // Now we get points for polygon. Must we within the hole
            // and we add contain constraints
            let mut point_constraints = Constraints::new();
            point_constraints.put(area_of_polygon(hole.clone()), WITHIN);
            point_constraints.put_all(&constraints.get_contains());
            let geo_points = Self::points(vertex_count, planet_model, &point_constraints)?;
            if geo_points.len() < 3 {
                continue;
            }
            let polygon = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let mut ordered_geo_points = Self::order_points(&geo_points);
                // Java passes the unordered points here.
                let inverse_polygon =
                    GeoPolygonFactory::make_geo_polygon(planet_model, geo_points.clone())?;
                // The convex polygon must be within the hole
                match &inverse_polygon {
                    None => return Ok(None),
                    Some(inverse_polygon) => {
                        if hole.get_relationship(inverse_polygon.as_geo_shape())? != WITHIN {
                            return Ok(None);
                        }
                    }
                }
                ordered_geo_points.reverse();
                let polygon = GeoPolygonFactory::make_geo_polygon_with_holes(
                    planet_model,
                    ordered_geo_points,
                    Some(vec![hole.clone()]),
                )?;
                // final polygon must be convex
                if !constraints.valid(polygon_shape(&polygon))?
                    || Self::is_concave_polygon(planet_model, polygon_ref(&polygon))
                {
                    return Ok(None);
                }
                Ok(polygon)
            })());
            match polygon {
                None => continue,
                Some(polygon) => return Ok(Some(polygon)),
            }
        }
        Ok(None)
    }

    /// `complexPolygon(PlanetModel, Constraints)`: a random complex
    /// GeoPolygon under the given constraints, `None` when it cannot be built.
    fn complex_polygon(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let polygons_count = Self::random().next_int_bound(2) + 1;
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let mut pol_description: Vec<PolygonDescription> = Vec::new();
            while (pol_description.len() as i32) < polygons_count {
                let vertex_count = Self::random().next_int_bound(14) + 3;
                let geo_points = Self::points(vertex_count, planet_model, constraints)?;
                if geo_points.len() < 3 {
                    break;
                }
                // Java discards the ordered list: the description keeps the
                // points in drawing order.
                let _ = Self::order_points(&geo_points);
                pol_description.push(PolygonDescription::new(geo_points));
            }
            let polygon = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let polygon =
                    GeoPolygonFactory::make_large_geo_polygon(planet_model, &pol_description)?;
                if !constraints.valid(Some(polygon.as_geo_shape()))? {
                    return Ok(None);
                }
                Ok(Some(polygon))
            })());
            match polygon {
                None => continue,
                Some(polygon) => return Ok(Some(polygon)),
            }
        }
        Ok(None)
    }

    /// `simpleConvexPolygon(PlanetModel, Constraints)`: a random three sided
    /// convex GeoPolygon under the given constraints, `None` when it cannot be
    /// built. This shape is an utility to build constraints.
    fn simple_convex_polygon(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let points = Self::points(3, planet_model, constraints)?;
            if points.len() < 3 {
                continue;
            }
            let points = Self::order_points(&points);
            let polygon = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let polygon =
                    GeoPolygonFactory::make_geo_convex_polygon(planet_model, points.clone())?;
                if !constraints.valid(Some(polygon.as_geo_shape()))?
                    || Self::is_concave_polygon(planet_model, &*polygon)
                {
                    return Ok(None);
                }
                Ok(Some(polygon))
            })());
            match polygon {
                None => continue,
                Some(polygon) => return Ok(Some(polygon)),
            }
        }
        Ok(None)
    }

    /// `concaveSimplePolygon(PlanetModel, Constraints)`: a random three sided
    /// concave GeoPolygon under the given constraints, `None` when it cannot
    /// be built. This shape is an utility to build constraints.
    fn concave_simple_polygon(
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let mut iterations = 0;
        while iterations < MAX_SHAPE_ITERATIONS {
            iterations += 1;
            let points = Self::points(3, planet_model, constraints)?;
            if points.len() < 3 {
                continue;
            }
            let mut points = Self::order_points(&points);
            points.reverse();
            let polygon = try_or_continue!((|| -> Result<Option<Arc<dyn GeoPolygon>>> {
                let polygon =
                    GeoPolygonFactory::make_geo_concave_polygon(planet_model, points.clone())?;
                if !constraints.valid(Some(polygon.as_geo_shape()))?
                    || Self::is_convex_polygon(planet_model, &*polygon)
                {
                    return Ok(None);
                }
                Ok(Some(polygon))
            })());
            match polygon {
                None => continue,
                Some(polygon) => return Ok(Some(polygon)),
            }
        }
        Ok(None)
    }

    /// `points(int, PlanetModel, Constraints)`: a random list of GeoPoints
    /// under the given constraints; it may hold fewer than `count` points.
    fn points(
        count: i32,
        planet_model: &Arc<PlanetModel>,
        constraints: &Constraints,
    ) -> Result<Vec<GeoPoint>> {
        let mut geo_points = Vec::with_capacity(count.max(0) as usize);
        for _ in 0..count {
            if let Some(point) = Self::random_geo_point_with_constraints(planet_model, constraints)?
            {
                geo_points.push(point);
            }
        }
        Ok(geo_points)
    }

    /// `isConcave(PlanetModel, GeoPolygon)`: whether the polygon is pure
    /// concave, i.e. contains antipodal points.
    fn is_concave_polygon(planet_model: &PlanetModel, shape: &dyn GeoPolygon) -> bool {
        (shape.is_within(&planet_model.north_pole) && shape.is_within(&planet_model.south_pole))
            || (shape.is_within(&planet_model.max_x_pole)
                && shape.is_within(&planet_model.min_x_pole))
            || (shape.is_within(&planet_model.max_y_pole)
                && shape.is_within(&planet_model.min_y_pole))
    }

    /// `isConvex(PlanetModel, GeoPolygon)`: whether the polygon is pure
    /// convex, i.e. does not contain antipodal points.
    fn is_convex_polygon(planet_model: &PlanetModel, shape: &dyn GeoPolygon) -> bool {
        !Self::is_concave_polygon(planet_model, shape)
    }

    /// `randomCutoffAngle()`: a random number between 0 and PI.
    fn random_cutoff_angle() -> f64 {
        RandomizedTest::random_double() * PI
    }

    /// `orderPoints(List<GeoPoint>)`: the points ordered anti-clockwise
    /// around their center of mass, to prevent crossing edges.
    ///
    /// Java keys a `HashMap<Double, GeoPoint>` by angle, so a later point with
    /// an equal angle (`Double.equals`) replaces an earlier one, and then
    /// walks the keys sorted by `Double.compareTo` in descending order; the
    /// map's own iteration order is not observable.
    pub fn order_points(points: &[GeoPoint]) -> Vec<GeoPoint> {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut z = 0.0;
        // get center of mass
        for point in points {
            x += point.x;
            y += point.y;
            z += point.z;
        }
        let size = points.len() as i32;
        let mut point_with_angle: Vec<(f64, GeoPoint)> = Vec::new();
        // get angle respect center of mass
        for point in points {
            let center = GeoPoint::new(
                x / f64::from(size),
                y / f64::from(size),
                z / f64::from(size),
            );
            let cs = java_math::sin(center.get_latitude()) * java_math::sin(point.get_latitude())
                + java_math::cos(center.get_latitude())
                    * java_math::cos(point.get_latitude())
                    * java_math::cos(point.get_longitude() - center.get_longitude());
            let pos_ang = java_math::atan2(
                java_math::cos(center.get_latitude())
                    * java_math::cos(point.get_latitude())
                    * java_math::sin(point.get_longitude() - center.get_longitude()),
                java_math::sin(point.get_latitude()) - java_math::sin(center.get_latitude()) * cs,
            );
            // `HashMap.put`: `Double.equals` compares `doubleToLongBits`.
            match point_with_angle
                .iter_mut()
                .find(|(angle, _)| double_to_long_bits(*angle) == double_to_long_bits(pos_ang))
            {
                Some(entry) => entry.1 = point.clone(),
                None => point_with_angle.push((pos_ang, point.clone())),
            }
        }
        // order points
        point_with_angle.sort_by(|a, b| java_double_compare(a.0, b.0));
        point_with_angle.reverse();
        point_with_angle
            .into_iter()
            .map(|(_, point)| point)
            .collect()
    }
}

/// `Double.doubleToLongBits(double)`: the bits with every NaN canonicalized.
fn double_to_long_bits(value: f64) -> u64 {
    if value.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        value.to_bits()
    }
}

/// `Math.min(double, double)`: NaN if either is NaN, `-0.0` below `0.0`.
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return a;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(double, double)`: NaN if either is NaN, `0.0` above `-0.0`.
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// A `GeoPolygon` at its `GeoAreaShape` static type (Java's implicit
/// widening).
fn area_of_polygon(polygon: Arc<dyn GeoPolygon>) -> Arc<dyn GeoAreaShape> {
    polygon
}

/// A possibly-`null` polygon as the `GeoShape` argument of
/// [`Constraints::valid`].
fn polygon_shape(polygon: &Option<Arc<dyn GeoPolygon>>) -> Option<&dyn GeoShape> {
    polygon.as_ref().map(|p| p.as_geo_shape())
}

/// The polygon `isConcave`/`isConvex` receive; reached only after
/// `valid(polygon)` returned `true`, which a `null` polygon never does.
fn polygon_ref(polygon: &Option<Arc<dyn GeoPolygon>>) -> &dyn GeoPolygon {
    &**polygon.as_ref().expect("valid(null) is false")
}

/// `HashMap.hash(Object)`: `(h = key.hashCode()) ^ (h >>> 16)`.
fn spread(h: i32) -> i32 {
    h ^ ((h as u32) >> 16) as i32
}

/// `HashMap.DEFAULT_INITIAL_CAPACITY`.
const DEFAULT_INITIAL_CAPACITY: usize = 16;
/// `HashMap.MAXIMUM_CAPACITY`.
const MAXIMUM_CAPACITY: usize = 1 << 30;
/// `HashMap.DEFAULT_LOAD_FACTOR`.
const DEFAULT_LOAD_FACTOR: f32 = 0.75;
/// `HashMap.TREEIFY_THRESHOLD`.
const TREEIFY_THRESHOLD: usize = 8;
/// `HashMap.MIN_TREEIFY_CAPACITY`.
const MIN_TREEIFY_CAPACITY: usize = 64;

/// One mapping of [`Constraints`] (`HashMap.Node`).
struct Entry {
    hash: i32,
    key: Arc<dyn GeoAreaShape>,
    value: i32,
}

/// Class that holds the constraints that are given to build shapes. It
/// consists in a list of GeoAreaShapes and relationships the new shape needs
/// to satisfy.
///
/// Equivalent to
/// `org.apache.lucene.spatial3d.tests.RandomGeo3dShapeGenerator.Constraints`.
///
/// Java's class `extends HashMap<GeoAreaShape, Integer>`; the port carries
/// that map itself: keys compare by the shapes' `equals`/`hashCode`
/// ([`java_equals`](rucene_spatial3d::geom::serializable_object::SerializableObject::java_equals),
/// [`java_hash_code`](rucene_spatial3d::geom::serializable_object::SerializableObject::java_hash_code)),
/// and iteration follows `java.util.HashMap`'s order — the bucket index
/// `spread(hashCode) & (n - 1)`, insertion-ordered chains, the lazily
/// allocated 16-slot table, `putAll`'s pre-sizing and the order-preserving
/// doubling resize, as `rucene::util::java_collections::JavaHashMap`
/// reproduces them for `String` keys. The order decides which constraint
/// [`Constraints::valid`] evaluates first. As there, a bucket that Java would
/// treeify (more than 8 colliding entries in a table of 64 or more) keeps its
/// insertion-ordered chain.
///
/// The members of `HashMap` the generator and the spatial3d tests use are
/// ported: `put`, `putAll`, `get`, `containsKey`, `keySet`, `size` and
/// `isEmpty`.
pub struct Constraints {
    table: Vec<Vec<Entry>>,
    size: usize,
    threshold: i32,
}

impl Default for Constraints {
    fn default() -> Self {
        Self::new()
    }
}

impl Constraints {
    /// `new Constraints()`: an empty map whose table is allocated on the
    /// first insertion.
    pub fn new() -> Self {
        Self {
            table: Vec::new(),
            size: 0,
            threshold: 0,
        }
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `keySet()`, in iteration order.
    pub fn key_set(&self) -> impl Iterator<Item = &Arc<dyn GeoAreaShape>> {
        self.iter().map(|(key, _)| key)
    }

    /// The mappings (`entrySet()`), in iteration order.
    pub fn iter(&self) -> impl Iterator<Item = (&Arc<dyn GeoAreaShape>, i32)> {
        self.table
            .iter()
            .flat_map(|bucket| bucket.iter().map(|e| (&e.key, e.value)))
    }

    /// `get(Object)`: `None` (Java `null`) when the key is absent.
    pub fn get(&self, key: &dyn GeoAreaShape) -> Option<i32> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(key.java_hash_code());
        let index = index_for(hash, self.table.len());
        self.table[index]
            .iter()
            .find(|e| e.hash == hash && same_key(key, &*e.key))
            .map(|e| e.value)
    }

    /// `containsKey(Object)`.
    pub fn contains_key(&self, key: &dyn GeoAreaShape) -> bool {
        self.get(key).is_some()
    }

    /// `put(GeoAreaShape, Integer)`, returning the previous value as Java
    /// does.
    pub fn put(&mut self, key: Arc<dyn GeoAreaShape>, value: i32) -> Option<i32> {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = spread(key.java_hash_code());
        let n = self.table.len();
        let index = index_for(hash, n);
        let bucket = &mut self.table[index];
        for entry in bucket.iter_mut() {
            if entry.hash == hash && same_key(&*key, &*entry.key) {
                return Some(std::mem::replace(&mut entry.value, value));
            }
        }
        bucket.push(Entry { hash, key, value });
        // `putVal` treeifies a bucket that already held `TREEIFY_THRESHOLD`
        // nodes, which resizes instead while the table is short.
        if bucket.len() > TREEIFY_THRESHOLD && n < MIN_TREEIFY_CAPACITY {
            self.resize();
        }
        self.size += 1;
        if self.size as i32 > self.threshold {
            self.resize();
        }
        None
    }

    /// `putAll(Map)`: pre-sizes the table from `m.size()` (`putMapEntries`),
    /// then puts `m`'s mappings in `m`'s iteration order.
    pub fn put_all(&mut self, m: &Constraints) {
        let s = m.size;
        if s == 0 {
            return;
        }
        if self.table.is_empty() {
            // `Math.ceil(s / (double) loadFactor)` (JDK 19 and later).
            let dt = (s as f64 / f64::from(DEFAULT_LOAD_FACTOR)).ceil();
            let t = if dt < MAXIMUM_CAPACITY as f64 {
                dt as i32
            } else {
                MAXIMUM_CAPACITY as i32
            };
            if t > self.threshold {
                self.threshold = table_size_for(t);
            }
        } else {
            while s as i32 > self.threshold && self.table.len() < MAXIMUM_CAPACITY {
                self.resize();
            }
        }
        for (key, value) in m.iter() {
            self.put(key.clone(), value);
        }
    }

    /// `HashMap.resize()`: allocates or doubles the table, keeping each
    /// bucket's chain order.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let new_cap: usize;
        let mut new_thr: i32 = 0;
        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = i32::MAX;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_INITIAL_CAPACITY {
                new_thr = old_thr << 1;
            }
        } else if old_thr > 0 {
            new_cap = old_thr as usize;
        } else {
            new_cap = DEFAULT_INITIAL_CAPACITY;
            new_thr = (DEFAULT_LOAD_FACTOR * DEFAULT_INITIAL_CAPACITY as f32) as i32;
        }
        if new_thr == 0 {
            let ft = new_cap as f32 * DEFAULT_LOAD_FACTOR;
            new_thr = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 {
                ft as i32
            } else {
                i32::MAX
            };
        }
        self.threshold = new_thr;
        let old_table = std::mem::take(&mut self.table);
        let mut new_table: Vec<Vec<Entry>> = Vec::with_capacity(new_cap);
        new_table.resize_with(new_cap, Vec::new);
        for bucket in old_table {
            for entry in bucket {
                let index = index_for(entry.hash, new_cap);
                new_table[index].push(entry);
            }
        }
        self.table = new_table;
    }

    /// `valid(GeoShape)`: whether the shape satisfies every constraint; a
    /// `None` (Java `null`) shape is never valid.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `getRelationship` when the planet
    /// models differ.
    pub fn valid(&self, shape: Option<&dyn GeoShape>) -> Result<bool> {
        let Some(shape) = shape else {
            return Ok(false);
        };
        for (constraint, relationship) in self.iter() {
            if constraint.get_relationship(shape)? != relationship {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `isWithin(GeoPoint)`: whether the point satisfies every constraint.
    pub fn is_within(&self, point: &GeoPoint) -> bool {
        for (constraint, relationship) in self.iter() {
            if !(Self::valid_point(point, constraint.as_geo_shape(), relationship)) {
                return false;
            }
        }
        true
    }

    /// `validPoint(GeoPoint, GeoShape, int)`: whether the point satisfies one
    /// constraint given by a shape and a relationship.
    fn valid_point(point: &GeoPoint, shape: &dyn GeoShape, relationship: i32) -> bool {
        // For GeoCompositeMembershipShape we only consider the first shape to help
        // converging
        let mut shape = shape;
        if relationship == WITHIN {
            if let Some(composite) = shape.as_any().downcast_ref::<GeoCompositeMembershipShape>() {
                shape = composite.get_shapes()[0].as_geo_shape();
            }
        }
        match relationship {
            DISJOINT => !shape.is_within(point),
            OVERLAPS => true,
            CONTAINS => !shape.is_within(point),
            WITHIN => shape.is_within(point),
            _ => true,
        }
    }

    /// `getContains()`: the CONTAINS constraints.
    pub fn get_contains(&self) -> Constraints {
        self.get_constraints_of_type(CONTAINS)
    }

    /// `getWithin()`: the WITHIN constraints.
    pub fn get_within(&self) -> Constraints {
        self.get_constraints_of_type(WITHIN)
    }

    /// `getOverlaps()`: the OVERLAPS constraints.
    pub fn get_overlaps(&self) -> Constraints {
        self.get_constraints_of_type(OVERLAPS)
    }

    /// `getDisjoint()`: the DISJOINT constraints.
    pub fn get_disjoint(&self) -> Constraints {
        self.get_constraints_of_type(DISJOINT)
    }

    /// `getConstraintsOfType(int)`.
    fn get_constraints_of_type(&self, type_: i32) -> Constraints {
        let mut constraints = Constraints::new();
        for (constraint, relationship) in self.iter() {
            if type_ == relationship {
                constraints.put(constraint.clone(), type_);
            }
        }
        constraints
    }
}

/// The bucket index `hash & (n - 1)`.
fn index_for(hash: i32, n: usize) -> usize {
    (hash as u32 as usize) & (n - 1)
}

/// `HashMap.tableSizeFor(int)`: the smallest power of two `>= cap`.
fn table_size_for(cap: i32) -> i32 {
    let leading = (cap.wrapping_sub(1) as u32).leading_zeros() & 31;
    let n = (u32::MAX >> leading) as i32;
    if n < 0 {
        1
    } else if n as usize >= MAXIMUM_CAPACITY {
        MAXIMUM_CAPACITY as i32
    } else {
        n + 1
    }
}

/// `HashMap`'s key match `(k = p.key) == key || key.equals(k)`, with `key`
/// the probing key and `existing` the stored one.
fn same_key(key: &dyn GeoAreaShape, existing: &dyn GeoAreaShape) -> bool {
    std::ptr::addr_eq(
        key as *const dyn GeoAreaShape,
        existing as *const dyn GeoAreaShape,
    ) || key.java_equals(existing.as_any() as &dyn Any)
}
