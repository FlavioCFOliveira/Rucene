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

//! Port of `MultiFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.
//!
//! The static members of the class are the free functions of this module;
//! its concrete instance members are the functions taking `this`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_collections::java_string_hash_code;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{sources_eq, sources_hash, ValueSource, ValueSourceContext};

/// Abstract parent class for [`ValueSource`] implementations that wrap
/// multiple value sources and apply their own logic.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.MultiFunction`. Its
/// instance members are [`description`], [`create_weight`],
/// [`value_source_hash`] and [`value_source_eq`], which an implementor's
/// [`ValueSource`] implementation delegates to.
pub trait MultiFunction: ValueSource {
    /// The protected `sources` field.
    fn sources(&self) -> &[Arc<dyn ValueSource>];

    /// Equivalent to the abstract protected `MultiFunction.name()`.
    fn name(&self) -> String;
}

/// Equivalent to `MultiFunction.description()`: `description(name(), sources)`.
pub fn description<T: MultiFunction + ?Sized>(this: &T) -> String {
    description_of(&this.name(), this.sources())
}

/// Equivalent to the static `MultiFunction.allExists(int, FunctionValues[])`.
///
/// # Errors
///
/// Propagates the error of any `exists`.
pub fn all_exists(doc: i32, values: &mut [Box<dyn FunctionValues + '_>]) -> Result<bool> {
    for v in values {
        if !v.exists(doc)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Equivalent to the static `MultiFunction.anyExists(int, FunctionValues[])`.
///
/// # Errors
///
/// Propagates the error of any `exists`.
pub fn any_exists(doc: i32, values: &mut [Box<dyn FunctionValues + '_>]) -> Result<bool> {
    for v in values {
        if v.exists(doc)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Equivalent to the static two-argument
/// `MultiFunction.allExists(int, FunctionValues, FunctionValues)`.
///
/// # Errors
///
/// Propagates the error of either `exists`.
pub fn all_exists2(
    doc: i32,
    values1: &mut dyn FunctionValues,
    values2: &mut dyn FunctionValues,
) -> Result<bool> {
    Ok(values1.exists(doc)? && values2.exists(doc)?)
}

/// Equivalent to the static two-argument
/// `MultiFunction.anyExists(int, FunctionValues, FunctionValues)`.
///
/// # Errors
///
/// Propagates the error of either `exists`.
pub fn any_exists2(
    doc: i32,
    values1: &mut dyn FunctionValues,
    values2: &mut dyn FunctionValues,
) -> Result<bool> {
    Ok(values1.exists(doc)? || values2.exists(doc)?)
}

/// Equivalent to the static `MultiFunction.description(String,
/// List<ValueSource>)`: `name(s1,s2,...)`, each source rendered with
/// `toString()`.
pub fn description_of(name: &str, sources: &[Arc<dyn ValueSource>]) -> String {
    let mut sb = String::new();
    sb.push_str(name);
    sb.push('(');
    let mut first_time = true;
    for source in sources {
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

/// Equivalent to the static `MultiFunction.valsArr(List<ValueSource>, Map,
/// LeafReaderContext)`: the values of every source, in order.
///
/// # Errors
///
/// Propagates the error of any source's `get_values`.
pub fn vals_arr<'a>(
    sources: &[Arc<dyn ValueSource>],
    fcontext: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Vec<Box<dyn FunctionValues + 'a>>> {
    let mut vals_arr = Vec::with_capacity(sources.len());
    for source in sources {
        vals_arr.push(Arc::clone(source).get_values(fcontext, reader_context)?);
    }
    Ok(vals_arr)
}

/// Equivalent to the static `MultiFunction.toString(String,
/// FunctionValues[], int)`: `name(v1,v2,...)`, each value rendered with
/// `toString(doc)`.
///
/// # Errors
///
/// Propagates the error of any `to_doc_string`.
pub fn to_string(
    name: &str,
    vals_arr: &mut [Box<dyn FunctionValues + '_>],
    doc: i32,
) -> Result<String> {
    let mut sb = String::new();
    sb.push_str(name);
    sb.push('(');
    let mut first_time = true;
    for vals in vals_arr {
        if first_time {
            first_time = false;
        } else {
            sb.push(',');
        }
        sb.push_str(&vals.to_doc_string(doc)?);
    }
    sb.push(')');
    Ok(sb)
}

/// Equivalent to `MultiFunction.createWeight(Map, IndexSearcher)`, which
/// propagates to every source.
///
/// # Errors
///
/// Propagates the error of any source's `create_weight`.
pub fn create_weight<T: MultiFunction + ?Sized>(
    this: &T,
    context: &ValueSourceContext<'_>,
    searcher: &IndexSearcher,
) -> Result<()> {
    for source in this.sources() {
        source.create_weight(context, searcher)?;
    }
    Ok(())
}

/// Equivalent to `MultiFunction.hashCode()`:
/// `sources.hashCode() + name().hashCode()`.
pub fn value_source_hash<T: MultiFunction + ?Sized>(this: &T) -> i32 {
    sources_hash(this.sources()).wrapping_add(java_string_hash_code(&this.name()))
}

/// Equivalent to `MultiFunction.equals(Object)`: the same class and equal
/// sources.
pub fn value_source_eq<T: MultiFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other
        .as_any()
        .downcast_ref::<T>()
        .is_some_and(|other| sources_eq(this.sources(), other.sources()))
}

/// The values of the sources of a [`MultiFunction`].
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.function.valuesource.MultiFunction.Values`;
/// its enclosing instance's `name()` is captured at construction.
pub struct Values<'a> {
    /// `MultiFunction.this.name()`.
    name: String,
    /// The package-private `valsArr` field.
    pub(crate) vals_arr: Vec<Box<dyn FunctionValues + 'a>>,
}

impl std::fmt::Debug for Values<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Values")
            .field("name", &self.name)
            .field("vals", &self.vals_arr.len())
            .finish()
    }
}

impl<'a> Values<'a> {
    /// Equivalent to `Values(FunctionValues[])`, built by the function whose
    /// `name()` is `name`.
    pub fn new(name: String, vals_arr: Vec<Box<dyn FunctionValues + 'a>>) -> Self {
        Self { name, vals_arr }
    }
}

impl FunctionValues for Values<'_> {
    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        to_string(&self.name, &mut self.vals_arr, doc)
    }
}
