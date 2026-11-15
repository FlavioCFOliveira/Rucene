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

//! Port of `FloatDocValues.java`, of the Java package
//! `org.apache.lucene.queries.function.docvalues`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`; a subclass implements [`FunctionValues`] and
//! delegates each member it inherits to them with
//! `float_doc_values_members!`,
//! one line per member, and writes the members it overrides.

use rucene::error::Result;
use rucene::util::java_fmt;
use rucene::util::mutable::MutableValueObject;

use crate::function::function_values::{
    f32_to_byte, f32_to_short, FloatValuesFiller, FunctionValues, ValueFiller,
};
use crate::function::value_source::ValueSource;

/// Abstract [`FunctionValues`] implementation which supports retrieving float
/// values. Implementations can control how the float values are loaded through
/// [`FunctionValues::float_val`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.docvalues.FloatDocValues`.
///
/// **Divergence.** Java's abstract class carries the protected `vs` field and
/// the members below; a Rust trait cannot override its supertrait's methods,
/// so the class is this trait, exposing `vs`, plus the module functions that
/// hold the members' bodies (see the module documentation).
pub trait FloatDocValues: FunctionValues {
    /// The protected `vs` field.
    fn vs(&self) -> &dyn ValueSource;
}

/// Equivalent to `FloatDocValues.byteVal(int)`: `(byte) floatVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `float_val`.
pub fn byte_val<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i8> {
    Ok(f32_to_byte(this.float_val(doc)?))
}

/// Equivalent to `FloatDocValues.shortVal(int)`: `(short) floatVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `float_val`.
pub fn short_val<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i16> {
    Ok(f32_to_short(this.float_val(doc)?))
}

/// Equivalent to `FloatDocValues.intVal(int)`: `(int) floatVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `float_val`.
pub fn int_val<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i32> {
    Ok(this.float_val(doc)? as i32)
}

/// Equivalent to `FloatDocValues.longVal(int)`: `(long) floatVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `float_val`.
pub fn long_val<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i64> {
    Ok(this.float_val(doc)? as i64)
}

/// Equivalent to `FloatDocValues.boolVal(int)`: `floatVal(doc) != 0.0f`.
///
/// # Errors
///
/// Propagates the error of `float_val`.
pub fn bool_val<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<bool> {
    Ok(this.float_val(doc)? != 0.0)
}

/// Equivalent to `FloatDocValues.doubleVal(int)`: `(double) floatVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `float_val`.
pub fn double_val<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f64> {
    Ok(f64::from(this.float_val(doc)?))
}

/// Equivalent to `FloatDocValues.strVal(int)`: `Float.toString(floatVal(doc))`.
///
/// # Errors
///
/// Propagates the error of `float_val`.
pub fn str_val<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<Option<String>> {
    Ok(Some(java_fmt::float_to_string(this.float_val(doc)?)))
}

/// Equivalent to `FloatDocValues.objectVal(int)`:
/// `exists(doc) ? floatVal(doc) : null`.
///
/// # Errors
///
/// Propagates the errors of `exists` and `float_val`.
pub fn object_val<T: FloatDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
) -> Result<Option<MutableValueObject>> {
    Ok(if this.exists(doc)? {
        Some(MutableValueObject::Float(this.float_val(doc)?))
    } else {
        None
    })
}

/// Equivalent to `FloatDocValues.toString(int)`:
/// `vs.description() + '=' + strVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `str_val`.
pub fn to_doc_string<T: FloatDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}={}",
        this.vs().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `FloatDocValues.getValueFiller()`: a `MutableValueFloat`
/// filled with `floatVal(doc)` and `exists(doc)`.
pub fn get_value_filler<T: FloatDocValues + ?Sized>(_this: &T) -> Box<dyn ValueFiller> {
    Box::new(FloatValuesFiller::new(true))
}

/// Delegates, inside an `impl FunctionValues` block, each listed member to
/// the `FloatDocValues` member of the same name: the members a subclass
/// inherits without overriding them.
macro_rules! float_doc_values_members {
    ($($member:ident),* $(,)?) => {
        $( $crate::function::docvalues::float_doc_values::float_doc_values_members!(@ $member); )*
    };
    (@ byte_val) => {
        fn byte_val(&mut self, doc: i32) -> ::rucene::error::Result<i8> {
            $crate::function::docvalues::float_doc_values::byte_val(self, doc)
        }
    };
    (@ short_val) => {
        fn short_val(&mut self, doc: i32) -> ::rucene::error::Result<i16> {
            $crate::function::docvalues::float_doc_values::short_val(self, doc)
        }
    };
    (@ int_val) => {
        fn int_val(&mut self, doc: i32) -> ::rucene::error::Result<i32> {
            $crate::function::docvalues::float_doc_values::int_val(self, doc)
        }
    };
    (@ long_val) => {
        fn long_val(&mut self, doc: i32) -> ::rucene::error::Result<i64> {
            $crate::function::docvalues::float_doc_values::long_val(self, doc)
        }
    };
    (@ bool_val) => {
        fn bool_val(&mut self, doc: i32) -> ::rucene::error::Result<bool> {
            $crate::function::docvalues::float_doc_values::bool_val(self, doc)
        }
    };
    (@ double_val) => {
        fn double_val(&mut self, doc: i32) -> ::rucene::error::Result<f64> {
            $crate::function::docvalues::float_doc_values::double_val(self, doc)
        }
    };
    (@ str_val) => {
        fn str_val(&mut self, doc: i32) -> ::rucene::error::Result<Option<String>> {
            $crate::function::docvalues::float_doc_values::str_val(self, doc)
        }
    };
    (@ object_val) => {
        fn object_val(
            &mut self,
            doc: i32,
        ) -> ::rucene::error::Result<Option<::rucene::util::mutable::MutableValueObject>> {
            $crate::function::docvalues::float_doc_values::object_val(self, doc)
        }
    };
    (@ to_doc_string) => {
        fn to_doc_string(&mut self, doc: i32) -> ::rucene::error::Result<String> {
            $crate::function::docvalues::float_doc_values::to_doc_string(self, doc)
        }
    };
    (@ get_value_filler) => {
        fn get_value_filler(&self) -> Box<dyn $crate::function::function_values::ValueFiller> {
            $crate::function::docvalues::float_doc_values::get_value_filler(self)
        }
    };
}
pub(crate) use float_doc_values_members;
