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

use std::any::TypeId;
use std::sync::Arc;

use s2_geometry::s2_cell_id::S2CellId;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::shape_factory::ShapeFactory;

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.S2ShapeFactory`
/// (`public interface S2ShapeFactory extends ShapeFactory`): shape factory
/// for spatial contexts that support S2 geometry. It is an extension of
/// spatial4j's `ShapeFactory`.
pub trait S2ShapeFactory: ShapeFactory {
    /// `getS2CellShape(S2CellId cellId)`: the shape of the S2 cell `cell_id`.
    fn get_s2_cell_shape(&self, cell_id: S2CellId) -> Arc<dyn Shape>;
}

/// `shapeFactory instanceof S2ShapeFactory ? (S2ShapeFactory) shapeFactory :
/// null` (`S2PrefixTree.java:73-76`).
///
/// Every factory that implements [`S2ShapeFactory`] answers spatial4j's
/// interface query ([`ShapeFactory::as_interface`]) for
/// `TypeId::of::<dyn S2ShapeFactory>()` with itself, so any implementation is
/// recognised, as Java's `instanceof` recognises it.
pub fn as_s2_shape_factory(
    shape_factory: &Arc<dyn ShapeFactory>,
) -> Option<Arc<dyn S2ShapeFactory>> {
    Arc::clone(shape_factory)
        .as_interface(TypeId::of::<dyn S2ShapeFactory>())
        .and_then(|view| view.downcast::<Arc<dyn S2ShapeFactory>>().ok())
        .map(|view| *view)
}
