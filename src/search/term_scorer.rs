//! Scoring the documents matching a single term, ported from
//! `org.apache.lucene.search.TermScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::{
    DocAndFloatFeatureBuffer, ImpactsEnum, ImpactsRef, ImpactsSource, NumericDocValues,
    PostingsEnum, SlowImpactsEnum,
};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::impacts_disi::ImpactsDISI;
use crate::search::max_score_cache::MaxScoreCache;
use crate::search::scorable::Scorable;
use crate::search::scorer::Scorer;
use crate::search::sim_scorer_source::{SharedSimScorer, SharedSimScorerRef};
use crate::util::Bits;

/// A [`Box<dyn ImpactsEnum>`] seen as an [`ImpactsEnum`].
///
/// **Divergence from Lucene 10.5.0.** Java passes the `ImpactsEnum` straight to
/// `ImpactsDISI`. This port's [`ImpactsDISI`] is generic over a sized source
/// type, which it coerces to the `dyn ImpactsSource` its cache takes, and
/// `Box<dyn ImpactsEnum>` does not implement [`ImpactsEnum`] on its own, so
/// this newtype forwards every method.
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

impl std::fmt::Display for BoxedImpactsEnum {
    /// Port-only newtype with no Lucene counterpart: Java passes the
    /// `ImpactsEnum` straight through, so this forwards to the enumeration it
    /// wraps.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
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

    fn get_impacts(&mut self) -> Result<ImpactsRef<'_>> {
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
    /// [`SlowImpactsEnum`] so that it can also answer impacts. As in Java,
    /// only the score cache sees the wrapper: the postings, the doc ID and
    /// the iterator are the wrapped enum itself (`TermScorer.java:48-50`), so
    /// its bulk paths (`intoBitSet`, `docIDRunEnd`) are the ones callers reach.
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
            Self::Slow(e, _) => e.delegate_mut(),
            Self::Impacts(e, _) => e,
            Self::ImpactsDisi(d) => d.inner(),
        }
    }

    fn doc_id(&self) -> i32 {
        match self {
            Self::Slow(e, _) => e.delegate_ref().doc_id(),
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
            // PERF: `iterator = this.postingsEnum` (`TermScorer.java:48`): the
            // wrapped enum, whose `into_bit_set`/`doc_id_run_end` overrides
            // the `SlowImpactsEnum` forwarder does not carry.
            Self::Slow(e, _) => e.delegate_mut(),
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
    /// This scorer answers with itself: Java's `instanceof` and
    /// `getClass()` on a `Scorable` see the `TermScorer`.
    fn as_scorer(&mut self) -> Option<&mut dyn crate::search::Scorer> {
        Some(self)
    }

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
    /// This scorer answers with itself: Java tests `instanceof TermScorer`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
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

        // `bulkScorer.score(buffer.size, buffer.features, normValues,
        // buffer.features)` (`TermScorer.java:166`), with Java's `features` as
        // both the frequencies and the destination.
        //
        // PERF: Java builds `bulkScorer` once, in the constructor
        // (`TermScorer.java:54, 77`). This port's `as_bulk_sim_scorer` borrows
        // the scorer, so it was boxed again on every call, through a
        // `SharedSimScorerRef` whose bulk scorer is the default one: each
        // score is `score(freq, norm)`. That default loop now runs here, in
        // place, without the per-call box, `Arc` clone and spare copy; every
        // frequency is read before the score at its index is written, so the
        // values are the same.
        let scorer = &*self.scorer;
        let norms = &self.norm_values[..size];
        for (feature, &norm) in buffer.features[..size].iter_mut().zip(norms) {
            *feature = scorer.score(*feature, norm);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::similarities::SimScorer;
    use crate::search::NO_MORE_DOCS;
    use crate::util::FixedBitSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A postings enum over the docs `[0, len)` that counts the calls to its
    /// own bulk overrides.
    struct RunPostings {
        doc: i32,
        len: i32,
        into_bit_set_calls: Arc<AtomicUsize>,
        run_end_calls: Arc<AtomicUsize>,
    }

    impl DocIdSetIterator for RunPostings {
        fn doc_id(&self) -> i32 {
            self.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            let target = self.doc + 1;
            self.advance(target)
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            self.doc = if target >= self.len {
                NO_MORE_DOCS
            } else {
                target
            };
            Ok(self.doc)
        }
        fn cost(&self) -> i64 {
            i64::from(self.len)
        }
        fn into_bit_set(
            &mut self,
            up_to: i32,
            bit_set: &mut FixedBitSet,
            offset: i32,
        ) -> Result<()> {
            self.into_bit_set_calls.fetch_add(1, Ordering::Relaxed);
            let up_to = up_to.min(self.len);
            if up_to > self.doc {
                bit_set.set_range((self.doc - offset) as usize, (up_to - offset) as usize);
                self.advance(up_to)?;
            }
            Ok(())
        }
        fn doc_id_run_end(&mut self) -> Result<i32> {
            self.run_end_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.len)
        }
    }

    impl std::fmt::Display for RunPostings {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("RunPostings")
        }
    }

    impl PostingsEnum for RunPostings {
        fn freq(&self) -> Result<i32> {
            Ok(1)
        }
        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }
        fn start_offset(&self) -> i32 {
            -1
        }
        fn end_offset(&self) -> i32 {
            -1
        }
        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    struct ConstantSimScorer;

    impl SimScorer for ConstantSimScorer {
        fn score(&self, _freq: f32, _norm: i64) -> f32 {
            1.0
        }
    }

    /// `TermScorer(PostingsEnum, SimScorer, NumericDocValues)` iterates the
    /// postings enum itself (`iterator = this.postingsEnum`,
    /// `TermScorer.java:48`); the `SlowImpactsEnum` it builds serves only the
    /// score cache. So the enum's own `into_bit_set` and `doc_id_run_end`
    /// answer, not the forwarder's per-document defaults.
    #[test]
    fn plain_term_scorer_iterates_the_postings_enum_itself() {
        let into_bit_set_calls = Arc::new(AtomicUsize::new(0));
        let run_end_calls = Arc::new(AtomicUsize::new(0));
        let postings = RunPostings {
            doc: -1,
            len: 100,
            into_bit_set_calls: Arc::clone(&into_bit_set_calls),
            run_end_calls: Arc::clone(&run_end_calls),
        };
        let mut scorer = TermScorer::new(Box::new(postings), Arc::new(ConstantSimScorer), None);
        assert_eq!(scorer.iterator().next_doc().unwrap(), 0);
        assert_eq!(scorer.iterator().doc_id_run_end().unwrap(), 100);
        let mut bits = FixedBitSet::new(100);
        scorer.iterator().into_bit_set(60, &mut bits, 0).unwrap();
        assert_eq!(scorer.doc_id(), 60);
        assert_eq!(bits.cardinality(), 60);
        assert_eq!(scorer.freq().unwrap(), 1);
        assert_eq!(into_bit_set_calls.load(Ordering::Relaxed), 1);
        assert_eq!(run_end_calls.load(Ordering::Relaxed), 1);
    }
}
