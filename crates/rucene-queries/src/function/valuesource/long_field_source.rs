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

//! Port of `LongFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, LeafReaderContext, NumericDocValues};
use rucene::search::sort::{SortField, SortFieldType};
use rucene::util::mutable::MutableValueObject;

use crate::function::docvalues::long_doc_values::{long_doc_values_members, LongDocValues};
use crate::function::function_values::{java_parse_long, FunctionValues};
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};
use crate::function::valuesource::int_field_source::numeric_exists;

/// Obtains long field values from `LeafReader.getNumericDocValues` and makes
/// those values available as other numeric types, casting as needed.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.LongFieldSource`.
#[derive(Debug, Clone)]
pub struct LongFieldSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
}

impl LongFieldSource {
    /// Equivalent to `LongFieldSource(String)`.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }

    /// Equivalent to `LongFieldSource.externalToLong(String)`:
    /// `Long.parseLong(extVal)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of a value that does not parse.
    pub fn external_to_long(&self, ext_val: &str) -> Result<i64> {
        java_parse_long(ext_val)
    }

    /// Equivalent to `LongFieldSource.longToObject(long)`: the boxed `Long`.
    pub fn long_to_object(&self, val: i64) -> MutableValueObject {
        MutableValueObject::Long(val)
    }

    /// Equivalent to `LongFieldSource.longToString(long)`:
    /// `longToObject(val).toString()`.
    pub fn long_to_string(&self, val: i64) -> String {
        val.to_string()
    }

    /// Equivalent to the protected
    /// `LongFieldSource.getNumericDocValues(Map, LeafReaderContext)`.
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

    /// The anonymous `LongDocValues` of `LongFieldSource.getValues`, over the
    /// doc values `getNumericDocValues` returned; `vs` is `this`, whose
    /// `externalToLong`, `longToObject` and `longToString` the values call.
    pub(crate) fn values(
        vs: Arc<dyn ValueSource>,
        arr: Box<dyn NumericDocValues>,
    ) -> Box<dyn FunctionValues> {
        Box::new(LongFieldValues {
            vs,
            arr,
            last_doc_id: 0,
        })
    }
}

/// The anonymous `LongDocValues` of `LongFieldSource.getValues`.
///
/// It calls `LongFieldSource.this.externalToLong`, `longToObject` and
/// `longToString`, which `LongFieldSource` and its subclass
/// `MultiValuedLongFieldSource` share unchanged; the bodies are those of
/// [`LongFieldSource`].
struct LongFieldValues {
    vs: Arc<dyn ValueSource>,
    arr: Box<dyn NumericDocValues>,
    last_doc_id: i32,
}

impl FunctionValues for LongFieldValues {
    fn long_val(&mut self, doc: i32) -> Result<i64> {
        if self.exists(doc)? {
            self.arr.long_value()
        } else {
            Ok(0)
        }
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        numeric_exists(&mut *self.arr, &mut self.last_doc_id, doc)
    }

    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        if self.exists(doc)? {
            let value = self.long_val(doc)?;
            Ok(Some(MutableValueObject::Long(value)))
        } else {
            Ok(None)
        }
    }

    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        if self.exists(doc)? {
            let value = self.long_val(doc)?;
            Ok(Some(value.to_string()))
        } else {
            Ok(None)
        }
    }

    long_doc_values_members!(
        byte_val,
        short_val,
        float_val,
        int_val,
        double_val,
        bool_val,
        to_doc_string,
        get_range_matcher,
        get_value_filler
    );
}

impl LongDocValues for LongFieldValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.vs
    }

    /// `LongFieldSource.this.externalToLong(extVal)`.
    fn external_to_long(&self, ext_val: &str) -> Result<i64> {
        java_parse_long(ext_val)
    }
}

impl FieldCacheSource for LongFieldSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for LongFieldSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let arr = self.get_numeric_doc_values(context, reader_context)?;
        Ok(LongFieldSource::values(self, arr))
    }

    /// `o.getClass() == this.getClass() && super.equals(o)`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other.as_any().downcast_ref::<LongFieldSource>().is_some()
            && field_cache_source::value_source_eq(self, other)
    }

    /// `getClass().hashCode() + super.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("org.apache.lucene.queries.function.valuesource.LongFieldSource")
            .wrapping_add(field_cache_source::value_source_hash(self))
    }

    fn description(&self) -> String {
        format!("long({})", self.field)
    }

    fn get_sort_field(self: Arc<Self>, reverse: bool) -> Result<SortField> {
        SortField::new_reverse(Some(self.field.clone()), SortFieldType::Long, reverse)
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
