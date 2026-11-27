//! Stop-word token filter ported from `org.apache.lucene.analysis.StopFilter`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::character_utils::term_to_utf16;
use crate::analysis::filtering_token_filter::{
    new_filtering_token_filter_with, FilteringTokenFilter, FilteringTokenFilterLogic,
};
use crate::analysis::tokenattributes::CharTermAttribute;
use crate::analysis::{AttributeHandle, CharArraySet, TokenStream};
use crate::error::Result;
use crate::util::attribute::AttributeSource;

/// Accept predicate that rejects terms present in a stop-word set.
#[derive(Debug)]
pub struct StopFilterLogic {
    stop_words: CharArraySet,
    /// The UTF-16 units of the current term.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's term buffer is a `char[]`
    /// that `stopWords.contains` reads directly; this port's
    /// `CharTermAttribute` holds `char`s while `CharArraySet` keys are
    /// UTF-16 units (as Java's are), so each term is encoded here first.
    scratch: Vec<u16>,
    /// `private final CharTermAttribute termAtt = addAttribute(CharTermAttribute.class)`
    /// (`StopFilter.java:28`).
    term_att: AttributeHandle<dyn CharTermAttribute>,
}

impl StopFilterLogic {
    /// Creates a stop-word filter logic with the given set, over the filter's
    /// source `source`, to which it adds the `CharTermAttribute` its field
    /// holds (`StopFilter.java:28`).
    ///
    /// # Errors
    ///
    /// What the source's attribute factory raises for an interface it cannot
    /// implement, as Java's `addAttribute` throws.
    pub fn new(stop_words: CharArraySet, source: &mut AttributeSource) -> Result<Self> {
        Ok(Self {
            stop_words,
            scratch: Vec::new(),
            term_att: AttributeHandle::add(source)?,
        })
    }
}

impl FilteringTokenFilterLogic for StopFilterLogic {
    fn accept(&mut self, _source: &AttributeSource) -> Result<bool> {
        // `!stopWords.contains(termAtt.buffer(), 0, termAtt.length())`
        // (`StopFilter.java:96-98`).
        // PERF: an empty set contains no term, whatever its UTF-16 units: the
        // conversion below (a port-only step, see `scratch`) is skipped
        // (rmp #679; `StandardAnalyzer()`'s default set is empty).
        if self.stop_words.is_empty() {
            return Ok(true);
        }
        let term_att = self.term_att.borrow();
        let length = term_att.length();
        // PERF: a term of BMP `char`s is looked up as it is held.
        if let Some(contained) = self
            .stop_words
            .contains_bmp_chars(&term_att.buffer()[..length])
        {
            return Ok(!contained);
        }
        term_to_utf16(&term_att.buffer()[..length], &mut self.scratch);
        Ok(!self
            .stop_words
            .contains_chars(Some(&self.scratch), 0, self.scratch.len()))
    }
}

/// A [`TokenFilter`](crate::analysis::TokenFilter) that removes tokens contained in a stop-word set.
pub type StopFilter = FilteringTokenFilter<StopFilterLogic>;

/// Creates a [`StopFilter`] over `input` using the supplied stop-word set.
///
/// Equivalent to `StopFilter(TokenStream, CharArraySet)`, whose `super(in)`
/// is [`new_filtering_token_filter`](crate::analysis::new_filtering_token_filter), then adds the `CharTermAttribute`
/// (`StopFilter.java:28`).
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_stop_filter(
    input: Box<dyn TokenStream>,
    stop_words: CharArraySet,
) -> Result<StopFilter> {
    new_filtering_token_filter_with(input, |source| StopFilterLogic::new(stop_words, source))
}

/// Builds a stop-word set from an iterable of string-like items.
///
/// Equivalent to the `StopFilter.makeStopSet` overloads, whose general form,
/// `makeStopSet(List, boolean)`, sizes the set to the list and adds every item.
pub fn make_stop_set<I, S>(words: I, ignore_case: bool) -> CharArraySet
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    CharArraySet::from_iter(words, ignore_case)
}

/// `StopFilter` as an owned stage (see [`owned`](crate::analysis::owned)):
/// `FilteringTokenFilter.incrementToken()` (`FilteringTokenFilter.java:43-61`)
/// with `StopFilter.accept()` (`StopFilter.java:96-98`), answered as
/// [`StopFilterLogic`] answers it, and `FilteringTokenFilter.end()`/`reset()`.
#[doc(hidden)]
#[derive(Debug)]
pub struct OwnedStopFilter {
    stop_words: CharArraySet,
    /// The term's UTF-16 units for the set lookup (see [`StopFilterLogic`]).
    scratch: Vec<u16>,
    /// `FilteringTokenFilter.skippedPositions`.
    skipped_positions: i32,
}

impl OwnedStopFilter {
    /// The stage over `stop_words`.
    pub fn new(stop_words: CharArraySet) -> Self {
        Self {
            stop_words,
            scratch: Vec::new(),
            skipped_positions: 0,
        }
    }

    #[inline]
    fn accept(&mut self, attrs: &crate::analysis::owned::TokenAttrs<'_>) -> bool {
        if self.stop_words.is_empty() {
            return true;
        }
        let length = attrs.term.length();
        if let Some(contained) = self
            .stop_words
            .contains_bmp_chars(&attrs.term.buffer()[..length])
        {
            return !contained;
        }
        term_to_utf16(&attrs.term.buffer()[..length], &mut self.scratch);
        !self
            .stop_words
            .contains_chars(Some(&self.scratch), 0, self.scratch.len())
    }
}

impl OwnedStopFilter {
    /// `FilteringTokenFilter.incrementToken()` over `input`, the stages
    /// below, for a caller that holds them statically (a fused stage).
    ///
    /// # Errors
    ///
    /// What `input` raises.
    #[inline]
    pub fn increment_with(
        &mut self,
        attrs: &mut crate::analysis::owned::TokenAttrs<'_>,
        mut input: impl FnMut(&mut crate::analysis::owned::TokenAttrs<'_>) -> Result<bool>,
    ) -> Result<bool> {
        use crate::analysis::tokenattributes::PositionIncrementAttribute;
        self.skipped_positions = 0;
        while input(attrs)? {
            if self.accept(attrs) {
                if self.skipped_positions != 0 {
                    let pos_incr = attrs.term.get_position_increment();
                    attrs
                        .term
                        .set_position_increment(pos_incr + self.skipped_positions);
                }
                return Ok(true);
            }
            self.skipped_positions += attrs.term.get_position_increment();
        }
        Ok(false)
    }

    /// `FilteringTokenFilter.end()` after `input.end()`: the skipped
    /// positions added to the end position increment.
    pub fn end_after_input(&mut self, attrs: &mut crate::analysis::owned::TokenAttrs<'_>) {
        use crate::analysis::tokenattributes::PositionIncrementAttribute;
        let pos_incr = attrs.term.get_position_increment();
        attrs
            .term
            .set_position_increment(pos_incr + self.skipped_positions);
    }

    /// `FilteringTokenFilter.reset()` after `input.reset()`.
    pub fn reset_state(&mut self) {
        self.skipped_positions = 0;
    }
}

impl crate::analysis::owned::OwnedTokenFilter for OwnedStopFilter {
    #[inline]
    fn increment_token(
        &mut self,
        attrs: &mut crate::analysis::owned::TokenAttrs<'_>,
        input: &mut dyn crate::analysis::owned::OwnedInput,
    ) -> Result<bool> {
        self.increment_with(attrs, |attrs| input.increment_token(attrs))
    }

    fn end(
        &mut self,
        attrs: &mut crate::analysis::owned::TokenAttrs<'_>,
        input: &mut dyn crate::analysis::owned::OwnedInput,
    ) -> Result<()> {
        input.end(attrs)?;
        self.end_after_input(attrs);
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.reset_state();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_stop_set_basic() {
        let set = make_stop_set(["a", "an", "the"], false);
        assert!(set.contains(Some("a")));
        assert!(set.contains(Some("an")));
        assert!(set.contains(Some("the")));
        assert!(!set.contains(Some("and")));
    }

    #[test]
    fn make_stop_set_ignore_case() {
        let set = make_stop_set(["Stop"], true);
        assert!(set.contains(Some("stop")));
        assert!(set.contains(Some("STOP")));
    }
}
