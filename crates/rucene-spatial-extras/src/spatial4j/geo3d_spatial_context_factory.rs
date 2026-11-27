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

//! The types declared in `Geo3dSpatialContextFactory.java`, of the Java
//! package `org.apache.lucene.spatial.spatial4j`.

use spatial4j::java_map::StringMap;
use std::any::Any;
use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene_spatial3d::geom::planet_model::{self, PlanetModel};
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::{
    init_calculator_default, init_default, JavaClass, JavaClassKind, SpatialContextFactory,
    SpatialContextFactoryOps,
};

use crate::spatial4j::geo3d_binary_codec::Geo3dBinaryCodec;
use crate::spatial4j::geo3d_distance_calculator::Geo3dDistanceCalculator;
use crate::spatial4j::geo3d_shape_factory::Geo3dShapeFactory;

/// `DEFAULT_PLANET_MODEL`: the default planet model.
static DEFAULT_PLANET_MODEL: LazyLock<Arc<PlanetModel>> =
    LazyLock::new(|| Arc::clone(&planet_model::SPHERE));

/// `Geo3dSpatialContextFactory.class`.
pub const GEO3D_SPATIAL_CONTEXT_FACTORY_CLASS: JavaClass = JavaClass {
    name: "org.apache.lucene.spatial.spatial4j.Geo3dSpatialContextFactory",
    kind: JavaClassKind::SpatialContextFactory(|| Box::new(Geo3dSpatialContextFactory::new())),
};

/// `Geo3dShapeFactory.class`.
pub const GEO3D_SHAPE_FACTORY_CLASS: JavaClass = JavaClass {
    name: "org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory",
    kind: JavaClassKind::ShapeFactory(|ctx, factory| {
        Ok(Arc::new(Geo3dShapeFactory::new(ctx, factory)?))
    }),
};

/// `Geo3dBinaryCodec.class`.
pub const GEO3D_BINARY_CODEC_CLASS: JavaClass = JavaClass {
    name: "org.apache.lucene.spatial.spatial4j.Geo3dBinaryCodec",
    kind: JavaClassKind::BinaryCodec(|ctx, factory| {
        Ok(Arc::new(Geo3dBinaryCodec::new(ctx, factory)?))
    }),
};

/// `(Geo3dSpatialContextFactory) factory`.
///
/// # Errors
///
/// `ClassCastException` for another factory class.
pub(crate) fn geo3d_factory(
    factory: &dyn SpatialContextFactoryOps,
) -> Result<&Geo3dSpatialContextFactory> {
    factory.as_any().downcast_ref::<Geo3dSpatialContextFactory>().ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.ClassCastException",
            "class cannot be cast to class org.apache.lucene.spatial.spatial4j.Geo3dSpatialContextFactory",
        )
    })
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dSpatialContextFactory`
/// (`public class Geo3dSpatialContextFactory extends SpatialContextFactory`):
/// geo3d spatial context factory class.
#[derive(Debug, Clone)]
pub struct Geo3dSpatialContextFactory {
    /// The `SpatialContextFactory` fields.
    pub base: SpatialContextFactory,
    /// `planetModel`: the planet model (`None` for `null`).
    pub planet_model: Option<Arc<PlanetModel>>,
}

impl Default for Geo3dSpatialContextFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl Geo3dSpatialContextFactory {
    /// `Geo3dSpatialContextFactory()`: empty constructor.
    pub fn new() -> Self {
        crate::spatial4j::register_classes();
        let mut base = SpatialContextFactory::new();
        base.binary_codec_class = GEO3D_BINARY_CODEC_CLASS;
        base.shape_factory_class = GEO3D_SHAPE_FACTORY_CLASS;
        Self {
            base,
            planet_model: None,
        }
    }

    /// `planetModel`, which `newSpatialContext()` defaults.
    ///
    /// # Panics
    ///
    /// When it is `null` (Java's `NullPointerException` on its use).
    pub fn planet_model(&self) -> Arc<PlanetModel> {
        Arc::clone(
            self.planet_model
                .as_ref()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException: planetModel is null")),
        )
    }

    /// `initPlanetModel(Map<String, String> args)`.
    ///
    /// # Errors
    ///
    /// `RuntimeException("Unknown planet model: ...")`.
    pub fn init_planet_model(&mut self, args: &dyn StringMap) -> Result<()> {
        match args.get("planetModel")? {
            Some(planet_model) => {
                if planet_model.eq_ignore_ascii_case("sphere") {
                    self.planet_model = Some(Arc::clone(&planet_model::SPHERE));
                } else if planet_model.eq_ignore_ascii_case("wgs84") {
                    self.planet_model = Some(Arc::clone(&planet_model::WGS84));
                } else if planet_model.eq_ignore_ascii_case("clarke1866") {
                    self.planet_model = Some(Arc::clone(&planet_model::CLARKE_1866));
                } else {
                    return Err(LuceneError::unchecked(
                        "java.lang.RuntimeException",
                        format!("Unknown planet model: {planet_model}"),
                    ));
                }
            }
            None => self.planet_model = Some(Arc::clone(&DEFAULT_PLANET_MODEL)),
        }
        Ok(())
    }
}

impl SpatialContextFactoryOps for Geo3dSpatialContextFactory {
    fn base(&self) -> &SpatialContextFactory {
        &self.base
    }

    fn base_mut(&mut self) -> &mut SpatialContextFactory {
        &mut self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn new_spatial_context(&mut self) -> Result<Arc<SpatialContext>> {
        if self.planet_model.is_none() {
            self.planet_model = Some(Arc::clone(&DEFAULT_PLANET_MODEL));
        }
        if self.base.dist_calc.is_none() {
            self.base.dist_calc = Some(Arc::new(Geo3dDistanceCalculator::new(self.planet_model())));
        }
        SpatialContext::new(self)
    }

    fn init(&mut self, args: Arc<dyn StringMap>) -> Result<()> {
        self.init_planet_model(&*args)?;
        init_default(self, args)
    }

    fn init_calculator(&mut self) -> Result<()> {
        let calc_str = self.base.arg("distCalculator")?;
        match calc_str {
            None => Ok(()),
            Some(calc_str) if calc_str == "geo3d" => {
                self.base.dist_calc =
                    Some(Arc::new(Geo3dDistanceCalculator::new(self.planet_model())));
                Ok(())
            }
            Some(_) => init_calculator_default(self), // some other distance calculator
        }
    }

    fn as_ops(&self) -> &dyn SpatialContextFactoryOps {
        self
    }

    fn as_ops_mut(&mut self) -> &mut dyn SpatialContextFactoryOps {
        self
    }
}
