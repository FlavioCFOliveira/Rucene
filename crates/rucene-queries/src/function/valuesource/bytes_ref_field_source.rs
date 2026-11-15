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

//! Port of `BytesRefFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{BinaryDocValues, DocValues, DocValuesType, LeafReaderContext};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::util::mutable::{MutableValue, MutableValueObject, MutableValueStr};
use rucene::util::BytesRefBuilder;

use crate::function::docvalues::doc_terms_index_doc_values::{
    doc_terms_index_doc_values_members, DocTermsIndexDocValues, DocTermsIndexState,
};
use crate::function::function_values::{FunctionValues, ValueFiller};
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};

/// An implementation for retrieving [`FunctionValues`] instances for string
/// based fields.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.BytesRefFieldSource`,
/// which reads binary doc values when the field has them and sorted doc
/// values otherwise. It inherits `FieldCacheSource`'s `equals`, `hashCode`
/// and `description`.
#[derive(Debug, Clone)]
pub struct BytesRefFieldSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
}

impl BytesRefFieldSource {
    /// Equivalent to `BytesRefFieldSource(String)`.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }
}

/// The anonymous `FunctionValues` of `BytesRefFieldSource.getValues` over a
/// binary doc-values field.
struct BinaryFieldValues {
    this: Arc<BytesRefFieldSource>,
    arr: Box<dyn BinaryDocValues>,
    last_doc_id: i32,
}

impl FunctionValues for BinaryFieldValues {
    fn exists(&mut self, doc: i32) -> Result<bool> {
        if doc < self.last_doc_id {
            return Err(LuceneError::illegal_argument(format!(
                "docs were sent out-of-order: lastDocID={} vs docID={doc}",
                self.last_doc_id
            )));
        }
        self.last_doc_id = doc;
        let mut cur_doc_id = self.arr.doc_id();
        if doc > cur_doc_id {
            cur_doc_id = self.arr.advance(doc)?;
        }
        Ok(doc == cur_doc_id)
    }

    fn bytes_val(&mut self, doc: i32, target: &mut BytesRefBuilder) -> Result<bool> {
        let value = if self.exists(doc)? {
            Some(self.arr.binary_value()?)
        } else {
            None
        };
        match value {
            Some(value) if value.length != 0 => {
                target.copy_bytes(value.slice(), 0, value.length);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        let mut bytes = BytesRefBuilder::new();
        if self.bytes_val(doc, &mut bytes)? {
            Ok(Some(bytes.get().utf8_to_string()?))
        } else {
            Ok(None)
        }
    }

    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        Ok(self.str_val(doc)?.map(MutableValueObject::Str))
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let value = self.str_val(doc)?;
        Ok(format!(
            "{}={}",
            self.this.description(),
            value.as_deref().unwrap_or("null")
        ))
    }

    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        Box::new(BinaryValuesFiller {
            mval: MutableValueStr::new(),
        })
    }
}

/// The anonymous `ValueFiller` of the binary values.
struct BinaryValuesFiller {
    mval: MutableValueStr,
}

impl ValueFiller for BinaryValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        self.mval.exists = values.exists(doc)?;
        self.mval.value.clear();
        values.bytes_val(doc, &mut self.mval.value)?;
        Ok(())
    }
}

/// The anonymous `DocTermsIndexDocValues` of `BytesRefFieldSource.getValues`
/// over a sorted doc-values field.
struct SortedFieldValues {
    this: Arc<BytesRefFieldSource>,
    state: DocTermsIndexState,
}

impl FunctionValues for SortedFieldValues {
    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        Ok(self.str_val(doc)?.map(MutableValueObject::Str))
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let value = self.str_val(doc)?;
        Ok(format!(
            "{}={}",
            self.this.description(),
            value.as_deref().unwrap_or("null")
        ))
    }

    doc_terms_index_doc_values_members!(
        exists,
        ord_val,
        num_ord,
        bytes_val,
        str_val,
        bool_val,
        get_range_matcher,
        get_value_filler
    );
}

impl DocTermsIndexDocValues for SortedFieldValues {
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

impl FieldCacheSource for BytesRefFieldSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for BytesRefFieldSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let reader = reader_context.leaf_reader();
        let field_infos = reader.get_field_infos();
        let is_binary = field_infos
            .field_info(&self.field)
            .is_some_and(|fi| fi.get_doc_values_type() == DocValuesType::BINARY);
        if is_binary {
            let arr = DocValues::get_binary(reader.as_ref(), &self.field)?;
            Ok(Box::new(BinaryFieldValues {
                this: self,
                arr,
                last_doc_id: -1,
            }))
        } else {
            let state = DocTermsIndexState::open(reader_context, &self.field)?;
            Ok(Box::new(SortedFieldValues { this: self, state }))
        }
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        field_cache_source::value_source_eq(self, other)
    }

    fn value_source_hash(&self) -> i32 {
        field_cache_source::value_source_hash(self)
    }

    fn description(&self) -> String {
        field_cache_source::description(self)
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
