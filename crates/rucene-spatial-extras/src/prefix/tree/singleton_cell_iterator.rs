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

//! The types declared in `SingletonCellIterator.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::{CellIterator, CellIteratorBase};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.SingletonCellIterator`
/// (package-private `class SingletonCellIterator extends CellIterator`): a
/// singleton (one-element) cell iterator.
pub(crate) struct SingletonCellIterator {
    base: CellIteratorBase,
}

impl SingletonCellIterator {
    /// `SingletonCellIterator(Cell cell)`.
    pub(crate) fn new(cell: CellRef) -> Self {
        Self {
            base: CellIteratorBase {
                next_cell: Some(cell), // preload nextCell
                this_cell: None,
            },
        }
    }
}

impl CellIterator for SingletonCellIterator {
    fn has_next(&mut self) -> bool {
        self.base.this_cell = None;
        self.base.next_cell.is_some()
    }

    fn next(&mut self) -> CellRef {
        crate::cell_iterator_next!(self, base)
    }

    fn this_cell(&self) -> CellRef {
        self.base.this_cell()
    }
}
