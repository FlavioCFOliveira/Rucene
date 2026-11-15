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

//! Port of `DivFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::dual_float_function::{self, DualFloatFunction};

/// Function to divide `a` by `b`.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.DivFloatFunction`, a
/// [`DualFloatFunction`].
#[derive(Debug, Clone)]
pub struct DivFloatFunction {
    a: Arc<dyn ValueSource>,
    b: Arc<dyn ValueSource>,
}

impl DivFloatFunction {
    /// Equivalent to `DivFloatFunction(ValueSource, ValueSource)`.
    pub fn new(a: Arc<dyn ValueSource>, b: Arc<dyn ValueSource>) -> Self {
        Self { a, b }
    }
}

impl DualFloatFunction for DivFloatFunction {
    fn a(&self) -> &Arc<dyn ValueSource> {
        &self.a
    }

    fn b(&self) -> &Arc<dyn ValueSource> {
        &self.b
    }

    fn name(&self) -> String {
        "div".to_string()
    }

    /// `aVals.floatVal(doc) / bVals.floatVal(doc)`.
    fn func(
        &self,
        doc: i32,
        a_vals: &mut dyn FunctionValues,
        b_vals: &mut dyn FunctionValues,
    ) -> Result<f32> {
        Ok(a_vals.float_val(doc)? / b_vals.float_val(doc)?)
    }
}

impl ValueSource for DivFloatFunction {
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
