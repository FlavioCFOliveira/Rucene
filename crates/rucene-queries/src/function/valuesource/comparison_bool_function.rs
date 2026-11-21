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

//! Port of `ComparisonBoolFunction.java`, of the Java package
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
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::bool_function::BoolFunction;

/// Base class for comparison operators useful within an "if"/conditional.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.ComparisonBoolFunction`,
/// whose `lhs`, `rhs` and `name` are private fields set by its constructor.
/// Its members are [`get_values`], [`value_source_eq`],
/// [`value_source_hash`], [`description`] and [`create_weight`].
pub trait ComparisonBoolFunction: BoolFunction {
    /// The private `lhs` field.
    fn lhs(&self) -> &Arc<dyn ValueSource>;

    /// The private `rhs` field.
    fn rhs(&self) -> &Arc<dyn ValueSource>;

    /// Equivalent to `ComparisonBoolFunction.name()`, the private `name`.
    fn name(&self) -> String;

    /// Perform the comparison, returning true or false.
    ///
    /// Equivalent to the abstract
    /// `ComparisonBoolFunction.compare(int, FunctionValues, FunctionValues)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    fn compare(
        &self,
        doc: i32,
        lhs: &mut dyn FunctionValues,
        rhs: &mut dyn FunctionValues,
    ) -> Result<bool>;
}

/// Equivalent to `ComparisonBoolFunction.getValues(Map, LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the error of either source's `get_values`.
pub fn get_values<'a, T: ComparisonBoolFunction>(
    this: Arc<T>,
    context: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn FunctionValues + 'a>> {
    let lhs_val = Arc::clone(this.lhs()).get_values(context, reader_context)?;
    let rhs_val = Arc::clone(this.rhs()).get_values(context, reader_context)?;
    let comp_label = this.name();
    Ok(Box::new(ComparisonBoolFunctionValues {
        this,
        lhs_val,
        rhs_val,
        comp_label,
    }))
}

/// The anonymous `BoolDocValues` of `ComparisonBoolFunction.getValues`.
struct ComparisonBoolFunctionValues<'a, T: ComparisonBoolFunction> {
    this: Arc<T>,
    lhs_val: Box<dyn FunctionValues + 'a>,
    rhs_val: Box<dyn FunctionValues + 'a>,
    comp_label: String,
}

impl<T: ComparisonBoolFunction> FunctionValues for ComparisonBoolFunctionValues<'_, T> {
    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        self.this
            .compare(doc, &mut *self.lhs_val, &mut *self.rhs_val)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "{}({},{})",
            self.comp_label,
            self.lhs_val.to_doc_string(doc)?,
            self.rhs_val.to_doc_string(doc)?
        ))
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        Ok(self.lhs_val.exists(doc)? && self.rhs_val.exists(doc)?)
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

impl<T: ComparisonBoolFunction> BoolDocValues for ComparisonBoolFunctionValues<'_, T> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

/// Equivalent to `ComparisonBoolFunction.equals(Object)`: the same class, an
/// equal name and equal operands.
pub fn value_source_eq<T: ComparisonBoolFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other.as_any().downcast_ref::<T>().is_some_and(|other| {
        this.name() == other.name()
            && this.lhs().value_source_eq(&**other.lhs())
            && this.rhs().value_source_eq(&**other.rhs())
    })
}

/// Equivalent to `ComparisonBoolFunction.hashCode()`, which starts from
/// `getClass().hashCode()`; the class of a subclass outside Lucene is named
/// after its Rust type.
pub fn value_source_hash<T: ComparisonBoolFunction>(this: &T) -> i32 {
    let mut h = class_hash(std::any::type_name::<T>());
    h = h
        .wrapping_mul(31)
        .wrapping_add(java_string_hash_code(&this.name()));
    h = h
        .wrapping_mul(31)
        .wrapping_add(this.lhs().value_source_hash());
    h = h
        .wrapping_mul(31)
        .wrapping_add(this.rhs().value_source_hash());
    h
}

/// Equivalent to `ComparisonBoolFunction.description()`:
/// `name() + "(" + lhs.description() + "," + rhs.description() + ")"`.
pub fn description<T: ComparisonBoolFunction + ?Sized>(this: &T) -> String {
    format!(
        "{}({},{})",
        this.name(),
        this.lhs().description(),
        this.rhs().description()
    )
}

/// Equivalent to `ComparisonBoolFunction.createWeight(Map, IndexSearcher)`.
///
/// # Errors
///
/// Propagates the error of either source's `create_weight`.
pub fn create_weight<T: ComparisonBoolFunction + ?Sized>(
    this: &T,
    context: &ValueSourceContext<'_>,
    searcher: &IndexSearcher,
) -> Result<()> {
    this.lhs().create_weight(context, searcher)?;
    this.rhs().create_weight(context, searcher)
}
