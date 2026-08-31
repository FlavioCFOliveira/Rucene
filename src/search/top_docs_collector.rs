//! Top-N collection, ported from `org.apache.lucene.search.TopDocsCollector`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::collector::Collector;
use crate::search::top_docs::TopDocs;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};

/// Returned when [`TopDocsCollector::top_docs_range`] is called with arguments
/// that select nothing, or when there simply are not (enough) results.
///
/// Equivalent to the `TopDocsCollector.EMPTY_TOPDOCS` constant. It is a
/// function rather than a constant because [`TotalHits`] is validated on
/// construction and [`TopDocs`] owns its hits.
///
/// # Panics
///
/// Never: `TotalHits::new(0, EQUAL_TO)` is always valid.
pub fn empty_top_docs() -> TopDocs {
    TopDocs::new(
        TotalHits::new(0, TotalHitsRelation::EQUAL_TO)
            .expect("INVARIANT: a total hit count of 0 is always valid"),
        Vec::new(),
    )
}

/// Base trait for the collectors that return a [`TopDocs`] result.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.TopDocsCollector<T extends ScoreDoc>`.
///
/// **Divergence from Lucene 10.5.0.** Java holds the priority queue, the hit
/// counter and the hit-count relation in `protected` fields that subclasses
/// read and write directly, and implements the paging logic on top of them. A
/// Rust trait cannot carry fields, so the state is reached through the required
/// accessors below — [`total_hits`](Self::total_hits),
/// [`total_hits_relation`](Self::total_hits_relation),
/// [`pq_size`](Self::pq_size) and [`pop`](Self::pop) — and every method Java
/// implements concretely is a default method here, with the same body. Java's
/// escape hatch of passing a `null` priority queue and overriding everything
/// corresponds to implementing the accessors over some other structure, which
/// is exactly what
/// [`TopScoreDocCollector`](crate::search::TopScoreDocCollector) does.
///
/// Java overloads `topDocs()` on arity; Rust has no overloading, so the three
/// forms are [`top_docs`](Self::top_docs), [`top_docs_from`](Self::top_docs_from)
/// and [`top_docs_range`](Self::top_docs_range).
///
/// Java's `T extends ScoreDoc` type parameter becomes the associated type
/// [`Hit`](Self::Hit). The result type is the associated
/// [`Docs`](Self::Docs), because a `TopFieldCollector` returns a
/// [`TopFieldDocs`](crate::search::TopFieldDocs) that this port cannot express
/// as a subtype of [`TopDocs`].
pub trait TopDocsCollector: Collector {
    /// The hit type this collector accumulates.
    ///
    /// Equivalent to the `T extends ScoreDoc` type parameter of
    /// `TopDocsCollector<T>`.
    type Hit;

    /// The result type this collector returns.
    ///
    /// Equivalent to the return type of the covariantly-overridden
    /// `topDocs()`: [`TopDocs`] for `TopScoreDocCollector`, and
    /// [`TopFieldDocs`](crate::search::TopFieldDocs) for `TopFieldCollector`.
    type Docs;

    /// The total number of documents that matched this query.
    ///
    /// Equivalent to `TopDocsCollector.getTotalHits()`, reading the
    /// `protected int totalHits` field.
    fn total_hits(&self) -> i32;

    /// Whether [`total_hits`](Self::total_hits) is exact or a lower bound.
    ///
    /// Equivalent to reading the `protected TotalHits.Relation
    /// totalHitsRelation` field.
    fn total_hits_relation(&self) -> TotalHitsRelation;

    /// The number of entries currently in the priority queue.
    ///
    /// Equivalent to `pq.size()`.
    fn pq_size(&self) -> usize;

    /// Removes and returns the least competitive entry of the priority queue.
    ///
    /// Equivalent to `pq.pop()`.
    fn pop(&mut self) -> Option<Self::Hit>;

    /// The number of valid priority-queue entries.
    ///
    /// Equivalent to `TopDocsCollector.topDocsSize()`. In case the queue was
    /// populated with sentinel values there may be fewer results than
    /// [`pq_size`](Self::pq_size), so the count is capped by
    /// [`total_hits`](Self::total_hits).
    fn top_docs_size(&self) -> usize {
        (self.total_hits().max(0) as usize).min(self.pq_size())
    }

    /// Populates `results` with the hits, most competitive last.
    ///
    /// Equivalent to `TopDocsCollector.populateResults(ScoreDoc[], int)`. It
    /// can be overridden in case a different hit type should be returned.
    ///
    /// # Panics
    ///
    /// Panics when the priority queue holds fewer than `how_many` entries;
    /// [`top_docs_range`](Self::top_docs_range) guarantees it does not, as
    /// Java's `NullPointerException` on the same misuse implies.
    fn populate_results(&mut self, how_many: usize) -> Vec<Self::Hit> {
        let mut results = Vec::with_capacity(how_many);
        for _ in 0..how_many {
            results.push(
                self.pop()
                    .expect("INVARIANT: prune_least_competitive_hits_to left how_many entries"),
            );
        }
        // Java writes the pops into `results` from the last index down to the
        // first, so the most competitive hit ends up first.
        results.reverse();
        results
    }

    /// Builds the result set for the given hits.
    ///
    /// Equivalent to `TopDocsCollector.newTopDocs(ScoreDoc[], int)`. A
    /// `results` of `None` means there are no results to return, either because
    /// there were no calls to collect or because the arguments to
    /// [`top_docs_range`](Self::top_docs_range) selected nothing.
    ///
    /// **Divergence from Lucene 10.5.0.** Java implements this concretely,
    /// returning the shared `EMPTY_TOPDOCS` or a new `TopDocs`. The result type
    /// is an associated type here, so the base implementation cannot construct
    /// it and every collector supplies its own; the two in this crate reproduce
    /// Java's body for their result type.
    ///
    /// # Errors
    ///
    /// Propagates the [`TotalHits`] validation error, which cannot trigger for
    /// a non-negative hit count.
    fn new_top_docs(&self, results: Option<Vec<Self::Hit>>, start: i32) -> Result<Self::Docs>;

    /// Prunes the least competitive hits until at most `keep` candidates
    /// remain.
    ///
    /// Equivalent to `TopDocsCollector.pruneLeastCompetitiveHitsTo(int)`. It is
    /// typically called before [`populate_results`](Self::populate_results) to
    /// ensure the queue is at the right position.
    fn prune_least_competitive_hits_to(&mut self, keep: usize) {
        let mut i = self.pq_size().saturating_sub(keep);
        while i > 0 {
            self.pop();
            i -= 1;
        }
    }

    /// Returns the top documents collected by this collector.
    ///
    /// Equivalent to `TopDocsCollector.topDocs()`.
    ///
    /// # Errors
    ///
    /// As [`top_docs_range`](Self::top_docs_range).
    fn top_docs(&mut self) -> Result<Self::Docs> {
        // In case the queue was populated with sentinel values there may be
        // fewer results than pq.size(), so return all results up to either
        // pq.size() or totalHits.
        let size = self.top_docs_size();
        self.top_docs_range(0, size as i32)
    }

    /// Returns the documents in `[start, pq.size())` collected by this
    /// collector, or an empty result when `start >= pq.size()`.
    ///
    /// Equivalent to `TopDocsCollector.topDocs(int)`. It is convenient when the
    /// application always asks for the last page of results.
    ///
    /// This method cannot be called more than once per search execution; to
    /// page repeatedly, call [`top_docs`](Self::top_docs) once and work with
    /// the returned [`TopDocs`].
    ///
    /// # Errors
    ///
    /// As [`top_docs_range`](Self::top_docs_range).
    fn top_docs_from(&mut self, start: i32) -> Result<Self::Docs> {
        let size = self.top_docs_size();
        self.top_docs_range(start, size as i32)
    }

    /// Returns the documents in `[start, start + how_many)` collected by this
    /// collector.
    ///
    /// Equivalent to `TopDocsCollector.topDocs(int, int)`. If
    /// `start >= pq.size()` an empty result is returned, and if
    /// `pq.size() - start < how_many` only the available documents in
    /// `[start, pq.size())` are returned. It is useful when the search
    /// application paginates results, since it allocates only as much as
    /// requested.
    ///
    /// This method cannot be called more than once per search execution.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when `how_many` or `start` is negative.
    fn top_docs_range(&mut self, start: i32, how_many: i32) -> Result<Self::Docs> {
        // In case the queue was populated with sentinel values there may be
        // fewer results than pq.size(), so return all results up to either
        // pq.size() or totalHits.
        let size = self.top_docs_size();

        if how_many < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "Number of hits requested must be greater than 0 but value was {how_many}"
            )));
        }

        if start < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "Expected value of starting position is between 0 and {size}, got {start}"
            )));
        }

        let start_usize = start as usize;
        if start_usize >= size || how_many == 0 {
            return self.new_top_docs(None, start);
        }

        // We know that start < size, so just fix how_many.
        let how_many = (size - start_usize).min(how_many as usize);

        // Prune the least competitive hits until we reach the requested range.
        // Note that this loop will usually not be executed, since the common
        // usage should be that the caller asks for the last how_many results.
        // However it is needed here for completeness.
        self.prune_least_competitive_hits_to(start_usize + how_many);

        // Get the requested results from the queue.
        let results = self.populate_results(how_many);

        self.new_top_docs(Some(results), start)
    }
}
