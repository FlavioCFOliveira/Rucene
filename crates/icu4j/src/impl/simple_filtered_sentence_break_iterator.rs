// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.SimpleFilteredSentenceBreakIterator`, with its nested
//! class `Builder`.

use std::any::Any;

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::{
    java_identity_hash_code_stand_in, JavaHashCode, JavaObjectHashSet,
};
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::index_of_unit;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{ICUResourceBundle, OpenType};
use crate::text::break_iterator::{BreakIterator, BreakIteratorLocales, DONE};
use crate::text::filtered_break_iterator_builder::FilteredBreakIteratorBuilder;
use crate::text::rule_based_break_iterator::RuleBasedBreakIterator;
use crate::text::u_character_iterator::{get_instance_character_iterator, UCharacterIterator};
use crate::util::bytes_trie::Result as TrieResult;
use crate::util::chars_trie::CharsTrie;
use crate::util::chars_trie_builder::CharsTrieBuilder;
use crate::util::string_trie_builder::StringTrieBuilderOption;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// A sentence break iterator that suppresses the breaks after the strings
/// of a filter (abbreviations).
///
/// Equivalent to the class
/// `com.ibm.icu.impl.SimpleFilteredSentenceBreakIterator`.
#[derive(Debug)]
pub struct SimpleFilteredSentenceBreakIterator {
    delegate: Box<dyn BreakIterator>,
    text: Option<Box<dyn UCharacterIterator>>,
    backwards_trie: Option<CharsTrie>, // i.e. ".srM" for Mrs.
    forwards_partial_trie: Option<CharsTrie>, // Has ".a" for "a.M."
    locales: BreakIteratorLocales,
}

impl SimpleFilteredSentenceBreakIterator {
    /// `SimpleFilteredSentenceBreakIterator(BreakIterator adoptBreakIterator,
    /// CharsTrie forwardsPartialTrie, CharsTrie backwardsTrie)`.
    pub fn new(
        adopt_break_iterator: Box<dyn BreakIterator>,
        forwards_partial_trie: Option<CharsTrie>,
        backwards_trie: Option<CharsTrie>,
    ) -> Self {
        Self {
            delegate: adopt_break_iterator,
            text: None,
            forwards_partial_trie,
            backwards_trie,
            locales: BreakIteratorLocales::default(),
        }
    }

    /// `resetState()`: reset the filter from the delegate.
    fn reset_state(&mut self) {
        let text = self
            .delegate
            .get_text()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: delegate.getText()"))
            .clone_box();
        self.text = Some(get_instance_character_iterator(text));
    }

    /// `breakExceptionAt(int n)`: whether the break at `n` is suppressed.
    fn break_exception_at(&mut self, n: i32) -> bool {
        // Note: the C++ version of this function is SimpleFilteredSentenceBreakIterator::breakExceptionAt()

        let mut best_posn = -1;
        let mut best_value = -1;

        // loops while 'n' points to an exception
        let text = self.text.as_deref_mut().expect("resetState() set the text");
        text.set_index(n);
        let backwards_trie = self
            .backwards_trie
            .as_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: backwardsTrie"));
        backwards_trie.reset();
        let mut uch;

        // Assume a space is following the '.' (so we handle the case: "Mr. /Brown")
        uch = text.previous_code_point();
        if uch == ' ' as i32 {
            // TODO: skip a class of chars here??
            // TODO only do this the 1st time?
        } else {
            uch = text.next_code_point();
        }
        let _ = uch;

        loop {
            uch = text.previous_code_point();
            if uch < 0 {
                break;
            }
            let r = backwards_trie.next_for_code_point(uch);
            if r.has_value() {
                // remember the best match so far
                best_posn = text.get_index();
                best_value = backwards_trie.get_value();
            }
            if !r.has_next() {
                break;
            }
        }
        backwards_trie.reset(); // for equals & hashCode

        if best_posn >= 0 {
            if best_value == Builder::MATCH {
                // exact match!
                return true; // Exception here.
            } else if best_value == Builder::PARTIAL {
                if let Some(forwards_partial_trie) = self.forwards_partial_trie.as_mut() {
                    // make sure there's a forward trie
                    // We matched the "Ph." in "Ph.D." - now we need to run everything through the forwards trie
                    // to see if it matches something going forward.
                    forwards_partial_trie.reset();

                    let mut rfwd = TrieResult::IntermediateValue;
                    text.set_index(best_posn); // hope that's close ..
                    loop {
                        uch = text.next_code_point();
                        if uch == DONE {
                            break;
                        }
                        rfwd = forwards_partial_trie.next_for_code_point(uch);
                        if !rfwd.has_next() {
                            break;
                        }
                    }
                    forwards_partial_trie.reset(); // for equals & hashCode
                    if rfwd.matches() {
                        // Exception here
                        return true;
                    } // else fall through
                } // else fall through
            } // else fall through
        } // else fall through
        false // No exception here.
    }

    /// `internalNext(int n)`: moves forward past the suppressed breaks.
    fn internal_next(&mut self, mut n: i32) -> i32 {
        if n == DONE || // at end or
            self.backwards_trie.is_none()
        {
            // .. no backwards table loaded == no exceptions
            return n;
        }
        self.reset_state();

        let text_len = self
            .text
            .as_deref()
            .expect("resetState() set the text")
            .get_length();

        while n != DONE && n != text_len {
            // outer loop runs once per underlying break (from fDelegate).
            // loops while 'n' points to an exception.

            if self.break_exception_at(n) {
                // n points to a break exception
                n = self.delegate.next();
            } else {
                // no exception at this spot
                return n;
            }
        }
        n // hit underlying DONE or break at end of text
    }

    /// `internalPrev(int n)`: moves backwards past the suppressed breaks.
    fn internal_prev(&mut self, mut n: i32) -> i32 {
        if n == 0 || n == DONE || // at end or
            self.backwards_trie.is_none()
        {
            // .. no backwards table loaded == no exceptions
            return n;
        }
        self.reset_state();

        while n != DONE && n != 0 {
            // outer loop runs once per underlying break (from fDelegate).
            // loops while 'n' points to an exception.

            if self.break_exception_at(n) {
                // n points to a break exception
                n = self.delegate.previous();
            } else {
                // no exception at this spot
                return n;
            }
        }
        n // hit underlying DONE or break at end of text
    }

    /// `equals(Object obj)`.
    ///
    /// Java compares the delegates, then the `UCharacterIterator`s with
    /// `Object.equals` (identity: `CharacterIteratorWrapper` does not
    /// override it), then the tries (identity too): two distinct iterators
    /// are never equal once the delegates match.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the delegates are equal and this
    /// iterator's text was never reset (Java dereferences `text`).
    pub fn java_equals(&self, obj: &dyn Any) -> bool {
        let Some(other) = obj.downcast_ref::<SimpleFilteredSentenceBreakIterator>() else {
            return false;
        };
        if std::ptr::eq(self, other) {
            return true;
        }
        let delegate_any = other.delegate.as_any();
        let delegates_equal = if let Some(rbbi) = self
            .delegate
            .as_any()
            .downcast_ref::<RuleBasedBreakIterator>()
        {
            rbbi.java_equals(delegate_any)
        } else if let Some(sfsbi) = self
            .delegate
            .as_any()
            .downcast_ref::<SimpleFilteredSentenceBreakIterator>()
        {
            sfsbi.java_equals(delegate_any)
        } else {
            false
        };
        if !delegates_equal {
            return false;
        }
        if self.text.is_none() {
            panic!("java.lang.NullPointerException: text");
        }
        // `text.equals(other.text)`: identity of distinct wrappers.
        false
    }

    /// `hashCode()`: `forwardsPartialTrie.hashCode() * 39 +
    /// backwardsTrie.hashCode() * 11 + delegate.hashCode()`.
    ///
    /// **Divergence.** The tries have Java's identity hash
    /// (`SimpleFilteredSentenceBreakIterator.java`, `hashCode`), which no
    /// Rust value has; the port uses the crate's stable stand-ins for them.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when a trie is absent.
    pub fn hash_code(&self) -> i32 {
        if self.forwards_partial_trie.is_none() || self.backwards_trie.is_none() {
            panic!("java.lang.NullPointerException: trie");
        }
        let delegate_hash = if let Some(rbbi) = self
            .delegate
            .as_any()
            .downcast_ref::<RuleBasedBreakIterator>()
        {
            rbbi.hash_code()
        } else if let Some(sfsbi) = self
            .delegate
            .as_any()
            .downcast_ref::<SimpleFilteredSentenceBreakIterator>()
        {
            sfsbi.hash_code()
        } else {
            java_identity_hash_code_stand_in("com.ibm.icu.text.BreakIterator")
        };
        java_identity_hash_code_stand_in("forwardsPartialTrie")
            .wrapping_mul(39)
            .wrapping_add(java_identity_hash_code_stand_in("backwardsTrie").wrapping_mul(11))
            .wrapping_add(delegate_hash)
    }
}

impl BreakIterator for SimpleFilteredSentenceBreakIterator {
    /// `clone()`: copies the delegate, the text and the tries.
    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(SimpleFilteredSentenceBreakIterator {
            delegate: self.delegate.clone_box(),
            text: self.text.as_ref().map(|t| t.clone_box()),
            backwards_trie: self.backwards_trie.clone(),
            forwards_partial_trie: self.forwards_partial_trie.clone(),
            locales: self.locales.clone(),
        })
    }

    fn first(&mut self) -> i32 {
        // Don't suppress a break opportunity at the beginning of text.
        self.delegate.first()
    }

    fn preceding(&mut self, offset: i32) -> i32 {
        let n = self.delegate.preceding(offset);
        self.internal_prev(n)
    }

    fn previous(&mut self) -> i32 {
        let n = self.delegate.previous();
        self.internal_prev(n)
    }

    fn current(&self) -> i32 {
        self.delegate.current()
    }

    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        if !self.delegate.is_boundary(offset)? {
            return Ok(false); // No underlying break to suppress?
        }

        // delegate thinks there's a break…
        if self.backwards_trie.is_none() {
            return Ok(true); // no data
        }

        self.reset_state();
        Ok(!self.break_exception_at(offset)) // if there's an exception: no break.
    }

    fn next(&mut self) -> i32 {
        let n = self.delegate.next();
        self.internal_next(n)
    }

    fn next_n(&mut self, n: i32) -> i32 {
        let r = self.delegate.next_n(n);
        self.internal_next(r)
    }

    fn following(&mut self, offset: i32) -> i32 {
        let n = self.delegate.following(offset);
        self.internal_next(n)
    }

    fn last(&mut self) -> i32 {
        // Don't suppress a break opportunity at the end of text.
        self.delegate.last()
    }

    fn get_text(&self) -> Option<&dyn CharacterIterator> {
        self.delegate.get_text()
    }

    fn set_text(&mut self, new_text: Option<Box<dyn CharacterIterator>>) {
        self.delegate.set_text(new_text);
    }

    fn locales(&self) -> &BreakIteratorLocales {
        &self.locales
    }

    fn locales_mut(&mut self) -> &mut BreakIteratorLocales {
        &mut self.locales
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `CharSequence` element of the filter set: UTF-16 units hashed as
/// `String.hashCode()`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilterString(Vec<u16>);

impl JavaHashCode for FilterString {
    fn java_hash_code(&self) -> i32 {
        self.0
            .iter()
            .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(c as i32))
    }
}

/// `StringBuilder.reverse()`: reverses the units, keeping surrogate pairs
/// in order.
fn java_reverse(s: &[u16]) -> Vec<u16> {
    let mut v: Vec<u16> = s.iter().rev().copied().collect();
    let mut i = 0;
    while i + 1 < v.len() {
        if crate::jdk::is_low_surrogate(v[i]) && crate::jdk::is_high_surrogate(v[i + 1]) {
            v.swap(i, i + 1);
            i += 1;
        }
        i += 1;
    }
    v
}

/// `String.regionMatches(0, other, 0, len)`.
fn region_matches(this: &[u16], other: &[u16], len: usize) -> bool {
    this.len() >= len && other.len() >= len && this[..len] == other[..len]
}

/// The builder of [`SimpleFilteredSentenceBreakIterator`]s.
///
/// Equivalent to the public static nested class
/// `SimpleFilteredSentenceBreakIterator.Builder`.
#[derive(Debug, Default)]
pub struct Builder {
    /// `filterSet`: the set of strings that may be followed by an exception.
    filter_set: JavaObjectHashSet<FilterString>,
}

impl Builder {
    /// `PARTIAL`: may be part of a match.
    pub(crate) const PARTIAL: i32 = 1 << 0;
    /// `MATCH`: exact match.
    pub(crate) const MATCH: i32 = 1 << 1;
    /// `SuppressInReverse`: this partial match is also a backward match.
    const SUPPRESS_IN_REVERSE: i32 = 1 << 0;
    /// `AddToForward`: this partial match is also a forward match.
    const ADD_TO_FORWARD: i32 = 1 << 1;

    /// `Builder(Locale loc)`.
    ///
    /// # Errors
    ///
    /// As [`new_ulocale`](Self::new_ulocale); `NullPointerException` for a
    /// `null` locale.
    pub fn new_locale(loc: Option<&Locale>) -> Result<Self> {
        let uloc = ULocale::for_locale(loc)?.ok_or_else(LuceneError::null_pointer)?;
        Self::new_ulocale(&uloc)
    }

    /// `Builder(ULocale loc)`: the `exceptions/SentenceBreak` strings of
    /// the locale's break-iterator bundle.
    ///
    /// # Errors
    ///
    /// The exceptions of the bundle lookup.
    pub fn new_ulocale(loc: &ULocale) -> Result<Self> {
        let mut this = Self::default();
        let rb = ICUResourceBundle::get_bundle_instance_ulocale(
            Some(ICUData::ICU_BRKITR_BASE_NAME),
            Some(loc),
            OpenType::LocaleRoot,
        )?;

        let breaks = rb.find_with_fallback("exceptions/SentenceBreak")?;

        if let Some(breaks) = breaks {
            let size = breaks.get_size();
            let breaks = UResourceBundle::Icu(breaks);
            for index in 0..size {
                let b = breaks.get_at(index)?;
                let br = b.get_string()?;
                this.filter_set.insert(FilterString(br));
            }
        }
        Ok(this)
    }

    /// `Builder()`: no suppressions.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FilteredBreakIteratorBuilder for Builder {
    fn suppress_break_after(&mut self, str: &[u16]) -> bool {
        self.filter_set.insert(FilterString(str.to_vec()))
    }

    fn unsuppress_break_after(&mut self, str: &[u16]) -> bool {
        self.filter_set.remove(&FilterString(str.to_vec()))
    }

    fn wrap_iterator_with_filter(
        &self,
        adopt_break_iterator: Box<dyn BreakIterator>,
    ) -> Result<Box<dyn BreakIterator>> {
        if self.filter_set.is_empty() {
            // Short circuit - nothing to except.
            return Ok(adopt_break_iterator);
        }

        let mut builder = CharsTrieBuilder::new();
        let mut builder2 = CharsTrieBuilder::new();

        let mut rev_count = 0;
        let mut fwd_count = 0;

        let sub_count = self.filter_set.len();
        let mut ustrs: Vec<&[u16]> = Vec::with_capacity(sub_count);
        let mut partials = vec![0i32; sub_count];

        let mut backwards_trie = None; // i.e. ".srM" for Mrs.
        let mut forwards_partial_trie = None; // Has ".a" for "a.M."

        for s in self.filter_set.iter() {
            ustrs.push(&s.0);
        }

        for i in 0..sub_count {
            let this_str = ustrs[i]; // TODO: don't cast to String?
            let nn = index_of_unit(this_str, b'.' as u16, 0); // TODO: non-'.' abbreviations
            if nn > -1 && (nn + 1) as usize != this_str.len() {
                let nn = nn as usize;
                // is partial.
                // is it unique?
                let mut same_as: i32 = -1;
                for j in 0..sub_count {
                    if j == i {
                        continue;
                    }
                    if region_matches(this_str, ustrs[j], nn + 1) {
                        if partials[j] == 0 {
                            // hasn't been processed yet
                            partials[j] = Self::SUPPRESS_IN_REVERSE | Self::ADD_TO_FORWARD;
                        } else if (partials[j] & Self::SUPPRESS_IN_REVERSE) != 0 {
                            same_as = j as i32; // the other entry is already in the reverse table.
                        }
                    }
                }

                if same_as == -1 && partials[i] == 0 {
                    // first one - add the prefix to the reverse table.
                    let prefix = java_reverse(&this_str[..nn + 1]);
                    builder.add(&prefix, Self::PARTIAL)?;
                    rev_count += 1;
                    partials[i] = Self::SUPPRESS_IN_REVERSE | Self::ADD_TO_FORWARD;
                }
            }
        }

        for i in 0..sub_count {
            let this_str = ustrs[i];
            if partials[i] == 0 {
                let reversed = java_reverse(this_str);
                builder.add(&reversed, Self::MATCH)?;
                rev_count += 1;
            } else {
                // an optimization would be to only add the portion after the '.'
                // for example, for "Ph.D." we store ".hP" in the reverse table. We could just store "D." in the
                // forward,
                // instead of "Ph.D." since we already know the "Ph." part is a match.
                // would need the trie to be able to hold 0-length strings, though.
                builder2.add(this_str, Self::MATCH)?; // forward
                fwd_count += 1;
            }
        }

        if rev_count > 0 {
            backwards_trie = Some(builder.build(StringTrieBuilderOption::Fast)?);
        }

        if fwd_count > 0 {
            forwards_partial_trie = Some(builder2.build(StringTrieBuilderOption::Fast)?);
        }
        Ok(Box::new(SimpleFilteredSentenceBreakIterator::new(
            adopt_break_iterator,
            forwards_partial_trie,
            backwards_trie,
        )))
    }
}
