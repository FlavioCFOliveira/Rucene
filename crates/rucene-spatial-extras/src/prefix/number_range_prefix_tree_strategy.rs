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

//! The types declared in `NumberRangePrefixTreeStrategy.java`, of the Java
//! package `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::index::{IndexReaderContext, IndexableField};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::Query;
use rucene::util::Bits;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::jdk::unsupported;
use crate::prefix::prefix_tree_facet_counter::{self, FacetVisitor};
use crate::prefix::prefix_tree_strategy::{
    prefix_create_indexable_fields, PrefixTreeStrategy, PrefixTreeStrategyBase,
};
use crate::prefix::recursive_prefix_tree_strategy::{
    rpt_create_cell_iterator_to_index, rpt_make_query, rpt_to_string,
    RecursivePrefixTreeStrategyBase, RecursivePrefixTreeStrategyOps,
};
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::number_range_prefix_tree::{
    as_unit_nr_shape, NumberRangePrefixTreeImpl, UnitNRShape,
};
use crate::query::spatial_args::SpatialArgs;
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};

/// Equivalent to `org.apache.lucene.spatial.prefix.NumberRangePrefixTreeStrategy`
/// (`public class NumberRangePrefixTreeStrategy extends
/// RecursivePrefixTreeStrategy`): a `PrefixTree` based on Number/Date
/// ranges. This isn't very "spatial" on the surface (to the user) but it's
/// implemented using spatial so that's why it's here extending a
/// SpatialStrategy. When using this class, you will use various utility
/// methods on the prefix tree implementation to convert objects/strings
/// to/from shapes.
///
/// To use with dates, pass in a `DateRangePrefixTree`.
#[derive(Debug)]
pub struct NumberRangePrefixTreeStrategy {
    base: RecursivePrefixTreeStrategyBase,
    nr_grid: Arc<dyn NumberRangePrefixTreeImpl>,
    me: Weak<NumberRangePrefixTreeStrategy>,
}

impl NumberRangePrefixTreeStrategy {
    /// `NumberRangePrefixTreeStrategy(NumberRangePrefixTree prefixTree,
    /// String fieldName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(
        prefix_tree: Arc<dyn NumberRangePrefixTreeImpl>,
        field_name: &str,
    ) -> Result<Arc<NumberRangePrefixTreeStrategy>> {
        let grid: Arc<dyn crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree> =
            prefix_tree.clone();
        let base = RecursivePrefixTreeStrategyBase::new(grid, field_name)?;
        let strategy = Arc::new_cyclic(|me| NumberRangePrefixTreeStrategy {
            base,
            nr_grid: Arc::clone(&prefix_tree),
            me: me.clone(),
        });
        strategy.set_prune_leafy_branches(false);
        strategy.set_prefix_grid_scan_level(prefix_tree.get_max_levels() - 2); // user might want to change, however
        strategy.set_points_only(false);
        strategy.set_dist_err_pct(0.0);
        Ok(strategy)
    }

    /// `getGrid()` (the covariant override).
    pub fn get_nr_grid(&self) -> Arc<dyn NumberRangePrefixTreeImpl> {
        Arc::clone(&self.nr_grid)
    }

    /// `calcFacets(IndexReaderContext context, Bits topAcceptDocs,
    /// UnitNRShape start, UnitNRShape end)`: calculates facets between
    /// `start` and `end` to a detail level one greater than that provided
    /// by the arguments. For example providing March to October of 2014
    /// would return facets to the day level of those months. This is just a
    /// convenience method.
    ///
    /// # Errors
    ///
    /// The errors of `toRangeShape` and of the facet computation.
    pub fn calc_facets_range(
        &self,
        context: &Arc<dyn IndexReaderContext>,
        top_accept_docs: Option<&dyn Bits>,
        start: Arc<dyn UnitNRShape>,
        end: Arc<dyn UnitNRShape>,
    ) -> Result<Facets> {
        let detail_level = start.get_unit_level().max(end.get_unit_level()) + 1;
        let facet_range: Arc<dyn Shape> = self.get_nr_grid().to_range_shape(start, end)?;
        self.calc_nr_facets(context, top_accept_docs, &facet_range, detail_level)
    }

    /// `calcFacets(IndexReaderContext context, Bits topAcceptDocs, Shape
    /// facetRange, int level)`: calculates facets (aggregated counts) given
    /// a range shape (start-end span) and a level, which specifies the
    /// detail. To get the level of an existing shape, say a Calendar, call
    /// `toUnitShape` then call `getLevel()`. Facet computation is
    /// implemented by navigating the underlying indexed terms efficiently.
    ///
    /// # Errors
    ///
    /// The errors of the facet computation.
    pub fn calc_nr_facets(
        &self,
        context: &Arc<dyn IndexReaderContext>,
        top_accept_docs: Option<&dyn Bits>,
        facet_range: &Arc<dyn Shape>,
        level: i32,
    ) -> Result<Facets> {
        let mut facets = Facets::new(level);
        let mut visitor = NrFacetVisitor {
            facets: &mut facets,
            grid: self.get_nr_grid(),
            level,
            parent_facet: None,
            parent_shape: None,
        };
        prefix_tree_facet_counter::compute(
            self,
            context,
            top_accept_docs,
            facet_range,
            level,
            &mut visitor,
        )?;
        Ok(facets)
    }
}

/// The anonymous `FacetVisitor` of `calcFacets`.
struct NrFacetVisitor<'a> {
    facets: &'a mut Facets,
    grid: Arc<dyn NumberRangePrefixTreeImpl>,
    level: i32,
    /// `parentFacet`: the index of the parent in `facets.parents`.
    parent_facet: Option<usize>,
    parent_shape: Option<Arc<dyn UnitNRShape>>,
}

impl NrFacetVisitor<'_> {
    fn setup_parent(&mut self, unit_shape: &dyn UnitNRShape) {
        let parent_shape = unit_shape.clone_unit();
        // Look for existing parentFacet (from previous segment), or create anew if needed
        let idx = match self.facets.find(&*parent_shape) {
            Ok(idx) => idx,
            Err(insert_at) => {
                // didn't find one; make a new one
                let parent_facet = FacetParentVal {
                    parent_leaves: 0,
                    child_counts_len: self.grid.get_num_sub_cells(&*parent_shape),
                    child_counts: None,
                };
                self.facets
                    .parents
                    .insert(insert_at, (Arc::clone(&parent_shape), parent_facet));
                insert_at
            }
        };
        self.parent_shape = Some(parent_shape);
        self.parent_facet = Some(idx);
    }
}

/// `(UnitNRShape) cell.getShape()`.
fn unit_shape_of(cell: &CellRef) -> Arc<dyn UnitNRShape> {
    let shape = cell.get_shape();
    as_unit_nr_shape(&*shape).unwrap_or_else(|| {
        panic!("java.lang.ClassCastException: {shape} cannot be cast to UnitNRShape")
    })
}

impl FacetVisitor for NrFacetVisitor<'_> {
    fn visit(&mut self, cell: &CellRef, count: i32) {
        if cell.get_level() < self.level - 1 {
            // some ancestor of parent facet level, direct or distant
            self.parent_facet = None; // reset
            self.parent_shape = None; // reset
            self.facets.top_leaves += count;
        } else if cell.get_level() == self.level - 1 {
            // parent
            // set up FacetParentVal
            self.setup_parent(&*unit_shape_of(cell));
            let idx = self.parent_facet.expect("set by setupParent");
            self.facets.parents[idx].1.parent_leaves += count;
        } else {
            // at facet level
            let unit_shape = unit_shape_of(cell);
            let unit_shape_parent = unit_shape.get_shape_at_level(unit_shape.get_unit_level() - 1);
            let same_parent = match (&self.parent_facet, &self.parent_shape) {
                (Some(_), Some(parent_shape)) => parent_shape.equals(unit_shape_parent.as_shape()),
                _ => false,
            };
            if !same_parent {
                self.setup_parent(&*unit_shape_parent);
            }
            let idx = self.parent_facet.expect("set by setupParent");
            let parent_facet = &mut self.facets.parents[idx].1;
            // lazy init childCounts
            let len = parent_facet.child_counts_len.max(0) as usize;
            let child_counts = parent_facet
                .child_counts
                .get_or_insert_with(|| vec![0; len]);
            child_counts[unit_shape.get_val_at_level(cell.get_level()) as usize] += count;
        }
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.NumberRangePrefixTreeStrategy.Facets`
/// (`public static class Facets`): facet response information.
///
/// `parents` is Java's `TreeMap<UnitNRShape, FacetParentVal>`, sorted by
/// `UnitNRShape.compareTo`.
pub struct Facets {
    /// `detailLevel`: the requested detail level.
    pub detail_level: i32,
    /// `topLeaves`: the count of documents with ranges that completely
    /// spanned the parents of the detail level. In more technical terms,
    /// this is the count of leaf cells 2 up and higher from the bottom.
    /// Usually you only care about counts at detailLevel, and so you will
    /// add this number to all other counts below, including to omitted/implied
    /// children counts of 0. If there are no indexed ranges (just instances,
    /// i.e. fully specified dates) then this value will always be 0.
    pub top_leaves: i32,
    /// `parents`: holds all the `FacetParentVal` instances in order of the
    /// key. This is sparse; there won't be an instance if its count and
    /// children are all 0. The keys are `UnitNRShape` shapes, which can be
    /// converted back to the original Object (i.e. a Calendar) via
    /// `NumberRangePrefixTree.toObject(UnitNRShape)`.
    pub parents: Vec<(Arc<dyn UnitNRShape>, FacetParentVal)>,
}

impl Facets {
    /// `Facets(int detailLevel)`.
    pub fn new(detail_level: i32) -> Self {
        Self {
            detail_level,
            top_leaves: 0,
            parents: Vec::new(),
        }
    }

    /// `parents.get(key)`, mutable (Java's `FacetParentVal` is a mutable
    /// object the map holds by reference).
    pub fn get_parent_mut(&mut self, key: &dyn UnitNRShape) -> Option<&mut FacetParentVal> {
        match self.find(key) {
            Ok(i) => Some(&mut self.parents[i].1),
            Err(_) => None,
        }
    }

    /// The position of `key` in `parents` (`Ok`), or where it would be
    /// inserted (`Err`), by `UnitNRShape.compareTo`.
    fn find(&self, key: &dyn UnitNRShape) -> std::result::Result<usize, usize> {
        self.parents
            .binary_search_by(|(k, _)| k.compare_to(key).cmp(&0))
    }
}

impl fmt::Display for Facets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = String::with_capacity(2048);
        buf.push_str(&format!(
            "Facets: level={} topLeaves={} parentCount={}",
            self.detail_level,
            self.top_leaves,
            self.parents.len()
        ));
        for (key, p_val) in &self.parents {
            buf.push('\n');
            if buf.encode_utf16().count() > 1000 {
                buf.push_str("...");
                break;
            }
            buf.push_str(&format!(" {key} leafCount={}", p_val.parent_leaves));
            if let Some(child_counts) = &p_val.child_counts {
                let items: Vec<String> = child_counts.iter().map(|c| c.to_string()).collect();
                buf.push_str(&format!(" [{}]", items.join(", ")));
            }
        }
        f.write_str(&buf)
    }
}

impl fmt::Debug for Facets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.NumberRangePrefixTreeStrategy.Facets.FacetParentVal`
/// (`public static class FacetParentVal`): holds a block of detailLevel
/// counts aggregated to their parent level.
#[derive(Debug, Clone, Default)]
pub struct FacetParentVal {
    /// `parentLeaves`: the count of ranges that span all of the childCounts.
    /// In more technical terms, this is the number of leaf cells found at
    /// this parent. Treat this like `topLeaves`.
    pub parent_leaves: i32,
    /// `childCountsLen`: the length of `childCounts`. If childCounts is not
    /// null then this is `childCounts.length`, otherwise it says how long
    /// it would have been if it weren't null.
    pub child_counts_len: i32,
    /// `childCounts`: the detail level counts. It will be null if there are
    /// none, and thus they are assumed 0. Most apps, when presenting the
    /// information, will add `topLeaves` and `parentLeaves` to each count.
    pub child_counts: Option<Vec<i32>>,
}

impl fmt::Display for NumberRangePrefixTreeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rpt_to_string(self))
    }
}

impl SpatialStrategy for NumberRangePrefixTreeStrategy {
    fn as_recursive_prefix_tree_strategy(&self) -> Option<&dyn RecursivePrefixTreeStrategyOps> {
        Some(self)
    }

    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base.prefix.strategy
    }

    fn create_indexable_fields(
        &self,
        shape: &Arc<dyn Shape>,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        prefix_create_indexable_fields(self, shape)
    }

    fn make_distance_value_source(
        &self,
        _query_point: &Arc<dyn Point>,
        _multiplier: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        Err(unsupported())
    }

    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        rpt_make_query(self, args)
    }
}

impl PrefixTreeStrategy for NumberRangePrefixTreeStrategy {
    fn prefix_base(&self) -> &PrefixTreeStrategyBase {
        &self.base.prefix
    }

    fn me_prefix(&self) -> Arc<dyn PrefixTreeStrategy> {
        self.me
            .upgrade()
            .expect("a NumberRangePrefixTreeStrategy is shared as an Arc")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_cell_iterator_to_index(
        &self,
        shape: &Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Box<dyn CellIterator>> {
        rpt_create_cell_iterator_to_index(self, shape, detail_level)
    }

    fn is_point_shape(&self, shape: &dyn Shape) -> bool {
        match as_unit_nr_shape(shape) {
            Some(unit) => unit.get_unit_level() == self.base.prefix.grid.get_max_levels(),
            None => false,
        }
    }
}

impl RecursivePrefixTreeStrategyOps for NumberRangePrefixTreeStrategy {
    fn rpt_base(&self) -> &RecursivePrefixTreeStrategyBase {
        &self.base
    }

    fn simple_name(&self) -> &'static str {
        "NumberRangePrefixTreeStrategy"
    }

    fn is_grid_aligned_shape(&self, shape: &dyn Shape) -> bool {
        // any UnitNRShape other than the world is a single cell/term
        match as_unit_nr_shape(shape) {
            Some(unit) => unit.get_unit_level() > 0,
            None => false,
        }
    }
}
