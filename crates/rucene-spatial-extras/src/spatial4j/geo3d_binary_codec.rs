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

//! The types declared in `Geo3dBinaryCodec.java`, of the Java package
//! `org.apache.lucene.spatial.spatial4j`.

use std::any::Any;
use std::io::{Read, Write};
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::planet_model::PlanetModel;
use rucene_spatial3d::geom::serializable_object::{self, SerializableObject};
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;
use spatial4j::io::binary_codec::BinaryCodec;
use spatial4j::shape::circle::Circle;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::shape_collection::ShapeCollection;

use crate::jdk::unsupported;
use crate::spatial4j::geo3d_circle_shape::Geo3dCircleShape;
use crate::spatial4j::geo3d_point_shape::Geo3dPointShape;
use crate::spatial4j::geo3d_rectangle_shape::Geo3dRectangleShape;
use crate::spatial4j::geo3d_shape::{geo3d_shape_of, Geo3dShape};
use crate::spatial4j::geo3d_spatial_context_factory::geo3d_factory;

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dBinaryCodec`
/// (`public class Geo3dBinaryCodec extends BinaryCodec`): geo3d binary
/// codec for spatial4j, which reads and writes the spatial3d serialization
/// of the wrapped shapes.
///
/// The context owns its codec, so the codec refers back to it weakly (as
/// spatial4j's `BinaryCodec` does).
pub struct Geo3dBinaryCodec {
    ctx: Weak<SpatialContext>,
    planet_model: Arc<PlanetModel>,
}

impl Geo3dBinaryCodec {
    /// `Geo3dBinaryCodec(SpatialContext ctx, SpatialContextFactory factory)`.
    ///
    /// # Errors
    ///
    /// `ClassCastException` when `factory` is not a
    /// `Geo3dSpatialContextFactory`.
    pub fn new(ctx: &Arc<SpatialContext>, factory: &dyn SpatialContextFactoryOps) -> Result<Self> {
        Ok(Self {
            ctx: Arc::downgrade(ctx),
            planet_model: geo3d_factory(factory)?.planet_model(),
        })
    }

    fn read_object(&self, data_input: &mut dyn Read) -> Result<Arc<dyn SerializableObject>> {
        serializable_object::read_object_with_model(&self.planet_model, data_input)
    }
}

/// `"...: " + serializableObject.getClass()`.
fn not_supported(what: &str, o: &dyn SerializableObject) -> LuceneError {
    LuceneError::illegal_argument(format!("trying to read a not supported {what}: class {}", o.class_name()))
}

impl BinaryCodec for Geo3dBinaryCodec {
    fn ctx(&self) -> Arc<SpatialContext> {
        self.ctx.upgrade().expect("the SpatialContext of a Geo3dBinaryCodec is alive")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn read_shape(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Shape>> {
        let serializable_object = self.read_object(data_input)?;
        let class = serializable_object.class_name();
        match serializable_object.into_geo_area_shape() {
            Some(shape) => Ok(Arc::new(Geo3dShape::new(shape, self.ctx()))),
            None => Err(LuceneError::illegal_argument(format!(
                "trying to read a not supported shape: class {class}"
            ))),
        }
    }

    fn write_shape(&self, data_output: &mut dyn Write, s: &dyn Shape) -> Result<()> {
        match geo3d_shape_of(s) {
            Some(geo_area_shape) => {
                let shape: Arc<dyn GeoAreaShape> = geo_area_shape.shape();
                serializable_object::write_object(data_output, &*shape)
            }
            None => Err(LuceneError::illegal_argument(format!(
                "trying to write a not supported shape: {}",
                s.java_class_name()
            ))),
        }
    }

    fn read_point(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Point>> {
        let serializable_object = self.read_object(data_input)?;
        match Arc::clone(&serializable_object).into_geo_point_shape() {
            Some(shape) => Ok(Geo3dPointShape::new(shape, self.ctx())),
            None => Err(not_supported("point shape", &*serializable_object)),
        }
    }

    fn write_point(&self, data_output: &mut dyn Write, pt: &dyn Point) -> Result<()> {
        self.write_shape(data_output, pt.as_shape())
    }

    fn read_rect(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Rectangle>> {
        let serializable_object = self.read_object(data_input)?;
        match Arc::clone(&serializable_object).into_geo_b_box() {
            Some(shape) => Ok(Geo3dRectangleShape::new(shape, self.ctx())),
            None => Err(not_supported("rectangle shape", &*serializable_object)),
        }
    }

    fn write_rect(&self, data_output: &mut dyn Write, r: &dyn Rectangle) -> Result<()> {
        self.write_shape(data_output, r.as_shape())
    }

    fn read_circle(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Circle>> {
        let serializable_object = self.read_object(data_input)?;
        match Arc::clone(&serializable_object).into_geo_circle() {
            Some(shape) => Ok(Geo3dCircleShape::new(shape, self.ctx())),
            None => Err(not_supported("circle shape", &*serializable_object)),
        }
    }

    fn write_circle(&self, data_output: &mut dyn Write, c: &dyn Circle) -> Result<()> {
        self.write_shape(data_output, c.as_shape())
    }

    fn read_collection(&self, _data_input: &mut dyn Read) -> Result<Arc<dyn Shape>> {
        Err(unsupported())
    }

    fn write_collection(&self, _data_output: &mut dyn Write, _col: &ShapeCollection) -> Result<()> {
        Err(unsupported())
    }
}
