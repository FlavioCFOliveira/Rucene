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

//! The types declared in `TermQueryPrefixTreeStrategy.java`, of the Java
//! package `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::index::IndexableField;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::term_in_set_query::TermInSetQuery;
use rucene::search::Query;
use rucene::util::BytesRef;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::prefix::cell_to_bytes_ref_iterator::{
    CellToBytesRefIterator, CellToBytesRefIteratorOps,
};
use crate::prefix::prefix_tree_strategy::{
    prefix_create_indexable_fields, prefix_make_distance_value_source, PrefixTreeStrategy,
    PrefixTreeStrategyBase,
};
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use crate::query::spatial_args::SpatialArgs;
use crate::query::spatial_operation::SpatialOperation;
use crate::query::unsupported_spatial_operation::UnsupportedSpatialOperation;
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};

/// Equivalent to `org.apache.lucene.spatial.prefix.TermQueryPrefixTreeStrategy`
/// (`public class TermQueryPrefixTreeStrategy extends PrefixTreeStrategy`):
/// a basic implementation of [`PrefixTreeStrategy`] using a large
/// `TermInSetQuery` of all the cells from
/// `SpatialPrefixTree.getTreeCellIterator`. It only supports the search of
/// indexed Point shapes.
///
/// The precision of query shapes (distErrPct) is an important factor in
/// using this Strategy. If the precision is too precise then it will result
/// in many terms which will amount to a slower query.
#[derive(Debug)]
pub struct TermQueryPrefixTreeStrategy {
    base: PrefixTreeStrategyBase,
    /// `simplifyIndexedCells`.
    pub simplify_indexed_cells: AtomicBool,
    me: Weak<TermQueryPrefixTreeStrategy>,
}

impl TermQueryPrefixTreeStrategy {
    /// `TermQueryPrefixTreeStrategy(SpatialPrefixTree grid, String
    /// fieldName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(
        grid: Arc<dyn SpatialPrefixTree>,
        field_name: &str,
    ) -> Result<Arc<TermQueryPrefixTreeStrategy>> {
        let base = PrefixTreeStrategyBase::new(grid, field_name)?;
        Ok(Arc::new_cyclic(|me| TermQueryPrefixTreeStrategy {
            base,
            simplify_indexed_cells: AtomicBool::new(false),
            me: me.clone(),
        }))
    }
}

impl fmt::Display for TermQueryPrefixTreeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            &self
                .base
                .strategy
                .to_string_named("TermQueryPrefixTreeStrategy"),
        )
    }
}

impl SpatialStrategy for TermQueryPrefixTreeStrategy {
    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base.strategy
    }

    fn create_indexable_fields(
        &self,
        shape: &Arc<dyn Shape>,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        prefix_create_indexable_fields(self, shape)
    }

    fn make_distance_value_source(
        &self,
        query_point: &Arc<dyn Point>,
        multiplier: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(prefix_make_distance_value_source(
            self,
            query_point,
            multiplier,
        ))
    }

    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        let op = args.get_operation();
        if op != SpatialOperation::Intersects {
            return Err(UnsupportedSpatialOperation::new(op));
        }

        let shape = &args.get_shape();
        let grid = &self.base.grid;
        let detail_level = grid.get_level_for_distance(
            args.resolve_dist_err(&self.base.strategy.ctx, self.get_dist_err_pct())?,
        );

        // --get a List of BytesRef for each term we want (no parents, no leaf bytes))
        let guess_num_terms = if shape.as_point().is_some() {
            detail_level // perfect guess
        } else {
            4096 // should this be a method on SpatialPrefixTree?
        };
        let mut terms: Vec<BytesRef> = Vec::with_capacity(guess_num_terms.max(0) as usize);
        let mut cells = grid.get_tree_cell_iterator(Arc::clone(shape), detail_level)?;
        while cells.has_next() {
            let cell = cells.next();
            if !cell.is_leaf() {
                continue;
            }
            let term = cell.get_token_bytes_no_leaf(None); // null because we want a new BytesRef
                                                           // We copy out the bytes because it may be re-used across the iteration. Java appends
                                                           // them all to one shared byte array; each term here holds its own copy of the same bytes.
            terms.push(BytesRef::new(
                term.bytes[term.offset..term.offset + term.length].to_vec(),
            ));
        }
        // unfortunately TermsQuery will needlessly sort & dedupe
        // TODO an automatonQuery might be faster?
        Ok(Arc::new(TermInSetQuery::new(
            self.get_field_name().to_string(),
            terms,
        )?))
    }
}

impl PrefixTreeStrategy for TermQueryPrefixTreeStrategy {
    fn prefix_base(&self) -> &PrefixTreeStrategyBase {
        &self.base
    }

    fn me_prefix(&self) -> Arc<dyn PrefixTreeStrategy> {
        self.me
            .upgrade()
            .expect("a TermQueryPrefixTreeStrategy is shared as an Arc")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn new_cell_to_bytes_ref_iterator(&self) -> Box<dyn CellToBytesRefIteratorOps> {
        // Ensure we don't have leaves, as this strategy doesn't handle them.
        Box::new(CellToBytesRefIterator::no_leaf())
    }
}
