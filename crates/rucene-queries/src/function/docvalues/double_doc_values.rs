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

//! Port of `DoubleDocValues.java`, of the Java package
//! `org.apache.lucene.queries.function.docvalues`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`; a subclass delegates the members it inherits to
//! them with
//! `double_doc_values_members!`.

use rucene::error::Result;
use rucene::util::java_fmt;
use rucene::util::mutable::{MutableValue, MutableValueDouble, MutableValueObject};

use crate::function::function_values::{
    f64_to_byte, f64_to_short, java_parse_double, FunctionValues, ValueFiller,
};
use crate::function::value_source::ValueSource;
use crate::function::value_source_scorer::ValueSourceScorerMatcher;

/// Abstract [`FunctionValues`] implementation which supports retrieving double
/// values. Implementations can control how the double values are loaded
/// through [`FunctionValues::double_val`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.docvalues.DoubleDocValues`.
///
/// **Divergence.** As for
/// [`FloatDocValues`](crate::function::docvalues::float_doc_values::FloatDocValues):
/// the class is this trait plus the module functions.
pub trait DoubleDocValues: FunctionValues {
    /// The protected `vs` field.
    fn vs(&self) -> &dyn ValueSource;
}

/// Equivalent to `DoubleDocValues.byteVal(int)`: `(byte) doubleVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `double_val`.
pub fn byte_val<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i8> {
    Ok(f64_to_byte(this.double_val(doc)?))
}

/// Equivalent to `DoubleDocValues.shortVal(int)`: `(short) doubleVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `double_val`.
pub fn short_val<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i16> {
    Ok(f64_to_short(this.double_val(doc)?))
}

/// Equivalent to `DoubleDocValues.floatVal(int)`: `(float) doubleVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `double_val`.
pub fn float_val<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f32> {
    Ok(this.double_val(doc)? as f32)
}

/// Equivalent to `DoubleDocValues.intVal(int)`: `(int) doubleVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `double_val`.
pub fn int_val<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i32> {
    Ok(this.double_val(doc)? as i32)
}

/// Equivalent to `DoubleDocValues.longVal(int)`: `(long) doubleVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `double_val`.
pub fn long_val<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i64> {
    Ok(this.double_val(doc)? as i64)
}

/// Equivalent to `DoubleDocValues.boolVal(int)`: `doubleVal(doc) != 0`.
///
/// # Errors
///
/// Propagates the error of `double_val`.
pub fn bool_val<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<bool> {
    Ok(this.double_val(doc)? != 0.0)
}

/// Equivalent to `DoubleDocValues.strVal(int)`:
/// `Double.toString(doubleVal(doc))`.
///
/// # Errors
///
/// Propagates the error of `double_val`.
pub fn str_val<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<Option<String>> {
    Ok(Some(java_fmt::double_to_string(this.double_val(doc)?)))
}

/// Equivalent to `DoubleDocValues.objectVal(int)`:
/// `exists(doc) ? doubleVal(doc) : null`.
///
/// # Errors
///
/// Propagates the errors of `exists` and `double_val`.
pub fn object_val<T: DoubleDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
) -> Result<Option<MutableValueObject>> {
    Ok(if this.exists(doc)? {
        Some(MutableValueObject::Double(this.double_val(doc)?))
    } else {
        None
    })
}

/// Equivalent to `DoubleDocValues.toString(int)`:
/// `vs.description() + '=' + strVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `str_val`.
pub fn to_doc_string<T: DoubleDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}={}",
        this.vs().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `DoubleDocValues.getRangeScorer(LeafReaderContext, String,
/// String, boolean, boolean)`: the bounds parsed with `Double.parseDouble`,
/// `null` bounds open, and one anonymous `ValueSourceScorer` per pair of
/// inclusion flags comparing `doubleVal(doc)`.
///
/// # Errors
///
/// The `NumberFormatException` of a bound that does not parse.
pub fn get_range_matcher<T: DoubleDocValues + ?Sized>(
    _this: &mut T,
    lower_val: Option<&str>,
    upper_val: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Result<Box<dyn ValueSourceScorerMatcher>> {
    let lower = match lower_val {
        None => f64::NEG_INFINITY,
        Some(s) => java_parse_double(s)?,
    };
    let upper = match upper_val {
        None => f64::INFINITY,
        Some(s) => java_parse_double(s)?,
    };
    Ok(Box::new(DoubleRangeMatcher {
        l: lower,
        u: upper,
        include_lower,
        include_upper,
    }))
}

/// The four anonymous `ValueSourceScorer`s of
/// `DoubleDocValues.getRangeScorer`, selected by the inclusion flags.
#[derive(Debug, Clone, Copy)]
struct DoubleRangeMatcher {
    l: f64,
    u: f64,
    include_lower: bool,
    include_upper: bool,
}

impl ValueSourceScorerMatcher for DoubleRangeMatcher {
    fn matches(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<bool> {
        if !values.exists(doc)? {
            return Ok(false);
        }
        let doc_val = values.double_val(doc)?;
        Ok(match (self.include_lower, self.include_upper) {
            (true, true) => doc_val >= self.l && doc_val <= self.u,
            (true, false) => doc_val >= self.l && doc_val < self.u,
            (false, true) => doc_val > self.l && doc_val <= self.u,
            (false, false) => doc_val > self.l && doc_val < self.u,
        })
    }
}

/// Equivalent to `DoubleDocValues.getValueFiller()`: a `MutableValueDouble`
/// filled with `doubleVal(doc)` and `exists(doc)`.
pub fn get_value_filler<T: DoubleDocValues + ?Sized>(_this: &T) -> Box<dyn ValueFiller> {
    Box::new(DoubleValuesFiller {
        mval: MutableValueDouble::new(),
    })
}

/// The anonymous `ValueFiller` of `DoubleDocValues.getValueFiller`.
struct DoubleValuesFiller {
    mval: MutableValueDouble,
}

impl ValueFiller for DoubleValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        self.mval.value = values.double_val(doc)?;
        self.mval.exists = values.exists(doc)?;
        Ok(())
    }
}

/// Delegates, inside an `impl FunctionValues` block, each listed member to
/// the `DoubleDocValues` member of the same name.
macro_rules! double_doc_values_members {
    ($($member:ident),* $(,)?) => {
        $( $crate::function::docvalues::double_doc_values::double_doc_values_members!(@ $member); )*
    };
    (@ byte_val) => {
        fn byte_val(&mut self, doc: i32) -> ::rucene::error::Result<i8> {
            $crate::function::docvalues::double_doc_values::byte_val(self, doc)
        }
    };
    (@ short_val) => {
        fn short_val(&mut self, doc: i32) -> ::rucene::error::Result<i16> {
            $crate::function::docvalues::double_doc_values::short_val(self, doc)
        }
    };
    (@ float_val) => {
        fn float_val(&mut self, doc: i32) -> ::rucene::error::Result<f32> {
            $crate::function::docvalues::double_doc_values::float_val(self, doc)
        }
    };
    (@ int_val) => {
        fn int_val(&mut self, doc: i32) -> ::rucene::error::Result<i32> {
            $crate::function::docvalues::double_doc_values::int_val(self, doc)
        }
    };
    (@ long_val) => {
        fn long_val(&mut self, doc: i32) -> ::rucene::error::Result<i64> {
            $crate::function::docvalues::double_doc_values::long_val(self, doc)
        }
    };
    (@ bool_val) => {
        fn bool_val(&mut self, doc: i32) -> ::rucene::error::Result<bool> {
            $crate::function::docvalues::double_doc_values::bool_val(self, doc)
        }
    };
    (@ str_val) => {
        fn str_val(&mut self, doc: i32) -> ::rucene::error::Result<Option<String>> {
            $crate::function::docvalues::double_doc_values::str_val(self, doc)
        }
    };
    (@ object_val) => {
        fn object_val(
            &mut self,
            doc: i32,
        ) -> ::rucene::error::Result<Option<::rucene::util::mutable::MutableValueObject>> {
            $crate::function::docvalues::double_doc_values::object_val(self, doc)
        }
    };
    (@ to_doc_string) => {
        fn to_doc_string(&mut self, doc: i32) -> ::rucene::error::Result<String> {
            $crate::function::docvalues::double_doc_values::to_doc_string(self, doc)
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
            $crate::function::docvalues::double_doc_values::get_range_matcher(
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
            $crate::function::docvalues::double_doc_values::get_value_filler(self)
        }
    };
}
pub(crate) use double_doc_values_members;
