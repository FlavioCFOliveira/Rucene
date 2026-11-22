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


//! The types declared in `GeoArea.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::error::Result;

use super::geo_shape::GeoShape;
use super::membership::Membership;

/// A `GeoArea` is used just to determine the relationship between a shape
/// and a specific area.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoArea`.
pub trait GeoArea: Membership {
    /// `getRelationship(GeoShape)`: one of [`CONTAINS`], [`WITHIN`],
    /// [`OVERLAPS`] or [`DISJOINT`].
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the planet models differ.
    fn get_relationship(&self, shape: &dyn GeoShape) -> Result<i32>;
}

/// The referenced shape CONTAINS this area.
pub const CONTAINS: i32 = 0;
/// The referenced shape IS WITHIN this area.
pub const WITHIN: i32 = 1;
/// The referenced shape OVERLAPS this area.
pub const OVERLAPS: i32 = 2;
/// The referenced shape has no relation to this area.
pub const DISJOINT: i32 = 3;
