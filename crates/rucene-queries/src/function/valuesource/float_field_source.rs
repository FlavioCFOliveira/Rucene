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

//! Port of `FloatFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, LeafReaderContext, NumericDocValues};
use rucene::search::sort::{SortField, SortFieldType};

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};
use crate::function::valuesource::int_field_source::numeric_exists;

/// Obtains float field values from `LeafReader.getNumericDocValues` and makes
/// those values available as other numeric types, casting as needed.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.FloatFieldSource`.
#[derive(Debug, Clone)]
pub struct FloatFieldSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
}

impl FloatFieldSource {
    /// Equivalent to `FloatFieldSource(String)`.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }

    /// Equivalent to the protected
    /// `FloatFieldSource.getNumericDocValues(Map, LeafReaderContext)`.
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

    /// The anonymous `FloatDocValues` of `FloatFieldSource.getValues`, over
    /// the doc values `getNumericDocValues` returned; `vs` is `this`.
    pub(crate) fn values(
        vs: Arc<dyn ValueSource>,
        arr: Box<dyn NumericDocValues>,
    ) -> Box<dyn FunctionValues> {
        Box::new(FloatFieldValues {
            vs,
            arr,
            last_doc_id: 0,
        })
    }
}

/// The anonymous `FloatDocValues` of `FloatFieldSource.getValues`.
struct FloatFieldValues {
    vs: Arc<dyn ValueSource>,
    arr: Box<dyn NumericDocValues>,
    last_doc_id: i32,
}

impl FunctionValues for FloatFieldValues {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        if self.exists(doc)? {
            Ok(f32::from_bits(self.arr.long_value()? as i32 as u32))
        } else {
            Ok(0.0)
        }
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        numeric_exists(&mut *self.arr, &mut self.last_doc_id, doc)
    }

    float_doc_values_members!(
        byte_val,
        short_val,
        int_val,
        long_val,
        bool_val,
        double_val,
        str_val,
        object_val,
        to_doc_string,
        get_value_filler
    );
}

impl FloatDocValues for FloatFieldValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.vs
    }
}

impl FieldCacheSource for FloatFieldSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for FloatFieldSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let arr = self.get_numeric_doc_values(context, reader_context)?;
        Ok(FloatFieldSource::values(self, arr))
    }

    /// `o.getClass() == FloatFieldSource.class && super.equals(o)`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other.as_any().downcast_ref::<FloatFieldSource>().is_some()
            && field_cache_source::value_source_eq(self, other)
    }

    /// `Float.class.hashCode() + super.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("java.lang.Float").wrapping_add(field_cache_source::value_source_hash(self))
    }

    fn description(&self) -> String {
        format!("float({})", self.field)
    }

    fn get_sort_field(self: Arc<Self>, reverse: bool) -> Result<SortField> {
        SortField::new_reverse(Some(self.field.clone()), SortFieldType::Float, reverse)
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
