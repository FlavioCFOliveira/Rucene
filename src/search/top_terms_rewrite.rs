//! Rewriting a multi-term query to its most competitive terms, ported from
//! `org.apache.lucene.search.TopTermsRewrite`.

#![deny(unsafe_code)]

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

use crate::error::Result;
use crate::index::{IndexReaderContext, LeafReaderContext, Term, TermsEnum};
use crate::search::boost_attribute::{add_boost_attribute, boost_of};
use crate::search::index_searcher::IndexSearcher;
use crate::search::max_non_competitive_boost_attribute::{
    add_max_non_competitive_boost_attribute, set_max_non_competitive_boost,
};
use crate::search::multi_term_query::{MultiTermQuery, RewriteMethod};
use crate::search::query::Query;
use crate::search::term_collecting_rewrite::{collect_terms, TermCollectingRewrite, TermCollector};
use crate::search::term_states::TermStates;
use crate::util::attribute::AttributeSource;
use crate::util::BytesRef;

/// The base rewrite method that collects only the top terms through a priority
/// queue.
///
/// Equivalent to the abstract class `org.apache.lucene.search.TopTermsRewrite`,
/// which is only public in Java so that the spans package can reach it. The
/// `final rewrite` is the free function [`top_terms_rewrite`], and the
/// `equals`/`hashCode` pair is [`top_terms_rewrite_eq`] and
/// [`top_terms_rewrite_hash`].
pub trait TopTermsRewrite: TermCollectingRewrite {
    /// Returns the maximum priority-queue size requested.
    ///
    /// Equivalent to `TopTermsRewrite.getSize()`.
    fn get_size(&self) -> i32;

    /// Returns the maximum size of the priority queue; for the boolean
    /// rewrites this is
    /// [`IndexSearcher::get_max_clause_count`].
    ///
    /// Equivalent to the `protected abstract
    /// TopTermsRewrite.getMaxSize()`.
    fn get_max_size(&self) -> i32;
}

/// The hash code shared by every [`TopTermsRewrite`].
///
/// Equivalent to `TopTermsRewrite.hashCode()`, which is `31 * size`.
pub fn top_terms_rewrite_hash(rewrite: &dyn TopTermsRewrite) -> u64 {
    31u64.wrapping_mul(rewrite.get_size() as u64)
}

/// The equality shared by every [`TopTermsRewrite`] of the same concrete type.
///
/// Equivalent to `TopTermsRewrite.equals(Object)`, which compares the class and
/// then the size; the caller has already established that the classes match.
pub fn top_terms_rewrite_eq(a: &dyn TopTermsRewrite, b: &dyn TopTermsRewrite) -> bool {
    a.get_size() == b.get_size()
}

/// Java's `Float.compare(float, float)`.
///
/// It differs from `f32::partial_cmp` on `NaN` and on the two zeroes, which
/// matters here because [`FuzzyQuery`](crate::search::FuzzyQuery) produces
/// boosts that may be negative. `total_cmp` reproduces `Float.compare` for
/// every value except a negative `NaN`, which Java canonicalises to the
/// positive one through `floatToIntBits` and `total_cmp` orders below
/// `-inf`; a boost is never `NaN`, because a `NaN` boost is rejected by
/// [`BoostQuery`](crate::search::BoostQuery) in both implementations.
fn java_float_compare(a: f32, b: f32) -> Ordering {
    a.total_cmp(&b)
}

/// Java's `Math.max(float, float)`.
///
/// It differs from [`f32::max`], which returns the other operand when one is
/// `NaN` while Java propagates the `NaN`.
fn java_math_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.max(b)
    }
}

/// The ordering key of a collected term.
///
/// Equivalent to `TopTermsRewrite.ScoreTerm`, restricted to the two fields its
/// `compareTo` reads. The [`TermStates`] the Java class also carries is mutated
/// after insertion, which a heap element in Rust cannot be, so it lives in
/// [`TopTermsCollector::entries`] and is reached through `index`.
#[derive(Debug, Clone)]
struct ScoreTerm {
    boost: f32,
    bytes: BytesRef,
    index: usize,
}

impl PartialEq for ScoreTerm {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ScoreTerm {}

impl PartialOrd for ScoreTerm {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoreTerm {
    /// Equivalent to `ScoreTerm.compareTo(ScoreTerm)`: equal boosts are broken
    /// by the *reversed* term order, so that the least element of the queue is
    /// the least competitive term.
    fn cmp(&self, other: &Self) -> Ordering {
        if self.boost == other.boost {
            other.bytes.cmp(&self.bytes)
        } else {
            java_float_compare(self.boost, other.boost)
        }
    }
}

/// The mutable half of a collected term.
struct ScoreTermState {
    bytes: BytesRef,
    boost: f32,
    term_state: TermStates,
}

/// The collector `TopTermsRewrite.rewrite` feeds.
///
/// Equivalent to the anonymous `TermCollector` it builds.
struct TopTermsCollector {
    attributes: AttributeSource,
    max_size: usize,
    /// The priority queue of the most competitive terms seen so far. Java uses
    /// a `java.util.PriorityQueue`, a min-heap on `ScoreTerm.compareTo`;
    /// [`BinaryHeap`] is a max-heap, so the elements are wrapped in
    /// [`Reverse`].
    queue: BinaryHeap<Reverse<ScoreTerm>>,
    /// The mutable state of every live entry, indexed by [`ScoreTerm::index`].
    entries: Vec<Option<ScoreTermState>>,
    /// Equivalent to the `visitedTerms` map, from term bytes to the entry.
    visited_terms: HashMap<Vec<u8>, ScoreTerm>,
    top_reader_context: Option<Arc<dyn IndexReaderContext>>,
    reader_ord: usize,
}

impl TermCollector for TopTermsCollector {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn set_reader_context(
        &mut self,
        top_reader_context: &Arc<dyn IndexReaderContext>,
        reader_context: &Arc<LeafReaderContext>,
    ) -> Result<()> {
        self.top_reader_context = Some(Arc::clone(top_reader_context));
        self.reader_ord = reader_context.ord() as usize;
        Ok(())
    }

    fn set_next_enum(&mut self, terms_enum: &mut dyn TermsEnum) -> Result<()> {
        add_boost_attribute(terms_enum.attributes());
        Ok(())
    }

    fn collect(&mut self, bytes: &BytesRef, terms_enum: &mut dyn TermsEnum) -> Result<bool> {
        let boost = boost_of(terms_enum.attributes());

        // Ignore uncompetitive hits.
        if self.queue.len() == self.max_size {
            if let Some(Reverse(t)) = self.queue.peek() {
                if boost < t.boost {
                    return Ok(true);
                }
                if boost == t.boost && bytes.cmp(&t.bytes) == Ordering::Greater {
                    return Ok(true);
                }
            }
        }

        let key = bytes.slice().to_vec();
        let state = terms_enum.term_state()?;
        let doc_freq = terms_enum.doc_freq()?;
        let total_term_freq = terms_enum.total_term_freq()?;

        if let Some(t) = self.visited_terms.get(&key) {
            // The term is already in the queue; only update the doc freq of the
            // entry that is in it.
            let index = t.index;
            debug_assert_eq!(
                t.boost, boost,
                "boost should be equal in all segment TermsEnums"
            );
            if let Some(entry) = self.entries[index].as_mut() {
                entry
                    .term_state
                    .register(state, self.reader_ord, doc_freq, total_term_freq);
            }
            return Ok(true);
        }

        // Add a new entry to the queue; the term must be copied, else it may
        // get overwritten.
        let top = self
            .top_reader_context
            .as_ref()
            .expect("INVARIANT: set_reader_context runs before collect");
        let mut term_state = TermStates::new(top)?;
        term_state.register(state, self.reader_ord, doc_freq, total_term_freq);
        let bytes = BytesRef::deep_copy_of(bytes);
        let index = self.entries.len();
        self.entries.push(Some(ScoreTermState {
            bytes: bytes.clone(),
            boost,
            term_state,
        }));
        let st = ScoreTerm {
            boost,
            bytes,
            index,
        };
        self.visited_terms.insert(key, st.clone());
        self.queue.push(Reverse(st));

        // Possibly drop entries from the queue.
        if self.queue.len() > self.max_size {
            if let Some(Reverse(dropped)) = self.queue.pop() {
                self.visited_terms.remove(dropped.bytes.slice());
                // Java resets the term state and reuses the object; dropping
                // the entry is the same thing without the pooling.
                self.entries[dropped.index] = None;
            }
        }
        debug_assert!(
            self.queue.len() <= self.max_size,
            "the PQ size must be limited to maxSize"
        );

        // Set the max-non-competitive-boost attribute so that
        // `FuzzyTermsEnum` can optimise.
        if self.queue.len() == self.max_size {
            let top = self
                .queue
                .peek()
                .map(|Reverse(t)| (t.boost, t.bytes.clone()));
            if let Some((boost, term)) = top {
                set_max_non_competitive_boost(&mut self.attributes, boost, Some(term));
            }
        }

        Ok(true)
    }
}

/// Rewrites a multi-term query by keeping only its most competitive terms.
///
/// Equivalent to the `final TopTermsRewrite.rewrite(IndexSearcher,
/// MultiTermQuery)`.
///
/// # Errors
///
/// Propagates any I/O error raised while enumerating terms, and any error the
/// builder raises.
pub fn top_terms_rewrite<R>(
    rewrite: &R,
    index_searcher: &IndexSearcher,
    query: &dyn MultiTermQuery,
) -> Result<Arc<dyn Query>>
where
    R: TopTermsRewrite + RewriteMethod,
{
    let max_size = rewrite.get_size().min(rewrite.get_max_size()).max(0) as usize;
    // Java installs the attribute in the anonymous collector's field
    // initialiser, before any segment enum is pulled, so that
    // `FuzzyTermsEnum` shares it from the very first segment.
    let mut attributes = AttributeSource::new();
    add_max_non_competitive_boost_attribute(&mut attributes);
    let mut collector = TopTermsCollector {
        attributes,
        max_size,
        queue: BinaryHeap::new(),
        entries: Vec::new(),
        visited_terms: HashMap::new(),
        top_reader_context: None,
        reader_ord: 0,
    };
    collect_terms(index_searcher, rewrite, query, &mut collector)?;

    let mut builder = rewrite.get_top_level_builder()?;
    let TopTermsCollector {
        queue, mut entries, ..
    } = collector;
    // `stQueue.toArray()` followed by a sort on the term bytes. The terms are
    // distinct, so the sort is total and the arbitrary heap order does not
    // reach the result.
    let mut score_terms: Vec<ScoreTermState> = queue
        .into_iter()
        .filter_map(|Reverse(st)| entries[st.index].take())
        .collect();
    score_terms.sort_by(|a, b| a.bytes.cmp(&b.bytes));

    for st in score_terms {
        let term = Term::new(query.get_field(), st.bytes);
        let doc_freq = st.term_state.doc_freq()?;
        // Negative term scores are allowed while collecting the terms — fuzzy
        // query produces them — but such boosts are truncated to `0` when
        // building the query.
        builder.add_clause(
            term,
            doc_freq,
            java_math_max(0.0, st.boost),
            Some(Arc::new(st.term_state)),
        )?;
    }
    builder.build()
}
