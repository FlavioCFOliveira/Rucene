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

//! The types declared in `GeoPathFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use super::geo_degenerate_path::GeoDegeneratePath;
use super::geo_path::GeoPath;
use super::geo_point::GeoPoint;
use super::geo_standard_path::GeoStandardPath;
use super::planet_model::PlanetModel;
use super::vector::Vector;

/// Factory for [`GeoPath`].
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPathFactory`.
pub struct GeoPathFactory;

impl GeoPathFactory {
    /// `makeGeoPath(PlanetModel, double maxCutoffAngle, GeoPoint[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal path;
    /// `ArrayIndexOutOfBoundsException` for no points.
    pub fn make_geo_path(
        planet_model: &Arc<PlanetModel>,
        max_cutoff_angle: f64,
        path_points: &[GeoPoint],
    ) -> Result<Arc<dyn GeoPath>> {
        if max_cutoff_angle < Vector::MINIMUM_ANGULAR_RESOLUTION {
            return Ok(Arc::new(GeoDegeneratePath::new(
                planet_model,
                &Self::filter_points(path_points)?,
            )?));
        }
        Ok(Arc::new(GeoStandardPath::new(
            planet_model,
            max_cutoff_angle,
            &Self::filter_points(path_points)?,
        )?))
    }

    /// `filterPoints(GeoPoint[])`: drops consecutive numerically identical
    /// points.
    fn filter_points(path_points: &[GeoPoint]) -> Result<Vec<GeoPoint>> {
        let mut no_identical_points = Vec::with_capacity(path_points.len());
        let n = path_points.len() as i64;
        let mut i: i64 = 0;
        while i < n - 1 {
            let a = &path_points[i as usize];
            let b = &path_points[i as usize + 1];
            if !a.is_numerically_identical(b) {
                no_identical_points.push(a.clone());
            }
            i += 1;
        }
        let Some(last) = path_points.last() else {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                "Index -1 out of bounds for length 0",
            ));
        };
        no_identical_points.push(last.clone());
        Ok(no_identical_points)
    }
}
