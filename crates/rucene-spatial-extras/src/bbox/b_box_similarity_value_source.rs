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

//! The types declared in `BBoxSimilarityValueSource.java`, of the Java
//! package `org.apache.lucene.spatial.bbox`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::double_values::{with_default, DoubleValues};
use rucene::search::double_values_source::{constant, DoubleValuesSource};
use rucene::search::similarities::Explanation;
use spatial4j::shape::rectangle::Rectangle;

use crate::shape_values::ShapeValues;
use crate::shape_values_source::ShapeValuesSource;

/// Equivalent to `org.apache.lucene.spatial.bbox.BBoxSimilarityValueSource`
/// (`public abstract class BBoxSimilarityValueSource extends
/// DoubleValuesSource`): a base class for calculating a spatial relevance
/// rank per document from a provided `ShapeValuesSource` returning a
/// `Rectangle` per-document.
///
/// Implementors: [`similarity_description`](Self::similarity_description)
/// and [`score`](Self::score). The `DoubleValuesSource` methods of the class
/// are the `similarity_*` functions of this module, which an implementor's
/// `DoubleValuesSource` implementation calls.
pub trait BBoxSimilarityValueSource: DoubleValuesSource {
    /// `bboxValueSource`.
    fn bbox_value_source(&self) -> &Arc<dyn ShapeValuesSource>;

    /// `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str;

    /// `similarityDescription()`: a description of the similarity, for
    /// `toString`.
    fn similarity_description(&self) -> String;

    /// `score(Rectangle rect, AtomicReference<Explanation> exp)`: return a
    /// relevancy score. If `exp` is provided then diagnostic information is
    /// added.
    fn score(&self, rect: &dyn Rectangle, exp: Option<&mut Option<Explanation>>) -> f64;
}

/// `BBoxSimilarityValueSource.toString()`.
pub fn similarity_to_string<S: BBoxSimilarityValueSource + ?Sized>(s: &S) -> String {
    format!("{}({},{})", s.simple_name(), s.bbox_value_source(), s.similarity_description())
}

/// The anonymous `DoubleValues` of `getValues`.
struct SimilarityValues<S: ?Sized> {
    source: Arc<S>,
    shape_values: Box<dyn ShapeValues>,
}

/// `(Rectangle) shape`.
fn as_rectangle(shape: &dyn spatial4j::shape::shape::Shape) -> &dyn Rectangle {
    shape.as_rectangle().unwrap_or_else(|| {
        panic!(
            "java.lang.ClassCastException: class {} cannot be cast to class org.locationtech.spatial4j.shape.Rectangle",
            shape.java_class_name()
        )
    })
}

impl<S: BBoxSimilarityValueSource + ?Sized> DoubleValues for SimilarityValues<S> {
    fn double_value(&mut self) -> Result<f64> {
        let shape = self.shape_values.value()?;
        Ok(self.source.score(as_rectangle(&*shape), None))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.shape_values.advance_exact(doc)
    }
}

/// `BBoxSimilarityValueSource.getValues(LeafReaderContext readerContext,
/// DoubleValues scores)`.
///
/// # Errors
///
/// The errors of the shape values source.
pub fn similarity_get_values<'a, S: BBoxSimilarityValueSource + 'static>(
    s: Arc<S>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn DoubleValues + 'a>> {
    let shape_values = s.bbox_value_source().get_values(reader_context)?;
    Ok(Box::new(with_default(SimilarityValues { source: s, shape_values }, 0.0)))
}

/// `BBoxSimilarityValueSource.equals(Object o)`: same class and equal
/// `bboxValueSource`.
pub fn similarity_eq<S: BBoxSimilarityValueSource + 'static>(s: &S, other: &dyn DoubleValuesSource) -> bool {
    match other.as_any().downcast_ref::<S>() {
        Some(that) => {
            std::ptr::eq(s, that) || s.bbox_value_source().source_eq(&**that.bbox_value_source())
        }
        None => false,
    }
}

/// `BBoxSimilarityValueSource.hashCode()`.
pub fn similarity_hash<S: BBoxSimilarityValueSource + ?Sized>(s: &S) -> i32 {
    s.bbox_value_source().source_hash()
}

/// `BBoxSimilarityValueSource.explain(LeafReaderContext ctx, int docId,
/// Explanation scoreExplanation)`.
///
/// # Errors
///
/// The errors of the values.
pub fn similarity_explain<S: BBoxSimilarityValueSource + 'static>(
    s: Arc<S>,
    ctx: &LeafReaderContext,
    doc_id: i32,
    score_explanation: &Explanation,
) -> Result<Explanation> {
    let scores = constant(score_explanation.value().double_value()).get_values(ctx, None)?;
    let mut dv = Arc::clone(&s).get_values(ctx, Some(scores))?;
    if dv.advance_exact(doc_id)? {
        let mut explanation: Option<Explanation> = None;
        let mut shape_values = s.bbox_value_source().get_values(ctx)?;
        if shape_values.advance_exact(doc_id)? {
            let shape = shape_values.value()?;
            s.score(as_rectangle(&*shape), Some(&mut explanation));
            // `explanation.get()`, set by `score`
            return Ok(explanation.unwrap_or_else(|| panic!("java.lang.NullPointerException")));
        }
    }
    Ok(Explanation::no_match(similarity_to_string(&*s), Vec::new()))
}
