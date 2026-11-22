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


//! The types declared in `GeoBBox.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::sync::Arc;

use rucene::error::Result;

use super::geo_area_shape::GeoAreaShape;
use super::geo_sizeable::GeoSizeable;

/// All bounding box shapes have this interface in common.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoBBox`.
pub trait GeoBBox: GeoAreaShape + GeoSizeable {
    /// `expand(double)`: a bounding box expanded by `angle` radians.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the bounding box factory.
    ///
    /// Takes the shared box, as a box may return itself (`return this`).
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>>;
}
