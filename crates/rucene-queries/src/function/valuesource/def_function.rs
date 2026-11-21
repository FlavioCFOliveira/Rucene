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

//! Port of `DefFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::mutable::MutableValueObject;
use rucene::util::BytesRefBuilder;

use crate::function::function_values::{FunctionValues, ValueFiller};
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::multi_function::{self, MultiFunction, Values};

/// [`ValueSource`] implementation which only returns the values from the
/// provided value sources which are available for a particular document; it
/// returns the values of the first source that has a value, and the last
/// source's otherwise.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.DefFunction`.
#[derive(Debug, Clone)]
pub struct DefFunction {
    sources: Vec<Arc<dyn ValueSource>>,
}

impl DefFunction {
    /// Equivalent to `DefFunction(List<ValueSource>)`.
    pub fn new(sources: Vec<Arc<dyn ValueSource>>) -> Self {
        Self { sources }
    }
}

impl MultiFunction for DefFunction {
    fn sources(&self) -> &[Arc<dyn ValueSource>] {
        &self.sources
    }

    fn name(&self) -> String {
        "def".to_string()
    }
}

/// The anonymous `MultiFunction.Values` of `DefFunction.getValues`.
struct DefValues<'a> {
    values: Values<'a>,
    /// `valsArr.length - 1`.
    upto: i32,
}

impl DefValues<'_> {
    /// The position of the first values that exist for `doc`, or of the last.
    ///
    /// Equivalent to the private `get(int)` of the anonymous class.
    fn get(&mut self, doc: i32) -> Result<usize> {
        for i in 0..self.upto.max(0) as usize {
            if self.values.vals_arr[i].exists(doc)? {
                return Ok(i);
            }
        }
        if self.upto < 0 {
            // `valsArr[upto]` with no sources.
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {} out of bounds for length 0", self.upto),
            ));
        }
        Ok(self.upto as usize)
    }
}

impl FunctionValues for DefValues<'_> {
    fn byte_val(&mut self, doc: i32) -> Result<i8> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].byte_val(doc)
    }

    fn short_val(&mut self, doc: i32) -> Result<i16> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].short_val(doc)
    }

    fn float_val(&mut self, doc: i32) -> Result<f32> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].float_val(doc)
    }

    fn int_val(&mut self, doc: i32) -> Result<i32> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].int_val(doc)
    }

    fn long_val(&mut self, doc: i32) -> Result<i64> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].long_val(doc)
    }

    fn double_val(&mut self, doc: i32) -> Result<f64> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].double_val(doc)
    }

    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].str_val(doc)
    }

    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].bool_val(doc)
    }

    fn bytes_val(&mut self, doc: i32, target: &mut BytesRefBuilder) -> Result<bool> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].bytes_val(doc, target)
    }

    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        let i = self.get(doc)?;
        self.values.vals_arr[i].object_val(doc)
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        for vals in &mut self.values.vals_arr {
            if vals.exists(doc)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `super.getValueFiller()`: `MultiFunction.Values` inherits
    /// `FunctionValues.getValueFiller()`.
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        self.values.get_value_filler()
    }

    /// Inherited from `MultiFunction.Values`.
    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        self.values.to_doc_string(doc)
    }
}

impl ValueSource for DefFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        fcontext: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let vals_arr = multi_function::vals_arr(&self.sources, fcontext, reader_context)?;
        let upto = vals_arr.len() as i32 - 1;
        Ok(Box::new(DefValues {
            values: Values::new(self.name(), vals_arr),
            upto,
        }))
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        multi_function::value_source_eq(self, other)
    }

    fn value_source_hash(&self) -> i32 {
        multi_function::value_source_hash(self)
    }

    fn description(&self) -> String {
        multi_function::description(self)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        multi_function::create_weight(self, context, searcher)
    }
}
