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

//! Port of `SimpleBoolFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_collections::java_string_hash_code;

use crate::function::docvalues::bool_doc_values::{bool_doc_values_members, BoolDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::bool_function::BoolFunction;

/// [`BoolFunction`] implementation which applies an extendible boolean
/// function to the values of a single wrapped value source.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.SimpleBoolFunction`.
/// Functions this can be used for include whether a field has a value or not,
/// or inverting the boolean value of the wrapped value source. Its members are
/// [`get_values`], [`description`], [`value_source_hash`],
/// [`value_source_eq`] and [`create_weight`].
pub trait SimpleBoolFunction: BoolFunction {
    /// The protected `source` field.
    fn source(&self) -> &Arc<dyn ValueSource>;

    /// Equivalent to the abstract protected `SimpleBoolFunction.name()`.
    fn name(&self) -> String;

    /// Equivalent to the abstract protected
    /// `SimpleBoolFunction.func(int, FunctionValues)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    fn func(&self, doc: i32, vals: &mut dyn FunctionValues) -> Result<bool>;
}

/// Equivalent to `SimpleBoolFunction.getValues(Map, LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the error of the source's `get_values`.
pub fn get_values<'a, T: SimpleBoolFunction>(
    this: Arc<T>,
    context: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn FunctionValues + 'a>> {
    let vals = Arc::clone(this.source()).get_values(context, reader_context)?;
    Ok(Box::new(SimpleBoolFunctionValues { this, vals }))
}

/// The anonymous `BoolDocValues` of `SimpleBoolFunction.getValues`.
struct SimpleBoolFunctionValues<'a, T: SimpleBoolFunction> {
    this: Arc<T>,
    vals: Box<dyn FunctionValues + 'a>,
}

impl<T: SimpleBoolFunction> FunctionValues for SimpleBoolFunctionValues<'_, T> {
    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        self.this.func(doc, &mut *self.vals)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "{}({})",
            self.this.name(),
            self.vals.to_doc_string(doc)?
        ))
    }

    bool_doc_values_members!(
        byte_val,
        short_val,
        float_val,
        int_val,
        long_val,
        double_val,
        str_val,
        object_val,
        get_value_filler
    );
}

impl<T: SimpleBoolFunction> BoolDocValues for SimpleBoolFunctionValues<'_, T> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

/// Equivalent to `SimpleBoolFunction.description()`:
/// `name() + '(' + source.description() + ')'`.
pub fn description<T: SimpleBoolFunction + ?Sized>(this: &T) -> String {
    format!("{}({})", this.name(), this.source().description())
}

/// Equivalent to `SimpleBoolFunction.hashCode()`:
/// `source.hashCode() + name().hashCode()`.
pub fn value_source_hash<T: SimpleBoolFunction + ?Sized>(this: &T) -> i32 {
    this.source()
        .value_source_hash()
        .wrapping_add(java_string_hash_code(&this.name()))
}

/// Equivalent to `SimpleBoolFunction.equals(Object)`: the same class and an
/// equal source.
pub fn value_source_eq<T: SimpleBoolFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other
        .as_any()
        .downcast_ref::<T>()
        .is_some_and(|other| this.source().value_source_eq(&**other.source()))
}

/// Equivalent to `SimpleBoolFunction.createWeight(Map, IndexSearcher)`.
///
/// # Errors
///
/// Propagates the error of the source's `create_weight`.
pub fn create_weight<T: SimpleBoolFunction + ?Sized>(
    this: &T,
    context: &ValueSourceContext<'_>,
    searcher: &IndexSearcher,
) -> Result<()> {
    this.source().create_weight(context, searcher)
}
