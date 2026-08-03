//! Stop-word token filter ported from `org.apache.lucene.analysis.StopFilter`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::filtering_token_filter::{
    FilteringTokenFilter, FilteringTokenFilterAdapter, FilteringTokenFilterLogic,
};
use crate::analysis::tokenattributes::CharTermAttribute;
use crate::analysis::{CharArraySet, PackedTokenAttributeImpl, TokenFilter, TokenStream};
use crate::error::Result;
use crate::util::attribute::AttributeSource;

/// Accept predicate that rejects terms present in a stop-word set.
#[derive(Debug)]
pub struct StopFilterLogic {
    stop_words: CharArraySet,
}

impl StopFilterLogic {
    /// Creates a stop-word filter logic with the given set.
    pub fn new(stop_words: CharArraySet) -> Self {
        Self { stop_words }
    }
}

impl FilteringTokenFilterLogic for StopFilterLogic {
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        let term_att = source.get_attribute::<PackedTokenAttributeImpl>().unwrap();
        let buffer = term_att.buffer();
        let length = term_att.length();
        Ok(!self.stop_words.contains_chars(buffer, 0, length))
    }
}

/// A [`TokenFilter`] that removes tokens contained in a stop-word set.
pub type StopFilter = FilteringTokenFilter<StopFilterLogic>;

/// Creates a [`StopFilter`] over `input` using the supplied stop-word set.
pub fn new_stop_filter(input: Box<dyn TokenStream>, stop_words: CharArraySet) -> StopFilter {
    TokenFilter::new(
        input,
        FilteringTokenFilterAdapter::new(StopFilterLogic::new(stop_words)),
    )
}

/// Builds a stop-word set from an iterable of string-like items.
///
/// Equivalent to the `StopFilter.makeStopSet` overloads.
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
        assert!(set.contains("a"));
        assert!(set.contains("an"));
        assert!(set.contains("the"));
        assert!(!set.contains("and"));
    }

    #[test]
    fn make_stop_set_ignore_case() {
        let set = make_stop_set(["Stop"], true);
        assert!(set.contains("stop"));
        assert!(set.contains("STOP"));
    }
}
