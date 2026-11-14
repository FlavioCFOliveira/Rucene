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

//! Port of `ScaleFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_fmt::float_to_string;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{
    float_to_int_bits, top_level_context, ValueSource, ValueSourceContext,
};

/// Scales values to be between `min` and `max`.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.ScaleFloatFunction`.
/// This implementation currently traverses all of the source values to obtain
/// their min and max. It does not currently skip deleted documents, so values
/// in deleted documents may influence the scaling.
#[derive(Debug, Clone)]
pub struct ScaleFloatFunction {
    source: Arc<dyn ValueSource>,
    min: f32,
    max: f32,
}

/// The minimum and maximum source values over the whole index, kept in the
/// context under the function's identity.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.valuesource.ScaleFloatFunction.ScaleInfo`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ScaleInfo {
    min_val: f32,
    max_val: f32,
}

impl ScaleFloatFunction {
    /// Equivalent to `ScaleFloatFunction(ValueSource, float, float)`.
    pub fn new(source: Arc<dyn ValueSource>, min: f32, max: f32) -> Self {
        Self { source, min, max }
    }

    /// Equivalent to the private
    /// `ScaleFloatFunction.createScaleInfo(Map, LeafReaderContext)`: the
    /// finite minimum and maximum of the source over every leaf of the
    /// top-level reader, stored in the context.
    fn create_scale_info(
        &self,
        context: &ValueSourceContext<'_>,
        reader_context: &LeafReaderContext,
    ) -> Result<ScaleInfo> {
        let leaves = top_level_context(reader_context).leaves();

        let mut min_val = f32::INFINITY;
        let mut max_val = f32::NEG_INFINITY;

        for leaf in &leaves {
            let max_doc = leaf.leaf_reader().max_doc();
            let mut vals = Arc::clone(&self.source).get_values(context, leaf)?;
            for i in 0..max_doc {
                if !vals.exists(i)? {
                    continue;
                }
                let val = vals.float_val(i)?;
                if (val.to_bits() & (0xff << 23)) == 0xff << 23 {
                    // if the exponent in the float is all ones, then this is
                    // +Inf, -Inf or NaN which don't make sense to factor into
                    // the scale function
                    continue;
                }
                if val < min_val {
                    min_val = val;
                }
                if val > max_val {
                    max_val = val;
                }
            }
        }

        if min_val == f32::INFINITY {
            // must have been an empty index
            min_val = 0.0;
            max_val = 0.0;
        }

        let scale_info = ScaleInfo { min_val, max_val };
        context.put(self, Arc::new(scale_info));
        Ok(scale_info)
    }
}

/// The anonymous `FloatDocValues` of `ScaleFloatFunction.getValues`.
struct ScaleFloatValues<'a> {
    this: Arc<ScaleFloatFunction>,
    vals: Box<dyn FunctionValues + 'a>,
    scale: f32,
    min_source: f32,
    max_source: f32,
}

impl FunctionValues for ScaleFloatValues<'_> {
    fn exists(&mut self, doc: i32) -> Result<bool> {
        self.vals.exists(doc)
    }

    fn float_val(&mut self, doc: i32) -> Result<f32> {
        Ok((self.vals.float_val(doc)? - self.min_source) * self.scale + self.this.min)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "scale({},toMin={},toMax={},fromMin={},fromMax={})",
            self.vals.to_doc_string(doc)?,
            float_to_string(self.this.min),
            float_to_string(self.this.max),
            float_to_string(self.min_source),
            float_to_string(self.max_source)
        ))
    }

    float_doc_values_members!(
        byte_val,
        short_val,
        int_val,
        long_val,
        bool_val,
        double_val,
        str_val,
        object_val,
        get_value_filler
    );
}

impl FloatDocValues for ScaleFloatValues<'_> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for ScaleFloatFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let stored = context
            .get(&*self)
            .and_then(|info| info.downcast_ref::<ScaleInfo>().copied());
        let scale_info = match stored {
            Some(scale_info) => scale_info,
            None => self.create_scale_info(context, reader_context)?,
        };

        let scale = if scale_info.max_val - scale_info.min_val == 0.0 {
            0.0
        } else {
            (self.max - self.min) / (scale_info.max_val - scale_info.min_val)
        };
        let min_source = scale_info.min_val;
        let max_source = scale_info.max_val;

        let vals = Arc::clone(&self.source).get_values(context, reader_context)?;
        Ok(Box::new(ScaleFloatValues {
            this: self,
            vals,
            scale,
            min_source,
            max_source,
        }))
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        self.source.create_weight(context, searcher)
    }

    fn value_source_hash(&self) -> i32 {
        let mut h = float_to_int_bits(self.min);
        h = h.wrapping_mul(29);
        h = h.wrapping_add(float_to_int_bits(self.max));
        h = h.wrapping_mul(29);
        h = h.wrapping_add(self.source.value_source_hash());
        h
    }

    /// `ScaleFloatFunction.class == o.getClass()` with equal bounds and
    /// source.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<ScaleFloatFunction>()
            .is_some_and(|other| {
                self.min == other.min
                    && self.max == other.max
                    && self.source.value_source_eq(&*other.source)
            })
    }

    fn description(&self) -> String {
        format!(
            "scale({},{},{})",
            self.source.description(),
            float_to_string(self.min),
            float_to_string(self.max)
        )
    }
}
