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

//! Port of `MultiValuedFloatFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, LeafReaderContext, NumericDocValues};
use rucene::search::sort::{MissingValue, SortField, SortFieldType};
use rucene::search::sorted_numeric_selector::{SortedNumericSelector, SortedNumericSelectorType};
use rucene::search::sorted_numeric_sort_field::SortedNumericSortField;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};
use crate::function::valuesource::float_field_source::FloatFieldSource;

/// Obtains float field values from `LeafReader.getSortedNumericDocValues` and
/// uses a `SortedNumericSelector` to obtain a value for each document.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.MultiValuedFloatFieldSource`, a
/// subclass of [`FloatFieldSource`] that reads its values through
/// `getNumericDocValues`, which it overrides.
#[derive(Debug, Clone)]
pub struct MultiValuedFloatFieldSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
    /// The protected `selector` field.
    selector: SortedNumericSelectorType,
    /// The private `missingValue` field, `None` for `null`.
    missing_value: Option<f32>,
}

impl MultiValuedFloatFieldSource {
    /// Equivalent to `MultiValuedFloatFieldSource(String, SortedNumericSelector.Type)`.
    pub fn new(field: impl Into<String>, selector: SortedNumericSelectorType) -> Self {
        Self::with_missing_value(field, selector, None)
    }

    /// Equivalent to `MultiValuedFloatFieldSource(String, SortedNumericSelector.Type, Float)`;
    /// Java's `requireNonNull` checks are the types themselves.
    pub fn with_missing_value(
        field: impl Into<String>,
        selector: SortedNumericSelectorType,
        missing_value: Option<f32>,
    ) -> Self {
        Self {
            field: field.into(),
            selector,
            missing_value,
        }
    }

    /// Equivalent to the protected
    /// `MultiValuedFloatFieldSource.getNumericDocValues(Map, LeafReaderContext)`: the sorted
    /// numeric doc values reduced by the selector.
    ///
    /// # Errors
    ///
    /// Propagates the errors of `DocValues.getSortedNumeric` and
    /// `SortedNumericSelector.wrap`.
    pub fn get_numeric_doc_values(
        &self,
        _context: &ValueSourceContext<'_>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn NumericDocValues>> {
        let sorted_dv =
            DocValues::get_sorted_numeric(reader_context.leaf_reader().as_ref(), &self.field)?;
        SortedNumericSelector::wrap(sorted_dv, self.selector, SortFieldType::Float)
    }
}

impl FieldCacheSource for MultiValuedFloatFieldSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for MultiValuedFloatFieldSource {
    /// The inherited `FloatFieldSource.getValues`, over the overridden
    /// `getNumericDocValues`.
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let arr = self.get_numeric_doc_values(context, reader_context)?;
        Ok(FloatFieldSource::values(self, arr))
    }

    /// `o.getClass() == MultiValuedFloatFieldSource.class`, the same selector and an equal field.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<MultiValuedFloatFieldSource>()
            .is_some_and(|other| self.selector == other.selector && self.field == other.field)
    }

    /// `super.hashCode() + selector.hashCode()`, where `super.hashCode()` is
    /// `Float.class.hashCode()` plus `field.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("java.lang.Float")
            .wrapping_add(field_cache_source::value_source_hash(self))
            .wrapping_add(class_hash(&format!(
                "org.apache.lucene.search.SortedNumericSelector$Type${}",
                self.selector
            )))
    }

    fn description(&self) -> String {
        format!("float({},{})", self.field, self.selector)
    }

    fn get_sort_field(self: Arc<Self>, reverse: bool) -> Result<SortField> {
        Ok(SortedNumericSortField::with_missing_value(
            &self.field,
            SortFieldType::Float,
            reverse,
            self.selector,
            self.missing_value.map(MissingValue::Float),
        )?
        .into_sort_field())
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
