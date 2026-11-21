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

//! Port of `EnumFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, LeafReaderContext, NumericDocValues};
use rucene::util::java_collections::java_string_hash_code;

use crate::function::docvalues::int_doc_values::{
    int_doc_values_members, IntDocValues, IntRangeMatcher,
};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::value_source_scorer::ValueSourceScorerMatcher;
use crate::function::valuesource::field_cache_source::{self, FieldCacheSource};
use crate::function::valuesource::int_field_source::numeric_exists;

/// The value of an unknown enum string or integer.
///
/// Equivalent to the package-private `EnumFieldSource.DEFAULT_VALUE`.
pub(crate) const DEFAULT_VALUE: i32 = -1;

/// Obtains integer field values from `LeafReader.getNumericDocValues` and
/// makes them available as an enum: each integer maps to a string.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.EnumFieldSource`.
///
/// The two maps are compared and hashed as Java's `Map.equals` and
/// `Map.hashCode` do, which do not depend on iteration order.
#[derive(Debug, Clone)]
pub struct EnumFieldSource {
    /// The protected `field` of `FieldCacheSource`.
    field: String,
    /// The package-private `enumIntToStringMap` field.
    enum_int_to_string_map: HashMap<i32, String>,
    /// The package-private `enumStringToIntMap` field.
    enum_string_to_int_map: HashMap<String, i32>,
}

impl EnumFieldSource {
    /// Equivalent to `EnumFieldSource(String, Map<Integer, String>,
    /// Map<String, Integer>)`.
    pub fn new(
        field: impl Into<String>,
        enum_int_to_string_map: HashMap<i32, String>,
        enum_string_to_int_map: HashMap<String, i32>,
    ) -> Self {
        Self {
            field: field.into(),
            enum_int_to_string_map,
            enum_string_to_int_map,
        }
    }

    /// Equivalent to the private static `EnumFieldSource.tryParseInt(String)`:
    /// `Integer.parseInt`, `null` where it throws.
    fn try_parse_int(value_str: &str) -> Option<i32> {
        rucene::util::extra::java_parse_int(value_str)
    }

    /// Equivalent to the private
    /// `EnumFieldSource.intValueToStringValue(Integer)`; `None` stands for
    /// `null`.
    fn int_value_to_string_value(&self, int_val: Option<i32>) -> Option<String> {
        let int_val = int_val?;
        if let Some(enum_string) = self.enum_int_to_string_map.get(&int_val) {
            return Some(enum_string.clone());
        }
        Some(DEFAULT_VALUE.to_string())
    }

    /// Equivalent to the private
    /// `EnumFieldSource.stringValueToIntValue(String)`; `None` stands for
    /// `null`.
    fn string_value_to_int_value(&self, string_val: Option<&str>) -> Option<i32> {
        let string_val = string_val?;
        if let Some(&enum_int) = self.enum_string_to_int_map.get(string_val) {
            // enum int found for string
            return Some(enum_int);
        }
        // not Integer
        let int_value = Self::try_parse_int(string_val).unwrap_or(DEFAULT_VALUE);
        if self.enum_int_to_string_map.contains_key(&int_value) {
            // has matching string
            return Some(int_value);
        }
        Some(DEFAULT_VALUE)
    }
}

/// The anonymous `IntDocValues` of `EnumFieldSource.getValues`.
struct EnumFieldValues {
    this: Arc<EnumFieldSource>,
    arr: Box<dyn NumericDocValues>,
    last_doc_id: i32,
}

impl FunctionValues for EnumFieldValues {
    fn int_val(&mut self, doc: i32) -> Result<i32> {
        if self.exists(doc)? {
            Ok(self.arr.long_value()? as i32)
        } else {
            Ok(0)
        }
    }

    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        let int_value = self.int_val(doc)?;
        Ok(self.this.int_value_to_string_value(Some(int_value)))
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        numeric_exists(&mut *self.arr, &mut self.last_doc_id, doc)
    }

    /// The `getRangeScorer` override: the bounds go through
    /// `stringValueToIntValue`, then are adjusted as `IntDocValues` does.
    fn get_range_matcher(
        &mut self,
        lower_val: Option<&str>,
        upper_val: Option<&str>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Box<dyn ValueSourceScorerMatcher>> {
        let lower = self.this.string_value_to_int_value(lower_val);
        let upper = self.this.string_value_to_int_value(upper_val);

        let lower = match lower {
            None => i32::MIN,
            Some(mut lower) => {
                if !include_lower && lower < i32::MAX {
                    lower += 1;
                }
                lower
            }
        };
        let upper = match upper {
            None => i32::MAX,
            Some(mut upper) => {
                if !include_upper && upper > i32::MIN {
                    upper -= 1;
                }
                upper
            }
        };
        Ok(Box::new(IntRangeMatcher {
            ll: lower,
            uu: upper,
        }))
    }

    int_doc_values_members!(
        byte_val,
        short_val,
        float_val,
        long_val,
        double_val,
        object_val,
        to_doc_string,
        get_value_filler
    );
}

impl IntDocValues for EnumFieldValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl FieldCacheSource for EnumFieldSource {
    fn get_field(&self) -> &str {
        &self.field
    }
}

impl ValueSource for EnumFieldSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let arr = DocValues::get_numeric(reader_context.leaf_reader().as_ref(), &self.field)?;
        Ok(Box::new(EnumFieldValues {
            this: self,
            arr,
            last_doc_id: 0,
        }))
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        let Some(that) = other.as_any().downcast_ref::<EnumFieldSource>() else {
            return false;
        };
        if !field_cache_source::value_source_eq(self, other) {
            return false;
        }
        self.enum_int_to_string_map == that.enum_int_to_string_map
            && self.enum_string_to_int_map == that.enum_string_to_int_map
    }

    /// `31 * (31 * super.hashCode() + enumIntToStringMap.hashCode()) +
    /// enumStringToIntMap.hashCode()`, a map's hash code being the sum of
    /// `key.hashCode() ^ value.hashCode()` over its entries.
    fn value_source_hash(&self) -> i32 {
        let int_to_string = self.enum_int_to_string_map.iter().fold(0i32, |h, (k, v)| {
            h.wrapping_add(*k ^ java_string_hash_code(v))
        });
        let string_to_int = self.enum_string_to_int_map.iter().fold(0i32, |h, (k, v)| {
            h.wrapping_add(java_string_hash_code(k) ^ *v)
        });
        let mut result = field_cache_source::value_source_hash(self);
        result = result.wrapping_mul(31).wrapping_add(int_to_string);
        result = result.wrapping_mul(31).wrapping_add(string_to_int);
        result
    }

    fn description(&self) -> String {
        format!("enum({})", self.field)
    }

    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        Some(self)
    }
}
