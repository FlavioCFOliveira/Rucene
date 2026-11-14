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

//! Port of `IntDocValues.java`, of the Java package
//! `org.apache.lucene.queries.function.docvalues`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`; a subclass delegates the members it inherits to
//! them with
//! `int_doc_values_members!`.

use rucene::error::Result;
use rucene::util::mutable::{MutableValue, MutableValueInt, MutableValueObject};

use crate::function::function_values::{java_parse_int, FunctionValues, ValueFiller};
use crate::function::value_source::ValueSource;
use crate::function::value_source_scorer::ValueSourceScorerMatcher;

/// Abstract [`FunctionValues`] implementation which supports retrieving int
/// values. Implementations can control how the int values are loaded through
/// [`FunctionValues::int_val`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.docvalues.IntDocValues`.
///
/// **Divergence.** As for
/// [`FloatDocValues`](crate::function::docvalues::float_doc_values::FloatDocValues):
/// the class is this trait plus the module functions.
pub trait IntDocValues: FunctionValues {
    /// The protected `vs` field.
    fn vs(&self) -> &dyn ValueSource;
}

/// Equivalent to `IntDocValues.byteVal(int)`: `(byte) intVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `int_val`.
pub fn byte_val<T: IntDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i8> {
    Ok(this.int_val(doc)? as i8)
}

/// Equivalent to `IntDocValues.shortVal(int)`: `(short) intVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `int_val`.
pub fn short_val<T: IntDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i16> {
    Ok(this.int_val(doc)? as i16)
}

/// Equivalent to `IntDocValues.floatVal(int)`: `(float) intVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `int_val`.
pub fn float_val<T: IntDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f32> {
    Ok(this.int_val(doc)? as f32)
}

/// Equivalent to `IntDocValues.longVal(int)`: `(long) intVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `int_val`.
pub fn long_val<T: IntDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i64> {
    Ok(i64::from(this.int_val(doc)?))
}

/// Equivalent to `IntDocValues.doubleVal(int)`: `(double) intVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `int_val`.
pub fn double_val<T: IntDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f64> {
    Ok(f64::from(this.int_val(doc)?))
}

/// Equivalent to `IntDocValues.strVal(int)`: `Integer.toString(intVal(doc))`.
///
/// # Errors
///
/// Propagates the error of `int_val`.
pub fn str_val<T: IntDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<Option<String>> {
    Ok(Some(this.int_val(doc)?.to_string()))
}

/// Equivalent to `IntDocValues.objectVal(int)`:
/// `exists(doc) ? intVal(doc) : null`.
///
/// # Errors
///
/// Propagates the errors of `exists` and `int_val`.
pub fn object_val<T: IntDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
) -> Result<Option<MutableValueObject>> {
    Ok(if this.exists(doc)? {
        Some(MutableValueObject::Int(this.int_val(doc)?))
    } else {
        None
    })
}

/// Equivalent to `IntDocValues.toString(int)`:
/// `vs.description() + '=' + strVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `str_val`.
pub fn to_doc_string<T: IntDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}={}",
        this.vs().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `IntDocValues.getRangeScorer(LeafReaderContext, String,
/// String, boolean, boolean)`: instead of using separate comparison functions,
/// the endpoints are adjusted, and the anonymous `ValueSourceScorer` matches
/// `exists(doc)` with `intVal(doc)` in `[ll, uu]`.
///
/// # Errors
///
/// The `NumberFormatException` of a bound that does not parse.
pub fn get_range_matcher<T: IntDocValues + ?Sized>(
    _this: &mut T,
    lower_val: Option<&str>,
    upper_val: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Result<Box<dyn ValueSourceScorerMatcher>> {
    // instead of using separate comparison functions, adjust the endpoints.
    let lower = match lower_val {
        None => i32::MIN,
        Some(s) => {
            let mut lower = java_parse_int(s)?;
            if !include_lower && lower < i32::MAX {
                lower += 1;
            }
            lower
        }
    };
    let upper = match upper_val {
        None => i32::MAX,
        Some(s) => {
            let mut upper = java_parse_int(s)?;
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

/// The anonymous `ValueSourceScorer` of `IntDocValues.getRangeScorer`, which
/// `EnumFieldSource`'s values reuse with their own bounds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IntRangeMatcher {
    /// The adjusted lower bound.
    pub(crate) ll: i32,
    /// The adjusted upper bound.
    pub(crate) uu: i32,
}

impl ValueSourceScorerMatcher for IntRangeMatcher {
    fn matches(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<bool> {
        if !values.exists(doc)? {
            return Ok(false);
        }
        let val = values.int_val(doc)?;
        Ok(val >= self.ll && val <= self.uu)
    }
}

/// Equivalent to `IntDocValues.getValueFiller()`: a `MutableValueInt` filled
/// with `intVal(doc)` and `exists(doc)`.
pub fn get_value_filler<T: IntDocValues + ?Sized>(_this: &T) -> Box<dyn ValueFiller> {
    Box::new(IntValuesFiller {
        mval: MutableValueInt::new(),
    })
}

/// The anonymous `ValueFiller` of `IntDocValues.getValueFiller`.
struct IntValuesFiller {
    mval: MutableValueInt,
}

impl ValueFiller for IntValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        self.mval.value = values.int_val(doc)?;
        self.mval.exists = values.exists(doc)?;
        Ok(())
    }
}

/// Delegates, inside an `impl FunctionValues` block, each listed member to
/// the `IntDocValues` member of the same name.
macro_rules! int_doc_values_members {
    ($($member:ident),* $(,)?) => {
        $( $crate::function::docvalues::int_doc_values::int_doc_values_members!(@ $member); )*
    };
    (@ byte_val) => {
        fn byte_val(&mut self, doc: i32) -> ::rucene::error::Result<i8> {
            $crate::function::docvalues::int_doc_values::byte_val(self, doc)
        }
    };
    (@ short_val) => {
        fn short_val(&mut self, doc: i32) -> ::rucene::error::Result<i16> {
            $crate::function::docvalues::int_doc_values::short_val(self, doc)
        }
    };
    (@ float_val) => {
        fn float_val(&mut self, doc: i32) -> ::rucene::error::Result<f32> {
            $crate::function::docvalues::int_doc_values::float_val(self, doc)
        }
    };
    (@ long_val) => {
        fn long_val(&mut self, doc: i32) -> ::rucene::error::Result<i64> {
            $crate::function::docvalues::int_doc_values::long_val(self, doc)
        }
    };
    (@ double_val) => {
        fn double_val(&mut self, doc: i32) -> ::rucene::error::Result<f64> {
            $crate::function::docvalues::int_doc_values::double_val(self, doc)
        }
    };
    (@ str_val) => {
        fn str_val(&mut self, doc: i32) -> ::rucene::error::Result<Option<String>> {
            $crate::function::docvalues::int_doc_values::str_val(self, doc)
        }
    };
    (@ object_val) => {
        fn object_val(
            &mut self,
            doc: i32,
        ) -> ::rucene::error::Result<Option<::rucene::util::mutable::MutableValueObject>> {
            $crate::function::docvalues::int_doc_values::object_val(self, doc)
        }
    };
    (@ to_doc_string) => {
        fn to_doc_string(&mut self, doc: i32) -> ::rucene::error::Result<String> {
            $crate::function::docvalues::int_doc_values::to_doc_string(self, doc)
        }
    };
    (@ get_range_matcher) => {
        fn get_range_matcher(
            &mut self,
            lower_val: Option<&str>,
            upper_val: Option<&str>,
            include_lower: bool,
            include_upper: bool,
        ) -> ::rucene::error::Result<
            Box<dyn $crate::function::value_source_scorer::ValueSourceScorerMatcher>,
        > {
            $crate::function::docvalues::int_doc_values::get_range_matcher(
                self,
                lower_val,
                upper_val,
                include_lower,
                include_upper,
            )
        }
    };
    (@ get_value_filler) => {
        fn get_value_filler(&self) -> Box<dyn $crate::function::function_values::ValueFiller> {
            $crate::function::docvalues::int_doc_values::get_value_filler(self)
        }
    };
}
pub(crate) use int_doc_values_members;
