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

//! Port of `BoolDocValues.java`, of the Java package
//! `org.apache.lucene.queries.function.docvalues`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`; a subclass delegates the members it inherits to
//! them with
//! `bool_doc_values_members!`.

use rucene::error::Result;
use rucene::util::mutable::{MutableValue, MutableValueBool, MutableValueObject};

use crate::function::function_values::{FunctionValues, ValueFiller};
use crate::function::value_source::ValueSource;

/// Abstract [`FunctionValues`] implementation which supports retrieving
/// boolean values. Implementations can control how the boolean values are
/// loaded through [`FunctionValues::bool_val`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.docvalues.BoolDocValues`, whose
/// `boolVal(int)` is abstract.
///
/// **Divergence.** As for
/// [`FloatDocValues`](crate::function::docvalues::float_doc_values::FloatDocValues):
/// the class is this trait plus the module functions.
pub trait BoolDocValues: FunctionValues {
    /// The protected `vs` field.
    fn vs(&self) -> &dyn ValueSource;
}

/// Equivalent to `BoolDocValues.byteVal(int)`:
/// `boolVal(doc) ? (byte) 1 : (byte) 0`.
///
/// # Errors
///
/// Propagates the error of `bool_val`.
pub fn byte_val<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i8> {
    Ok(if this.bool_val(doc)? { 1 } else { 0 })
}

/// Equivalent to `BoolDocValues.shortVal(int)`.
///
/// # Errors
///
/// Propagates the error of `bool_val`.
pub fn short_val<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i16> {
    Ok(if this.bool_val(doc)? { 1 } else { 0 })
}

/// Equivalent to `BoolDocValues.floatVal(int)`.
///
/// # Errors
///
/// Propagates the error of `bool_val`.
pub fn float_val<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f32> {
    Ok(if this.bool_val(doc)? { 1.0 } else { 0.0 })
}

/// Equivalent to `BoolDocValues.intVal(int)`.
///
/// # Errors
///
/// Propagates the error of `bool_val`.
pub fn int_val<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i32> {
    Ok(if this.bool_val(doc)? { 1 } else { 0 })
}

/// Equivalent to `BoolDocValues.longVal(int)`.
///
/// # Errors
///
/// Propagates the error of `bool_val`.
pub fn long_val<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i64> {
    Ok(if this.bool_val(doc)? { 1 } else { 0 })
}

/// Equivalent to `BoolDocValues.doubleVal(int)`.
///
/// # Errors
///
/// Propagates the error of `bool_val`.
pub fn double_val<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<f64> {
    Ok(if this.bool_val(doc)? { 1.0 } else { 0.0 })
}

/// Equivalent to `BoolDocValues.strVal(int)`: `Boolean.toString(boolVal(doc))`.
///
/// # Errors
///
/// Propagates the error of `bool_val`.
pub fn str_val<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<Option<String>> {
    Ok(Some(this.bool_val(doc)?.to_string()))
}

/// Equivalent to `BoolDocValues.objectVal(int)`:
/// `exists(doc) ? boolVal(doc) : null`.
///
/// # Errors
///
/// Propagates the errors of `exists` and `bool_val`.
pub fn object_val<T: BoolDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
) -> Result<Option<MutableValueObject>> {
    Ok(if this.exists(doc)? {
        Some(MutableValueObject::Bool(this.bool_val(doc)?))
    } else {
        None
    })
}

/// Equivalent to `BoolDocValues.toString(int)`:
/// `vs.description() + '=' + strVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `str_val`.
pub fn to_doc_string<T: BoolDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}={}",
        this.vs().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `BoolDocValues.getValueFiller()`: a `MutableValueBool`
/// filled with `boolVal(doc)` and `exists(doc)`.
pub fn get_value_filler<T: BoolDocValues + ?Sized>(_this: &T) -> Box<dyn ValueFiller> {
    Box::new(BoolValuesFiller {
        mval: MutableValueBool::new(),
    })
}

/// The anonymous `ValueFiller` of `BoolDocValues.getValueFiller`.
struct BoolValuesFiller {
    mval: MutableValueBool,
}

impl ValueFiller for BoolValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        self.mval.value = values.bool_val(doc)?;
        self.mval.exists = values.exists(doc)?;
        Ok(())
    }
}

/// Delegates, inside an `impl FunctionValues` block, each listed member to
/// the `BoolDocValues` member of the same name.
macro_rules! bool_doc_values_members {
    ($($member:ident),* $(,)?) => {
        $( $crate::function::docvalues::bool_doc_values::bool_doc_values_members!(@ $member); )*
    };
    (@ byte_val) => {
        fn byte_val(&mut self, doc: i32) -> ::rucene::error::Result<i8> {
            $crate::function::docvalues::bool_doc_values::byte_val(self, doc)
        }
    };
    (@ short_val) => {
        fn short_val(&mut self, doc: i32) -> ::rucene::error::Result<i16> {
            $crate::function::docvalues::bool_doc_values::short_val(self, doc)
        }
    };
    (@ float_val) => {
        fn float_val(&mut self, doc: i32) -> ::rucene::error::Result<f32> {
            $crate::function::docvalues::bool_doc_values::float_val(self, doc)
        }
    };
    (@ int_val) => {
        fn int_val(&mut self, doc: i32) -> ::rucene::error::Result<i32> {
            $crate::function::docvalues::bool_doc_values::int_val(self, doc)
        }
    };
    (@ long_val) => {
        fn long_val(&mut self, doc: i32) -> ::rucene::error::Result<i64> {
            $crate::function::docvalues::bool_doc_values::long_val(self, doc)
        }
    };
    (@ double_val) => {
        fn double_val(&mut self, doc: i32) -> ::rucene::error::Result<f64> {
            $crate::function::docvalues::bool_doc_values::double_val(self, doc)
        }
    };
    (@ str_val) => {
        fn str_val(&mut self, doc: i32) -> ::rucene::error::Result<Option<String>> {
            $crate::function::docvalues::bool_doc_values::str_val(self, doc)
        }
    };
    (@ object_val) => {
        fn object_val(
            &mut self,
            doc: i32,
        ) -> ::rucene::error::Result<Option<::rucene::util::mutable::MutableValueObject>> {
            $crate::function::docvalues::bool_doc_values::object_val(self, doc)
        }
    };
    (@ to_doc_string) => {
        fn to_doc_string(&mut self, doc: i32) -> ::rucene::error::Result<String> {
            $crate::function::docvalues::bool_doc_values::to_doc_string(self, doc)
        }
    };
    (@ get_value_filler) => {
        fn get_value_filler(&self) -> Box<dyn $crate::function::function_values::ValueFiller> {
            $crate::function::docvalues::bool_doc_values::get_value_filler(self)
        }
    };
}
pub(crate) use bool_doc_values_members;
