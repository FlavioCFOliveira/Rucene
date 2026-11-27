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

//! The types declared in `PointInGeo3DShapeQuery.java`, of the Java package
//! `org.apache.lucene.spatial3d`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::{
    ConstantScoreScorer, ConstantScoreWeight, ConstantScoreWeightImpl, DefaultScorerSupplier,
    IndexSearcher, Query, QueryVisitor, ScoreMode, ScorerSupplier, Weight,
};
use rucene::util::doc_id_set::DocIdSetBuilder;
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::{Accountable, RamUsageEstimator};

use crate::geom::geo_shape::GeoShape;
use crate::geom::standard_objects::java_shallow_size_of_instance;
use crate::geom::xyz_bounds::XYZBounds;
use crate::point_in_shape_intersect_visitor::PointInShapeIntersectVisitor;

/// Query that matches documents whose 3D point is within the given shape.
///
/// Equivalent to `org.apache.lucene.spatial3d.PointInGeo3DShapeQuery`.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
#[derive(Clone)]
pub struct PointInGeo3DShapeQuery {
    field: String,
    shape: Arc<dyn GeoShape>,
    shape_bounds: XYZBounds,
}

impl PointInGeo3DShapeQuery {
    /// `RamUsageEstimator.shallowSizeOfInstance(PointInGeo3DShapeQuery.class)`:
    /// `Query.CLASS_NAME_HASH` and the three reference fields.
    const BASE_RAM_BYTES: i64 = RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 4
            + 3 * RamUsageEstimator::NUM_BYTES_OBJECT_REF,
    );

    /// `new PointInGeo3DShapeQuery(String field, GeoShape shape)`.
    pub(crate) fn new(field: &str, shape: Arc<dyn GeoShape>) -> Self {
        let mut shape_bounds = XYZBounds::new();
        shape.get_bounds(&mut shape_bounds);
        Self {
            field: field.to_string(),
            shape,
            shape_bounds,
        }
    }

    /// `getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// `getShape()`.
    pub fn get_shape(&self) -> &Arc<dyn GeoShape> {
        &self.shape
    }

    fn equals_to(&self, other: &PointInGeo3DShapeQuery) -> bool {
        self.field == other.field && self.shape.java_equals(other.shape.as_any())
    }
}

impl fmt::Debug for PointInGeo3DShapeQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_query_string(""))
    }
}

/// The `ConstantScoreWeight` subclass `createWeight` returns.
#[derive(Debug)]
struct PointInGeo3DShapeWeight {
    query: PointInGeo3DShapeQuery,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for PointInGeo3DShapeWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let Some(values) = reader.get_point_values(&self.query.field)? else {
            return Ok(None);
        };
        let mut result = DocIdSetBuilder::from_point_values(reader.max_doc(), values.as_ref())?;
        {
            let mut visitor = PointInShapeIntersectVisitor::new(
                &mut result,
                self.query.shape.as_ref(),
                &self.query.shape_bounds,
            );
            values.intersect(&mut visitor)?;
        }
        let scorer =
            ConstantScoreScorer::from_iterator(score, self.score_mode, result.build()?.iterator()?);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Query for PointInGeo3DShapeQuery {
    /// `toString(String)`.
    fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::from("PointInGeo3DShapeQuery:");
        if self.field != field {
            sb.push_str(" field=");
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push_str(" Shape: ");
        sb.push_str(&self.shape.to_string());
        sb
    }

    /// `visit(QueryVisitor)`.
    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_accountable(&self) -> Option<&dyn Accountable> {
        Some(self)
    }

    /// `createWeight(IndexSearcher, ScoreMode, float)`.
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let query: Arc<dyn Query> = Arc::new(self.clone());
        Ok(Arc::new(ConstantScoreWeight::new(
            query,
            boost,
            PointInGeo3DShapeWeight {
                query: self.clone(),
                score_mode,
            },
        )))
    }

    /// `equals(Object)`: `sameClassAs(other) && equalsTo(other)`.
    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<PointInGeo3DShapeQuery>()
            .is_some_and(|other| self.equals_to(other))
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial3d.PointInGeo3DShapeQuery"
    }

    /// `hashCode()`.
    fn query_hash(&self) -> i32 {
        let mut result = self.class_hash();
        result = result
            .wrapping_mul(31)
            .wrapping_add(java_string_hash_code(&self.field));
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.shape.java_hash_code());
        result
    }
}

impl Accountable for PointInGeo3DShapeQuery {
    /// `ramBytesUsed()`: the shape and its bounds count with their shallow
    /// size, as `RamUsageEstimator.sizeOfObject` measures an object that is
    /// neither `Accountable` nor a string, array, collection or query.
    fn ram_bytes_used(&self) -> i64 {
        Self::BASE_RAM_BYTES
            + RamUsageEstimator::size_of_string(&self.field)
            + java_shallow_size_of_instance(self.shape.class_name())
            + java_shallow_size_of_instance("org.apache.lucene.spatial3d.geom.XYZBounds")
    }
}
