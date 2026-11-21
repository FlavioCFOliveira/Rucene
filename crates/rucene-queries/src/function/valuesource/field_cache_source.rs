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

//! Port of `FieldCacheSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use rucene::util::java_collections::java_string_hash_code;

use crate::function::value_source::ValueSource;

/// A base class for [`ValueSource`] implementations that retrieve values for
/// a single field from doc values.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.FieldCacheSource`, whose
/// `field` is the protected field [`get_field`](Self::get_field) returns.
/// Every implementor also answers
/// [`ValueSource::as_field_cache_source`] with `self`.
pub trait FieldCacheSource: ValueSource {
    /// Equivalent to `FieldCacheSource.getField()`, the protected `field`.
    fn get_field(&self) -> &str;
}

/// Equivalent to `FieldCacheSource.description()`: the field.
pub fn description<T: FieldCacheSource + ?Sized>(this: &T) -> String {
    this.get_field().to_string()
}

/// Equivalent to `FieldCacheSource.equals(Object)`:
/// `o instanceof FieldCacheSource` with an equal field.
pub fn value_source_eq<T: FieldCacheSource + ?Sized>(this: &T, other: &dyn ValueSource) -> bool {
    other
        .as_field_cache_source()
        .is_some_and(|other| this.get_field() == other.get_field())
}

/// Equivalent to `FieldCacheSource.hashCode()`: `field.hashCode()`.
pub fn value_source_hash<T: FieldCacheSource + ?Sized>(this: &T) -> i32 {
    java_string_hash_code(this.get_field())
}
