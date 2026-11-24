// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.AffixMatcher`.

use std::any::Any;
use std::cmp::Ordering;
use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::affix_pattern_provider::{
    AffixPatternProvider, FLAG_NEG_PREFIX, FLAG_NEG_SUFFIX, FLAG_POS_PREFIX, FLAG_POS_SUFFIX,
};
use crate::r#impl::number::affix_utils::{AffixUtils, TYPE_MINUS_SIGN, TYPE_PLUS_SIGN};
use crate::r#impl::number::parse::affix_pattern_matcher::AffixPatternMatcher;
use crate::r#impl::number::parse::affix_token_matcher_factory::AffixTokenMatcherFactory;
use crate::r#impl::number::parse::ignorables_matcher::IgnorablesMatcher;
use crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher;
use crate::r#impl::number::parse::number_parser_impl::NumberParserImpl;
use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::number::pattern_string_utils::{PatternSignType, PatternStringUtils};
use crate::r#impl::standard_plural::StandardPlural;
use crate::r#impl::string_segment::StringSegment;

/// `Objects.equals` of two optional affix pattern matchers.
fn objects_equals(
    a: &Option<Arc<AffixPatternMatcher>>,
    b: &Option<Arc<AffixPatternMatcher>>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.java_equals(b),
        _ => false,
    }
}

/// `Objects.hashCode` of an optional affix pattern matcher.
fn objects_hash_code(a: &Option<Arc<AffixPatternMatcher>>) -> i32 {
    a.as_ref().map_or(0, |m| m.hash_code())
}

/// Matches a prefix and a suffix of a pattern, flagging the parse negative
/// when they belong to the negative subpattern.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.AffixMatcher`.
#[derive(Debug)]
pub struct AffixMatcher {
    prefix: Option<Arc<AffixPatternMatcher>>,
    suffix: Option<Arc<AffixPatternMatcher>>,
    flags: i32,
}

impl AffixMatcher {
    /// Comparator for two AffixMatcher instances which prioritizes longer
    /// prefixes followed by longer suffixes, ensuring that the longest prefix
    /// match is always tried first. Equivalent to `COMPARATOR.compare`.
    pub fn compare(lhs: &AffixMatcher, rhs: &AffixMatcher) -> Ordering {
        if Self::length(&lhs.prefix) != Self::length(&rhs.prefix) {
            if Self::length(&lhs.prefix) > Self::length(&rhs.prefix) {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else if Self::length(&lhs.suffix) != Self::length(&rhs.suffix) {
            if Self::length(&lhs.suffix) > Self::length(&rhs.suffix) {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else if !lhs.java_equals(rhs) {
            // If the prefix and suffix are the same length, arbitrarily break ties.
            // We can't return zero unless the elements are equal.
            if lhs.hash_code() > rhs.hash_code() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            Ordering::Equal
        }
    }

    /// `isInteresting(AffixPatternProvider, IgnorablesMatcher, int)`.
    fn is_interesting(
        pattern_info: &dyn AffixPatternProvider,
        ignorables: &IgnorablesMatcher,
        parse_flags: i32,
    ) -> Result<bool> {
        let pos_prefix_string = pattern_info.get_string(FLAG_POS_PREFIX);
        let pos_suffix_string = pattern_info.get_string(FLAG_POS_SUFFIX);
        let mut neg_prefix_string = None;
        let mut neg_suffix_string = None;
        if pattern_info.has_negative_subpattern() {
            neg_prefix_string = Some(pattern_info.get_string(FLAG_NEG_PREFIX));
            neg_suffix_string = Some(pattern_info.get_string(FLAG_NEG_SUFFIX));
        }

        let set = ignorables.get_set();
        if 0 == (parse_flags & ParsingUtils::PARSE_FLAG_USE_FULL_AFFIXES)
            && AffixUtils::contains_only_symbols_and_ignorables(Some(&pos_prefix_string), set)?
            && AffixUtils::contains_only_symbols_and_ignorables(Some(&pos_suffix_string), set)?
            && AffixUtils::contains_only_symbols_and_ignorables(neg_prefix_string.as_deref(), set)?
            && AffixUtils::contains_only_symbols_and_ignorables(neg_suffix_string.as_deref(), set)?
            // HACK: Plus and minus sign are a special case: we accept them trailing only if they are
            // trailing in the pattern string.
            && !AffixUtils::contains_type(Some(&pos_suffix_string), TYPE_PLUS_SIGN)?
            && !AffixUtils::contains_type(Some(&pos_suffix_string), TYPE_MINUS_SIGN)?
            && !AffixUtils::contains_type(neg_suffix_string.as_deref(), TYPE_PLUS_SIGN)?
            && !AffixUtils::contains_type(neg_suffix_string.as_deref(), TYPE_MINUS_SIGN)?
        {
            // The affixes contain only symbols and ignorables.
            // No need to generate affix matchers.
            return Ok(false);
        }
        Ok(true)
    }

    /// Equivalent to `createMatchers(AffixPatternProvider, NumberParserImpl,
    /// AffixTokenMatcherFactory, IgnorablesMatcher, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the affix pattern matchers.
    pub fn create_matchers(
        pattern_info: &dyn AffixPatternProvider,
        output: &mut NumberParserImpl,
        factory: &AffixTokenMatcherFactory,
        ignorables: &IgnorablesMatcher,
        parse_flags: i32,
    ) -> Result<()> {
        if !Self::is_interesting(pattern_info, ignorables, parse_flags)? {
            return Ok(());
        }

        // The affixes have interesting characters, or we are in strict mode.
        // Use initial capacity of 6, the highest possible number of AffixMatchers.
        let mut sb: Vec<u16> = Vec::new();
        let mut matchers: Vec<AffixMatcher> = Vec::with_capacity(6);
        let include_unpaired =
            0 != (parse_flags & ParsingUtils::PARSE_FLAG_INCLUDE_UNPAIRED_AFFIXES);

        let mut pos_prefix: Option<Arc<AffixPatternMatcher>> = None;
        let mut pos_suffix: Option<Arc<AffixPatternMatcher>> = None;

        // Pre-process the affix strings to resolve LDML rules like sign display.
        for type_ in PatternSignType::VALUES {
            // Skip affixes in some cases
            if type_ == PatternSignType::Pos
                && 0 != (parse_flags & ParsingUtils::PARSE_FLAG_PLUS_SIGN_ALLOWED)
            {
                continue;
            }
            if type_ == PatternSignType::PosSign
                && 0 == (parse_flags & ParsingUtils::PARSE_FLAG_PLUS_SIGN_ALLOWED)
            {
                continue;
            }

            // Generate Prefix
            // TODO: Handle approximately sign?
            PatternStringUtils::pattern_info_to_string_builder(
                pattern_info,
                true,
                type_,
                false,
                Some(StandardPlural::Other),
                false,
                &mut sb,
            )?;
            let prefix = AffixPatternMatcher::from_affix_pattern(&sb, factory, parse_flags)?;

            // Generate Suffix
            // TODO: Handle approximately sign?
            PatternStringUtils::pattern_info_to_string_builder(
                pattern_info,
                false,
                type_,
                false,
                Some(StandardPlural::Other),
                false,
                &mut sb,
            )?;
            let suffix = AffixPatternMatcher::from_affix_pattern(&sb, factory, parse_flags)?;

            if type_ == PatternSignType::Pos {
                pos_prefix = prefix.clone();
                pos_suffix = suffix.clone();
            } else if objects_equals(&prefix, &pos_prefix) && objects_equals(&suffix, &pos_suffix) {
                // Skip adding these matchers (we already have equivalents)
                continue;
            }

            // Flags for setting in the ParsedNumber; the token matchers may add more.
            let flags = if type_ == PatternSignType::Neg {
                parsed_number::FLAG_NEGATIVE
            } else {
                0
            };

            // Note: it is indeed possible for posPrefix and posSuffix to both be null.
            // We still need to add that matcher for strict mode to work.
            matchers.push(Self::get_instance(prefix.clone(), suffix.clone(), flags));
            if include_unpaired && prefix.is_some() && suffix.is_some() {
                // The following "if" statements are designed to prevent adding two identical matchers.
                if type_ == PatternSignType::Pos || !objects_equals(&prefix, &pos_prefix) {
                    matchers.push(Self::get_instance(prefix.clone(), None, flags));
                }
                if type_ == PatternSignType::Pos || !objects_equals(&suffix, &pos_suffix) {
                    matchers.push(Self::get_instance(None, suffix.clone(), flags));
                }
            }
        }

        // Put the AffixMatchers in order, and then add them to the output.
        matchers.sort_by(Self::compare);
        output.add_matchers(
            matchers
                .into_iter()
                .map(|m| Arc::new(m) as Arc<dyn NumberParseMatcher>),
        );
        Ok(())
    }

    /// `getInstance(AffixPatternMatcher, AffixPatternMatcher, int)`.
    fn get_instance(
        prefix: Option<Arc<AffixPatternMatcher>>,
        suffix: Option<Arc<AffixPatternMatcher>>,
        flags: i32,
    ) -> AffixMatcher {
        // TODO: Special handling for common cases like both strings empty.
        AffixMatcher {
            prefix,
            suffix,
            flags,
        }
    }

    /// Helper method to return whether the given AffixPatternMatcher equals
    /// the given pattern string. Either both arguments must be null or the
    /// pattern string inside the AffixPatternMatcher must equal the given
    /// pattern string. Equivalent to `matched(AffixPatternMatcher, String)`.
    pub(crate) fn matched(
        affix: Option<&AffixPatternMatcher>,
        pattern_string: Option<&[u16]>,
    ) -> bool {
        match (affix, pattern_string) {
            (None, None) => true,
            (Some(a), Some(p)) => a.get_pattern() == p,
            _ => false,
        }
    }

    /// Helper method to return the length of the given AffixPatternMatcher.
    /// Returns 0 for null.
    fn length(matcher: &Option<Arc<AffixPatternMatcher>>) -> i32 {
        matcher.as_ref().map_or(0, |m| m.get_pattern().len() as i32)
    }

    /// Equivalent to `equals(Object)`.
    pub fn java_equals(&self, other: &AffixMatcher) -> bool {
        objects_equals(&self.prefix, &other.prefix)
            && objects_equals(&self.suffix, &other.suffix)
            && self.flags == other.flags
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        objects_hash_code(&self.prefix) ^ objects_hash_code(&self.suffix) ^ self.flags
    }
}

impl NumberParseMatcher for AffixMatcher {
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        if !result.seen_number() {
            // Prefix
            // Do not match if:
            // 1. We have already seen a prefix (result.prefix != null)
            // 2. The prefix in this AffixMatcher is empty (prefix == null)
            let Some(prefix) = self.prefix.as_ref().filter(|_| result.prefix.is_none()) else {
                return Ok(false);
            };

            // Attempt to match the prefix.
            let initial_offset = segment.get_offset();
            let maybe_more = prefix.match_(segment, result)?;
            if initial_offset != segment.get_offset() {
                result.prefix = Some(prefix.get_pattern().to_vec());
            }
            Ok(maybe_more)
        } else {
            // Suffix
            // Do not match if:
            // 1. We have already seen a suffix (result.suffix != null)
            // 2. The suffix in this AffixMatcher is empty (suffix == null)
            // 3. The matched prefix does not equal this AffixMatcher's prefix
            if result.suffix.is_some()
                || !Self::matched(self.prefix.as_deref(), result.prefix.as_deref())
            {
                return Ok(false);
            }
            let Some(suffix) = self.suffix.as_ref() else {
                return Ok(false);
            };

            // Attempt to match the suffix.
            let initial_offset = segment.get_offset();
            let maybe_more = suffix.match_(segment, result)?;
            if initial_offset != segment.get_offset() {
                result.suffix = Some(suffix.get_pattern().to_vec());
            }
            Ok(maybe_more)
        }
    }

    fn smoke_test(&self, segment: &StringSegment) -> bool {
        self.prefix.as_ref().is_some_and(|p| p.smoke_test(segment))
            || self.suffix.as_ref().is_some_and(|s| s.smoke_test(segment))
    }

    fn post_process(&self, result: &mut ParsedNumber) -> Result<()> {
        // Check to see if our affix is the one that was matched. If so, set the flags in the result.
        if Self::matched(self.prefix.as_deref(), result.prefix.as_deref())
            && Self::matched(self.suffix.as_deref(), result.suffix.as_deref())
        {
            // Fill in the result prefix and suffix with non-null values (empty string).
            // Used by strict mode to determine whether an entire affix pair was matched.
            if result.prefix.is_none() {
                result.prefix = Some(Vec::new());
            }
            if result.suffix.is_none() {
                result.suffix = Some(Vec::new());
            }
            result.flags |= self.flags;
            if let Some(prefix) = self.prefix.as_ref() {
                prefix.post_process(result)?;
            }
            if let Some(suffix) = self.suffix.as_ref() {
                suffix.post_process(result)?;
            }
        }
        Ok(())
    }

    fn to_java_string(&self) -> String {
        let is_negative = 0 != (self.flags & parsed_number::FLAG_NEGATIVE);
        let p = self
            .prefix
            .as_ref()
            .map_or_else(|| "null".to_string(), |m| m.to_java_string());
        let s = self
            .suffix
            .as_ref()
            .map_or_else(|| "null".to_string(), |m| m.to_java_string());
        format!(
            "<AffixMatcher{}{}#{}>",
            if is_negative { ":negative " } else { " " },
            p,
            s
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
