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

//! The types declared in `AbstractVisitingPrefixTreeQuery.java`, of the Java
//! package `org.apache.lucene.spatial.prefix`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{LeafReaderContext, SeekStatus};
use rucene::util::BytesRef;
use spatial4j::shape::shape::Shape;

use crate::prefix::abstract_prefix_tree_query::{AbstractPrefixTreeQuery, BaseTermsEnumTraverser, TreeDocIdSet};
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;

/// Equivalent to `org.apache.lucene.spatial.prefix.AbstractVisitingPrefixTreeQuery`
/// (`public abstract class AbstractVisitingPrefixTreeQuery extends
/// AbstractPrefixTreeQuery`): traverses a `SpatialPrefixTree` indexed field,
/// using the template and visitor design patterns for subclasses to guide
/// the traversal and collect matching documents.
///
/// Subclasses implement `getDocIdSet` by instantiating a
/// [`VisitorTemplate`] over their [`VisitorHooks`] and calling
/// [`VisitorTemplate::get_doc_id_set`].
#[derive(Clone, Debug)]
pub struct AbstractVisitingPrefixTreeQuery {
    /// The `AbstractPrefixTreeQuery` fields.
    pub base: AbstractPrefixTreeQuery,
    /// `prefixGridScanLevel`: at least one less than
    /// `grid.getMaxLevels()`.
    pub prefix_grid_scan_level: i32,
}

impl AbstractVisitingPrefixTreeQuery {
    /// `AbstractVisitingPrefixTreeQuery(Shape queryShape, String fieldName,
    /// SpatialPrefixTree grid, int detailLevel, int prefixGridScanLevel)`.
    pub fn new(
        query_shape: Arc<dyn Shape>,
        field_name: &str,
        grid: Arc<dyn SpatialPrefixTree>,
        detail_level: i32,
        prefix_grid_scan_level: i32,
    ) -> Self {
        let max_levels = grid.get_max_levels();
        debug_assert!(detail_level <= max_levels);
        Self {
            base: AbstractPrefixTreeQuery::new(query_shape, field_name, grid, detail_level),
            prefix_grid_scan_level: 0.max(prefix_grid_scan_level.min(max_levels - 1)),
        }
    }
}

/// The abstract and overridable methods of `VisitorTemplate`: an
/// implementation of the anonymous subclass a query's `getDocIdSet`
/// instantiates. The template's own state is the
/// [`BaseTermsEnumTraverser`] each method receives (Java's inherited
/// fields), and `query` the enclosing query.
pub trait VisitorHooks {
    /// `start()`: called first to setup things.
    ///
    /// # Errors
    ///
    /// The I/O errors of the implementation.
    fn start(&mut self, t: &mut BaseTermsEnumTraverser) -> Result<()>;

    /// `finish()`: called last to return the result.
    ///
    /// # Errors
    ///
    /// The I/O errors of the implementation.
    fn finish(&mut self, t: &mut BaseTermsEnumTraverser) -> Result<Option<TreeDocIdSet>>;

    /// `visitPrefix(Cell cell)`: visit an indexed non-leaf cell. Returns
    /// true if this cell's children should be traversed.
    ///
    /// # Errors
    ///
    /// The I/O errors of collecting documents.
    fn visit_prefix(&mut self, t: &mut BaseTermsEnumTraverser, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<bool>;

    /// `visitLeaf(Cell cell)`: visit an indexed leaf cell. Implementations
    /// usually collect the documents.
    ///
    /// # Errors
    ///
    /// The I/O errors of collecting documents.
    fn visit_leaf(&mut self, t: &mut BaseTermsEnumTraverser, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<()>;

    /// `visitScanned(Cell cell)`: the cell is either indexed as a leaf or
    /// is the last level of detail. It might not even intersect the query
    /// shape, so be sure to check for that. The default implementation will
    /// check that and if passes then call `visitLeaf` or `visitPrefix`.
    ///
    /// # Errors
    ///
    /// The I/O errors of collecting documents.
    fn visit_scanned(&mut self, t: &mut BaseTermsEnumTraverser, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<()> {
        let relate = cell.get_shape().relate(&*q.base.query_shape);
        if relate.intersects() {
            cell.set_shape_rel(Some(relate)); // just being pedantic
            if cell.is_leaf() {
                self.visit_leaf(t, q, cell)?;
            } else {
                self.visit_prefix(t, q, cell)?;
            }
        }
        Ok(())
    }

    /// `findSubCellsToVisit(Cell cell)`: called when doing a divide and
    /// conquer to find the next intersecting cells of the query shape that
    /// are beneath `cell`. `cell` is guaranteed to have an intersection and
    /// thus this must return some number of nodes.
    fn find_sub_cells_to_visit(&mut self, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Box<dyn CellIterator> {
        cell.get_next_level_cells(Some(Arc::clone(&q.base.query_shape)))
    }

    /// `preSiblings(VNode vNode)`: called before the siblings of `v_node`
    /// are visited.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn pre_siblings(&mut self, _v_node: &VNode) -> Result<()> {
        Ok(())
    }

    /// `postSiblings(VNode vNode)`: called after the siblings of `v_node`
    /// were visited.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn post_siblings(&mut self, _v_node: &VNode) -> Result<()> {
        Ok(())
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.AbstractVisitingPrefixTreeQuery.VNode`
/// (`protected static class VNode`): a visitor node/cell found via the
/// query shape for `VisitorTemplate`. Sometimes these are reset(cell). It's
/// like a LinkedList node but forms a tree.
///
/// The nodes of one traversal live in the template's node stack: `parent`
/// is the index of the parent node there (only `None` at the root).
pub struct VNode {
    /// `parent`: only null at the root.
    pub parent: Option<usize>,
    /// `children`: null, then sometimes set, then null.
    pub(crate) children: Option<VNodeCellIterator>,
    /// `cell`: not null (except initially before reset()).
    pub cell: Option<CellRef>,
}

impl VNode {
    /// `VNode(VNode parent)`: remember to call reset(cell) after.
    fn new(parent: Option<usize>) -> Self {
        Self {
            parent,
            children: None,
            cell: None,
        }
    }

    /// `reset(Cell cell)`.
    fn reset(&mut self, cell: CellRef) {
        self.cell = Some(cell);
        debug_assert!(self.children.is_none());
    }

    fn cell(&self) -> &CellRef {
        self.cell.as_ref().expect("reset(cell) was called")
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.AbstractVisitingPrefixTreeQuery.VisitorTemplate.VNodeCellIterator`
/// (`private class VNodeCellIterator implements Iterator<VNode>`): used in
/// `VNode.children`; `v_node` is the index of the one reused child node.
pub(crate) struct VNodeCellIterator {
    cell_iter: Box<dyn CellIterator>,
    v_node: usize,
}

/// Equivalent to `org.apache.lucene.spatial.prefix.AbstractVisitingPrefixTreeQuery.VisitorTemplate`
/// (`public abstract class VisitorTemplate extends BaseTermsEnumTraverser`):
/// an abstract class designed to make it easy to implement predicates or
/// other operations on a `SpatialPrefixTree` indexed field. An instance of
/// this class is not designed to be re-used across LeafReaderContext
/// instances so simply create a new one per-leaf. The `getDocIdSet()`
/// method here starts the work. It first checks that there are indexed
/// terms; if not it quickly returns null. Then it calls `start()` so a
/// subclass can set up a return value, like a `FixedBitSet`. Then it starts
/// the traversal process, calling `findSubCellsToVisit` which by default
/// finds the top cells that intersect `queryShape`. If there isn't an
/// indexed cell for a corresponding cell returned for this method then
/// it's short-circuited until it finds one, at which point
/// `visitPrefix` is called. At some depths, of the tree, the algorithm
/// switches to a scanning mode that calls `visitScanned` for each leaf cell
/// found.
///
/// The VNode tree is held as a stack indexed by depth: a node's children
/// all reuse the node one level down, as Java's single `VNode` per
/// `VNodeCellIterator` does.
pub struct VisitorTemplate<'q, H: VisitorHooks> {
    /// The `BaseTermsEnumTraverser` fields.
    pub traverser: BaseTermsEnumTraverser,
    query: &'q AbstractVisitingPrefixTreeQuery,
    /// The subclass.
    pub hooks: H,
    nodes: Vec<VNode>,
    cur_v_node: Option<usize>, // current pointer, derived from query shape
    cur_v_node_term: BytesRef, // curVNode.cell's term, without leaf. in main loop only
    this_term: Option<BytesRef>, // the result of termsEnum.term()
    indexed_cell: Option<CellRef>, // Cell wrapper of thisTerm. Always updated when thisTerm is.
}

impl<'q, H: VisitorHooks> VisitorTemplate<'q, H> {
    /// `VisitorTemplate(LeafReaderContext context)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of reading the terms.
    pub fn new(query: &'q AbstractVisitingPrefixTreeQuery, context: &LeafReaderContext, hooks: H) -> Result<Self> {
        Ok(Self {
            traverser: BaseTermsEnumTraverser::new(context, &query.base.field_name)?,
            query,
            hooks,
            nodes: Vec::new(),
            cur_v_node: None,
            cur_v_node_term: BytesRef::new(Vec::new()),
            this_term: None,
            indexed_cell: None,
        })
    }

    fn indexed_cell(&self) -> &CellRef {
        self.indexed_cell.as_ref().expect("set with thisTerm")
    }

    fn cur(&self) -> usize {
        self.cur_v_node.expect("curVNode is set")
    }

    /// `curVNode.children.next()` of the node `p`.
    fn children_next(&mut self, p: usize) -> usize {
        let children = self.nodes[p].children.as_mut().expect("children != null");
        debug_assert!(children.cell_iter.has_next());
        let cell = children.cell_iter.next();
        let v = children.v_node;
        self.nodes[v].reset(cell);
        v
    }

    /// `getDocIdSet()`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the traversal and of the hooks.
    pub fn get_doc_id_set(&mut self) -> Result<Option<TreeDocIdSet>> {
        debug_assert!(self.cur_v_node.is_none(), "Called more than once?");
        if self.traverser.terms_enum.is_none() {
            return Ok(None);
        }
        if !self.next_term()? {
            // advances
            return Ok(None);
        }

        self.nodes.clear();
        self.nodes.push(VNode::new(None));
        self.cur_v_node = Some(0);
        let world = self.query.base.grid.get_world_cell();
        self.nodes[0].reset(world);

        self.hooks.start(&mut self.traverser)?;

        self.add_intersecting_children()?;

        'main: while self.this_term.is_some() {
            // terminates for other reasons too!

            // Advance curVNode pointer
            let cur = self.cur();
            if self.nodes[cur].children.is_some() {
                // -- HAVE CHILDREN: DESCEND
                self.hooks.pre_siblings(&self.nodes[cur])?;
                self.cur_v_node = Some(self.children_next(cur));
            } else {
                // -- NO CHILDREN: ADVANCE TO NEXT SIBLING
                let mut parent_v_node = self.nodes[cur].parent;
                loop {
                    let Some(p) = parent_v_node else {
                        break 'main; // all done
                    };
                    let has_next = self.nodes[p]
                        .children
                        .as_mut()
                        .expect("a parent has children")
                        .cell_iter
                        .has_next();
                    if has_next {
                        // advance next sibling
                        self.cur_v_node = Some(self.children_next(p));
                        break;
                    } else {
                        // reached end of siblings; pop up
                        self.hooks.post_siblings(&self.nodes[p])?;
                        self.nodes[p].children = None; // GC
                        parent_v_node = self.nodes[p].parent;
                    }
                }
            }

            // Seek to curVNode's cell (or skip if termsEnum has moved beyond)
            let cur_cell = Arc::clone(self.nodes[self.cur()].cell());
            let compare = self.indexed_cell().compare_to_no_leaf(&*cur_cell);
            if compare > 0 {
                // The indexed cell is after; continue loop to next query cell
                continue;
            }
            if compare < 0 {
                // The indexed cell is before; seek ahead to query cell:
                //      Seek !
                let reuse = std::mem::replace(&mut self.cur_v_node_term, BytesRef::new(Vec::new()));
                self.cur_v_node_term = cur_cell.get_token_bytes_no_leaf(Some(reuse));
                let seek_status = self.traverser.terms_enum().seek_ceil(&self.cur_v_node_term)?;
                if seek_status == SeekStatus::END {
                    break; // all done
                }
                let term = self.traverser.terms_enum().term()?.cloned();
                self.read_this_term(term);
                if seek_status == SeekStatus::NOT_FOUND {
                    // Did we find a leaf of the cell we were looking for or something after?
                    if !self.indexed_cell().is_leaf() || self.indexed_cell().compare_to_no_leaf(&*cur_cell) != 0 {
                        continue; // The indexed cell is after; continue loop to next query cell
                    }
                }
            }

            // indexedCell == queryCell (disregarding leaf).

            // If indexedCell is a leaf then there's no prefix (prefix sorts before) -- just visit and
            // continue
            if self.indexed_cell().is_leaf() {
                let indexed_cell = Arc::clone(self.indexed_cell());
                self.hooks.visit_leaf(&mut self.traverser, self.query, &indexed_cell)?; // TODO or query cell? Though shouldn't matter.
                if !self.next_term()? {
                    break;
                }
                continue;
            }
            // If a prefix (non-leaf) then visit; see if we descend.
            // need to use curVNode.cell not indexedCell
            let descend = self.hooks.visit_prefix(&mut self.traverser, self.query, &cur_cell)?;
            if !self.next_term()? {
                break;
            }
            // Check for adjacent leaf with the same prefix
            if self.indexed_cell().is_leaf() && self.indexed_cell().get_level() == cur_cell.get_level() {
                let indexed_cell = Arc::clone(self.indexed_cell());
                self.hooks.visit_leaf(&mut self.traverser, self.query, &indexed_cell)?; // TODO or query cell? Though shouldn't matter.
                if !self.next_term()? {
                    break;
                }
            }

            if descend {
                self.add_intersecting_children()?;
            }
        } // main loop

        self.hooks.finish(&mut self.traverser)
    }

    /// `addIntersectingChildren()`: called initially, and whenever
    /// `visitPrefix` returns true.
    fn add_intersecting_children(&mut self) -> Result<()> {
        debug_assert!(self.this_term.is_some());
        let cur = self.cur();
        let cell = Arc::clone(self.nodes[cur].cell());
        if cell.get_level() >= self.query.base.detail_level {
            return Err(LuceneError::illegal_state("Spatial logic error"));
        }

        // Decide whether to continue to divide & conquer, or whether it's time to
        // scan through terms beneath this cell.
        // Scanning is a performance optimization trade-off.

        // TODO use termsEnum.docFreq() as heuristic
        let scan = cell.get_level() >= self.query.prefix_grid_scan_level; // simple heuristic

        if !scan {
            // Divide & conquer (ultimately termsEnum.seek())

            let mut sub_cells_iter = self.hooks.find_sub_cells_to_visit(self.query, &cell);
            if !sub_cells_iter.has_next() {
                // not expected
                return Ok(());
            }
            // new VNode(curVNode): the one child node, one level down
            self.nodes.truncate(cur + 1);
            self.nodes.push(VNode::new(Some(cur)));
            self.nodes[cur].children = Some(VNodeCellIterator {
                cell_iter: sub_cells_iter,
                v_node: cur + 1,
            });
        } else {
            // Scan (loop of termsEnum.next())

            self.scan(self.query.base.detail_level)?;
        }
        Ok(())
    }

    /// `scan(int scanDetailLevel)`: scans (`termsEnum.next()`) terms until a
    /// term is found that does not start with curVNode's cell. If it finds
    /// a leaf cell or a cell at level `scan_detail_level` then it calls
    /// `visitScanned(Cell)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the traversal and of the hooks.
    pub fn scan(&mut self, scan_detail_level: i32) -> Result<()> {
        // note: this can be a do-while instead in 6x; 5x has a back-compat with redundant leaves --
        // LUCENE-4942
        let cur_cell = Arc::clone(self.nodes[self.cur()].cell());
        while cur_cell.is_prefix_of(&**self.indexed_cell()) {
            let indexed_cell = Arc::clone(self.indexed_cell());
            if indexed_cell.get_level() == scan_detail_level
                || (indexed_cell.get_level() < scan_detail_level && indexed_cell.is_leaf())
            {
                self.hooks.visit_scanned(&mut self.traverser, self.query, &indexed_cell)?;
            }
            // advance
            if !self.next_term()? {
                break;
            }
        }
        Ok(())
    }

    /// `thisTerm = term; indexedCell = grid.readCell(thisTerm, indexedCell)`.
    fn read_this_term(&mut self, term: Option<BytesRef>) {
        if let Some(t) = &term {
            let scratch = self.indexed_cell.take();
            self.indexed_cell = Some(self.query.base.grid.read_cell(t, scratch));
        }
        self.this_term = term;
    }

    /// `nextTerm()`.
    fn next_term(&mut self) -> Result<bool> {
        let term = self.traverser.terms_enum().next()?.cloned();
        if term.is_none() {
            self.this_term = None;
            return Ok(false);
        }
        self.read_this_term(term);
        Ok(true)
    }
}
