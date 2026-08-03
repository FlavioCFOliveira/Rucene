//! Lowercase token filter ported from `org.apache.lucene.analysis.LowerCaseFilter`.
//!
//! The filter lowercases the term text of each token. Rust's
//! `String::to_lowercase` is used, which applies the Unicode full case-folding
//! rules (language-independent). This matches Java's
//! `CharacterUtils.toLowerCase` for ASCII and for many common Unicode code
//! points, but differs for cases such as U+0130 LATIN CAPITAL LETTER I WITH
//! DOT ABOVE, where Java's simple `Character.toLowerCase` returns `i` while
//! Rust produces `i` followed by U+0307 COMBINING DOT ABOVE.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::tokenattributes::CharTermAttribute;
use crate::analysis::{PackedTokenAttributeImpl, TokenFilter, TokenFilterLogic, TokenStream};
use crate::error::Result;
use crate::util::attribute::AttributeSource;

/// Lowercases each token's term text.
///
/// Equivalent to `org.apache.lucene.analysis.LowerCaseFilter`.
#[derive(Debug, Default)]
pub struct LowerCaseFilterLogic;

impl LowerCaseFilterLogic {
    /// Creates a new lowercase filter logic instance.
    pub fn new() -> Self {
        Self
    }
}

impl TokenFilterLogic for LowerCaseFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        let term = source
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .unwrap()
            .term()
            .to_lowercase();
        let mut att = source
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .unwrap();
        att.set_empty();
        att.append_string(&term);
        Ok(true)
    }
}

/// A [`TokenFilter`] that lowercases token terms.
pub type LowerCaseFilter = TokenFilter<LowerCaseFilterLogic>;

/// Creates a [`LowerCaseFilter`] over `input`.
pub fn new_lower_case_filter(input: Box<dyn TokenStream>) -> LowerCaseFilter {
    TokenFilter::new(input, LowerCaseFilterLogic::new())
}
