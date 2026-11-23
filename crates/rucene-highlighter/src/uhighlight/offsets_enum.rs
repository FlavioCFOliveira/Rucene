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

//! Port of `OffsetsEnum.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::any::Any;
use std::cmp::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::{LuceneError, Result};
use rucene::index::{PostingsEnum, Term};
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::BytesRef;

/// An enumeration/iterator of a term and its offsets for use by
/// `FieldHighlighter`. It is advanced and is placed in a priority queue by
/// `FieldHighlighter.highlightOffsetsEnums(OffsetsEnum)` based on the start
/// offset.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.uhighlight.OffsetsEnum`; `compareTo` is
/// [`compare_offsets_enums`] and `toString` [`offsets_enum_to_string`].
pub trait OffsetsEnum {
    /// Advances to the next position and returns true, or if can't then
    /// returns false. Note that the initial state of this class is not
    /// positioned.
    ///
    /// Equivalent to `nextPosition()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn next_position(&mut self) -> Result<bool>;

    /// An estimate of the number of occurrences of this term/OffsetsEnum.
    ///
    /// Equivalent to `freq()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn freq(&self) -> Result<i32>;

    /// The term at this position. This BytesRef is safe to continue to
    /// refer to, even after we move to the next position; `None` for Java's
    /// `null`.
    ///
    /// Equivalent to `getTerm()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn get_term(&self) -> Result<Option<BytesRef>>;

    /// Equivalent to `startOffset()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn start_offset(&self) -> Result<i32>;

    /// Equivalent to `endOffset()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn end_offset(&self) -> Result<i32>;

    /// Equivalent to `close()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    /// `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str;

    /// This enum as `Any`, for Java's `instanceof` and identity tests.
    fn as_any(&self) -> &dyn Any;
}

/// Equivalent to `OffsetsEnum.compareTo(OffsetsEnum)`: by start offset,
/// end offset, then term (a missing term last).
///
/// # Errors
///
/// What reading the offsets or terms raises (Java wraps it in a
/// `RuntimeException`).
pub fn compare_offsets_enums(this: &dyn OffsetsEnum, other: &dyn OffsetsEnum) -> Result<Ordering> {
    let cmp = this.start_offset()?.cmp(&other.start_offset()?);
    if cmp != Ordering::Equal {
        return Ok(cmp); // vast majority of the time we return here.
    }
    let cmp = this.end_offset()?.cmp(&other.end_offset()?);
    if cmp != Ordering::Equal {
        return Ok(cmp);
    }
    let this_term = this.get_term()?;
    let other_term = other.get_term()?;
    match (this_term, other_term) {
        (None, None) => Ok(Ordering::Equal),
        (None, Some(_)) => Ok(Ordering::Greater), // put "this" (wildcard mtq enum) last
        (Some(_), None) => Ok(Ordering::Less),
        (Some(a), Some(b)) => Ok(a.cmp(&b)),
    }
}

/// Equivalent to `OffsetsEnum.toString()`.
pub fn offsets_enum_to_string(this: &dyn OffsetsEnum) -> String {
    let name = this.simple_name();
    let mut offset = String::new();
    if let (Ok(start), Ok(end)) = (this.start_offset(), this.end_offset()) {
        offset = format!(",[{start}-{end}]");
    }
    match this.get_term() {
        Ok(Some(term)) => match term.utf8_to_string() {
            Ok(text) => format!("{name}(term:{text}{offset})"),
            Err(_) => name.to_string(),
        },
        _ => name.to_string(),
    }
}

/// `java.util.PriorityQueue` ordered by a fallible comparator: a binary
/// heap whose `offer` and `poll` sift as the JDK's do, so that equal
/// elements come out in the JDK's order.
pub(crate) struct JavaPriorityQueue<T> {
    queue: Vec<T>,
    comparator: fn(&T, &T) -> Result<Ordering>,
}

impl<T> JavaPriorityQueue<T> {
    pub(crate) fn new(comparator: fn(&T, &T) -> Result<Ordering>) -> Self {
        Self {
            queue: Vec::new(),
            comparator,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub(crate) fn peek(&self) -> Option<&T> {
        self.queue.first()
    }

    pub(crate) fn peek_mut(&mut self) -> Option<&mut T> {
        self.queue.first_mut()
    }

    /// `offer(E)` / `add(E)`: `siftUpUsingComparator`.
    pub(crate) fn offer(&mut self, x: T) -> Result<()> {
        self.queue.push(x);
        let mut k = self.queue.len() - 1;
        while k > 0 {
            let parent = (k - 1) >> 1;
            if (self.comparator)(&self.queue[k], &self.queue[parent])? != Ordering::Less {
                break;
            }
            self.queue.swap(k, parent);
            k = parent;
        }
        Ok(())
    }

    /// `poll()`: `siftDownUsingComparator` of the last element.
    pub(crate) fn poll(&mut self) -> Result<Option<T>> {
        if self.queue.is_empty() {
            return Ok(None);
        }
        let result = self.queue.swap_remove(0);
        let n = self.queue.len();
        let mut k = 0;
        let half = n >> 1;
        while k < half {
            let mut child = (k << 1) + 1;
            let right = child + 1;
            if right < n
                && (self.comparator)(&self.queue[child], &self.queue[right])? == Ordering::Greater
            {
                child = right;
            }
            if (self.comparator)(&self.queue[k], &self.queue[child])? != Ordering::Greater {
                break;
            }
            self.queue.swap(k, child);
            k = child;
        }
        Ok(Some(result))
    }

    /// The elements in array order (`toArray()`).
    pub(crate) fn into_vec(self) -> Vec<T> {
        self.queue
    }

    pub(crate) fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.queue.iter_mut()
    }
}

/// Based on a `PostingsEnum` -- the typical/standard OE impl.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.OffsetsEnum.OfPostings`.
pub struct OfPostings {
    term: BytesRef,
    postings_enum: Box<dyn PostingsEnum>, // with offsets
    freq: i32,

    pos_counter: i32,
}

impl OfPostings {
    /// Equivalent to `new OfPostings(BytesRef, int, PostingsEnum)`.
    ///
    /// # Errors
    ///
    /// What the postings' `freq()` raises.
    pub fn with_freq(
        term: BytesRef,
        freq: i32,
        postings_enum: Box<dyn PostingsEnum>,
    ) -> Result<Self> {
        let pos_counter = postings_enum.freq()?;
        Ok(Self {
            term,
            postings_enum,
            freq,
            pos_counter,
        })
    }

    /// Equivalent to `new OfPostings(BytesRef, PostingsEnum)`.
    ///
    /// # Errors
    ///
    /// What the postings' `freq()` raises.
    pub fn new(term: BytesRef, postings_enum: Box<dyn PostingsEnum>) -> Result<Self> {
        let freq = postings_enum.freq()?;
        Self::with_freq(term, freq, postings_enum)
    }

    /// Equivalent to `getPostingsEnum()`.
    pub fn get_postings_enum(&self) -> &dyn PostingsEnum {
        &*self.postings_enum
    }
}

impl OffsetsEnum for OfPostings {
    fn next_position(&mut self) -> Result<bool> {
        if self.pos_counter > 0 {
            self.pos_counter -= 1;
            self.postings_enum.next_position()?; // note: we don't need to save the position
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn get_term(&self) -> Result<Option<BytesRef>> {
        Ok(Some(self.term.clone()))
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(self.postings_enum.start_offset())
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(self.postings_enum.end_offset())
    }

    fn freq(&self) -> Result<i32> {
        Ok(self.freq)
    }

    fn simple_name(&self) -> &'static str {
        "OfPostings"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The `queryToTermMap` of `OfMatchesIteratorWithSubs`: Java's
/// `HashMap<Query, BytesRef>`, keyed by query equality.
#[derive(Default)]
struct QueryToTermMap {
    entries: Vec<(Arc<dyn Query>, BytesRef)>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The anonymous visitor of `queryToTerm`: the terms joined with spaces.
struct TermJoiner {
    bytes: Vec<u8>,
}

impl QueryVisitor for TermJoiner {
    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        for term in terms {
            if !self.bytes.is_empty() {
                self.bytes.push(b' ');
            }
            self.bytes.extend_from_slice(term.bytes().slice());
        }
    }
}

/// `queryToTerm(Query)`: maps a query to a term, cached per query.
fn query_to_term(map: &Mutex<QueryToTermMap>, query: Option<Arc<dyn Query>>) -> Result<BytesRef> {
    let Some(query) = query else {
        return Err(LuceneError::null_pointer());
    };
    let mut map = lock(map);
    if let Some((_, term)) = map.entries.iter().find(|(q, _)| q.query_eq(&*query)) {
        return Ok(term.clone());
    }
    let mut joiner = TermJoiner { bytes: Vec::new() };
    query.visit(&mut joiner);
    let term = if !joiner.bytes.is_empty() {
        BytesRef::new(joiner.bytes)
    } else {
        BytesRef::new(query.to_query_string("").into_bytes())
    };
    map.entries.push((query, term.clone()));
    Ok(term)
}

/// The term supplier of an [`OfMatchesIterator`]: Java's
/// `Supplier<BytesRef>`, handed the iterator it describes (Java's lambdas
/// capture it).
pub type TermSupplier = Arc<dyn Fn(&dyn MatchesIterator) -> Result<BytesRef> + Send + Sync>;

/// Based on a `MatchesIterator` with submatches.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.OffsetsEnum.OfMatchesIteratorWithSubs`.
pub struct OfMatchesIteratorWithSubs {
    // Either CachedOE impls (which are the submatches) or OfMatchesIterator
    // impls
    pending_queue: JavaPriorityQueue<PendingOE>,
    query_to_term_map: Arc<Mutex<QueryToTermMap>>,
}

/// An element of the pending queue: an [`OfMatchesIterator`] or a
/// [`CachedOE`].
enum PendingOE {
    MatchesIterator(OfMatchesIterator),
    Cached(CachedOE),
}

impl PendingOE {
    fn as_oe(&self) -> &dyn OffsetsEnum {
        match self {
            PendingOE::MatchesIterator(oe) => oe,
            PendingOE::Cached(oe) => oe,
        }
    }
}

fn compare_pending(a: &PendingOE, b: &PendingOE) -> Result<Ordering> {
    compare_offsets_enums(a.as_oe(), b.as_oe())
}

impl OfMatchesIteratorWithSubs {
    /// Equivalent to `new OfMatchesIteratorWithSubs(MatchesIterator)`.
    ///
    /// # Errors
    ///
    /// What comparing the first element raises (none: the queue is empty).
    pub fn new(matches_iterator: Box<dyn MatchesIterator>) -> Result<Self> {
        let query_to_term_map = Arc::new(Mutex::new(QueryToTermMap::default()));
        let map = Arc::clone(&query_to_term_map);
        let supplier: TermSupplier =
            Arc::new(move |mi: &dyn MatchesIterator| query_to_term(&map, mi.get_query()));
        let mut pending_queue = JavaPriorityQueue::new(compare_pending as fn(&_, &_) -> _);
        pending_queue.offer(PendingOE::MatchesIterator(OfMatchesIterator::new(
            matches_iterator,
            supplier,
        )))?;
        Ok(Self {
            pending_queue,
            query_to_term_map,
        })
    }

    /// Handles the head of the queue, an [`OfMatchesIterator`] that stays
    /// enqueued unless it has submatches: `nextWhenMatchesIterator` with
    /// `isHead`.
    fn next_when_matches_iterator_head(&mut self) -> Result<()> {
        let sub_matches = match self.pending_queue.peek() {
            Some(PendingOE::MatchesIterator(mi_oe)) => mi_oe.matches_iterator.get_sub_matches()?,
            _ => return Ok(()),
        };
        if let Some(sub_matches) = sub_matches {
            let Some(PendingOE::MatchesIterator(mut mi_oe)) = self.pending_queue.poll()? else {
                unreachable!("the head was checked above");
            }; // remove
            self.enqueue_cached_matches(Some(sub_matches))?;
            if mi_oe.next_position()? {
                self.pending_queue
                    .offer(PendingOE::MatchesIterator(mi_oe))?;
            }
        } // else has no subMatches.  It will stay enqueued.
        Ok(())
    }

    /// `nextWhenMatchesIterator` of an iterator that is not enqueued.
    fn next_when_matches_iterator_detached(&mut self, mut mi_oe: OfMatchesIterator) -> Result<()> {
        let sub_matches = mi_oe.matches_iterator.get_sub_matches()?;
        if let Some(sub_matches) = sub_matches {
            self.enqueue_cached_matches(Some(sub_matches))?;
            if mi_oe.next_position()? {
                self.pending_queue
                    .offer(PendingOE::MatchesIterator(mi_oe))?;
            }
        } else {
            self.pending_queue
                .offer(PendingOE::MatchesIterator(mi_oe))?;
        }
        Ok(())
    }

    fn enqueue_cached_matches(
        &mut self,
        this_mi: Option<Box<dyn MatchesIterator>>,
    ) -> Result<bool> {
        let Some(mut this_mi) = this_mi else {
            return Ok(false);
        };
        while this_mi.next()? {
            if !self.enqueue_cached_matches(this_mi.get_sub_matches()?)? {
                // recursion
                // if no sub-matches then add ourselves
                let term = query_to_term(&self.query_to_term_map, this_mi.get_query())?;
                self.pending_queue.offer(PendingOE::Cached(CachedOE {
                    term,
                    start_offset: this_mi.start_offset()?,
                    end_offset: this_mi.end_offset()?,
                }))?;
            }
        }
        Ok(true)
    }

    fn head(&self) -> Result<&dyn OffsetsEnum> {
        self.pending_queue
            .peek()
            .map(PendingOE::as_oe)
            .ok_or_else(LuceneError::null_pointer)
    }
}

impl OffsetsEnum for OfMatchesIteratorWithSubs {
    fn next_position(&mut self) -> Result<bool> {
        let former_head_oe = self.pending_queue.poll()?; // removes the head
        match former_head_oe {
            Some(PendingOE::Cached(_)) => {
                // we're done with the former head.  CachedOE's are leaf
                // matches.
                if let Some(PendingOE::MatchesIterator(_)) = self.pending_queue.peek() {
                    self.next_when_matches_iterator_head()?;
                } // else new head is a CacheOE or no more.  Nothing to do with it.
            }
            Some(PendingOE::MatchesIterator(mut mi_oe)) => {
                // formerHeadOE is OfMatchesIterator; advance it
                if mi_oe.next_position()? {
                    self.next_when_matches_iterator_detached(mi_oe)?; // requires processing.  May or may not re-enqueue itself
                }
            }
            None => return Err(LuceneError::null_pointer()),
        }
        Ok(!self.pending_queue.is_empty())
    }

    fn freq(&self) -> Result<i32> {
        self.head()?.freq()
    }

    fn get_term(&self) -> Result<Option<BytesRef>> {
        self.head()?.get_term()
    }

    fn start_offset(&self) -> Result<i32> {
        self.head()?.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.head()?.end_offset()
    }

    fn simple_name(&self) -> &'static str {
        "OfMatchesIteratorWithSubs"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Equivalent to the private nested class
/// `OffsetsEnum.OfMatchesIteratorWithSubs.CachedOE`.
pub(crate) struct CachedOE {
    term: BytesRef,
    start_offset: i32,
    end_offset: i32,
}

impl OffsetsEnum for CachedOE {
    fn next_position(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn freq(&self) -> Result<i32> {
        Ok(1) // documented short-coming of MatchesIterator based UnifiedHighlighter
    }

    fn get_term(&self) -> Result<Option<BytesRef>> {
        Ok(Some(self.term.clone()))
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(self.start_offset)
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(self.end_offset)
    }

    fn simple_name(&self) -> &'static str {
        "CachedOE"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Based on a `MatchesIterator`; does not look at submatches.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.OffsetsEnum.OfMatchesIterator`.
pub struct OfMatchesIterator {
    matches_iterator: Box<dyn MatchesIterator>,
    term_supplier: TermSupplier,
}

impl OfMatchesIterator {
    /// Equivalent to `new OfMatchesIterator(MatchesIterator,
    /// Supplier<BytesRef>)`.
    pub fn new(matches_iterator: Box<dyn MatchesIterator>, term_supplier: TermSupplier) -> Self {
        Self {
            matches_iterator,
            term_supplier,
        }
    }
}

impl OffsetsEnum for OfMatchesIterator {
    fn next_position(&mut self) -> Result<bool> {
        self.matches_iterator.next()
    }

    fn freq(&self) -> Result<i32> {
        Ok(1) // documented short-coming of MatchesIterator based UnifiedHighlighter
    }

    fn get_term(&self) -> Result<Option<BytesRef>> {
        Ok(Some((self.term_supplier)(&*self.matches_iterator)?))
    }

    fn start_offset(&self) -> Result<i32> {
        self.matches_iterator.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.matches_iterator.end_offset()
    }

    fn simple_name(&self) -> &'static str {
        "OfMatchesIterator"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The empty OffsetsEnum.
///
/// Equivalent to the anonymous class of the singleton `OffsetsEnum.EMPTY`:
/// [`is_empty_offsets_enum`] stands for Java's `== OffsetsEnum.EMPTY`.
pub struct EmptyOffsetsEnum {
    _private: (),
}

/// Equivalent to `OffsetsEnum.EMPTY`.
pub fn empty() -> Box<dyn OffsetsEnum> {
    Box::new(EmptyOffsetsEnum { _private: () })
}

/// `offsetsEnum == OffsetsEnum.EMPTY`.
pub fn is_empty_offsets_enum(offsets_enum: &dyn OffsetsEnum) -> bool {
    offsets_enum.as_any().is::<EmptyOffsetsEnum>()
}

impl OffsetsEnum for EmptyOffsetsEnum {
    fn next_position(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn get_term(&self) -> Result<Option<BytesRef>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn start_offset(&self) -> Result<i32> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn end_offset(&self) -> Result<i32> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn freq(&self) -> Result<i32> {
        Ok(0)
    }

    fn simple_name(&self) -> &'static str {
        ""
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A view over several OffsetsEnum instances, merging them in-place.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.OffsetsEnum.MultiOffsetsEnum`.
pub struct MultiOffsetsEnum {
    queue: JavaPriorityQueue<Box<dyn OffsetsEnum>>,
    started: bool,
}

fn compare_boxed(a: &Box<dyn OffsetsEnum>, b: &Box<dyn OffsetsEnum>) -> Result<Ordering> {
    compare_offsets_enums(&**a, &**b)
}

impl MultiOffsetsEnum {
    /// Equivalent to `new MultiOffsetsEnum(List)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn new(inner: Vec<Box<dyn OffsetsEnum>>) -> Result<Self> {
        let mut queue = JavaPriorityQueue::new(compare_boxed as fn(&_, &_) -> _);
        for mut oe in inner {
            if oe.next_position()? {
                queue.offer(oe)?;
            }
        }
        Ok(Self {
            queue,
            started: false,
        })
    }

    fn head(&self) -> Result<&dyn OffsetsEnum> {
        self.queue
            .peek()
            .map(|oe| &**oe)
            .ok_or_else(LuceneError::null_pointer)
    }
}

impl OffsetsEnum for MultiOffsetsEnum {
    fn next_position(&mut self) -> Result<bool> {
        if !self.started {
            self.started = true;
            return Ok(self.queue.len() > 0);
        }
        if self.queue.len() > 0 {
            let mut top = self
                .queue
                .poll()?
                .expect("INVARIANT: the queue is not empty");
            if top.next_position()? {
                self.queue.offer(top)?;
                return Ok(true);
            } else {
                top.close()?;
            }
            return Ok(self.queue.len() > 0);
        }
        Ok(false)
    }

    fn get_term(&self) -> Result<Option<BytesRef>> {
        self.head()?.get_term()
    }

    fn start_offset(&self) -> Result<i32> {
        self.head()?.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.head()?.end_offset()
    }

    fn freq(&self) -> Result<i32> {
        self.head()?.freq()
    }

    fn close(&mut self) -> Result<()> {
        // IOUtils.close(queue): every element closed, the first failure
        // rethrown after the others were tried.
        let mut first: Option<LuceneError> = None;
        for oe in self.queue.iter_mut() {
            if let Err(e) = oe.close() {
                match &mut first {
                    None => first = Some(e),
                    Some(f) => f.add_suppressed(e),
                }
            }
        }
        match first {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn simple_name(&self) -> &'static str {
        "MultiOffsetsEnum"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
