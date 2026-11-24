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

//! The types declared in `CellCanPrune.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use crate::prefix::tree::cell::Cell;

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.CellCanPrune`
/// (`public interface CellCanPrune extends Cell`): a cell whose sub-cells
/// can be counted, so that a fully covered cell is indexed as a leaf instead
/// of all its children.
pub trait CellCanPrune: Cell {
    /// `getSubCellsSize()`: the number of sub-cells of this cell.
    fn get_sub_cells_size(&self) -> i32;
}
