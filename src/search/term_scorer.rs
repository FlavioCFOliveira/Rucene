//! Scoring the documents matching a single term, ported from
//! `org.apache.lucene.search.TermScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::{
    DocAndFloatFeatureBuffer, Impacts, ImpactsEnum, ImpactsSource, NumericDocValues, PostingsEnum,
    SlowImpactsEnum,
};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::impacts_disi::ImpactsDISI;
use crate::search::max_score_cache::MaxScoreCache;
use crate::search::scorable::Scorable;
use crate::search::scorer::Scorer;
use crate::search::sim_scorer_source::{SharedSimScorer, SharedSimScorerRef};
use crate::search::similarities::SimScorer;
use crate::util::Bits;

/// A [`Box<dyn ImpactsEnum>`] seen as an [`ImpactsEnum`].
///
/// **Divergence from Lucene 10.5.0.** Java passes the `ImpactsEnum` straight to
/// `ImpactsDISI`. This port's [`ImpactsDISI`] is generic over the concrete enum
/// type — a `dyn ImpactsEnum` cannot be coerced to the `dyn ImpactsSource` its
/// cache takes below Rust 1.86 — and `Box<dyn ImpactsEnum>` does not implement
/// [`ImpactsEnum`] on its own, so this newtype forwards every method.
pub struct BoxedImpactsEnum(Box<dyn ImpactsEnum>);

impl BoxedImpactsEnum {
    /// Wraps a boxed impacts enum.
    pub fn new(inner: Box<dyn ImpactsEnum>) -> Self {
        Self(inner)
    }
}

impl std::fmt::Debug for BoxedImpactsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoxedImpactsEnum")
    }
}

impl DocIdSetIterator for BoxedImpactsEnum {
    fn doc_id(&self) -> i32 {
        self.0.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.0.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.advance(target)
    }

    fn cost(&self) -> i64 {
        self.0.cost()
    }

    fn into_bit_set(
        &mut self,
        up_to: i32,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        self.0.into_bit_set(up_to, bit_set, offset)
    }
}

impl PostingsEnum for BoxedImpactsEnum {
    fn freq(&self) -> Result<i32> {
        self.0.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.0.next_position()
    }

    fn start_offset(&self) -> i32 {
        self.0.start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.0.end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.0.get_payload()
    }

    fn next_postings(&mut self, up_to: i32, buffer: &mut DocAndFloatFeatureBuffer) -> Result<()> {
        self.0.next_postings(up_to, buffer)
    }
}

impl ImpactsSource for BoxedImpactsEnum {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        self.0.advance_shallow(target)
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        self.0.get_impacts()
    }
}

impl ImpactsEnum for BoxedImpactsEnum {}

/// How a [`TermScorer`] iterates its postings.
///
/// **Divergence from Lucene 10.5.0.** Java keeps three aliasing references to
/// one object — `postingsEnum`, `iterator` and the `ImpactsSource` inside the
/// `MaxScoreCache` — which Rust's ownership rules forbid. This enum owns the
/// single underlying enum and hands out each view on demand; the three Java
/// combinations become its three variants.
enum Iteration {
    /// The first constructor: a plain postings enum, wrapped in a
    /// [`SlowImpactsEnum`] so that it can also answer impacts.
    Slow(SlowImpactsEnum, MaxScoreCache),
    /// The second constructor with `topLevelScoringClause == false`: the
    /// impacts enum is iterated directly.
    Impacts(BoxedImpactsEnum, MaxScoreCache),
    /// The second constructor with `topLevelScoringClause == true`: the impacts
    /// enum is iterated through an [`ImpactsDISI`], which skips
    /// non-competitive blocks and owns the [`MaxScoreCache`] in this port.
    ImpactsDisi(Box<ImpactsDISI<BoxedImpactsEnum>>),
}

impl Iteration {
    fn postings(&mut self) -> &mut dyn PostingsEnum {
        match self {
            Self::Slow(e, _) => e,
            Self::Impacts(e, _) => e,
            Self::ImpactsDisi(d) => d.inner(),
        }
    }

    fn doc_id(&self) -> i32 {
        match self {
            Self::Slow(e, _) => e.doc_id(),
            Self::Impacts(e, _) => e.doc_id(),
            Self::ImpactsDisi(d) => d.doc_id(),
        }
    }

    /// Borrows the impacts source and the score cache at the same time, which
    /// is what `MaxScoreCache.advanceShallow` and `MaxScoreCache.getMaxScore`
    /// need; Java reaches them through two aliasing references.
    fn split(&mut self) -> (&mut dyn ImpactsSource, &mut MaxScoreCache) {
        match self {
            Self::Slow(e, c) => (e, c),
            Self::Impacts(e, c) => (e, c),
            Self::ImpactsDisi(d) => {
                let (inner, cache) = d.split_mut();
                (inner, cache)
            }
        }
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        match self {
            Self::Slow(e, _) => e,
            Self::Impacts(e, _) => e,
            Self::ImpactsDisi(d) => &mut **d,
        }
    }
}

/// Expert: a [`Scorer`] for documents matching a
/// [`Term`](crate::index::Term).
///
/// Equivalent to the `final org.apache.lucene.search.TermScorer`.
pub struct TermScorer {
    iteration: Iteration,
    scorer: SharedSimScorer,
    norms: Option<Box<dyn NumericDocValues>>,
    norm_values: Vec<i64>,
    score_spare: Vec<f32>,
}

impl std::fmt::Debug for TermScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermScorer")
            .field("doc", &self.iteration.doc_id())
            .finish_non_exhaustive()
    }
}

impl TermScorer {
    /// Constructs a `TermScorer` that will iterate all documents.
    ///
    /// Equivalent to
    /// `TermScorer(PostingsEnum, SimScorer, NumericDocValues)`.
    pub fn new(
        postings_enum: Box<dyn PostingsEnum>,
        scorer: SharedSimScorer,
        norms: Option<Box<dyn NumericDocValues>>,
    ) -> Self {
        let max_score_cache = MaxScoreCache::new(Box::new(SharedSimScorerRef::new(scorer.clone())));
        Self {
            iteration: Iteration::Slow(SlowImpactsEnum::new(postings_enum), max_score_cache),
            scorer,
            norms,
            norm_values: Vec::new(),
            score_spare: Vec::new(),
        }
    }

    /// Constructs a `TermScorer` that will use impacts to skip blocks of
    /// non-competitive documents.
    ///
    /// Equivalent to
    /// `TermScorer(ImpactsEnum, SimScorer, NumericDocValues, boolean)`.
    pub fn with_impacts(
        impacts_enum: Box<dyn ImpactsEnum>,
        scorer: SharedSimScorer,
        norms: Option<Box<dyn NumericDocValues>>,
        top_level_scoring_clause: bool,
    ) -> Self {
        let max_score_cache = MaxScoreCache::new(Box::new(SharedSimScorerRef::new(scorer.clone())));
        let impacts_enum = BoxedImpactsEnum::new(impacts_enum);
        let iteration = if top_level_scoring_clause {
            // `ImpactsDISI` owns the cache in this port, because both need the
            // same `ImpactsEnum`; see its type documentation.
            Iteration::ImpactsDisi(Box::new(ImpactsDISI::new(impacts_enum, max_score_cache)))
        } else {
            Iteration::Impacts(impacts_enum, max_score_cache)
        };
        Self {
            iteration,
            scorer,
            norms,
            norm_values: Vec::new(),
            score_spare: Vec::new(),
        }
    }

    /// Returns the term frequency in the current document.
    ///
    /// Equivalent to the `final TermScorer.freq()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the frequency.
    pub fn freq(&mut self) -> Result<i32> {
        self.iteration.postings().freq()
    }

    /// Reads the norm of `doc`, or `1` when the field has no norms.
    ///
    /// Equivalent to the `norms != null && norms.advanceExact(doc)` guard Java
    /// writes in `score()` and `smoothingScore(int)`.
    fn norm(&mut self, doc: i32) -> Result<i64> {
        if let Some(norms) = self.norms.as_mut() {
            if norms.advance_exact(doc)? {
                return norms.long_value();
            }
        }
        Ok(1)
    }
}

impl Scorable for TermScorer {
    fn score(&mut self) -> Result<f32> {
        let doc = self.iteration.doc_id();
        let freq = self.iteration.postings().freq()?;
        let norm = self.norm(doc)?;
        Ok(self.scorer.score(freq as f32, norm))
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        let norm = self.norm(doc_id)?;
        Ok(self.scorer.score(0.0, norm))
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        if let Iteration::ImpactsDisi(disi) = &mut self.iteration {
            disi.set_min_competitive_score(min_score);
        }
        Ok(())
    }
}

impl Scorer for TermScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.iteration.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.iteration.iterator()
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        let (source, cache) = self.iteration.split();
        cache.advance_shallow(source, target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let (source, cache) = self.iteration.split();
        cache.get_max_score(source, up_to)
    }

    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        loop {
            if let Iteration::ImpactsDisi(disi) = &mut self.iteration {
                disi.ensure_competitive()?;
            }

            self.iteration.postings().next_postings(up_to, buffer)?;
            if let Some(live_docs) = live_docs {
                if buffer.size != 0 {
                    // An empty result indicates that there are no more docs
                    // before `up_to`. We may be unlucky, and there are docs
                    // left, but all docs from the current batch happen to be
                    // marked as deleted. So we need to iterate until we find a
                    // batch that has at least one non-deleted doc.
                    buffer.apply(live_docs);
                    if buffer.size == 0 {
                        continue;
                    }
                }
            }
            break;
        }

        let size = buffer.size;
        if self.norm_values.len() < size {
            // Java allocates `ArrayUtil.oversize(size, Long.BYTES)` longs and,
            // when there are no norms, fills the whole array with `1`. Growing
            // the vector with a fill value of `1` is the same, because the
            // entries that survive the growth were already `1`.
            let fill = if self.norms.is_none() { 1 } else { 0 };
            self.norm_values.resize(size, fill);
        }
        if let Some(norms) = self.norms.as_mut() {
            norms.long_values(size as i32, &buffer.docs, 0, &mut self.norm_values, 0, 1)?;
        }

        if self.score_spare.len() < size {
            self.score_spare.resize(size, 0.0);
        }
        // Java passes `buffer.features` as both the frequencies and the
        // destination; this port's `BulkSimScorer` takes distinct slices, so the
        // scores are computed into a spare buffer and copied back. The values
        // are identical: Java reads every frequency before writing the score at
        // the same index.
        let scorer = SharedSimScorerRef::new(self.scorer.clone());
        {
            let mut bulk = scorer.as_bulk_sim_scorer();
            bulk.score(
                size,
                &buffer.features,
                &self.norm_values,
                &mut self.score_spare,
            );
        }
        buffer.features[..size].copy_from_slice(&self.score_spare[..size]);
        Ok(())
    }
}
