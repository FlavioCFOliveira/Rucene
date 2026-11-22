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

//! The types declared in `PointVectorStrategy.java`, of the Java package
//! `org.apache.lucene.spatial.vector`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock, Weak};

use rucene::error::{LuceneError, Result};
use rucene::document::{DoubleDocValuesField, DoublePoint, FieldType, NumericValue, StoredField};
use rucene::index::{DocValuesType, IndexableField, IndexableFieldType, LeafReaderContext};
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::{
    BooleanQueryBuilder, ConstantScoreQuery, ConstantScoreScorer, ConstantScoreWeight, ConstantScoreWeightImpl,
    DefaultScorerSupplier, IndexSearcher, Occur, Query, QueryVisitor, ScoreMode, ScorerSupplier, Weight,
};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;

use crate::jdk::{d2s, hash_double};
use crate::query::spatial_args::SpatialArgs;
use crate::query::spatial_operation::SpatialOperation;
use crate::query::unsupported_spatial_operation::UnsupportedSpatialOperation;
use crate::search_util::{share, ScorerDisi, ScorerScores};
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};
use crate::vector::distance_value_source::DistanceValueSource;

/// `DEFAULT_FIELDTYPE`: pointValues + docValues.
pub static DEFAULT_FIELDTYPE: LazyLock<FieldType> = LazyLock::new(|| {
    // Default: pointValues + docValues
    let field_type = FieldType::new();
    field_type.set_dimensions(1, 8).expect("unfrozen"); // pointValues (assume Double)
    field_type.set_doc_values_type(DocValuesType::NUMERIC).expect("unfrozen"); // docValues
    field_type.set_stored(false).expect("unfrozen");
    field_type.freeze();
    field_type
});

/// Equivalent to `org.apache.lucene.spatial.vector.PointVectorStrategy`
/// (`public class PointVectorStrategy extends SpatialStrategy`): simple
/// `SpatialStrategy` which represents Points in two numeric fields. The
/// Strategy's best feature is decent distance sort.
///
/// Characteristics: only indexes points; can query by a rectangle or circle
/// (`Intersects` and `IsWithin`); requires point values for querying and
/// doc values for distance sort/relevancy.
///
/// The strategy is shared as an `Arc` (its distance value source refers to
/// it).
#[derive(Debug)]
pub struct PointVectorStrategy {
    base: SpatialStrategyBase,
    field_name_x: String,
    field_name_y: String,
    fields_len: usize,
    has_stored: bool,
    has_doc_vals: bool,
    has_point_vals: bool,
    me: Weak<PointVectorStrategy>,
}

impl PointVectorStrategy {
    /// `SUFFIX_X`.
    pub const SUFFIX_X: &'static str = "__x";
    /// `SUFFIX_Y`.
    pub const SUFFIX_Y: &'static str = "__y";

    /// `newInstance(SpatialContext ctx, String fieldNamePrefix)`: create a
    /// new `PointVectorStrategy` instance that uses `DoublePoint` and
    /// `DoublePoint.newRangeQuery`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new_instance(ctx: Arc<SpatialContext>, field_name_prefix: &str) -> Result<Arc<PointVectorStrategy>> {
        Self::new(ctx, field_name_prefix, &DEFAULT_FIELDTYPE)
    }

    /// `PointVectorStrategy(SpatialContext ctx, String fieldNamePrefix,
    /// FieldType fieldType)`: create a new instance configured with the
    /// provided `FieldType`. This allows options like stored, docValues and
    /// pointValues to be configured.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(ctx: Arc<SpatialContext>, field_name_prefix: &str, field_type: &FieldType) -> Result<Arc<PointVectorStrategy>> {
        let base = SpatialStrategyBase::new(ctx, field_name_prefix)?;
        let mut num_pairs = 0;
        let has_stored = field_type.stored();
        if has_stored {
            num_pairs += 1;
        }
        let has_doc_vals = field_type.doc_values_type() != DocValuesType::NONE;
        if has_doc_vals {
            num_pairs += 1;
        }
        let has_point_vals = field_type.point_dimension_count() > 0;
        if has_point_vals {
            num_pairs += 1;
        }
        Ok(Arc::new_cyclic(|me| PointVectorStrategy {
            base,
            field_name_x: format!("{field_name_prefix}{}", Self::SUFFIX_X),
            field_name_y: format!("{field_name_prefix}{}", Self::SUFFIX_Y),
            fields_len: num_pairs * 2,
            has_stored,
            has_doc_vals,
            has_point_vals,
            me: me.clone(),
        }))
    }

    fn me(&self) -> Arc<PointVectorStrategy> {
        self.me.upgrade().expect("a PointVectorStrategy is shared as an Arc")
    }

    /// `getFieldNameX()`.
    pub(crate) fn get_field_name_x(&self) -> &str {
        &self.field_name_x
    }

    /// `getFieldNameY()`.
    pub(crate) fn get_field_name_y(&self) -> &str {
        &self.field_name_y
    }

    /// `createIndexableFields(Point point)`.
    ///
    /// # Errors
    ///
    /// The errors of the field constructors.
    pub fn create_indexable_fields_point(&self, point: &dyn Point) -> Result<Vec<Arc<dyn IndexableField>>> {
        let mut fields: Vec<Arc<dyn IndexableField>> = Vec::with_capacity(self.fields_len);
        if self.has_stored {
            fields.push(Arc::new(StoredField::new_number(self.field_name_x.clone(), NumericValue::Double(point.get_x()))?));
            fields.push(Arc::new(StoredField::new_number(self.field_name_y.clone(), NumericValue::Double(point.get_y()))?));
        }
        if self.has_doc_vals {
            fields.push(Arc::new(DoubleDocValuesField::new(self.field_name_x.clone(), point.get_x())));
            fields.push(Arc::new(DoubleDocValuesField::new(self.field_name_y.clone(), point.get_y())));
        }
        if self.has_point_vals {
            fields.push(Arc::new(DoublePoint::new(self.field_name_x.clone(), &[point.get_x()])?));
            fields.push(Arc::new(DoublePoint::new(self.field_name_y.clone(), &[point.get_y()])?));
        }
        debug_assert_eq!(fields.len(), self.fields_len);
        Ok(fields)
    }

    /// `makeWithin(Rectangle bbox)`: constructs a query to retrieve
    /// documents that fully contain the input envelope.
    fn make_within(&self, bbox: &dyn Rectangle) -> Result<Arc<dyn Query>> {
        let mut bq = BooleanQueryBuilder::new();
        let must = Occur::MUST;
        if bbox.get_crosses_date_line() {
            // use null as performance trick since no data will be beyond the world bounds
            bq.add(self.range_query(&self.field_name_x, None, Some(bbox.get_max_x()))?, Occur::SHOULD)?;
            bq.add(self.range_query(&self.field_name_x, Some(bbox.get_min_x()), None)?, Occur::SHOULD)?;
            bq.set_minimum_number_should_match(1); // must match at least one of the SHOULD
        } else {
            bq.add(self.range_query(&self.field_name_x, Some(bbox.get_min_x()), Some(bbox.get_max_x()))?, must)?;
        }
        bq.add(self.range_query(&self.field_name_y, Some(bbox.get_min_y()), Some(bbox.get_max_y()))?, must)?;
        Ok(Arc::new(bq.build()))
    }

    /// `rangeQuery(String fieldName, Double min, Double max)`: returns a
    /// numeric range query based on FieldType `DoublePoint` is used if
    /// field type indexes point values.
    fn range_query(&self, field_name: &str, min: Option<f64>, max: Option<f64>) -> Result<Arc<dyn Query>> {
        if self.has_point_vals {
            let min = min.unwrap_or(f64::NEG_INFINITY);
            let max = max.unwrap_or(f64::INFINITY);
            return DoublePoint::new_range_query(field_name, &[min], &[max]);
        }
        // TODO try doc-value range query?
        Err(LuceneError::unsupported_operation("An index is required for this operation."))
    }
}

impl fmt::Display for PointVectorStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_named("PointVectorStrategy"))
    }
}

impl SpatialStrategy for PointVectorStrategy {
    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base
    }

    fn create_indexable_fields(&self, shape: &Arc<dyn Shape>) -> Result<Vec<Arc<dyn IndexableField>>> {
        if let Some(point) = shape.as_point() {
            return self.create_indexable_fields_point(point);
        }
        Err(LuceneError::unsupported_operation(format!("Can only index Point, not {shape}")))
    }

    fn make_distance_value_source(&self, query_point: &Arc<dyn Point>, multiplier: f64) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(Arc::new(DistanceValueSource::new(self.me(), Arc::clone(query_point), multiplier)))
    }

    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        if !SpatialOperation::is(args.get_operation(), &[SpatialOperation::Intersects, SpatialOperation::IsWithin]) {
            return Err(UnsupportedSpatialOperation::new(args.get_operation()));
        }
        let shape = &args.get_shape();
        if let Some(bbox) = shape.as_rectangle() {
            Ok(Arc::new(ConstantScoreQuery::new(self.make_within(bbox)?)))
        } else if let Some(circle) = shape.as_circle() {
            let bbox = circle.get_bounding_box();
            Ok(Arc::new(DistanceRangeQuery::new(
                self.make_within(&*bbox)?,
                self.make_distance_value_source_default(&circle.get_center())?,
                circle.get_radius(),
            )))
        } else {
            Err(LuceneError::unsupported_operation(format!(
                "Only Rectangles and Circles are currently supported, found [class {}]",
                shape.java_class_name()
            ))) // TODO
        }
    }
}

/// Equivalent to `org.apache.lucene.spatial.vector.PointVectorStrategy.DistanceRangeQuery`
/// (`private static class DistanceRangeQuery extends Query`): the query of
/// points within a distance of a point, verified over an approximation.
#[derive(Debug, Clone)]
pub(crate) struct DistanceRangeQuery {
    inner: Arc<dyn Query>,
    distance_source: Arc<dyn DoubleValuesSource>,
    limit: f64,
}

impl DistanceRangeQuery {
    fn new(inner: Arc<dyn Query>, distance_source: Arc<dyn DoubleValuesSource>, limit: f64) -> Self {
        Self {
            inner,
            distance_source,
            limit,
        }
    }
}

/// The `ConstantScoreWeight` of `DistanceRangeQuery.createWeight`.
#[derive(Debug)]
struct DistanceRangeWeight {
    w: Arc<dyn Weight>,
    distance_source: Arc<dyn DoubleValuesSource>,
    limit: f64,
    score_mode: ScoreMode,
}

/// The `TwoPhaseIterator` of `DistanceRangeWeight.scorerSupplier`.
struct DistanceTwoPhase {
    approximation: ScorerDisi,
    v: Box<dyn DoubleValues>,
    limit: f64,
}

impl TwoPhaseIterator for DistanceTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.approximation.doc_id();
        Ok(self.v.advance_exact(doc)? && self.v.double_value()? <= self.limit)
    }

    fn match_cost(&self) -> f32 {
        100.0 // distance calculation can be heavy!
    }
}

impl ConstantScoreWeightImpl for DistanceRangeWeight {
    fn scorer_supplier(&self, context: &LeafReaderContext, score: f32) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(scorer) = self.w.scorer(context)? else {
            return Ok(None);
        };
        let scorer = share(scorer);
        let v = Arc::clone(&self.distance_source).get_values(context, Some(Box::new(ScorerScores(scorer.clone()))))?;
        let two_phase = DistanceTwoPhase {
            approximation: ScorerDisi(scorer),
            v,
            limit: self.limit,
        };
        let scorer = ConstantScoreScorer::from_two_phase(score, self.score_mode, Box::new(two_phase));
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.distance_source.is_cacheable(ctx)
    }
}

impl Query for DistanceRangeQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "DistanceRangeQuery({}; {} < {})",
            self.inner.to_query_string(field),
            self.distance_source.to_source_string(),
            d2s(self.limit)
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.inner.rewrite(searcher)? {
            None => Ok(None),
            Some(rewritten) => Ok(Some(Arc::new(DistanceRangeQuery::new(
                rewritten,
                Arc::clone(&self.distance_source),
                self.limit,
            )))),
        }
    }

    fn create_weight(&self, searcher: &IndexSearcher, score_mode: ScoreMode, boost: f32) -> Result<Arc<dyn Weight>> {
        let w = self.inner.create_weight(searcher, score_mode, 1.0)?;
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            DistanceRangeWeight {
                w,
                distance_source: Arc::clone(&self.distance_source),
                limit: self.limit,
                score_mode,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        let Some(that) = other.as_any().downcast_ref::<DistanceRangeQuery>() else {
            return false;
        };
        self.inner.query_eq(&*that.inner) && self.distance_source.source_eq(&*that.distance_source) && self.limit == that.limit
    }

    /// `Objects.hash(inner, distanceSource, limit)`.
    fn query_hash(&self) -> i32 {
        let mut result = 1i32;
        result = result.wrapping_mul(31).wrapping_add(self.inner.query_hash());
        result = result.wrapping_mul(31).wrapping_add(self.distance_source.source_hash());
        result = result.wrapping_mul(31).wrapping_add(hash_double(self.limit));
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial.vector.PointVectorStrategy$DistanceRangeQuery"
    }
}
