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

//! Port of `CellToBytesRefIterator50.java` (package
//! `org.apache.lucene.spatial.prefix`): for testing Lucene <= 5.0. Index
//! redundant prefixes for leaf cells. Fixed in LUCENE-4942.

use rucene::error::Result;
use rucene::util::{BytesRef, BytesRefIterator};
use rucene_spatial_extras::prefix::cell_to_bytes_ref_iterator::CellToBytesRefIteratorOps;
use rucene_spatial_extras::prefix::tree::cell::CellRef;
use rucene_spatial_extras::prefix::tree::cell_iterator::CellIterator;

/// `CellToBytesRefIterator50 extends CellToBytesRefIterator` (package-private).
pub struct CellToBytesRefIterator50 {
    /// `cellIter` (inherited).
    cell_iter: Option<Box<dyn CellIterator>>,
    /// `bytesRef` (inherited).
    bytes_ref: BytesRef,
    /// `repeatCell`.
    repeat_cell: Option<CellRef>,
}

impl CellToBytesRefIterator50 {
    /// `CellToBytesRefIterator50()`.
    pub fn new() -> Self {
        Self {
            cell_iter: None,
            bytes_ref: BytesRef::new(Vec::new()),
            repeat_cell: None,
        }
    }
}

impl CellToBytesRefIteratorOps for CellToBytesRefIterator50 {
    /// `reset(Iterator<Cell> cellIter)` (inherited).
    fn reset(&mut self, cell_iter: Box<dyn CellIterator>) {
        self.cell_iter = Some(cell_iter);
    }
}

impl BytesRefIterator for CellToBytesRefIterator50 {
    /// `next()`.
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if let Some(repeat_cell) = self.repeat_cell.take() {
            let reuse = std::mem::replace(&mut self.bytes_ref, BytesRef::new(Vec::new()));
            self.bytes_ref = repeat_cell.get_token_bytes_with_leaf(Some(reuse));
            return Ok(Some(self.bytes_ref.clone()));
        }
        let cell_iter = self
            .cell_iter
            .as_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: reset() was not called"));
        if !cell_iter.has_next() {
            return Ok(None);
        }
        let cell = cell_iter.next();
        let reuse = std::mem::replace(&mut self.bytes_ref, BytesRef::new(Vec::new()));
        self.bytes_ref = cell.get_token_bytes_no_leaf(Some(reuse));
        if cell.is_leaf() {
            self.repeat_cell = Some(cell);
        }
        Ok(Some(self.bytes_ref.clone()))
    }
}
