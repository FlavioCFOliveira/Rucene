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

//! Port of `JoinDocFreqValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::terms::TermsEnum;
use rucene::index::{DocValues, EmptyTermsEnum, LeafReaderContext, MultiTerms, SortedDocValues};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::util::java_collections::java_string_hash_code;

use crate::function::docvalues::int_doc_values::{int_doc_values_members, IntDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{top_level_context, ValueSource, ValueSourceContext};
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};

/// Use a field value and find the document frequency within another field.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.JoinDocFreqValueSource`.
#[derive(Debug, Clone)]
pub struct JoinDocFreqValueSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
    /// The protected `qfield` field.
    qfield: String,
}

impl JoinDocFreqValueSource {
    /// The name of the function.
    ///
    /// Equivalent to `JoinDocFreqValueSource.NAME`.
    pub const NAME: &'static str = "joindf";

    /// Equivalent to `JoinDocFreqValueSource(String, String)`.
    pub fn new(field: impl Into<String>, qfield: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            qfield: qfield.into(),
        }
    }
}

/// The anonymous `IntDocValues` of `JoinDocFreqValueSource.getValues`.
struct JoinDocFreqValues {
    this: Arc<JoinDocFreqValueSource>,
    terms: Box<dyn SortedDocValues>,
    terms_enum: Box<dyn TermsEnum>,
    last_doc_id: i32,
}

impl FunctionValues for JoinDocFreqValues {
    fn int_val(&mut self, doc: i32) -> Result<i32> {
        if doc < self.last_doc_id {
            return Err(LuceneError::illegal_argument(format!(
                "docs were sent out-of-order: lastDocID={} vs docID={doc}",
                self.last_doc_id
            )));
        }
        self.last_doc_id = doc;
        let mut cur_doc_id = self.terms.doc_id();
        if doc > cur_doc_id {
            cur_doc_id = self.terms.advance(doc)?;
        }
        if doc == cur_doc_id {
            let term = self.terms.lookup_ord(self.terms.ord_value()?)?;
            if self.terms_enum.seek_exact(&term)? {
                return self.terms_enum.doc_freq();
            }
        }
        Ok(0)
    }

    int_doc_values_members!(
        byte_val,
        short_val,
        float_val,
        long_val,
        double_val,
        str_val,
        object_val,
        to_doc_string,
        get_range_matcher,
        get_value_filler
    );
}

impl IntDocValues for JoinDocFreqValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl FieldCacheSource for JoinDocFreqValueSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for JoinDocFreqValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let terms = DocValues::get_sorted(reader_context.leaf_reader().as_ref(), &self.field)?;
        let top = top_level_context(reader_context).reader();
        let t = MultiTerms::get_terms(&top, &self.qfield)?;
        let terms_enum: Box<dyn TermsEnum> = match t {
            None => Box::new(EmptyTermsEnum::new()),
            Some(t) => t.iterator()?,
        };
        Ok(Box::new(JoinDocFreqValues {
            this: self,
            terms,
            terms_enum,
            last_doc_id: -1,
        }))
    }

    /// `o.getClass() == JoinDocFreqValueSource.class`, an equal `qfield` and
    /// `super.equals(o)`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        let Some(that) = other.as_any().downcast_ref::<JoinDocFreqValueSource>() else {
            return false;
        };
        if self.qfield != that.qfield {
            return false;
        }
        field_cache_source::value_source_eq(self, other)
    }

    /// `qfield.hashCode() + super.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        java_string_hash_code(&self.qfield)
            .wrapping_add(field_cache_source::value_source_hash(self))
    }

    fn description(&self) -> String {
        format!("{}({}:({}))", Self::NAME, self.field, self.qfield)
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
