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

//! The types declared in `ReciprocalDoubleValuesSource.java`, of the Java
//! package `org.apache.lucene.spatial.util`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::similarities::Explanation;
use rucene::search::{IndexSearcher, SegmentCacheable};

use crate::jdk::{d2s, double_compare, hash_double};

/// Equivalent to `org.apache.lucene.spatial.util.ReciprocalDoubleValuesSource`
/// (`public class ReciprocalDoubleValuesSource extends DoubleValuesSource`):
/// transforms a `DoubleValuesSource` using the formula `v = k / (v + k)`.
#[derive(Debug)]
pub struct ReciprocalDoubleValuesSource {
    dist_to_edge: f64,
    input: Arc<dyn DoubleValuesSource>,
}

impl ReciprocalDoubleValuesSource {
    /// `ReciprocalDoubleValuesSource(double distToEdge, DoubleValuesSource
    /// input)`: creates a `ReciprocalDoubleValuesSource`.
    pub fn new(dist_to_edge: f64, input: Arc<dyn DoubleValuesSource>) -> Self {
        Self {
            dist_to_edge,
            input,
        }
    }

    /// `recip(double in)`.
    fn recip(dist_to_edge: f64, v: f64) -> f64 {
        dist_to_edge / (v + dist_to_edge)
    }
}

struct RecipValues<'a> {
    input: Box<dyn DoubleValues + 'a>,
    dist_to_edge: f64,
}

impl DoubleValues for RecipValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        Ok(ReciprocalDoubleValuesSource::recip(
            self.dist_to_edge,
            self.input.double_value()?,
        ))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.input.advance_exact(doc)
    }
}

impl SegmentCacheable for ReciprocalDoubleValuesSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.input.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for ReciprocalDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let input = Arc::clone(&self.input).get_values(ctx, scores)?;
        Ok(Box::new(RecipValues {
            input,
            dist_to_edge: self.dist_to_edge,
        }))
    }

    fn needs_scores(&self) -> bool {
        self.input.needs_scores()
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(Arc::new(ReciprocalDoubleValuesSource::new(
            self.dist_to_edge,
            Arc::clone(&self.input).rewrite(searcher)?,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other
            .as_any()
            .downcast_ref::<ReciprocalDoubleValuesSource>()
        {
            Some(that) => {
                double_compare(that.dist_to_edge, self.dist_to_edge) == 0
                    && self.input.source_eq(&*that.input)
            }
            None => false,
        }
    }

    /// `Objects.hash(distToEdge, input)`.
    fn source_hash(&self) -> i32 {
        let mut result = 1i32;
        result = result
            .wrapping_mul(31)
            .wrapping_add(hash_double(self.dist_to_edge));
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.input.source_hash());
        result
    }

    fn to_source_string(&self) -> String {
        format!(
            "recip({}, {})",
            d2s(self.dist_to_edge),
            self.input.to_source_string()
        )
    }

    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        score_explanation: &Explanation,
    ) -> Result<Explanation> {
        let expl = Arc::clone(&self.input).explain(ctx, doc_id, score_explanation)?;
        let value = Self::recip(self.dist_to_edge, expl.value().double_value());
        let d = d2s(self.dist_to_edge);
        Ok(Explanation::matched(
            value,
            format!("{d} / (v + {d}), computed from:"),
            vec![expl],
        ))
    }
}
