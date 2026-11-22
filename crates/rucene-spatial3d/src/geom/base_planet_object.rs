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


//! The types declared in `BasePlanetObject.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use super::planet_model::PlanetModel;

/// All Geo3D shapes can derive from this base class, which furnishes some
/// common code.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.BasePlanetObject`.
///
/// **Divergence.** Rust has no class inheritance. The abstract class is this
/// namespace of its shared behaviour, which each concrete class (a struct)
/// reaches through the `impl_*` macros of [`super`] and through these
/// associated functions where Java calls `super`.
/// The `planetModel` field is the `planet_model` field of each struct.
pub struct BasePlanetObject;

impl BasePlanetObject {
    /// `BasePlanetObject.hashCode()`.
    pub fn hash_code(planet_model: &PlanetModel) -> i32 {
        use super::serializable_object::SerializableObject;
        planet_model.java_hash_code()
    }

    /// `BasePlanetObject.equals(Object)` over the two planet models.
    pub fn equals(planet_model: &PlanetModel, other: &PlanetModel) -> bool {
        planet_model == other
    }

    /// `BasePlanetObject.write(OutputStream)`: unsupported.
    pub fn write() -> rucene::error::Result<()> {
        Err(rucene::error::LuceneError::unsupported_operation(""))
    }
}
