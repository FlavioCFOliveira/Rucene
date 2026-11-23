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

//! The types declared in `GeoPointShapeFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::sync::Arc;

use rucene::error::Result;

use super::geo_degenerate_point::GeoDegeneratePoint;
use super::geo_point_shape::GeoPointShape;
use super::planet_model::PlanetModel;

/// Class which constructs a [`GeoPointShape`].
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPointShapeFactory`.
pub struct GeoPointShapeFactory;

impl GeoPointShapeFactory {
    /// `makeGeoPointShape(PlanetModel, double lat, double lon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the latitude or longitude is out of
    /// range.
    pub fn make_geo_point_shape(
        planet_model: &Arc<PlanetModel>,
        lat: f64,
        lon: f64,
    ) -> Result<Arc<dyn GeoPointShape>> {
        Ok(Arc::new(GeoDegeneratePoint::new(planet_model, lat, lon)?))
    }
}
