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

//! Port of `LinearFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_fmt::float_to_string;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{float_to_int_bits, ValueSource, ValueSourceContext};

/// `LinearFloatFunction` implements a linear function over another value
/// source: `slope * float(source) + intercept`.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.LinearFloatFunction`.
/// Normally used as an argument to a `FunctionQuery`.
#[derive(Debug, Clone)]
pub struct LinearFloatFunction {
    source: Arc<dyn ValueSource>,
    slope: f32,
    intercept: f32,
}

impl LinearFloatFunction {
    /// Equivalent to `LinearFloatFunction(ValueSource, float, float)`.
    pub fn new(source: Arc<dyn ValueSource>, slope: f32, intercept: f32) -> Self {
        Self {
            source,
            slope,
            intercept,
        }
    }
}

/// The anonymous `FloatDocValues` of `LinearFloatFunction.getValues`.
struct LinearFloatValues<'a> {
    this: Arc<LinearFloatFunction>,
    vals: Box<dyn FunctionValues + 'a>,
}

impl FunctionValues for LinearFloatValues<'_> {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        Ok(self.vals.float_val(doc)? * self.this.slope + self.this.intercept)
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        self.vals.exists(doc)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "{}*float({})+{}",
            float_to_string(self.this.slope),
            self.vals.to_doc_string(doc)?,
            float_to_string(self.this.intercept)
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

impl FloatDocValues for LinearFloatValues<'_> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for LinearFloatFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let vals = Arc::clone(&self.source).get_values(context, reader_context)?;
        Ok(Box::new(LinearFloatValues { this: self, vals }))
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        self.source.create_weight(context, searcher)
    }

    /// `LinearFloatFunction.class == o.getClass()` with equal slope,
    /// intercept and source.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<LinearFloatFunction>()
            .is_some_and(|other| {
                self.slope == other.slope
                    && self.intercept == other.intercept
                    && self.source.value_source_eq(&*other.source)
            })
    }

    fn value_source_hash(&self) -> i32 {
        let mut h = float_to_int_bits(self.slope);
        h = ((h as u32) >> 2) as i32 | (h << 30);
        h = h.wrapping_add(float_to_int_bits(self.intercept));
        h ^= (h << 14) | ((h as u32) >> 19) as i32;
        h.wrapping_add(self.source.value_source_hash())
    }

    fn description(&self) -> String {
        format!(
            "{}*float({})+{}",
            float_to_string(self.slope),
            self.source.description(),
            float_to_string(self.intercept)
        )
    }
}
