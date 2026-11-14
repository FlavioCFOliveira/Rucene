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

//! Port of `ConstValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::similarities::ExplanationValue;
use rucene::util::java_fmt::float_to_string;
use rucene::util::mutable::MutableValueObject;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{float_to_int_bits, ValueSource, ValueSourceContext};
use crate::function::valuesource::const_number_source::ConstNumberSource;

/// `ConstValueSource` returns a constant for all documents.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.ConstValueSource`.
#[derive(Debug, Clone, Copy)]
pub struct ConstValueSource {
    /// The package-private `constant` field.
    constant: f32,
    /// The private `dv` field, `(double) constant`.
    dv: f64,
}

impl ConstValueSource {
    /// Equivalent to `ConstValueSource(float)`.
    pub fn new(constant: f32) -> Self {
        Self {
            constant,
            dv: f64::from(constant),
        }
    }
}

/// The anonymous `FloatDocValues` of `ConstValueSource.getValues`.
struct ConstValues {
    this: Arc<ConstValueSource>,
}

impl FunctionValues for ConstValues {
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        Ok(self.this.constant)
    }

    fn int_val(&mut self, _doc: i32) -> Result<i32> {
        Ok(self.this.constant as i32)
    }

    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Ok(self.this.constant as i64)
    }

    fn double_val(&mut self, _doc: i32) -> Result<f64> {
        Ok(self.this.dv)
    }

    fn to_doc_string(&mut self, _doc: i32) -> Result<String> {
        Ok(self.this.description())
    }

    fn object_val(&mut self, _doc: i32) -> Result<Option<MutableValueObject>> {
        Ok(Some(MutableValueObject::Float(self.this.constant)))
    }

    fn bool_val(&mut self, _doc: i32) -> Result<bool> {
        Ok(self.this.constant != 0.0)
    }

    float_doc_values_members!(byte_val, short_val, str_val, get_value_filler);
}

impl FloatDocValues for ConstValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for ConstValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        Ok(Box::new(ConstValues { this: self }))
    }

    fn description(&self) -> String {
        format!("const({})", float_to_string(self.constant))
    }

    /// `Float.floatToIntBits(constant) * 31`.
    fn value_source_hash(&self) -> i32 {
        float_to_int_bits(self.constant).wrapping_mul(31)
    }

    /// `o instanceof ConstValueSource && this.constant == other.constant`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<ConstValueSource>()
            .is_some_and(|other| self.constant == other.constant)
    }
}

impl ConstNumberSource for ConstValueSource {
    fn get_int(&self) -> i32 {
        self.constant as i32
    }

    fn get_long(&self) -> i64 {
        self.constant as i64
    }

    fn get_float(&self) -> f32 {
        self.constant
    }

    fn get_double(&self) -> f64 {
        self.dv
    }

    fn get_number(&self) -> ExplanationValue {
        ExplanationValue::Float(self.constant)
    }

    fn get_bool(&self) -> bool {
        self.constant != 0.0
    }
}
