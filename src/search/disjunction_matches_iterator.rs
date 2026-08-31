//! Combining the match positions of several sub-iterators, ported from
//! `org.apache.lucene.search.DisjunctionMatchesIterator`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::POSTINGS_ENUM_OFFSETS;
use crate::index::{EmptyTerms, LeafReaderContext, PostingsEnum, Term, Terms, TermsEnum};
use crate::search::matches::MatchesIterator;
use crate::search::query::Query;
use crate::search::term_matches_iterator::TermMatchesIterator;
use crate::util::{BytesRef, BytesRefIterator};

/// Message used where the queue is known to be non-empty because the contract
/// of [`MatchesIterator`] forbids reading a position before
/// [`MatchesIterator::next`] has returned `true`.
const NON_EMPTY: &str =
    "INVARIANT: MatchesIterator positions may only be read after next() returned true";

/// Creates a [`DisjunctionMatchesIterator`] over a list of terms.
///
/// Equivalent to
/// `DisjunctionMatchesIterator.fromTerms(LeafReaderContext, int, Query, String, List<Term>)`.
/// Only terms that have at least one match in the given document are included.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when a term does not belong to
/// `field`, and propagates any I/O error raised while reading the postings.
pub fn from_terms(
    context: &LeafReaderContext,
    doc: i32,
    query: Arc<dyn Query>,
    field: &str,
    terms: &[Term],
) -> Result<Option<Box<dyn MatchesIterator>>> {
    for term in terms {
        if term.field() != field {
            return Err(LuceneError::IllegalArgument(format!(
                "Tried to generate iterator from terms in multiple fields: expected [{}] but got [{}]",
                field,
                term.field()
            )));
        }
    }
    from_terms_enum(
        context,
        doc,
        query,
        field,
        Box::new(TermBytesIterator {
            terms: terms.iter().map(|t| t.bytes().clone()).collect(),
            i: 0,
        }),
    )
}

/// A [`BytesRefIterator`] over the terms of a [`TermsEnum`].
///
/// **Divergence from Lucene 10.5.0.** Java's `TermsEnum` implements
/// `BytesRefIterator`, so a terms enum can be passed straight to
/// [`from_terms_enum`]. This port's [`TermsEnum`] declares the same `next`
/// method but does not extend [`BytesRefIterator`], so the adapter is spelled
/// out; it forwards the one method.
pub struct TermsEnumBytesRefIterator(Box<dyn TermsEnum>);

impl TermsEnumBytesRefIterator {
    /// Views a terms enum as a [`BytesRefIterator`].
    pub fn new(terms_enum: Box<dyn TermsEnum>) -> Self {
        Self(terms_enum)
    }
}

impl BytesRefIterator for TermsEnumBytesRefIterator {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.0.next()
    }
}

/// A [`BytesRefIterator`] over the bytes of a list of terms.
///
/// Equivalent to the anonymous `DisjunctionMatchesIterator.asBytesRefIterator`.
struct TermBytesIterator {
    terms: Vec<BytesRef>,
    i: usize,
}

impl BytesRefIterator for TermBytesIterator {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.i >= self.terms.len() {
            return Ok(None);
        }
        let term = self.terms[self.i].clone();
        self.i += 1;
        Ok(Some(term))
    }
}

/// Creates a [`DisjunctionMatchesIterator`] over the terms produced by a
/// [`BytesRefIterator`].
///
/// Equivalent to
/// `DisjunctionMatchesIterator.fromTermsEnum(LeafReaderContext, int, Query, String, BytesRefIterator)`.
/// Only terms that have at least one match in the given document are included.
///
/// # Errors
///
/// Propagates any I/O error raised while seeking terms or reading postings.
pub fn from_terms_enum(
    context: &LeafReaderContext,
    doc: i32,
    query: Arc<dyn Query>,
    field: &str,
    mut terms: Box<dyn BytesRefIterator>,
) -> Result<Option<Box<dyn MatchesIterator>>> {
    // Equivalent to `Terms.getTerms(context.reader(), field)`, which answers
    // `Terms.EMPTY` when the field is absent.
    let t: Box<dyn Terms> = match context.leaf_reader().terms(field)? {
        Some(t) => t,
        None => Box::new(EmptyTerms),
    };
    let mut te = t.iterator()?;
    let mut reuse: Option<Box<dyn PostingsEnum>> = None;
    while let Some(term) = terms.next()? {
        if te.seek_exact(&term)? {
            let mut pe = te.postings(reuse.take(), POSTINGS_ENUM_OFFSETS)?;
            if pe.advance(doc)? == doc {
                let first = TermMatchesIterator::new(Arc::clone(&query), pe)?;
                return Ok(Some(Box::new(TermsEnumDisjunctionMatchesIterator {
                    first: Some(Box::new(first)),
                    terms,
                    te,
                    doc,
                    query,
                    it: None,
                })));
            }
            reuse = Some(pe);
        }
    }
    Ok(None)
}

/// A [`MatchesIterator`] over a set of terms that only loads the first matching
/// term at construction, waiting until the iterator is actually used before it
/// loads all other matching terms.
///
/// Equivalent to the private
/// `DisjunctionMatchesIterator.TermsEnumDisjunctionMatchesIterator`.
struct TermsEnumDisjunctionMatchesIterator {
    first: Option<Box<dyn MatchesIterator>>,
    terms: Box<dyn BytesRefIterator>,
    te: Box<dyn TermsEnum>,
    doc: i32,
    query: Arc<dyn Query>,
    it: Option<Box<dyn MatchesIterator>>,
}

impl TermsEnumDisjunctionMatchesIterator {
    /// Equivalent to the private `init()`.
    fn init(&mut self) -> Result<()> {
        let mut mis: Vec<Box<dyn MatchesIterator>> = Vec::new();
        if let Some(first) = self.first.take() {
            mis.push(first);
        }
        let mut reuse: Option<Box<dyn PostingsEnum>> = None;
        while let Some(term) = self.terms.next()? {
            if self.te.seek_exact(&term)? {
                let mut pe = self.te.postings(reuse.take(), POSTINGS_ENUM_OFFSETS)?;
                if pe.advance(self.doc)? == self.doc {
                    mis.push(Box::new(TermMatchesIterator::new(
                        Arc::clone(&self.query),
                        pe,
                    )?));
                    reuse = None;
                } else {
                    reuse = Some(pe);
                }
            }
        }
        self.it = from_sub_iterators(mis)?;
        Ok(())
    }

    fn it(&self) -> &dyn MatchesIterator {
        &**self
            .it
            .as_ref()
            .expect("INVARIANT: init() runs on the first next() call")
    }
}

impl MatchesIterator for TermsEnumDisjunctionMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if self.it.is_none() {
            self.init()?;
        }
        match self.it.as_mut() {
            // `fromSubIterators` cannot answer `null` here: the list always
            // holds the first matching term.
            None => Ok(false),
            Some(it) => it.next(),
        }
    }

    fn start_position(&self) -> i32 {
        self.it().start_position()
    }

    fn end_position(&self) -> i32 {
        self.it().end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.it().start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.it().end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.it().get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.it().get_query()
    }
}

/// Combines several [`MatchesIterator`]s into one, or returns `None` when the
/// list is empty.
///
/// Equivalent to
/// `DisjunctionMatchesIterator.fromSubIterators(List<MatchesIterator>)`.
///
/// # Errors
///
/// Propagates any I/O error raised while priming the sub-iterators.
pub fn from_sub_iterators(
    mut mis: Vec<Box<dyn MatchesIterator>>,
) -> Result<Option<Box<dyn MatchesIterator>>> {
    if mis.is_empty() {
        return Ok(None);
    }
    if mis.len() == 1 {
        return Ok(mis.pop());
    }
    Ok(Some(Box::new(DisjunctionMatchesIterator::new(mis)?)))
}

/// A [`MatchesIterator`] that combines matches from a set of sub-iterators.
///
/// Equivalent to `org.apache.lucene.search.DisjunctionMatchesIterator`, which
/// is package-private in Java; it is public here because Rust has no package
/// visibility and the queries that build it live in sibling modules.
///
/// Matches are sorted by their start positions, and then by their end
/// positions, so that prefixes sort first. Matches may overlap, or be
/// duplicated if they appear in more than one of the sub-iterators.
pub struct DisjunctionMatchesIterator {
    queue: MatchesQueue,
    started: bool,
}

impl std::fmt::Debug for DisjunctionMatchesIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisjunctionMatchesIterator")
            .field("size", &self.queue.size)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

impl DisjunctionMatchesIterator {
    /// Builds the iterator from the sub-iterators, discarding those that have
    /// no match.
    ///
    /// Equivalent to the private
    /// `DisjunctionMatchesIterator(List<MatchesIterator>)`; call
    /// [`from_sub_iterators`] instead, which reproduces the static factory.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while priming the sub-iterators.
    pub fn new(matches: Vec<Box<dyn MatchesIterator>>) -> Result<Self> {
        let mut queue = MatchesQueue::new(matches.len());
        for mut mi in matches {
            if mi.next()? {
                queue.add(mi)?;
            }
        }
        Ok(Self {
            queue,
            started: false,
        })
    }

    fn top(&self) -> &dyn MatchesIterator {
        self.queue.top().expect(NON_EMPTY)
    }
}

impl MatchesIterator for DisjunctionMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if !self.started {
            self.started = true;
            return Ok(self.queue.size > 0);
        }
        let top_exhausted = !self.queue.top_mut().expect(NON_EMPTY).next()?;
        if top_exhausted {
            self.queue.pop()?;
        }
        if self.queue.size > 0 {
            self.queue.update_top()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn start_position(&self) -> i32 {
        self.top().start_position()
    }

    fn end_position(&self) -> i32 {
        self.top().end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.top().start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.top().end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.top().get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.top().get_query()
    }
}

/// Orders two match iterators by start position, then end position, falling
/// back to offsets when positions are unavailable.
///
/// Equivalent to the anonymous `PriorityQueue.lessThan` of
/// `DisjunctionMatchesIterator`.
///
/// **Divergence from Lucene 10.5.0.** Java's `lessThan` cannot throw a checked
/// exception, so it catches the `IOException` raised while reading an offset
/// and rethrows it as an unchecked `IllegalArgumentException`. This port
/// returns [`Result`] instead, and the error travels to the caller of
/// [`MatchesIterator::next`] unchanged. The ordering is identical.
fn less_than(a: &dyn MatchesIterator, b: &dyn MatchesIterator) -> Result<bool> {
    if a.start_position() == -1 && b.start_position() == -1 {
        let (a_start, a_end) = (a.start_offset()?, a.end_offset()?);
        let (b_start, b_end) = (b.start_offset()?, b.end_offset()?);
        return Ok(a_start < b_start
            || (a_start == b_start && a_end < b_end)
            || (a_start == b_start && a_end == b_end));
    }
    Ok(a.start_position() < b.start_position()
        || (a.start_position() == b.start_position() && a.end_position() < b.end_position())
        || (a.start_position() == b.start_position() && a.end_position() == b.end_position()))
}

/// A binary heap of [`MatchesIterator`]s ordered by [`less_than`].
///
/// Equivalent to the `PriorityQueue<MatchesIterator>` field of
/// `DisjunctionMatchesIterator`; `up_heap` and `down_heap` reproduce
/// `org.apache.lucene.util.PriorityQueue` exactly, including the unused slot at
/// index 0.
///
/// **Divergence from Lucene 10.5.0.** This is a local heap rather than
/// [`crate::util::PriorityQueue`], for two reasons that are both consequences
/// of Rust's rules rather than of a different algorithm: the queue's elements
/// must be *mutated* through `top()` — `queue.top().next()` — which the shared
/// queue cannot express because its `top()` hands out a shared reference; and
/// this comparator is fallible, while [`crate::util::PriorityQueueComparator`]
/// is not. The ordering, the heap layout and the operation sequence are
/// Lucene's.
struct MatchesQueue {
    heap: Vec<Option<Box<dyn MatchesIterator>>>,
    size: usize,
}

impl MatchesQueue {
    fn new(max_size: usize) -> Self {
        let heap_size = if max_size == 0 { 2 } else { max_size + 1 };
        Self {
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
        }
    }

    fn top(&self) -> Option<&dyn MatchesIterator> {
        self.heap[1].as_deref()
    }

    fn top_mut(&mut self) -> Option<&mut Box<dyn MatchesIterator>> {
        self.heap[1].as_mut()
    }

    fn add(&mut self, element: Box<dyn MatchesIterator>) -> Result<()> {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(index)?;
        Ok(())
    }

    fn pop(&mut self) -> Result<Option<Box<dyn MatchesIterator>>> {
        if self.size == 0 {
            return Ok(None);
        }
        let result = self.heap[1].take();
        if self.size > 1 {
            self.heap[1] = self.heap[self.size].take();
            self.size -= 1;
            self.down_heap(1)?;
        } else {
            self.size = 0;
        }
        Ok(result)
    }

    fn update_top(&mut self) -> Result<()> {
        self.down_heap(1)
    }

    fn up_heap(&mut self, orig_pos: usize) -> Result<()> {
        let mut i = orig_pos;
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: up_heap starts from an occupied slot");
        let mut j = i >> 1;
        while j > 0
            && less_than(
                &*node,
                &**self.heap[j]
                    .as_ref()
                    .expect("INVARIANT: slots 1..=size are occupied"),
            )?
        {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
        Ok(())
    }

    fn down_heap(&mut self, mut i: usize) -> Result<()> {
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: down_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && less_than(
                &**self.heap[k].as_ref().expect(occupied),
                &**self.heap[j].as_ref().expect(occupied),
            )?
        {
            j = k;
        }
        while j <= self.size && less_than(&**self.heap[j].as_ref().expect(occupied), &*node)? {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && less_than(
                    &**self.heap[k].as_ref().expect(occupied),
                    &**self.heap[j].as_ref().expect(occupied),
                )?
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
        Ok(())
    }
}
