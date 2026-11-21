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

//! Port of `ProductFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::multi_float_function::{self, MultiFloatFunction};

/// Returns the product of its components.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.ProductFloatFunction`, a
/// [`MultiFloatFunction`].
#[derive(Debug, Clone)]
pub struct ProductFloatFunction {
    sources: Vec<Arc<dyn ValueSource>>,
}

impl ProductFloatFunction {
    /// Equivalent to `ProductFloatFunction(ValueSource[])`.
    pub fn new(sources: Vec<Arc<dyn ValueSource>>) -> Self {
        Self { sources }
    }
}

impl MultiFloatFunction for ProductFloatFunction {
    fn sources(&self) -> &[Arc<dyn ValueSource>] {
        &self.sources
    }

    fn name(&self) -> String {
        "product".to_string()
    }

    fn func(&self, doc: i32, vals_arr: &mut [Box<dyn FunctionValues + '_>]) -> Result<f32> {
        let mut val = 1.0f32;
        for vals in vals_arr.iter_mut() {
            val *= vals.float_val(doc)?;
        }
        Ok(val)
    }
}

impl ValueSource for ProductFloatFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        multi_float_function::get_values(self, context, reader_context)
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        multi_float_function::value_source_eq(self, other)
    }

    fn value_source_hash(&self) -> i32 {
        multi_float_function::value_source_hash(self)
    }

    fn description(&self) -> String {
        multi_float_function::description(self)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        multi_float_function::create_weight(self, context, searcher)
    }
}
