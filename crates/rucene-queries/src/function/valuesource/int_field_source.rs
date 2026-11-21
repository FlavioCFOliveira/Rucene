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

//! Port of `IntFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{DocValues, LeafReaderContext, NumericDocValues};
use rucene::search::sort::{SortField, SortFieldType};

use crate::function::docvalues::int_doc_values::{int_doc_values_members, IntDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};

/// The `exists(int)` every numeric field source's values share: documents
/// must come in order, and the doc values are advanced to `doc`.
///
/// Equivalent to the `exists(int)` of the anonymous values of
/// `IntFieldSource`, `LongFieldSource`, `FloatFieldSource`,
/// `DoubleFieldSource` and `EnumFieldSource`, which are the same code.
///
/// # Errors
///
/// `IllegalArgumentException` when documents are sent out of order, and any
/// I/O error of the doc values.
pub(crate) fn numeric_exists(
    arr: &mut dyn NumericDocValues,
    last_doc_id: &mut i32,
    doc: i32,
) -> Result<bool> {
    if doc < *last_doc_id {
        return Err(LuceneError::illegal_argument(format!(
            "docs were sent out-of-order: lastDocID={} vs docID={doc}",
            *last_doc_id
        )));
    }
    *last_doc_id = doc;
    let mut cur_doc_id = arr.doc_id();
    if doc > cur_doc_id {
        cur_doc_id = arr.advance(doc)?;
    }
    Ok(doc == cur_doc_id)
}

/// Obtains integer field values from `LeafReader.getNumericDocValues` and
/// makes those values available as other numeric types, casting as needed.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.IntFieldSource`.
#[derive(Debug, Clone)]
pub struct IntFieldSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
}

impl IntFieldSource {
    /// Equivalent to `IntFieldSource(String)`.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }

    /// Equivalent to the protected
    /// `IntFieldSource.getNumericDocValues(Map, LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates the error of `DocValues.getNumeric`.
    pub fn get_numeric_doc_values(
        &self,
        _context: &ValueSourceContext<'_>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn NumericDocValues>> {
        DocValues::get_numeric(reader_context.leaf_reader().as_ref(), &self.field)
    }

    /// The anonymous `IntDocValues` of `IntFieldSource.getValues`, over the
    /// doc values `getNumericDocValues` returned; `vs` is `this`.
    pub(crate) fn values(
        vs: Arc<dyn ValueSource>,
        arr: Box<dyn NumericDocValues>,
    ) -> Box<dyn FunctionValues> {
        Box::new(IntFieldValues {
            vs,
            arr,
            last_doc_id: 0,
        })
    }
}

/// The anonymous `IntDocValues` of `IntFieldSource.getValues`.
struct IntFieldValues {
    vs: Arc<dyn ValueSource>,
    arr: Box<dyn NumericDocValues>,
    last_doc_id: i32,
}

impl FunctionValues for IntFieldValues {
    fn int_val(&mut self, doc: i32) -> Result<i32> {
        if self.exists(doc)? {
            Ok(self.arr.long_value()? as i32)
        } else {
            Ok(0)
        }
    }

    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        Ok(Some(self.int_val(doc)?.to_string()))
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        numeric_exists(&mut *self.arr, &mut self.last_doc_id, doc)
    }

    int_doc_values_members!(
        byte_val,
        short_val,
        float_val,
        long_val,
        double_val,
        object_val,
        to_doc_string,
        get_range_matcher,
        get_value_filler
    );
}

impl IntDocValues for IntFieldValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.vs
    }
}

impl FieldCacheSource for IntFieldSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for IntFieldSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let arr = self.get_numeric_doc_values(context, reader_context)?;
        Ok(IntFieldSource::values(self, arr))
    }

    /// `o.getClass() == IntFieldSource.class && super.equals(o)`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other.as_any().downcast_ref::<IntFieldSource>().is_some()
            && field_cache_source::value_source_eq(self, other)
    }

    /// `Integer.class.hashCode() + super.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("java.lang.Integer").wrapping_add(field_cache_source::value_source_hash(self))
    }

    fn description(&self) -> String {
        format!("int({})", self.field)
    }

    fn get_sort_field(self: Arc<Self>, reverse: bool) -> Result<SortField> {
        SortField::new_reverse(Some(self.field.clone()), SortFieldType::Int, reverse)
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
