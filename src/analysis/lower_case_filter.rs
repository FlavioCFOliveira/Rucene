//! Lowercase token filter ported from `org.apache.lucene.analysis.LowerCaseFilter`.
//!
//! The filter lowercases the term text of each token in place with
//! [`to_lower_case`](crate::analysis::character_utils::to_lower_case), Java's
//! `CharacterUtils.toLowerCase`: the simple, one-for-one mapping of
//! `Character.toLowerCase(int)`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::analysis::character_utils::to_lower_case;
use crate::analysis::tokenattributes::CharTermAttribute;
use crate::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
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
        // `CharacterUtils.toLowerCase(termAtt.buffer(), 0, termAtt.length())`
        // (`LowerCaseFilter.java:35-43`).
        if !input.increment_token()? {
            return Ok(false);
        }
        let mut att = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
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
    let mut filter = TokenFilter::new(input, LowerCaseFilterLogic::new());
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
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
