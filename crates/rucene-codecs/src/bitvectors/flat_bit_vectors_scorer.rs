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

//! Port of `org.apache.lucene.codecs.bitvectors.FlatBitVectorsScorer`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use rucene::codecs::{ByteVectorValues, FloatVectorValues};
use rucene::error::{LuceneError, Result};
use rucene::index::VectorSimilarityFunction;
use rucene::util::hnsw::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use rucene::util::vector_util::xor_bit_count;
use rucene::util::Bits;

/// A bit vector scorer for scoring byte vectors.
///
/// Port of `org.apache.lucene.codecs.bitvectors.FlatBitVectorsScorer`.
///
/// Java's methods take any `KnnVectorValues` and accept only
/// `ByteVectorValues` (`instanceof`), throwing `IllegalArgumentException`
/// otherwise; the port's [`FlatVectorsScorer`] has one method per vector
/// kind, so the float methods are the rejected branch.
#[derive(Debug, Default, Clone, Copy)]
pub struct FlatBitVectorsScorer;

impl FlatBitVectorsScorer {
    /// Port of `FlatBitVectorsScorer()`.
    pub fn new() -> Self {
        Self
    }
}

impl FlatVectorsScorer for FlatBitVectorsScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        _similarity_function: VectorSimilarityFunction,
        _vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Err(LuceneError::IllegalArgument(
            "vectorValues must be an instance of ByteVectorValues".to_string(),
        ))
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        _similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(BitRandomVectorScorerSupplier::new(Arc::from(
            vector_values,
        ))?))
    }

    fn get_random_vector_scorer_float(
        &self,
        _similarity_function: VectorSimilarityFunction,
        _vector_values: Box<dyn FloatVectorValues>,
        _target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        Err(LuceneError::IllegalArgument(
            "bit vectors do not support float[] targets".to_string(),
        ))
    }

    fn get_random_vector_scorer_byte(
        &self,
        _similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        Ok(Box::new(BitRandomVectorScorer::new(
            Arc::from(vector_values),
            target.to_vec(),
        )))
    }
}

impl fmt::Display for FlatBitVectorsScorer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FlatBitVectorsScorer()")
    }
}

/// Port of `FlatBitVectorsScorer.BitRandomVectorScorer`: the share of bits
/// two vectors have in common, `(bits - hamming distance) / bits`.
struct BitRandomVectorScorer {
    vector_values: Arc<dyn ByteVectorValues>,
    bit_dimensions: i32,
    query: Vec<u8>,
}

impl BitRandomVectorScorer {
    fn new(vector_values: Arc<dyn ByteVectorValues>, query: Vec<u8>) -> Self {
        let bit_dimensions = vector_values.dimension() * 8;
        Self {
            vector_values,
            bit_dimensions,
            query,
        }
    }
}

impl RandomVectorScorer for BitRandomVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let distance = xor_bit_count(&self.query, &self.vector_values.vector_value(node)?)?;
        Ok((self.bit_dimensions - distance) as f32 / self.bit_dimensions as f32)
    }

    fn max_ord(&self) -> i32 {
        self.vector_values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.vector_values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.vector_values.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for BitRandomVectorScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        // `System.arraycopy(vectorValues.vectorValue(node), 0, query, 0, query.length)`
        let vector = self.vector_values.vector_value(node)?;
        let length = self.query.len();
        let source = vector.get(..length).ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!(
                    "arraycopy: last source index {length} out of bounds for byte[{}]",
                    vector.len()
                ),
            )
        })?;
        self.query.copy_from_slice(source);
        Ok(())
    }
}

/// Port of `FlatBitVectorsScorer.BitRandomVectorScorerSupplier`.
struct BitRandomVectorScorerSupplier {
    vector_values: Arc<dyn ByteVectorValues>,
    /// Java's `targetVectors`, a copy of the values that no scorer reads.
    #[allow(dead_code)]
    target_vectors: Box<dyn ByteVectorValues>,
}

impl BitRandomVectorScorerSupplier {
    fn new(vector_values: Arc<dyn ByteVectorValues>) -> Result<Self> {
        let target_vectors = vector_values.copy_byte()?;
        Ok(Self {
            vector_values,
            target_vectors,
        })
    }
}

impl RandomVectorScorerSupplier for BitRandomVectorScorerSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        let query = vec![0u8; self.vector_values.dimension() as usize];
        Ok(Box::new(BitRandomVectorScorer::new(
            Arc::clone(&self.vector_values),
            query,
        )))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(BitRandomVectorScorerSupplier::new(Arc::clone(
            &self.vector_values,
        ))?))
    }
}
