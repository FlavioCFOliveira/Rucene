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

//! Port of `RandomSamplingFacetsCollector.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};
use rucene::index::{LeafReaderContext, Term};
use rucene::search::{
    CollectionResult, Collector, CollectorManager, IndexSearcher, LeafCollector, ScoreMode,
    NO_MORE_DOCS,
};
use rucene::util::doc_id_set::BitDocIdSet;
use rucene::util::{BitSet, FixedBitSet};

use crate::facet_result::FacetResult;
use crate::facets_collector::{FacetsCollector, MatchingDocs, SharedDocIdSet};
use crate::facets_collector_manager::FacetsCollectorManager;
use crate::facets_config::FacetsConfig;
use crate::label_and_value::{number_double_value, number_int_value, LabelAndValue};

const NOT_CALCULATED: i32 = -1;

/// Implements the `xorshift64*` random generator.
///
/// Equivalent to `org.apache.lucene.facet.RandomSamplingFacetsCollector.XORShift64Random`.
#[derive(Debug, Clone)]
pub(crate) struct XORShift64Random {
    x: i64,
}

impl XORShift64Random {
    /// Creates a xorshift random generator using the provided seed.
    ///
    /// Equivalent to `XORShift64Random(long)`.
    pub(crate) fn new(seed: i64) -> Self {
        Self {
            x: if seed == 0 { 0xdead_beef } else { seed },
        }
    }

    /// Get the next random long value.
    ///
    /// Equivalent to `randomLong()`.
    pub(crate) fn random_long(&mut self) -> i64 {
        self.x ^= self.x << 21;
        self.x ^= ((self.x as u64) >> 35) as i64;
        self.x ^= self.x << 4;
        self.x
    }

    /// Get the next random int, between 0 (inclusive) and `n` (exclusive).
    ///
    /// Equivalent to `nextInt(int)`.
    pub(crate) fn next_int(&mut self, n: i32) -> i32 {
        let res = self.random_long().wrapping_rem(n as i64) as i32;
        if res < 0 {
            res.wrapping_neg()
        } else {
            res
        }
    }
}

/// The mutable sampling state.
#[derive(Debug)]
struct SamplingMutable {
    random: XORShift64Random,
    sampling_rate: f64,
    total_hits: i32,
    leftover_bin: i32,
    leftover_index: i32,
}

/// The fields `RandomSamplingFacetsCollector` adds to `FacetsCollector`, with
/// its `getMatchingDocs()` override.
#[derive(Debug)]
pub struct SamplingState {
    sample_size: i32,
    state: Mutex<SamplingMutable>,
    sampled_docs: OnceLock<Vec<MatchingDocs>>,
}

impl SamplingState {
    fn new(sample_size: i32, seed: i64) -> Self {
        Self {
            sample_size,
            state: Mutex::new(SamplingMutable {
                random: XORShift64Random::new(seed),
                sampling_rate: 0.0,
                total_hits: NOT_CALCULATED,
                leftover_bin: NOT_CALCULATED,
                leftover_index: NOT_CALCULATED,
            }),
            sampled_docs: OnceLock::new(),
        }
    }

    /// Equivalent to the `getMatchingDocs()` override.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException` wrapping the `IOException` sampling a
    /// segment raises (`RandomSamplingFacetsCollector.java:210-212`).
    pub(crate) fn get_matching_docs<'a>(&'a self, matching_docs: &'a Vec<MatchingDocs>) -> Result<&'a Vec<MatchingDocs>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.total_hits == NOT_CALCULATED {
            state.total_hits = 0;
            for md in matching_docs {
                state.total_hits = state.total_hits.wrapping_add(md.total_hits);
            }
        }

        if state.total_hits <= self.sample_size {
            return Ok(matching_docs);
        }

        if self.sampled_docs.get().is_none() {
            state.sampling_rate = (1.0 * self.sample_size as f64) / state.total_hits as f64;
            let sampled = Self::create_sampled_docs(&mut state, matching_docs)?;
            let _ = self.sampled_docs.set(sampled);
        }
        Ok(self.sampled_docs.get().expect("INVARIANT: set above"))
    }

    /// Create a sampled copy of the matching documents list.
    ///
    /// Equivalent to `createSampledDocs(List)`.
    fn create_sampled_docs(state: &mut SamplingMutable, matching_docs_list: &[MatchingDocs]) -> Result<Vec<MatchingDocs>> {
        matching_docs_list
            .iter()
            .map(|docs| Self::create_sample(state, docs))
            .collect()
    }

    /// Create a sampled of the given hits.
    ///
    /// Equivalent to `createSample(MatchingDocs)`; Java wraps the
    /// `IOException` in a `RuntimeException`, which propagates as the
    /// `Err` of [`LuceneError::runtime_exception`].
    fn create_sample(state: &mut SamplingMutable, docs: &MatchingDocs) -> Result<MatchingDocs> {
        let maxdoc = docs.context.leaf_reader().max_doc();

        // TODO: we could try the WAH8DocIdSet here as well, as the results will
        // be sparse
        let mut sample_docs = FixedBitSet::new(maxdoc.max(0) as usize);

        let bin_size = (1.0 / state.sampling_rate) as i32;

        let mut counter = 0;
        let mut limit;
        let mut random_index;
        if state.leftover_bin != NOT_CALCULATED {
            limit = state.leftover_bin;
            // either NOT_CALCULATED, which means we already sampled from that
            // bin, or the next document to sample
            random_index = state.leftover_index;
        } else {
            limit = bin_size;
            random_index = state.random.next_int(bin_size);
        }
        // try { ... } catch (IOException e) { throw new RuntimeException(e); }
        let mut it = docs.bits.iterator().map_err(LuceneError::runtime_exception)?;
        let mut doc = it.next_doc().map_err(LuceneError::runtime_exception)?;
        while doc != NO_MORE_DOCS {
            if counter == random_index {
                sample_docs.set(doc as usize);
            }
            counter += 1;
            if counter >= limit {
                counter = 0;
                limit = bin_size;
                random_index = state.random.next_int(bin_size);
            }
            doc = it.next_doc().map_err(LuceneError::runtime_exception)?;
        }

        if counter == 0 {
            // we either exhausted the bin and the iterator at the same time,
            // or this segment had no results. in the latter case we might want
            // to carry leftover to the next segment as is, but that
            // complicates the code and doesn't seem so important.
            state.leftover_bin = NOT_CALCULATED;
            state.leftover_index = NOT_CALCULATED;
        } else {
            state.leftover_bin = limit - counter;
            if random_index > counter {
                // the document to sample is in the next bin
                state.leftover_index = random_index - counter;
            } else if random_index < counter {
                // we sampled a document from the bin, so just skip over
                // remaining documents in the bin in the next segment.
                state.leftover_index = NOT_CALCULATED;
            }
        }

        let set: Arc<dyn BitSet> = Arc::new(sample_docs);
        let bits: SharedDocIdSet = Arc::new(
            BitDocIdSet::from_set(set).expect("INVARIANT: a non-negative cardinality"),
        );
        let mut sample = MatchingDocs::new(Arc::clone(&docs.context), bits, docs.total_hits, None);
        sample.context_identity = docs.context_identity;
        Ok(sample)
    }
}

/// Collects hits for subsequent faceting, using sampling if needed.
///
/// Equivalent to `org.apache.lucene.facet.RandomSamplingFacetsCollector`, a
/// `FacetsCollector` subclass: it derefs to the [`FacetsCollector`] whose
/// `getMatchingDocs()` returns the sample.
pub struct RandomSamplingFacetsCollector {
    inner: FacetsCollector,
    sample_size: i32,
    seed: i64,
}

impl RandomSamplingFacetsCollector {
    /// Constructor with the given sample size and default seed.
    ///
    /// Equivalent to `RandomSamplingFacetsCollector(int)`.
    pub fn new(sample_size: i32) -> Self {
        Self::with_seed(sample_size, 0)
    }

    /// Constructor with the given sample size and seed.
    ///
    /// Equivalent to `RandomSamplingFacetsCollector(int, long)`.
    pub fn with_seed(sample_size: i32, seed: i64) -> Self {
        let mut inner = FacetsCollector::with_keep_scores(false);
        inner.sampling = Some(Box::new(SamplingState::new(sample_size, seed)));
        Self {
            inner,
            sample_size,
            seed,
        }
    }

    fn sampling(&self) -> &SamplingState {
        self.inner.sampling.as_ref().expect("INVARIANT: a sampling collector")
    }

    /// Returns the sampled list of the matching documents.
    ///
    /// Equivalent to `getMatchingDocs()`.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException` wrapping an `IOException` raised while
    /// sampling.
    pub fn get_matching_docs(&self) -> Result<&Vec<MatchingDocs>> {
        self.inner.get_matching_docs()
    }

    /// Returns the original matching documents.
    ///
    /// Equivalent to `getOriginalMatchingDocs()`.
    pub fn get_original_matching_docs(&self) -> &Vec<MatchingDocs> {
        self.inner.get_original_matching_docs()
    }

    /// Note: if you use a counting `Facets` implementation, you can amortize
    /// the sampled counts by calling this method. Uses the `FacetsConfig` and
    /// the `IndexSearcher` to determine the upper bound for each facet value.
    ///
    /// Equivalent to `amortizeFacetCounts(FacetResult, FacetsConfig,
    /// IndexSearcher)`.
    pub fn amortize_facet_counts(
        &self,
        res: Option<FacetResult>,
        config: &FacetsConfig,
        searcher: &IndexSearcher,
    ) -> Result<Option<FacetResult>> {
        let (total_hits, sampling_rate) = {
            let state = self
                .sampling()
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (state.total_hits, state.sampling_rate)
        };
        let Some(res) = res else {
            return Ok(None);
        };
        if total_hits <= self.sample_size {
            return Ok(Some(res));
        }

        let mut fixed_label_values = Vec::with_capacity(res.label_values.len());
        let reader = searcher.get_index_reader();
        let dim_config = config.get_dim_config(&res.dim);

        // +2 to prepend dimension, append child label
        let mut child_path: Vec<String> = Vec::with_capacity(res.path.len() + 2);
        child_path.push(res.dim.clone());
        child_path.extend(res.path.iter().cloned()); // reuse
        child_path.push(String::new());

        for label_value in &res.label_values {
            let last = child_path.len() - 1;
            child_path[last] = label_value.label.clone();
            let full_path = FacetsConfig::path_to_string_len(&child_path, child_path.len())?;
            let max = reader.doc_freq(&Term::new(
                dim_config.index_field_name(),
                rucene::util::BytesRef::from_char_sequence(&full_path),
            ))?;
            let mut corrected_count = (number_double_value(&label_value.value) / sampling_rate) as i32;
            corrected_count = max.min(corrected_count);
            fixed_label_values.push(LabelAndValue::new(
                label_value.label.clone(),
                NumericValue::Int(corrected_count),
            ));
        }

        // cap the total count on the total number of non-deleted documents in
        // the reader
        let mut corrected_total_count = number_int_value(&res.value);
        if corrected_total_count > 0 {
            corrected_total_count = reader
                .num_docs()
                .min((number_double_value(&res.value) / sampling_rate) as i32);
        }

        Ok(Some(FacetResult::new(
            res.dim,
            res.path,
            NumericValue::Int(corrected_total_count),
            fixed_label_values,
            res.child_count,
        )))
    }

    /// Returns the sampling rate that was used.
    ///
    /// Equivalent to `getSamplingRate()`.
    pub fn get_sampling_rate(&self) -> f64 {
        self.sampling()
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sampling_rate
    }

    /// Creates a `CollectorManager` for concurrent random sampling through
    /// `RandomSamplingFacetsCollector`.
    ///
    /// Equivalent to `createManager(int, long)`.
    pub fn create_manager(sample_size: i32, seed: i64) -> RandomSamplingCollectorManager {
        RandomSamplingCollectorManager { sample_size, seed }
    }
}

impl Deref for RandomSamplingFacetsCollector {
    type Target = FacetsCollector;

    fn deref(&self) -> &FacetsCollector {
        &self.inner
    }
}

impl DerefMut for RandomSamplingFacetsCollector {
    fn deref_mut(&mut self) -> &mut FacetsCollector {
        &mut self.inner
    }
}

impl Collector for RandomSamplingFacetsCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.inner.get_leaf_collector(context)
    }

    fn score_mode(&self) -> ScoreMode {
        self.inner.score_mode()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// The anonymous `CollectorManager` of `createManager(int, long)`.
#[derive(Debug, Clone, Copy)]
pub struct RandomSamplingCollectorManager {
    sample_size: i32,
    seed: i64,
}

impl CollectorManager for RandomSamplingCollectorManager {
    type Collector = RandomSamplingFacetsCollector;
    type Output = RandomSamplingFacetsCollector;

    fn new_collector(&self) -> Result<RandomSamplingFacetsCollector> {
        Ok(RandomSamplingFacetsCollector::with_seed(self.sample_size, self.seed))
    }

    fn reduce(&self, collectors: Vec<RandomSamplingFacetsCollector>) -> Result<RandomSamplingFacetsCollector> {
        if collectors.is_empty() {
            return Ok(RandomSamplingFacetsCollector::with_seed(self.sample_size, self.seed));
        }
        if collectors.len() == 1 {
            return Ok(collectors.into_iter().next().expect("INVARIANT: one collector"));
        }
        ReducedRandomSamplingFacetsCollector::create(self.sample_size, self.seed, &collectors)
    }
}

/// Reduces the given `RandomSamplingFacetsCollector`s.
///
/// Equivalent to
/// `org.apache.lucene.facet.RandomSamplingFacetsCollector.ReducedRandomSamplingFacetsCollector`.
pub(crate) struct ReducedRandomSamplingFacetsCollector;

impl ReducedRandomSamplingFacetsCollector {
    /// Equivalent to `ReducedRandomSamplingFacetsCollector(int, long,
    /// Collection)`: the reduced collectors' original matching docs (Java's
    /// `reduceMatchingDocs` reads each collector's `getMatchingDocs()`,
    /// which may be its sample).
    fn create(
        sample_size: i32,
        seed: i64,
        facets_collectors: &[RandomSamplingFacetsCollector],
    ) -> Result<RandomSamplingFacetsCollector> {
        let mut collector = RandomSamplingFacetsCollector::with_seed(sample_size, seed);
        let refs: Vec<&FacetsCollector> = facets_collectors.iter().map(|c| &c.inner).collect();
        let reduced = FacetsCollectorManager::reduce_matching_docs(&refs)?;
        collector.inner.get_matching_docs_mut().extend(reduced);
        Ok(collector)
    }
}