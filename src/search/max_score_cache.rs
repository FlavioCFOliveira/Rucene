//! Impact-based score bounds, ported from
//! `org.apache.lucene.search.MaxScoreCache`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::{FreqAndNormBuffer, ImpactsSource};
use crate::search::similarities::SimScorer;

/// Computes maximum scores from the
/// [`Impacts`](crate::index::Impacts) stored in the index and keeps them in a
/// cache, so that expensive similarity computations are not run several times
/// on the same data.
///
/// Equivalent to the `final class org.apache.lucene.search.MaxScoreCache`.
///
/// **Divergence from Lucene 10.5.0.** Java's cache holds the
/// [`ImpactsSource`] it reads from, which is the very
/// [`ImpactsEnum`](crate::index::ImpactsEnum) the enclosing scorer iterates.
/// Rust forbids that aliasing, so the source is passed to every method instead
/// of being stored. For the same reason the
/// [`BulkSimScorer`](crate::search::similarities::BulkSimScorer) is built on
/// each bulk computation rather than once in the constructor: this crate's
/// `as_bulk_sim_scorer` borrows the similarity scorer, which a field cannot
/// hold beside the scorer it borrows from. The scores produced are identical.
pub struct MaxScoreCache {
    scorer: Box<dyn SimScorer>,
    global_max_score: f32,
    max_score_cache: Vec<f32>,
    max_score_cache_up_to: Vec<i32>,
    freq_spare: Vec<f32>,
    score_spare: Vec<f32>,
}

impl std::fmt::Debug for MaxScoreCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaxScoreCache")
            .field("global_max_score", &self.global_max_score)
            .field("levels", &self.max_score_cache.len())
            .finish_non_exhaustive()
    }
}

impl MaxScoreCache {
    /// Creates a cache for the given similarity scorer.
    ///
    /// Equivalent to `new MaxScoreCache(ImpactsSource, SimScorer)`, minus the
    /// impacts source; see the type documentation.
    pub fn new(scorer: Box<dyn SimScorer>) -> Self {
        let global_max_score = scorer.score(f32::MAX, 1);
        Self {
            scorer,
            global_max_score,
            max_score_cache: Vec::new(),
            max_score_cache_up_to: Vec::new(),
            freq_spare: Vec::new(),
            score_spare: Vec::new(),
        }
    }

    /// Implements the contract of
    /// [`Scorer::advance_shallow`](crate::search::Scorer::advance_shallow)
    /// based on the wrapped impacts source.
    ///
    /// Equivalent to `MaxScoreCache.advanceShallow(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    pub fn advance_shallow(
        &mut self,
        impacts_source: &mut dyn ImpactsSource,
        target: i32,
    ) -> Result<i32> {
        impacts_source.advance_shallow(target)?;
        let impacts = impacts_source.get_impacts()?;
        Ok(impacts.doc_id_up_to(0))
    }

    /// Equivalent to the private `MaxScoreCache.ensureCacheSize(int)`.
    fn ensure_cache_size(&mut self, size: usize) {
        if self.max_score_cache.len() < size {
            let old_length = self.max_score_cache.len();
            // `ArrayUtil.grow` over-allocates; the exact capacity does not
            // change any observable behaviour, only the number of resizes.
            self.max_score_cache.resize(size.max(old_length * 2 + 1), 0.0);
            self.max_score_cache_up_to
                .resize(self.max_score_cache.len(), -1);
        }
    }

    /// Equivalent to the private
    /// `MaxScoreCache.computeMaxScore(FreqAndNormBuffer)`.
    fn compute_max_score(&mut self, impacts: &FreqAndNormBuffer) -> f32 {
        let size = impacts.size;
        if self.freq_spare.len() < size {
            self.freq_spare.resize(size, 0.0);
            self.score_spare.resize(size, 0.0);
        }
        for i in 0..size {
            self.freq_spare[i] = impacts.freqs[i] as f32;
        }
        {
            let mut bulk_scorer = self.scorer.as_bulk_sim_scorer();
            bulk_scorer.score(
                size,
                &self.freq_spare,
                &impacts.norms,
                &mut self.score_spare,
            );
        }

        let mut max_score = 0.0f32;
        for i in 0..size {
            max_score = max_score.max(self.score_spare[i]);
        }
        max_score
    }

    /// Returns the maximum score up to `up_to`, included.
    ///
    /// Equivalent to `MaxScoreCache.getMaxScore(int)`; see
    /// [`Scorer::get_max_score`](crate::search::Scorer::get_max_score).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    pub fn get_max_score(
        &mut self,
        impacts_source: &mut dyn ImpactsSource,
        up_to: i32,
    ) -> Result<f32> {
        let level = self.get_level(impacts_source, up_to)?;
        if level == -1 {
            return Ok(self.global_max_score);
        }
        self.get_max_score_for_level(impacts_source, level)
    }

    /// Returns the first level that includes all doc IDs up to `up_to`, or `-1`
    /// if there is no such level.
    ///
    /// Equivalent to the private `MaxScoreCache.getLevel(int)`.
    fn get_level(&mut self, impacts_source: &mut dyn ImpactsSource, up_to: i32) -> Result<i32> {
        let impacts = impacts_source.get_impacts()?;
        let num_levels = impacts.num_levels();
        for level in 0..num_levels {
            let impacts_up_to = impacts.doc_id_up_to(level);
            if up_to <= impacts_up_to {
                return Ok(level);
            }
        }
        Ok(-1)
    }

    /// Returns the maximum score of level `0`.
    ///
    /// Equivalent to the package-private
    /// `MaxScoreCache.getMaxScoreForLevelZero()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    pub fn get_max_score_for_level_zero(
        &mut self,
        impacts_source: &mut dyn ImpactsSource,
    ) -> Result<f32> {
        self.get_max_score_for_level(impacts_source, 0)
    }

    /// Returns the maximum score for the given level.
    ///
    /// Equivalent to the private `MaxScoreCache.getMaxScoreForLevel(int)`.
    fn get_max_score_for_level(
        &mut self,
        impacts_source: &mut dyn ImpactsSource,
        level: i32,
    ) -> Result<f32> {
        debug_assert!(level >= 0, "level must not be a negative integer");
        let impacts = impacts_source.get_impacts()?;
        self.ensure_cache_size(level as usize + 1);
        let level_up_to = impacts.doc_id_up_to(level);
        if self.max_score_cache_up_to[level as usize] < level_up_to {
            let buffer = impacts.get_impacts(level);
            let max_score = self.compute_max_score(&buffer);
            self.max_score_cache[level as usize] = max_score;
            self.max_score_cache_up_to[level as usize] = level_up_to;
        }
        Ok(self.max_score_cache[level as usize])
    }

    /// Returns the maximum level at which scores are all less than `min_score`,
    /// or `-1` if there is none.
    ///
    /// Equivalent to the private `MaxScoreCache.getSkipLevel(Impacts, float)`.
    fn get_skip_level(
        &mut self,
        impacts_source: &mut dyn ImpactsSource,
        num_levels: i32,
        min_score: f32,
    ) -> Result<i32> {
        for level in 0..num_levels {
            if self.get_max_score_for_level(impacts_source, level)? >= min_score {
                return Ok(level - 1);
            }
        }
        Ok(num_levels - 1)
    }

    /// Returns an inclusive upper bound of documents that all have a score less
    /// than `min_score`, or `-1` if the current document may be competitive.
    ///
    /// Equivalent to the package-private `MaxScoreCache.getSkipUpTo(float)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    pub fn get_skip_up_to(
        &mut self,
        impacts_source: &mut dyn ImpactsSource,
        min_score: f32,
    ) -> Result<i32> {
        let num_levels = impacts_source.get_impacts()?.num_levels();
        let level = self.get_skip_level(impacts_source, num_levels, min_score)?;
        if level == -1 {
            return Ok(-1);
        }
        let impacts = impacts_source.get_impacts()?;
        Ok(impacts.doc_id_up_to(level))
    }
}
