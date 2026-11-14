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

//! Port of `LongDocValues.java`, of the Java package
//! `org.apache.lucene.queries.function.docvalues`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`; a subclass delegates the members it inherits to
//! them with
//! `long_doc_values_members!`.

use rucene::error::Result;
use rucene::util::mutable::{MutableValue, MutableValueLong, MutableValueObject};

use crate::function::function_values::{java_parse_long, FunctionValues, ValueFiller};
use crate::function::value_source::ValueSource;
use crate::function::value_source_scorer::ValueSourceScorerMatcher;

/// Abstract [`FunctionValues`] implementation which supports retrieving long
/// values. Implementations can control how the long values are loaded through
/// [`FunctionValues::long_val`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.docvalues.LongDocValues`.
///
/// **Divergence.** As for
/// [`FloatDocValues`](crate::function::docvalues::float_doc_values::FloatDocValues):
/// the class is this trait plus the module functions.
pub trait LongDocValues: FunctionValues {
    /// The protected `vs` field.
    fn vs(&self) -> &dyn ValueSource;

    /// Converts the external representation of a bound into a `long`.
    ///
    /// Equivalent to the protected `LongDocValues.externalToLong(String)`:
    /// `Long.parseLong(extVal)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of a value that does not parse.
    fn external_to_long(&self, ext_val: &str) -> Result<i64> {
        java_parse_long(ext_val)
    }
}

/// Equivalent to `LongDocValues.byteVal(int)`: `(byte) longVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `long_val`.
pub fn byte_val<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i8> {
    Ok(this.long_val(doc)? as i8)
}

/// Equivalent to `LongDocValues.shortVal(int)`: `(short) longVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `long_val`.
pub fn short_val<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i16> {
    Ok(this.long_val(doc)? as i16)
}

/// Equivalent to `LongDocValues.floatVal(int)`: `(float) longVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `long_val`.
pub fn float_val<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f32> {
    Ok(this.long_val(doc)? as f32)
}

/// Equivalent to `LongDocValues.intVal(int)`: `(int) longVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `long_val`.
pub fn int_val<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i32> {
    Ok(this.long_val(doc)? as i32)
}

/// Equivalent to `LongDocValues.doubleVal(int)`: `(double) longVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `long_val`.
pub fn double_val<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f64> {
    Ok(this.long_val(doc)? as f64)
}

/// Equivalent to `LongDocValues.boolVal(int)`: `longVal(doc) != 0`.
///
/// # Errors
///
/// Propagates the error of `long_val`.
pub fn bool_val<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<bool> {
    Ok(this.long_val(doc)? != 0)
}

/// Equivalent to `LongDocValues.strVal(int)`: `Long.toString(longVal(doc))`.
///
/// # Errors
///
/// Propagates the error of `long_val`.
pub fn str_val<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<Option<String>> {
    Ok(Some(this.long_val(doc)?.to_string()))
}

/// Equivalent to `LongDocValues.objectVal(int)`:
/// `exists(doc) ? longVal(doc) : null`.
///
/// # Errors
///
/// Propagates the errors of `exists` and `long_val`.
pub fn object_val<T: LongDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
) -> Result<Option<MutableValueObject>> {
    Ok(if this.exists(doc)? {
        Some(MutableValueObject::Long(this.long_val(doc)?))
    } else {
        None
    })
}

/// Equivalent to `LongDocValues.toString(int)`:
/// `vs.description() + '=' + strVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `str_val`.
pub fn to_doc_string<T: LongDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}={}",
        this.vs().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `LongDocValues.getRangeScorer(LeafReaderContext, String,
/// String, boolean, boolean)`: the endpoints, converted with
/// [`LongDocValues::external_to_long`], are adjusted instead of using separate
/// comparison functions, and the anonymous `ValueSourceScorer` matches
/// `exists(doc)` with `longVal(doc)` in `[ll, uu]`.
///
/// # Errors
///
/// The error of a bound that does not convert.
pub fn get_range_matcher<T: LongDocValues + ?Sized>(
    this: &mut T,
    lower_val: Option<&str>,
    upper_val: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Result<Box<dyn ValueSourceScorerMatcher>> {
    // instead of using separate comparison functions, adjust the endpoints.
    let lower = match lower_val {
        None => i64::MIN,
        Some(s) => {
            let mut lower = this.external_to_long(s)?;
            if !include_lower && lower < i64::MAX {
                lower += 1;
            }
            lower
        }
    };
    let upper = match upper_val {
        None => i64::MAX,
        Some(s) => {
            let mut upper = this.external_to_long(s)?;
            if !include_upper && upper > i64::MIN {
                upper -= 1;
            }
            upper
        }
    };
    Ok(Box::new(LongRangeMatcher {
        ll: lower,
        uu: upper,
    }))
}

/// The anonymous `ValueSourceScorer` of `LongDocValues.getRangeScorer`.
#[derive(Debug, Clone, Copy)]
struct LongRangeMatcher {
    ll: i64,
    uu: i64,
}

impl ValueSourceScorerMatcher for LongRangeMatcher {
    fn matches(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<bool> {
        if !values.exists(doc)? {
            return Ok(false);
        }
        let val = values.long_val(doc)?;
        Ok(val >= self.ll && val <= self.uu)
    }
}

/// Equivalent to `LongDocValues.getValueFiller()`: a `MutableValueLong`
/// filled with `longVal(doc)` and `exists(doc)`.
pub fn get_value_filler<T: LongDocValues + ?Sized>(_this: &T) -> Box<dyn ValueFiller> {
    Box::new(LongValuesFiller {
        mval: MutableValueLong::new(),
    })
}

/// The anonymous `ValueFiller` of `LongDocValues.getValueFiller`.
struct LongValuesFiller {
    mval: MutableValueLong,
}

impl ValueFiller for LongValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        self.mval.value = values.long_val(doc)?;
        self.mval.exists = values.exists(doc)?;
        Ok(())
    }
}

/// Delegates, inside an `impl FunctionValues` block, each listed member to
/// the `LongDocValues` member of the same name.
macro_rules! long_doc_values_members {
    ($($member:ident),* $(,)?) => {
        $( $crate::function::docvalues::long_doc_values::long_doc_values_members!(@ $member); )*
    };
    (@ byte_val) => {
        fn byte_val(&mut self, doc: i32) -> ::rucene::error::Result<i8> {
            $crate::function::docvalues::long_doc_values::byte_val(self, doc)
        }
    };
    (@ short_val) => {
        fn short_val(&mut self, doc: i32) -> ::rucene::error::Result<i16> {
            $crate::function::docvalues::long_doc_values::short_val(self, doc)
        }
    };
    (@ float_val) => {
        fn float_val(&mut self, doc: i32) -> ::rucene::error::Result<f32> {
            $crate::function::docvalues::long_doc_values::float_val(self, doc)
        }
    };
    (@ int_val) => {
        fn int_val(&mut self, doc: i32) -> ::rucene::error::Result<i32> {
            $crate::function::docvalues::long_doc_values::int_val(self, doc)
        }
    };
    (@ double_val) => {
        fn double_val(&mut self, doc: i32) -> ::rucene::error::Result<f64> {
            $crate::function::docvalues::long_doc_values::double_val(self, doc)
        }
    };
    (@ bool_val) => {
        fn bool_val(&mut self, doc: i32) -> ::rucene::error::Result<bool> {
            $crate::function::docvalues::long_doc_values::bool_val(self, doc)
        }
    };
    (@ str_val) => {
        fn str_val(&mut self, doc: i32) -> ::rucene::error::Result<Option<String>> {
            $crate::function::docvalues::long_doc_values::str_val(self, doc)
        }
    };
    (@ object_val) => {
        fn object_val(
            &mut self,
            doc: i32,
        ) -> ::rucene::error::Result<Option<::rucene::util::mutable::MutableValueObject>> {
            $crate::function::docvalues::long_doc_values::object_val(self, doc)
        }
    };
    (@ to_doc_string) => {
        fn to_doc_string(&mut self, doc: i32) -> ::rucene::error::Result<String> {
            $crate::function::docvalues::long_doc_values::to_doc_string(self, doc)
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
            $crate::function::docvalues::long_doc_values::get_range_matcher(
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
            $crate::function::docvalues::long_doc_values::get_value_filler(self)
        }
    };
}
pub(crate) use long_doc_values_members;
