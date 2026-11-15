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

//! Port of `MaxFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;

use crate::function::function_values::{java_max_f32, FunctionValues};
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::multi_float_function::{self, MultiFloatFunction};
use crate::function::valuesource::multi_function;

/// Returns the max of its components.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.MaxFloatFunction`, a
/// [`MultiFloatFunction`].
#[derive(Debug, Clone)]
pub struct MaxFloatFunction {
    sources: Vec<Arc<dyn ValueSource>>,
}

impl MaxFloatFunction {
    /// Equivalent to `MaxFloatFunction(ValueSource[])`.
    pub fn new(sources: Vec<Arc<dyn ValueSource>>) -> Self {
        Self { sources }
    }
}

impl MultiFloatFunction for MaxFloatFunction {
    fn sources(&self) -> &[Arc<dyn ValueSource>] {
        &self.sources
    }

    fn name(&self) -> String {
        "max".to_string()
    }

    fn func(&self, doc: i32, vals_arr: &mut [Box<dyn FunctionValues + '_>]) -> Result<f32> {
        let mut none_found = true;
        let mut val = f32::NEG_INFINITY;
        for vals in vals_arr.iter_mut() {
            if vals.exists(doc)? {
                none_found = false;
                val = java_max_f32(vals.float_val(doc)?, val);
            }
        }
        if none_found {
            return Ok(0.0);
        }
        Ok(val)
    }

    /// Values exist when any source has a value: `MultiFunction.anyExists`.
    fn exists(&self, doc: i32, vals_arr: &mut [Box<dyn FunctionValues + '_>]) -> Result<bool> {
        multi_function::any_exists(doc, vals_arr)
    }
}

impl ValueSource for MaxFloatFunction {
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
