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

//! Port of `SimpleFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::single_function::SingleFunction;

/// A simple float function with a single argument.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.SimpleFloatFunction`, a
/// [`SingleFunction`] whose values are [`func`](Self::func) of the source's
/// values; an implementor's `get_values` delegates to [`get_values`].
pub trait SimpleFloatFunction: SingleFunction {
    /// Equivalent to the abstract protected
    /// `SimpleFloatFunction.func(int, FunctionValues)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    fn func(&self, doc: i32, vals: &mut dyn FunctionValues) -> Result<f32>;
}

/// Equivalent to `SimpleFloatFunction.getValues(Map, LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the error of the source's `get_values`.
pub fn get_values<'a, T: SimpleFloatFunction>(
    this: Arc<T>,
    context: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn FunctionValues + 'a>> {
    let vals = Arc::clone(this.source()).get_values(context, reader_context)?;
    Ok(Box::new(SimpleFloatFunctionValues { this, vals }))
}

/// The anonymous `FloatDocValues` of `SimpleFloatFunction.getValues`.
struct SimpleFloatFunctionValues<'a, T: SimpleFloatFunction> {
    this: Arc<T>,
    vals: Box<dyn FunctionValues + 'a>,
}

impl<T: SimpleFloatFunction> FunctionValues for SimpleFloatFunctionValues<'_, T> {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        self.this.func(doc, &mut *self.vals)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "{}({})",
            self.this.name(),
            self.vals.to_doc_string(doc)?
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

impl<T: SimpleFloatFunction> FloatDocValues for SimpleFloatFunctionValues<'_, T> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}
