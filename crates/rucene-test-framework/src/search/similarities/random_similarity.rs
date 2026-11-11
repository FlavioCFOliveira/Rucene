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

//! Port of `org.apache.lucene.tests.search.similarities.RandomSimilarity`.

use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::util::shuffle;
use rucene::error::Result;
use rucene::index::FieldInvertState;
use rucene::search::similarities::{
    per_field_compute_norm, per_field_scorer, AfterEffect, AfterEffectB, AfterEffectL,
    AxiomaticF1EXP, AxiomaticF1LOG, AxiomaticF2EXP, AxiomaticF2LOG, BM25Similarity, BasicModel,
    BasicModelG, BasicModelIF, BasicModelIn, BasicModelIne, BooleanSimilarity, ClassicSimilarity,
    CollectionStatistics, DFISimilarity, DFRSimilarity, Distribution, DistributionLL,
    DistributionSPL, IBSimilarity, Independence, IndependenceChiSquared, IndependenceSaturated,
    IndependenceStandardized, LMDirichletSimilarity, LMJelinekMercerSimilarity, Lambda, LambdaDF,
    LambdaTTF, Normalization, NormalizationH1, NormalizationH2, NormalizationH3, NormalizationZ,
    PerFieldSimilarityWrapper, SimScorer, Similarity, TermStatistics,
};
use rucene::util::java_collections::{java_string_hash_code, JavaHashMap};

/// One of the similarities [`RandomSimilarity`] rotates through, with its
/// Java `toString()`.
struct KnownSim {
    similarity: Arc<dyn Similarity>,
    to_string: String,
}

fn known<S: Similarity + fmt::Display + 'static>(similarity: S) -> KnownSim {
    let to_string = similarity.to_string();
    KnownSim {
        similarity: Arc::new(similarity),
        to_string,
    }
}

/// `BASIC_MODELS`.
fn basic_models() -> Vec<Box<dyn BasicModel>> {
    vec![
        Box::new(BasicModelG::new()),
        Box::new(BasicModelIF::new()),
        Box::new(BasicModelIn::new()),
        Box::new(BasicModelIne::new()),
    ]
}

/// `AFTER_EFFECTS`.
fn after_effects() -> Vec<Box<dyn AfterEffect>> {
    vec![Box::new(AfterEffectB::new()), Box::new(AfterEffectL::new())]
}

/// `NORMALIZATIONS`.
fn normalizations() -> Vec<Box<dyn Normalization>> {
    // TODO: if we enable NoNormalization, we have to deal with a couple tests
    // (e.g. TestDocBoost, TestSort) that expect length normalization
    vec![
        Box::new(NormalizationH1::default()),
        Box::new(NormalizationH2::default()),
        Box::new(NormalizationH3::default()),
        Box::new(NormalizationZ::default()),
    ]
}

/// `DISTRIBUTIONS`.
fn distributions() -> Vec<Box<dyn Distribution>> {
    vec![
        Box::new(DistributionLL::new()),
        Box::new(DistributionSPL::new()),
    ]
}

/// `LAMBDAS`.
fn lambdas() -> Vec<Box<dyn Lambda>> {
    vec![Box::new(LambdaDF::new()), Box::new(LambdaTTF::new())]
}

/// `INDEPENDENCE_MEASURES`.
fn independence_measures() -> Vec<Box<dyn Independence>> {
    vec![
        Box::new(IndependenceStandardized::new()),
        Box::new(IndependenceSaturated::new()),
        Box::new(IndependenceChiSquared::new()),
    ]
}

/// `allSims`: all the similarities that we rotate through, built once as
/// Java's static initialiser does. Java shares the component instances
/// (`BASIC_MODELS`, …) between the similarities; they are stateless, so each
/// similarity owns its own here.
static ALL_SIMS: LazyLock<Vec<KnownSim>> = LazyLock::new(|| {
    let mut all_sims = vec![
        known(ClassicSimilarity::new()),
        known(BM25Similarity::default()),
        known(AxiomaticF1EXP::new()),
        known(AxiomaticF1LOG::new()),
        known(AxiomaticF2EXP::new()),
        known(AxiomaticF2LOG::new()),
    ];
    // `BooleanSimilarity` does not override `toString()`: Java prints
    // `Object.toString()`, the class name and the identity hash, for which
    // this port uses the address (as `AttributeSource`'s rendering does).
    let boolean: Arc<dyn Similarity> = Arc::new(BooleanSimilarity::new());
    let address = Arc::as_ptr(&boolean) as *const () as usize;
    all_sims.push(KnownSim {
        to_string: format!(
            "org.apache.lucene.search.similarities.BooleanSimilarity@{:x}",
            // `Integer.toHexString(int)`: the unsigned hex digits.
            address as i32 as u32
        ),
        similarity: boolean,
    });
    for basic_model in 0..basic_models().len() {
        for after_effect in 0..after_effects().len() {
            for normalization in 0..normalizations().len() {
                all_sims.push(known(DFRSimilarity::new(
                    basic_models().swap_remove(basic_model),
                    after_effects().swap_remove(after_effect),
                    normalizations().swap_remove(normalization),
                )));
            }
        }
    }
    for distribution in 0..distributions().len() {
        for lambda in 0..lambdas().len() {
            for normalization in 0..normalizations().len() {
                all_sims.push(known(IBSimilarity::new(
                    distributions().swap_remove(distribution),
                    lambdas().swap_remove(lambda),
                    normalizations().swap_remove(normalization),
                )));
            }
        }
    }
    all_sims.push(known(LMDirichletSimilarity::new()));
    all_sims.push(known(
        LMJelinekMercerSimilarity::with_lambda(0.1).expect("INVARIANT: 0.1 is a legal lambda"),
    ));
    all_sims.push(known(
        LMJelinekMercerSimilarity::with_lambda(0.7).expect("INVARIANT: 0.7 is a legal lambda"),
    ));
    for independence in 0..independence_measures().len() {
        all_sims.push(known(DFISimilarity::new(
            independence_measures().swap_remove(independence),
        )));
    }
    all_sims
});

/// Similarity implementation that randomizes Similarity implementations per
/// field.
///
/// Equivalent to
/// `org.apache.lucene.tests.search.similarities.RandomSimilarity`, a
/// `PerFieldSimilarityWrapper`. The choices are per-field random but stable,
/// so that a field gets the same similarity for its whole life.
pub struct RandomSimilarity {
    /// `knownSims`: indices into [`ALL_SIMS`], shuffled.
    known_sims: Vec<usize>,
    /// `previousMappings`: field name to an index into [`ALL_SIMS`], behind
    /// the lock Java's `synchronized` methods take.
    previous_mappings: Mutex<JavaHashMap<usize>>,
    /// `perFieldSeed`.
    per_field_seed: i32,
    /// `shouldQueryNorm`.
    should_query_norm: bool,
}

impl fmt::Debug for RandomSimilarity {
    /// Starts with the type name: the core's explanations read a similarity's
    /// simple class name from its `Debug` rendering.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RandomSimilarity({self})")
    }
}

impl RandomSimilarity {
    /// `new RandomSimilarity(random)`.
    pub fn new(random: &dyn Random) -> RandomSimilarity {
        let per_field_seed = random.next_int();
        let should_query_norm = random.next_boolean();
        let mut known_sims: Vec<usize> = (0..ALL_SIMS.len()).collect();
        shuffle(&mut known_sims, random);
        RandomSimilarity {
            known_sims,
            previous_mappings: Mutex::new(JavaHashMap::new()),
            per_field_seed,
            should_query_norm,
        }
    }

    /// The index into [`ALL_SIMS`] of `get(field)`.
    fn get_index(&self, field: &str) -> usize {
        let mut previous_mappings = self
            .previous_mappings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *previous_mappings.compute_if_absent(field, || {
            // `Math.max(0, Math.abs(perFieldSeed ^ f.hashCode())) % size`:
            // `Math.abs(Integer.MIN_VALUE)` stays negative.
            let mixed = (self.per_field_seed ^ java_string_hash_code(field)).wrapping_abs();
            let index = mixed.max(0) as usize % self.known_sims.len();
            self.known_sims[index]
        })
    }

    /// The Java `toString()` of the similarity `get(field)` returns.
    pub fn get_to_string(&self, field: &str) -> &'static str {
        &ALL_SIMS[self.get_index(field)].to_string
    }
}

impl fmt::Display for RandomSimilarity {
    /// `"RandomSimilarity(queryNorm=" + shouldQueryNorm + "): " +
    /// previousMappings`, the map in Java's `HashMap` iteration order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let previous_mappings = self
            .previous_mappings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entries: Vec<String> = previous_mappings
            .iter()
            .map(|(field, &index)| format!("{field}={}", ALL_SIMS[index].to_string))
            .collect();
        write!(
            f,
            "RandomSimilarity(queryNorm={}): {{{}}}",
            self.should_query_norm,
            entries.join(", ")
        )
    }
}

impl PerFieldSimilarityWrapper for RandomSimilarity {
    fn get(&self, name: &str) -> &dyn Similarity {
        &*ALL_SIMS[self.get_index(name)].similarity
    }
}

impl Similarity for RandomSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.search.similarities.RandomSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        per_field_compute_norm(self, state)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        per_field_scorer(self, boost, collection_stats, term_stats)
    }
}
