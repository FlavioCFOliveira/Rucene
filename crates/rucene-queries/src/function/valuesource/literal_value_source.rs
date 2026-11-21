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

//! Port of `LiteralValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::{BytesRef, BytesRefBuilder};

use crate::function::docvalues::str_doc_values::{str_doc_values_members, StrDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};

/// Pass a string to get back the same string. Useful for passing a field
/// name to a function, for instance.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.LiteralValueSource`.
#[derive(Debug, Clone)]
pub struct LiteralValueSource {
    /// The protected `string` field.
    string: String,
    /// The protected `bytesRef` field, `new BytesRef(string)`.
    bytes_ref: BytesRef,
}

impl LiteralValueSource {
    /// Equivalent to `LiteralValueSource(String)`.
    pub fn new(string: impl Into<String>) -> Self {
        let string = string.into();
        let bytes_ref = BytesRef::from_char_sequence(&string);
        Self { string, bytes_ref }
    }

    /// Returns the literal.
    ///
    /// Equivalent to `LiteralValueSource.getValue()`.
    pub fn get_value(&self) -> &str {
        &self.string
    }

    /// The public static `LiteralValueSource.hash`:
    /// `LiteralValueSource.class.hashCode()`, an identity hash (see
    /// `class_hash`).
    pub fn hash() -> i32 {
        class_hash("org.apache.lucene.queries.function.valuesource.LiteralValueSource")
    }
}

/// The anonymous `StrDocValues` of `LiteralValueSource.getValues`.
struct LiteralValues {
    this: Arc<LiteralValueSource>,
}

impl FunctionValues for LiteralValues {
    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Ok(Some(self.this.string.clone()))
    }

    fn bytes_val(&mut self, _doc: i32, target: &mut BytesRefBuilder) -> Result<bool> {
        let bytes = &self.this.bytes_ref;
        target.copy_bytes(bytes.slice(), 0, bytes.length);
        Ok(true)
    }

    fn to_doc_string(&mut self, _doc: i32) -> Result<String> {
        Ok(self.this.string.clone())
    }

    str_doc_values_members!(object_val, bool_val, get_value_filler);
}

impl StrDocValues for LiteralValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for LiteralValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        Ok(Box::new(LiteralValues { this: self }))
    }

    fn description(&self) -> String {
        format!("literal({})", self.string)
    }

    /// `o instanceof LiteralValueSource` with an equal string.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<LiteralValueSource>()
            .is_some_and(|that| self.string == that.string)
    }

    /// `hash + string.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        Self::hash().wrapping_add(java_string_hash_code(&self.string))
    }
}
