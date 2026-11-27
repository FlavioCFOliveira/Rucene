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

//! The types declared in `GeoAreaFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::sync::Arc;

use rucene::error::Result;

use super::geo_area::GeoArea;
use super::geo_b_box_factory::GeoBBoxFactory;
use super::planet_model::PlanetModel;
use super::xyz_solid_factory::XYZSolidFactory;

/// Factory for [`GeoArea`].
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoAreaFactory`.
pub struct GeoAreaFactory;

impl GeoAreaFactory {
    /// `makeGeoArea(PlanetModel, double topLat, double bottomLat, double
    /// leftLon, double rightLon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the bounding box factory.
    pub fn make_geo_area(
        planet_model: &Arc<PlanetModel>,
        top_lat: f64,
        bottom_lat: f64,
        left_lon: f64,
        right_lon: f64,
    ) -> Result<Arc<dyn GeoArea>> {
        Ok(GeoBBoxFactory::make_geo_bbox(
            planet_model,
            top_lat,
            bottom_lat,
            left_lon,
            right_lon,
        )?)
    }

    /// `makeGeoArea(PlanetModel, double minX, double maxX, double minY, double
    /// maxY, double minZ, double maxZ)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the solid constructors.
    #[allow(clippy::too_many_arguments)]
    pub fn make_geo_area_xyz(
        planet_model: &Arc<PlanetModel>,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        min_z: f64,
        max_z: f64,
    ) -> Result<Arc<dyn GeoArea>> {
        Ok(XYZSolidFactory::make_xyz_solid(
            planet_model,
            min_x,
            max_x,
            min_y,
            max_y,
            min_z,
            max_z,
        )?)
    }
}
