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

//! The types declared in `S2ShapeFactory.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::sync::Arc;

use s2_geometry::s2_cell_id::S2CellId;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::shape_factory::ShapeFactory;

use crate::spatial4j::geo3d_shape_factory::Geo3dShapeFactory;

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.S2ShapeFactory`
/// (`public interface S2ShapeFactory extends ShapeFactory`): shape factory
/// for spatial contexts that support S2 geometry. It is an extension of
/// spatial4j's `ShapeFactory`.
pub trait S2ShapeFactory: ShapeFactory {
    /// `getS2CellShape(S2CellId cellId)`: the shape of the S2 cell `cell_id`.
    fn get_s2_cell_shape(&self, cell_id: S2CellId) -> Arc<dyn Shape>;
}

/// `shapeFactory instanceof S2ShapeFactory ? (S2ShapeFactory) shapeFactory :
/// null`.
///
/// **Divergence.** Java tests the runtime class of any factory
/// (`S2PrefixTree.java:73-76`); a Rust `dyn ShapeFactory` cannot be narrowed
/// to another trait, so the factory is matched against the implementations
/// Lucene ships, which is [`Geo3dShapeFactory`] only.
pub fn as_s2_shape_factory(shape_factory: &dyn ShapeFactory) -> Option<&dyn S2ShapeFactory> {
    shape_factory
        .as_any()
        .downcast_ref::<Geo3dShapeFactory>()
        .map(|f| f as &dyn S2ShapeFactory)
}
