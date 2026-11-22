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

//! The types declared in `WithinPrefixTreeQuery.java`, of the Java package
//! `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::{IndexSearcher, Query, QueryVisitor, ScoreMode, Weight};
use rucene::util::doc_id_set::BitDocIdSet;
use rucene::util::FixedBitSet;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::jdk::shapes_equal;
use crate::prefix::abstract_prefix_tree_query::{
    create_weight, AbstractPrefixTreeQuery, BaseTermsEnumTraverser, PrefixTreeQuery, TreeDocIdSet,
};
use crate::prefix::abstract_visiting_prefix_tree_query::{AbstractVisitingPrefixTreeQuery, VisitorHooks, VisitorTemplate};
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;

/// Equivalent to `org.apache.lucene.spatial.prefix.WithinPrefixTreeQuery`
/// (`public class WithinPrefixTreeQuery extends
/// AbstractVisitingPrefixTreeQuery`): finds docs where its indexed shape is
/// `IsWithin` the query shape. It works by looking at cells outside of the
/// query shape to ensure documents there are excluded. By default, it will
/// examine all cells, and it's fairly slow. If you know that the indexed
/// shapes are never comprised of more than one disjoint part (and so in
/// particular, a shape isn't a point cloud), then you can specify a buffer
/// distance to examine only a limited ring around the query shape.
#[derive(Clone, Debug)]
pub struct WithinPrefixTreeQuery {
    avptq: AbstractVisitingPrefixTreeQuery,
    buffered_query_shape: Option<Arc<dyn Shape>>, // if null then the whole world
}

impl WithinPrefixTreeQuery {
    /// `WithinPrefixTreeQuery(Shape queryShape, String fieldName,
    /// SpatialPrefixTree grid, int detailLevel, int prefixGridScanLevel,
    /// double queryBuffer)`: see `AbstractVisitingPrefixTreeQuery`. The
    /// `query_buffer` argument expands the query by this amount for the
    /// purposes of exclusion; -1 means the whole world.
    ///
    /// # Errors
    ///
    /// The errors of buffering the query shape.
    pub fn new(
        query_shape: Arc<dyn Shape>,
        field_name: &str,
        grid: Arc<dyn SpatialPrefixTree>,
        detail_level: i32,
        prefix_grid_scan_level: i32,
        query_buffer: f64,
    ) -> Result<Self> {
        let avptq = AbstractVisitingPrefixTreeQuery::new(query_shape, field_name, grid, detail_level, prefix_grid_scan_level);
        let buffered_query_shape = if query_buffer == -1.0 {
            None
        } else {
            Some(Self::buffer_shape(&avptq.base, &avptq.base.query_shape, query_buffer)?)
        };
        Ok(Self {
            avptq,
            buffered_query_shape,
        })
    }

    /// `bufferShape(Shape shape, double distErr)`: returns a new shape that
    /// is larger than shape by at distErr.
    fn buffer_shape(base: &AbstractPrefixTreeQuery, shape: &Arc<dyn Shape>, dist_err: f64) -> Result<Arc<dyn Shape>> {
        // TODO move this generic code elsewhere?  Spatial4j?
        if dist_err <= 0.0 {
            return Err(LuceneError::illegal_argument("distErr must be > 0"));
        }
        let ctx = base.grid.get_spatial_context();
        if let Some(point) = shape.as_point() {
            Ok(ctx.get_shape_factory().circle(point.get_x(), point.get_y(), dist_err)?)
        } else if let Some(circle) = shape.as_circle() {
            let mut new_dist = circle.get_radius() + dist_err;
            if ctx.is_geo() && new_dist > 180.0 {
                new_dist = 180.0;
            }
            Ok(ctx.get_shape_factory().circle_at(circle.get_center(), new_dist)?)
        } else {
            let bbox = shape.get_bounding_box();
            let mut new_min_x = bbox.get_min_x() - dist_err;
            let mut new_max_x = bbox.get_max_x() + dist_err;
            let mut new_min_y = bbox.get_min_y() - dist_err;
            let mut new_max_y = bbox.get_max_y() + dist_err;
            if ctx.is_geo() {
                if new_min_y < -90.0 {
                    new_min_y = -90.0;
                }
                if new_max_y > 90.0 {
                    new_max_y = 90.0;
                }
                if new_min_y == -90.0 || new_max_y == 90.0 || bbox.get_width() + 2.0 * dist_err > 360.0 {
                    new_min_x = -180.0;
                    new_max_x = 180.0;
                } else {
                    new_min_x = DistanceUtils::norm_lon_deg(new_min_x);
                    new_max_x = DistanceUtils::norm_lon_deg(new_max_x);
                }
            } else {
                // restrict to world bounds
                let world = ctx.get_world_bounds();
                new_min_x = new_min_x.max(world.get_min_x());
                new_max_x = new_max_x.min(world.get_max_x());
                new_min_y = new_min_y.max(world.get_min_y());
                new_max_y = new_max_y.min(world.get_max_y());
            }
            Ok(ctx.get_shape_factory().rect(new_min_x, new_max_x, new_min_y, new_max_y)?)
        }
    }
}

/// The anonymous `VisitorTemplate` of `getDocIdSet`.
struct WithinVisitor<'a> {
    buffered_query_shape: &'a Option<Arc<dyn Shape>>,
    inside: Option<FixedBitSet>,
    outside: Option<FixedBitSet>,
}

impl WithinVisitor<'_> {
    /// `allCellsIntersectQuery(Cell cell)`: returns true if the provided
    /// cell, and all its sub-cells down to detailLevel all intersect the
    /// queryShape.
    fn all_cells_intersect_query(q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> bool {
        let relate = cell.get_shape().relate(&*q.base.query_shape);
        if cell.get_level() == q.base.detail_level {
            return relate.intersects();
        }
        if relate == SpatialRelation::Within {
            return true;
        }
        if relate == SpatialRelation::Disjoint {
            return false;
        }
        // Note: Generating all these cells just to determine intersection is not ideal.
        // The real solution is LUCENE-4869.
        let mut sub_cells = cell.get_next_level_cells(None);
        while sub_cells.has_next() {
            let sub_cell = sub_cells.next();
            if !Self::all_cells_intersect_query(q, &sub_cell) {
                // recursion
                return false;
            }
        }
        true
    }
}

impl VisitorHooks for WithinVisitor<'_> {
    fn start(&mut self, t: &mut BaseTermsEnumTraverser) -> Result<()> {
        self.inside = Some(FixedBitSet::new(t.max_doc as usize));
        self.outside = Some(FixedBitSet::new(t.max_doc as usize));
        Ok(())
    }

    fn finish(&mut self, _t: &mut BaseTermsEnumTraverser) -> Result<Option<TreeDocIdSet>> {
        let mut inside = self.inside.take().expect("set by start()");
        let outside = self.outside.take().expect("set by start()");
        inside.and_not(&outside);
        Ok(Some(TreeDocIdSet::Core(Arc::new(BitDocIdSet::from_set(Arc::new(inside))?))))
    }

    fn find_sub_cells_to_visit(&mut self, _q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Box<dyn CellIterator> {
        // use buffered query shape instead of orig.  Works with null too.
        cell.get_next_level_cells(self.buffered_query_shape.clone())
    }

    fn visit_prefix(&mut self, t: &mut BaseTermsEnumTraverser, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<bool> {
        // cell.relate is based on the bufferedQueryShape; we need to examine what
        // the relation is against the queryShape
        let visit_relation = cell.get_shape().relate(&*q.base.query_shape);
        if cell.get_level() == q.base.detail_level {
            let set = if visit_relation.intersects() { &mut self.inside } else { &mut self.outside };
            t.collect_docs_bit_set(set.as_mut().expect("set by start()"))?;
            return Ok(false);
        } else if visit_relation == SpatialRelation::Within {
            t.collect_docs_bit_set(self.inside.as_mut().expect("set by start()"))?;
            return Ok(false);
        } else if visit_relation == SpatialRelation::Disjoint {
            t.collect_docs_bit_set(self.outside.as_mut().expect("set by start()"))?;
            return Ok(false);
        }
        Ok(true)
    }

    fn visit_leaf(&mut self, t: &mut BaseTermsEnumTraverser, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<()> {
        if Self::all_cells_intersect_query(q, cell) {
            t.collect_docs_bit_set(self.inside.as_mut().expect("set by start()"))
        } else {
            t.collect_docs_bit_set(self.outside.as_mut().expect("set by start()"))
        }
    }

    fn visit_scanned(&mut self, t: &mut BaseTermsEnumTraverser, q: &AbstractVisitingPrefixTreeQuery, cell: &CellRef) -> Result<()> {
        self.visit_leaf(t, q, cell)
        // collects as we want, even if not a leaf
        //        if (cell.isLeaf()) {
        //          visitLeaf(cell);
        //        } else {
        //          visitPrefix(cell);
        //        }
    }
}

impl PrefixTreeQuery for WithinPrefixTreeQuery {
    fn tree_query(&self) -> &AbstractPrefixTreeQuery {
        &self.avptq.base
    }

    fn get_doc_id_set(&self, context: &LeafReaderContext) -> Result<Option<TreeDocIdSet>> {
        let hooks = WithinVisitor {
            buffered_query_shape: &self.buffered_query_shape,
            inside: None,
            outside: None,
        };
        VisitorTemplate::new(&self.avptq, context, hooks)?.get_doc_id_set()
    }
}

impl Query for WithinPrefixTreeQuery {
    fn to_query_string(&self, _field: &str) -> String {
        let b = &self.avptq.base;
        format!(
            "WithinPrefixTreeQuery(fieldName={},queryShape={},detailLevel={},prefixGridScanLevel={})",
            b.field_name, b.query_shape, b.detail_level, self.avptq.prefix_grid_scan_level
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.avptq.base.visit(self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(&self, _searcher: &IndexSearcher, score_mode: ScoreMode, boost: f32) -> Result<Arc<dyn Weight>> {
        Ok(create_weight(self, score_mode, boost))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        // checks getClass == o.getClass & instanceof
        other.as_any().downcast_ref::<WithinPrefixTreeQuery>().is_some_and(|that| {
            self.avptq.base.equals_to(&that.avptq.base)
                && shapes_equal(self.buffered_query_shape.as_ref(), that.buffered_query_shape.as_ref())
        })
    }

    fn query_hash(&self) -> i32 {
        let mut result = self.avptq.base.hash_code(self.class_hash());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.buffered_query_shape.as_ref().map_or(0, |s| s.hash_code()));
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial.prefix.WithinPrefixTreeQuery"
    }
}
