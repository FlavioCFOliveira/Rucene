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

//! Port of `IfFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::mutable::MutableValueObject;
use rucene::util::BytesRefBuilder;

use crate::function::function_values::{FunctionValues, ValueFiller};
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::bool_function::BoolFunction;

/// Depending on the boolean value of the `if_source` function, returns the
/// value of the `true_source` or `false_source` function.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.IfFunction`.
#[derive(Debug, Clone)]
pub struct IfFunction {
    if_source: Arc<dyn ValueSource>,
    true_source: Arc<dyn ValueSource>,
    false_source: Arc<dyn ValueSource>,
}

impl IfFunction {
    /// Equivalent to `IfFunction(ValueSource, ValueSource, ValueSource)`.
    pub fn new(
        if_source: Arc<dyn ValueSource>,
        true_source: Arc<dyn ValueSource>,
        false_source: Arc<dyn ValueSource>,
    ) -> Self {
        Self {
            if_source,
            true_source,
            false_source,
        }
    }
}

impl BoolFunction for IfFunction {}

/// The anonymous `FunctionValues` of `IfFunction.getValues`.
struct IfValues<'a> {
    if_vals: Box<dyn FunctionValues + 'a>,
    true_vals: Box<dyn FunctionValues + 'a>,
    false_vals: Box<dyn FunctionValues + 'a>,
}

impl<'a> IfValues<'a> {
    /// `ifVals.boolVal(doc) ? trueVals : falseVals`.
    fn branch(&mut self, doc: i32) -> Result<&mut Box<dyn FunctionValues + 'a>> {
        Ok(if self.if_vals.bool_val(doc)? {
            &mut self.true_vals
        } else {
            &mut self.false_vals
        })
    }
}

impl FunctionValues for IfValues<'_> {
    fn byte_val(&mut self, doc: i32) -> Result<i8> {
        self.branch(doc)?.byte_val(doc)
    }

    fn short_val(&mut self, doc: i32) -> Result<i16> {
        self.branch(doc)?.short_val(doc)
    }

    fn float_val(&mut self, doc: i32) -> Result<f32> {
        self.branch(doc)?.float_val(doc)
    }

    fn int_val(&mut self, doc: i32) -> Result<i32> {
        self.branch(doc)?.int_val(doc)
    }

    fn long_val(&mut self, doc: i32) -> Result<i64> {
        self.branch(doc)?.long_val(doc)
    }

    fn double_val(&mut self, doc: i32) -> Result<f64> {
        self.branch(doc)?.double_val(doc)
    }

    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        self.branch(doc)?.str_val(doc)
    }

    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        self.branch(doc)?.bool_val(doc)
    }

    fn bytes_val(&mut self, doc: i32, target: &mut BytesRefBuilder) -> Result<bool> {
        self.branch(doc)?.bytes_val(doc, target)
    }

    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        self.branch(doc)?.object_val(doc)
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        self.branch(doc)?.exists(doc)
    }

    /// `super.getValueFiller()`, the default of `FunctionValues`.
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        Box::new(crate::function::function_values::FloatValuesFiller::new(
            false,
        ))
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "if({},{},{})",
            self.if_vals.to_doc_string(doc)?,
            self.true_vals.to_doc_string(doc)?,
            self.false_vals.to_doc_string(doc)?
        ))
    }
}

impl ValueSource for IfFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let if_vals = Arc::clone(&self.if_source).get_values(context, reader_context)?;
        let true_vals = Arc::clone(&self.true_source).get_values(context, reader_context)?;
        let false_vals = Arc::clone(&self.false_source).get_values(context, reader_context)?;
        Ok(Box::new(IfValues {
            if_vals,
            true_vals,
            false_vals,
        }))
    }

    /// `o instanceof IfFunction` with equal sources.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<IfFunction>()
            .is_some_and(|other| {
                self.if_source.value_source_eq(&*other.if_source)
                    && self.true_source.value_source_eq(&*other.true_source)
                    && self.false_source.value_source_eq(&*other.false_source)
            })
    }

    fn value_source_hash(&self) -> i32 {
        let mut h = self.if_source.value_source_hash();
        h = h
            .wrapping_mul(31)
            .wrapping_add(self.true_source.value_source_hash());
        h = h
            .wrapping_mul(31)
            .wrapping_add(self.false_source.value_source_hash());
        h
    }

    /// `"if(" + ifSource.description() + ',' + trueSource.description() + ','
    /// + falseSource + ')'`.
    fn description(&self) -> String {
        format!(
            "if({},{},{})",
            self.if_source.description(),
            self.true_source.description(),
            self.false_source
        )
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        self.if_source.create_weight(context, searcher)?;
        self.true_source.create_weight(context, searcher)?;
        self.false_source.create_weight(context, searcher)
    }
}
