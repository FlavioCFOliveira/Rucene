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

//! Port of `DualFloatFunction.java`, of the Java package
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
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::multi_function;

/// Abstract [`ValueSource`] implementation which wraps two value sources and
/// applies an extendible float function to their values.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.DualFloatFunction`. Its
/// members are [`description`], [`get_values`], [`create_weight`],
/// [`value_source_hash`] and [`value_source_eq`].
pub trait DualFloatFunction: ValueSource {
    /// The protected `a` field.
    fn a(&self) -> &Arc<dyn ValueSource>;

    /// The protected `b` field.
    fn b(&self) -> &Arc<dyn ValueSource>;

    /// Equivalent to the abstract protected `DualFloatFunction.name()`.
    fn name(&self) -> String;

    /// Equivalent to the abstract protected
    /// `DualFloatFunction.func(int, FunctionValues, FunctionValues)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    fn func(
        &self,
        doc: i32,
        a_vals: &mut dyn FunctionValues,
        b_vals: &mut dyn FunctionValues,
    ) -> Result<f32>;
}

/// Equivalent to `DualFloatFunction.description()`:
/// `name() + "(" + a.description() + "," + b.description() + ")"`.
pub fn description<T: DualFloatFunction + ?Sized>(this: &T) -> String {
    format!(
        "{}({},{})",
        this.name(),
        this.a().description(),
        this.b().description()
    )
}

/// Equivalent to `DualFloatFunction.getValues(Map, LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the error of either source's `get_values`.
pub fn get_values<'a, T: DualFloatFunction>(
    this: Arc<T>,
    context: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn FunctionValues + 'a>> {
    let a_vals = Arc::clone(this.a()).get_values(context, reader_context)?;
    let b_vals = Arc::clone(this.b()).get_values(context, reader_context)?;
    Ok(Box::new(DualFloatFunctionValues {
        this,
        a_vals,
        b_vals,
    }))
}

/// The anonymous `FloatDocValues` of `DualFloatFunction.getValues`.
struct DualFloatFunctionValues<'a, T: DualFloatFunction> {
    this: Arc<T>,
    a_vals: Box<dyn FunctionValues + 'a>,
    b_vals: Box<dyn FunctionValues + 'a>,
}

impl<T: DualFloatFunction> FunctionValues for DualFloatFunctionValues<'_, T> {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        self.this.func(doc, &mut *self.a_vals, &mut *self.b_vals)
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        multi_function::all_exists2(doc, &mut *self.a_vals, &mut *self.b_vals)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "{}({},{})",
            self.this.name(),
            self.a_vals.to_doc_string(doc)?,
            self.b_vals.to_doc_string(doc)?
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

impl<T: DualFloatFunction> FloatDocValues for DualFloatFunctionValues<'_, T> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

/// Equivalent to `DualFloatFunction.createWeight(Map, IndexSearcher)`.
///
/// # Errors
///
/// Propagates the error of either source's `create_weight`.
pub fn create_weight<T: DualFloatFunction + ?Sized>(
    this: &T,
    context: &ValueSourceContext<'_>,
    searcher: &IndexSearcher,
) -> Result<()> {
    this.a().create_weight(context, searcher)?;
    this.b().create_weight(context, searcher)
}

/// Equivalent to `DualFloatFunction.hashCode()`.
pub fn value_source_hash<T: DualFloatFunction + ?Sized>(this: &T) -> i32 {
    let mut h = this.a().value_source_hash();
    h ^= (h << 13) | ((h as u32) >> 20) as i32;
    h = h.wrapping_add(this.b().value_source_hash());
    h ^= (h << 23) | ((h as u32) >> 10) as i32;
    h = h.wrapping_add(java_string_hash_code(&this.name()));
    h
}

/// Equivalent to `DualFloatFunction.equals(Object)`: the same class and equal
/// `a` and `b`.
pub fn value_source_eq<T: DualFloatFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other.as_any().downcast_ref::<T>().is_some_and(|other| {
        this.a().value_source_eq(&**other.a()) && this.b().value_source_eq(&**other.b())
    })
}
