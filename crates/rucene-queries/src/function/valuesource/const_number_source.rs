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

//! Port of `ConstNumberSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use rucene::search::similarities::ExplanationValue;

use crate::function::value_source::ValueSource;

/// A [`ValueSource`] that returns a constant number.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.ConstNumberSource`.
///
/// **Divergence.** `getNumber()` returns a `java.lang.Number`; the port
/// returns the boxed type as an [`ExplanationValue`], the core's stand-in for
/// a Java `Number`, whose rendering is the boxed type's `toString()`.
pub trait ConstNumberSource: ValueSource {
    /// Equivalent to the abstract `ConstNumberSource.getInt()`.
    fn get_int(&self) -> i32;

    /// Equivalent to the abstract `ConstNumberSource.getLong()`.
    fn get_long(&self) -> i64;

    /// Equivalent to the abstract `ConstNumberSource.getFloat()`.
    fn get_float(&self) -> f32;

    /// Equivalent to the abstract `ConstNumberSource.getDouble()`.
    fn get_double(&self) -> f64;

    /// Equivalent to the abstract `ConstNumberSource.getNumber()`.
    fn get_number(&self) -> ExplanationValue;

    /// Equivalent to the abstract `ConstNumberSource.getBool()`.
    fn get_bool(&self) -> bool;
}
