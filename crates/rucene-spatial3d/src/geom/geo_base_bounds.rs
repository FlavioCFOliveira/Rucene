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


//! The types declared in `GeoBaseBounds.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use super::bounds::Bounds;
use super::membership::Membership;
use super::planet_model::PlanetModel;

/// Base extended bounds object.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoBaseBounds`.
///
/// **Divergence.** Rust has no class inheritance. The abstract class is this
/// namespace of its shared behaviour, which each concrete class (a struct)
/// reaches through the `impl_*` macros of [`super`] and through these
/// associated functions where Java calls `super`.
pub struct GeoBaseBounds;

impl GeoBaseBounds {
    /// `GeoBaseBounds.getBounds(Bounds)`: records the poles within `this`.
    pub fn get_bounds(this: &dyn Membership, planet_model: &PlanetModel, bounds: &mut dyn Bounds) {
        if this.is_within(&planet_model.north_pole) {
            bounds
                .no_top_latitude_bound()
                .no_longitude_bound()
                .add_point(&planet_model.north_pole);
        }
        if this.is_within(&planet_model.south_pole) {
            bounds
                .no_bottom_latitude_bound()
                .no_longitude_bound()
                .add_point(&planet_model.south_pole);
        }
        if this.is_within(&planet_model.min_x_pole) {
            bounds.add_point(&planet_model.min_x_pole);
        }
        if this.is_within(&planet_model.max_x_pole) {
            bounds.add_point(&planet_model.max_x_pole);
        }
        if this.is_within(&planet_model.min_y_pole) {
            bounds.add_point(&planet_model.min_y_pole);
        }
        if this.is_within(&planet_model.max_y_pole) {
            bounds.add_point(&planet_model.max_y_pole);
        }
    }
}
