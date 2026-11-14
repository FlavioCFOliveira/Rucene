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

//! The parsed Word2Vec model, ported from
//! `org.apache.lucene.analysis.synonym.word2vec.Word2VecModel`.

use std::sync::atomic::AtomicI64;
use std::sync::{Arc, OnceLock, RwLock};

use rucene::error::{LuceneError, Result};
use rucene::index::{FloatVectorValues, KnnVectorValues, VectorEncoding};
use rucene::util::byte_block_pool::{ByteBlockPool, DirectAllocator};
use rucene::util::misc::TermAndVector;
use rucene::util::{BytesRef, BytesRefHash};

/// Java's `BytesRefHash`, which owns its `ByteBlockPool`: `rucene`'s
/// [`BytesRefHash`] keeps the pool apart.
#[derive(Debug)]
struct TermHash {
    hash: BytesRefHash,
    pool: ByteBlockPool,
}

/// `new ArrayIndexOutOfBoundsException("Index i out of bounds for length l")`.
pub(crate) fn array_index_out_of_bounds(index: usize, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// Word2VecModel is a class representing the parsed Word2Vec model
/// containing the vectors for each word in dictionary
///
/// Equivalent to `org.apache.lucene.analysis.synonym.word2vec.Word2VecModel`
/// (`@lucene.experimental`), which extends `FloatVectorValues`. Java's
/// `copy()` shares the `TermAndVector[]` and the `BytesRefHash` with the
/// original; here both sit behind an `Arc`, the array as slots each written
/// once (`termsAndVectors[loadedCount++] = ...`) and the hash behind a lock.
#[derive(Debug)]
pub struct Word2VecModel {
    dictionary_size: i32,
    vector_dimension: i32,
    terms_and_vectors: Arc<[OnceLock<TermAndVector>]>,
    word2_vec: Arc<RwLock<TermHash>>,
    loaded_count: usize,
}

impl Word2VecModel {
    /// Equivalent to `Word2VecModel(int, int)`.
    ///
    /// # Errors
    ///
    /// `NegativeArraySizeException` for a negative `dictionary_size`, as
    /// Java's `new TermAndVector[dictionarySize]` throws.
    pub fn new(dictionary_size: i32, vector_dimension: i32) -> Result<Self> {
        let Ok(length) = usize::try_from(dictionary_size) else {
            return Err(LuceneError::unchecked(
                "java.lang.NegativeArraySizeException",
                dictionary_size.to_string(),
            ));
        };
        Ok(Self {
            dictionary_size,
            vector_dimension,
            terms_and_vectors: (0..length).map(|_| OnceLock::new()).collect(),
            word2_vec: Arc::new(RwLock::new(TermHash {
                hash: BytesRefHash::new(Arc::new(AtomicI64::new(0))),
                pool: ByteBlockPool::with_allocator(Box::new(DirectAllocator::new())),
            })),
            loaded_count: 0,
        })
    }

    /// Equivalent to the private `Word2VecModel(int, int, TermAndVector[],
    /// BytesRefHash)` of `copy()`: the arrays are shared and `loadedCount`
    /// starts at 0.
    fn shared_copy(&self) -> Self {
        Self {
            dictionary_size: self.dictionary_size,
            vector_dimension: self.vector_dimension,
            terms_and_vectors: Arc::clone(&self.terms_and_vectors),
            word2_vec: Arc::clone(&self.word2_vec),
            loaded_count: 0,
        }
    }

    /// Equivalent to `addTermAndVector(TermAndVector)`: the entry is stored
    /// with its vector normalized.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a zero vector, which cannot be
    /// normalized, `ArrayIndexOutOfBoundsException` past the dictionary
    /// size, and what adding the term to the hash raises.
    pub fn add_term_and_vector(&mut self, model_entry: TermAndVector) -> Result<()> {
        let model_entry = model_entry.normalize_vector()?;
        let index = self.loaded_count;
        self.loaded_count += 1;
        let Some(slot) = self.terms_and_vectors.get(index) else {
            return Err(array_index_out_of_bounds(
                index,
                self.terms_and_vectors.len(),
            ));
        };
        let term = model_entry.term().clone();
        slot.set(model_entry)
            .expect("INVARIANT: each slot is written once, at loadedCount");
        let mut word2_vec = self
            .word2_vec
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let TermHash { hash, pool } = &mut *word2_vec;
        hash.add(pool, term.slice())?;
        Ok(())
    }

    /// The entry of `target_ord`: Java's `termsAndVectors[targetOrd]`,
    /// whose dereference fails for an ord past the array or not loaded yet.
    fn entry(&self, target_ord: i32) -> Result<&TermAndVector> {
        let index = usize::try_from(target_ord).unwrap_or(usize::MAX);
        match self.terms_and_vectors.get(index) {
            None => Err(array_index_out_of_bounds(
                index,
                self.terms_and_vectors.len(),
            )),
            Some(slot) => slot.get().ok_or_else(LuceneError::null_pointer),
        }
    }

    /// The vector of `term`, `None` when the model does not hold it.
    ///
    /// Equivalent to `vectorValue(BytesRef)`.
    pub fn vector_value_of(&self, term: &BytesRef) -> Option<Vec<f32>> {
        let term_ord = {
            let word2_vec = self
                .word2_vec
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            word2_vec.hash.find(&word2_vec.pool, term.slice())
        };
        if term_ord < 0 {
            return None;
        }
        self.terms_and_vectors[term_ord as usize]
            .get()
            .map(|entry| entry.vector().to_vec())
    }

    /// The term of `target_ord`.
    ///
    /// Equivalent to `termValue(int)`.
    ///
    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` past the dictionary size, and
    /// `NullPointerException` for an ord not loaded.
    pub fn term_value(&self, target_ord: i32) -> Result<BytesRef> {
        Ok(self.entry(target_ord)?.term().clone())
    }

    /// Equivalent to `copy()`.
    pub fn copy_model(&self) -> Word2VecModel {
        self.shared_copy()
    }
}

impl KnnVectorValues for Word2VecModel {
    /// Equivalent to `dimension()`.
    fn dimension(&self) -> i32 {
        self.vector_dimension
    }

    /// Equivalent to `size()`: the declared dictionary size.
    fn size(&self) -> i32 {
        self.dictionary_size
    }

    /// Equivalent to `copy()`.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.shared_copy()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }
}

impl FloatVectorValues for Word2VecModel {
    /// Equivalent to `vectorValue(int)`.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        Ok(self.entry(ord)?.vector().to_vec())
    }

    /// Equivalent to `copy()`.
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.shared_copy()))
    }
}
