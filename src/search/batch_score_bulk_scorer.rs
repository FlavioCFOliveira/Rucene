//! Batched scoring, ported from
//! `org.apache.lucene.search.BatchScoreBulkScorer`.

#![deny(unsafe_code)]

use crate::index::DocAndFloatFeatureBuffer;
use crate::search::bulk_scorer::{BulkScorer, DefaultBulkScorer};
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::scorable::SimpleScorable;
use crate::search::scorer::Scorer;
use crate::util::Bits;

/// A [`BulkScorer`] used when
/// [`ScoreMode::needs_scores`](crate::search::ScoreMode::needs_scores) is true
/// and [`Scorer::next_docs_and_scores`] has optimisations that make it run
/// faster than one-by-one iteration.
///
/// Equivalent to `org.apache.lucene.search.BatchScoreBulkScorer`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility.
pub struct BatchScoreBulkScorer {
    scorable: SimpleScorable,
    buffer: DocAndFloatFeatureBuffer,
    scorer: Option<Box<dyn Scorer>>,
    cost: i64,
}

impl std::fmt::Debug for BatchScoreBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchScoreBulkScorer")
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

/// Message used where the scorer is known to be present because it is only ever
/// taken out for the length of one delegated `score` call, which puts it back.
const SCORER_INVARIANT: &str =
    "INVARIANT: the scorer is only moved out for the duration of one delegated score call";

impl BatchScoreBulkScorer {
    /// Wraps the given scorer.
    ///
    /// Equivalent to `new BatchScoreBulkScorer(Scorer)`.
    pub fn new(mut scorer: Box<dyn Scorer>) -> Self {
        let cost = scorer.iterator().cost();
        Self {
            scorable: SimpleScorable::new(),
            buffer: DocAndFloatFeatureBuffer::new(),
            scorer: Some(scorer),
            cost,
        }
    }

    fn scorer(&mut self) -> &mut Box<dyn Scorer> {
        self.scorer.as_mut().expect(SCORER_INVARIANT)
    }
}

impl BulkScorer for BatchScoreBulkScorer {
    fn cost(&self) -> i64 {
        self.cost
    }

    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        if collector.competitive_iterator()?.is_some() {
            // Java builds a throw-away `Weight.DefaultBulkScorer` around the
            // very same scorer; Rust cannot alias an owned scorer, so it is
            // handed over for the duration of the call and taken back.
            let scorer = self.scorer.take().expect(SCORER_INVARIANT);
            let mut delegate = DefaultBulkScorer::new(scorer);
            let result = delegate.score(collector, accept_docs, min, max);
            self.scorer = Some(delegate.into_scorer());
            return result;
        }

        collector.set_scorer(&mut self.scorable)?;
        let min_competitive_score = self.scorable.min_competitive_score();
        self.scorer()
            .set_min_competitive_score(min_competitive_score)?;

        if self.scorer().doc_id() < min {
            self.scorer().iterator().advance(min)?;
        }

        loop {
            {
                let Self { scorer, buffer, .. } = self;
                let scorer = scorer.as_mut().expect(SCORER_INVARIANT);
                scorer.next_docs_and_scores(max, accept_docs, buffer)?;
            }
            if self.buffer.size == 0 {
                break;
            }
            for i in 0..self.buffer.size {
                let score = self.buffer.features[i];
                self.scorable.set_score(score);
                if score >= self.scorable.min_competitive_score() {
                    let doc = self.buffer.docs[i];
                    collector.collect(doc, &mut self.scorable)?;
                }
            }
            let min_competitive_score = self.scorable.min_competitive_score();
            self.scorer()
                .set_min_competitive_score(min_competitive_score)?;
        }

        Ok(self.scorer().doc_id())
    }
}
