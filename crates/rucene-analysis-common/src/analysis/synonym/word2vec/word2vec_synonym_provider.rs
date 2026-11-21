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

//! Generates the list of synonyms of a term, ported from
//! `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymProvider`.

use rucene::codecs::hnsw::{DefaultFlatVectorScorer, FlatVectorsScorer};
use rucene::error::Result;
use rucene::index::VectorSimilarityFunction;
use rucene::search::KnnCollector;
use rucene::util::hnsw::builder::{DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN};
use rucene::util::hnsw::{HnswBuilder, HnswGraphBuilder, HnswGraphSearcher, OnHeapHnswGraph};
use rucene::util::BytesRef;

use super::term_and_boost::TermAndBoost;
use super::word2vec_model::Word2VecModel;

/// `SIMILARITY_FUNCTION`.
const SIMILARITY_FUNCTION: VectorSimilarityFunction = VectorSimilarityFunction::DOT_PRODUCT;

/// The Word2VecSynonymProvider generates the list of sysnonyms of a term.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymProvider`
/// (`@lucene.experimental`).
#[derive(Debug)]
pub struct Word2VecSynonymProvider {
    word2_vec_model: Word2VecModel,
    hnsw_graph: OnHeapHnswGraph,
    default_flat_vector_scorer: DefaultFlatVectorScorer,
}

impl Word2VecSynonymProvider {
    /// Word2VecSynonymProvider constructor: `model` containing the set of
    /// TermAndVector entries.
    ///
    /// Equivalent to `Word2VecSynonymProvider(Word2VecModel)`: an HNSW graph
    /// over the model's vectors, built with `HnswGraphBuilder.create(scorerSupplier,
    /// DEFAULT_MAX_CONN, DEFAULT_BEAM_WIDTH, HnswGraphBuilder.randSeed,
    /// model.size())` and the dot-product similarity. The scorer supplier
    /// reads a copy of the model, which shares its vectors (Java hands it
    /// the model itself).
    ///
    /// # Errors
    ///
    /// What building the graph raises (a vector the model did not load
    /// fails as Java's `NullPointerException`).
    pub fn new(model: Word2VecModel) -> Result<Self> {
        let default_flat_vector_scorer = DefaultFlatVectorScorer;
        let scorer_supplier = default_flat_vector_scorer.get_random_vector_scorer_supplier_float(
            SIMILARITY_FUNCTION,
            Box::new(model.copy_model()),
        )?;
        let mut builder = HnswGraphBuilder::create(
            scorer_supplier.as_ref(),
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            HnswGraphBuilder::rand_seed(),
            rucene::index::KnnVectorValues::size(&model),
        )?;
        builder.build(rucene::index::KnnVectorValues::size(&model))?;
        let hnsw_graph = builder.into_graph()?;
        Ok(Self {
            word2_vec_model: model,
            hnsw_graph,
            default_flat_vector_scorer,
        })
    }

    /// The synonyms of `term`: the terms of its `max_synonyms_per_term`
    /// nearest vectors whose similarity reaches `min_accepted_similarity`,
    /// the term itself excluded, best first.
    ///
    /// Equivalent to `getSynonyms(BytesRef, int, float)`; Java's
    /// `IllegalArgumentException("Term must not be null")` has no
    /// counterpart, as a reference cannot be null.
    ///
    /// # Errors
    ///
    /// What scoring and searching the graph raise.
    pub fn get_synonyms(
        &self,
        term: &BytesRef,
        max_synonyms_per_term: i32,
        min_accepted_similarity: f32,
    ) -> Result<Vec<TermAndBoost>> {
        let mut result = Vec::new();
        let query = self.word2_vec_model.vector_value_of(term);
        if let Some(query) = query {
            let mut scorer = self
                .default_flat_vector_scorer
                .get_random_vector_scorer_float(
                    SIMILARITY_FUNCTION,
                    Box::new(self.word2_vec_model.copy_model()),
                    &query,
                )?;
            let mut synonyms = HnswGraphSearcher::search_on_heap(
                scorer.as_mut(),
                // The query vector is in the model. When looking for the top-k
                // it's always the nearest neighbour of itself so, we look for the top-k+1
                max_synonyms_per_term + 1,
                &self.hnsw_graph,
                None,
                i64::from(i32::MAX),
            )?;
            let top_docs = synonyms.top_docs()?;

            for score_doc in &top_docs.score_docs {
                let similarity = score_doc.score;
                let id = score_doc.doc;

                let synonym = self.word2_vec_model.term_value(id)?;
                // We remove the original query term
                if synonym != *term && similarity >= min_accepted_similarity {
                    result.push(TermAndBoost::new(&synonym, similarity));
                }
            }
        }
        Ok(result)
    }
}
