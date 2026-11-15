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

//! Port of `SortedSetFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, LeafReaderContext};
use rucene::search::sort::SortField;
use rucene::search::sorted_set_selector::{SortedSetSelector, SortedSetSelectorType};
use rucene::search::sorted_set_sort_field::SortedSetSortField;
use rucene::util::mutable::MutableValueObject;

use crate::function::docvalues::doc_terms_index_doc_values::{
    doc_terms_index_doc_values_members, DocTermsIndexDocValues, DocTermsIndexState,
};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};

/// Retrieves [`FunctionValues`] instances for multi-valued string based
/// fields, reducing each document's values to one with a
/// `SortedSetSelector`.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.SortedSetFieldSource`.
#[derive(Debug, Clone)]
pub struct SortedSetFieldSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
    /// The protected `selector` field.
    selector: SortedSetSelectorType,
}

impl SortedSetFieldSource {
    /// Equivalent to `SortedSetFieldSource(String)`, with the `MIN` selector.
    pub fn new(field: impl Into<String>) -> Self {
        Self::with_selector(field, SortedSetSelectorType::MIN)
    }

    /// Equivalent to `SortedSetFieldSource(String, SortedSetSelector.Type)`.
    pub fn with_selector(field: impl Into<String>, selector: SortedSetSelectorType) -> Self {
        Self {
            field: field.into(),
            selector,
        }
    }
}

/// The anonymous `DocTermsIndexDocValues` of `SortedSetFieldSource.getValues`.
struct SortedSetFieldValues {
    this: Arc<SortedSetFieldSource>,
    state: DocTermsIndexState,
}

impl FunctionValues for SortedSetFieldValues {
    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        Ok(self.str_val(doc)?.map(MutableValueObject::Str))
    }

    doc_terms_index_doc_values_members!(
        exists,
        ord_val,
        num_ord,
        bytes_val,
        str_val,
        bool_val,
        get_range_matcher,
        to_doc_string,
        get_value_filler
    );
}

impl DocTermsIndexDocValues for SortedSetFieldValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }

    fn state(&mut self) -> &mut DocTermsIndexState {
        &mut self.state
    }

    fn state_ref(&self) -> &DocTermsIndexState {
        &self.state
    }

    fn to_term(&self, readable_value: &str) -> String {
        readable_value.to_string()
    }
}

impl FieldCacheSource for SortedSetFieldSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for SortedSetFieldSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let sorted_set =
            DocValues::get_sorted_set(reader_context.leaf_reader().as_ref(), &self.field)?;
        let view = SortedSetSelector::wrap(sorted_set, self.selector)?;
        Ok(Box::new(SortedSetFieldValues {
            this: self,
            state: DocTermsIndexState::new(view),
        }))
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        if !field_cache_source::value_source_eq(self, other) {
            return false;
        }
        other
            .as_any()
            .downcast_ref::<SortedSetFieldSource>()
            .is_some_and(|other| self.selector == other.selector)
    }

    /// `31 * super.hashCode() + selector.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        let prime = 31i32;
        let mut result = field_cache_source::value_source_hash(self);
        result = prime.wrapping_mul(result).wrapping_add(class_hash(&format!(
            "org.apache.lucene.search.SortedSetSelector$Type${}",
            self.selector
        )));
        result
    }

    fn description(&self) -> String {
        format!("sortedset({},selector={})", self.field, self.selector)
    }

    fn get_sort_field(self: Arc<Self>, reverse: bool) -> Result<SortField> {
        Ok(
            SortedSetSortField::with_selector(&self.field, reverse, self.selector)?
                .into_sort_field(),
        )
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
