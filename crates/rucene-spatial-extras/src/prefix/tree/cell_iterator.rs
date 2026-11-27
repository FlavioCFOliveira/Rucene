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

//! The types declared in `CellIterator.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use crate::prefix::tree::cell::{Cell, CellRef};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.CellIterator`
/// (`public abstract class CellIterator implements Iterator<Cell>`): an
/// iterator of cells with the extra method [`this_cell`](Self::this_cell).
///
/// Java's protected fields `nextCell` and `thisCell` are the
/// [`CellIteratorBase`] an implementation embeds; Java's `next()` is shared
/// by every subclass that does not override it, which here is
/// `CellIteratorBase::next` called by the implementation.
///
/// **Divergence.** `next()` past the end throws
/// `java.util.NoSuchElementException` in Java (`CellIterator.java:66-71`);
/// that is a broken caller invariant (every caller tests `hasNext()` first),
/// so the port panics with the exception's name instead of making every
/// iteration fallible.
pub trait CellIterator: Send + Sync {
    /// `hasNext()`.
    fn has_next(&mut self) -> bool;

    /// `next()`.
    ///
    /// # Panics
    ///
    /// `java.util.NoSuchElementException` when there is no next cell.
    fn next(&mut self) -> CellRef;

    /// `thisCell()`: the cell last returned by [`next`](Self::next). Only
    /// call it after `next()`, not after `hasNext()`.
    ///
    /// # Panics
    ///
    /// When there is no current cell (Java's `assert`, followed by the `null`
    /// it returns).
    fn this_cell(&self) -> CellRef;

    /// `nextFrom(Cell fromCell)`: advances to the first cell at or after
    /// `from_cell` (disregarding leaf status); `None` (Java `null`) when there
    /// is none.
    fn next_from(&mut self, from_cell: &dyn Cell) -> Option<CellRef> {
        loop {
            if !self.has_next() {
                return None;
            }
            let c = self.next(); // will update thisCell
            if c.compare_to_no_leaf(from_cell) >= 0 {
                return Some(c);
            }
        }
    }

    /// `remove()`: this method is implemented in some iterators as a hint
    /// not to descend into the current cell.
    fn remove(&mut self) {}
}

/// The protected fields of `CellIterator`, and its `next()` and
/// `thisCell()` bodies.
#[derive(Default)]
pub struct CellIteratorBase {
    /// `nextCell`: to be returned by `next()`, and cleared after.
    pub next_cell: Option<CellRef>,
    /// `thisCell`: see `next()` and `thisCell()`; cleared in `hasNext()`.
    pub this_cell: Option<CellRef>,
}

impl CellIteratorBase {
    /// The body of `CellIterator.next()` after `hasNext()` was checked by
    /// the caller: moves `nextCell` to `thisCell` and returns it.
    ///
    /// # Panics
    ///
    /// `java.util.NoSuchElementException` when `has_next` is `false`.
    pub fn next_after(&mut self, has_next: bool) -> CellRef {
        if self.next_cell.is_none() && !has_next {
            panic!("java.util.NoSuchElementException");
        }
        self.this_cell = self.next_cell.take();
        self.this_cell.clone().expect("hasNext() set nextCell")
    }

    /// The body of `CellIterator.thisCell()`.
    ///
    /// # Panics
    ///
    /// When `thisCell` is `null`: "Only call thisCell() after next(), not
    /// hasNext()".
    pub fn this_cell(&self) -> CellRef {
        self.this_cell
            .clone()
            .expect("Only call thisCell() after next(), not hasNext()")
    }
}

/// `next()` of a `CellIterator` subclass that does not override it: calls
/// `hasNext()` only when `nextCell` is `null`.
#[macro_export]
#[doc(hidden)]
macro_rules! cell_iterator_next {
    ($self:ident, $base:ident) => {{
        let has_next = $self.$base.next_cell.is_some() || $self.has_next();
        $self.$base.next_after(has_next)
    }};
}

/// An iterator over a list of cells: Java's `List<Cell>.iterator()`, which
/// `SpatialPrefixTree` implementations wrap or return where Java uses a
/// plain `Iterator<Cell>`.
pub(crate) struct ListCellIterator {
    cells: std::vec::IntoIter<CellRef>,
    peeked: Option<CellRef>,
    base: CellIteratorBase,
}

impl ListCellIterator {
    /// `cells.iterator()`.
    pub(crate) fn new(cells: Vec<CellRef>) -> Self {
        Self {
            cells: cells.into_iter(),
            peeked: None,
            base: CellIteratorBase::default(),
        }
    }
}

impl CellIterator for ListCellIterator {
    fn has_next(&mut self) -> bool {
        if self.peeked.is_none() {
            self.peeked = self.cells.next();
        }
        self.peeked.is_some()
    }

    fn next(&mut self) -> CellRef {
        if !self.has_next() {
            panic!("java.util.NoSuchElementException");
        }
        self.base.next_cell = self.peeked.take();
        self.base.next_after(true)
    }

    fn this_cell(&self) -> CellRef {
        self.base.this_cell()
    }
}
