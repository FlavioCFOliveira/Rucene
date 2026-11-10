//! Stop-word token filter ported from `org.apache.lucene.analysis.StopFilter`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::filtering_token_filter::{
    new_filtering_token_filter, FilteringTokenFilter, FilteringTokenFilterLogic,
};
use crate::analysis::tokenattributes::CharTermAttribute;
use crate::analysis::{CharArraySet, TokenStream};
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
}

impl StopFilterLogic {
    /// Creates a stop-word filter logic with the given set.
    pub fn new(stop_words: CharArraySet) -> Self {
        Self {
            stop_words,
            scratch: Vec::new(),
        }
    }
}

impl FilteringTokenFilterLogic for StopFilterLogic {
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        // `!stopWords.contains(termAtt.buffer(), 0, termAtt.length())`
        // (`StopFilter.java:96-98`).
        let term_att = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let length = term_att.length();
        self.scratch.clear();
        let mut units = [0u16; 2];
        for c in &term_att.buffer()[..length] {
            self.scratch.extend_from_slice(c.encode_utf16(&mut units));
        }
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
/// is [`new_filtering_token_filter`], then adds the `CharTermAttribute`
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
    let mut filter = new_filtering_token_filter(input, StopFilterLogic::new(stop_words))?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
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
