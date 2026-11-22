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

//! The types declared in `ContainsPrefixTreeQuery.java`, of the Java package
//! `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{LeafReaderContext, SeekStatus};
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::{IndexSearcher, Query, QueryVisitor, ScoreMode, Weight};
use rucene::util::misc::SentinelIntSet;
use rucene::util::{Accountable, Bits, BytesRef, RamUsageEstimator};
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::abstract_prefix_tree_query::{
    create_weight, AbstractPrefixTreeQuery, BaseTermsEnumTraverser, PrefixTreeQuery, TreeDocIdSet,
};
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;

/// Equivalent to `org.apache.lucene.spatial.prefix.ContainsPrefixTreeQuery`
/// (`public class ContainsPrefixTreeQuery extends AbstractPrefixTreeQuery`):
/// finds docs where its indexed shape `CONTAINS` the query shape. For use
/// on `RecursivePrefixTreeStrategy`.
#[derive(Clone, Debug)]
pub struct ContainsPrefixTreeQuery {
    base: AbstractPrefixTreeQuery,
    /// `multiOverlappingIndexedShapes`: if the spatial data for a document
    /// is comprised of multiple overlapping or adjacent parts, it might fail
    /// to match a query shape when doing the CONTAINS predicate when the
    /// sum of those shapes contain the query shape but none do
    /// individually. Set this to false to increase performance if you don't
    /// care about that circumstance (such as if your indexed data doesn't
    /// even have such conditions).
    pub(crate) multi_overlapping_indexed_shapes: bool,
}

impl ContainsPrefixTreeQuery {
    /// `ContainsPrefixTreeQuery(Shape queryShape, String fieldName,
    /// SpatialPrefixTree grid, int detailLevel, boolean
    /// multiOverlappingIndexedShapes)`.
    pub fn new(
        query_shape: Arc<dyn Shape>,
        field_name: &str,
        grid: Arc<dyn SpatialPrefixTree>,
        detail_level: i32,
        multi_overlapping_indexed_shapes: bool,
    ) -> Self {
        Self {
            base: AbstractPrefixTreeQuery::new(query_shape, field_name, grid, detail_level),
            multi_overlapping_indexed_shapes,
        }
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.ContainsPrefixTreeQuery.ContainsVisitor`
/// (`private class ContainsVisitor extends BaseTermsEnumTraverser`).
pub(crate) struct ContainsVisitor<'q> {
    t: BaseTermsEnumTraverser,
    q: &'q ContainsPrefixTreeQuery,
    seek_term: BytesRef,        // temp; see seek()
    this_term: Option<BytesRef>, // current term in termsEnum
    indexed_cell: Option<CellRef>, // the cell wrapper around thisTerm
}

impl<'q> ContainsVisitor<'q> {
    /// `ContainsVisitor(LeafReaderContext context)`.
    fn new(q: &'q ContainsPrefixTreeQuery, context: &LeafReaderContext) -> Result<Self> {
        let mut v = Self {
            t: BaseTermsEnumTraverser::new(context, &q.base.field_name)?,
            q,
            seek_term: BytesRef::new(Vec::new()),
            this_term: None,
            indexed_cell: None,
        };
        if v.t.terms_enum.is_some() {
            v.next_term()?; // advance to first
        }
        Ok(v)
    }

    fn indexed_cell(&self) -> &CellRef {
        self.indexed_cell.as_ref().expect("set with thisTerm")
    }

    /// `visit(Cell cell, Bits acceptContains)`: this is the primary
    /// algorithm; recursive. Returns null if finds none.
    fn visit(&mut self, cell: &CellRef, accept_contains: Option<Rc<SmallDocSet>>) -> Result<Option<Rc<SmallDocSet>>> {
        if self.this_term.is_none() {
            // signals all done
            return Ok(None);
        }
        let mut accept_contains = accept_contains;

        // Get the AND of all child results (into combinedSubResults)
        let mut combined_sub_results: Option<Rc<SmallDocSet>> = None;
        //   Optimization: use null subCellsFilter when we know cell is within the query shape.
        let mut sub_cells_filter: Option<Arc<dyn Shape>> = Some(Arc::clone(&self.q.base.query_shape));
        if cell.get_level() != 0
            && (cell.get_shape_rel().is_none() || cell.get_shape_rel() == Some(SpatialRelation::Within))
        {
            sub_cells_filter = None;
            debug_assert!(cell.get_shape().relate(&*self.q.base.query_shape) == SpatialRelation::Within);
        }
        let mut sub_cells = cell.get_next_level_cells(sub_cells_filter);
        while sub_cells.has_next() {
            let sub_cell = sub_cells.next();
            if !self.seek(&sub_cell)? {
                combined_sub_results = None;
            } else if sub_cell.get_level() == self.q.base.detail_level {
                combined_sub_results = self.get_docs(&sub_cell, accept_contains.as_deref())?;
            } else if !self.q.multi_overlapping_indexed_shapes && sub_cell.get_shape_rel() == Some(SpatialRelation::Within) {
                combined_sub_results = self.get_leaf_docs(&sub_cell, accept_contains.as_deref())?;
            } else {
                // OR the leaf docs with all child results
                let leaf_docs = self.get_leaf_docs(&sub_cell, accept_contains.as_deref())?;
                let sub_docs = self.visit(&sub_cell, accept_contains.clone())?; // recursion
                combined_sub_results = Self::union(leaf_docs, sub_docs);
            }

            if combined_sub_results.is_none() {
                break;
            }
            accept_contains = combined_sub_results.clone(); // has the 'AND' effect on next iteration
        }

        Ok(combined_sub_results)
    }

    /// `seek(Cell cell)`.
    fn seek(&mut self, cell: &CellRef) -> Result<bool> {
        if self.this_term.is_none() {
            return Ok(false);
        }
        let compare = self.indexed_cell().compare_to_no_leaf(&**cell);
        if compare > 0 {
            Ok(false) // leap-frog effect
        } else if compare == 0 {
            Ok(true) // already there!
        } else {
            // compare > 0
            // seek!
            let reuse = std::mem::replace(&mut self.seek_term, BytesRef::new(Vec::new()));
            self.seek_term = cell.get_token_bytes_no_leaf(Some(reuse));
            let seek_status = self.t.terms_enum().seek_ceil(&self.seek_term)?;
            if seek_status == SeekStatus::END {
                self.this_term = None; // all done
                return Ok(false);
            }
            let term = self.t.terms_enum().term()?.cloned();
            self.read_this_term(term);
            if seek_status == SeekStatus::FOUND {
                return Ok(true);
            }
            Ok(self.indexed_cell().is_leaf() && self.indexed_cell().compare_to_no_leaf(&**cell) == 0)
        }
    }

    /// `getDocs(Cell cell, Bits acceptContains)`: get prefix & leaf docs at
    /// this cell.
    fn get_docs(&mut self, cell: &CellRef, accept_contains: Option<&SmallDocSet>) -> Result<Option<Rc<SmallDocSet>>> {
        debug_assert_eq!(self.indexed_cell().compare_to_no_leaf(&**cell), 0);
        // called when we've reached detailLevel.
        if self.indexed_cell().is_leaf() {
            // only a leaf
            let result = self.collect_docs(accept_contains)?;
            self.next_term()?;
            Ok(result)
        } else {
            let docs_at_prefix = self.collect_docs(accept_contains)?;
            if !self.next_term()? {
                return Ok(docs_at_prefix);
            }
            // collect leaf too
            if self.indexed_cell().is_leaf() && self.indexed_cell().compare_to_no_leaf(&**cell) == 0 {
                let docs_at_leaf = self.collect_docs(accept_contains)?;
                self.next_term()?;
                Ok(Self::union(docs_at_prefix, docs_at_leaf))
            } else {
                Ok(docs_at_prefix)
            }
        }
    }

    /// `getLeafDocs(Cell cell, Bits acceptContains)`: gets docs on the leaf
    /// of the given cell, `None` if there aren't any.
    fn get_leaf_docs(&mut self, cell: &CellRef, accept_contains: Option<&SmallDocSet>) -> Result<Option<Rc<SmallDocSet>>> {
        debug_assert_eq!(self.indexed_cell().compare_to_no_leaf(&**cell), 0);
        // Advance past prefix if we're at a prefix; return null if no leaf
        if !self.indexed_cell().is_leaf()
            && (!self.next_term()?
                || !self.indexed_cell().is_leaf()
                || self.indexed_cell().get_level() != cell.get_level())
        {
            return Ok(None);
        }
        let result = self.collect_docs(accept_contains)?;
        self.next_term()?;
        Ok(result)
    }

    fn read_this_term(&mut self, term: Option<BytesRef>) {
        if let Some(t) = &term {
            let scratch = self.indexed_cell.take();
            self.indexed_cell = Some(self.q.base.grid.read_cell(t, scratch));
        }
        self.this_term = term;
    }

    /// `nextTerm()`.
    fn next_term(&mut self) -> Result<bool> {
        let term = self.t.terms_enum().next()?.cloned();
        if term.is_none() {
            self.this_term = None;
            return Ok(false);
        }
        self.read_this_term(term);
        Ok(true)
    }

    /// `union(SmallDocSet aSet, SmallDocSet bSet)`: `None` (Java `null`)
    /// args are treated as an empty set.
    fn union(a_set: Option<Rc<SmallDocSet>>, b_set: Option<Rc<SmallDocSet>>) -> Option<Rc<SmallDocSet>> {
        match b_set {
            Some(b_set) => match a_set {
                None => Some(b_set),
                Some(a_set) => Some(SmallDocSet::union(a_set, b_set)), // union is 'or'
            },
            None => a_set,
        }
    }

    /// `collectDocs(Bits acceptContains)`: returns null if no docs.
    fn collect_docs(&mut self, accept_contains: Option<&SmallDocSet>) -> Result<Option<Rc<SmallDocSet>>> {
        let mut set: Option<SmallDocSet> = None;
        self.t.postings()?;
        loop {
            let docid = self.t.postings_enum.as_mut().expect("set by postings()").next_doc()?;
            if docid == NO_MORE_DOCS {
                break;
            }
            if let Some(accept) = accept_contains {
                if !accept.get(docid as usize) {
                    continue;
                }
            }
            if set.is_none() {
                let mut size = self.t.terms_enum().doc_freq()?;
                if size <= 0 {
                    size = 16;
                }
                set = Some(SmallDocSet::new(size));
            }
            set.as_mut().expect("set above").set(docid);
        }
        Ok(set.map(Rc::new))
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.ContainsPrefixTreeQuery.SmallDocSet`
/// (`private static class SmallDocSet extends DocIdSet implements Bits`): a
/// hash based mutable set of docIds. If this were Solr code then we might
/// use a combination of HashDocSet and SortedIntDocSet instead.
///
/// Private in Java; public here because the
/// [`TreeDocIdSet`] a prefix tree query returns carries it.
#[derive(Debug, Clone)]
pub struct SmallDocSet {
    int_set: SentinelIntSet,
    max_int: i32,
}

impl SmallDocSet {
    /// `SmallDocSet(int size)`.
    pub fn new(size: i32) -> Self {
        Self {
            int_set: SentinelIntSet::new(size.max(0) as usize, -1),
            max_int: 0,
        }
    }

    /// `set(int index)`.
    pub fn set(&mut self, index: i32) {
        self.int_set.put(index);
        if index > self.max_int {
            self.max_int = index;
        }
    }

    /// `size()`: number of docids.
    pub fn size(&self) -> usize {
        self.int_set.size()
    }

    /// `union(SmallDocSet other)`: NOTE: modifies and returns either
    /// `this` or `other`.
    fn union(this: Rc<SmallDocSet>, other: Rc<SmallDocSet>) -> Rc<SmallDocSet> {
        let (mut bigger, smaller) = if other.int_set.size() > this.int_set.size() {
            (other, this)
        } else {
            (this, other)
        };
        // modify bigger
        let b = Rc::make_mut(&mut bigger);
        for &v in &smaller.int_set.keys {
            if v == smaller.int_set.empty_val {
                continue;
            }
            b.set(v);
        }
        bigger
    }

    /// `iterator()`: `None` (Java `null`) when the set is empty.
    ///
    /// # Errors
    ///
    /// None in practice; the signature is `DocIdSet`'s.
    pub fn java_iterator(&self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        if self.size() == 0 {
            return Ok(None);
        }
        // copy the unsorted values to a new array then sort them
        let mut docs: Vec<i32> = Vec::with_capacity(self.int_set.size());
        for &v in &self.int_set.keys {
            if v == self.int_set.empty_val {
                continue;
            }
            docs.push(v);
        }
        debug_assert_eq!(docs.len(), self.int_set.size());
        // sort them
        docs.sort_unstable();
        let size = docs.len();
        Ok(Some(Box::new(SmallDocSetIterator { docs, size, idx: -1 })))
    }
}

impl Bits for SmallDocSet {
    fn get(&self, index: usize) -> bool {
        self.int_set.exists(index as i32)
    }

    fn length(&self) -> usize {
        self.max_int as usize
    }
}

impl Accountable for SmallDocSet {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_OBJECT_REF + 4) + self.int_set.ram_bytes_used()
    }
}

impl DocIdSet for SmallDocSet {
    /// `iterator()`. Java returns `null` for an empty set (see
    /// [`java_iterator`](SmallDocSet::java_iterator), which the query uses);
    /// this trait cannot, so an empty set yields an exhausted iterator.
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        match self.java_iterator()? {
            Some(it) => Ok(it),
            None => Ok(Box::new(SmallDocSetIterator {
                docs: Vec::new(),
                size: 0,
                idx: -1,
            })),
        }
    }

    /// `bits()`: if the # of docids is super small, return null since
    /// iteration is going to be faster.
    fn bits(&self) -> Result<Option<Box<dyn Bits>>> {
        Ok(if self.size() > 4 { Some(Box::new(self.clone())) } else { None })
    }
}

/// The anonymous `DocIdSetIterator` of `SmallDocSet.iterator()`.
struct SmallDocSetIterator {
    docs: Vec<i32>,
    size: usize,
    idx: i32,
}

impl DocIdSetIterator for SmallDocSetIterator {
    fn doc_id(&self) -> i32 {
        if self.idx < 0 {
            -1
        } else if (self.idx as usize) < self.size {
            self.docs[self.idx as usize]
        } else {
            NO_MORE_DOCS
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.idx += 1;
        if (self.idx as usize) < self.size {
            return Ok(self.docs[self.idx as usize]);
        }
        Ok(NO_MORE_DOCS)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        // for this small set this is likely faster vs. a binary search
        // into the sorted array
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        self.size as i64
    }
}

impl PrefixTreeQuery for ContainsPrefixTreeQuery {
    fn tree_query(&self) -> &AbstractPrefixTreeQuery {
        &self.base
    }

    fn get_doc_id_set(&self, context: &LeafReaderContext) -> Result<Option<TreeDocIdSet>> {
        let mut visitor = ContainsVisitor::new(self, context)?;
        let world = self.base.grid.get_world_cell();
        let result = visitor.visit(&world, None)?;
        Ok(result.map(|set| TreeDocIdSet::Small(Rc::try_unwrap(set).unwrap_or_else(|rc| (*rc).clone()))))
    }
}

impl Query for ContainsPrefixTreeQuery {
    fn to_query_string(&self, _field: &str) -> String {
        let b = &self.base;
        format!(
            "ContainsPrefixTreeQuery(fieldName={},queryShape={},detailLevel={},multiOverlappingIndexedShapes={})",
            b.field_name, b.query_shape, b.detail_level, self.multi_overlapping_indexed_shapes
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.base.visit(self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(&self, _searcher: &IndexSearcher, score_mode: ScoreMode, boost: f32) -> Result<Arc<dyn Weight>> {
        Ok(create_weight(self, score_mode, boost))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other.as_any().downcast_ref::<ContainsPrefixTreeQuery>().is_some_and(|that| {
            self.base.equals_to(&that.base) && self.multi_overlapping_indexed_shapes == that.multi_overlapping_indexed_shapes
        })
    }

    fn query_hash(&self) -> i32 {
        self.base
            .hash_code(self.class_hash())
            .wrapping_add(if self.multi_overlapping_indexed_shapes { 1 } else { 0 })
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial.prefix.ContainsPrefixTreeQuery"
    }
}
