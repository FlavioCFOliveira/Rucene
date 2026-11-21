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

//! Port of `StrDocValues.java`, of the Java package
//! `org.apache.lucene.queries.function.docvalues`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`; a subclass delegates the members it inherits to
//! them with
//! `str_doc_values_members!`.

use rucene::error::Result;
use rucene::util::mutable::{MutableValue, MutableValueObject, MutableValueStr};

use crate::function::function_values::{FunctionValues, ValueFiller};
use crate::function::value_source::ValueSource;

/// Abstract [`FunctionValues`] implementation which supports retrieving
/// string values. Implementations can control how the string values are
/// loaded through [`FunctionValues::str_val`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.docvalues.StrDocValues`, whose
/// `strVal(int)` is abstract.
///
/// **Divergence.** As for
/// [`FloatDocValues`](crate::function::docvalues::float_doc_values::FloatDocValues):
/// the class is this trait plus the module functions.
pub trait StrDocValues: FunctionValues {
    /// The protected `vs` field.
    fn vs(&self) -> &dyn ValueSource;
}

/// Equivalent to `StrDocValues.objectVal(int)`:
/// `exists(doc) ? strVal(doc) : null`.
///
/// # Errors
///
/// Propagates the errors of `exists` and `str_val`.
pub fn object_val<T: StrDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
) -> Result<Option<MutableValueObject>> {
    Ok(if this.exists(doc)? {
        this.str_val(doc)?.map(MutableValueObject::Str)
    } else {
        None
    })
}

/// Equivalent to `StrDocValues.boolVal(int)`: `exists(doc)`.
///
/// # Errors
///
/// Propagates the error of `exists`.
pub fn bool_val<T: StrDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<bool> {
    this.exists(doc)
}

/// Equivalent to `StrDocValues.toString(int)`:
/// `vs.description() + "='" + strVal(doc) + "'"`.
///
/// # Errors
///
/// Propagates the error of `str_val`.
pub fn to_doc_string<T: StrDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}='{}'",
        this.vs().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `StrDocValues.getValueFiller()`: a `MutableValueStr` whose
/// `exists` is `bytesVal(doc, mval.value)`.
pub fn get_value_filler<T: StrDocValues + ?Sized>(_this: &T) -> Box<dyn ValueFiller> {
    Box::new(StrValuesFiller {
        mval: MutableValueStr::new(),
    })
}

/// The anonymous `ValueFiller` of `StrDocValues.getValueFiller`.
struct StrValuesFiller {
    mval: MutableValueStr,
}

impl ValueFiller for StrValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        self.mval.exists = values.bytes_val(doc, &mut self.mval.value)?;
        Ok(())
    }
}

/// Delegates, inside an `impl FunctionValues` block, each listed member to
/// the `StrDocValues` member of the same name.
macro_rules! str_doc_values_members {
    ($($member:ident),* $(,)?) => {
        $( $crate::function::docvalues::str_doc_values::str_doc_values_members!(@ $member); )*
    };
    (@ object_val) => {
        fn object_val(
            &mut self,
            doc: i32,
        ) -> ::rucene::error::Result<Option<::rucene::util::mutable::MutableValueObject>> {
            $crate::function::docvalues::str_doc_values::object_val(self, doc)
        }
    };
    (@ bool_val) => {
        fn bool_val(&mut self, doc: i32) -> ::rucene::error::Result<bool> {
            $crate::function::docvalues::str_doc_values::bool_val(self, doc)
        }
    };
    (@ to_doc_string) => {
        fn to_doc_string(&mut self, doc: i32) -> ::rucene::error::Result<String> {
            $crate::function::docvalues::str_doc_values::to_doc_string(self, doc)
        }
    };
    (@ get_value_filler) => {
        fn get_value_filler(&self) -> Box<dyn $crate::function::function_values::ValueFiller> {
            $crate::function::docvalues::str_doc_values::get_value_filler(self)
        }
    };
}
pub(crate) use str_doc_values_members;
