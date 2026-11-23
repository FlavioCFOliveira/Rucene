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

//! Port of `SortedSetDocValuesReaderState.java`, of the Java package
//! `org.apache.lucene.facet.sortedset`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::multi_doc_values::WithMapping;
use rucene::index::{IndexReader, SortedSetDocValues};
use rucene::util::{Accountable, FixedBitSet};

use crate::facets_config::FacetsConfig;

/// Invalid ordinal const.
///
/// Equivalent to `SortedSetDocValuesReaderState.INVALID_ORDINAL`.
pub const INVALID_ORDINAL: i32 = -1;

/// Holds start/end range of ords, which maps to one dimension. Only used for
/// flat hierarchies.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.sortedset.SortedSetDocValuesReaderState.OrdRange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrdRange {
    /// Start of range, inclusive.
    pub start: i32,
    /// End of range, inclusive.
    pub end: i32,
}

impl OrdRange {
    /// The record's canonical constructor.
    pub fn new(start: i32, end: i32) -> Self {
        Self { start, end }
    }

    /// Equivalent to `start()`.
    pub fn start(&self) -> i32 {
        self.start
    }

    /// Equivalent to `end()`.
    pub fn end(&self) -> i32 {
        self.end
    }

    /// Iterates from start to end ord (inclusive).
    ///
    /// Equivalent to `iterator()`.
    pub fn iterator(&self) -> OrdRangeIterator {
        OrdRangeIterator {
            current: self.start,
            end: self.end,
        }
    }
}

/// The anonymous `PrimitiveIterator.OfInt` of `OrdRange.iterator()`.
#[derive(Debug, Clone)]
pub struct OrdRangeIterator {
    current: i32,
    end: i32,
}

impl OrdRangeIterator {
    /// Equivalent to `nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        if self.current > self.end {
            return INVALID_ORDINAL;
        }
        let current = self.current;
        self.current += 1;
        current
    }

    /// Equivalent to `hasNext()`.
    pub fn has_next(&self) -> bool {
        self.current <= self.end
    }
}

/// Holds children and sibling information for a single dimension. Only used
/// with hierarchical dimensions.
///
/// Equivalent to
/// `org.apache.lucene.facet.sortedset.SortedSetDocValuesReaderState.DimTree`.
#[derive(Debug, Clone)]
pub struct DimTree {
    has_children: Arc<FixedBitSet>,
    /// See `FacetsConfig.DimTree.siblings` in Java.
    siblings: Arc<[i32]>,
    /// The first ord of the dimension.
    pub dim_start_ord: i32,
}

impl DimTree {
    /// Sole constructor.
    ///
    /// Equivalent to `DimTree(int, List, List)`.
    pub fn new(dim_start_ord: i32, sibling: Vec<i32>, has_children: Vec<bool>) -> Result<Self> {
        if sibling.len() != has_children.len() {
            return Err(LuceneError::illegal_argument(format!(
                "Sibling list and children list must have the same size. Got sibling list size of {} and child list size of {}",
                sibling.len(),
                has_children.len()
            )));
        }
        let mut bits = FixedBitSet::new(has_children.len());
        for (i, &has) in has_children.iter().enumerate() {
            if has {
                debug_assert!(i < sibling.len() - 1);
                bits.set(i);
            }
            debug_assert!(sibling[i] < sibling.len() as i32);
        }
        Ok(Self {
            has_children: Arc::new(bits),
            siblings: Arc::from(sibling),
            dim_start_ord,
        })
    }

    /// Iterates through all first level children of dimension.
    ///
    /// Equivalent to `iterator()`.
    pub fn iterator(&self) -> DimTreeIterator {
        self.iterator_from(self.dim_start_ord)
    }

    /// Iterates through all children of given `path_ord`.
    ///
    /// Equivalent to `iterator(int)`.
    pub fn iterator_from(&self, path_ord: i32) -> DimTreeIterator {
        DimTreeIterator {
            tree: self.clone(),
            at_start: true,
            current_ord: path_ord - self.dim_start_ord,
        }
    }
}

/// The anonymous `PrimitiveIterator.OfInt` of `DimTree.iterator(int)`.
#[derive(Debug, Clone)]
pub struct DimTreeIterator {
    tree: DimTree,
    at_start: bool,
    current_ord: i32,
}

impl DimTreeIterator {
    fn has_children_len(&self) -> i32 {
        self.tree.has_children.length() as i32
    }

    /// Equivalent to `nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        if self.at_start {
            if self.current_ord < 0 || self.current_ord >= self.has_children_len() {
                return INVALID_ORDINAL;
            }
            self.at_start = false;
            if self.tree.has_children.get(self.current_ord as usize) {
                self.current_ord += 1;
                self.current_ord + self.tree.dim_start_ord
            } else {
                INVALID_ORDINAL
            }
        } else {
            self.current_ord = self.tree.siblings[self.current_ord as usize];
            self.current_ord + self.tree.dim_start_ord
        }
    }

    /// Equivalent to `hasNext()`.
    pub fn has_next(&self) -> bool {
        if self.at_start {
            if self.current_ord < 0 || self.current_ord >= self.has_children_len() {
                return false;
            }
            self.tree.has_children.get(self.current_ord as usize)
        } else {
            self.tree.siblings[self.current_ord as usize] != INVALID_ORDINAL
        }
    }
}

/// A child-ordinal iterator: Java's `PrimitiveIterator.OfInt` from an
/// `OrdRange` or a `DimTree`.
#[derive(Debug, Clone)]
pub enum ChildOrdIterator {
    /// `OrdRange.iterator()`.
    Range(OrdRangeIterator),
    /// `DimTree.iterator(int)`.
    Tree(DimTreeIterator),
}

impl ChildOrdIterator {
    /// Equivalent to `hasNext()`.
    pub fn has_next(&self) -> bool {
        match self {
            ChildOrdIterator::Range(it) => it.has_next(),
            ChildOrdIterator::Tree(it) => it.has_next(),
        }
    }

    /// Equivalent to `next()` / `nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        match self {
            ChildOrdIterator::Range(it) => it.next_int(),
            ChildOrdIterator::Tree(it) => it.next_int(),
        }
    }
}

/// Wraps a `IndexReader` and resolves ords using existing `SortedSetDocValues`
/// APIs without a separate taxonomy index.
///
/// Equivalent to `org.apache.lucene.facet.sortedset.SortedSetDocValuesReaderState`.
/// `getDocValues()` returns the merged view together with the
/// `OrdinalMap` Java reads through `instanceof MultiSortedSetDocValues`.
pub trait SortedSetDocValuesReaderState: Accountable + Send + Sync + std::fmt::Display {
    /// Return top-level doc values, and the ordinal map when they are a
    /// `MultiSortedSetDocValues`; `None` for Java's `null`.
    ///
    /// Equivalent to `getDocValues()`.
    fn get_doc_values(&self) -> Result<Option<WithMapping<dyn SortedSetDocValues>>>;

    /// Indexed field we are reading.
    ///
    /// Equivalent to `getField()`.
    fn get_field(&self) -> &str;

    /// Returns top-level index reader.
    ///
    /// Equivalent to `getReader()`.
    fn get_reader(&self) -> &Arc<dyn IndexReader>;

    /// Number of unique labels.
    ///
    /// Equivalent to `getSize()`.
    fn get_size(&self) -> i32;

    /// Returns the associated facets config.
    ///
    /// Equivalent to `getFacetsConfig()`.
    fn get_facets_config(&self) -> &Arc<FacetsConfig>;

    /// Returns the `OrdRange` for this dimension.
    ///
    /// Equivalent to `getOrdRange(String)`.
    fn get_ord_range(&self, dim: &str) -> Result<Option<OrdRange>>;

    /// Returns mapping from prefix to `OrdRange`.
    ///
    /// Equivalent to `getPrefixToOrdRange()`.
    fn get_prefix_to_ord_range(&self) -> Vec<(String, OrdRange)>;

    /// Returns the `DimTree` for the given dimension.
    ///
    /// Equivalent to `getDimTree(String)`.
    fn get_dim_tree(&self, dim: &str) -> Result<Option<DimTree>>;

    /// Returns a list of all dimensions.
    ///
    /// Equivalent to `getDims()`.
    fn get_dims(&self) -> Vec<String>;
}
