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

//! Port of `ConstKnnByteVectorValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::util::java_collections::java_arrays_hash_code_bytes;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, objects_hash, ValueSource, ValueSourceContext};

/// `Arrays.toString(byte[])`: `[e1, e2, ...]`, each element a signed Java
/// `byte`.
pub(crate) fn bytes_to_string(vector: &[u8]) -> String {
    let parts: Vec<String> = vector.iter().map(|&v| (v as i8).to_string()).collect();
    format!("[{}]", parts.join(", "))
}

/// Function that returns a constant byte vector value for every document.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.ConstKnnByteVectorValueSource`.
/// The Java `byte[]` is a vector of raw bytes here, as the core's byte
/// vectors are.
#[derive(Debug, Clone)]
pub struct ConstKnnByteVectorValueSource {
    vector: Vec<u8>,
}

impl ConstKnnByteVectorValueSource {
    /// Equivalent to `ConstKnnByteVectorValueSource(byte[])`; Java's `null`
    /// check is the type itself.
    pub fn new(const_vector: Vec<u8>) -> Self {
        Self {
            vector: const_vector,
        }
    }
}

/// The anonymous `FunctionValues` of `ConstKnnByteVectorValueSource.getValues`.
struct ConstKnnByteValues {
    this: Arc<ConstKnnByteVectorValueSource>,
}

impl FunctionValues for ConstKnnByteValues {
    fn byte_vector_val(&mut self, _doc: i32) -> Result<Option<Vec<u8>>> {
        Ok(Some(self.this.vector.clone()))
    }

    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Ok(Some(bytes_to_string(&self.this.vector)))
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let value = self.str_val(doc)?;
        Ok(format!(
            "{}={}",
            self.this.description(),
            value.as_deref().unwrap_or("null")
        ))
    }
}

impl ValueSource for ConstKnnByteVectorValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        Ok(Box::new(ConstKnnByteValues { this: self }))
    }

    /// The same class and `Arrays.equals(vector, other.vector)`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<ConstKnnByteVectorValueSource>()
            .is_some_and(|other| self.vector == other.vector)
    }

    /// `Objects.hash(getClass().hashCode(), Arrays.hashCode(vector))`.
    fn value_source_hash(&self) -> i32 {
        objects_hash(&[
            class_hash(
                "org.apache.lucene.queries.function.valuesource.ConstKnnByteVectorValueSource",
            ),
            java_arrays_hash_code_bytes(&self.vector),
        ])
    }

    fn description(&self) -> String {
        format!(
            "ConstKnnByteVectorValueSource({})",
            bytes_to_string(&self.vector)
        )
    }
}
