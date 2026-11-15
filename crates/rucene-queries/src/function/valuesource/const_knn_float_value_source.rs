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

//! Port of `ConstKnnFloatValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::util::java_collections::java_arrays_hash_code_floats;
use rucene::util::java_fmt::float_to_string;
use rucene::util::vector_util;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{
    class_hash, float_to_int_bits, objects_hash, ValueSource, ValueSourceContext,
};

/// `Arrays.toString(float[])`: `[e1, e2, ...]`, each element rendered with
/// `Float.toString`.
pub(crate) fn floats_to_string(vector: &[f32]) -> String {
    let parts: Vec<String> = vector.iter().map(|&v| float_to_string(v)).collect();
    format!("[{}]", parts.join(", "))
}

/// Function that returns a constant float vector value for every document.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.ConstKnnFloatValueSource`.
#[derive(Debug, Clone)]
pub struct ConstKnnFloatValueSource {
    vector: Vec<f32>,
}

impl ConstKnnFloatValueSource {
    /// Equivalent to `ConstKnnFloatValueSource(float[])`, which requires a
    /// finite vector (`VectorUtil.checkFinite`).
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of a non-finite value.
    pub fn new(const_vector: Vec<f32>) -> Result<Self> {
        vector_util::check_finite(&const_vector)?;
        Ok(Self {
            vector: const_vector,
        })
    }
}

/// The anonymous `FunctionValues` of `ConstKnnFloatValueSource.getValues`.
struct ConstKnnFloatValues {
    this: Arc<ConstKnnFloatValueSource>,
}

impl FunctionValues for ConstKnnFloatValues {
    fn float_vector_val(&mut self, _doc: i32) -> Result<Option<Vec<f32>>> {
        Ok(Some(self.this.vector.clone()))
    }

    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Ok(Some(floats_to_string(&self.this.vector)))
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

impl ValueSource for ConstKnnFloatValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        Ok(Box::new(ConstKnnFloatValues { this: self }))
    }

    /// The same class and `Arrays.equals(vector, other.vector)`, which
    /// compares the elements' `Float.floatToIntBits`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<ConstKnnFloatValueSource>()
            .is_some_and(|other| {
                self.vector.len() == other.vector.len()
                    && self
                        .vector
                        .iter()
                        .zip(&other.vector)
                        .all(|(&a, &b)| float_to_int_bits(a) == float_to_int_bits(b))
            })
    }

    /// `Objects.hash(getClass().hashCode(), Arrays.hashCode(vector))`.
    fn value_source_hash(&self) -> i32 {
        objects_hash(&[
            class_hash("org.apache.lucene.queries.function.valuesource.ConstKnnFloatValueSource"),
            java_arrays_hash_code_floats(&self.vector),
        ])
    }

    fn description(&self) -> String {
        format!(
            "ConstKnnFloatValueSource({})",
            floats_to_string(&self.vector)
        )
    }
}
