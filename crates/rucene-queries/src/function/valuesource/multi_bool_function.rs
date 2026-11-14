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

//! Port of `MultiBoolFunction.java`, of the Java package
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
use crate::function::value_source::{sources_eq, sources_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::bool_function::BoolFunction;

/// Abstract [`ValueSource`] implementation which wraps multiple value sources
/// and applies an extendible boolean function to their values.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.MultiBoolFunction`. Its
/// members are [`get_values`], [`description`], [`value_source_hash`],
/// [`value_source_eq`] and [`create_weight`].
pub trait MultiBoolFunction: BoolFunction {
    /// The protected `sources` field.
    fn sources(&self) -> &[Arc<dyn ValueSource>];

    /// Equivalent to the abstract protected `MultiBoolFunction.name()`.
    fn name(&self) -> String;

    /// Equivalent to the abstract protected
    /// `MultiBoolFunction.func(int, FunctionValues[])`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    fn func(&self, doc: i32, vals: &mut [Box<dyn FunctionValues + '_>]) -> Result<bool>;
}

/// Equivalent to `MultiBoolFunction.getValues(Map, LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the error of any source's `get_values`.
pub fn get_values<'a, T: MultiBoolFunction>(
    this: Arc<T>,
    context: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn FunctionValues + 'a>> {
    let mut vals = Vec::with_capacity(this.sources().len());
    for source in this.sources() {
        vals.push(Arc::clone(source).get_values(context, reader_context)?);
    }
    Ok(Box::new(MultiBoolFunctionValues { this, vals }))
}

/// The anonymous `BoolDocValues` of `MultiBoolFunction.getValues`.
struct MultiBoolFunctionValues<'a, T: MultiBoolFunction> {
    this: Arc<T>,
    vals: Vec<Box<dyn FunctionValues + 'a>>,
}

impl<T: MultiBoolFunction> FunctionValues for MultiBoolFunctionValues<'_, T> {
    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        self.this.func(doc, &mut self.vals)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let mut sb = self.this.name();
        sb.push('(');
        let mut first = true;
        for dv in &mut self.vals {
            if first {
                first = false;
            } else {
                sb.push(',');
            }
            sb.push_str(&dv.to_doc_string(doc)?);
        }
        sb.push(')');
        Ok(sb)
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

impl<T: MultiBoolFunction> BoolDocValues for MultiBoolFunctionValues<'_, T> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

/// Equivalent to `MultiBoolFunction.description()`: `name(d1,d2,...)`, each
/// source rendered with `description()`.
pub fn description<T: MultiBoolFunction + ?Sized>(this: &T) -> String {
    let mut sb = this.name();
    sb.push('(');
    let mut first = true;
    for source in this.sources() {
        if first {
            first = false;
        } else {
            sb.push(',');
        }
        sb.push_str(&source.description());
    }
    sb.push(')');
    sb
}

/// Equivalent to `MultiBoolFunction.hashCode()`:
/// `sources.hashCode() + name().hashCode()`.
pub fn value_source_hash<T: MultiBoolFunction + ?Sized>(this: &T) -> i32 {
    sources_hash(this.sources()).wrapping_add(java_string_hash_code(&this.name()))
}

/// Equivalent to `MultiBoolFunction.equals(Object)`: the same class and equal
/// sources.
pub fn value_source_eq<T: MultiBoolFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other
        .as_any()
        .downcast_ref::<T>()
        .is_some_and(|other| sources_eq(this.sources(), other.sources()))
}

/// Equivalent to `MultiBoolFunction.createWeight(Map, IndexSearcher)`.
///
/// # Errors
///
/// Propagates the error of any source's `create_weight`.
pub fn create_weight<T: MultiBoolFunction + ?Sized>(
    this: &T,
    context: &ValueSourceContext<'_>,
    searcher: &IndexSearcher,
) -> Result<()> {
    for source in this.sources() {
        source.create_weight(context, searcher)?;
    }
    Ok(())
}
