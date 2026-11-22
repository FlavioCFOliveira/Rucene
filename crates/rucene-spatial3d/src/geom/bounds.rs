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


//! The types declared in `Bounds.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::any::Any;

use super::geo_point::GeoPoint;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;

/// An interface for accumulating bounds information.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.Bounds`. Each method
/// returns the bounds object, as Java's `return this`.
pub trait Bounds: Any {
    /// `addPlane(PlanetModel, Plane, Membership...)`.
    fn add_plane(
        &mut self,
        planet_model: &PlanetModel,
        plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds;

    /// `addHorizontalPlane(PlanetModel, double, Plane, Membership...)`.
    fn add_horizontal_plane(
        &mut self,
        planet_model: &PlanetModel,
        latitude: f64,
        horizontal_plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds;

    /// `addVerticalPlane(PlanetModel, double, Plane, Membership...)`.
    fn add_vertical_plane(
        &mut self,
        planet_model: &PlanetModel,
        longitude: f64,
        vertical_plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds;

    /// `addIntersection(PlanetModel, Plane, Plane, Membership...)`.
    fn add_intersection(
        &mut self,
        planet_model: &PlanetModel,
        plane1: &Plane,
        plane2: &Plane,
        bounds: &[&dyn Membership],
    ) -> &mut dyn Bounds;

    /// `addPoint(GeoPoint)`.
    fn add_point(&mut self, point: &GeoPoint) -> &mut dyn Bounds;

    /// `addXValue(GeoPoint)`.
    fn add_x_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds;

    /// `addYValue(GeoPoint)`.
    fn add_y_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds;

    /// `addZValue(GeoPoint)`.
    fn add_z_value(&mut self, point: &GeoPoint) -> &mut dyn Bounds;

    /// `isWide()`.
    fn is_wide(&mut self) -> &mut dyn Bounds;

    /// `noLongitudeBound()`.
    fn no_longitude_bound(&mut self) -> &mut dyn Bounds;

    /// `noTopLatitudeBound()`.
    fn no_top_latitude_bound(&mut self) -> &mut dyn Bounds;

    /// `noBottomLatitudeBound()`.
    fn no_bottom_latitude_bound(&mut self) -> &mut dyn Bounds;

    /// `noBound(PlanetModel)`.
    fn no_bound(&mut self, planet_model: &PlanetModel) -> &mut dyn Bounds;

    /// The bounds as [`Any`], for `instanceof` checks.
    fn as_any(&self) -> &dyn Any;

    /// The bounds as mutable [`Any`], for `instanceof` checks and casts.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
