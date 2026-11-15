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

//! Port of `ByteVectorSimilarityFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{LeafReaderContext, VectorSimilarityFunction as IndexVectorSimilarityFunction};

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::vector_similarity_function::{self, VectorSimilarityFunction};

/// [`VectorSimilarityFunction`] implementation for byte vectors.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.ByteVectorSimilarityFunction`.
#[derive(Debug, Clone)]
pub struct ByteVectorSimilarityFunction {
    similarity_function: IndexVectorSimilarityFunction,
    vector1: Arc<dyn ValueSource>,
    vector2: Arc<dyn ValueSource>,
}

impl ByteVectorSimilarityFunction {
    /// Equivalent to `ByteVectorSimilarityFunction(VectorSimilarityFunction, ValueSource,
    /// ValueSource)`.
    pub fn new(
        similarity_function: IndexVectorSimilarityFunction,
        vector1: Arc<dyn ValueSource>,
        vector2: Arc<dyn ValueSource>,
    ) -> Self {
        Self {
            similarity_function,
            vector1,
            vector2,
        }
    }
}

impl VectorSimilarityFunction for ByteVectorSimilarityFunction {
    fn similarity_function(&self) -> IndexVectorSimilarityFunction {
        self.similarity_function
    }

    fn vector1(&self) -> &Arc<dyn ValueSource> {
        &self.vector1
    }

    fn vector2(&self) -> &Arc<dyn ValueSource> {
        &self.vector2
    }

    /// `0` when either vector is `null`, the similarity otherwise.
    fn func(
        &self,
        doc: i32,
        f1: &mut dyn FunctionValues,
        f2: &mut dyn FunctionValues,
    ) -> Result<f32> {
        let v1 = f1.byte_vector_val(doc)?;
        let v2 = f2.byte_vector_val(doc)?;
        let (Some(v1), Some(v2)) = (v1, v2) else {
            return Ok(0.0);
        };
        debug_assert!(v1.len() == v2.len(), "Vectors must have the same length");
        self.similarity_function.compare_bytes(&v1, &v2)
    }
}

impl ValueSource for ByteVectorSimilarityFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        vector_similarity_function::get_values(self, context, reader_context)
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        vector_similarity_function::value_source_eq(self, other)
    }

    fn value_source_hash(&self) -> i32 {
        vector_similarity_function::value_source_hash(self)
    }

    fn description(&self) -> String {
        vector_similarity_function::description(self)
    }
}
