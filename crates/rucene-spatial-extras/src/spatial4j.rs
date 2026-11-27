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

//! The Java package `org.apache.lucene.spatial.spatial4j`: the spatial4j
//! bridge to spatial3d (`Geo3d*`).

pub mod geo3d_binary_codec;
pub mod geo3d_circle_shape;
pub mod geo3d_distance_calculator;
pub mod geo3d_point_shape;
pub mod geo3d_rectangle_shape;
pub mod geo3d_shape;
pub mod geo3d_shape_factory;
pub mod geo3d_spatial_context_factory;

use std::sync::Once;

use spatial4j::context::spatial_context_factory::register_class;

/// Registers the classes of this package in spatial4j's class lookup
/// (`SpatialContextFactory`'s `Class.forName`) under their Java binary names:
/// `Geo3dSpatialContextFactory`, `Geo3dShapeFactory` and `Geo3dBinaryCodec`.
///
/// **Divergence.** In Java these classes are found by reflection as soon as
/// they are on the classpath (`SpatialContextFactory.makeSpatialContext`,
/// `initField` of `shapeFactoryClass` / `binaryCodecClass`); Rust has no
/// reflection nor static initializers, so spatial4j's port looks classes up
/// in a registry that this function fills. It runs once, from
/// [`Geo3dSpatialContextFactory::new`](geo3d_spatial_context_factory::Geo3dSpatialContextFactory::new)
/// and from the spatial-extras entry points that look classes up by name;
/// a caller that names these classes to spatial4j before using this crate
/// calls it first.
pub fn register_classes() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        register_class(geo3d_spatial_context_factory::GEO3D_SPATIAL_CONTEXT_FACTORY_CLASS);
        register_class(geo3d_spatial_context_factory::GEO3D_SHAPE_FACTORY_CLASS);
        register_class(geo3d_spatial_context_factory::GEO3D_BINARY_CODEC_CLASS);
    });
}
