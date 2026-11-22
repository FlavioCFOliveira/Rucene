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


//! The types declared in `GeoBBoxFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::f64::consts::PI;
use std::sync::Arc;

use rucene::error::Result;

use super::geo_b_box::GeoBBox;
use super::geo_degenerate_horizontal_line::GeoDegenerateHorizontalLine;
use super::geo_degenerate_latitude_zone::GeoDegenerateLatitudeZone;
use super::geo_degenerate_longitude_slice::GeoDegenerateLongitudeSlice;
use super::geo_degenerate_point::GeoDegeneratePoint;
use super::geo_degenerate_vertical_line::GeoDegenerateVerticalLine;
use super::geo_latitude_zone::GeoLatitudeZone;
use super::geo_longitude_slice::GeoLongitudeSlice;
use super::geo_north_latitude_zone::GeoNorthLatitudeZone;
use super::geo_north_rectangle::GeoNorthRectangle;
use super::geo_rectangle::GeoRectangle;
use super::geo_south_latitude_zone::GeoSouthLatitudeZone;
use super::geo_south_rectangle::GeoSouthRectangle;
use super::geo_wide_degenerate_horizontal_line::GeoWideDegenerateHorizontalLine;
use super::geo_wide_longitude_slice::GeoWideLongitudeSlice;
use super::geo_wide_north_rectangle::GeoWideNorthRectangle;
use super::geo_wide_rectangle::GeoWideRectangle;
use super::geo_wide_south_rectangle::GeoWideSouthRectangle;
use super::geo_world::GeoWorld;
use super::lat_lon_bounds::LatLonBounds;
use super::npe;
use super::planet_model::PlanetModel;
use super::vector::Vector;

/// Factory for [`GeoBBox`].
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoBBoxFactory`.
pub struct GeoBBoxFactory;

impl GeoBBoxFactory {
    /// `makeGeoBBox(PlanetModel, double topLat, double bottomLat, double
    /// leftLon, double rightLon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the shape constructors.
    pub fn make_geo_bbox(
        planet_model: &Arc<PlanetModel>,
        mut top_lat: f64,
        mut bottom_lat: f64,
        mut left_lon: f64,
        mut right_lon: f64,
    ) -> Result<Arc<dyn GeoBBox>> {
        let pm = planet_model;
        if top_lat > PI * 0.5 {
            top_lat = PI * 0.5;
        }
        if bottom_lat < -PI * 0.5 {
            bottom_lat = -PI * 0.5;
        }
        if left_lon < -PI {
            left_lon = -PI;
        }
        if right_lon > PI {
            right_lon = PI;
        }
        if (Self::longitudes_equals(left_lon, -PI) && Self::longitudes_equals(right_lon, PI))
            || (Self::longitudes_equals(right_lon, -PI) && Self::longitudes_equals(left_lon, PI))
        {
            if Self::is_north_pole(top_lat) && Self::is_south_pole(bottom_lat) {
                return Ok(Arc::new(GeoWorld::new(pm)));
            }
            if Self::latitudes_equals(top_lat, bottom_lat) {
                if Self::is_north_pole(top_lat) {
                    return Ok(Arc::new(GeoDegeneratePoint::new(pm, top_lat, 0.0)?));
                } else if Self::is_south_pole(bottom_lat) {
                    return Ok(Arc::new(GeoDegeneratePoint::new(pm, bottom_lat, 0.0)?));
                }
                return Ok(Arc::new(GeoDegenerateLatitudeZone::new(pm, top_lat)?));
            }
            if Self::is_north_pole(top_lat) {
                return Ok(Arc::new(GeoNorthLatitudeZone::new(pm, bottom_lat)?));
            } else if Self::is_south_pole(bottom_lat) {
                return Ok(Arc::new(GeoSouthLatitudeZone::new(pm, top_lat)?));
            }
            return Ok(Arc::new(GeoLatitudeZone::new(pm, top_lat, bottom_lat)?));
        }
        let mut extent = right_lon - left_lon;
        if extent < 0.0 {
            extent += PI * 2.0;
        }
        if Self::is_north_pole(top_lat) && Self::is_south_pole(bottom_lat) {
            if Self::longitudes_equals(left_lon, right_lon) {
                return Ok(Arc::new(GeoDegenerateLongitudeSlice::new(pm, left_lon)?));
            }
            if extent >= PI {
                return Ok(Arc::new(GeoWideLongitudeSlice::new(pm, left_lon, right_lon)?));
            }
            return Ok(Arc::new(GeoLongitudeSlice::new(pm, left_lon, right_lon)?));
        }
        if Self::longitudes_equals(left_lon, right_lon) {
            if Self::latitudes_equals(top_lat, bottom_lat) {
                return Ok(Arc::new(GeoDegeneratePoint::new(pm, top_lat, left_lon)?));
            }
            return Ok(Arc::new(GeoDegenerateVerticalLine::new(
                pm, top_lat, bottom_lat, left_lon,
            )?));
        }
        if extent >= GeoWideRectangle::MIN_WIDE_EXTENT {
            if Self::latitudes_equals(top_lat, bottom_lat) {
                if Self::is_north_pole(top_lat) {
                    return Ok(Arc::new(GeoDegeneratePoint::new(pm, top_lat, 0.0)?));
                } else if Self::is_south_pole(bottom_lat) {
                    return Ok(Arc::new(GeoDegeneratePoint::new(pm, bottom_lat, 0.0)?));
                }
                return Ok(Arc::new(GeoWideDegenerateHorizontalLine::new(
                    pm, top_lat, left_lon, right_lon,
                )?));
            }
            if Self::is_north_pole(top_lat) {
                return Ok(Arc::new(GeoWideNorthRectangle::new(
                    pm, bottom_lat, left_lon, right_lon,
                )?));
            } else if Self::is_south_pole(bottom_lat) {
                return Ok(Arc::new(GeoWideSouthRectangle::new(
                    pm, top_lat, left_lon, right_lon,
                )?));
            }
            return Ok(Arc::new(GeoWideRectangle::new(
                pm, top_lat, bottom_lat, left_lon, right_lon,
            )?));
        }
        if Self::latitudes_equals(top_lat, bottom_lat) {
            if Self::is_north_pole(top_lat) {
                return Ok(Arc::new(GeoDegeneratePoint::new(pm, top_lat, 0.0)?));
            } else if Self::is_south_pole(bottom_lat) {
                return Ok(Arc::new(GeoDegeneratePoint::new(pm, bottom_lat, 0.0)?));
            }
            return Ok(Arc::new(GeoDegenerateHorizontalLine::new(
                pm, top_lat, left_lon, right_lon,
            )?));
        }
        if Self::is_north_pole(top_lat) {
            return Ok(Arc::new(GeoNorthRectangle::new(
                pm, bottom_lat, left_lon, right_lon,
            )?));
        } else if Self::is_south_pole(bottom_lat) {
            return Ok(Arc::new(GeoSouthRectangle::new(
                pm, top_lat, left_lon, right_lon,
            )?));
        }
        Ok(Arc::new(GeoRectangle::new(
            pm, top_lat, bottom_lat, left_lon, right_lon,
        )?))
    }

    fn is_north_pole(lat: f64) -> bool {
        Self::latitudes_equals(lat, PI * 0.5)
    }

    fn is_south_pole(lat: f64) -> bool {
        Self::latitudes_equals(lat, -PI * 0.5)
    }

    fn latitudes_equals(lat1: f64, lat2: f64) -> bool {
        (lat1 - lat2).abs() < Vector::MINIMUM_ANGULAR_RESOLUTION
            || (lat1.sin() - lat2.sin()).abs() < Vector::MINIMUM_RESOLUTION
    }

    fn longitudes_equals(lon1: f64, lon2: f64) -> bool {
        (lon1 - lon2).abs() < Vector::MINIMUM_ANGULAR_RESOLUTION
    }

    /// `makeGeoBBox(PlanetModel, LatLonBounds)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the shape constructors.
    pub fn make_geo_bbox_from_bounds(
        planet_model: &Arc<PlanetModel>,
        bounds: &LatLonBounds,
    ) -> Result<Arc<dyn GeoBBox>> {
        let unbox = |v: Option<f64>| v.unwrap_or_else(|| npe());
        let top_lat = if bounds.check_no_top_latitude_bound() {
            PI * 0.5
        } else {
            unbox(bounds.get_max_latitude())
        };
        let bottom_lat = if bounds.check_no_bottom_latitude_bound() {
            -PI * 0.5
        } else {
            unbox(bounds.get_min_latitude())
        };
        let left_lon = if bounds.check_no_longitude_bound() {
            -PI
        } else {
            unbox(bounds.get_left_longitude())
        };
        let right_lon = if bounds.check_no_longitude_bound() {
            PI
        } else {
            unbox(bounds.get_right_longitude())
        };
        Self::make_geo_bbox(planet_model, top_lat, bottom_lat, left_lon, right_lon)
    }
}
