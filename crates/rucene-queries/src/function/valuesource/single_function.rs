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

//! Port of `SingleFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_collections::java_string_hash_code;

use crate::function::value_source::{ValueSource, ValueSourceContext};

/// A function with a single (one) argument.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.SingleFunction`. Note:
/// subclasses must override `description()` if the name is not the default
/// `name(source)`. Its members are [`description`], [`value_source_hash`],
/// [`value_source_eq`] and [`create_weight`], which an implementor's
/// [`ValueSource`] implementation delegates to.
pub trait SingleFunction: ValueSource {
    /// The protected `source` field.
    fn source(&self) -> &Arc<dyn ValueSource>;

    /// Equivalent to the abstract protected `SingleFunction.name()`.
    fn name(&self) -> String;
}

/// Equivalent to `SingleFunction.description()`:
/// `name() + '(' + source.description() + ')'`.
pub fn description<T: SingleFunction + ?Sized>(this: &T) -> String {
    format!("{}({})", this.name(), this.source().description())
}

/// Equivalent to `SingleFunction.hashCode()`:
/// `source.hashCode() + name().hashCode()`.
pub fn value_source_hash<T: SingleFunction + ?Sized>(this: &T) -> i32 {
    this.source()
        .value_source_hash()
        .wrapping_add(java_string_hash_code(&this.name()))
}

/// Equivalent to `SingleFunction.equals(Object)`: the same class, an equal
/// name and an equal source.
pub fn value_source_eq<T: SingleFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other.as_any().downcast_ref::<T>().is_some_and(|other| {
        this.name() == other.name() && this.source().value_source_eq(&**other.source())
    })
}

/// Equivalent to `SingleFunction.createWeight(Map, IndexSearcher)`, which
/// propagates to the source.
///
/// # Errors
///
/// Propagates the error of the source's `create_weight`.
pub fn create_weight<T: SingleFunction + ?Sized>(
    this: &T,
    context: &ValueSourceContext<'_>,
    searcher: &IndexSearcher,
) -> Result<()> {
    this.source().create_weight(context, searcher)
}
