//! Enumerating the terms similar to a filter term, ported from
//! `org.apache.lucene.search.FuzzyTermsEnum`.

#![deny(unsafe_code)]

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{LuceneError, Result};
use crate::index::{ImpactsEnum, PostingsEnum, SeekStatus, Term, TermState, Terms, TermsEnum};
use crate::search::boost_attribute::{add_boost_attribute, boost_of, set_boost};
use crate::search::fuzzy_automaton_builder::FuzzyAutomatonBuilder;
use crate::search::max_non_competitive_boost_attribute::{
    add_max_non_competitive_boost_attribute, competitive_term_of, max_non_competitive_boost_of,
    set_max_non_competitive_boost,
};
use crate::util::attribute::{Attribute, AttributeImpl, AttributeReflector, AttributeSource};
use crate::util::automaton::ByteRunnable;
use crate::util::automaton::CompiledAutomaton;
use crate::util::{BytesRef, BytesRefBuilder, UnicodeUtil};

/// The Levenshtein automata of a fuzzy term, shared between segments.
///
/// Equivalent to the state of the private
/// `FuzzyTermsEnum.AutomatonAttributeImpl`.
#[derive(Debug, Clone)]
pub struct AutomatonSet {
    /// One compiled automaton per edit distance, from `0` to the maximum.
    pub automata: Vec<CompiledAutomaton>,
    /// The number of code points of the fuzzy term.
    pub term_length: i32,
}

/// Shares the Levenshtein automata between segments.
///
/// Equivalent to the private `FuzzyTermsEnum.AutomatonAttribute`. Levenshtein
/// automata are large and expensive to build; they must not be built directly
/// on the query, because that would blow up caches keyed by queries, and they
/// should not be rebuilt for every segment. This attribute lets the enum build
/// them once, for its first segment, and share them with the later ones.
pub trait AutomatonAttribute: Attribute {
    /// Returns the shared automata, or `None` before
    /// [`init`](Self::init) has run.
    ///
    /// Equivalent to `AutomatonAttribute.getAutomata()` together with
    /// `getTermLength()`.
    fn get_automaton_set(&self) -> Option<AutomatonSet>;

    /// Builds the automata, unless they have already been built.
    ///
    /// Equivalent to
    /// `AutomatonAttribute.init(Supplier<FuzzyAutomatonBuilder>)`.
    ///
    /// # Errors
    ///
    /// Propagates the error the builder raises.
    fn init(&mut self, builder: &dyn Fn() -> Result<FuzzyAutomatonBuilder>) -> Result<()>;
}

/// Implementation class for [`AutomatonAttribute`].
///
/// Equivalent to the private `FuzzyTermsEnum.AutomatonAttributeImpl`.
///
/// **Divergence from Lucene 10.5.0.** The automata live behind an
/// `Arc<Mutex<..>>` rather than in a plain field, because this port's
/// [`CompiledAutomaton`] may hold an
/// [`NFARunAutomaton`](crate::util::automaton::NFARunAutomaton) whose
/// determinization cache is a `RefCell`, so it is not `Sync` while an
/// [`AttributeImpl`] must be. The lock is taken only when the automata are
/// built and when the enum takes its copy of them, never per term. The `Arc`
/// makes a clone of the attribute share the automata, which is what Java's
/// reference semantics give.
#[derive(Debug, Clone, Default)]
pub struct AutomatonAttributeImpl {
    automata: Arc<Mutex<Option<AutomatonSet>>>,
}

impl AutomatonAttributeImpl {
    /// Creates an attribute with no automata yet.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Attribute for AutomatonAttributeImpl {}

impl AutomatonAttribute for AutomatonAttributeImpl {
    fn get_automaton_set(&self) -> Option<AutomatonSet> {
        self.automata
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn init(&mut self, builder: &dyn Fn() -> Result<FuzzyAutomatonBuilder>) -> Result<()> {
        let mut guard = self
            .automata
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_some() {
            return Ok(());
        }
        let builder = builder()?;
        *guard = Some(AutomatonSet {
            term_length: builder.get_term_length(),
            automata: builder.build_automaton_set()?,
        });
        Ok(())
    }
}

impl AttributeImpl for AutomatonAttributeImpl {
    fn clear(&mut self) {
        *self
            .automata
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    fn copy_to(&self, _target: &mut dyn AttributeImpl) {
        // Java throws `UnsupportedOperationException`; `copy_to` cannot report
        // an error, so the copy is simply not performed. Nothing captures or
        // restores the state of this attribute.
    }

    fn reflect_with(&self, _reflector: &mut dyn AttributeReflector) {
        // Java throws `UnsupportedOperationException`; see `copy_to`.
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static IDS: OnceLock<&'static [TypeId]> = OnceLock::new();
        IDS.get_or_init(|| {
            let ids = vec![
                TypeId::of::<AutomatonAttributeImpl>(),
                TypeId::of::<dyn AutomatonAttribute>(),
            ];
            Box::leak(ids.into_boxed_slice())
        })
    }
}

/// Builds the automata into `atts`, sharing them across the segments of one
/// rewrite, and returns them.
///
/// Equivalent to the three lines of the private `FuzzyTermsEnum` constructor
/// that add the [`AutomatonAttribute`] to the shared attribute source and call
/// `init`.
///
/// # Errors
///
/// Propagates the error the builder raises.
pub fn shared_automata(
    atts: &mut AttributeSource,
    builder: &dyn Fn() -> Result<FuzzyAutomatonBuilder>,
) -> Result<AutomatonSet> {
    if !atts.has_attribute::<AutomatonAttributeImpl>() {
        atts.add_attribute_impl_instance(Box::new(AutomatonAttributeImpl::new()));
    }
    {
        let mut aa = atts
            .get_attribute_mut::<AutomatonAttributeImpl>()
            .ok_or_else(|| {
                LuceneError::IllegalState(
                    "the AutomatonAttribute was just installed and must be present".to_string(),
                )
            })?;
        aa.init(builder)?;
    }
    let aa = atts
        .get_attribute::<AutomatonAttributeImpl>()
        .ok_or_else(|| {
            LuceneError::IllegalState(
                "the AutomatonAttribute was just installed and must be present".to_string(),
            )
        })?;
    aa.get_automaton_set().ok_or_else(|| {
        LuceneError::IllegalState("the AutomatonAttribute was just initialised".to_string())
    })
}

/// A [`TermsEnum`] enumerating all terms similar to a filter term.
///
/// Equivalent to the `final org.apache.lucene.search.FuzzyTermsEnum`, which
/// extends `BaseTermsEnum`. Term enumerations are always ordered by the byte
/// ordering of [`BytesRef`]: each term in the enumeration is greater than all
/// that precede it.
pub struct FuzzyTermsEnum {
    // NOTE: this cannot be a `FilteredTermsEnum`, because the actual enum
    // sometimes has to change.
    actual_enum: Box<dyn TermsEnum>,
    atts: AttributeSource,
    /// **Divergence from Lucene 10.5.0.** Java shares the `CompiledAutomaton[]`
    /// with the attribute by reference; this port takes a copy, because the
    /// attribute has to keep them behind a lock — see
    /// [`AutomatonAttributeImpl`] — and locking per term would be paid on the
    /// hot path. Building the automata, which is the expensive part, still
    /// happens once per rewrite.
    automata: Vec<CompiledAutomaton>,
    terms: Arc<dyn Terms>,
    term_length: i32,
    term: Term,
    bottom: f32,
    bottom_term: Option<BytesRef>,
    queued_bottom: Option<BytesRef>,
    /// The maximum number of edits accepted. It starts as the `2` or `1` — or,
    /// degenerately, `0` — the user passed, and drops as terms are collected:
    /// when the term queue is full and every collected term is at edit distance
    /// `1`, the automaton can be reduced.
    max_edits: i32,
}

impl std::fmt::Debug for FuzzyTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuzzyTermsEnum")
            .field("term", &self.term)
            .field("max_edits", &self.max_edits)
            .finish_non_exhaustive()
    }
}

impl FuzzyTermsEnum {
    /// Enumerates all the terms of `terms` that share a prefix of length
    /// `prefix_length` with `term` and are at most `max_edits` edits away.
    ///
    /// Equivalent to the public
    /// `FuzzyTermsEnum(Terms, Term, int, int, boolean)`, which builds its own
    /// attribute source. After the call the enumeration already points at the
    /// first valid term, if such a term exists.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction error and any I/O error raised
    /// while intersecting the terms dictionary.
    pub fn new(
        terms: Arc<dyn Terms>,
        term: Term,
        max_edits: i32,
        prefix_length: i32,
        transpositions: bool,
    ) -> Result<Self> {
        let mut atts = AttributeSource::new();
        Self::with_attributes(
            terms,
            &mut atts,
            term,
            max_edits,
            prefix_length,
            transpositions,
        )
    }

    /// Enumerates the similar terms, sharing the automata through `atts`.
    ///
    /// Equivalent to the package-private
    /// `FuzzyTermsEnum(Terms, AttributeSource, Term, int, int, boolean)`, whose
    /// attribute source is shared by all the segments of one rewrite.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction error and any I/O error raised
    /// while intersecting the terms dictionary.
    pub fn with_attributes(
        terms: Arc<dyn Terms>,
        atts: &mut AttributeSource,
        term: Term,
        max_edits: i32,
        prefix_length: i32,
        transpositions: bool,
    ) -> Result<Self> {
        let text = term.text();
        let build =
            move || FuzzyAutomatonBuilder::new(&text, max_edits, prefix_length, transpositions);

        add_max_non_competitive_boost_attribute(atts);
        add_boost_attribute(atts);
        let automaton_set = shared_automata(atts, &build)?;

        // Java holds the very `AttributeSource` it was given; this port's
        // `new_from` shares the attribute instances with it, which is what the
        // rewrite methods read and write.
        let atts = AttributeSource::new_from(atts);

        let automata = automaton_set.automata;
        let term_length = automaton_set.term_length;
        let max_edits = automata.len() as i32 - 1;

        let bottom = max_non_competitive_boost_of(&atts);
        let bottom_term = competitive_term_of(&atts);

        // `bottomChanged(null)` always installs an enum, so a placeholder is
        // never observed; `terms.iterator()` is the cheapest one to build.
        let mut enumeration = Self {
            actual_enum: terms.iterator()?,
            atts,
            automata,
            terms,
            term_length,
            term,
            bottom,
            bottom_term,
            queued_bottom: None,
            max_edits,
        };
        enumeration.bottom_changed(None)?;
        Ok(enumeration)
    }

    /// Sets the maximum non-competitive boost, which may allow switching to a
    /// lower max-edit automaton at run time.
    ///
    /// Equivalent to `FuzzyTermsEnum.setMaxNonCompetitiveBoost(float)`.
    pub fn set_max_non_competitive_boost(&mut self, boost: f32) {
        let competitive_term = competitive_term_of(&self.atts);
        set_max_non_competitive_boost(&mut self.atts, boost, competitive_term);
    }

    /// Returns the boost of the current term.
    ///
    /// Equivalent to `FuzzyTermsEnum.getBoost()`.
    pub fn get_boost(&self) -> f32 {
        boost_of(&self.atts)
    }

    /// Returns an automaton-based enum matching up to `edit_distance` edits
    /// from `last_term`, if possible.
    ///
    /// Equivalent to the private
    /// `FuzzyTermsEnum.getAutomatonEnum(int, BytesRef)`.
    fn get_automaton_enum(
        &self,
        edit_distance: i32,
        last_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        debug_assert!((edit_distance as usize) < self.automata.len());
        let compiled = &self.automata[edit_distance as usize];
        let initial_seek_term = match last_term {
            // This is the first enum being pulled.
            None => None,
            // This enum — `ed=1`, say — is pulled after iterating for a while
            // already at `ed=2`.
            Some(last_term) => {
                let mut builder = BytesRefBuilder::new();
                compiled.floor(last_term, &mut builder)
            }
        };
        self.terms.intersect(compiled, initial_seek_term.as_ref())
    }

    /// Fired when the max non-competitive boost has changed; this is the hook
    /// that swaps in a smarter actual enum.
    ///
    /// Equivalent to the private
    /// `FuzzyTermsEnum.bottomChanged(BytesRef)`.
    fn bottom_changed(&mut self, last_term: Option<&BytesRef>) -> Result<()> {
        let old_max_edits = self.max_edits;

        // True if the last term encountered is lexicographically equal to or
        // after the bottom term in the priority queue.
        let term_after = match (self.bottom_term.as_ref(), last_term) {
            (None, _) => true,
            (Some(bottom_term), Some(last_term)) => last_term >= bottom_term,
            (Some(_), None) => false,
        };

        // As long as the max non-competitive boost is >= the max boost for some
        // edit distance, keep dropping the max edit distance.
        while self.max_edits > 0 {
            let max_boost = 1.0f32 - (self.max_edits as f32 / self.term_length as f32);
            if self.bottom < max_boost || (self.bottom == max_boost && !term_after) {
                break;
            }
            self.max_edits -= 1;
        }

        if old_max_edits != self.max_edits || last_term.is_none() {
            // This is a very powerful optimization: the maximum edit distance
            // has changed. It happens because only the top scoring N terms are
            // collected — 50 by default — so when `maxEdits=2`, the queue is
            // full of matching terms and the worst entry in that queue is at
            // `ed=1`, the automaton can be switched to `ed=1`, which is a big
            // speed-up.
            self.actual_enum = self.get_automaton_enum(self.max_edits, last_term)?;
        }
        Ok(())
    }

    /// Returns `true` if `term_in` is within `k` edits of the query term.
    ///
    /// Equivalent to the private
    /// `FuzzyTermsEnum.matches(BytesRef, int)`.
    fn matches(&self, term_in: &BytesRef, k: i32) -> bool {
        if k == 0 {
            return term_in == self.term.bytes();
        }
        match self.automata[k as usize].run_automaton.as_ref() {
            Some(run_automaton) => {
                run_automaton.run_range(&term_in.bytes, term_in.offset, term_in.length)
            }
            // A compiled Levenshtein automaton is always `NORMAL` and therefore
            // always carries a run automaton; Java would raise a
            // NullPointerException here.
            None => false,
        }
    }
}

impl TermsEnum for FuzzyTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if let Some(queued_bottom) = self.queued_bottom.take() {
            self.bottom_changed(Some(&queued_bottom))?;
        }

        let Some(term) = self.actual_enum.next()? else {
            // End.
            return Ok(None);
        };

        let mut ed = self.max_edits;

        // The outer DFA always matches; now compute the exact edit distance.
        while ed > 0 {
            if self.matches(&term, ed - 1) {
                ed -= 1;
            } else {
                break;
            }
        }

        if ed == 0 {
            // Exact match.
            set_boost(&mut self.atts, 1.0);
        } else {
            let code_point_count = UnicodeUtil::code_point_count(&term)? as i32;
            let min_term_length = code_point_count.min(self.term_length);
            let similarity = 1.0f32 - (ed as f32 / min_term_length as f32);
            set_boost(&mut self.atts, similarity);
        }

        let bottom = max_non_competitive_boost_of(&self.atts);
        let bottom_term = competitive_term_of(&self.atts);
        if bottom != self.bottom || bottom_term != self.bottom_term {
            self.bottom = bottom;
            self.bottom_term = bottom_term;
            // Clone the term before potentially doing something with it; this
            // is a rare but wonderful occurrence anyway.
            //
            // `bottom_changed` must be delayed until the next `next()` call,
            // otherwise the doc frequency and the rest of the current term's
            // statistics are lost.
            self.queued_bottom = Some(BytesRef::deep_copy_of(&term));
        }

        Ok(Some(term))
    }

    // Proxy all other enum calls to the actual enum.

    fn doc_freq(&self) -> Result<i32> {
        self.actual_enum.doc_freq()
    }

    fn total_term_freq(&self) -> Result<i64> {
        self.actual_enum.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.actual_enum.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.actual_enum.impacts(flags)
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.actual_enum.seek_term_state(text, state)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.actual_enum.term_state()
    }

    fn ord(&self) -> Result<i64> {
        self.actual_enum.ord()
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.actual_enum.seek_exact(text)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.actual_enum.seek_ceil(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.actual_enum.seek_ord(ord)
    }

    fn term(&self) -> Result<BytesRef> {
        self.actual_enum.term()
    }
}
