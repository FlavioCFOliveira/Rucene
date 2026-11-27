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

//! The types declared in `Geo3dShape.java`, of the Java package
//! `org.apache.lucene.spatial.spatial4j`.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use rucene::error::Result;
use rucene_spatial3d::geom::geo_area;
use rucene_spatial3d::geom::geo_area_factory::GeoAreaFactory;
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::geo_shape::GeoShape;
use rucene_spatial3d::geom::lat_lon_bounds::LatLonBounds;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::jdk::{panic_unsupported, unsupported};
use crate::spatial4j::geo3d_circle_shape::Geo3dCircleShape;
use crate::spatial4j::geo3d_point_shape::Geo3dPointShape;
use crate::spatial4j::geo3d_rectangle_shape::Geo3dRectangleShape;

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShape` (`public
/// class Geo3dShape<T extends GeoAreaShape> implements Shape`): a spatial4j
/// shape wrapping a spatial3d `GeoAreaShape`, used to index 3D geometries
/// with spatial4j strategies. Shared as `Arc<dyn Shape>`.
///
/// Java's subclasses (`Geo3dPointShape`, `Geo3dCircleShape`,
/// `Geo3dRectangleShape`) embed this struct; `instanceof Geo3dShape<?>` is
/// [`geo3d_shape_of`]. The type parameter `T` is the type of the wrapped
/// shape, which the subclasses keep in their own typed field next to the
/// `GeoAreaShape` held here. `shape`, `boundingBox` and `center` are
/// reassigned by `reset`, so they are behind locks.
pub struct Geo3dShape {
    /// `spatialcontext`.
    pub(crate) spatialcontext: Arc<SpatialContext>,
    /// `shape`.
    pub(crate) shape: RwLock<Arc<dyn GeoAreaShape>>,
    /// `boundingBox`: lazy initialized.
    pub(crate) bounding_box: Mutex<Option<Arc<dyn Rectangle>>>,
    /// `center`: lazy initialized.
    pub(crate) center: Mutex<Option<Arc<dyn Point>>>,
}

/// `other instanceof Geo3dShape<?> ? (Geo3dShape<?>) other : null`.
pub fn geo3d_shape_of(other: &dyn Shape) -> Option<&Geo3dShape> {
    let any = other.as_any();
    if let Some(s) = any.downcast_ref::<Geo3dShape>() {
        return Some(s);
    }
    if let Some(s) = any.downcast_ref::<Geo3dPointShape>() {
        return Some(&s.base);
    }
    if let Some(s) = any.downcast_ref::<Geo3dCircleShape>() {
        return Some(&s.base);
    }
    if let Some(s) = any.downcast_ref::<Geo3dRectangleShape>() {
        return Some(&s.base);
    }
    None
}

/// Locks a lazily-initialized field; a poisoned lock is recovered.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Geo3dShape {
    /// `Geo3dShape(T shape, SpatialContext spatialcontext)`.
    pub fn new(shape: Arc<dyn GeoAreaShape>, spatialcontext: Arc<SpatialContext>) -> Self {
        Self {
            spatialcontext,
            shape: RwLock::new(shape),
            bounding_box: Mutex::new(None),
            center: Mutex::new(None),
        }
    }

    /// `shape`.
    pub fn shape(&self) -> Arc<dyn GeoAreaShape> {
        Arc::clone(&self.shape.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// `shape = value`.
    pub(crate) fn set_shape(&self, value: Arc<dyn GeoAreaShape>) {
        *self.shape.write().unwrap_or_else(|e| e.into_inner()) = value;
    }

    /// `relate(Shape other)`.
    ///
    /// # Panics
    ///
    /// `RuntimeException("Unimplemented shape relationship determination:
    /// ...")` for a shape that is neither a `Geo3dShape`, a `Rectangle` nor a
    /// `Point`, and the exceptions of spatial3d.
    pub(crate) fn relate_shape(&self, other: &dyn Shape) -> SpatialRelation {
        let relationship = if let Some(other_geo) = geo3d_shape_of(other) {
            self.relate_geo3d(other_geo)
        } else if let Some(r) = other.as_rectangle() {
            self.relate_rectangle(r)
        } else if let Some(p) = other.as_point() {
            self.relate_point(p)
        } else {
            panic!(
                "java.lang.RuntimeException: Unimplemented shape relationship determination: class {}",
                other.java_class_name()
            );
        };
        let relationship = relationship.unwrap_or_else(|e| panic!("{e}"));
        match relationship {
            geo_area::DISJOINT => SpatialRelation::Disjoint,
            geo_area::OVERLAPS => {
                if other.as_point().is_some() {
                    SpatialRelation::Contains
                } else {
                    SpatialRelation::Intersects
                }
            }
            geo_area::CONTAINS => {
                if other.as_point().is_some() {
                    SpatialRelation::Contains
                } else {
                    SpatialRelation::Within
                }
            }
            geo_area::WITHIN => SpatialRelation::Contains,
            _ => panic!(
                "java.lang.RuntimeException: Undetermined shape relationship: {relationship}"
            ),
        }
    }

    fn relate_geo3d(&self, s: &Geo3dShape) -> Result<i32> {
        let other = s.shape();
        self.shape().get_relationship(&*other as &dyn GeoShape)
    }

    fn relate_rectangle(&self, r: &dyn Rectangle) -> Result<i32> {
        // Construct the right kind of GeoArea first
        let shape = self.shape();
        let geo_area = GeoAreaFactory::make_geo_area(
            shape.get_planet_model(),
            r.get_max_y() * DistanceUtils::DEGREES_TO_RADIANS,
            r.get_min_y() * DistanceUtils::DEGREES_TO_RADIANS,
            r.get_min_x() * DistanceUtils::DEGREES_TO_RADIANS,
            r.get_max_x() * DistanceUtils::DEGREES_TO_RADIANS,
        )?;
        geo_area.get_relationship(&*shape as &dyn GeoShape)
    }

    fn relate_point(&self, p: &dyn Point) -> Result<i32> {
        let shape = self.shape();
        let point = GeoPoint::new_lat_lon(
            shape.get_planet_model(),
            p.get_y() * DistanceUtils::DEGREES_TO_RADIANS,
            p.get_x() * DistanceUtils::DEGREES_TO_RADIANS,
        )?;
        if shape.is_within(&point) {
            return Ok(geo_area::WITHIN);
        }
        Ok(geo_area::DISJOINT)
    }

    /// `getBoundingBox()`.
    pub(crate) fn bounding_box_default(&self) -> Arc<dyn Rectangle> {
        let mut bbox = lock(&self.bounding_box); // volatile read once
        if let Some(b) = &*bbox {
            return Arc::clone(b);
        }
        let shape = self.shape();
        let mut bounds = LatLonBounds::new();
        shape.get_bounds(&mut bounds);
        let geo_b_box =
            GeoBBoxFactory::make_geo_bbox_from_bounds(shape.get_planet_model(), &bounds)
                .unwrap_or_else(|e| panic!("{e}"));
        let b: Arc<dyn Rectangle> =
            Geo3dRectangleShape::new(geo_b_box, Arc::clone(&self.spatialcontext));
        *bbox = Some(Arc::clone(&b));
        b
    }

    /// `getCenter()`, over the given `getBoundingBox()`.
    pub(crate) fn center_with(
        &self,
        get_bounding_box: impl FnOnce() -> Arc<dyn Rectangle>,
    ) -> Arc<dyn Point> {
        if let Some(c) = &*lock(&self.center) {
            // volatile read once
            return Arc::clone(c);
        }
        let center = get_bounding_box().get_center();
        *lock(&self.center) = Some(Arc::clone(&center));
        center
    }

    /// `equals(Object o)`.
    pub(crate) fn equals_shape(&self, o: &dyn Shape) -> bool {
        let Some(other) = geo3d_shape_of(o) else {
            return false;
        };
        // `SpatialContext.equals` is `Object.equals`: identity.
        Arc::ptr_eq(&other.spatialcontext, &self.spatialcontext)
            && other.shape().java_equals(self.shape().as_any())
    }

    /// `hashCode()`: `spatialcontext.hashCode() + shape.hashCode()`, the
    /// first being an identity hash (the core's stand-in).
    pub(crate) fn hash_code_shape(&self) -> i32 {
        crate::jdk::identity_hash("org.locationtech.spatial4j.context.SpatialContext")
            .wrapping_add(self.shape().java_hash_code())
    }

    /// `toString()`: the shape usually prints its planet model.
    pub(crate) fn to_string_shape(&self) -> String {
        format!("Geo3D:{}", self.shape())
    }
}

impl fmt::Display for Geo3dShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_shape())
    }
}

impl fmt::Debug for Geo3dShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_shape())
    }
}

impl Shape for Geo3dShape {
    spatial4j::spatial4j_shape_boilerplate!("org.apache.lucene.spatial.spatial4j.Geo3dShape");

    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        self.relate_shape(other)
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.bounding_box_default()
    }

    fn has_area(&self) -> bool {
        true
    }

    fn get_area(&self, _ctx: Option<&SpatialContext>) -> f64 {
        panic_unsupported()
    }

    fn get_center(&self) -> Arc<dyn Point> {
        self.center_with(|| self.bounding_box_default())
    }

    fn get_buffered(&self, _distance: f64, _ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        Err(unsupported())
    }

    fn is_empty(&self) -> bool {
        false
    }

    fn equals(&self, other: &dyn Shape) -> bool {
        self.equals_shape(other)
    }

    fn hash_code(&self) -> i32 {
        self.hash_code_shape()
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        Some(Arc::clone(&self.spatialcontext))
    }
}
