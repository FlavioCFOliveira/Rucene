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


//! The types declared in `GeoCircleFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::sync::Arc;

use rucene::error::Result;

use super::geo_circle::GeoCircle;
use super::geo_degenerate_point::GeoDegeneratePoint;
use super::geo_exact_circle::GeoExactCircle;
use super::geo_standard_circle::GeoStandardCircle;
use super::planet_model::PlanetModel;
use super::vector::Vector;

/// Class which constructs a [`GeoCircle`] representing an arbitrary circle.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoCircleFactory`.
pub struct GeoCircleFactory;

impl GeoCircleFactory {
    /// `makeGeoCircle(PlanetModel, double latitude, double longitude, double
    /// cutoffAngle)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the circle constructors.
    pub fn make_geo_circle(
        planet_model: &Arc<PlanetModel>,
        latitude: f64,
        longitude: f64,
        cutoff_angle: f64,
    ) -> Result<Arc<dyn GeoCircle>> {
        if cutoff_angle < Vector::MINIMUM_ANGULAR_RESOLUTION {
            return Ok(Arc::new(GeoDegeneratePoint::new(planet_model, latitude, longitude)?));
        }
        Ok(Arc::new(GeoStandardCircle::new(planet_model, latitude, longitude, cutoff_angle)?))
    }

    /// `makeExactGeoCircle(PlanetModel, double latitude, double longitude,
    /// double radius, double accuracy)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the circle constructors.
    pub fn make_exact_geo_circle(
        planet_model: &Arc<PlanetModel>,
        latitude: f64,
        longitude: f64,
        radius: f64,
        accuracy: f64,
    ) -> Result<Arc<dyn GeoCircle>> {
        if radius < Vector::MINIMUM_ANGULAR_RESOLUTION {
            return Ok(Arc::new(GeoDegeneratePoint::new(planet_model, latitude, longitude)?));
        }
        Ok(Arc::new(GeoExactCircle::new(planet_model, latitude, longitude, radius, accuracy)?))
    }
}
