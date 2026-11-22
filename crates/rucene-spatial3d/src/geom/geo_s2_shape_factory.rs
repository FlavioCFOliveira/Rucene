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


//! The types declared in `GeoS2ShapeFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::sync::Arc;

use rucene::error::Result;

use super::geo_point::GeoPoint;
use super::geo_polygon::GeoPolygon;
use super::geo_s2_shape::GeoS2Shape;
use super::planet_model::PlanetModel;

/// Class which constructs a GeoPolygon representing an S2 google pixel.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoS2ShapeFactory`.
pub struct GeoS2ShapeFactory;

impl GeoS2ShapeFactory {
    /// `makeGeoS2Shape(PlanetModel, GeoPoint, GeoPoint, GeoPoint, GeoPoint)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a side plane cannot be built.
    pub fn make_geo_s2_shape(
        planet_model: &Arc<PlanetModel>,
        point1: GeoPoint,
        point2: GeoPoint,
        point3: GeoPoint,
        point4: GeoPoint,
    ) -> Result<Arc<dyn GeoPolygon>> {
        Ok(Arc::new(GeoS2Shape::new(planet_model, point1, point2, point3, point4)?))
    }
}
