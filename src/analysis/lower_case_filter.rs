//! Lowercase token filter ported from `org.apache.lucene.analysis.LowerCaseFilter`.
//!
//! The filter lowercases the term text of each token in place with
//! [`to_lower_case`], Java's
//! `CharacterUtils.toLowerCase`: the simple, one-for-one mapping of
//! `Character.toLowerCase(int)`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::character_utils::to_lower_case;
use crate::analysis::tokenattributes::{CharTermAttribute, PackedTokenAttributeImpl};
use crate::analysis::{AttributeHandle, TokenFilter, TokenFilterLogic, TokenStream};
use crate::error::Result;
use crate::util::attribute::AttributeSource;

/// Lowercases each token's term text.
///
/// Equivalent to `org.apache.lucene.analysis.LowerCaseFilter`.
#[derive(Debug)]
pub struct LowerCaseFilterLogic {
    /// `private final CharTermAttribute termAtt = addAttribute(CharTermAttribute.class)`.
    term_att: AttributeHandle<dyn CharTermAttribute>,
}

impl LowerCaseFilterLogic {
    /// Creates the logic of a filter whose source is `source`, adding the
    /// `CharTermAttribute` its field holds (`LowerCaseFilter.java:24`).
    ///
    /// # Errors
    ///
    /// What the source's attribute factory raises for an interface it cannot
    /// implement, as Java's `addAttribute` throws.
    pub fn new(source: &mut AttributeSource) -> Result<Self> {
        Ok(Self {
            term_att: AttributeHandle::add(source)?,
        })
    }
}

impl TokenFilterLogic for LowerCaseFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        // `CharacterUtils.toLowerCase(termAtt.buffer(), 0, termAtt.length())`
        // (`LowerCaseFilter.java:35-43`).
        if !input.increment_token()? {
            return Ok(false);
        }
        // PERF: the default factory's `PackedTokenAttributeImpl` is reached
        // through its concrete type, statically and inlined, as the JIT
        // inlines Java's monomorphic `termAtt` calls (rmp #679
        // devirtualisation); any other implementation takes the `dyn` path.
        let mut att = self.term_att.borrow_impl_mut();
        if let Some(packed) = att.as_any_mut().downcast_mut::<PackedTokenAttributeImpl>() {
            let length = packed.length();
            to_lower_case(packed.buffer_mut(), 0, length);
            return Ok(true);
        }
        drop(att);
        let mut att = self.term_att.borrow_mut();
        let length = att.length();
        to_lower_case(att.buffer_mut(), 0, length);
        Ok(true)
    }
}

/// A [`TokenFilter`] that lowercases token terms.
pub type LowerCaseFilter = TokenFilter<LowerCaseFilterLogic>;

/// Creates a [`LowerCaseFilter`] over `input`.
///
/// Equivalent to `LowerCaseFilter(TokenStream)`, which adds the
/// `CharTermAttribute` (`LowerCaseFilter.java:24`).
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_lower_case_filter(input: Box<dyn TokenStream>) -> Result<LowerCaseFilter> {
    TokenFilter::with_logic(input, LowerCaseFilterLogic::new)
}

/// `LowerCaseFilter` as an owned stage (see [`owned`](crate::analysis::owned)):
/// `if (input.incrementToken()) { CharacterUtils.toLowerCase(termAtt.buffer(),
/// 0, termAtt.length()); return true; }` (`LowerCaseFilter.java:36-43`).
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct OwnedLowerCaseFilter;

impl OwnedLowerCaseFilter {
    /// The filter's work on a token its input produced:
    /// `CharacterUtils.toLowerCase(termAtt.buffer(), 0, termAtt.length())`.
    #[inline]
    pub fn lower_case(attrs: &mut crate::analysis::owned::TokenAttrs<'_>) {
        let length = attrs.term.length();
        to_lower_case(attrs.term.buffer_mut(), 0, length);
    }
}

impl crate::analysis::owned::OwnedTokenFilter for OwnedLowerCaseFilter {
    #[inline]
    fn increment_token(
        &mut self,
        attrs: &mut crate::analysis::owned::TokenAttrs<'_>,
        input: &mut dyn crate::analysis::owned::OwnedInput,
    ) -> Result<bool> {
        if !input.increment_token(attrs)? {
            return Ok(false);
        }
        Self::lower_case(attrs);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::StringTokenStream;

    /// `MiscOracle` O3 (`oracle/h2/MiscOracle.java`): the filter lower-cases
    /// with `CharacterUtils.toLowerCase`, so `\u{130}` becomes `i`, a final
    /// sigma stays `\u{3C3}` and U+1E9E becomes U+00DF, as Java printed.
    #[test]
    fn the_filter_lower_cases_with_the_simple_mapping() {
        let input = StringTokenStream::new(
            "\u{130}STANBUL \u{3A3}\u{391}\u{3A3} \u{1E9E} \u{10400} \u{1C5}".to_string(),
        )
        .unwrap();
        let mut filter = new_lower_case_filter(Box::new(input)).unwrap();
        filter.reset().unwrap();
        assert!(filter.increment_token().unwrap());
        let term_att = filter
            .attribute_source()
            .get_attribute_interface::<dyn CharTermAttribute>()
            .unwrap();
        let term: String = term_att.buffer()[..term_att.length()].iter().collect();
        assert_eq!(
            term,
            "istanbul \u{3C3}\u{3B1}\u{3C3} \u{DF} \u{10428} \u{1C6}"
        );
    }
}
