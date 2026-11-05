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

//! Port of `org.apache.lucene.tests.index.AssertingLeafReader`: the reader
//! itself ([`AssertingLeafReader`], `extends FilterLeafReader`), its
//! [`AssertingStoredFields`], [`AssertingTermVectors`], [`AssertingBits`] and
//! `AssertingLiveDocs` (rmp #344), and the inner classes the asserting codecs
//! (`org.apache.lucene.tests.codecs.asserting`) wrap their readers' values in,
//! ported earlier under the sprint's cycle rule: [`AssertingFields`],
//! [`AssertingTerms`], `AssertingTermsEnum`, [`AssertingPostingsEnum`],
//! [`AssertingImpactsEnum`], `AssertingImpacts`, the five doc-values
//! wrappers, [`AssertingDocValuesSkipper`], [`AssertingPointValues`],
//! `AssertingPointTree` and `AssertingIntersectVisitor`.
//!
//! Every Java `assert` is a `java_assert!` (an
//! `Err(AssertionError)`) where the Rust signature returns a `Result`, and a
//! panicking `java.lang.AssertionError` where it cannot fail (`docID()`,
//! `cost()`, `startOffset()`, the skipper's accessors, ...): the unchecked Java
//! error unwinds to the test either way.
//!
//! **Divergence (thread names).** `assertThread` prints the creating and the
//! consuming thread with `Thread.toString()`; the port prints the
//! [`Thread`] objects of `randomizedtesting`'s JDK layer, whose `toString()`
//! has Java's shape (`Thread[#id,name,priority,group]`) but the port's ids.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::{self, ASSERTION_ERROR};
use randomizedtesting::jdk::thread::Thread;
use rucene::error::{LuceneError, Result};
use rucene::index::DocAndFloatFeatureBuffer;
use rucene::index::{
    BinaryDocValues, DocValues, DocValuesIterator, DocValuesSkipper, Fields, FreqAndNormBuffer,
    Impacts, ImpactsEnum, ImpactsSource, IntersectVisitor, NumericDocValues, PointTree,
    PointValues, PostingsEnum, Relation, SeekStatus, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, TermState, Terms, TermsEnum, POSTINGS_ENUM_FREQS,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::CompiledAutomaton;
use rucene::util::io_function::IOBooleanSupplier;
use rucene::util::{BytesRef, FixedBitSet};

/// The message of `assertThread(object, creationThread)`.
fn thread_message(object: &str, creation_thread: &Thread, current: &Thread) -> String {
    format!(
        "{object} are only supposed to be consumed in the thread in which they have been \
         acquired. But was acquired in {creation_thread} and consumed in {current}."
    )
}

/// `AssertingLeafReader.assertThread(object, creationThread)`: Java throws an
/// `AssertionError` (always, not under `-ea` only) when the calling thread is
/// not the one that created the object.
pub(crate) fn assert_thread(object: &str, creation_thread: &Thread) -> Result<()> {
    let current = Thread::current();
    if *creation_thread != current {
        return Err(LuceneError::AssertionError(thread_message(
            object,
            creation_thread,
            &current,
        )));
    }
    Ok(())
}

/// [`assert_thread`] where the Rust signature cannot fail: the
/// `AssertionError` panics.
pub(crate) fn assert_thread_unchecked(object: &str, creation_thread: &Thread) {
    let current = Thread::current();
    if *creation_thread != current {
        exception::throw(
            ASSERTION_ERROR,
            thread_message(object, creation_thread, &current),
        )
    }
}

/// `assert b.isValid();`: under `-ea`, `BytesRef.isValid()` throws its
/// `IllegalStateException` on an invalid reference (and returns `true`
/// otherwise).
pub(crate) fn assert_valid(b: &BytesRef) -> Result<()> {
    if cfg!(debug_assertions) {
        b.is_valid()?;
    }
    Ok(())
}

/// The `NullPointerException` (no message) Java throws where it dereferences
/// a `null` the Rust signature returns as `None`.
pub(crate) fn npe() -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.NullPointerException",
        message: None,
        cause: None,
    }
}

/// `Objects.checkIndex(index, length)`: `IndexOutOfBoundsException` ("Index
/// %d out of bounds for length %d") when `index` is not in `[0, length)`.
fn check_index(index: i32, length: i32) {
    if index < 0 || index >= length {
        exception::throw(
            "java.lang.IndexOutOfBoundsException",
            format!("Index {index} out of bounds for length {length}"),
        )
    }
}

// =============================================================================
// AssertingFields
// =============================================================================

/// Port of `AssertingLeafReader.AssertingFields` (`extends FilterFields`):
/// wraps a [`Fields`] and checks the thread that consumes it and the
/// [`Terms`] it hands out ([`AssertingTerms`]).
pub struct AssertingFields {
    creation_thread: Thread,
    r#in: Box<dyn Fields>,
}

impl AssertingFields {
    /// `new AssertingFields(in)`.
    pub fn new(r#in: Box<dyn Fields>) -> Self {
        AssertingFields {
            creation_thread: Thread::current(),
            r#in,
        }
    }
}

impl Fields for AssertingFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        assert_thread_unchecked("Fields", &self.creation_thread);
        self.r#in.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        assert_thread("Fields", &self.creation_thread)?;
        Ok(match self.r#in.terms(field)? {
            None => None,
            Some(terms) => Some(Box::new(AssertingTerms::new(terms))),
        })
    }

    /// `FilterFields.size()`: `in.size()`.
    fn size(&self) -> i32 {
        self.r#in.size()
    }
}

// =============================================================================
// AssertingTerms
// =============================================================================

/// Port of `AssertingLeafReader.AssertingTerms` (`extends FilterTerms`):
/// wraps a [`Terms`] but with additional asserts.
pub struct AssertingTerms {
    creation_thread: Thread,
    r#in: Box<dyn Terms>,
}

impl AssertingTerms {
    /// `new AssertingTerms(in)`.
    pub fn new(r#in: Box<dyn Terms>) -> Self {
        AssertingTerms {
            creation_thread: Thread::current(),
            r#in,
        }
    }
}

impl fmt::Debug for AssertingTerms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingTerms")
    }
}

impl Terms for AssertingTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        assert_thread("Terms", &self.creation_thread)?;
        let terms_enum = self.r#in.iterator()?;
        Ok(Box::new(AssertingTermsEnum::new(
            terms_enum,
            self.has_freqs(),
        )))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        assert_thread("Terms", &self.creation_thread)?;
        let terms_enum = self.r#in.intersect(compiled, start_term)?;
        if let Some(bytes) = start_term {
            assert_valid(bytes)?;
        }
        Ok(Box::new(AssertingTermsEnum::new(
            terms_enum,
            self.has_freqs(),
        )))
    }

    fn size(&self) -> Result<i64> {
        self.r#in.size()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        assert_thread("Terms", &self.creation_thread)?;
        let sum_ttf = self.r#in.sum_total_term_freq()?;
        if !self.has_freqs() {
            java_assert!(sum_ttf == self.r#in.sum_doc_freq()?);
        }
        java_assert!(sum_ttf >= self.sum_doc_freq()?);
        Ok(sum_ttf)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        assert_thread("Terms", &self.creation_thread)?;
        let sum_df = self.r#in.sum_doc_freq()?;
        java_assert!(sum_df >= i64::from(self.doc_count()?));
        Ok(sum_df)
    }

    fn doc_count(&self) -> Result<i32> {
        assert_thread("Terms", &self.creation_thread)?;
        let doc_count = self.r#in.doc_count()?;
        java_assert!(doc_count > 0);
        Ok(doc_count)
    }

    fn has_freqs(&self) -> bool {
        self.r#in.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.r#in.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.r#in.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.r#in.has_payloads()
    }

    fn min(&self) -> Result<Option<BytesRef>> {
        assert_thread("Terms", &self.creation_thread)?;
        let v = self.r#in.min()?;
        if let Some(v) = &v {
            assert_valid(v)?;
        }
        Ok(v)
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        assert_thread("Terms", &self.creation_thread)?;
        let v = self.r#in.max()?;
        if let Some(v) = &v {
            assert_valid(v)?;
        }
        Ok(v)
    }

    /// `FilterTerms.getStats()`: `in.getStats()`.
    fn stats(&self) -> Result<String> {
        self.r#in.stats()
    }
}

// =============================================================================
// AssertingTermsEnum
// =============================================================================

/// `AssertingTermsEnum.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermsEnumState {
    Initial,
    Positioned,
    Unpositioned,
    TwoPhaseSeeking,
}

/// Port of `AssertingLeafReader.AssertingTermsEnum` (`extends
/// FilterTermsEnum`).
///
/// Java decides with `VirtualMethod` whether the delegate overrides
/// `seekExact(BytesRef)` and calls `in.seekExact` or the inherited one; both
/// branches end in the delegate's `seekExact` (`FilterTermsEnum` forwards it),
/// so the port always calls the delegate's.
pub struct AssertingTermsEnum {
    creation_thread: Thread,
    r#in: Box<dyn TermsEnum>,
    state: TermsEnumState,
    has_freqs: bool,
}

impl AssertingTermsEnum {
    /// `new AssertingTermsEnum(in, hasFreqs)`.
    pub fn new(r#in: Box<dyn TermsEnum>, has_freqs: bool) -> Self {
        AssertingTermsEnum {
            creation_thread: Thread::current(),
            r#in,
            state: TermsEnumState::Initial,
            has_freqs,
        }
    }

    /// `reset()`: back to the initial state.
    pub fn reset(&mut self) {
        self.state = TermsEnumState::Initial;
    }
}

impl fmt::Display for AssertingTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingTermsEnum({})", self.r#in)
    }
}

impl TermsEnum for AssertingTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.r#in.attributes()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Positioned,
            "docs(...) called on unpositioned TermsEnum"
        );
        // reuse if the codec reused
        let mut reuse = reuse;
        let actual_reuse = reuse
            .as_mut()
            .and_then(|r| r.as_any_mut())
            .and_then(|a| a.downcast_mut::<AssertingPostingsEnum>())
            .map(|asserting| asserting.take_in());
        let reused_identity = actual_reuse.as_deref().map(identity);
        let docs = self.r#in.postings(actual_reuse, flags)?;
        if reused_identity == Some(identity(&*docs)) {
            // codec reused, reset asserting state
            let mut reuse = reuse.expect("a reused enum comes from `reuse`");
            let asserting = reuse
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<AssertingPostingsEnum>())
                .expect("`reuse` is an AssertingPostingsEnum");
            asserting.r#in = docs;
            asserting.reset();
            Ok(reuse)
        } else {
            Ok(Box::new(AssertingPostingsEnum::new(docs)))
        }
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Positioned,
            "docs(...) called on unpositioned TermsEnum"
        );
        java_assert!(
            (flags & POSTINGS_ENUM_FREQS) != 0,
            "Freqs should be requested on impacts"
        );
        Ok(Box::new(AssertingImpactsEnum::new(
            self.r#in.impacts(flags)?,
        )))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Initial || self.state == TermsEnumState::Positioned,
            "next() called on unpositioned TermsEnum"
        );
        let result = self.r#in.next()?;
        match &result {
            None => self.state = TermsEnumState::Unpositioned,
            Some(term) => {
                assert_valid(term)?;
                self.state = TermsEnumState::Positioned;
            }
        }
        Ok(result)
    }

    fn ord(&self) -> Result<i64> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Positioned,
            "ord() called on unpositioned TermsEnum"
        );
        self.r#in.ord()
    }

    fn doc_freq(&mut self) -> Result<i32> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Positioned,
            "docFreq() called on unpositioned TermsEnum"
        );
        let df = self.r#in.doc_freq()?;
        java_assert!(df > 0);
        Ok(df)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Positioned,
            "totalTermFreq() called on unpositioned TermsEnum"
        );
        let ttf = self.r#in.total_term_freq()?;
        if self.has_freqs {
            java_assert!(ttf >= i64::from(self.doc_freq()?));
        } else {
            java_assert!(ttf == i64::from(self.doc_freq()?));
        }
        Ok(ttf)
    }

    fn term(&self) -> Result<BytesRef> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Positioned,
            "term() called on unpositioned TermsEnum"
        );
        let ret = self.r#in.term()?;
        assert_valid(&ret)?;
        Ok(ret)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state != TermsEnumState::TwoPhaseSeeking,
            "Unfinished two-phase seeking"
        );
        self.r#in.seek_ord(ord)?;
        self.state = TermsEnumState::Positioned;
        Ok(())
    }

    fn seek_ceil(&mut self, term: &BytesRef) -> Result<SeekStatus> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state != TermsEnumState::TwoPhaseSeeking,
            "Unfinished two-phase seeking"
        );
        assert_valid(term)?;
        let result = self.r#in.seek_ceil(term)?;
        self.state = if result == SeekStatus::END {
            TermsEnumState::Unpositioned
        } else {
            TermsEnumState::Positioned
        };
        Ok(result)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state != TermsEnumState::TwoPhaseSeeking,
            "Unfinished two-phase seeking"
        );
        assert_valid(text)?;
        let result = self.r#in.seek_exact(text)?;
        self.state = if result {
            TermsEnumState::Positioned
        } else {
            TermsEnumState::Unpositioned
        };
        Ok(result)
    }

    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state != TermsEnumState::TwoPhaseSeeking,
            "Unfinished two-phase seeking"
        );
        assert_valid(text)?;
        let state = &mut self.state;
        let Some(mut supplier) = self.r#in.prepare_seek_exact(text)? else {
            return Ok(None);
        };
        *state = TermsEnumState::TwoPhaseSeeking;
        Ok(Some(Box::new(move || {
            let exists = supplier.get()?;
            *state = if exists {
                TermsEnumState::Positioned
            } else {
                TermsEnumState::Unpositioned
            };
            Ok(exists)
        })))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state == TermsEnumState::Positioned,
            "termState() called on unpositioned TermsEnum"
        );
        self.r#in.term_state()
    }

    fn seek_term_state(&mut self, term: &BytesRef, state: &dyn TermState) -> Result<()> {
        assert_thread("Terms enums", &self.creation_thread)?;
        java_assert!(
            self.state != TermsEnumState::TwoPhaseSeeking,
            "Unfinished two-phase seeking"
        );
        assert_valid(term)?;
        self.r#in.seek_term_state(term, state)?;
        self.state = TermsEnumState::Positioned;
        Ok(())
    }
}

/// The identity of an enum: the address of its data, which is what Java's
/// `docs == actualReuse` compares.
fn identity(postings: &dyn PostingsEnum) -> *const () {
    postings as *const dyn PostingsEnum as *const ()
}

// =============================================================================
// AssertingPostingsEnum
// =============================================================================

/// `AssertingLeafReader.DocsEnumState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocsEnumState {
    Start,
    Iterating,
    Finished,
}

/// The delegate an [`AssertingPostingsEnum`] holds while
/// [`AssertingTermsEnum::postings`] lends the real one to the codec for reuse:
/// it is never called, since the wrapper gets its delegate back (or is
/// dropped) before the call returns.
struct Lent;

impl fmt::Display for Lent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Lent")
    }
}

impl DocIdSetIterator for Lent {
    fn doc_id(&self) -> i32 {
        -1
    }
    fn next_doc(&mut self) -> Result<i32> {
        Ok(NO_MORE_DOCS)
    }
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Ok(NO_MORE_DOCS)
    }
    fn cost(&self) -> i64 {
        0
    }
}

impl PostingsEnum for Lent {
    fn freq(&self) -> Result<i32> {
        Ok(0)
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

/// Port of `AssertingLeafReader.AssertingPostingsEnum` (`extends
/// FilterPostingsEnum`): wraps a docs enum with additional checks.
///
/// `P` is the delegate's type: `dyn PostingsEnum` for the enums
/// [`AssertingTermsEnum::postings`] returns, `dyn ImpactsEnum` for the one an
/// [`AssertingImpactsEnum`] shares with Java's `in` field.
pub struct AssertingPostingsEnum<P: ?Sized + PostingsEnum + 'static = dyn PostingsEnum> {
    creation_thread: Thread,
    state: DocsEnumState,
    position_count: i32,
    position_max: i32,
    doc: i32,
    r#in: Box<P>,
}

impl<P: ?Sized + PostingsEnum + 'static> AssertingPostingsEnum<P> {
    /// `new AssertingPostingsEnum(in)`.
    pub fn new(r#in: Box<P>) -> Self {
        let doc = r#in.doc_id();
        AssertingPostingsEnum {
            creation_thread: Thread::current(),
            state: DocsEnumState::Start,
            position_count: 0,
            position_max: 0,
            doc,
            r#in,
        }
    }

    /// `FilterPostingsEnum.unwrap()`: the delegate.
    pub fn unwrap(&self) -> &P {
        &self.r#in
    }

    fn unwrap_mut(&mut self) -> &mut P {
        &mut self.r#in
    }

    /// `reset()`: the state of a fresh enum, positioned where the delegate is.
    pub fn reset(&mut self) {
        self.state = DocsEnumState::Start;
        self.doc = self.r#in.doc_id();
        self.position_count = 0;
        self.position_max = 0;
    }

    /// The state after a move to `doc`: `FINISHED` past the end, `ITERATING`
    /// with `positionMax = freq()` otherwise.
    fn moved_to(&mut self, doc: i32) -> Result<()> {
        if doc == NO_MORE_DOCS {
            self.state = DocsEnumState::Finished;
            self.position_max = 0;
        } else {
            self.state = DocsEnumState::Iterating;
            self.position_max = self.r#in.freq()?;
        }
        Ok(())
    }
}

impl AssertingPostingsEnum {
    /// Hands the delegate out (for the codec to reuse) and leaves [`Lent`] in
    /// its place.
    fn take_in(&mut self) -> Box<dyn PostingsEnum> {
        std::mem::replace(&mut self.r#in, Box::new(Lent))
    }
}

impl<P: ?Sized + PostingsEnum + 'static> fmt::Display for AssertingPostingsEnum<P> {
    /// `Object.toString()`: Java's class does not override it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.tests.index.AssertingLeafReader$AssertingPostingsEnum",
            self,
        ))
    }
}

impl<P: ?Sized + PostingsEnum + 'static> DocIdSetIterator for AssertingPostingsEnum<P> {
    fn doc_id(&self) -> i32 {
        assert_thread_unchecked("Docs enums", &self.creation_thread);
        java_assert_unchecked!(
            self.doc == self.r#in.doc_id(),
            " invalid docID() in {} {} instead of {}",
            self.r#in,
            self.r#in.doc_id(),
            self.doc
        );
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        assert_thread("Docs enums", &self.creation_thread)?;
        java_assert!(
            self.state != DocsEnumState::Finished,
            "nextDoc() called after NO_MORE_DOCS"
        );
        let next_doc = self.r#in.next_doc()?;
        java_assert!(
            next_doc > self.doc,
            "backwards nextDoc from {} to {} {}",
            self.doc,
            next_doc,
            self.r#in
        );
        self.moved_to(next_doc)?;
        self.position_count = 0;
        java_assert!(self.r#in.doc_id() == next_doc);
        self.doc = next_doc;
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        assert_thread("Docs enums", &self.creation_thread)?;
        java_assert!(
            self.state != DocsEnumState::Finished,
            "advance() called after NO_MORE_DOCS"
        );
        java_assert!(
            target > self.doc,
            "target must be > docID(), got {} <= {}",
            target,
            self.doc
        );
        let advanced = self.r#in.advance(target)?;
        java_assert!(
            advanced >= target,
            "backwards advance from: {} to: {}",
            target,
            advanced
        );
        self.moved_to(advanced)?;
        self.position_count = 0;
        java_assert!(self.r#in.doc_id() == advanced);
        self.doc = advanced;
        Ok(advanced)
    }

    /// `FilterPostingsEnum.cost()`: `in.cost()`.
    fn cost(&self) -> i64 {
        self.r#in.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        assert_thread("Docs enums", &self.creation_thread)?;
        java_assert!(
            self.state != DocsEnumState::Start,
            "intoBitSet() called before nextDoc()/advance()"
        );
        self.r#in.into_bit_set(up_to, bit_set, offset)?;
        java_assert!(self.r#in.doc_id() >= up_to);
        java_assert!(self.r#in.doc_id() >= self.doc);
        self.doc = self.r#in.doc_id();
        self.moved_to(self.doc)?;
        self.position_count = 0;
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        assert_thread("Docs enums", &self.creation_thread)?;
        java_assert!(
            self.state != DocsEnumState::Start,
            "intoBitSet() called before nextDoc()/advance()"
        );
        let start_doc_id = self.r#in.doc_id();
        let end_doc_id = self.r#in.doc_id_run_end()?;
        java_assert!(
            self.r#in.doc_id() == start_doc_id,
            "iterator changed docID while running docIDRunEnd()"
        );
        java_assert!(end_doc_id > start_doc_id);
        Ok(end_doc_id)
    }
}

impl<P: ?Sized + PostingsEnum + 'static> PostingsEnum for AssertingPostingsEnum<P> {
    fn freq(&self) -> Result<i32> {
        assert_thread("Docs enums", &self.creation_thread)?;
        java_assert!(
            self.state != DocsEnumState::Start,
            "freq() called before nextDoc()/advance()"
        );
        java_assert!(
            self.state != DocsEnumState::Finished,
            "freq() called after NO_MORE_DOCS"
        );
        let freq = self.r#in.freq()?;
        java_assert!(freq > 0);
        Ok(freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        java_assert!(
            self.state != DocsEnumState::Start,
            "nextPosition() called before nextDoc()/advance()"
        );
        java_assert!(
            self.state != DocsEnumState::Finished,
            "nextPosition() called after NO_MORE_DOCS"
        );
        java_assert!(
            self.position_count < self.position_max,
            "nextPosition() called more than freq() times!"
        );
        let position = self.r#in.next_position()?;
        java_assert!(
            position >= 0 || position == -1,
            "invalid position: {}",
            position
        );
        self.position_count += 1;
        Ok(position)
    }

    fn start_offset(&self) -> i32 {
        java_assert_unchecked!(
            self.state != DocsEnumState::Start,
            "startOffset() called before nextDoc()/advance()"
        );
        java_assert_unchecked!(
            self.state != DocsEnumState::Finished,
            "startOffset() called after NO_MORE_DOCS"
        );
        java_assert_unchecked!(
            self.position_count > 0,
            "startOffset() called before nextPosition()!"
        );
        self.r#in.start_offset()
    }

    fn end_offset(&self) -> i32 {
        java_assert_unchecked!(
            self.state != DocsEnumState::Start,
            "endOffset() called before nextDoc()/advance()"
        );
        java_assert_unchecked!(
            self.state != DocsEnumState::Finished,
            "endOffset() called after NO_MORE_DOCS"
        );
        java_assert_unchecked!(
            self.position_count > 0,
            "endOffset() called before nextPosition()!"
        );
        self.r#in.end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        java_assert!(
            self.state != DocsEnumState::Start,
            "getPayload() called before nextDoc()/advance()"
        );
        java_assert!(
            self.state != DocsEnumState::Finished,
            "getPayload() called after NO_MORE_DOCS"
        );
        java_assert!(
            self.position_count > 0,
            "getPayload() called before nextPosition()!"
        );
        let payload = self.r#in.get_payload()?;
        java_assert!(
            payload.is_none_or(|p| !p.is_empty()),
            "getPayload() returned payload with invalid length!"
        );
        Ok(payload)
    }

    fn next_postings(&mut self, up_to: i32, buffer: &mut DocAndFloatFeatureBuffer) -> Result<()> {
        java_assert!(
            self.state != DocsEnumState::Start,
            "nextPostings() called before nextDoc()/advance()"
        );
        self.r#in.next_postings(up_to, buffer)?;
        self.doc = self.r#in.doc_id();
        self.moved_to(self.doc)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// =============================================================================
// AssertingImpactsEnum
// =============================================================================

/// Port of `AssertingLeafReader.AssertingImpactsEnum`: wraps an
/// [`ImpactsEnum`] with additional checks, the iteration checks inherited
/// from an [`AssertingPostingsEnum`] over the same delegate.
pub struct AssertingImpactsEnum {
    asserting_postings: AssertingPostingsEnum<dyn ImpactsEnum>,
    last_shallow_target: i32,
    /// `Math.max(docID(), lastShallowTarget)` after the last move, shared
    /// with the [`AssertingImpacts`] this enum hands out: Java's impacts read
    /// it off the enum they keep a reference to.
    position: Arc<AtomicI32>,
}

impl AssertingImpactsEnum {
    /// `new AssertingImpactsEnum(impacts)`.
    pub fn new(impacts: Box<dyn ImpactsEnum>) -> Self {
        let asserting_postings = AssertingPostingsEnum::new(impacts);
        let position = Arc::new(AtomicI32::new(asserting_postings.doc.max(-1)));
        AssertingImpactsEnum {
            asserting_postings,
            last_shallow_target: -1,
            position,
        }
    }

    /// Publishes `max(docID(), lastShallowTarget)` to the impacts handed out.
    fn publish_position(&self) {
        self.position.store(
            self.asserting_postings.doc.max(self.last_shallow_target),
            Ordering::Relaxed,
        );
    }
}

impl fmt::Display for AssertingImpactsEnum {
    /// `Object.toString()`: Java's class does not override it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.tests.index.AssertingLeafReader$AssertingImpactsEnum",
            self,
        ))
    }
}

impl ImpactsSource for AssertingImpactsEnum {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        java_assert!(
            target >= self.last_shallow_target,
            "called on decreasing targets: target = {} < last target = {}",
            target,
            self.last_shallow_target
        );
        let doc_id = self.doc_id();
        java_assert!(target >= doc_id, "target = {} < docID = {}", target, doc_id);
        self.last_shallow_target = target;
        self.publish_position();
        self.asserting_postings.unwrap_mut().advance_shallow(target)
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        java_assert!(
            self.doc_id() >= 0 || self.last_shallow_target >= 0,
            "Cannot get impacts until the iterator is positioned or advanceShallow has been called"
        );
        let impacts = self.asserting_postings.unwrap_mut().get_impacts()?;
        let valid_for = self.doc_id().max(self.last_shallow_target);
        rucene::index::check_index::check_impacts(impacts.as_ref(), valid_for)?;
        Ok(Box::new(AssertingImpacts {
            r#in: impacts,
            position: Arc::clone(&self.position),
            valid_for,
        }))
    }
}

impl DocIdSetIterator for AssertingImpactsEnum {
    fn doc_id(&self) -> i32 {
        self.asserting_postings.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.doc_id();
        java_assert!(
            doc_id + 1 >= self.last_shallow_target,
            "target = {} < last shallow target = {}",
            doc_id + 1,
            self.last_shallow_target
        );
        let doc = self.asserting_postings.next_doc()?;
        self.publish_position();
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        java_assert!(
            target >= self.last_shallow_target,
            "target = {} < last shallow target = {}",
            target,
            self.last_shallow_target
        );
        let doc = self.asserting_postings.advance(target)?;
        self.publish_position();
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.asserting_postings.cost()
    }
}

impl PostingsEnum for AssertingImpactsEnum {
    fn freq(&self) -> Result<i32> {
        self.asserting_postings.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.asserting_postings.next_position()
    }

    fn start_offset(&self) -> i32 {
        self.asserting_postings.start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.asserting_postings.end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.asserting_postings.get_payload()
    }
}

impl ImpactsEnum for AssertingImpactsEnum {}

/// Port of `AssertingLeafReader.AssertingImpacts`: impacts that refuse to be
/// read once their enum has moved.
struct AssertingImpacts {
    r#in: Box<dyn Impacts>,
    position: Arc<AtomicI32>,
    valid_for: i32,
}

impl AssertingImpacts {
    fn assert_valid_for(&self) {
        java_assert_unchecked!(
            self.valid_for == self.position.load(Ordering::Relaxed),
            "Cannot reuse impacts after advancing the iterator"
        );
    }
}

impl Impacts for AssertingImpacts {
    fn num_levels(&self) -> i32 {
        self.assert_valid_for();
        self.r#in.num_levels()
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        self.assert_valid_for();
        self.r#in.doc_id_up_to(level)
    }

    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer {
        self.assert_valid_for();
        let impacts = self.r#in.get_impacts(level);
        java_assert_unchecked!(impacts.size > 0);
        impacts
    }
}

// =============================================================================
// Doc values
// =============================================================================

/// The checks `intoBitSet` shares across the doc-values wrappers.
macro_rules! dv_into_bit_set {
    ($self:ident, $name:literal, $up_to:ident, $bit_set:ident, $offset:ident) => {{
        assert_thread($name, &$self.creation_thread)?;
        java_assert!($self.exists || $self.doc_id() == NO_MORE_DOCS);
        java_assert!($self.doc_id() != -1);
        java_assert!($offset <= $self.doc_id());
        $self.r#in.into_bit_set($up_to, $bit_set, $offset)?;
        java_assert!($self.doc_id() >= $up_to);
        $self.last_doc_id = $self.doc_id();
    }};
}

/// The checks `docIDRunEnd` shares across the doc-values wrappers.
macro_rules! dv_doc_id_run_end {
    ($self:ident, $name:literal) => {{
        assert_thread($name, &$self.creation_thread)?;
        java_assert!($self.doc_id() != -1);
        java_assert!($self.doc_id() != NO_MORE_DOCS);
        java_assert!($self.exists);
        let next_non_matching_doc_id = $self.r#in.doc_id_run_end()?;
        java_assert!(next_non_matching_doc_id > $self.doc_id());
        Ok(next_non_matching_doc_id)
    }};
}

/// The checks `cost()` shares across the doc-values wrappers.
macro_rules! dv_cost {
    ($self:ident, $name:literal) => {{
        assert_thread_unchecked($name, &$self.creation_thread);
        let cost = $self.r#in.cost();
        java_assert_unchecked!(cost >= 0);
        cost
    }};
}

/// Port of `AssertingLeafReader.AssertingNumericDocValues`: wraps a
/// [`NumericDocValues`] but with additional asserts.
pub struct AssertingNumericDocValues {
    creation_thread: Thread,
    r#in: Box<dyn NumericDocValues>,
    max_doc: i32,
    last_doc_id: i32,
    exists: bool,
}

impl AssertingNumericDocValues {
    /// `new AssertingNumericDocValues(in, maxDoc)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` when `in` is positioned (it should start unpositioned).
    pub fn new(r#in: Box<dyn NumericDocValues>, max_doc: i32) -> Result<Self> {
        // should start unpositioned:
        java_assert!(r#in.doc_id() == -1);
        Ok(AssertingNumericDocValues {
            creation_thread: Thread::current(),
            r#in,
            max_doc,
            last_doc_id: -1,
            exists: false,
        })
    }
}

impl DocIdSetIterator for AssertingNumericDocValues {
    fn doc_id(&self) -> i32 {
        assert_thread_unchecked("Numeric doc values", &self.creation_thread);
        self.r#in.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        assert_thread("Numeric doc values", &self.creation_thread)?;
        let doc_id = self.r#in.next_doc()?;
        java_assert!(doc_id > self.last_doc_id);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        java_assert!(doc_id == self.r#in.doc_id());
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        assert_thread("Numeric doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target > self.r#in.doc_id());
        let doc_id = self.r#in.advance(target)?;
        java_assert!(doc_id >= target);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        dv_cost!(self, "Numeric doc values")
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        dv_into_bit_set!(self, "Numeric doc values", up_to, bit_set, offset);
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        dv_doc_id_run_end!(self, "Numeric doc values")
    }
}

impl DocValuesIterator for AssertingNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        assert_thread("Numeric doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target >= self.r#in.doc_id());
        java_assert!(target < self.max_doc);
        self.exists = self.r#in.advance_exact(target)?;
        java_assert!(self.r#in.doc_id() == target);
        self.last_doc_id = target;
        Ok(self.exists)
    }
}

impl NumericDocValues for AssertingNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        assert_thread("Numeric doc values", &self.creation_thread)?;
        java_assert!(self.exists);
        self.r#in.long_value()
    }

    /// `longValues(size, docs, values, defaultValue)`: the checks, then
    /// `super.longValues` — `NumericDocValues`'s default, which reads every
    /// document through this wrapper's `advanceExact` and `longValue`.
    fn long_values(
        &mut self,
        size: i32,
        docs: &[i32],
        docs_offset: i32,
        values: &mut [i64],
        values_offset: i32,
        default_value: i64,
    ) -> Result<()> {
        assert_thread("Numeric doc values", &self.creation_thread)?;
        let at = |i: i32| docs[(docs_offset + i) as usize];
        java_assert!(size >= 0);
        java_assert!(size == 0 || at(0) >= self.doc_id());
        java_assert!(size == 0 || at(0) >= 0);
        for i in 1..size {
            java_assert!(at(i) > at(i - 1));
        }
        java_assert!(size == 0 || at(size - 1) < self.max_doc);
        let expected_doc_id_on_return = if size == 0 {
            self.doc_id()
        } else {
            at(size - 1)
        };
        for i in 0..size.max(0) {
            let doc = docs[(docs_offset + i) as usize];
            values[(values_offset + i) as usize] = if self.advance_exact(doc)? {
                self.long_value()?
            } else {
                default_value
            };
        }
        self.last_doc_id = self.r#in.doc_id();
        java_assert!(self.last_doc_id == expected_doc_id_on_return);
        Ok(())
    }
}

impl fmt::Debug for AssertingNumericDocValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingNumericDocValues")
    }
}

/// Port of `AssertingLeafReader.AssertingBinaryDocValues`: wraps a
/// [`BinaryDocValues`] but with additional asserts.
pub struct AssertingBinaryDocValues {
    creation_thread: Thread,
    r#in: Box<dyn BinaryDocValues>,
    max_doc: i32,
    last_doc_id: i32,
    exists: bool,
}

impl AssertingBinaryDocValues {
    /// `new AssertingBinaryDocValues(in, maxDoc)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` when `in` is positioned (it should start unpositioned).
    pub fn new(r#in: Box<dyn BinaryDocValues>, max_doc: i32) -> Result<Self> {
        // should start unpositioned:
        java_assert!(r#in.doc_id() == -1);
        Ok(AssertingBinaryDocValues {
            creation_thread: Thread::current(),
            r#in,
            max_doc,
            last_doc_id: -1,
            exists: false,
        })
    }
}

impl DocIdSetIterator for AssertingBinaryDocValues {
    fn doc_id(&self) -> i32 {
        assert_thread_unchecked("Binary doc values", &self.creation_thread);
        self.r#in.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        assert_thread("Binary doc values", &self.creation_thread)?;
        let doc_id = self.r#in.next_doc()?;
        java_assert!(doc_id > self.last_doc_id);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        java_assert!(doc_id == self.r#in.doc_id());
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        assert_thread("Binary doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target > self.r#in.doc_id());
        let doc_id = self.r#in.advance(target)?;
        java_assert!(doc_id >= target);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        dv_cost!(self, "Binary doc values")
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        dv_into_bit_set!(self, "Binary doc values", up_to, bit_set, offset);
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        dv_doc_id_run_end!(self, "Binary doc values")
    }
}

impl DocValuesIterator for AssertingBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        assert_thread("Binary doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target >= self.r#in.doc_id());
        java_assert!(target < self.max_doc);
        self.exists = self.r#in.advance_exact(target)?;
        java_assert!(self.r#in.doc_id() == target);
        self.last_doc_id = target;
        Ok(self.exists)
    }
}

impl BinaryDocValues for AssertingBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        assert_thread("Binary doc values", &self.creation_thread)?;
        java_assert!(self.exists);
        self.r#in.binary_value()
    }
}

/// Port of `AssertingLeafReader.AssertingSortedDocValues`: wraps a
/// [`SortedDocValues`] but with additional asserts.
pub struct AssertingSortedDocValues {
    creation_thread: Thread,
    r#in: Box<dyn SortedDocValues>,
    max_doc: i32,
    value_count: i32,
    last_doc_id: i32,
    exists: bool,
}

impl AssertingSortedDocValues {
    /// `new AssertingSortedDocValues(in, maxDoc)`.
    ///
    /// # Errors
    ///
    /// What `in.getValueCount()` raises, and `AssertionError` when the value
    /// count is negative or above `max_doc`.
    pub fn new(r#in: Box<dyn SortedDocValues>, max_doc: i32) -> Result<Self> {
        let value_count = r#in.get_value_count()?;
        java_assert!(value_count >= 0 && value_count <= max_doc);
        Ok(AssertingSortedDocValues {
            creation_thread: Thread::current(),
            r#in,
            max_doc,
            value_count,
            last_doc_id: -1,
            exists: false,
        })
    }
}

impl DocIdSetIterator for AssertingSortedDocValues {
    fn doc_id(&self) -> i32 {
        assert_thread_unchecked("Sorted doc values", &self.creation_thread);
        self.r#in.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        assert_thread("Sorted doc values", &self.creation_thread)?;
        let doc_id = self.r#in.next_doc()?;
        java_assert!(doc_id > self.last_doc_id);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        java_assert!(doc_id == self.r#in.doc_id());
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        assert_thread("Sorted doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target > self.r#in.doc_id());
        let doc_id = self.r#in.advance(target)?;
        java_assert!(doc_id >= target);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        dv_cost!(self, "Sorted doc values")
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        dv_into_bit_set!(self, "Sorted doc values", up_to, bit_set, offset);
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        dv_doc_id_run_end!(self, "Sorted doc values")
    }
}

impl DocValuesIterator for AssertingSortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        assert_thread("Sorted doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target >= self.r#in.doc_id());
        java_assert!(target < self.max_doc);
        self.exists = self.r#in.advance_exact(target)?;
        java_assert!(self.r#in.doc_id() == target);
        self.last_doc_id = target;
        Ok(self.exists)
    }
}

impl SortedDocValues for AssertingSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        assert_thread("Sorted doc values", &self.creation_thread)?;
        java_assert!(self.exists);
        let ord = self.r#in.ord_value()?;
        java_assert!(ord >= -1 && ord < self.value_count);
        Ok(ord)
    }

    fn get_value_count(&self) -> Result<i32> {
        assert_thread("Sorted doc values", &self.creation_thread)?;
        let value_count = self.r#in.get_value_count()?;
        java_assert!(value_count == self.value_count); // should not change
        Ok(value_count)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        assert_thread("Sorted doc values", &self.creation_thread)?;
        java_assert!(ord >= 0 && ord < self.value_count);
        let result = self.r#in.lookup_ord(ord)?;
        assert_valid(&result)?;
        Ok(result)
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i32> {
        assert_thread("Sorted doc values", &self.creation_thread)?;
        assert_valid(key)?;
        let result = self.r#in.lookup_term(key)?;
        java_assert!(result < self.value_count);
        assert_valid(key)?;
        Ok(result)
    }
}

/// Port of `AssertingLeafReader.AssertingSortedNumericDocValues`: wraps a
/// [`SortedNumericDocValues`] but with additional asserts.
pub struct AssertingSortedNumericDocValues {
    creation_thread: Thread,
    r#in: Box<dyn SortedNumericDocValues>,
    max_doc: i32,
    last_doc_id: i32,
    value_upto: i32,
    exists: bool,
}

impl AssertingSortedNumericDocValues {
    /// `AssertingSortedNumericDocValues.create(in, maxDoc)`: a singleton's
    /// single-valued view is wrapped in an [`AssertingNumericDocValues`] and
    /// made a singleton again; any other `in` is wrapped as is.
    ///
    /// # Errors
    ///
    /// What [`AssertingNumericDocValues::new`] raises.
    pub fn create(
        r#in: Box<dyn SortedNumericDocValues>,
        max_doc: i32,
    ) -> Result<Box<dyn SortedNumericDocValues>> {
        if r#in.is_singleton() {
            if let Some(single_doc_values) = DocValues::unwrap_singleton_numeric(r#in) {
                let asserting_doc_values =
                    AssertingNumericDocValues::new(single_doc_values, max_doc)?;
                return Ok(Box::new(DocValues::singleton_numeric(Box::new(
                    asserting_doc_values,
                ))));
            }
            return Err(LuceneError::illegal_state(
                "a singleton sorted-numeric doc values gave up no single-valued view".to_string(),
            ));
        }
        Ok(Box::new(AssertingSortedNumericDocValues {
            creation_thread: Thread::current(),
            r#in,
            max_doc,
            last_doc_id: -1,
            value_upto: 0,
            exists: false,
        }))
    }
}

impl DocIdSetIterator for AssertingSortedNumericDocValues {
    /// Java's `docID()` makes no thread check here.
    fn doc_id(&self) -> i32 {
        self.r#in.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        assert_thread("Sorted numeric doc values", &self.creation_thread)?;
        let doc_id = self.r#in.next_doc()?;
        java_assert!(doc_id > self.last_doc_id);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        java_assert!(doc_id == self.r#in.doc_id());
        self.last_doc_id = doc_id;
        self.value_upto = 0;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        assert_thread("Sorted numeric doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target > self.r#in.doc_id());
        let doc_id = self.r#in.advance(target)?;
        java_assert!(doc_id == self.r#in.doc_id());
        java_assert!(doc_id >= target);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        self.last_doc_id = doc_id;
        self.value_upto = 0;
        self.exists = doc_id != NO_MORE_DOCS;
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        dv_cost!(self, "Sorted numeric doc values")
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        dv_into_bit_set!(self, "Sorted numeric doc values", up_to, bit_set, offset);
        self.value_upto = 0;
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        dv_doc_id_run_end!(self, "Sorted numeric doc values")
    }
}

impl DocValuesIterator for AssertingSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        assert_thread("Sorted numeric doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target >= self.r#in.doc_id());
        java_assert!(target < self.max_doc);
        self.exists = self.r#in.advance_exact(target)?;
        java_assert!(self.r#in.doc_id() == target);
        self.last_doc_id = target;
        self.value_upto = 0;
        Ok(self.exists)
    }
}

impl SortedNumericDocValues for AssertingSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        assert_thread("Sorted numeric doc values", &self.creation_thread)?;
        java_assert!(self.exists);
        let count = self.r#in.doc_value_count()?;
        java_assert!(
            self.value_upto < count,
            "valueUpto={} in.docValueCount()={}",
            self.value_upto,
            count
        );
        self.value_upto += 1;
        self.r#in.next_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        assert_thread("Sorted numeric doc values", &self.creation_thread)?;
        java_assert!(self.exists);
        java_assert!(self.r#in.doc_value_count()? > 0);
        self.r#in.doc_value_count()
    }
}

/// Port of `AssertingLeafReader.AssertingSortedSetDocValues`: wraps a
/// [`SortedSetDocValues`] but with additional asserts.
pub struct AssertingSortedSetDocValues {
    creation_thread: Thread,
    r#in: Box<dyn SortedSetDocValues>,
    max_doc: i32,
    value_count: i64,
    last_doc_id: i32,
    ords_retrieved: i32,
    exists: bool,
}

impl AssertingSortedSetDocValues {
    /// `AssertingSortedSetDocValues.create(in, maxDoc)`: a singleton's
    /// single-valued view is wrapped in an [`AssertingSortedDocValues`] and
    /// made a singleton again; any other `in` is wrapped as is.
    ///
    /// # Errors
    ///
    /// What `in.getValueCount()` and the wrappers' constructors raise.
    pub fn create(
        r#in: Box<dyn SortedSetDocValues>,
        max_doc: i32,
    ) -> Result<Box<dyn SortedSetDocValues>> {
        if r#in.is_singleton() {
            if let Some(single_doc_values) = DocValues::unwrap_singleton_sorted(r#in) {
                let asserting_doc_values =
                    AssertingSortedDocValues::new(single_doc_values, max_doc)?;
                return Ok(Box::new(DocValues::singleton_sorted(Box::new(
                    asserting_doc_values,
                ))));
            }
            return Err(LuceneError::illegal_state(
                "a singleton sorted-set doc values gave up no single-valued view".to_string(),
            ));
        }
        Ok(Box::new(AssertingSortedSetDocValues::new(r#in, max_doc)?))
    }

    /// The private `AssertingSortedSetDocValues(in, maxDoc)` constructor,
    /// which wraps `in` as is (a singleton too): [`create`](Self::create)
    /// calls it for a multi-valued `in`, and `AssertingLeafReader` for every
    /// `in` (`AssertingLeafReader.java:1796-1808`).
    ///
    /// # Errors
    ///
    /// What `in.getValueCount()` raises, and the `AssertionError` of
    /// `assert valueCount >= 0`.
    pub(crate) fn new(r#in: Box<dyn SortedSetDocValues>, max_doc: i32) -> Result<Self> {
        let value_count = r#in.get_value_count()?;
        java_assert!(value_count >= 0);
        Ok(AssertingSortedSetDocValues {
            creation_thread: Thread::current(),
            r#in,
            max_doc,
            value_count,
            last_doc_id: -1,
            ords_retrieved: 0,
            exists: false,
        })
    }
}

impl DocIdSetIterator for AssertingSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        assert_thread_unchecked("Sorted set doc values", &self.creation_thread);
        self.r#in.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        assert_thread("Sorted set doc values", &self.creation_thread)?;
        let doc_id = self.r#in.next_doc()?;
        java_assert!(doc_id > self.last_doc_id);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        java_assert!(doc_id == self.r#in.doc_id());
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        self.ords_retrieved = 0;
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        assert_thread("Sorted set doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target > self.r#in.doc_id());
        let doc_id = self.r#in.advance(target)?;
        java_assert!(doc_id == self.r#in.doc_id());
        java_assert!(doc_id >= target);
        java_assert!(doc_id == NO_MORE_DOCS || doc_id < self.max_doc);
        self.last_doc_id = doc_id;
        self.exists = doc_id != NO_MORE_DOCS;
        self.ords_retrieved = 0;
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        dv_cost!(self, "Sorted set doc values")
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        dv_into_bit_set!(self, "Sorted set doc values", up_to, bit_set, offset);
        self.ords_retrieved = 0;
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        dv_doc_id_run_end!(self, "Sorted set doc values")
    }
}

impl DocValuesIterator for AssertingSortedSetDocValues {
    /// Java's `advanceExact` names the object "Sorted numeric doc values" in
    /// its thread check; reproduced.
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        assert_thread("Sorted numeric doc values", &self.creation_thread)?;
        java_assert!(target >= 0);
        java_assert!(target >= self.r#in.doc_id());
        java_assert!(target < self.max_doc);
        self.exists = self.r#in.advance_exact(target)?;
        java_assert!(self.r#in.doc_id() == target);
        self.last_doc_id = target;
        self.ords_retrieved = 0;
        Ok(self.exists)
    }
}

impl SortedSetDocValues for AssertingSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        assert_thread("Sorted set doc values", &self.creation_thread)?;
        java_assert!(self.exists);
        java_assert!(self.ords_retrieved < self.doc_value_count()?);
        self.ords_retrieved += 1;
        let ord = self.r#in.next_ord()?;
        java_assert!(ord < self.value_count);
        Ok(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.r#in.doc_value_count()
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        assert_thread("Sorted set doc values", &self.creation_thread)?;
        java_assert!(ord >= 0 && ord < self.value_count);
        let result = self.r#in.lookup_ord(ord)?;
        assert_valid(&result)?;
        Ok(result)
    }

    fn get_value_count(&self) -> Result<i64> {
        assert_thread("Sorted set doc values", &self.creation_thread)?;
        let value_count = self.r#in.get_value_count()?;
        java_assert!(value_count == self.value_count); // should not change
        Ok(value_count)
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i64> {
        assert_thread("Sorted set doc values", &self.creation_thread)?;
        assert_valid(key)?;
        let result = self.r#in.lookup_term(key)?;
        java_assert!(result < self.value_count);
        assert_valid(key)?;
        Ok(result)
    }
}

// =============================================================================
// AssertingDocValuesSkipper
// =============================================================================

/// Port of `AssertingLeafReader.AssertingDocValuesSkipper`: wraps a
/// [`DocValuesSkipper`] but with additional asserts.
pub struct AssertingDocValuesSkipper {
    creation_thread: Thread,
    r#in: Box<dyn DocValuesSkipper>,
}

impl AssertingDocValuesSkipper {
    /// `new AssertingDocValuesSkipper(in)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` when `in` is positioned.
    pub fn new(r#in: Box<dyn DocValuesSkipper>) -> Result<Self> {
        let skipper = AssertingDocValuesSkipper {
            creation_thread: Thread::current(),
            r#in,
        };
        java_assert!(skipper.min_doc_id(0) == -1);
        java_assert!(skipper.max_doc_id(0) == -1);
        Ok(skipper)
    }

    fn iterating(&self) -> bool {
        self.max_doc_id(0) != -1
            && self.min_doc_id(0) != -1
            && self.max_doc_id(0) != NO_MORE_DOCS
            && self.min_doc_id(0) != NO_MORE_DOCS
    }
}

impl DocValuesSkipper for AssertingDocValuesSkipper {
    fn advance(&mut self, target: i32) -> Result<()> {
        assert_thread("Doc values skipper", &self.creation_thread)?;
        java_assert!(
            target > self.max_doc_id(0),
            "Illegal to call advance() on a target that is not beyond the current interval"
        );
        self.r#in.advance(target)?;
        java_assert!(self.r#in.min_doc_id(0) <= self.r#in.max_doc_id(0));
        Ok(())
    }

    fn num_levels(&self) -> i32 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        self.r#in.num_levels()
    }

    fn min_doc_id(&self, level: i32) -> i32 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        check_index(level, self.num_levels());
        let min_doc_id = self.r#in.min_doc_id(level);
        java_assert_unchecked!(min_doc_id <= self.r#in.max_doc_id(level));
        if level > 0 {
            java_assert_unchecked!(min_doc_id <= self.r#in.min_doc_id(level - 1));
        }
        min_doc_id
    }

    fn max_value_count(&self) -> i32 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        self.r#in.max_value_count()
    }

    fn max_doc_id(&self, level: i32) -> i32 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        check_index(level, self.num_levels());
        let max_doc_id = self.r#in.max_doc_id(level);
        java_assert_unchecked!(max_doc_id >= self.r#in.min_doc_id(level));
        if level > 0 {
            java_assert_unchecked!(max_doc_id >= self.r#in.max_doc_id(level - 1));
        }
        max_doc_id
    }

    fn min_value(&self, level: i32) -> i64 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        java_assert_unchecked!(self.iterating(), "Unpositioned iterator");
        check_index(level, self.num_levels());
        self.r#in.min_value(level)
    }

    fn max_value(&self, level: i32) -> i64 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        java_assert_unchecked!(self.iterating(), "Unpositioned iterator");
        check_index(level, self.num_levels());
        self.r#in.max_value(level)
    }

    fn doc_count(&self, level: i32) -> i32 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        java_assert_unchecked!(self.iterating(), "Unpositioned iterator");
        check_index(level, self.num_levels());
        self.r#in.doc_count(level)
    }

    fn global_min_value(&self) -> i64 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        self.r#in.global_min_value()
    }

    fn global_max_value(&self) -> i64 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        self.r#in.global_max_value()
    }

    fn global_doc_count(&self) -> i32 {
        assert_thread_unchecked("Doc values skipper", &self.creation_thread);
        self.r#in.global_doc_count()
    }
}

// =============================================================================
// AssertingPointValues
// =============================================================================

/// Port of `AssertingLeafReader.AssertingPointValues`: wraps a
/// [`PointValues`] but with additional asserts.
///
/// The delegate is shared ([`Arc`]) with the point trees this hands out:
/// Java's `AssertingPointTree` keeps a reference to it and reads its
/// dimensions at every visit.
pub struct AssertingPointValues {
    creation_thread: Thread,
    r#in: Arc<dyn PointValues>,
}

impl AssertingPointValues {
    /// `new AssertingPointValues(in, maxDoc)`: wraps `in` and checks its
    /// statistics against `max_doc`.
    ///
    /// # Errors
    ///
    /// What reading the statistics raises, and `AssertionError` when they are
    /// inconsistent.
    pub fn new(r#in: Box<dyn PointValues>, max_doc: i32) -> Result<Self> {
        let values = AssertingPointValues {
            creation_thread: Thread::current(),
            r#in: Arc::from(r#in),
        };
        values.assert_stats(max_doc)?;
        Ok(values)
    }

    /// `getWrapped()`: the delegate.
    pub fn get_wrapped(&self) -> &dyn PointValues {
        &*self.r#in
    }

    fn assert_stats(&self, max_doc: i32) -> Result<()> {
        java_assert!(self.r#in.size()? > 0);
        java_assert!(self.r#in.doc_count()? > 0);
        java_assert!(i64::from(self.r#in.doc_count()?) <= self.r#in.size()?);
        java_assert!(self.r#in.doc_count()? <= max_doc);
        Ok(())
    }
}

/// `Objects.requireNonNull(value)`: `NullPointerException` for `null`.
fn require_non_null(value: Option<Vec<u8>>) -> Result<Option<Vec<u8>>> {
    match value {
        Some(v) => Ok(Some(v)),
        None => Err(npe()),
    }
}

impl PointValues for AssertingPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        assert_thread("Points", &self.creation_thread)?;
        Ok(Box::new(AssertingPointTree {
            point_values: Arc::clone(&self.r#in),
            r#in: self.r#in.point_tree()?,
        }))
    }

    fn size(&self) -> Result<i64> {
        assert_thread("Points", &self.creation_thread)?;
        self.r#in.size()
    }

    fn doc_count(&self) -> Result<i32> {
        assert_thread("Points", &self.creation_thread)?;
        self.r#in.doc_count()
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        assert_thread("Points", &self.creation_thread)?;
        require_non_null(self.r#in.min_packed_value()?)
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        assert_thread("Points", &self.creation_thread)?;
        require_non_null(self.r#in.max_packed_value()?)
    }

    fn num_dimensions(&self) -> Result<i32> {
        assert_thread("Points", &self.creation_thread)?;
        self.r#in.num_dimensions()
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        assert_thread("Points", &self.creation_thread)?;
        self.r#in.num_index_dimensions()
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        assert_thread("Points", &self.creation_thread)?;
        self.r#in.bytes_per_dimension()
    }
}

/// Port of `AssertingLeafReader.AssertingPointTree`.
struct AssertingPointTree {
    point_values: Arc<dyn PointValues>,
    r#in: Box<dyn PointTree>,
}

impl AssertingPointTree {
    fn visitor<'v>(
        &self,
        visitor: &'v mut dyn IntersectVisitor,
    ) -> Result<AssertingIntersectVisitor<'v>> {
        Ok(AssertingIntersectVisitor::new(
            self.point_values.num_dimensions()?,
            self.point_values.num_index_dimensions()?,
            self.point_values.bytes_per_dimension()?,
            visitor,
        ))
    }
}

impl PointTree for AssertingPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(AssertingPointTree {
            point_values: Arc::clone(&self.point_values),
            r#in: self.r#in.clone_tree()?,
        }))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        self.r#in.move_to_child()
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        self.r#in.move_to_sibling()
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        self.r#in.move_to_parent()
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        self.r#in.min_packed_value()
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        self.r#in.max_packed_value()
    }

    fn size(&self) -> Result<i64> {
        let size = self.r#in.size()?;
        java_assert!(size > 0);
        Ok(size)
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let mut asserting = self.visitor(visitor)?;
        self.r#in.visit_doc_ids(&mut asserting)
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let mut asserting = self.visitor(visitor)?;
        self.r#in.visit_doc_values(&mut asserting)
    }
}

/// Port of `AssertingLeafReader.AssertingIntersectVisitor`: validates in the
/// 1D case that all points are visited in order, and point values are in
/// bounds of the last cell checked.
///
/// `compare` takes `&self` in this port and records the last cell, so the
/// fields it writes are cells.
struct AssertingIntersectVisitor<'v> {
    r#in: &'v mut dyn IntersectVisitor,
    num_data_dims: i32,
    num_index_dims: i32,
    bytes_per_dim: i32,
    last_doc_value: Option<Vec<u8>>,
    last_min_packed_value: RefCell<Vec<u8>>,
    last_max_packed_value: RefCell<Vec<u8>>,
    last_compare_result: Cell<Option<Relation>>,
    last_doc_id: i32,
    doc_budget: i32,
}

impl<'v> AssertingIntersectVisitor<'v> {
    fn new(
        num_data_dims: i32,
        num_index_dims: i32,
        bytes_per_dim: i32,
        r#in: &'v mut dyn IntersectVisitor,
    ) -> Self {
        let packed = (num_data_dims * bytes_per_dim).max(0) as usize;
        AssertingIntersectVisitor {
            r#in,
            num_data_dims,
            num_index_dims,
            bytes_per_dim,
            last_doc_value: (num_data_dims == 1).then(|| vec![0u8; bytes_per_dim.max(0) as usize]),
            last_min_packed_value: RefCell::new(vec![0u8; packed]),
            last_max_packed_value: RefCell::new(vec![0u8; packed]),
            last_compare_result: Cell::new(None),
            last_doc_id: -1,
            doc_budget: 0,
        }
    }

    /// `[dim * bytesPerDim, dim * bytesPerDim + bytesPerDim)`.
    fn dim_range(&self, dim: i32) -> std::ops::Range<usize> {
        let from = (dim * self.bytes_per_dim) as usize;
        from..from + self.bytes_per_dim as usize
    }
}

impl IntersectVisitor for AssertingIntersectVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        java_assert!(
            {
                self.doc_budget -= 1;
                self.doc_budget >= 0
            },
            "called add() more times than the last call to grow() reserved"
        );
        // This method, not filtering each hit, should only be invoked when the
        // cell is inside the query shape:
        java_assert!(matches!(
            self.last_compare_result.get(),
            None | Some(Relation::CellInsideQuery)
        ));
        self.r#in.visit(doc_id)
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        java_assert!(
            {
                self.doc_budget -= 1;
                self.doc_budget >= 0
            },
            "called add() more times than the last call to grow() reserved"
        );
        // This method, to filter each doc's value, should only be invoked when
        // the cell crosses the query shape:
        java_assert!(matches!(
            self.last_compare_result.get(),
            None | Some(Relation::CellCrossesQuery)
        ));
        if self.last_compare_result.get().is_some() {
            // This doc's packed value should be contained in the last cell
            // passed to compare:
            for dim in 0..self.num_index_dims {
                let range = self.dim_range(dim);
                java_assert!(
                    self.last_min_packed_value.borrow()[range.clone()]
                        <= packed_value[range.clone()],
                    "dim={} of {} value={}",
                    dim,
                    self.num_data_dims,
                    BytesRef::new(packed_value.to_vec())
                );
                java_assert!(
                    self.last_max_packed_value.borrow()[range.clone()] >= packed_value[range],
                    "dim={} of {} value={}",
                    dim,
                    self.num_data_dims,
                    BytesRef::new(packed_value.to_vec())
                );
            }
            self.last_compare_result.set(None);
        }
        // TODO: we should assert that this "matches" whatever relation the last
        // call to compare had returned
        java_assert!(packed_value.len() as i32 == self.num_data_dims * self.bytes_per_dim);
        if self.num_data_dims == 1 {
            let bytes_per_dim = self.bytes_per_dim as usize;
            let last_doc_value = self
                .last_doc_value
                .as_mut()
                .expect("a 1D visitor keeps the last doc value");
            let cmp = last_doc_value[..bytes_per_dim].cmp(&packed_value[..bytes_per_dim]);
            match cmp {
                std::cmp::Ordering::Less => {} // ok
                std::cmp::Ordering::Equal => {
                    java_assert!(
                        self.last_doc_id <= doc_id,
                        "doc ids are out of order when point values are the same!"
                    );
                }
                std::cmp::Ordering::Greater => {
                    // out of order!
                    java_assert!(false, "point values are out of order");
                }
            }
            last_doc_value[..bytes_per_dim].copy_from_slice(&packed_value[..bytes_per_dim]);
            self.last_doc_id = doc_id;
        }
        self.r#in.visit_with_value(doc_id, packed_value)
    }

    fn grow(&mut self, count: i32) -> Result<()> {
        self.r#in.grow(count)?;
        self.doc_budget = count;
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        for dim in 0..self.num_index_dims {
            let range = self.dim_range(dim);
            java_assert!(min_packed_value[range.clone()] <= max_packed_value[range]);
        }
        let len = (self.num_index_dims * self.bytes_per_dim) as usize;
        self.last_max_packed_value.borrow_mut()[..len].copy_from_slice(&max_packed_value[..len]);
        self.last_min_packed_value.borrow_mut()[..len].copy_from_slice(&min_packed_value[..len]);
        let result = self.r#in.compare(min_packed_value, max_packed_value)?;
        self.last_compare_result.set(Some(result));
        Ok(result)
    }
}

// =============================================================================
// AssertingLeafReader
// =============================================================================

use rucene::index::index_reader::{CacheHelper, CacheKey, ClosedListener, StoredFields};
use rucene::index::leaf_reader::TermVectors;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, LeafReader, StoredFieldVisitor,
};
use rucene::search::knn::KnnCollector;
use rucene::search::AcceptDocs;
use rucene::util::bit_set::LiveDocs;
use rucene::util::Bits;

use crate::search::asserting_accept_docs::AssertingAcceptDocs;

/// Port of `org.apache.lucene.tests.index.AssertingLeafReader`: a
/// `FilterLeafReader` that applies additional checks for tests. It checks the
/// basic sanity of the wrapped reader when built, wraps every terms,
/// doc-values, points, stored-fields, term-vectors and live-docs view it hands
/// out in the asserting wrapper of this module, checks that a doc-values
/// view exists exactly when the field infos say it does, and wraps the
/// `AcceptDocs` of a vector search in an [`AssertingAcceptDocs`]. It does not
/// change the behaviour of the reader: it only validates the API.
///
/// **Divergence (closed-listener key check).** Java's constructor registers on
/// the wrapped reader's core and reader cache helpers a listener that asserts
/// it is called with the helper's own key (`expectedKey == cacheKey`, an
/// identity test; `AssertingLeafReader.java:88-112`). This port's
/// [`ClosedListener::on_close`] receives the [`CacheKey`] by value, a unit
/// value without identity (the declared divergence of
/// `rucene::index::filter_directory_reader::DelegatingCacheHelper`), so the
/// identity cannot be tested: the listeners are registered, as in Java, and
/// check nothing.
pub struct AssertingLeafReader {
    core: rucene::index::index_reader::IndexReaderCore,
    r#in: Arc<dyn LeafReader>,
}

/// The listener `AssertingLeafReader`'s constructor registers on the wrapped
/// reader's cache helpers; see the divergence on [`AssertingLeafReader`].
struct KeyCheckListener;

impl ClosedListener for KeyCheckListener {
    fn on_close(&self, _cache_key: CacheKey) -> Result<()> {
        Ok(())
    }
}

impl AssertingLeafReader {
    /// `new AssertingLeafReader(in)`: checks the basic sanity of `in` (Java's
    /// four `assert`s) and registers the closed listeners on its cache
    /// helpers.
    ///
    /// # Errors
    ///
    /// The `AssertionError` (no message) of a failed sanity `assert`.
    pub fn new(r#in: Arc<dyn LeafReader>) -> Result<Self> {
        let reader = Arc::clone(&r#in).as_index_reader();
        // check some basic reader sanity
        java_assert!(reader.max_doc() >= 0);
        java_assert!(reader.num_docs() <= reader.max_doc());
        java_assert!(reader.num_deleted_docs() + reader.num_docs() == reader.max_doc());
        java_assert!(
            !reader.has_deletions()
                || reader.num_deleted_docs() > 0 && reader.num_docs() < reader.max_doc()
        );
        if let Some(core_cache_helper) = r#in.get_core_cache_helper() {
            core_cache_helper.add_closed_listener(Box::new(KeyCheckListener));
        }
        if let Some(reader_cache_helper) = r#in.get_reader_cache_helper() {
            reader_cache_helper.add_closed_listener(Box::new(KeyCheckListener));
        }
        Ok(AssertingLeafReader {
            core: crate::index::filter_leaf_core(&r#in),
            r#in,
        })
    }

    /// `getDelegate()`: the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn LeafReader> {
        &self.r#in
    }

    /// `getFieldInfos().fieldInfo(field)`.
    fn field_info(&self, field: &str) -> Option<FieldInfo> {
        self.get_field_infos().field_info(field).cloned()
    }
}

impl fmt::Debug for AssertingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertingLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for AssertingLeafReader {
    /// `AssertingLeafReader` does not override `toString()`: the inherited
    /// `FilterLeafReader.toString()` renders `FilterLeafReader(<in>)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FilterLeafReader({})", self.r#in)
    }
}

impl LeafReader for AssertingLeafReader {
    forward_leaf_reader!(filter r#in;
        core,
        filter_leaf_reader_delegate,
        num_docs,
        max_doc,
        do_close,
        get_float_vector_values,
        get_byte_vector_values,
        get_field_infos,
        check_integrity,
        get_meta_data,
    );

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.core.ensure_open()?;
        Ok(self
            .r#in
            .terms(field)?
            .map(|terms| Box::new(AssertingTerms::new(terms)) as Box<dyn Terms>))
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.core.ensure_open()?;
        Ok(Box::new(AssertingTermVectors::new(
            self.r#in.term_vectors()?,
        )))
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.core.ensure_open()?;
        Ok(Box::new(AssertingStoredFields::new(
            self.r#in.stored_fields()?,
        )))
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.core.ensure_open()?;
        let dv = self.r#in.get_numeric_doc_values(field)?;
        let fi = self.field_info(field);
        match dv {
            Some(dv) => {
                java_assert!(fi.is_some());
                java_assert!(fi.as_ref().unwrap().get_doc_values_type() == DocValuesType::NUMERIC);
                Ok(Some(Box::new(AssertingNumericDocValues::new(
                    dv,
                    self.max_doc(),
                )?)))
            }
            None => {
                java_assert!(fi
                    .as_ref()
                    .is_none_or(|fi| fi.get_doc_values_type() != DocValuesType::NUMERIC));
                Ok(None)
            }
        }
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.core.ensure_open()?;
        let dv = self.r#in.get_binary_doc_values(field)?;
        let fi = self.field_info(field);
        match dv {
            Some(dv) => {
                java_assert!(fi.is_some());
                java_assert!(fi.as_ref().unwrap().get_doc_values_type() == DocValuesType::BINARY);
                Ok(Some(Box::new(AssertingBinaryDocValues::new(
                    dv,
                    self.max_doc(),
                )?)))
            }
            None => {
                java_assert!(fi
                    .as_ref()
                    .is_none_or(|fi| fi.get_doc_values_type() != DocValuesType::BINARY));
                Ok(None)
            }
        }
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.core.ensure_open()?;
        let dv = self.r#in.get_sorted_doc_values(field)?;
        let fi = self.field_info(field);
        match dv {
            Some(dv) => {
                java_assert!(fi.is_some());
                java_assert!(fi.as_ref().unwrap().get_doc_values_type() == DocValuesType::SORTED);
                Ok(Some(Box::new(AssertingSortedDocValues::new(
                    dv,
                    self.max_doc(),
                )?)))
            }
            None => {
                java_assert!(fi
                    .as_ref()
                    .is_none_or(|fi| fi.get_doc_values_type() != DocValuesType::SORTED));
                Ok(None)
            }
        }
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let fi = self.field_info(field);
        self.core.ensure_open()?;
        let dv = self.r#in.get_sorted_numeric_doc_values(field)?;
        match dv {
            Some(dv) => {
                java_assert!(fi.is_some());
                java_assert!(
                    fi.as_ref().unwrap().get_doc_values_type() == DocValuesType::SORTED_NUMERIC
                );
                Ok(Some(AssertingSortedNumericDocValues::create(
                    dv,
                    self.max_doc(),
                )?))
            }
            None => {
                java_assert!(fi
                    .as_ref()
                    .is_none_or(|fi| fi.get_doc_values_type() != DocValuesType::SORTED_NUMERIC));
                Ok(None)
            }
        }
    }

    /// Java wraps with the private constructor, not `create`
    /// (`AssertingLeafReader.java:1796-1808`): a singleton is wrapped as is.
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.core.ensure_open()?;
        let dv = self.r#in.get_sorted_set_doc_values(field)?;
        let fi = self.field_info(field);
        match dv {
            Some(dv) => {
                java_assert!(fi.is_some());
                java_assert!(
                    fi.as_ref().unwrap().get_doc_values_type() == DocValuesType::SORTED_SET
                );
                Ok(Some(Box::new(AssertingSortedSetDocValues::new(
                    dv,
                    self.max_doc(),
                )?)))
            }
            None => {
                java_assert!(fi
                    .as_ref()
                    .is_none_or(|fi| fi.get_doc_values_type() != DocValuesType::SORTED_SET));
                Ok(None)
            }
        }
    }

    /// Java's `assert fi.docValuesSkipIndexType() != NONE` dereferences `fi`
    /// without a null check: under `-ea`, a skipper for a field the infos do
    /// not know throws the `NullPointerException`.
    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.core.ensure_open()?;
        let skipper = self.r#in.get_doc_values_skipper(field)?;
        let fi = self.field_info(field);
        match skipper {
            Some(skipper) => {
                if cfg!(debug_assertions) {
                    let Some(fi) = fi.as_ref() else {
                        return Err(npe());
                    };
                    java_assert!(fi.doc_values_skip_index_type() != DocValuesSkipIndexType::NONE);
                }
                Ok(Some(Box::new(AssertingDocValuesSkipper::new(skipper)?)))
            }
            None => {
                java_assert!(fi.as_ref().is_none_or(
                    |fi| fi.doc_values_skip_index_type() == DocValuesSkipIndexType::NONE
                ));
                Ok(None)
            }
        }
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.core.ensure_open()?;
        let dv = self.r#in.get_norm_values(field)?;
        let fi = self.field_info(field);
        match dv {
            Some(dv) => {
                java_assert!(fi.is_some());
                java_assert!(fi.as_ref().unwrap().has_norms());
                Ok(Some(Box::new(AssertingNumericDocValues::new(
                    dv,
                    self.max_doc(),
                )?)))
            }
            None => {
                java_assert!(fi.as_ref().is_none_or(|fi| !fi.has_norms()));
                Ok(None)
            }
        }
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        match self.r#in.get_point_values(field)? {
            None => Ok(None),
            Some(values) => Ok(Some(Box::new(AssertingPointValues::new(
                values,
                self.max_doc(),
            )?))),
        }
    }

    /// `liveDocs instanceof LiveDocs` is asked of the wrapped reader's bits
    /// with [`Bits::into_live_docs`], which consumes them; when they are not a
    /// [`LiveDocs`], the plain bits are asked for again (a getter without side
    /// effects) and wrapped in an [`AssertingBits`].
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        crate::index::ensure_open_unchecked(&self.core);
        match self.r#in.get_live_docs() {
            Some(live_docs) => {
                java_assert_unchecked!(self.max_doc() as usize == live_docs.length());
                match live_docs.into_live_docs() {
                    Some(live_docs) => Some(Box::new(AssertingLiveDocs::new(live_docs))),
                    None => self
                        .r#in
                        .get_live_docs()
                        .map(|bits| Box::new(AssertingBits::new(bits)) as Box<dyn Bits>),
                }
            }
            None => {
                java_assert_unchecked!(self.max_doc() == self.num_docs());
                java_assert_unchecked!(!Arc::clone(&self.r#in).as_index_reader().has_deletions());
                None
            }
        }
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        let mut accept_docs = AssertingAcceptDocs::wrap(accept_docs);
        self.r#in
            .search_nearest_vectors(field, target, collector, &mut accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        let mut accept_docs = AssertingAcceptDocs::wrap(accept_docs);
        self.r#in
            .search_nearest_vectors_byte(field, target, collector, &mut accept_docs)
    }

    // we don't change behavior of the reader: just validate the API.

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_reader_cache_helper()
    }
}

// =============================================================================
// AssertingStoredFields
// =============================================================================

/// Port of `AssertingLeafReader.AssertingStoredFields`: wraps a
/// [`StoredFields`] but with additional asserts (the consuming thread).
pub struct AssertingStoredFields {
    r#in: Box<dyn StoredFields>,
    creation_thread: Thread,
}

impl AssertingStoredFields {
    /// `new AssertingStoredFields(in)`.
    pub fn new(r#in: Box<dyn StoredFields>) -> Self {
        AssertingStoredFields {
            r#in,
            creation_thread: Thread::current(),
        }
    }
}

impl fmt::Debug for AssertingStoredFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertingStoredFields")
            .field("in", &self.r#in)
            .finish_non_exhaustive()
    }
}

impl StoredFields for AssertingStoredFields {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        assert_thread("StoredFields", &self.creation_thread)?;
        self.r#in.prefetch(doc_id)
    }

    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        assert_thread("StoredFields", &self.creation_thread)?;
        self.r#in.document_with_visitor(doc_id, visitor)
    }
}

// =============================================================================
// AssertingTermVectors
// =============================================================================

/// Port of `AssertingLeafReader.AssertingTermVectors`: wraps a
/// [`TermVectors`] but with additional asserts (the consuming thread), and
/// wraps the [`Fields`] it returns in an [`AssertingFields`].
pub struct AssertingTermVectors {
    r#in: Box<dyn TermVectors>,
    creation_thread: Thread,
}

impl AssertingTermVectors {
    /// `new AssertingTermVectors(in)`.
    pub fn new(r#in: Box<dyn TermVectors>) -> Self {
        AssertingTermVectors {
            r#in,
            creation_thread: Thread::current(),
        }
    }
}

impl fmt::Debug for AssertingTermVectors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertingTermVectors")
            .field("in", &self.r#in)
            .finish_non_exhaustive()
    }
}

impl TermVectors for AssertingTermVectors {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        assert_thread("TermVectors", &self.creation_thread)?;
        self.r#in.prefetch(doc_id)
    }

    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        assert_thread("TermVectors", &self.creation_thread)?;
        Ok(self
            .r#in
            .get(doc)?
            .map(|fields| Box::new(AssertingFields::new(fields)) as Box<dyn Fields>))
    }
}

// =============================================================================
// AssertingBits / AssertingLiveDocs
// =============================================================================

/// Port of `AssertingLeafReader.AssertingBits`: wraps a [`Bits`] but with
/// additional asserts (the consuming thread, and an index in range).
pub struct AssertingBits {
    creation_thread: Thread,
    r#in: Box<dyn Bits>,
}

impl AssertingBits {
    /// `new AssertingBits(in)`.
    pub fn new(r#in: Box<dyn Bits>) -> Self {
        AssertingBits {
            creation_thread: Thread::current(),
            r#in,
        }
    }
}

impl fmt::Debug for AssertingBits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertingBits")
            .field("in", &self.r#in)
            .finish_non_exhaustive()
    }
}

impl Bits for AssertingBits {
    /// Java's `assert index >= 0 && index < length()`: an `index` is never
    /// negative in this port's `usize` signature.
    fn get(&self, index: usize) -> bool {
        assert_thread_unchecked("Bits", &self.creation_thread);
        java_assert_unchecked!(index < self.length());
        self.r#in.get(index)
    }

    fn length(&self) -> usize {
        assert_thread_unchecked("Bits", &self.creation_thread);
        self.r#in.length()
    }
}

/// Port of `AssertingLeafReader.AssertingLiveDocs` (`extends AssertingBits
/// implements LiveDocs`): the [`AssertingBits`] of a [`LiveDocs`], which also
/// checks the deleted count.
struct AssertingLiveDocs {
    creation_thread: Thread,
    live_docs: Box<dyn LiveDocs>,
}

impl AssertingLiveDocs {
    fn new(live_docs: Box<dyn LiveDocs>) -> Self {
        AssertingLiveDocs {
            creation_thread: Thread::current(),
            live_docs,
        }
    }
}

impl fmt::Debug for AssertingLiveDocs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertingLiveDocs")
            .field("in", &self.live_docs)
            .finish_non_exhaustive()
    }
}

impl Bits for AssertingLiveDocs {
    /// `AssertingBits.get(index)`.
    fn get(&self, index: usize) -> bool {
        assert_thread_unchecked("Bits", &self.creation_thread);
        java_assert_unchecked!(index < self.length());
        self.live_docs.get(index)
    }

    /// `AssertingBits.length()`.
    fn length(&self) -> usize {
        assert_thread_unchecked("Bits", &self.creation_thread);
        self.live_docs.length()
    }

    /// This object is a `LiveDocs`: Java's `instanceof LiveDocs` on it is
    /// `true`.
    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn LiveDocs>> {
        Some(self)
    }
}

impl LiveDocs for AssertingLiveDocs {
    fn deleted_count(&self) -> i32 {
        let count = self.live_docs.deleted_count();
        java_assert_unchecked!(count >= 0);
        java_assert_unchecked!(count as usize <= self.length());
        count
    }

    fn live_docs_iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        self.live_docs.live_docs_iterator()
    }

    fn deleted_docs_iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        self.live_docs.deleted_docs_iterator()
    }
}
