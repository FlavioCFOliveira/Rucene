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

//! Port of `DoubleConstValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::similarities::ExplanationValue;
use rucene::util::java_fmt::double_to_string;
use rucene::util::mutable::MutableValueObject;

use crate::function::docvalues::double_doc_values::{double_doc_values_members, DoubleDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::const_number_source::ConstNumberSource;

/// Function that returns a constant double value for every document.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.DoubleConstValueSource`.
#[derive(Debug, Clone, Copy)]
pub struct DoubleConstValueSource {
    /// The package-private `constant` field.
    constant: f64,
    /// The private `fv` field, `(float) constant`.
    fv: f32,
    /// The private `lv` field, `(long) constant`.
    lv: i64,
}

impl DoubleConstValueSource {
    /// Equivalent to `DoubleConstValueSource(double)`.
    pub fn new(constant: f64) -> Self {
        Self {
            constant,
            fv: constant as f32,
            lv: constant as i64,
        }
    }
}

/// The anonymous `DoubleDocValues` of `DoubleConstValueSource.getValues`.
struct DoubleConstValues {
    this: Arc<DoubleConstValueSource>,
}

impl FunctionValues for DoubleConstValues {
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        Ok(self.this.fv)
    }

    fn int_val(&mut self, _doc: i32) -> Result<i32> {
        Ok(self.this.lv as i32)
    }

    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Ok(self.this.lv)
    }

    fn double_val(&mut self, _doc: i32) -> Result<f64> {
        Ok(self.this.constant)
    }

    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Ok(Some(double_to_string(self.this.constant)))
    }

    fn object_val(&mut self, _doc: i32) -> Result<Option<MutableValueObject>> {
        Ok(Some(MutableValueObject::Double(self.this.constant)))
    }

    fn to_doc_string(&mut self, _doc: i32) -> Result<String> {
        Ok(self.this.description())
    }

    double_doc_values_members!(
        byte_val,
        short_val,
        bool_val,
        get_range_matcher,
        get_value_filler
    );
}

impl DoubleDocValues for DoubleConstValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for DoubleConstValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        Ok(Box::new(DoubleConstValues { this: self }))
    }

    fn description(&self) -> String {
        format!("const({})", double_to_string(self.constant))
    }

    /// `(int) (bits ^ (bits >>> 32))` of `Double.doubleToRawLongBits(constant)`.
    fn value_source_hash(&self) -> i32 {
        let bits = self.constant.to_bits();
        (bits ^ (bits >> 32)) as i32
    }

    /// `o instanceof DoubleConstValueSource && this.constant == other.constant`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<DoubleConstValueSource>()
            .is_some_and(|other| self.constant == other.constant)
    }
}

impl ConstNumberSource for DoubleConstValueSource {
    fn get_int(&self) -> i32 {
        self.lv as i32
    }

    fn get_long(&self) -> i64 {
        self.lv
    }

    fn get_float(&self) -> f32 {
        self.fv
    }

    fn get_double(&self) -> f64 {
        self.constant
    }

    fn get_number(&self) -> ExplanationValue {
        ExplanationValue::Double(self.constant)
    }

    fn get_bool(&self) -> bool {
        self.constant != 0.0
    }
}
