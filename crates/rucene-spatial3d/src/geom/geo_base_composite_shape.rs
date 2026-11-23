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

//! The types declared in `GeoBaseCompositeShape.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::bounds::Bounds;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object::{self as so, SerializableObject};
use super::{iae, list_hash_code};

/// Base class to create a composite of GeoShapes.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoBaseCompositeShape`.
/// The concrete composites wrap one and implement the interfaces over it;
/// `T` is the trait object type of the members (`dyn GeoPolygon`, ...).
pub struct GeoBaseCompositeShape<T: ?Sized + GeoShape> {
    pub(crate) planet_model: Arc<PlanetModel>,
    /// Shape's container.
    pub(crate) shapes: Vec<Arc<T>>,
}

impl<T: ?Sized + GeoShape> GeoBaseCompositeShape<T> {
    /// `new GeoBaseCompositeShape(PlanetModel)`: an empty composite.
    pub fn new(planet_model: &Arc<PlanetModel>) -> Self {
        Self {
            planet_model: planet_model.clone(),
            shapes: Vec::new(),
        }
    }

    /// `addShape(T)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the planet models differ.
    pub fn add_shape(&mut self, shape: Arc<T>) -> Result<()> {
        if **shape.get_planet_model() != *self.planet_model {
            return Err(iae(
                "Cannot add a shape into a composite with different planet models.",
            ));
        }
        self.shapes.push(shape);
        Ok(())
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.shapes.len()
    }

    /// `getShape(int)`.
    pub fn get_shape(&self, index: usize) -> &Arc<T> {
        &self.shapes[index]
    }

    /// `new GeoBaseCompositeShape(PlanetModel, InputStream, Class<T>)`:
    /// `cast` is `clazz.cast`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `ClassCastException` for a member of
    /// the wrong type; `IllegalArgumentException` from `addShape`.
    pub fn from_stream(
        planet_model: &Arc<PlanetModel>,
        input: &mut dyn Read,
        clazz: &str,
        cast: impl Fn(Arc<dyn SerializableObject>) -> Option<Arc<T>>,
    ) -> Result<Self> {
        let mut s = Self::new(planet_model);
        let array = so::read_heterogeneous_array(planet_model, input, clazz, cast)?;
        for member in array {
            s.add_shape(member)?;
        }
        Ok(s)
    }

    /// `write(OutputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream.
    pub fn write(&self, output: &mut dyn Write) -> Result<()> {
        let values: Vec<&dyn SerializableObject> =
            self.shapes.iter().map(|s| s.as_serializable()).collect();
        so::write_heterogeneous_array(output, &values)
    }

    /// `isWithin(double, double, double)`.
    pub fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        for shape in &self.shapes {
            if shape.is_within_xyz(x, y, z) {
                return true;
            }
        }
        false
    }

    /// `getEdgePoints()`.
    pub fn get_edge_points(&self) -> Vec<GeoPoint> {
        let mut edge_points = Vec::new();
        for shape in &self.shapes {
            edge_points.extend(shape.get_edge_points());
        }
        edge_points
    }

    /// `intersects(Plane, GeoPoint[], Membership...)`.
    pub fn intersects(
        &self,
        p: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        for shape in &self.shapes {
            if shape.intersects(p, notable_points, bounds) {
                return true;
            }
        }
        false
    }

    /// `getBounds(Bounds)`.
    pub fn get_bounds(&self, bounds: &mut dyn Bounds) {
        for shape in &self.shapes {
            shape.get_bounds(bounds);
        }
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        BasePlanetObject::hash_code(&self.planet_model)
            .wrapping_add(list_hash_code(&self.shapes, |s| s.java_hash_code()))
    }

    /// `equals(Object)` against another composite.
    pub fn equals(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && self.shapes.len() == other.shapes.len()
            && self
                .shapes
                .iter()
                .zip(&other.shapes)
                .all(|(a, b)| a.java_equals(b.as_any()))
    }

    /// `getShapes()`: an unmodifiable view.
    pub fn get_shapes(&self) -> &[Arc<T>] {
        &self.shapes
    }
}
