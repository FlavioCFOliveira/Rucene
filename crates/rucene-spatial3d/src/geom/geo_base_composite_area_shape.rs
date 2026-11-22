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


//! The types declared in `GeoBaseCompositeAreaShape.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::error::Result;

use super::geo_area;
use super::geo_area_shape::GeoAreaShape;
use super::geo_base_composite_shape::GeoBaseCompositeShape;
use super::geo_shape::GeoShape;
use super::iae;

/// Base class to create a composite of GeoAreaShapes.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoBaseCompositeAreaShape`.
///
/// **Divergence.** Rust has no class inheritance. The abstract class is this
/// namespace of its shared behaviour over a
/// [`GeoBaseCompositeShape`] of area shapes.
pub(crate) struct GeoBaseCompositeAreaShape;

impl GeoBaseCompositeAreaShape {
    /// All edgepoints inside shape.
    pub(crate) const ALL_INSIDE: i32 = 0;
    /// Some edgepoints inside shape.
    pub(crate) const SOME_INSIDE: i32 = 1;
    /// No edgepoints inside shape.
    pub(crate) const NONE_INSIDE: i32 = 2;

    /// `intersects(GeoShape)`.
    pub(crate) fn intersects<T: ?Sized + GeoAreaShape>(base: &GeoBaseCompositeShape<T>, geo_shape: &dyn GeoShape) -> bool {
        for geo_area_shape in &base.shapes {
            if geo_area_shape.intersects_shape(geo_shape) {
                return true;
            }
        }
        false
    }

    /// `getRelationship(GeoShape)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the planet models differ.
    pub(crate) fn get_relationship<T: ?Sized + GeoAreaShape>(
        base: &GeoBaseCompositeShape<T>,
        geo_shape: &dyn GeoShape,
    ) -> Result<i32> {
        if **geo_shape.get_planet_model() != *base.planet_model {
            return Err(iae("Cannot relate shapes with different planet models."));
        }
        let inside_geo_area_shape = Self::is_shape_inside_geo_area_shape(base, geo_shape);
        if inside_geo_area_shape == Self::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        let inside_shape = Self::is_geo_area_shape_inside_shape(base, geo_shape);
        if inside_shape == Self::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        if inside_geo_area_shape == Self::ALL_INSIDE && inside_shape == Self::ALL_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        if Self::intersects(base, geo_shape) {
            return Ok(geo_area::OVERLAPS);
        }
        if inside_geo_area_shape == Self::ALL_INSIDE {
            return Ok(geo_area::WITHIN);
        }
        if inside_shape == Self::ALL_INSIDE {
            return Ok(geo_area::CONTAINS);
        }
        Ok(geo_area::DISJOINT)
    }

    /// `isShapeInsideGeoAreaShape(GeoShape)`.
    pub(crate) fn is_shape_inside_geo_area_shape<T: ?Sized + GeoAreaShape>(
        base: &GeoBaseCompositeShape<T>,
        geo_shape: &dyn GeoShape,
    ) -> i32 {
        let mut found_outside = false;
        let mut found_inside = false;
        for p in geo_shape.get_edge_points() {
            if base.is_within_xyz(p.x, p.y, p.z) {
                found_inside = true;
            } else {
                found_outside = true;
            }
            if found_inside && found_outside {
                return Self::SOME_INSIDE;
            }
        }
        if !found_inside && !found_outside {
            return Self::NONE_INSIDE;
        }
        if found_inside && !found_outside {
            return Self::ALL_INSIDE;
        }
        if found_outside && !found_inside {
            return Self::NONE_INSIDE;
        }
        Self::SOME_INSIDE
    }

    /// `isGeoAreaShapeInsideShape(GeoShape)`.
    pub(crate) fn is_geo_area_shape_inside_shape<T: ?Sized + GeoAreaShape>(
        base: &GeoBaseCompositeShape<T>,
        geoshape: &dyn GeoShape,
    ) -> i32 {
        let mut found_outside = false;
        let mut found_inside = false;
        for p in base.get_edge_points() {
            if geoshape.is_within(&p) {
                found_inside = true;
            } else {
                found_outside = true;
            }
            if found_inside && found_outside {
                return Self::SOME_INSIDE;
            }
        }
        if !found_inside && !found_outside {
            return Self::NONE_INSIDE;
        }
        if found_inside && !found_outside {
            return Self::ALL_INSIDE;
        }
        if found_outside && !found_inside {
            return Self::NONE_INSIDE;
        }
        Self::SOME_INSIDE
    }
}
