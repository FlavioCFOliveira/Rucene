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

//! The types declared in `CellToBytesRefIterator.java`, of the Java package
//! `org.apache.lucene.spatial.prefix`.

use rucene::error::Result;
use rucene::util::{BytesRef, BytesRefIterator};

use crate::prefix::tree::cell_iterator::CellIterator;

/// Equivalent to `org.apache.lucene.spatial.prefix.CellToBytesRefIterator`
/// (`public class CellToBytesRefIterator implements BytesRefIterator`): a
/// reset'able `BytesRefIterator` wrapper around an `Iterator` of `Cell`s.
///
/// Java's `TermQueryPrefixTreeStrategy` overrides `next()` in an anonymous
/// subclass to emit the bytes without the leaf; that subclass is
/// [`no_leaf`](Self::no_leaf).
pub struct CellToBytesRefIterator {
    /// `cellIter`.
    pub cell_iter: Option<Box<dyn CellIterator>>,
    /// `bytesRef`.
    pub bytes_ref: BytesRef,
    with_leaf: bool,
}

impl Default for CellToBytesRefIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl CellToBytesRefIterator {
    /// `CellToBytesRefIterator()`.
    pub fn new() -> Self {
        Self {
            cell_iter: None,
            bytes_ref: BytesRef::new(Vec::new()),
            with_leaf: true,
        }
    }

    /// The anonymous subclass of `TermQueryPrefixTreeStrategy`, which
    /// ensures we don't have leaves, as that strategy doesn't handle them.
    pub fn no_leaf() -> Self {
        Self {
            with_leaf: false,
            ..Self::new()
        }
    }

    /// `reset(Iterator<Cell> cellIter)`.
    pub fn reset(&mut self, cell_iter: Box<dyn CellIterator>) {
        self.cell_iter = Some(cell_iter);
    }
}

impl BytesRefIterator for CellToBytesRefIterator {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        let cell_iter = self
            .cell_iter
            .as_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: reset() was not called"));
        if !cell_iter.has_next() {
            return Ok(None);
        }
        let cell = cell_iter.next();
        let reuse = std::mem::replace(&mut self.bytes_ref, BytesRef::new(Vec::new()));
        self.bytes_ref = if self.with_leaf {
            cell.get_token_bytes_with_leaf(Some(reuse))
        } else {
            cell.get_token_bytes_no_leaf(Some(reuse))
        };
        Ok(Some(self.bytes_ref.clone()))
    }
}
