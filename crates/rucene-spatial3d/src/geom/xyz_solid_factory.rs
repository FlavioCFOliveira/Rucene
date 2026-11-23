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

//! The types declared in `XYZSolidFactory.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::sync::Arc;

use rucene::error::Result;

use super::d_x_yd_z_solid::dXYdZSolid;
use super::d_xd_yd_z_solid::dXdYdZSolid;
use super::d_xd_yz_solid::dXdYZSolid;
use super::d_xyz_solid::dXYZSolid;
use super::npe;
use super::planet_model::PlanetModel;
use super::standard_xyz_solid::StandardXYZSolid;
use super::vector::Vector;
use super::x_yd_z_solid::XYdZSolid;
use super::xd_yd_z_solid::XdYdZSolid;
use super::xd_yz_solid::XdYZSolid;
use super::xyz_bounds::XYZBounds;
use super::xyz_solid::XYZSolid;

/// Factory for [`XYZSolid`].
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.XYZSolidFactory`.
pub struct XYZSolidFactory;

impl XYZSolidFactory {
    /// `makeXYZSolid(PlanetModel, double minX, double maxX, double minY,
    /// double maxY, double minZ, double maxZ)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the values are in the wrong order.
    #[allow(clippy::too_many_arguments)]
    pub fn make_xyz_solid(
        planet_model: &Arc<PlanetModel>,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        min_z: f64,
        max_z: f64,
    ) -> Result<Arc<dyn XYZSolid>> {
        let pm = planet_model;
        if (max_x - min_x).abs() < Vector::MINIMUM_RESOLUTION {
            if (max_y - min_y).abs() < Vector::MINIMUM_RESOLUTION {
                if (max_z - min_z).abs() < Vector::MINIMUM_RESOLUTION {
                    return Ok(Arc::new(dXdYdZSolid::new(
                        pm,
                        (min_x + max_x) * 0.5,
                        (min_y + max_y) * 0.5,
                        min_z,
                    )));
                } else {
                    return Ok(Arc::new(dXdYZSolid::new(
                        pm,
                        (min_x + max_x) * 0.5,
                        (min_y + max_y) * 0.5,
                        min_z,
                        max_z,
                    )?));
                }
            } else if (max_z - min_z).abs() < Vector::MINIMUM_RESOLUTION {
                return Ok(Arc::new(dXYdZSolid::new(
                    pm,
                    (min_x + max_x) * 0.5,
                    min_y,
                    max_y,
                    (min_z + max_z) * 0.5,
                )?));
            } else {
                return Ok(Arc::new(dXYZSolid::new(
                    pm,
                    (min_x + max_x) * 0.5,
                    min_y,
                    max_y,
                    min_z,
                    max_z,
                )?));
            }
        }
        if (max_y - min_y).abs() < Vector::MINIMUM_RESOLUTION {
            if (max_z - min_z).abs() < Vector::MINIMUM_RESOLUTION {
                return Ok(Arc::new(XdYdZSolid::new(
                    pm,
                    min_x,
                    max_x,
                    (min_y + max_y) * 0.5,
                    (min_z + max_z) * 0.5,
                )?));
            } else {
                return Ok(Arc::new(XdYZSolid::new(
                    pm,
                    min_x,
                    max_x,
                    (min_y + max_y) * 0.5,
                    min_z,
                    max_z,
                )?));
            }
        }
        if (max_z - min_z).abs() < Vector::MINIMUM_RESOLUTION {
            return Ok(Arc::new(XYdZSolid::new(
                pm,
                min_x,
                max_x,
                min_y,
                max_y,
                (min_z + max_z) * 0.5,
            )?));
        }
        Ok(Arc::new(StandardXYZSolid::new(
            pm, min_x, max_x, min_y, max_y, min_z, max_z,
        )?))
    }

    /// `makeXYZSolid(PlanetModel, XYZBounds)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the values are in the wrong order.
    pub fn make_xyz_solid_from_bounds(
        planet_model: &Arc<PlanetModel>,
        bounds: &XYZBounds,
    ) -> Result<Arc<dyn XYZSolid>> {
        let ub = |v: Option<f64>| v.unwrap_or_else(|| npe());
        Self::make_xyz_solid(
            planet_model,
            ub(bounds.get_minimum_x()),
            ub(bounds.get_maximum_x()),
            ub(bounds.get_minimum_y()),
            ub(bounds.get_maximum_y()),
            ub(bounds.get_minimum_z()),
            ub(bounds.get_maximum_z()),
        )
    }
}
