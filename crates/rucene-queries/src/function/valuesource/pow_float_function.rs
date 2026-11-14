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

//! Port of `PowFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::dual_float_function::{self, DualFloatFunction};

/// Function to raise the base `a` to the power `b`.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.PowFloatFunction`, a
/// [`DualFloatFunction`].
#[derive(Debug, Clone)]
pub struct PowFloatFunction {
    a: Arc<dyn ValueSource>,
    b: Arc<dyn ValueSource>,
}

impl PowFloatFunction {
    /// Equivalent to `PowFloatFunction(ValueSource, ValueSource)`.
    pub fn new(a: Arc<dyn ValueSource>, b: Arc<dyn ValueSource>) -> Self {
        Self { a, b }
    }
}

impl DualFloatFunction for PowFloatFunction {
    fn a(&self) -> &Arc<dyn ValueSource> {
        &self.a
    }

    fn b(&self) -> &Arc<dyn ValueSource> {
        &self.b
    }

    fn name(&self) -> String {
        "pow".to_string()
    }

    /// `(float) Math.pow(aVals.floatVal(doc), bVals.floatVal(doc))`.
    fn func(
        &self,
        doc: i32,
        a_vals: &mut dyn FunctionValues,
        b_vals: &mut dyn FunctionValues,
    ) -> Result<f32> {
        // `Math.pow`, as `f64::powf`; see the core `similarities` module on
        // the one-ulp latitude both are allowed.
        let base = a_vals.float_val(doc)?;
        let exponent = b_vals.float_val(doc)?;
        Ok(f64::from(base).powf(f64::from(exponent)) as f32)
    }
}

impl ValueSource for PowFloatFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        dual_float_function::get_values(self, context, reader_context)
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        dual_float_function::value_source_eq(self, other)
    }

    fn value_source_hash(&self) -> i32 {
        dual_float_function::value_source_hash(self)
    }

    fn description(&self) -> String {
        dual_float_function::description(self)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        dual_float_function::create_weight(self, context, searcher)
    }
}
