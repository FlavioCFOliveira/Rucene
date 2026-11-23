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

//! The types declared in `IntersectsPrefixTreeQuery.java`, of the Java
//! package `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::{IndexSearcher, Query, QueryVisitor, ScoreMode, Weight};
use rucene::util::doc_id_set::DocIdSetBuilder;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::abstract_prefix_tree_query::{
    create_weight, AbstractPrefixTreeQuery, BaseTermsEnumTraverser, PrefixTreeQuery, TreeDocIdSet,
};
use crate::prefix::abstract_visiting_prefix_tree_query::{
    AbstractVisitingPrefixTreeQuery, VisitorHooks, VisitorTemplate,
};
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;

/// Equivalent to `org.apache.lucene.spatial.prefix.IntersectsPrefixTreeQuery`
/// (`public class IntersectsPrefixTreeQuery extends
/// AbstractVisitingPrefixTreeQuery`): a Query matching documents that have
/// an `SpatialRelation.INTERSECTS` (i.e. not DISTINCT) relationship with a
/// provided query shape.
#[derive(Clone, Debug)]
pub struct IntersectsPrefixTreeQuery {
    avptq: AbstractVisitingPrefixTreeQuery,
}

impl IntersectsPrefixTreeQuery {
    /// `IntersectsPrefixTreeQuery(Shape queryShape, String fieldName,
    /// SpatialPrefixTree grid, int detailLevel, int prefixGridScanLevel)`.
    pub fn new(
        query_shape: Arc<dyn Shape>,
        field_name: &str,
        grid: Arc<dyn SpatialPrefixTree>,
        detail_level: i32,
        prefix_grid_scan_level: i32,
    ) -> Self {
        Self {
            avptq: AbstractVisitingPrefixTreeQuery::new(
                query_shape,
                field_name,
                grid,
                detail_level,
                prefix_grid_scan_level,
            ),
        }
    }
}

/// The anonymous `VisitorTemplate` of `getDocIdSet`.
struct IntersectsVisitor {
    results: Option<DocIdSetBuilder>,
}

impl VisitorHooks for IntersectsVisitor {
    fn start(&mut self, t: &mut BaseTermsEnumTraverser) -> Result<()> {
        let terms = t
            .terms
            .as_ref()
            .expect("terms != null when termsEnum != null");
        self.results = Some(DocIdSetBuilder::from_terms(t.max_doc, &**terms)?);
        Ok(())
    }

    fn finish(&mut self, _t: &mut BaseTermsEnumTraverser) -> Result<Option<TreeDocIdSet>> {
        let results = self.results.take().expect("set by start()");
        Ok(Some(TreeDocIdSet::Core(Arc::from(results.build()?))))
    }

    fn visit_prefix(
        &mut self,
        t: &mut BaseTermsEnumTraverser,
        q: &AbstractVisitingPrefixTreeQuery,
        cell: &CellRef,
    ) -> Result<bool> {
        if cell.get_shape_rel() == Some(SpatialRelation::Within)
            || cell.get_level() == q.base.detail_level
        {
            t.collect_docs_builder(self.results.as_mut().expect("set by start()"))?;
            return Ok(false);
        }
        Ok(true)
    }

    fn visit_leaf(
        &mut self,
        t: &mut BaseTermsEnumTraverser,
        _q: &AbstractVisitingPrefixTreeQuery,
        _cell: &CellRef,
    ) -> Result<()> {
        t.collect_docs_builder(self.results.as_mut().expect("set by start()"))
    }
}

impl PrefixTreeQuery for IntersectsPrefixTreeQuery {
    fn tree_query(&self) -> &AbstractPrefixTreeQuery {
        &self.avptq.base
    }

    fn get_doc_id_set(&self, context: &LeafReaderContext) -> Result<Option<TreeDocIdSet>> {
        VisitorTemplate::new(&self.avptq, context, IntersectsVisitor { results: None })?
            .get_doc_id_set()
    }
}

impl Query for IntersectsPrefixTreeQuery {
    fn to_query_string(&self, _field: &str) -> String {
        let b = &self.avptq.base;
        format!(
            "IntersectsPrefixTreeQuery(fieldName={},queryShape={},detailLevel={},prefixGridScanLevel={})",
            b.field_name, b.query_shape, b.detail_level, self.avptq.prefix_grid_scan_level
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.avptq.base.visit(self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(create_weight(self, score_mode, boost))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<IntersectsPrefixTreeQuery>()
            .is_some_and(|o| self.avptq.base.equals_to(&o.avptq.base))
    }

    fn query_hash(&self) -> i32 {
        self.avptq.base.hash_code(self.class_hash())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial.prefix.IntersectsPrefixTreeQuery"
    }
}
