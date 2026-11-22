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

//! The types declared in `PrefixTreeFacetCounter.java`, of the Java package
//! `org.apache.lucene.spatial.prefix`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{IndexReaderContext, LeafReaderContext};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::util::Bits;
use spatial4j::shape::shape::Shape;

use crate::prefix::abstract_prefix_tree_query::{BaseTermsEnumTraverser, TreeDocIdSet};
use crate::prefix::abstract_visiting_prefix_tree_query::{AbstractVisitingPrefixTreeQuery, VisitorHooks, VisitorTemplate};
use crate::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use crate::prefix::tree::cell::CellRef;

/// Equivalent to `org.apache.lucene.spatial.prefix.PrefixTreeFacetCounter.FacetVisitor`
/// (`public abstract static class FacetVisitor`): a callback/visitor of
/// facet counts.
pub trait FacetVisitor {
    /// `startOfSegment()`: called at the start of the segment, if there is
    /// indexed data.
    fn start_of_segment(&mut self) {}

    /// `visit(Cell cell, int count)`: called for cells with a leaf, or cells
    /// at the target facet level. `count` is greater than zero. When
    /// `cell.isLeaf()` is true, the shapes of the documents are within the
    /// cell (or the facet level is too coarse).
    fn visit(&mut self, cell: &CellRef, count: i32);
}

/// Equivalent to `org.apache.lucene.spatial.prefix.PrefixTreeFacetCounter`
/// (`public class PrefixTreeFacetCounter`): computes facets on cells for a
/// [`PrefixTreeStrategy`] field. The `facetLevel` parameter is the depth of
/// the tree to count terms at (`SpatialPrefixTree.getLevelForDistance`).
/// This is a static utility class; its methods are the functions of this
/// module.
#[derive(Debug)]
pub struct PrefixTreeFacetCounter;

/// The anonymous `Bits` of `compute` over a leaf of `topAcceptDocs`.
struct LeafAcceptDocs<'a> {
    top_accept_docs: &'a dyn Bits,
    doc_base: i32,
    max_doc: i32,
}

impl fmt::Debug for LeafAcceptDocs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeafAcceptDocs").field("doc_base", &self.doc_base).finish()
    }
}

impl LeafAcceptDocs<'_> {
    fn get(&self, index: usize) -> bool {
        self.top_accept_docs.get(self.doc_base as usize + index)
    }

    fn length(&self) -> usize {
        self.max_doc as usize
    }
}

/// The accept docs of one leaf: its live docs, or a view of the top-level
/// accept docs.
enum LeafBits<'a> {
    Live(Box<dyn Bits>),
    Top(LeafAcceptDocs<'a>),
}

impl LeafBits<'_> {
    fn get(&self, index: usize) -> bool {
        match self {
            LeafBits::Live(b) => b.get(index),
            LeafBits::Top(b) => b.get(index),
        }
    }

    fn length(&self) -> usize {
        match self {
            LeafBits::Live(b) => b.length(),
            LeafBits::Top(b) => b.length(),
        }
    }
}

/// `compute(PrefixTreeStrategy strategy, IndexReaderContext context, Bits
/// topAcceptDocs, Shape queryShape, int facetLevel, FacetVisitor
/// facetVisitor)`: computes facets using a callback/visitor style design,
/// allowing flexibility for the caller to determine what to do with each
/// underlying count.
///
/// # Errors
///
/// The errors of the per-leaf computation.
pub fn compute(
    strategy: &dyn PrefixTreeStrategy,
    context: &Arc<dyn IndexReaderContext>,
    top_accept_docs: Option<&dyn Bits>,
    query_shape: &Arc<dyn Shape>,
    facet_level: i32,
    facet_visitor: &mut dyn FacetVisitor,
) -> Result<()> {
    // We collect per-leaf
    for leaf_ctx in Arc::clone(context).leaves() {
        // determine leaf acceptDocs Bits
        let leaf_accept_docs = match top_accept_docs {
            None => leaf_ctx.leaf_reader().get_live_docs().map(LeafBits::Live), // filter deleted
            Some(top) => Some(LeafBits::Top(LeafAcceptDocs {
                top_accept_docs: top,
                doc_base: leaf_ctx.doc_base(),
                max_doc: leaf_ctx.leaf_reader().max_doc(),
            })),
        };
        compute_leaf_bits(strategy, &leaf_ctx, leaf_accept_docs.as_ref(), query_shape, facet_level, facet_visitor)?;
    }
    Ok(())
}

/// `compute(PrefixTreeStrategy strategy, LeafReaderContext context, Bits
/// acceptDocs, Shape queryShape, int facetLevel, FacetVisitor
/// facetVisitor)`: lower-level per-leaf segment method.
///
/// # Errors
///
/// `IllegalArgumentException` when `acceptDocs` does not cover the leaf;
/// the I/O errors of the traversal.
pub fn compute_leaf(
    strategy: &dyn PrefixTreeStrategy,
    context: &LeafReaderContext,
    accept_docs: Option<Box<dyn Bits>>,
    query_shape: &Arc<dyn Shape>,
    facet_level: i32,
    facet_visitor: &mut dyn FacetVisitor,
) -> Result<()> {
    let bits = accept_docs.map(LeafBits::Live);
    compute_leaf_bits(strategy, context, bits.as_ref(), query_shape, facet_level, facet_visitor)
}

fn compute_leaf_bits(
    strategy: &dyn PrefixTreeStrategy,
    context: &LeafReaderContext,
    accept_docs: Option<&LeafBits<'_>>,
    query_shape: &Arc<dyn Shape>,
    facet_level: i32,
    facet_visitor: &mut dyn FacetVisitor,
) -> Result<()> {
    if let Some(accept_docs) = accept_docs {
        let max_doc = context.leaf_reader().max_doc();
        if accept_docs.length() != max_doc as usize {
            return Err(LuceneError::illegal_argument(format!(
                "acceptDocs bits length {} != leaf maxdoc {max_doc}",
                accept_docs.length()
            )));
        }
    }
    let tree = strategy.get_grid();

    // scanLevel is an optimization knob of AbstractVisitingPrefixTreeFilter. It's unlikely
    // another scanLevel would be much faster and it tends to be a risky knob (can help a little,
    // can hurt a ton).
    // TODO use RPT's configured scan level?  Do we know better here?  Hard to say.
    let scan_level = tree.get_max_levels();
    // AbstractVisitingPrefixTreeFilter is a Lucene Filter.  We don't need a filter; we use it for
    // its great prefix-tree
    // traversal code.  TODO consider refactoring if/when it makes sense (more use cases than this)
    let query = AbstractVisitingPrefixTreeQuery::new(
        Arc::clone(query_shape),
        strategy.get_field_name(),
        tree,
        facet_level,
        scan_level,
    );
    debug_assert_eq!(facet_level, query.base.detail_level); // same thing, FYI. (constant)
    let hooks = FacetHooks {
        facet_level,
        accept_docs,
        facet_visitor,
    };
    VisitorTemplate::new(&query, context, hooks)?.get_doc_id_set()?;
    Ok(())
}

/// The anonymous `VisitorTemplate` of the anonymous query of `compute`.
struct FacetHooks<'a, 'b> {
    facet_level: i32,
    accept_docs: Option<&'a LeafBits<'b>>,
    facet_visitor: &'a mut dyn FacetVisitor,
}

impl FacetHooks<'_, '_> {
    /// `countDocsAtThisTerm()`.
    fn count_docs_at_this_term(&mut self, t: &mut BaseTermsEnumTraverser) -> Result<i32> {
        let Some(accept_docs) = self.accept_docs else {
            return t.terms_enum().doc_freq();
        };
        let mut count = 0;
        let postings = t.postings()?;
        while postings.next_doc()? != NO_MORE_DOCS {
            if !accept_docs.get(postings.doc_id() as usize) {
                continue;
            }
            count += 1;
        }
        Ok(count)
    }

    /// `hasDocsAtThisTerm()`.
    fn has_docs_at_this_term(&mut self, t: &mut BaseTermsEnumTraverser) -> Result<bool> {
        let Some(accept_docs) = self.accept_docs else {
            return Ok(true);
        };
        let postings = t.postings()?;
        let mut next_doc = postings.next_doc()?;
        while next_doc != NO_MORE_DOCS && !accept_docs.get(next_doc as usize) {
            next_doc = postings.next_doc()?;
        }
        Ok(next_doc != NO_MORE_DOCS)
    }
}

impl VisitorHooks for FacetHooks<'_, '_> {
    fn start(&mut self, _t: &mut BaseTermsEnumTraverser) -> Result<()> {
        self.facet_visitor.start_of_segment();
        Ok(())
    }

    fn finish(&mut self, _t: &mut BaseTermsEnumTraverser) -> Result<Option<TreeDocIdSet>> {
        Ok(None) // unused;
    }

    fn visit_prefix(&mut self, t: &mut BaseTermsEnumTraverser, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<bool> {
        // At facetLevel...
        if cell.get_level() == self.facet_level {
            // Count docs
            self.visit_leaf(t, q, cell)?; // we're not a leaf but we treat it as such at facet level
            return Ok(false); // don't descend further; this is enough detail
        }

        // We optimize for discriminating filters (reflected in acceptDocs) and short-circuit if
        // no
        // matching docs. We could do this at all levels or never but the closer we get to the
        // facet level, the
        // higher the probability this is worthwhile. We do when docFreq == 1 because it's a
        // cheap check, especially
        // due to "pulsing" in the codec.
        // TODO this opt should move to VisitorTemplate (which contains an optimization TODO to
        // this effect)
        if (cell.get_level() == self.facet_level - 1 || t.terms_enum().doc_freq()? == 1)
            && !self.has_docs_at_this_term(t)?
        {
            return Ok(false);
        }
        Ok(true)
    }

    fn visit_leaf(&mut self, t: &mut BaseTermsEnumTraverser, _q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<()> {
        let count = self.count_docs_at_this_term(t)?;
        if count > 0 {
            self.facet_visitor.visit(cell, count);
        }
        Ok(())
    }
}
