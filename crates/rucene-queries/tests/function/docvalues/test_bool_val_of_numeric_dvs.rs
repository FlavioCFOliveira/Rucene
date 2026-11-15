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

//! Port of `TestBoolValOfNumericDVs.java`: sanity check that
//! `FunctionValues#boolVal` behaves as expected for trivial subclasses of
//! the various (Numeric) DocValue implementations.
//!
//! Any "non-zero" value should result in "true".
//!
//! Each anonymous subclass overrides one getter and inherits every other
//! member from its superclass, whose members the port exposes as the module
//! functions of `float_doc_values`, `double_doc_values`, `int_doc_values`
//! and `long_doc_values`: the subclass delegates to them (the crate's
//! `*_doc_values_members!` macros, private to the crate, do the same).
//! `fv.getClass().getSuperclass().toString()` is `"class " + getName()` of
//! the superclass. The superclasses are built with a `null` value source,
//! which only their `toString` dereferences: `vs()` throws the
//! `NullPointerException` that dereference would.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::mutable::MutableValueObject;
use rucene_queries::function::docvalues::double_doc_values::{self, DoubleDocValues};
use rucene_queries::function::docvalues::float_doc_values::{self, FloatDocValues};
use rucene_queries::function::docvalues::int_doc_values::{self, IntDocValues};
use rucene_queries::function::docvalues::long_doc_values::{self, LongDocValues};
use rucene_queries::function::function_values::{FunctionValues, ValueFiller};
use rucene_queries::function::value_source::ValueSource;
use rucene_queries::function::value_source_scorer::ValueSourceScorerMatcher;

use crate::queries_support::core_support::{class, test};

/// `TestBoolValOfNumericDVs`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.docvalues.TestBoolValOfNumericDVs",
        vec![test("test", test_)],
    )
}

fn test_() -> Result<()> {
    check(true)?;
    check(false)
}

/// The `null` value source the superclasses are built with.
fn null_vs() -> &'static dyn ValueSource {
    randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
}

/// The anonymous `new FloatDocValues(null) { floatVal }`.
struct ConstFloat(bool);

impl FloatDocValues for ConstFloat {
    fn vs(&self) -> &dyn ValueSource {
        null_vs()
    }
}

impl FunctionValues for ConstFloat {
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        // `Float.MIN_VALUE`, the smallest positive subnormal
        Ok(if self.0 { f32::from_bits(1) } else { 0.0 })
    }
    fn byte_val(&mut self, doc: i32) -> Result<i8> {
        float_doc_values::byte_val(self, doc)
    }
    fn short_val(&mut self, doc: i32) -> Result<i16> {
        float_doc_values::short_val(self, doc)
    }
    fn int_val(&mut self, doc: i32) -> Result<i32> {
        float_doc_values::int_val(self, doc)
    }
    fn long_val(&mut self, doc: i32) -> Result<i64> {
        float_doc_values::long_val(self, doc)
    }
    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        float_doc_values::bool_val(self, doc)
    }
    fn double_val(&mut self, doc: i32) -> Result<f64> {
        float_doc_values::double_val(self, doc)
    }
    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        float_doc_values::str_val(self, doc)
    }
    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        float_doc_values::object_val(self, doc)
    }
    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        float_doc_values::to_doc_string(self, doc)
    }
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        float_doc_values::get_value_filler(self)
    }
}

/// The anonymous `new DoubleDocValues(null) { doubleVal }`.
struct ConstDouble(bool);

impl DoubleDocValues for ConstDouble {
    fn vs(&self) -> &dyn ValueSource {
        null_vs()
    }
}

impl FunctionValues for ConstDouble {
    fn double_val(&mut self, _doc: i32) -> Result<f64> {
        Ok(if self.0 { f64::from_bits(1) } else { 0.0 })
    }
    fn byte_val(&mut self, doc: i32) -> Result<i8> {
        double_doc_values::byte_val(self, doc)
    }
    fn short_val(&mut self, doc: i32) -> Result<i16> {
        double_doc_values::short_val(self, doc)
    }
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        double_doc_values::float_val(self, doc)
    }
    fn int_val(&mut self, doc: i32) -> Result<i32> {
        double_doc_values::int_val(self, doc)
    }
    fn long_val(&mut self, doc: i32) -> Result<i64> {
        double_doc_values::long_val(self, doc)
    }
    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        double_doc_values::bool_val(self, doc)
    }
    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        double_doc_values::str_val(self, doc)
    }
    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        double_doc_values::object_val(self, doc)
    }
    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        double_doc_values::to_doc_string(self, doc)
    }
    fn get_range_matcher(
        &mut self,
        lower_val: Option<&str>,
        upper_val: Option<&str>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Box<dyn ValueSourceScorerMatcher>> {
        double_doc_values::get_range_matcher(
            self,
            lower_val,
            upper_val,
            include_lower,
            include_upper,
        )
    }
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        double_doc_values::get_value_filler(self)
    }
}

/// The anonymous `new IntDocValues(null) { intVal }`.
struct ConstInt(bool);

impl IntDocValues for ConstInt {
    fn vs(&self) -> &dyn ValueSource {
        null_vs()
    }
}

impl FunctionValues for ConstInt {
    fn int_val(&mut self, _doc: i32) -> Result<i32> {
        Ok(if self.0 { 1 } else { 0 })
    }
    fn byte_val(&mut self, doc: i32) -> Result<i8> {
        int_doc_values::byte_val(self, doc)
    }
    fn short_val(&mut self, doc: i32) -> Result<i16> {
        int_doc_values::short_val(self, doc)
    }
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        int_doc_values::float_val(self, doc)
    }
    fn long_val(&mut self, doc: i32) -> Result<i64> {
        int_doc_values::long_val(self, doc)
    }
    fn double_val(&mut self, doc: i32) -> Result<f64> {
        int_doc_values::double_val(self, doc)
    }
    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        int_doc_values::str_val(self, doc)
    }
    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        int_doc_values::object_val(self, doc)
    }
    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        int_doc_values::to_doc_string(self, doc)
    }
    fn get_range_matcher(
        &mut self,
        lower_val: Option<&str>,
        upper_val: Option<&str>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Box<dyn ValueSourceScorerMatcher>> {
        int_doc_values::get_range_matcher(self, lower_val, upper_val, include_lower, include_upper)
    }
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        int_doc_values::get_value_filler(self)
    }
}

/// The anonymous `new LongDocValues(null) { longVal }`.
struct ConstLong(bool);

impl LongDocValues for ConstLong {
    fn vs(&self) -> &dyn ValueSource {
        null_vs()
    }
}

impl FunctionValues for ConstLong {
    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Ok(if self.0 { 1 } else { 0 })
    }
    fn byte_val(&mut self, doc: i32) -> Result<i8> {
        long_doc_values::byte_val(self, doc)
    }
    fn short_val(&mut self, doc: i32) -> Result<i16> {
        long_doc_values::short_val(self, doc)
    }
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        long_doc_values::float_val(self, doc)
    }
    fn int_val(&mut self, doc: i32) -> Result<i32> {
        long_doc_values::int_val(self, doc)
    }
    fn double_val(&mut self, doc: i32) -> Result<f64> {
        long_doc_values::double_val(self, doc)
    }
    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        long_doc_values::bool_val(self, doc)
    }
    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        long_doc_values::str_val(self, doc)
    }
    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        long_doc_values::object_val(self, doc)
    }
    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        long_doc_values::to_doc_string(self, doc)
    }
    fn get_range_matcher(
        &mut self,
        lower_val: Option<&str>,
        upper_val: Option<&str>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Box<dyn ValueSourceScorerMatcher>> {
        long_doc_values::get_range_matcher(self, lower_val, upper_val, include_lower, include_upper)
    }
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        long_doc_values::get_value_filler(self)
    }
}

fn check(expected: bool) -> Result<()> {
    // create "constant" based instances of each superclass that should returned the expected value
    // based on
    // the constant used
    let mut values: Vec<(&str, Box<dyn FunctionValues>)> = vec![
        (
            "class org.apache.lucene.queries.function.docvalues.FloatDocValues",
            Box::new(ConstFloat(expected)),
        ),
        (
            "class org.apache.lucene.queries.function.docvalues.DoubleDocValues",
            Box::new(ConstDouble(expected)),
        ),
        (
            "class org.apache.lucene.queries.function.docvalues.IntDocValues",
            Box::new(ConstInt(expected)),
        ),
        (
            "class org.apache.lucene.queries.function.docvalues.LongDocValues",
            Box::new(ConstLong(expected)),
        ),
    ];
    for (superclass, fv) in values.iter_mut() {
        // docId is irrelevant since all of our FunctionValues return a constant value.
        Assert::assert_equals_msg(Some(superclass), expected, fv.bool_val(123)?);
    }
    Ok(())
}
