// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.ScientificMatcher`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::decimal_quantity_dual_storage_bcd::DecimalQuantity_DualStorageBCD;
use crate::r#impl::number::grouper::Grouper;
use crate::r#impl::number::parse::decimal_matcher::DecimalMatcher;
use crate::r#impl::number::parse::ignorables_matcher::IgnorablesMatcher;
use crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher;
use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::static_unicode_sets::{Key, StaticUnicodeSets};
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::unicode_set::UnicodeSet;

/// Matches the exponent part of a number in scientific notation.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.ScientificMatcher`.
#[derive(Debug)]
pub struct ScientificMatcher {
    exponent_separator_string: Vec<u16>,
    exponent_matcher: Arc<DecimalMatcher>,
    ignorables_matcher: Arc<IgnorablesMatcher>,
    custom_minus_sign: Option<Vec<u16>>,
    custom_plus_sign: Option<Vec<u16>>,
}

impl ScientificMatcher {
    /// Equivalent to `getInstance(DecimalFormatSymbols, Grouper)`.
    ///
    /// # Errors
    ///
    /// The errors of `DecimalMatcher.getInstance`.
    pub fn get_instance(
        symbols: &DecimalFormatSymbols,
        grouper: &Grouper,
    ) -> Result<Arc<ScientificMatcher>> {
        // TODO: Static-initialize most common instances?
        Ok(Arc::new(Self::new(symbols, grouper)?))
    }

    fn new(symbols: &DecimalFormatSymbols, grouper: &Grouper) -> Result<Self> {
        let exponent_separator_string = symbols.get_exponent_separator().to_vec();
        let exponent_matcher = DecimalMatcher::get_instance(
            symbols,
            grouper,
            ParsingUtils::PARSE_FLAG_INTEGER_ONLY | ParsingUtils::PARSE_FLAG_GROUPING_DISABLED,
        )?;
        let ignorables_matcher =
            IgnorablesMatcher::get_instance(ParsingUtils::PARSE_FLAG_STRICT_IGNORABLES);

        let minus_sign = symbols.get_minus_sign_string();
        let custom_minus_sign = if Self::minus_sign_set().contains_str(minus_sign) {
            None
        } else {
            Some(minus_sign.to_vec())
        };
        let plus_sign = symbols.get_plus_sign_string();
        let custom_plus_sign = if Self::plus_sign_set().contains_str(plus_sign) {
            None
        } else {
            Some(plus_sign.to_vec())
        };
        Ok(ScientificMatcher {
            exponent_separator_string,
            exponent_matcher,
            ignorables_matcher,
            custom_minus_sign,
            custom_plus_sign,
        })
    }

    fn minus_sign_set() -> &'static UnicodeSet {
        StaticUnicodeSets::get(Key::MinusSign)
    }

    fn plus_sign_set() -> &'static UnicodeSet {
        StaticUnicodeSets::get(Key::PlusSign)
    }
}

impl NumberParseMatcher for ScientificMatcher {
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        // Only accept scientific notation after the mantissa.
        if !result.seen_number() {
            return Ok(false);
        }

        // Only accept one exponent per string.
        if 0 != (result.flags & parsed_number::FLAG_HAS_EXPONENT) {
            return Ok(false);
        }

        // First match the scientific separator, and then match another number after it.
        // NOTE: This is guarded by the smoke test; no need to check exponentSeparatorString length again.
        let initial_offset = segment.get_offset();
        let mut overlap = segment.get_common_prefix_length(&self.exponent_separator_string);
        if overlap == self.exponent_separator_string.len() as i32 {
            // Full exponent separator match.

            // First attempt to get a code point, returning true if we can't get one.
            if segment.length() == overlap {
                return Ok(true);
            }
            segment.adjust_offset(overlap);

            // Allow ignorables before the sign.
            // Note: call site is guarded by the segment.length() check above.
            // Note: the ignorables matcher neither reads nor writes the result
            // (Java passes null).
            self.ignorables_matcher
                .match_(segment, &mut ParsedNumber::new())?;
            if segment.length() == 0 {
                segment.set_offset(initial_offset);
                return Ok(true);
            }

            // Allow a sign, and then try to match digits.
            let mut exponent_sign = 1;
            if segment.starts_with_set(Self::minus_sign_set()) {
                exponent_sign = -1;
                segment.adjust_offset_by_code_point();
            } else if segment.starts_with_set(Self::plus_sign_set()) {
                segment.adjust_offset_by_code_point();
            } else if segment.starts_with(self.custom_minus_sign.as_deref()) {
                // Note: call site is guarded with startsWith, which returns false on empty strings.
                let custom_minus_sign = self.custom_minus_sign.as_deref().unwrap_or_default();
                overlap = segment.get_common_prefix_length(custom_minus_sign);
                if overlap != custom_minus_sign.len() as i32 {
                    // Partial custom sign match; un-match the exponent separator.
                    segment.set_offset(initial_offset);
                    return Ok(true);
                }
                exponent_sign = -1;
                segment.adjust_offset(overlap);
            } else if segment.starts_with(self.custom_plus_sign.as_deref()) {
                // Note: call site is guarded with startsWith, which returns false on empty strings.
                let custom_plus_sign = self.custom_plus_sign.as_deref().unwrap_or_default();
                overlap = segment.get_common_prefix_length(custom_plus_sign);
                if overlap != custom_plus_sign.len() as i32 {
                    // Partial custom sign match; un-match the exponent separator.
                    segment.set_offset(initial_offset);
                    return Ok(true);
                }
                segment.adjust_offset(overlap);
            }

            // We already know that the segment is not empty.
            if segment.length() == 0 {
                segment.set_offset(initial_offset);
                return Ok(true);
            }

            // Allow ignorables after the sign.
            // Note: call site is guarded by the segment.length() check above.
            self.ignorables_matcher
                .match_(segment, &mut ParsedNumber::new())?;
            if segment.length() == 0 {
                segment.set_offset(initial_offset);
                return Ok(true);
            }

            // We are supposed to accept E0 after NaN, so we need to make sure result.quantity is available.
            let was_null = result.quantity.is_none();
            if was_null {
                result.quantity = Some(DecimalQuantity_DualStorageBCD::new());
            }
            let digits_offset = segment.get_offset();
            let digits_return_value =
                self.exponent_matcher
                    .match_exponent(segment, result, exponent_sign);
            if was_null {
                result.quantity = None;
            }
            let digits_return_value = digits_return_value?;

            if segment.get_offset() != digits_offset {
                // At least one exponent digit was matched.
                result.flags |= parsed_number::FLAG_HAS_EXPONENT;
            } else {
                // No exponent digits were matched; un-match the exponent separator.
                segment.set_offset(initial_offset);
            }
            return Ok(digits_return_value);
        } else if overlap == segment.length() {
            // Partial exponent separator match
            return Ok(true);
        }

        // No match
        Ok(false)
    }

    fn smoke_test(&self, segment: &StringSegment) -> bool {
        segment.starts_with(Some(&self.exponent_separator_string))
    }

    fn post_process(&self, _result: &mut ParsedNumber) -> Result<()> {
        // No-op
        Ok(())
    }

    fn to_java_string(&self) -> String {
        format!(
            "<ScientificMatcher {}>",
            String::from_utf16_lossy(&self.exponent_separator_string)
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
