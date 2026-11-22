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

//! The types declared in `FilterCellIterator.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::sync::Arc;

use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::{CellIterator, CellIteratorBase};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.FilterCellIterator`
/// (package-private `class FilterCellIterator extends CellIterator`): a
/// filtering iterator of cells. Does not include cells that are disjoint
/// from the shape filter, and sets the shape relation of the cells it
/// returns (a `WITHIN` cell becomes a leaf).
pub(crate) struct FilterCellIterator {
    base: CellIteratorBase,
    base_iter: Box<dyn CellIterator>,
    shape_filter: Option<Arc<dyn Shape>>,
}

impl FilterCellIterator {
    /// `FilterCellIterator(Iterator<Cell> baseIter, Shape shapeFilter)`.
    pub(crate) fn new(base_iter: Box<dyn CellIterator>, shape_filter: Option<Arc<dyn Shape>>) -> Self {
        Self {
            base: CellIteratorBase::default(),
            base_iter,
            shape_filter,
        }
    }
}

impl CellIterator for FilterCellIterator {
    fn has_next(&mut self) -> bool {
        self.base.this_cell = None;
        if self.base.next_cell.is_some() {
            // calling hasNext twice in a row
            return true;
        }
        while self.base_iter.has_next() {
            let next_cell = self.base_iter.next();
            self.base.next_cell = Some(Arc::clone(&next_cell));
            match &self.shape_filter {
                None => return true,
                Some(shape_filter) => {
                    let rel = next_cell.get_shape().relate(&**shape_filter);
                    if rel.intersects() {
                        next_cell.set_shape_rel(Some(rel));
                        if rel == SpatialRelation::Within {
                            next_cell.set_leaf();
                        }
                        return true;
                    }
                }
            }
        }
        false
    }

    fn next(&mut self) -> CellRef {
        crate::cell_iterator_next!(self, base)
    }

    fn this_cell(&self) -> CellRef {
        self.base.this_cell()
    }
}
