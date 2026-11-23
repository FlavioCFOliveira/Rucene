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

//! The types declared in `GeoShape.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use super::geo_bounds::GeoBounds;
use super::geo_point::GeoPoint;
use super::membership::Membership;
use super::plane::Plane;

/// Generic shape.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoShape`.
pub trait GeoShape: GeoBounds + AsGeoShape {
    /// `getEdgePoints()`: a sample of points on the edge of the shape.
    fn get_edge_points(&self) -> Vec<GeoPoint>;

    /// `intersects(Plane, GeoPoint[], Membership...)`.
    fn intersects(
        &self,
        plane: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool;
}

/// The upcast of any [`GeoShape`] to `&dyn GeoShape`, for generic code.
pub trait AsGeoShape {
    /// The shape as a trait object.
    fn as_geo_shape(&self) -> &dyn GeoShape;
}

impl<T: GeoShape> AsGeoShape for T {
    fn as_geo_shape(&self) -> &dyn GeoShape {
        self
    }
}
