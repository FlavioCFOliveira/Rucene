// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.AffixPatternMatcher`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::r#impl::number::affix_utils::{
    AffixUtils, TokenConsumer, TYPE_CURRENCY_DOUBLE, TYPE_CURRENCY_QUAD, TYPE_CURRENCY_QUINT, TYPE_CURRENCY_SINGLE,
    TYPE_CURRENCY_TRIPLE, TYPE_MINUS_SIGN, TYPE_PERCENT, TYPE_PERMILLE, TYPE_PLUS_SIGN,
};
use crate::r#impl::number::decimal_format_properties::string_hash_code;
use crate::r#impl::number::parse::affix_token_matcher_factory::AffixTokenMatcherFactory;
use crate::r#impl::number::parse::code_point_matcher::CodePointMatcher;
use crate::r#impl::number::parse::ignorables_matcher::IgnorablesMatcher;
use crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher;
use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::number::parse::series_matcher::SeriesMatcher;
use crate::r#impl::string_segment::StringSegment;

/// A specialized version of `SeriesMatcher` that matches EITHER a prefix OR
/// a suffix. `AffixMatcher` combines two of these in order to match both the
/// prefix and suffix.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.AffixPatternMatcher` (a
/// `SeriesMatcher` subclass): it holds its [`SeriesMatcher`] state.
#[derive(Debug)]
pub struct AffixPatternMatcher {
    series: SeriesMatcher,
    affix_pattern: Vec<u16>,
}

/// The transient construction state of `fromAffixPattern` (`factory`,
/// `ignorables`, `lastTypeOrCp`), consuming the tokens of the pattern.
struct Builder<'a> {
    series: SeriesMatcher,
    factory: &'a AffixTokenMatcherFactory,
    ignorables: Option<Arc<IgnorablesMatcher>>,
    last_type_or_cp: i32,
    /// The first exception a token threw; Java's propagates out of
    /// `iterateWithConsumer`, so no later token is consumed.
    error: Option<LuceneError>,
}

impl AffixPatternMatcher {
    /// Creates an AffixPatternMatcher (based on SeriesMatcher) from the
    /// given affix pattern. Returns `None` if the affix pattern is empty.
    /// Equivalent to `fromAffixPattern(String, AffixTokenMatcherFactory,
    /// int)`.
    ///
    /// # Errors
    ///
    /// The errors of the token iteration and of the currency matcher.
    pub fn from_affix_pattern(
        affix_pattern: &[u16],
        factory: &AffixTokenMatcherFactory,
        parse_flags: i32,
    ) -> Result<Option<Arc<AffixPatternMatcher>>> {
        if affix_pattern.is_empty() {
            return Ok(None);
        }

        let mut series = Builder {
            series: SeriesMatcher::default(),
            factory,
            ignorables: if 0 != (parse_flags & ParsingUtils::PARSE_FLAG_EXACT_AFFIX) {
                None
            } else {
                Some(factory.ignorables())
            },
            last_type_or_cp: 0,
            error: None,
        };
        AffixUtils::iterate_with_consumer(affix_pattern, &mut series)?;
        if let Some(e) = series.error {
            return Err(e);
        }

        // De-reference the memory
        let mut matcher = AffixPatternMatcher { series: series.series, affix_pattern: affix_pattern.to_vec() };
        matcher.series.freeze();
        Ok(Some(Arc::new(matcher)))
    }

    /// Equivalent to `getPattern()`.
    pub fn get_pattern(&self) -> &[u16] {
        &self.affix_pattern
    }

    /// Equivalent to `length()` (inherited).
    pub fn length(&self) -> i32 {
        self.series.length()
    }

    /// Equivalent to `equals(Object)`: same affix pattern.
    pub fn java_equals(&self, other: &AffixPatternMatcher) -> bool {
        std::ptr::eq(self, other) || self.affix_pattern == other.affix_pattern
    }

    /// Equivalent to `hashCode()`: the affix pattern's hash code.
    pub fn hash_code(&self) -> i32 {
        string_hash_code(&self.affix_pattern)
    }
}

impl TokenConsumer for Builder<'_> {
    fn consume_token(&mut self, type_or_cp: i32) {
        if self.error.is_some() {
            return;
        }
        // This is called by AffixUtils.iterateWithConsumer() for each token.

        // Add an ignorables matcher between tokens except between two literals, and don't put two
        // ignorables matchers in a row.
        if let Some(ignorables) = self.ignorables.as_ref() {
            if self.series.length() > 0
                && (self.last_type_or_cp < 0 || !ignorables.get_set().contains_valid(self.last_type_or_cp))
            {
                self.series.add_matcher(ignorables.clone());
            }
        }

        if type_or_cp < 0 {
            // Case 1: the token is a symbol.
            match type_or_cp {
                TYPE_MINUS_SIGN => self.series.add_matcher(self.factory.minus_sign()),
                TYPE_PLUS_SIGN => self.series.add_matcher(self.factory.plus_sign()),
                TYPE_PERCENT => self.series.add_matcher(self.factory.percent()),
                TYPE_PERMILLE => self.series.add_matcher(self.factory.permille()),
                TYPE_CURRENCY_SINGLE | TYPE_CURRENCY_DOUBLE | TYPE_CURRENCY_TRIPLE | TYPE_CURRENCY_QUAD
                | TYPE_CURRENCY_QUINT => match self.factory.currency() {
                    // All currency symbols use the same matcher
                    Ok(m) => self.series.add_matcher(m),
                    Err(e) => {
                        self.error = Some(e);
                        return;
                    }
                },
                _ => panic!("java.lang.AssertionError"),
            }
        } else if self.ignorables.as_ref().is_some_and(|i| i.get_set().contains_valid(type_or_cp)) {
            // Case 2: the token is an ignorable literal.
            // No action necessary: the ignorables matcher has already been added.
        } else {
            // Case 3: the token is a non-ignorable literal.
            self.series.add_matcher(CodePointMatcher::get_instance(type_or_cp));
        }
        self.last_type_or_cp = type_or_cp;
    }
}

impl NumberParseMatcher for AffixPatternMatcher {
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        self.series.series_match(segment, result)
    }

    fn smoke_test(&self, segment: &StringSegment) -> bool {
        self.series.series_smoke_test(segment)
    }

    fn post_process(&self, result: &mut ParsedNumber) -> Result<()> {
        self.series.series_post_process(result)
    }

    /// Equivalent to `toString()`: the affix pattern.
    fn to_java_string(&self) -> String {
        String::from_utf16_lossy(&self.affix_pattern)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
