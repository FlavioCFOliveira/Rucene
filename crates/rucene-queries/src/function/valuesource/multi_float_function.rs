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

//! Port of `MultiFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_collections::java_string_hash_code;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{sources_eq, sources_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::multi_function;

/// Abstract [`ValueSource`] implementation which wraps multiple value sources
/// and applies an extendible float function to their values.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.MultiFloatFunction`. Its
/// members are [`description`], [`get_values`], [`create_weight`],
/// [`value_source_hash`] and [`value_source_eq`].
pub trait MultiFloatFunction: ValueSource {
    /// The protected `sources` field.
    fn sources(&self) -> &[Arc<dyn ValueSource>];

    /// Equivalent to the abstract protected `MultiFloatFunction.name()`.
    fn name(&self) -> String;

    /// Equivalent to the abstract protected
    /// `MultiFloatFunction.func(int, FunctionValues[])`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    fn func(&self, doc: i32, vals_arr: &mut [Box<dyn FunctionValues + '_>]) -> Result<f32>;

    /// Called by the values' `exists(doc)` method to determine if a document
    /// exists for this function.
    ///
    /// Equivalent to the protected
    /// `MultiFloatFunction.exists(int, FunctionValues[])`: by default, a
    /// document exists only when every sub-value exists for it.
    ///
    /// # Errors
    ///
    /// Propagates the error of any `exists`.
    fn exists(&self, doc: i32, vals_arr: &mut [Box<dyn FunctionValues + '_>]) -> Result<bool> {
        multi_function::all_exists(doc, vals_arr)
    }
}

/// Equivalent to `MultiFloatFunction.description()`: `name(s1,s2,...)`.
pub fn description<T: MultiFloatFunction + ?Sized>(this: &T) -> String {
    let mut sb = String::new();
    sb.push_str(&this.name());
    sb.push('(');
    let mut first_time = true;
    for source in this.sources() {
        if first_time {
            first_time = false;
        } else {
            sb.push(',');
        }
        sb.push_str(&source.to_string());
    }
    sb.push(')');
    sb
}

/// Equivalent to `MultiFloatFunction.getValues(Map, LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the error of any source's `get_values`.
pub fn get_values<'a, T: MultiFloatFunction>(
    this: Arc<T>,
    context: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn FunctionValues + 'a>> {
    let mut vals_arr = Vec::with_capacity(this.sources().len());
    for source in this.sources() {
        vals_arr.push(Arc::clone(source).get_values(context, reader_context)?);
    }
    Ok(Box::new(MultiFloatFunctionValues { this, vals_arr }))
}

/// The anonymous `FloatDocValues` of `MultiFloatFunction.getValues`.
struct MultiFloatFunctionValues<'a, T: MultiFloatFunction> {
    this: Arc<T>,
    vals_arr: Vec<Box<dyn FunctionValues + 'a>>,
}

impl<T: MultiFloatFunction> FunctionValues for MultiFloatFunctionValues<'_, T> {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        self.this.func(doc, &mut self.vals_arr)
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        MultiFloatFunction::exists(&*self.this, doc, &mut self.vals_arr)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        multi_function::to_string(&self.this.name(), &mut self.vals_arr, doc)
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

impl<T: MultiFloatFunction> FloatDocValues for MultiFloatFunctionValues<'_, T> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

/// Equivalent to `MultiFloatFunction.createWeight(Map, IndexSearcher)`.
///
/// # Errors
///
/// Propagates the error of any source's `create_weight`.
pub fn create_weight<T: MultiFloatFunction + ?Sized>(
    this: &T,
    context: &ValueSourceContext<'_>,
    searcher: &IndexSearcher,
) -> Result<()> {
    for source in this.sources() {
        source.create_weight(context, searcher)?;
    }
    Ok(())
}

/// Equivalent to `MultiFloatFunction.hashCode()`:
/// `Arrays.hashCode(sources) + name().hashCode()`.
pub fn value_source_hash<T: MultiFloatFunction + ?Sized>(this: &T) -> i32 {
    sources_hash(this.sources()).wrapping_add(java_string_hash_code(&this.name()))
}

/// Equivalent to `MultiFloatFunction.equals(Object)`: the same class, an
/// equal name and equal sources.
pub fn value_source_eq<T: MultiFloatFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other.as_any().downcast_ref::<T>().is_some_and(|other| {
        this.name() == other.name() && sources_eq(this.sources(), other.sources())
    })
}
