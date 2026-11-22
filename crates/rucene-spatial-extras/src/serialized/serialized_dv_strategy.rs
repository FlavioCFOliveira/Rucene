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

//! The types declared in `SerializedDVStrategy.java`, of the Java package
//! `org.apache.lucene.spatial.serialized`.

use std::any::Any;
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::document::BinaryDocValuesField;
use rucene::error::{LuceneError, Result};
use rucene::index::doc_values::DocValues;
use rucene::index::{BinaryDocValues, IndexableField, LeafReaderContext};
use rucene::search::doc_id_set_iterator;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{
    ConstantScoreScorer, ConstantScoreWeight, ConstantScoreWeightImpl, DefaultScorerSupplier, IndexSearcher, Query,
    QueryVisitor, ScoreMode, ScorerSupplier, SegmentCacheable, Weight,
};
use rucene::util::BytesRef;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::io::binary_codec::BinaryCodec;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::jdk::string_hash;
use crate::query::spatial_args::SpatialArgs;
use crate::shape_values::ShapeValues;
use crate::shape_values_source::ShapeValuesSource;
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};
use crate::util::distance_to_shape_value_source::DistanceToShapeValueSource;
use crate::util::shape_values_predicate::ShapeValuesPredicate;

/// Equivalent to `org.apache.lucene.spatial.serialized.SerializedDVStrategy`
/// (`public class SerializedDVStrategy extends SpatialStrategy`): a
/// SpatialStrategy based on serializing a Shape stored into BinaryDocValues.
/// This is not at all fast; it's designed to be used in conjunction with
/// another index based SpatialStrategy that is approximated (like
/// `RecursivePrefixTreeStrategy`) to add precision or eventually make more
/// specific / advanced calculations on the per-document geometry. The
/// serialization uses spatial4j's `BinaryCodec`.
#[derive(Debug)]
pub struct SerializedDVStrategy {
    base: SpatialStrategyBase,
    /// `indexLastBufSize`: a heuristic; 8KB default on first run.
    index_last_buf_size: AtomicI32,
}

impl SerializedDVStrategy {
    /// `SerializedDVStrategy(SpatialContext ctx, String fieldName)`:
    /// constructs the spatial strategy with its mandatory arguments.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(ctx: Arc<SpatialContext>, field_name: &str) -> Result<Self> {
        Ok(Self {
            base: SpatialStrategyBase::new(ctx, field_name)?,
            index_last_buf_size: AtomicI32::new(8 * 1024),
        })
    }

    /// `makeShapeValueSource()`: provides access to each shape per document.
    pub fn make_shape_value_source(&self) -> Arc<dyn ShapeValuesSource> {
        // TODO raise to SpatialStrategy
        Arc::new(ShapeDocValueSource::new(self.get_field_name(), self.base.ctx.get_binary_codec()))
    }
}

impl fmt::Display for SerializedDVStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_named("SerializedDVStrategy"))
    }
}

impl SpatialStrategy for SerializedDVStrategy {
    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base
    }

    fn create_indexable_fields(&self, shape: &Arc<dyn Shape>) -> Result<Vec<Arc<dyn IndexableField>>> {
        // 50% headroom over last
        let buf_size = 128.max((f64::from(self.index_last_buf_size.load(Ordering::Relaxed)) * 1.5) as i32);
        let mut byte_stream: Vec<u8> = Vec::with_capacity(buf_size as usize);
        self.base
            .ctx
            .get_binary_codec()
            .write_shape(&mut byte_stream, &**shape)
            .map_err(LuceneError::runtime_exception)?;
        let bytes_ref = BytesRef::new(byte_stream); // receiver of byteStream's bytes
        self.index_last_buf_size.store(bytes_ref.length as i32, Ordering::Relaxed); // cache heuristic
        Ok(vec![Arc::new(BinaryDocValuesField::new(self.get_field_name().to_string(), bytes_ref))])
    }

    fn make_distance_value_source(&self, query_point: &Arc<dyn Point>, multiplier: f64) -> Result<Arc<dyn DoubleValuesSource>> {
        // TODO if makeShapeValueSource gets lifted to the top; this could become a generic impl.
        Ok(Arc::new(DistanceToShapeValueSource::new(
            self.make_shape_value_source(),
            Arc::clone(query_point),
            multiplier,
            &self.base.ctx,
        )))
    }

    /// Returns a Query that should be used in a random-access fashion. Use
    /// in another manner will be SLOW.
    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        let shape_value_source = self.make_shape_value_source();
        let predicate_value_source =
            ShapeValuesPredicate::new(shape_value_source, args.get_operation(), args.get_shape());
        Ok(Arc::new(PredicateValueSourceQuery::new(Arc::new(predicate_value_source))))
    }
}

/// Equivalent to `org.apache.lucene.spatial.serialized.SerializedDVStrategy.PredicateValueSourceQuery`
/// (package-private `static class PredicateValueSourceQuery extends
/// Query`): a Query that considers all documents (`DocIdSetIterator.all`)
/// and verifies each with the predicate. This is not fast.
#[derive(Debug, Clone)]
pub(crate) struct PredicateValueSourceQuery {
    predicate_value_source: Arc<ShapeValuesPredicate>,
}

impl PredicateValueSourceQuery {
    /// `PredicateValueSourceQuery(ShapeValuesPredicate predicateValueSource)`.
    pub(crate) fn new(predicate_value_source: Arc<ShapeValuesPredicate>) -> Self {
        Self { predicate_value_source }
    }
}

/// The `ConstantScoreWeight` of `PredicateValueSourceQuery.createWeight`.
#[derive(Debug)]
struct PredicateWeight {
    predicate_value_source: Arc<ShapeValuesPredicate>,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for PredicateWeight {
    fn scorer_supplier(&self, context: &LeafReaderContext, score: f32) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let approximation = doc_id_set_iterator::all(context.leaf_reader().max_doc())?;
        let it = self.predicate_value_source.iterator(context, Box::new(approximation))?;
        let scorer = ConstantScoreScorer::from_two_phase(score, self.score_mode, it);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.predicate_value_source.is_cacheable(ctx)
    }
}

impl Query for PredicateValueSourceQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!("PredicateValueSourceQuery({})", self.predicate_value_source)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(&self, _searcher: &IndexSearcher, score_mode: ScoreMode, boost: f32) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            PredicateWeight {
                predicate_value_source: Arc::clone(&self.predicate_value_source),
                score_mode,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<PredicateValueSourceQuery>()
            .is_some_and(|o| self.predicate_value_source.predicate_eq(&o.predicate_value_source))
    }

    fn query_hash(&self) -> i32 {
        self.class_hash()
            .wrapping_add(31i32.wrapping_mul(self.predicate_value_source.predicate_hash()))
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial.serialized.SerializedDVStrategy$PredicateValueSourceQuery"
    }
}

/// Equivalent to `org.apache.lucene.spatial.serialized.SerializedDVStrategy.ShapeDocValueSource`
/// (package-private `static class ShapeDocValueSource extends
/// ShapeValuesSource`): implements a ShapeValueSource by deserializing a
/// Shape from BinaryDocValues using `BinaryCodec`.
pub(crate) struct ShapeDocValueSource {
    field_name: String,
    binary_codec: Arc<dyn BinaryCodec>, // spatial4j
}

impl fmt::Debug for ShapeDocValueSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl ShapeDocValueSource {
    /// `ShapeDocValueSource(String fieldName, BinaryCodec binaryCodec)`.
    fn new(field_name: &str, binary_codec: Arc<dyn BinaryCodec>) -> Self {
        Self {
            field_name: field_name.to_string(),
            binary_codec,
        }
    }
}

/// The anonymous `ShapeValues` of `ShapeDocValueSource.getValues`.
struct DocValueShapes {
    doc_values: Box<dyn BinaryDocValues>,
    binary_codec: Arc<dyn BinaryCodec>,
}

impl ShapeValues for DocValueShapes {
    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.doc_values.advance_exact(doc)
    }

    fn value(&mut self) -> Result<Arc<dyn Shape>> {
        let bytes_ref = self.doc_values.binary_value()?;
        let mut data_input = &bytes_ref.bytes[bytes_ref.offset..bytes_ref.offset + bytes_ref.length];
        self.binary_codec.read_shape(&mut data_input)
    }
}

impl SegmentCacheable for ShapeDocValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        DocValues::is_cacheable(ctx, &[&self.field_name])
    }
}

impl ShapeValuesSource for ShapeDocValueSource {
    fn get_values(&self, reader_context: &LeafReaderContext) -> Result<Box<dyn ShapeValues>> {
        let doc_values = DocValues::get_binary(&*reader_context.leaf_reader(), &self.field_name)?;
        Ok(Box::new(DocValueShapes {
            doc_values,
            binary_codec: Arc::clone(&self.binary_codec),
        }))
    }

    fn source_eq(&self, other: &dyn ShapeValuesSource) -> bool {
        match other.as_any().downcast_ref::<ShapeDocValueSource>() {
            Some(that) => std::ptr::eq(self, that) || self.field_name == that.field_name,
            None => false,
        }
    }

    fn source_hash(&self) -> i32 {
        string_hash(&self.field_name)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl fmt::Display for ShapeDocValueSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "shapeDocVal({})", self.field_name)
    }
}
