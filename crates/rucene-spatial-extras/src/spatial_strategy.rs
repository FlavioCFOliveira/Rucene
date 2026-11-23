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

//! The types declared in `SpatialStrategy.java`, of the Java package
//! `org.apache.lucene.spatial`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::IndexableField;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::Query;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::query::spatial_args::SpatialArgs;
use crate::util::reciprocal_double_values_source::ReciprocalDoubleValuesSource;

/// The fields of `SpatialStrategy`: `ctx` and `fieldName`.
#[derive(Debug, Clone)]
pub struct SpatialStrategyBase {
    /// `ctx`.
    pub ctx: Arc<SpatialContext>,
    field_name: String,
}

impl SpatialStrategyBase {
    /// `SpatialStrategy(SpatialContext ctx, String fieldName)`: constructs
    /// the spatial strategy with its mandatory arguments.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("fieldName is required")` for an empty
    /// field name (a `null` context or name has no Rust counterpart).
    pub fn new(ctx: Arc<SpatialContext>, field_name: &str) -> Result<Self> {
        if field_name.is_empty() {
            return Err(LuceneError::illegal_argument("fieldName is required"));
        }
        Ok(Self {
            ctx,
            field_name: field_name.to_string(),
        })
    }

    /// `getFieldName()`.
    pub fn field_name(&self) -> &str {
        &self.field_name
    }

    /// `toString()`: `getClass().getSimpleName() + " field:" + fieldName + "
    /// ctx=" + ctx`, for the class named `simple_name`.
    pub fn to_string_named(&self, simple_name: &str) -> String {
        format!("{simple_name} field:{} ctx={}", self.field_name, self.ctx)
    }
}

/// Equivalent to `org.apache.lucene.spatial.SpatialStrategy` (`public
/// abstract class`): the SpatialStrategy encapsulates an approach to
/// indexing and searching shapes, and providing distance values for them.
/// It's a simple API to unify different approaches. There are a few
/// implementations of this abstract class.
///
/// Different implementations will support different features. A strategy
/// should document these common elements: can it index more than one shape
/// per field? What types of shapes can be indexed? What types of query
/// shapes can be used? What types of query operations are supported? Are
/// there caches?
///
/// Java's `Field[]` results are `Vec<Arc<dyn IndexableField>>` (Java's
/// `Field` subclasses, such as `StoredField` and `DoublePoint`, are distinct
/// types here), and `toString()` is the [`fmt::Display`] implementation.
pub trait SpatialStrategy: fmt::Display + Send + Sync {
    /// The `SpatialStrategy` fields.
    fn strategy_base(&self) -> &SpatialStrategyBase;

    /// `getSpatialContext()`.
    fn get_spatial_context(&self) -> Arc<SpatialContext> {
        Arc::clone(&self.strategy_base().ctx)
    }

    /// `getFieldName()`: the field name used by this strategy to index and
    /// query. Some strategies use other field names.
    fn get_field_name(&self) -> &str {
        self.strategy_base().field_name()
    }

    /// `createIndexableFields(Shape shape)`: returns the IndexableField(s)
    /// from the `shape` that are to be added to the document.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if it can't use the shape;
    /// `IllegalArgumentException` and the errors of computing the fields.
    fn create_indexable_fields(
        &self,
        shape: &Arc<dyn Shape>,
    ) -> Result<Vec<Arc<dyn IndexableField>>>;

    /// `makeDistanceValueSource(Point queryPoint)`: see
    /// [`make_distance_value_source`](Self::make_distance_value_source)
    /// called with a multiplier of 1.0 (i.e. units of degrees).
    ///
    /// # Errors
    ///
    /// As [`make_distance_value_source`](Self::make_distance_value_source).
    fn make_distance_value_source_default(
        &self,
        query_point: &Arc<dyn Point>,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        self.make_distance_value_source(query_point, 1.0)
    }

    /// `makeDistanceValueSource(Point queryPoint, double multiplier)`: make
    /// a ValueSource returning the distance between the center of the
    /// indexed shape and `query_point`. If there are multiple indexed shapes
    /// then the closest one is chosen. The result is multiplied by
    /// `multiplier`, which conveniently is used to get the desired units.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` where the strategy cannot.
    fn make_distance_value_source(
        &self,
        query_point: &Arc<dyn Point>,
        multiplier: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>>;

    /// `makeQuery(SpatialArgs args)`: make a Query based principally on
    /// `SpatialOperation` and `Shape` from the supplied `args`. It should
    /// be constant scoring of 1.
    ///
    /// # Errors
    ///
    /// `UnsupportedSpatialOperation` if the strategy does not support the
    /// operation; `UnsupportedOperationException` if it does not support
    /// the shape.
    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>>;

    /// `makeRecipDistanceValueSource(Shape queryShape)` (`final`): returns
    /// a ValueSource with values ranging from 1 to 0, depending inversely on
    /// the distance from `makeDistanceValueSource`. The formula is `c /
    /// (d + c)` where 'd' is the distance and 'c' is one tenth the distance
    /// to the farthest edge from the center. Thus the scores will be 1 for
    /// indexed points at the center of the query shape and as low as ~0.1
    /// at its furthest edges.
    ///
    /// # Errors
    ///
    /// The errors of the shape factory and of `makeDistanceValueSource`.
    fn make_recip_distance_value_source(
        &self,
        query_shape: &dyn Shape,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        let ctx = &self.strategy_base().ctx;
        let bbox = query_shape.get_bounding_box();
        let corner = ctx
            .get_shape_factory()
            .point_xy(bbox.get_min_x(), bbox.get_min_y())?;
        let diagonal_dist =
            ctx.get_dist_calc()
                .distance_xy(&*corner, bbox.get_max_x(), bbox.get_max_y());
        let dist_to_edge = diagonal_dist * 0.5;
        let c = (dist_to_edge as f32) * 0.1f32; // one tenth
        let distance = self.make_distance_value_source(&query_shape.get_center(), 1.0)?;
        Ok(Arc::new(ReciprocalDoubleValuesSource::new(
            f64::from(c),
            distance,
        )))
    }
}
