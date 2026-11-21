//! Search timeouts, ported from
//! `org.apache.lucene.search.TimeLimitingBulkScorer`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::index::QueryTimeout;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::collector::LeafCollector;
use crate::util::{Bits, MathUtil};

/// Times out search requests that take longer than the maximum allowed search
/// time.
///
/// Equivalent to `org.apache.lucene.search.TimeLimitingBulkScorer`, which is
/// package-private and `final` in Java; it is public here because Rust has no
/// package visibility. After the allowance is exceeded, scoring stops with
/// [`CollectionError::TimeExceeded`] — the counterpart of Java's
/// `TimeLimitingBulkScorer.TimeExceededException`, which
/// [`IndexSearcher`](crate::search::IndexSearcher) catches to record a partial
/// result.
pub struct TimeLimitingBulkScorer {
    inner: Box<dyn BulkScorer>,
    query_timeout: Arc<dyn QueryTimeout>,
}

/// Documents are scored in chunks of this many at a time, so as to avoid the
/// cost of checking the timeout for every document scored.
///
/// Equivalent to `TimeLimitingBulkScorer.INTERVAL`.
pub const INTERVAL: i32 = 100;

impl std::fmt::Debug for TimeLimitingBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeLimitingBulkScorer")
            .field("query_timeout", &self.query_timeout)
            .finish_non_exhaustive()
    }
}

impl TimeLimitingBulkScorer {
    /// Wraps another bulk scorer with the given timeout.
    ///
    /// Equivalent to `new TimeLimitingBulkScorer(BulkScorer, QueryTimeout)`;
    /// Java's `Objects.requireNonNull` checks are unnecessary because neither
    /// argument can be null.
    pub fn new(bulk_scorer: Box<dyn BulkScorer>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: bulk_scorer,
            query_timeout,
        }
    }
}

impl BulkScorer for TimeLimitingBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let mut interval = INTERVAL;
        let mut min = min;
        while min < max {
            let new_max = MathUtil::unsigned_min(min.wrapping_add(interval), max);
            // Increase the interval by 50% on each iteration, guarding against
            // overflow.
            let new_interval = interval.wrapping_add(interval >> 1);
            if interval < new_interval {
                interval = new_interval;
            }
            if self.query_timeout.should_exit() {
                return Err(CollectionError::TimeExceeded);
            }
            min = self.inner.score(collector, accept_docs, min, new_max)?;
        }
        Ok(min)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}
