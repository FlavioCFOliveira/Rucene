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

//! The types declared in `CompositeSpatialStrategy.java`, of the Java
//! package `org.apache.lucene.spatial.composite`.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::IndexableField;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::Query;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::composite::composite_verify_query::CompositeVerifyQuery;
use crate::composite::intersects_rpt_verify_query::IntersectsRPTVerifyQuery;
use crate::jdk::unsupported;
use crate::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategyOps;
use crate::query::spatial_args::SpatialArgs;
use crate::query::spatial_operation::SpatialOperation;
use crate::query::unsupported_spatial_operation::UnsupportedSpatialOperation;
use crate::serialized::serialized_dv_strategy::SerializedDVStrategy;
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};
use crate::util::shape_values_predicate::ShapeValuesPredicate;

/// Equivalent to `org.apache.lucene.spatial.composite.CompositeSpatialStrategy`
/// (`public class CompositeSpatialStrategy extends SpatialStrategy`): a
/// composite `SpatialStrategy` based on `RecursivePrefixTreeStrategy` (RPT)
/// and `SerializedDVStrategy` (SDV). RPT acts as an index to the precision
/// available in SDV, and in some circumstances can avoid geometry lookups
/// based on where a cell is in relation to the query shape. Currently the
/// only predicate optimized like this is Intersects. All predicates are
/// supported except for the BBox* ones, and Disjoint.
pub struct CompositeSpatialStrategy {
    base: SpatialStrategyBase,
    // TODO support others? (BBox)
    index_strategy: Arc<dyn RecursivePrefixTreeStrategyOps>,
    // TODO support others?
    geometry_strategy: Arc<SerializedDVStrategy>,
    optimize_predicates: AtomicBool,
}

impl fmt::Debug for CompositeSpatialStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl CompositeSpatialStrategy {
    /// `CompositeSpatialStrategy(String fieldName,
    /// RecursivePrefixTreeStrategy indexStrategy, SerializedDVStrategy
    /// geometryStrategy)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(
        field_name: &str,
        index_strategy: Arc<dyn RecursivePrefixTreeStrategyOps>,
        geometry_strategy: Arc<SerializedDVStrategy>,
    ) -> Result<Self> {
        // field name; unused
        Ok(Self {
            base: SpatialStrategyBase::new(index_strategy.get_spatial_context(), field_name)?,
            index_strategy,
            geometry_strategy,
            optimize_predicates: AtomicBool::new(true),
        })
    }

    /// `getIndexStrategy()`.
    pub fn get_index_strategy(&self) -> &Arc<dyn RecursivePrefixTreeStrategyOps> {
        &self.index_strategy
    }

    /// `getGeometryStrategy()`.
    pub fn get_geometry_strategy(&self) -> &Arc<SerializedDVStrategy> {
        &self.geometry_strategy
    }

    /// `isOptimizePredicates()`.
    pub fn is_optimize_predicates(&self) -> bool {
        self.optimize_predicates.load(Ordering::Relaxed)
    }

    /// `setOptimizePredicates(boolean optimizePredicates)`: set to false to
    /// NOT use optimized search predicates that avoid checking the geometry
    /// sometimes. Only useful for benchmarking.
    pub fn set_optimize_predicates(&self, optimize_predicates: bool) {
        self.optimize_predicates
            .store(optimize_predicates, Ordering::Relaxed);
    }
}

impl fmt::Display for CompositeSpatialStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_named("CompositeSpatialStrategy"))
    }
}

impl SpatialStrategy for CompositeSpatialStrategy {
    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base
    }

    fn create_indexable_fields(
        &self,
        shape: &Arc<dyn Shape>,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        let mut fields = self.index_strategy.create_indexable_fields(shape)?;
        fields.extend(self.geometry_strategy.create_indexable_fields(shape)?);
        Ok(fields)
    }

    fn make_distance_value_source(
        &self,
        _query_point: &Arc<dyn Point>,
        _multiplier: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        // TODO consider indexing center-point in DV?  Guarantee contained by the shape, which could
        // then be used for
        // other purposes like faster WITHIN predicate?
        Err(unsupported())
    }

    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        let pred = args.get_operation();

        if pred == SpatialOperation::BBoxIntersects || pred == SpatialOperation::BBoxWithin {
            return Err(UnsupportedSpatialOperation::new(pred));
        }

        if pred == SpatialOperation::IsDisjointTo {
            //      final Query intersectQuery = makeQuery(new SpatialArgs(SpatialOperation.Intersects,
            // args.getShape()));
            //      DocValues.getDocsWithField(reader, geometryStrategy.getFieldName());
            // TODO resurrect Disjoint spatial query utility accepting a field name known to have
            // DocValues.
            // update class docs when it's added.
            return Err(UnsupportedSpatialOperation::new(pred));
        }

        let predicate_value_source = Arc::new(ShapeValuesPredicate::new(
            self.geometry_strategy.make_shape_value_source(),
            pred,
            args.get_shape(),
        ));
        // System.out.println("PredOpt: " + optimizePredicates);
        if pred == SpatialOperation::Intersects && self.is_optimize_predicates() {
            // We have a smart Intersects impl

            let grid = self.index_strategy.get_grid();
            // default to max precision
            let detail_level =
                grid.get_level_for_distance(args.resolve_dist_err(&self.base.ctx, 0.0)?);
            Ok(Arc::new(IntersectsRPTVerifyQuery::new(
                args.get_shape(),
                self.index_strategy.get_field_name(),
                grid,
                detail_level,
                self.index_strategy.get_prefix_grid_scan_level(),
                predicate_value_source,
            )))
        } else {
            // The general path; all index matches get verified

            let new_index_args;
            let index_args: &SpatialArgs = if pred == SpatialOperation::Contains {
                // note: we could map IsWithin as well but it's pretty darned slow since it touches all
                // world grids
                args
            } else {
                // TODO add args.clone method with new predicate? Or simply make non-final?
                new_index_args = SpatialArgs::new(SpatialOperation::Intersects, args.get_shape());
                new_index_args.set_dist_err(args.get_dist_err());
                new_index_args.set_dist_err_pct(args.get_dist_err_pct());
                &new_index_args
            };

            if index_args.get_dist_err().is_none() && index_args.get_dist_err_pct().is_none() {
                index_args.set_dist_err_pct(Some(0.10));
            }

            let index_query = self.index_strategy.make_query(index_args)?;
            Ok(Arc::new(CompositeVerifyQuery::new(
                index_query,
                predicate_value_source,
            )))
        }
    }
}
