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

//! Port of `ReciprocalFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_fmt::float_to_string;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{float_to_int_bits, ValueSource, ValueSourceContext};

/// `ReciprocalFloatFunction` implements a reciprocal function
/// `f(x) = a/(mx+b)`, based on the float value of a field or function as
/// exported by another value source.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.ReciprocalFloatFunction`.
/// When `a` and `b` are equal, and `x >= 0`, this function has a maximum
/// value of 1 that drops as `x` increases. Increasing the value of `a` and
/// `b` together results in a movement of the entire function to a flatter
/// part of the curve.
#[derive(Debug, Clone)]
pub struct ReciprocalFloatFunction {
    source: Arc<dyn ValueSource>,
    m: f32,
    a: f32,
    b: f32,
}

impl ReciprocalFloatFunction {
    /// Equivalent to `ReciprocalFloatFunction(ValueSource, float, float,
    /// float)`: `f(source) = a/(m*float(source)+b)`.
    pub fn new(source: Arc<dyn ValueSource>, m: f32, a: f32, b: f32) -> Self {
        Self { source, m, a, b }
    }
}

/// The anonymous `FloatDocValues` of `ReciprocalFloatFunction.getValues`.
struct ReciprocalFloatValues<'a> {
    this: Arc<ReciprocalFloatFunction>,
    vals: Box<dyn FunctionValues + 'a>,
}

impl FunctionValues for ReciprocalFloatValues<'_> {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        let this = &self.this;
        Ok(this.a / (this.m * self.vals.float_val(doc)? + this.b))
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        self.vals.exists(doc)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "{}/({}*float({})+{})",
            float_to_string(self.this.a),
            float_to_string(self.this.m),
            self.vals.to_doc_string(doc)?,
            float_to_string(self.this.b)
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

impl FloatDocValues for ReciprocalFloatValues<'_> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for ReciprocalFloatFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let vals = Arc::clone(&self.source).get_values(context, reader_context)?;
        Ok(Box::new(ReciprocalFloatValues { this: self, vals }))
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        self.source.create_weight(context, searcher)
    }

    fn description(&self) -> String {
        format!(
            "{}/({}*float({})+{})",
            float_to_string(self.a),
            float_to_string(self.m),
            self.source.description(),
            float_to_string(self.b)
        )
    }

    fn value_source_hash(&self) -> i32 {
        let mut h = float_to_int_bits(self.a).wrapping_add(float_to_int_bits(self.m));
        h ^= (h << 13) | ((h as u32) >> 20) as i32;
        h.wrapping_add(float_to_int_bits(self.b))
            .wrapping_add(self.source.value_source_hash())
    }

    /// `ReciprocalFloatFunction.class == o.getClass()` with equal `m`, `a`,
    /// `b` and source.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<ReciprocalFloatFunction>()
            .is_some_and(|other| {
                self.m == other.m
                    && self.a == other.a
                    && self.b == other.b
                    && self.source.value_source_eq(&*other.source)
            })
    }
}
