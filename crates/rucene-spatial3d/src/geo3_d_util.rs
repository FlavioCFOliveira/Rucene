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

//! The types declared in `Geo3DUtil.java`, of the Java package
//! `org.apache.lucene.spatial3d`.

use std::f64::consts::PI;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::geo::encoding::GeoUtils;
use rucene::geo::geometry::Polygon;

use crate::geom::geo_b_box::GeoBBox;
use crate::geom::geo_b_box_factory::GeoBBoxFactory;
use crate::geom::geo_circle::GeoCircle;
use crate::geom::geo_circle_factory::GeoCircleFactory;
use crate::geom::geo_composite_polygon::GeoCompositePolygon;
use crate::geom::geo_path::GeoPath;
use crate::geom::geo_path_factory::GeoPathFactory;
use crate::geom::geo_point::GeoPoint;
use crate::geom::geo_polygon::GeoPolygon;
use crate::geom::geo_polygon_factory::{GeoPolygonFactory, PolygonDescription};
use crate::geom::planet_model::PlanetModel;

/// Utilities for the spatial3d point field and queries.
///
/// Equivalent to `org.apache.lucene.spatial3d.Geo3DUtil`.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub struct Geo3DUtil;

impl Geo3DUtil {
    /// How many radians are in one degree.
    pub const RADIANS_PER_DEGREE: f64 = PI / 180.0;

    /// `decodeValueFloor(int, PlanetModel)`: the lowest value an encoded value
    /// can stand for.
    pub fn decode_value_floor(x: i32, planet_model: &PlanetModel) -> f64 {
        debug_assert!(x <= planet_model.max_encoded_value && x >= planet_model.min_encoded_value);
        if x == planet_model.min_encoded_value {
            return -planet_model.max_value;
        }
        x as f64 * planet_model.decode
    }

    /// `decodeValueCeil(int, PlanetModel)`: the highest value an encoded value
    /// can stand for.
    pub fn decode_value_ceil(x: i32, planet_model: &PlanetModel) -> f64 {
        debug_assert!(x <= planet_model.max_encoded_value && x >= planet_model.min_encoded_value);
        if x == planet_model.max_encoded_value {
            return planet_model.max_value;
        }
        // Java's `(x + 1)` is `int` arithmetic before the conversion.
        (x.wrapping_add(1) as f64 * planet_model.decode).next_down()
    }

    /// `fromDegrees(double)`.
    pub fn from_degrees(degrees: f64) -> f64 {
        degrees * Self::RADIANS_PER_DEGREE
    }

    /// `fromPolygon(PlanetModel, Polygon...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for no polygon or an illegal one.
    pub(crate) fn from_polygon(
        planet_model: &Arc<PlanetModel>,
        polygons: &[Polygon],
    ) -> Result<Arc<dyn GeoPolygon>> {
        if polygons.is_empty() {
            return Err(LuceneError::illegal_argument("need at least one polygon"));
        }
        let shape: Arc<dyn GeoPolygon> = if polygons.len() == 1 {
            match Self::from_single_polygon(planet_model, &polygons[0])? {
                None => Arc::new(GeoCompositePolygon::new(planet_model)),
                Some(component) => component,
            }
        } else {
            let mut poly = GeoCompositePolygon::new(planet_model);
            for p in polygons {
                if let Some(component) = Self::from_single_polygon(planet_model, p)? {
                    poly.add_shape(component)?;
                }
            }
            Arc::new(poly)
        };
        Ok(shape)
    }

    /// `fromLargePolygon(PlanetModel, Polygon...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for no polygon or an illegal one.
    pub(crate) fn from_large_polygon(
        planet_model: &Arc<PlanetModel>,
        polygons: &[Polygon],
    ) -> Result<Arc<dyn GeoPolygon>> {
        if polygons.is_empty() {
            return Err(LuceneError::illegal_argument("need at least one polygon"));
        }
        GeoPolygonFactory::make_large_geo_polygon(
            planet_model,
            &Self::convert_to_description(planet_model, polygons)?,
        )
    }

    /// `fromPath(PlanetModel, double[], double[], double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for mismatched arrays, invalid coordinates
    /// or an illegal path.
    pub(crate) fn from_path(
        planet_model: &Arc<PlanetModel>,
        path_latitudes: &[f64],
        path_longitudes: &[f64],
        path_width_meters: f64,
    ) -> Result<Arc<dyn GeoPath>> {
        if path_latitudes.len() != path_longitudes.len() {
            return Err(LuceneError::illegal_argument(
                "same number of latitudes and longitudes required",
            ));
        }
        let mut points = Vec::with_capacity(path_latitudes.len());
        for i in 0..path_latitudes.len() {
            let latitude = path_latitudes[i];
            let longitude = path_longitudes[i];
            GeoUtils::check_latitude(latitude)?;
            GeoUtils::check_longitude(longitude)?;
            points.push(GeoPoint::new_lat_lon(
                planet_model,
                Self::from_degrees(latitude),
                Self::from_degrees(longitude),
            )?);
        }
        let radius_radians =
            path_width_meters / (planet_model.get_mean_radius() * planet_model.xy_scaling);
        GeoPathFactory::make_geo_path(planet_model, radius_radians, &points)
    }

    /// `fromDistance(PlanetModel, double, double, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid coordinates or radius.
    pub(crate) fn from_distance(
        planet_model: &Arc<PlanetModel>,
        latitude: f64,
        longitude: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn GeoCircle>> {
        GeoUtils::check_latitude(latitude)?;
        GeoUtils::check_longitude(longitude)?;
        let radius_radians = radius_meters / planet_model.get_mean_radius();
        GeoCircleFactory::make_geo_circle(
            planet_model,
            Self::from_degrees(latitude),
            Self::from_degrees(longitude),
            radius_radians,
        )
    }

    /// `fromBox(PlanetModel, double, double, double, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid coordinates.
    pub(crate) fn from_box(
        planet_model: &Arc<PlanetModel>,
        min_latitude: f64,
        max_latitude: f64,
        min_longitude: f64,
        max_longitude: f64,
    ) -> Result<Arc<dyn GeoBBox>> {
        GeoUtils::check_latitude(min_latitude)?;
        GeoUtils::check_longitude(min_longitude)?;
        GeoUtils::check_latitude(max_latitude)?;
        GeoUtils::check_longitude(max_longitude)?;
        GeoBBoxFactory::make_geo_bbox(
            planet_model,
            Self::from_degrees(max_latitude),
            Self::from_degrees(min_latitude),
            Self::from_degrees(min_longitude),
            Self::from_degrees(max_longitude),
        )
    }

    /// The private `fromPolygon(PlanetModel, Polygon)`: `None` when the
    /// polygon is degenerate.
    fn from_single_polygon(
        planet_model: &Arc<PlanetModel>,
        polygon: &Polygon,
    ) -> Result<Option<Arc<dyn GeoPolygon>>> {
        let the_holes = polygon.get_holes();
        let mut hole_list: Vec<Arc<dyn GeoPolygon>> = Vec::with_capacity(the_holes.len());
        for hole in the_holes {
            if let Some(component) = Self::from_single_polygon(planet_model, hole)? {
                hole_list.push(component);
            }
        }
        let points = Self::reversed_points(planet_model, polygon)?;
        GeoPolygonFactory::make_geo_polygon_with_holes(planet_model, points, Some(hole_list))
    }

    /// The polygon's vertices without the closing one, in reverse order, as
    /// both `fromPolygon` and `convertToDescription` build them.
    fn reversed_points(
        planet_model: &Arc<PlanetModel>,
        polygon: &Polygon,
    ) -> Result<Vec<GeoPoint>> {
        let poly_lats = polygon.get_polylats();
        let poly_lons = polygon.get_polylons();
        let n = poly_lats.len() as i64;
        let mut points = Vec::with_capacity((n - 1).max(0) as usize);
        let mut i: i64 = 0;
        while i < n - 1 {
            let index = (n - 2 - i) as usize;
            points.push(GeoPoint::new_lat_lon(
                planet_model,
                Self::from_degrees(poly_lats[index]),
                Self::from_degrees(poly_lons[index]),
            )?);
            i += 1;
        }
        Ok(points)
    }

    /// `convertToDescription(PlanetModel, Polygon...)`.
    fn convert_to_description(
        planet_model: &Arc<PlanetModel>,
        polygons: &[Polygon],
    ) -> Result<Vec<PolygonDescription>> {
        let mut descriptions = Vec::with_capacity(polygons.len());
        for polygon in polygons {
            let holes = Self::convert_to_description(planet_model, polygon.get_holes())?;
            let points = Self::reversed_points(planet_model, polygon)?;
            descriptions.push(PolygonDescription::with_holes(points, holes));
        }
        Ok(descriptions)
    }
}
