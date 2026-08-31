//! Terms dictionary abstractions ported from `org.apache.lucene.index`.
//!
//! This module provides [`Term`], [`Fields`], [`Terms`], [`TermsEnum`] and
//! [`TermState`], the core types used by postings producers and consumers to
//! enumerate terms and retrieve their associated postings.

#![deny(unsafe_code)]

use std::any::Any;
use std::cmp::Ordering;
use std::fmt;

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::postings_enum::{ImpactsEnum, PostingsEnum};
use crate::index::reader_context::{IndexReaderContext, LeafReaderContext};
use crate::search::index_searcher::IndexSearcher;
use crate::store::{ByteArrayDataInput, ByteArrayDataOutput, DataInput, DataOutput};
use crate::util::attribute::AttributeSource;
use crate::util::automaton::CompiledAutomaton;
use crate::util::string_helper::StringHelper;
use crate::util::BytesRef;

// -----------------------------------------------------------------------------
// Term
// -----------------------------------------------------------------------------

/// A search unit: a field name paired with term bytes.
///
/// Equivalent to `org.apache.lucene.index.Term`.
#[derive(Debug, Clone)]
pub struct Term {
    /// Field this term belongs to.
    field: String,
    /// Term bytes (should not be modified in place).
    bytes: BytesRef,
}

impl Term {
    /// Creates a term from a field and a byte sequence.
    pub fn new(field: impl Into<String>, bytes: BytesRef) -> Self {
        Self {
            field: field.into(),
            bytes,
        }
    }

    /// Creates a term from a field and UTF-8 text.
    pub fn from_text(field: impl Into<String>, text: &str) -> Self {
        Self::new(field, BytesRef::new(text.as_bytes().to_vec()))
    }

    /// Creates a term with the given field and empty bytes.
    pub fn empty(field: impl Into<String>) -> Self {
        Self::new(field, BytesRef::new(Vec::new()))
    }

    /// Returns the field name.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the term bytes.
    pub fn bytes(&self) -> &BytesRef {
        &self.bytes
    }

    /// Returns the term text, best-effort UTF-8 decoding.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(self.bytes.slice()).into_owned()
    }

    /// Replaces the field and bytes in place (bytes are not copied).
    ///
    /// Equivalent to the Java `Term.set`.
    pub fn set(&mut self, field: impl Into<String>, bytes: BytesRef) {
        self.field = field.into();
        self.bytes = bytes;
    }
}

impl PartialEq for Term {
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field && self.bytes == other.bytes
    }
}

impl Eq for Term {}

impl PartialOrd for Term {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Term {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.field.cmp(&other.field) {
            Ordering::Equal => self.bytes.cmp(&other.bytes),
            ord => ord,
        }
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.field, self.text())
    }
}

/// Internal state that allows re-positioning a [`TermsEnum`] without re-seeking
/// the term dictionary.
///
/// Equivalent to `org.apache.lucene.index.TermState`.
pub trait TermState: Send + Sync + std::fmt::Debug {
    /// Copies the content of `other` into `self`.
    fn copy_from(&mut self, other: &dyn TermState);

    /// Returns a boxed clone of this state.
    fn clone_box(&self) -> Box<dyn TermState>;

    /// Returns this state as `Any` for downcasting.
    fn as_any(&self) -> &dyn Any;
}

/// Ordinal-based term state.
///
/// Equivalent to `org.apache.lucene.index.OrdTermState`.
#[derive(Debug, Default, Clone, Copy)]
pub struct OrdTermState {
    /// Term ordinal in the full sorted term list.
    pub ord: i64,
}

impl OrdTermState {
    /// Creates a new `OrdTermState`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TermState for OrdTermState {
    fn copy_from(&mut self, other: &dyn TermState) {
        let other = other
            .as_any()
            .downcast_ref::<OrdTermState>()
            .expect("cannot copy from a different TermState type");
        self.ord = other.ord;
    }

    fn clone_box(&self) -> Box<dyn TermState> {
        Box::new(*self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Result of a [`TermsEnum::seek_ceil`] call.
///
/// Equivalent to `TermsEnum.SeekStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum SeekStatus {
    /// The precise term was found.
    FOUND,
    /// A different term was found after the requested term.
    NOT_FOUND,
    /// The term was not found and the end of iteration was hit.
    END,
}

/// Iterator over the terms of a single field.
///
/// Equivalent to `org.apache.lucene.index.TermsEnum`. The default
/// implementations of [`seek_exact`](Self::seek_exact),
/// [`seek_term_state`](Self::seek_term_state) and
/// [`term_state`](Self::term_state) mirror `BaseTermsEnum`.
pub trait TermsEnum {
    /// Returns the attribute source attached to this enum.
    fn attributes(&mut self) -> &mut AttributeSource;

    /// Attempts to seek to the exact term, returning `true` if found.
    ///
    /// Default implementation uses `seek_ceil`.
    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        Ok(self.seek_ceil(text)? == SeekStatus::FOUND)
    }

    /// Seeks to the ceiling term and reports the result.
    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus>;

    /// Seeks to the term at the given ordinal position.
    fn seek_ord(&mut self, ord: i64) -> Result<()>;

    /// Seeks to a previously captured [`TermState`].
    ///
    /// Default implementation seeks the exact term and then applies the state.
    fn seek_term_state(&mut self, text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        if !self.seek_exact(text)? {
            return Err(LuceneError::IllegalArgument(format!(
                "term={text:?} does not exist"
            )));
        }
        Ok(())
    }

    /// Returns the current term.
    fn term(&self) -> Result<BytesRef>;

    /// Returns the ordinal position of the current term, if supported.
    fn ord(&self) -> Result<i64>;

    /// Returns the number of documents containing the current term.
    fn doc_freq(&self) -> Result<i32>;

    /// Returns the total number of occurrences of the current term.
    fn total_term_freq(&self) -> Result<i64>;

    /// Returns a postings enum for the current term with the given flags.
    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>>;

    /// Returns an impacts enum for the current term with the given flags.
    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>>;

    /// Captures the current enum state.
    ///
    /// Default implementation returns an empty term state.
    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Ok(Box::new(EmptyTermState))
    }

    /// Returns `true` if this enum prefers exact seeks.
    fn prefer_seek_exact(&self) -> bool {
        false
    }

    /// Returns the next term, or `None` at the end of iteration.
    fn next(&mut self) -> Result<Option<BytesRef>>;
}

/// Acceptance result used by [`FilteredTermsEnum`].
///
/// Equivalent to `org.apache.lucene.index.FilteredTermsEnum.AcceptStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptStatus {
    /// Accept the term and continue iterating normally.
    Yes,
    /// Accept the term and seek to the next candidate.
    YesAndSeek,
    /// Reject the term and continue iterating normally.
    No,
    /// Reject the term and seek to the next candidate.
    NoAndSeek,
    /// Stop enumerating.
    End,
}

/// A filter implementation supplied to [`FilteredTermsEnum`].
///
/// Equivalent to a subclass of `org.apache.lucene.index.FilteredTermsEnum`.
pub trait FilteredTermsEnumFilter: Send {
    /// Decides whether a term is accepted, rejected, or ends the iteration.
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus>;

    /// Returns the next candidate term to seek to, or `None` if iteration
    /// should end. The default implementation returns the initial seek term
    /// once and then `None`.
    fn next_seek_term(&mut self, _current_term: Option<&BytesRef>) -> Result<Option<BytesRef>> {
        Ok(None)
    }

    /// Optionally sets the initial seek term. Filters that override
    /// [`next_seek_term`](Self::next_seek_term) do not need this.
    fn set_initial_seek_term(&mut self, _term: BytesRef) {}
}

/// Terms iterator that wraps another [`TermsEnum`] and filters terms.
///
/// Equivalent to `org.apache.lucene.index.FilteredTermsEnum`.
pub struct FilteredTermsEnum {
    tenum: Box<dyn TermsEnum>,
    filter: Box<dyn FilteredTermsEnumFilter>,
    initial_seek_term: Option<BytesRef>,
    do_seek: bool,
    actual_term: Option<BytesRef>,
}

impl FilteredTermsEnum {
    /// Creates a filtered enum that starts by seeking the initial term.
    /// Creates a filtered enum that iterates linearly over the delegate.
    pub fn new(tenum: Box<dyn TermsEnum>, filter: Box<dyn FilteredTermsEnumFilter>) -> Self {
        Self {
            tenum,
            filter,
            initial_seek_term: None,
            do_seek: false,
            actual_term: None,
        }
    }

    /// Creates a filtered enum that first seeks to `initial_seek_term`.
    pub fn new_with_seek(
        tenum: Box<dyn TermsEnum>,
        filter: Box<dyn FilteredTermsEnumFilter>,
        initial_seek_term: BytesRef,
    ) -> Self {
        Self {
            tenum,
            filter,
            initial_seek_term: Some(initial_seek_term),
            do_seek: true,
            actual_term: None,
        }
    }
}

impl TermsEnum for FilteredTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.tenum.attributes()
    }

    fn term(&self) -> Result<BytesRef> {
        self.tenum.term()
    }

    fn doc_freq(&self) -> Result<i32> {
        self.tenum.doc_freq()
    }

    fn total_term_freq(&self) -> Result<i64> {
        self.tenum.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.tenum.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.tenum.impacts(flags)
    }

    fn ord(&self) -> Result<i64> {
        self.tenum.ord()
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.tenum.term_state()
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Err(LuceneError::IllegalState(
            "FilteredTermsEnum does not support seeking".to_string(),
        ))
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Err(LuceneError::IllegalState(
            "FilteredTermsEnum does not support seeking".to_string(),
        ))
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::IllegalState(
            "FilteredTermsEnum does not support seeking".to_string(),
        ))
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(LuceneError::IllegalState(
            "FilteredTermsEnum does not support seeking".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        loop {
            if self.do_seek {
                self.do_seek = false;
                let seek_term = if self.initial_seek_term.is_some() {
                    self.initial_seek_term.take()
                } else {
                    self.filter.next_seek_term(self.actual_term.as_ref())?
                };
                if let Some(t) = &seek_term {
                    if let Some(ref actual) = self.actual_term {
                        assert!(
                            t.slice() > actual.slice(),
                            "seek term must be greater than the current term"
                        );
                    }
                }
                if seek_term.is_none()
                    || self.tenum.seek_ceil(seek_term.as_ref().unwrap())? == SeekStatus::END
                {
                    return Ok(None);
                }
                self.actual_term = Some(self.tenum.term()?);
            } else {
                self.actual_term = self.tenum.next()?;
                if self.actual_term.is_none() {
                    return Ok(None);
                }
            }

            match self.filter.accept(self.actual_term.as_ref().unwrap())? {
                AcceptStatus::YesAndSeek => {
                    self.do_seek = true;
                    return Ok(self.actual_term.clone());
                }
                AcceptStatus::Yes => return Ok(self.actual_term.clone()),
                AcceptStatus::NoAndSeek => {
                    self.do_seek = true;
                }
                AcceptStatus::End => return Ok(None),
                AcceptStatus::No => {}
            }
        }
    }
}

/// Filtered terms enum that exposes exactly one term.
///
/// Equivalent to `org.apache.lucene.index.SingleTermsEnum`.
pub struct SingleTermsEnum {
    single_ref: BytesRef,
}

impl SingleTermsEnum {
    /// Creates a single-term enum over the supplied delegate.
    ///
    /// After construction the enum is already positioned at the term if it
    /// exists; the first call to [`TermsEnum::next`] will advance past it.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(tenum: Box<dyn TermsEnum>, term_text: BytesRef) -> Box<dyn TermsEnum> {
        let filter = Box::new(SingleTermsEnum {
            single_ref: term_text.clone(),
        });
        Box::new(FilteredTermsEnum::new_with_seek(tenum, filter, term_text))
    }
}

impl FilteredTermsEnumFilter for SingleTermsEnum {
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
        if term == &self.single_ref {
            Ok(AcceptStatus::Yes)
        } else {
            Ok(AcceptStatus::End)
        }
    }
}

/// Empty term state returned by the default `TermsEnum::term_state`.
#[derive(Debug)]
struct EmptyTermState;

impl TermState for EmptyTermState {
    fn copy_from(&mut self, _other: &dyn TermState) {
        panic!("copy_from not supported by the default TermState");
    }

    fn clone_box(&self) -> Box<dyn TermState> {
        Box::new(EmptyTermState)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Access to the terms of a specific field.
///
/// Equivalent to `org.apache.lucene.index.Terms`.
pub trait Terms {
    /// Returns an iterator over all terms in this field.
    fn iterator(&self) -> Result<Box<dyn TermsEnum>>;

    /// Returns a terms enum restricted to terms accepted by the automaton,
    /// starting after `start_term`.
    fn intersect(
        &self,
        _compiled: &CompiledAutomaton,
        _start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "intersect not implemented".to_string(),
        ))
    }

    /// Returns the number of terms in this field, or `-1` if unknown.
    fn size(&self) -> i64;

    /// Returns the sum of `totalTermFreq` for all terms in this field.
    fn sum_total_term_freq(&self) -> i64;

    /// Returns the sum of `docFreq` for all terms in this field.
    fn sum_doc_freq(&self) -> i64;

    /// Returns the number of documents with at least one term for this field.
    fn doc_count(&self) -> i32;

    /// Returns `true` if this field stores term frequencies.
    fn has_freqs(&self) -> bool;

    /// Returns `true` if this field stores offsets.
    fn has_offsets(&self) -> bool;

    /// Returns `true` if this field stores positions.
    fn has_positions(&self) -> bool;

    /// Returns `true` if this field stores payloads.
    fn has_payloads(&self) -> bool;

    /// Returns the smallest term in this field, or `None` if empty.
    fn min(&self) -> Result<Option<BytesRef>> {
        let mut it = self.iterator()?;
        it.next()
    }

    /// Returns the largest term in this field, or `None` if empty.
    fn max(&self) -> Result<Option<BytesRef>> {
        let size = self.size();
        if size == 0 {
            return Ok(None);
        }
        if size > 0 {
            let mut it = self.iterator()?;
            if it.seek_ord(size - 1).is_ok() {
                return Ok(Some(it.term()?));
            }
        }
        // Fallback: binary-search for the last term one byte at a time.
        let mut it = self.iterator()?;
        if it.next()?.is_none() {
            return Ok(None);
        }
        let mut scratch = vec![0u8];
        loop {
            let mut low = 0u8;
            let mut high = 0u8.wrapping_sub(1);
            while low != high {
                let mid = ((low as u16 + high as u16) / 2) as u8;
                *scratch.last_mut().unwrap() = mid;
                match it.seek_ceil(&BytesRef::new(scratch.clone()))? {
                    SeekStatus::END => {
                        if mid == 0 {
                            scratch.pop();
                            return Ok(Some(BytesRef::new(scratch)));
                        }
                        high = mid;
                    }
                    _ => {
                        if low == mid {
                            break;
                        }
                        low = mid;
                    }
                }
            }
            scratch.push(0);
            let _ = it.term()?;
        }
    }

    /// Returns additional debug statistics about this terms instance.
    fn stats(&self) -> String {
        format!(
            "impl=Terms,size={},docCount={},sumTotalTermFreq={},sumDocFreq={}",
            self.size(),
            self.doc_count(),
            self.sum_total_term_freq(),
            self.sum_doc_freq()
        )
    }
}

/// Collection of per-field [`Terms`] instances.
///
/// Equivalent to `org.apache.lucene.index.Fields`.
pub trait Fields {
    /// Returns an iterator over all field names.
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_>;

    /// Returns the [`Terms`] for the given field, or `None` if absent.
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>>;

    /// Returns the number of fields, or `-1` if unknown.
    fn size(&self) -> i32;
}

// -----------------------------------------------------------------------------
// PrefixCodedTerms
// -----------------------------------------------------------------------------

/// A compact, prefix-shared encoding of a sorted term list.
///
/// Equivalent to `org.apache.lucene.index.PrefixCodedTerms`.
#[derive(Debug, Clone)]
pub struct PrefixCodedTerms {
    content: Vec<u8>,
    size: i64,
    del_gen: i64,
}

impl PrefixCodedTerms {
    /// Returns the number of encoded terms.
    pub fn size(&self) -> i64 {
        self.size
    }

    /// Returns an iterator over the encoded terms.
    pub fn iterator(&self) -> PrefixCodedTermsIterator {
        PrefixCodedTermsIterator::new(self.del_gen, ByteArrayDataInput::new(self.content.clone()))
    }

    /// Records the deletion generation for this packet.
    pub fn set_del_gen(&mut self, del_gen: i64) {
        self.del_gen = del_gen;
    }
}

/// Builder for [`PrefixCodedTerms`].
///
/// Equivalent to `org.apache.lucene.index.PrefixCodedTerms.Builder`.
#[derive(Debug)]
pub struct PrefixCodedTermsBuilder {
    output: ByteArrayDataOutput,
    last_term: Term,
    last_term_bytes: Vec<u8>,
    size: i64,
}

impl Default for PrefixCodedTermsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefixCodedTermsBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self {
            output: ByteArrayDataOutput::new(),
            last_term: Term::empty(""),
            last_term_bytes: Vec::new(),
            size: 0,
        }
    }

    /// Adds a term. Terms must be added in strictly ascending order.
    pub fn add(&mut self, term: &Term) -> Result<()> {
        self.add_bytes(term.field(), term.bytes())
    }

    /// Adds a term from its field and bytes.
    pub fn add_bytes(&mut self, field: &str, bytes: &BytesRef) -> Result<()> {
        assert!(
            self.size == 0 || {
                let candidate = Term::new(field, bytes.clone());
                candidate > self.last_term
            },
            "terms must be added in ascending order"
        );

        let bytes_slice = bytes.slice();
        let same_field = self.size > 0 && field == self.last_term.field();
        let prefix = if same_field {
            StringHelper::bytes_difference(
                &BytesRef::new(self.last_term_bytes.clone()),
                &BytesRef::new(bytes_slice.to_vec()),
            )? as usize
        } else {
            0
        };

        let code = (prefix << 1) | if same_field { 0 } else { 1 };
        self.output.write_v_int(code as i32)?;
        if !same_field {
            self.output.write_string(field)?;
        }
        let suffix = bytes_slice.len() - prefix;
        self.output.write_v_int(suffix as i32)?;
        self.output.write_bytes(bytes_slice, prefix, suffix)?;

        self.last_term_bytes = bytes_slice.to_vec();
        self.last_term = Term::new(field.to_string(), bytes.clone());
        self.size += 1;
        Ok(())
    }

    /// Finalizes the builder into a `PrefixCodedTerms`.
    pub fn finish(self) -> PrefixCodedTerms {
        PrefixCodedTerms {
            content: self.output.into_inner(),
            size: self.size,
            del_gen: -1,
        }
    }
}

/// Iterator over a [`PrefixCodedTerms`] payload.
///
/// Equivalent to `org.apache.lucene.index.PrefixCodedTerms.TermIterator`.
#[derive(Debug)]
pub struct PrefixCodedTermsIterator {
    input: ByteArrayDataInput,
    end: i64,
    del_gen: i64,
    field: String,
    bytes: Vec<u8>,
}

impl PrefixCodedTermsIterator {
    fn new(del_gen: i64, input: ByteArrayDataInput) -> Self {
        let end = input.length() as i64;
        Self {
            input,
            end,
            del_gen,
            field: String::new(),
            bytes: Vec::new(),
        }
    }

    /// Returns the deletion generation for this iterator.
    pub fn del_gen(&self) -> i64 {
        self.del_gen
    }

    /// Returns the current field. Use `==` to detect field changes.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the current term bytes, or `None` when exhausted.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.input.position() as i64 >= self.end {
            self.field = String::new();
            return Ok(None);
        }

        let code = self.input.read_v_int()? as usize;
        let new_field = (code & 1) != 0;
        let prefix = code >> 1;

        if new_field {
            self.field = self.input.read_string()?;
        }

        let suffix = self.input.read_v_int()? as usize;
        self.bytes.resize(prefix + suffix, 0);
        self.input.read_bytes(&mut self.bytes, prefix, suffix)?;
        self.bytes.truncate(prefix + suffix);

        Ok(Some(BytesRef::new(self.bytes.clone())))
    }
}

// -----------------------------------------------------------------------------
// TermStates
// -----------------------------------------------------------------------------

/// **Placement note.** Lucene declares this type in
/// `org.apache.lucene.index`, and its `build(IndexSearcher, Term, boolean)`
/// factory calls into `org.apache.lucene.search`. This port keeps both: the
/// type lives here, in its Lucene package, and reaches into
/// [`crate::search`](crate::search) for the searcher, which is the same
/// dependency Java's `index` package has on `search`.
/// Maintains a [`TermState`] view over the leaves of an index reader, for a
/// single term.
///
/// Equivalent to the `final org.apache.lucene.index.TermStates`. It does not
/// track whether the given [`TermState`] objects are valid, nor whether they
/// refer to the same term in the associated readers.
#[derive(Debug)]
pub struct TermStates {
    /// Important: do **not** keep hard references to index readers. Java stores
    /// `context.identity`, an `Object` used only for reference comparison; this
    /// port stores [`IndexReaderContext::id`], which is the same identity value
    /// and likewise does not reference the reader.
    top_reader_context_identity: usize,
    states: Vec<Option<Box<dyn TermState>>>,
    /// `None` if the statistics are to be used.
    term: Option<Term>,
    doc_freq: i32,
    total_term_freq: i64,
}

impl TermStates {
    fn with_term(term: Option<Term>, context: &Arc<dyn IndexReaderContext>) -> Result<Self> {
        if !context.is_top_level() {
            return Err(LuceneError::IllegalArgument(
                "TermStates must be built from a top-level IndexReaderContext".to_string(),
            ));
        }
        let num_leaves = Arc::clone(context).leaves().len();
        Ok(Self {
            top_reader_context_identity: context.id(),
            states: (0..num_leaves).map(|_| None).collect(),
            term,
            doc_freq: 0,
            total_term_freq: 0,
        })
    }

    /// Creates an empty `TermStates` from an [`IndexReaderContext`].
    ///
    /// Equivalent to `TermStates(IndexReaderContext)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `context` is not a
    /// top-level context, which is the condition Java asserts.
    pub fn new(context: &Arc<dyn IndexReaderContext>) -> Result<Self> {
        Self::with_term(None, context)
    }

    /// Creates a `TermStates` holding an initial [`TermState`].
    ///
    /// Equivalent to
    /// `TermStates(IndexReaderContext, TermState, int, int, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `context` is not a
    /// top-level context.
    pub fn with_state(
        context: &Arc<dyn IndexReaderContext>,
        state: Box<dyn TermState>,
        ord: usize,
        doc_freq: i32,
        total_term_freq: i64,
    ) -> Result<Self> {
        let mut states = Self::with_term(None, context)?;
        states.register(state, ord, doc_freq, total_term_freq);
        Ok(states)
    }

    /// Returns whether this `TermStates` was built for the given
    /// [`IndexReaderContext`].
    ///
    /// Equivalent to `TermStates.wasBuiltFor(IndexReaderContext)`, which
    /// compares the stored identity by reference.
    pub fn was_built_for(&self, context: &Arc<dyn IndexReaderContext>) -> bool {
        self.top_reader_context_identity == context.id()
    }

    /// Builds a `TermStates` for `term` over every leaf of the searcher's
    /// top-level context, registering the leaves that contain it.
    ///
    /// Equivalent to `TermStates.build(IndexSearcher, Term, boolean)`.
    ///
    /// `needs_stats` selects whether the term statistics are collected; when it
    /// is `false`, [`doc_freq`](Self::doc_freq) and
    /// [`total_term_freq`](Self::total_term_freq) refuse to answer, exactly as
    /// in Java.
    ///
    /// **Divergence from Lucene 10.5.0.** Java visits every leaf up front only
    /// when `needsStats` is `true`; otherwise it defers the term-dictionary
    /// seek to the first [`get`](Self::get) for that leaf, and it schedules the
    /// seeks in the background through `TermsEnum.prepareSeekExact`, which
    /// returns an `IOBooleanSupplier`. This port visits every leaf up front in
    /// both cases, because `prepareSeekExact` and `IOBooleanSupplier` are not
    /// ported, and because the deferred path mutates the state array from
    /// `&self` — which Rust would need interior mutability for, since a
    /// [`Weight`](crate::search::Weight) is `Sync`. The states registered, the
    /// statistics accumulated and every value [`get`](Self::get) answers are
    /// identical; only the moment the term-dictionary seek happens differs, and
    /// leaves that end up not being scored are now seeked eagerly.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while seeking the term dictionaries.
    pub fn build(searcher: &IndexSearcher, term: &Term, needs_stats: bool) -> Result<Self> {
        let context = searcher.get_top_reader_context();
        let mut per_reader_term_state = Self::with_term(
            if needs_stats {
                None
            } else {
                Some(term.clone())
            },
            context,
        )?;
        for ctx in searcher.get_leaf_contexts() {
            // `Terms.getTerms(ctx.reader(), term.field())` answers `Terms.EMPTY`
            // when the field is absent.
            let terms: Box<dyn Terms> = match ctx.leaf_reader().terms(term.field())? {
                Some(terms) => terms,
                None => Box::new(EmptyTerms),
            };
            let mut terms_enum = terms.iterator()?;
            if terms_enum.seek_exact(term.bytes())? {
                let ord = ctx.ord() as usize;
                let state = terms_enum.term_state()?;
                if needs_stats {
                    per_reader_term_state.register(
                        state,
                        ord,
                        terms_enum.doc_freq()?,
                        terms_enum.total_term_freq()?,
                    );
                } else {
                    per_reader_term_state.register_state(state, ord);
                }
            }
        }
        Ok(per_reader_term_state)
    }

    /// Clears the internal state and removes all registered [`TermState`]s.
    ///
    /// Equivalent to `TermStates.clear()`.
    pub fn clear(&mut self) {
        self.doc_freq = 0;
        self.total_term_freq = 0;
        for state in &mut self.states {
            *state = None;
        }
    }

    /// Registers a [`TermState`] for a leaf ordinal and accumulates its
    /// statistics.
    ///
    /// Equivalent to `TermStates.register(TermState, int, int, long)`.
    ///
    /// # Panics
    ///
    /// In debug builds, panics when `ord` is out of range or already carries a
    /// state; Java asserts the same two conditions.
    pub fn register(
        &mut self,
        state: Box<dyn TermState>,
        ord: usize,
        doc_freq: i32,
        total_term_freq: i64,
    ) {
        self.register_state(state, ord);
        self.accumulate_statistics(doc_freq, total_term_freq);
    }

    /// Registers a [`TermState`] for a leaf ordinal without updating the term
    /// statistics.
    ///
    /// Equivalent to the expert `TermStates.register(TermState, int)`.
    ///
    /// # Panics
    ///
    /// In debug builds, panics when `ord` is out of range or already carries a
    /// state; Java asserts the same two conditions.
    pub fn register_state(&mut self, state: Box<dyn TermState>, ord: usize) {
        debug_assert!(ord < self.states.len(), "ord {ord} is out of range");
        debug_assert!(
            self.states[ord].is_none(),
            "state for ord: {ord} already registered"
        );
        self.states[ord] = Some(state);
    }

    /// Accumulates term statistics.
    ///
    /// Equivalent to the expert
    /// `TermStates.accumulateStatistics(int, long)`.
    ///
    /// # Panics
    ///
    /// In debug builds, panics when either statistic is negative or when
    /// `doc_freq` exceeds `total_term_freq`; Java asserts the same.
    pub fn accumulate_statistics(&mut self, doc_freq: i32, total_term_freq: i64) {
        debug_assert!(doc_freq >= 0);
        debug_assert!(total_term_freq >= 0);
        debug_assert!(i64::from(doc_freq) <= total_term_freq);
        self.doc_freq += doc_freq;
        self.total_term_freq += total_term_freq;
    }

    /// Returns the [`TermState`] registered for the given leaf, or `None` when
    /// the term does not exist in it.
    ///
    /// Equivalent to `TermStates.get(LeafReaderContext)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns an
    /// `IOSupplier<TermState>` so that the term-dictionary I/O of several terms
    /// can be scheduled together and resolved later. Every state is resolved by
    /// the time [`build`](Self::build) returns here, so there is nothing left to
    /// defer and the state is returned directly. It is cloned because Java
    /// hands out the stored reference while Rust cannot let one escape the
    /// borrow; [`TermState::clone_box`] is the deep copy `TermState.copyFrom`
    /// performs.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the leaf ordinal is out of
    /// range, which is the condition Java asserts.
    pub fn get(&self, ctx: &LeafReaderContext) -> Result<Option<Box<dyn TermState>>> {
        let ord = ctx.ord();
        if ord < 0 || ord as usize >= self.states.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "leaf ordinal {ord} is out of range for a TermStates over {} leaves",
                self.states.len()
            )));
        }
        Ok(self.states[ord as usize]
            .as_ref()
            .map(|state| state.clone_box()))
    }

    /// Returns the accumulated document frequency of every registered
    /// [`TermState`].
    ///
    /// Equivalent to `TermStates.docFreq()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when this instance was built with
    /// `needs_stats = false`, which is the `IllegalStateException` Java throws.
    pub fn doc_freq(&self) -> Result<i32> {
        if self.term.is_some() {
            return Err(LuceneError::IllegalState(
                "Cannot call docFreq() when needsStats=false".to_string(),
            ));
        }
        Ok(self.doc_freq)
    }

    /// Returns the accumulated total term frequency of every registered
    /// [`TermState`].
    ///
    /// Equivalent to `TermStates.totalTermFreq()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when this instance was built with
    /// `needs_stats = false`, which is the `IllegalStateException` Java throws.
    pub fn total_term_freq(&self) -> Result<i64> {
        if self.term.is_some() {
            return Err(LuceneError::IllegalState(
                "Cannot call totalTermFreq() when needsStats=false".to_string(),
            ));
        }
        Ok(self.total_term_freq)
    }
}

impl std::fmt::Display for TermStates {
    /// Equivalent to `TermStates.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "TermStates")?;
        for state in &self.states {
            writeln!(f, "  state={state:?}")?;
        }
        Ok(())
    }
}

/// An empty [`TermsEnum`].
#[derive(Debug, Clone)]
pub struct EmptyTermsEnum {
    atts: AttributeSource,
}

impl Default for EmptyTermsEnum {
    fn default() -> Self {
        Self::new()
    }
}

impl EmptyTermsEnum {
    /// Creates an empty terms enum.
    pub fn new() -> Self {
        Self {
            atts: AttributeSource::new(),
        }
    }
}

impl TermsEnum for EmptyTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Ok(false)
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Ok(SeekStatus::END)
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Ok(())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(LuceneError::IllegalState(
            "seek_term_state should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn term(&self) -> Result<BytesRef> {
        Err(LuceneError::IllegalState(
            "term should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::IllegalState(
            "ord should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn doc_freq(&self) -> Result<i32> {
        Err(LuceneError::IllegalState(
            "doc_freq should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn total_term_freq(&self) -> Result<i64> {
        Err(LuceneError::IllegalState(
            "total_term_freq should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        Err(LuceneError::IllegalState(
            "postings should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(LuceneError::IllegalState(
            "impacts should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(LuceneError::IllegalState(
            "term_state should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        Ok(None)
    }
}

/// An empty [`Terms`] instance.
#[derive(Debug, Clone, Default)]
pub struct EmptyTerms;

impl EmptyTerms {
    /// Creates an empty terms instance.
    pub fn new() -> Self {
        Self
    }
}

impl Terms for EmptyTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(EmptyTermsEnum::new()))
    }

    fn size(&self) -> i64 {
        0
    }

    fn sum_total_term_freq(&self) -> i64 {
        0
    }

    fn sum_doc_freq(&self) -> i64 {
        0
    }

    fn doc_count(&self) -> i32 {
        0
    }

    fn has_freqs(&self) -> bool {
        false
    }

    fn has_offsets(&self) -> bool {
        false
    }

    fn has_positions(&self) -> bool {
        false
    }

    fn has_payloads(&self) -> bool {
        false
    }
}

/// An empty [`Fields`] instance.
#[derive(Debug, Clone, Default)]
pub struct EmptyFields;

impl EmptyFields {
    /// Creates an empty fields collection.
    pub fn new() -> Self {
        Self
    }
}

impl Fields for EmptyFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(std::iter::empty())
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(None)
    }

    fn size(&self) -> i32 {
        0
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::postings_enum::{EmptyPostingsEnum, POSTINGS_ENUM_FREQS};

    #[test]
    fn empty_terms_enum_returns_no_terms() {
        let mut it = EmptyTermsEnum::new();
        assert_eq!(it.next().unwrap(), None);
        assert_eq!(
            it.seek_ceil(&BytesRef::new(vec![0x61])).unwrap(),
            SeekStatus::END
        );
        assert!(!it.seek_exact(&BytesRef::new(vec![0x61])).unwrap());
    }

    #[test]
    fn empty_terms_reports_zero_counts() {
        let terms = EmptyTerms::new();
        assert_eq!(terms.size(), 0);
        assert_eq!(terms.sum_total_term_freq(), 0);
        assert_eq!(terms.sum_doc_freq(), 0);
        assert_eq!(terms.doc_count(), 0);
        assert!(!terms.has_freqs());
        assert!(!terms.has_positions());
        assert!(!terms.has_offsets());
        assert!(!terms.has_payloads());
    }

    #[test]
    fn empty_fields_has_no_terms() {
        let fields = EmptyFields::new();
        assert_eq!(fields.size(), 0);
        assert!(fields.terms("field").unwrap().is_none());
        assert_eq!(fields.iterator().count(), 0);
    }

    #[test]
    fn empty_terms_enum_postings_is_rejected() {
        let mut it = EmptyTermsEnum::new();
        assert!(
            it.postings(None, POSTINGS_ENUM_FREQS).is_err(),
            "postings should fail on empty enum"
        );
    }

    #[test]
    fn term_field_and_bytes_round_trip() {
        let t = Term::from_text("body", "hello");
        assert_eq!(t.field(), "body");
        assert_eq!(t.text(), "hello");
        assert_eq!(t.bytes().slice(), b"hello");
        assert_eq!(t.to_string(), "body:hello");
    }

    #[test]
    fn term_ordering_matches_lucene() {
        let a = Term::from_text("a", "cat");
        let b = Term::from_text("a", "dog");
        let c = Term::from_text("b", "cat");
        assert!(a < b);
        assert!(a < c);
        assert!(b < c);
    }

    #[test]
    fn term_set_replaces_content() {
        let mut t = Term::from_text("body", "hello");
        t.set("title", BytesRef::new(b"world".to_vec()));
        assert_eq!(t.field(), "title");
        assert_eq!(t.text(), "world");
    }

    #[test]
    fn ord_term_state_copy_from() {
        let mut a = OrdTermState { ord: 5 };
        let b = OrdTermState { ord: 42 };
        a.copy_from(&b);
        assert_eq!(a.ord, 42);
        let cloned_box = b.clone_box();
        let cloned = cloned_box.as_any().downcast_ref::<OrdTermState>().unwrap();
        assert_eq!(cloned.ord, 42);
    }

    #[test]
    fn prefix_coded_terms_round_trip() {
        let mut builder = PrefixCodedTermsBuilder::new();
        builder.add(&Term::from_text("body", "apple")).unwrap();
        builder.add(&Term::from_text("body", "apricot")).unwrap();
        builder.add(&Term::from_text("title", "banana")).unwrap();
        let terms = builder.finish();
        assert_eq!(terms.size(), 3);

        let mut it = terms.iterator();
        assert_eq!(it.next().unwrap().unwrap().slice(), b"apple");
        assert_eq!(it.field(), "body");
        assert_eq!(it.next().unwrap().unwrap().slice(), b"apricot");
        assert_eq!(it.next().unwrap().unwrap().slice(), b"banana");
        assert_eq!(it.field(), "title");
        assert!(it.next().unwrap().is_none());
    }

    #[test]
    fn prefix_coded_terms_preserves_del_gen() {
        let mut builder = PrefixCodedTermsBuilder::new();
        builder.add(&Term::from_text("f", "x")).unwrap();
        let mut terms = builder.finish();
        terms.set_del_gen(7);
        assert_eq!(terms.iterator().del_gen(), 7);
    }

    /// Simple list-backed [`TermsEnum`] for tests.
    struct VecTermsEnum {
        terms: Vec<BytesRef>,
        pos: usize,
        atts: AttributeSource,
    }

    impl VecTermsEnum {
        fn new(terms: Vec<BytesRef>) -> Self {
            Self {
                terms,
                pos: 0,
                atts: AttributeSource::new(),
            }
        }
    }

    impl TermsEnum for VecTermsEnum {
        fn attributes(&mut self) -> &mut AttributeSource {
            &mut self.atts
        }

        fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
            match self.terms.binary_search(text) {
                Ok(i) => {
                    self.pos = i;
                    Ok(true)
                }
                Err(_) => Ok(false),
            }
        }

        fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
            match self.terms.binary_search(text) {
                Ok(i) => {
                    self.pos = i;
                    Ok(SeekStatus::FOUND)
                }
                Err(i) => {
                    self.pos = i;
                    if i >= self.terms.len() {
                        Ok(SeekStatus::END)
                    } else {
                        Ok(SeekStatus::NOT_FOUND)
                    }
                }
            }
        }

        fn seek_ord(&mut self, ord: i64) -> Result<()> {
            self.pos = ord as usize;
            Ok(())
        }

        fn term(&self) -> Result<BytesRef> {
            Ok(self.terms[self.pos].clone())
        }

        fn ord(&self) -> Result<i64> {
            Ok(self.pos as i64)
        }

        fn doc_freq(&self) -> Result<i32> {
            Ok(1)
        }

        fn total_term_freq(&self) -> Result<i64> {
            Ok(1)
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(EmptyPostingsEnum::new()))
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
            Err(LuceneError::IllegalState(
                "impacts not supported in VecTermsEnum".to_string(),
            ))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            if self.pos + 1 >= self.terms.len() {
                self.pos = self.terms.len();
                return Ok(None);
            }
            self.pos += 1;
            Ok(Some(self.terms[self.pos].clone()))
        }
    }

    #[test]
    fn filtered_terms_enum_accepts_subset() {
        struct PrefixFilter {
            prefix: Vec<u8>,
        }
        impl FilteredTermsEnumFilter for PrefixFilter {
            fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
                if term.slice().starts_with(&self.prefix) {
                    Ok(AcceptStatus::Yes)
                } else {
                    Ok(AcceptStatus::No)
                }
            }
            fn next_seek_term(&mut self, _current: Option<&BytesRef>) -> Result<Option<BytesRef>> {
                Ok(None)
            }
        }

        let inner = Box::new(VecTermsEnum::new(vec![
            BytesRef::new(b"a".to_vec()),
            BytesRef::new(b"ab".to_vec()),
            BytesRef::new(b"abc".to_vec()),
            BytesRef::new(b"b".to_vec()),
        ]));
        let filter = Box::new(PrefixFilter {
            prefix: b"ab".to_vec(),
        });
        let mut it = FilteredTermsEnum::new(inner, filter);
        assert_eq!(it.next().unwrap().unwrap().slice(), b"ab");
        assert_eq!(it.next().unwrap().unwrap().slice(), b"abc");
        assert!(it.next().unwrap().is_none());
    }

    #[test]
    fn single_terms_enum_exposes_one_term() {
        let inner = Box::new(VecTermsEnum::new(vec![
            BytesRef::new(b"a".to_vec()),
            BytesRef::new(b"b".to_vec()),
            BytesRef::new(b"c".to_vec()),
        ]));
        let mut it = SingleTermsEnum::new(inner, BytesRef::new(b"b".to_vec()));
        assert_eq!(it.next().unwrap().unwrap().slice(), b"b");
        assert!(it.next().unwrap().is_none());
    }

    #[test]
    fn filtered_terms_enum_rejects_seek() {
        let inner = Box::new(VecTermsEnum::new(vec![]));
        let filter = Box::new(SingleTermsEnum {
            single_ref: BytesRef::new(vec![0]),
        });
        let mut it = FilteredTermsEnum::new(inner, filter);
        assert!(it.seek_exact(&BytesRef::new(vec![0])).is_err());
    }
}
