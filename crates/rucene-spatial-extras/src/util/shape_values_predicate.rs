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

//! The types declared in `ShapeValuesPredicate.java`, of the Java package
//! `org.apache.lucene.spatial.util`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::SegmentCacheable;
use spatial4j::shape::shape::Shape;

use crate::query::spatial_operation::SpatialOperation;
use crate::shape_values::ShapeValues;
use crate::shape_values_source::ShapeValuesSource;

/// Equivalent to `org.apache.lucene.spatial.util.ShapeValuesPredicate`
/// (`public class ShapeValuesPredicate implements SegmentCacheable`):
/// compares a shape from a provided [`ShapeValuesSource`] with a given
/// Shape and sees if it matches a given [`SpatialOperation`] (the
/// predicate). Consumers should call [`iterator`](Self::iterator) to obtain
/// a `TwoPhaseIterator` over a particular `DocIdSetIterator`. The initial
/// `DocIdSetIterator` will be used as the approximation, and the
/// `SpatialOperation` comparison will only be performed in
/// `TwoPhaseIterator.matches()`.
#[derive(Debug)]
pub struct ShapeValuesPredicate {
    shape_valuesource: Arc<dyn ShapeValuesSource>, // the left hand side
    op: SpatialOperation,
    query_shape: Arc<dyn Shape>, // the right hand side (constant)
}

impl ShapeValuesPredicate {
    /// `ShapeValuesPredicate(ShapeValuesSource shapeValuesource,
    /// SpatialOperation op, Shape queryShape)`.
    pub fn new(
        shape_valuesource: Arc<dyn ShapeValuesSource>,
        op: SpatialOperation,
        query_shape: Arc<dyn Shape>,
    ) -> Self {
        Self {
            shape_valuesource,
            op,
            query_shape,
        }
    }

    /// `iterator(LeafReaderContext ctx, DocIdSetIterator approximation)`.
    ///
    /// # Errors
    ///
    /// The errors of `getValues`.
    pub fn iterator(
        &self,
        ctx: &LeafReaderContext,
        approximation: Box<dyn DocIdSetIterator>,
    ) -> Result<Box<dyn TwoPhaseIterator>> {
        let shape_values = self.shape_valuesource.get_values(ctx)?;
        Ok(Box::new(PredicateTwoPhase {
            approximation,
            matcher: self.matcher(shape_values),
        }))
    }

    /// The state the anonymous `TwoPhaseIterator` of `iterator` reads in
    /// `matches()`, for a caller whose approximation is its own.
    pub(crate) fn matcher(&self, shape_values: Box<dyn ShapeValues>) -> PredicateMatcher {
        PredicateMatcher {
            shape_values,
            op: self.op,
            query_shape: Arc::clone(&self.query_shape),
        }
    }

    /// The shape values of this predicate's source for `ctx`.
    pub(crate) fn values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn ShapeValues>> {
        self.shape_valuesource.get_values(ctx)
    }

    /// `equals(Object o)`.
    pub fn predicate_eq(&self, that: &ShapeValuesPredicate) -> bool {
        if std::ptr::eq(self, that) {
            return true;
        }
        if !self.shape_valuesource.source_eq(&*that.shape_valuesource) {
            return false;
        }
        if self.op != that.op {
            return false;
        }
        self.query_shape.equals(&*that.query_shape)
    }

    /// `hashCode()`.
    pub fn predicate_hash(&self) -> i32 {
        let mut result = self.shape_valuesource.source_hash();
        result = result.wrapping_mul(31).wrapping_add(self.op.hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.query_shape.hash_code());
        result
    }
}

impl fmt::Display for ShapeValuesPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}",
            self.shape_valuesource, self.op, self.query_shape
        )
    }
}

impl SegmentCacheable for ShapeValuesPredicate {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.shape_valuesource.is_cacheable(ctx)
    }
}

/// `shapeValues.advanceExact(doc) && op.evaluate(shapeValues.value(),
/// queryShape)`: the body of the anonymous iterator's `matches()`.
pub(crate) struct PredicateMatcher {
    shape_values: Box<dyn ShapeValues>,
    op: SpatialOperation,
    query_shape: Arc<dyn Shape>,
}

impl PredicateMatcher {
    /// `matches()` for the approximation positioned on `doc`.
    pub(crate) fn matches(&mut self, doc: i32) -> Result<bool> {
        Ok(self.shape_values.advance_exact(doc)?
            && self
                .op
                .evaluate(&*self.shape_values.value()?, &*self.query_shape))
    }
}

/// The anonymous `TwoPhaseIterator` of `iterator`.
struct PredicateTwoPhase {
    approximation: Box<dyn DocIdSetIterator>,
    matcher: PredicateMatcher,
}

impl TwoPhaseIterator for PredicateTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.approximation.doc_id();
        self.matcher.matches(doc)
    }

    fn match_cost(&self) -> f32 {
        100.0 // is this necessary?
    }
}
