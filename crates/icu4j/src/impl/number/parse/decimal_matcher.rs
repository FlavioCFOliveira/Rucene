// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.DecimalMatcher`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::code_point_at;
use crate::lang::u_character::UCharacter;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_abstract_bcd::DecimalQuantityAbstractBcd;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::DecimalQuantity_DualStorageBCD;
use crate::r#impl::number::grouper::Grouper;
use crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher;
use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::static_unicode_sets::{Key, StaticUnicodeSets};
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::unicode_set::{UnicodeSet, EMPTY};

/// A `UnicodeSet` reference of the matcher: a shared static set, or a set
/// the constructor built and froze.
#[derive(Debug)]
enum SetRef {
    Static(&'static UnicodeSet),
    Owned(UnicodeSet),
}

impl SetRef {
    fn get(&self) -> &UnicodeSet {
        match self {
            SetRef::Static(s) => s,
            SetRef::Owned(s) => s,
        }
    }
}

/// Matches the digits, grouping separators and decimal separator of a
/// number.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.DecimalMatcher`.
#[derive(Debug)]
pub struct DecimalMatcher {
    /// If true, only accept strings whose grouping sizes match the locale
    require_grouping_match: bool,
    /// If true, do not accept grouping separators at all
    grouping_disabled: bool,
    // If true, do not accept fraction grouping separators
    // fractionGroupingDisabled: bool,
    /// If true, do not accept numbers in the fraction
    integer_only: bool,
    grouping1: i32,
    grouping2: i32,

    grouping_separator: Vec<u16>,
    decimal_separator: Vec<u16>,

    // Assumption: these sets all consist of single code points. If this assumption needs to be broken,
    // fix getLeadCodePoints() as well as matching logic. Be careful of the performance impact.
    grouping_uni_set: &'static UnicodeSet,
    decimal_uni_set: SetRef,
    separator_set: SetRef,
    lead_set: Option<&'static UnicodeSet>,
    digit_strings: Option<Vec<Vec<u16>>>,
}


impl DecimalMatcher {
    /// Equivalent to `getInstance(DecimalFormatSymbols, Grouper, int)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `UnicodeSet.add` (never for the
    /// first code point of a separator).
    pub fn get_instance(symbols: &DecimalFormatSymbols, grouper: &Grouper, parse_flags: i32) -> Result<Arc<DecimalMatcher>> {
        // TODO: Cache popular instances?
        Ok(Arc::new(Self::new(symbols, grouper, parse_flags)?))
    }

    fn new(symbols: &DecimalFormatSymbols, grouper: &Grouper, parse_flags: i32) -> Result<Self> {
        let (grouping_separator, decimal_separator) = if 0 != (parse_flags & ParsingUtils::PARSE_FLAG_MONETARY_SEPARATORS) {
            (
                symbols.get_monetary_grouping_separator_string().to_vec(),
                symbols.get_monetary_decimal_separator_string().to_vec(),
            )
        } else {
            (symbols.get_grouping_separator_string().to_vec(), symbols.get_decimal_separator_string().to_vec())
        };
        let strict_separators = 0 != (parse_flags & ParsingUtils::PARSE_FLAG_STRICT_SEPARATORS);
        let grouping_key = if strict_separators { Key::StrictAllSeparators } else { Key::AllSeparators };

        // Attempt to find separators in the static cache

        let grouping_uni_set = StaticUnicodeSets::get(grouping_key);
        let decimal_key = StaticUnicodeSets::choose_from2(
            &decimal_separator,
            if strict_separators { Key::StrictComma } else { Key::Comma },
            if strict_separators { Key::StrictPeriod } else { Key::Period },
        );
        let decimal_uni_set = if let Some(decimal_key) = decimal_key {
            SetRef::Static(StaticUnicodeSets::get(decimal_key))
        } else if !decimal_separator.is_empty() {
            let mut set = UnicodeSet::new();
            set.add(code_point_at(&decimal_separator, 0))?;
            set.freeze();
            SetRef::Owned(set)
        } else {
            SetRef::Static(&EMPTY)
        };

        let (separator_set, lead_set) = if decimal_key.is_some() {
            // Everything is available in the static cache
            (
                SetRef::Static(grouping_uni_set),
                Some(StaticUnicodeSets::get(if strict_separators {
                    Key::DigitsOrAllSeparators
                } else {
                    Key::DigitsOrStrictAllSeparators
                })),
            )
        } else {
            let mut set = UnicodeSet::new();
            set.add_all(grouping_uni_set)?;
            set.add_all(decimal_uni_set.get())?;
            set.freeze();
            (SetRef::Owned(set), None)
        };

        let cp_zero = symbols.get_code_point_zero();
        let digit_strings = if cp_zero == -1 || !UCharacter::is_digit(cp_zero) || UCharacter::digit(cp_zero) != 0 {
            // Uncommon case: okay to allocate.
            Some(symbols.get_digit_strings_local().to_vec())
        } else {
            None
        };

        Ok(DecimalMatcher {
            require_grouping_match: 0 != (parse_flags & ParsingUtils::PARSE_FLAG_STRICT_GROUPING_SIZE),
            grouping_disabled: 0 != (parse_flags & ParsingUtils::PARSE_FLAG_GROUPING_DISABLED),
            integer_only: 0 != (parse_flags & ParsingUtils::PARSE_FLAG_INTEGER_ONLY),
            grouping1: i32::from(grouper.get_primary()),
            grouping2: i32::from(grouper.get_secondary()),
            grouping_separator,
            decimal_separator,
            grouping_uni_set,
            decimal_uni_set,
            separator_set,
            lead_set,
            digit_strings,
        })

        // Fraction grouping parsing is disabled for now but could be enabled later.
        // See http://bugs.icu-project.org/trac/ticket/10794
        // fractionGrouping = 0 != (parseFlags & ParsingUtils.PARSE_FLAG_FRACTION_GROUPING_ENABLED);
    }

    /// Equivalent to `match(StringSegment, ParsedNumber, int)`.
    ///
    /// # Errors
    ///
    /// The errors of `UnicodeSet.contains` (never for code points of the
    /// input) and of `DecimalQuantity.adjustMagnitude` outside the exponent
    /// path (where an `ArithmeticException` is caught as an overflow).
    pub fn match_exponent(&self, segment: &mut StringSegment, result: &mut ParsedNumber, exponent_sign: i32) -> Result<bool> {
        if result.seen_number() && exponent_sign == 0 {
            // A number has already been consumed.
            return Ok(false);
        } else if exponent_sign != 0 {
            // scientific notation always comes after the number
            debug_assert!(result.quantity.is_some());
        }

        // Initial offset before any character consumption.
        let initial_offset = segment.get_offset();

        // Return value: whether to ask for more characters.
        let mut maybe_more = false;

        // All digits consumed so far.
        let mut digits_consumed: Option<DecimalQuantity_DualStorageBCD> = None;

        // The total number of digits after the decimal place, used for scaling the result.
        let mut digits_after_decimal_place = 0;

        // The actual grouping and decimal separators used in the string.
        // If non-null, we have seen that token.
        let mut actual_grouping_string: Option<Vec<u16>> = None;
        let mut actual_decimal_string: Option<Vec<u16>> = None;

        // Information for two groups: the previous group and the current group.
        //
        // Each group has three pieces of information:
        //
        // Offset: the string position of the beginning of the group, including a leading separator
        // if there was a leading separator. This is needed in case we need to rewind the parse to
        // that position.
        //
        // Separator type:
        // 0 => beginning of string
        // 1 => lead separator is a grouping separator
        // 2 => lead separator is a decimal separator
        //
        // Count: the number of digits in the group. If -1, the group has been validated.
        let mut curr_group_offset = 0;
        let mut curr_group_sep_type = 0;
        let mut curr_group_count = 0;
        let mut prev_group_offset = -1;
        let mut prev_group_sep_type = -1;
        let mut prev_group_count = -1;

        while segment.length() > 0 {
            maybe_more = false;

            // Attempt to match a digit.
            let mut digit: i8 = -1;

            // Try by code point digit value.
            let cp = segment.get_code_point();
            if UCharacter::is_digit(cp) {
                segment.adjust_offset(UCharacter::char_count(cp));
                digit = UCharacter::digit(cp) as i8;
            }

            // Try by digit string.
            if digit == -1 {
                if let Some(digit_strings) = self.digit_strings.as_ref() {
                    for (i, str) in digit_strings.iter().enumerate() {
                        if str.is_empty() {
                            continue;
                        }
                        let overlap = segment.get_common_prefix_length(str);
                        if overlap == str.len() as i32 {
                            segment.adjust_offset(overlap);
                            digit = i as i8;
                            break;
                        }
                        maybe_more = maybe_more || (overlap == segment.length());
                    }
                }
            }

            if digit >= 0 {
                // Digit was found.
                let dc = digits_consumed.get_or_insert_with(DecimalQuantity_DualStorageBCD::new);
                dc.append_digit(digit, 0, true);
                curr_group_count += 1;
                if actual_decimal_string.is_some() {
                    digits_after_decimal_place += 1;
                }
                continue;
            }

            // Attempt to match a literal grouping or decimal separator.
            let mut is_decimal = false;
            let mut is_grouping = false;

            // 1) Attempt the decimal separator string literal.
            // if (we have not seen a decimal separator yet) { ... }
            if actual_decimal_string.is_none() && !self.decimal_separator.is_empty() {
                let overlap = segment.get_common_prefix_length(&self.decimal_separator);
                maybe_more = maybe_more || (overlap == segment.length());
                if overlap == self.decimal_separator.len() as i32 {
                    is_decimal = true;
                    actual_decimal_string = Some(self.decimal_separator.clone());
                }
            }

            // 2) Attempt to match the actual grouping string literal.
            if let Some(actual) = actual_grouping_string.as_ref() {
                let overlap = segment.get_common_prefix_length(actual);
                maybe_more = maybe_more || (overlap == segment.length());
                if overlap == actual.len() as i32 {
                    is_grouping = true;
                }
            }

            // 2.5) Attempt to match a new the grouping separator string literal.
            // if (we have not seen a grouping or decimal separator yet) { ... }
            if !self.grouping_disabled
                && actual_grouping_string.is_none()
                && actual_decimal_string.is_none()
                && !self.grouping_separator.is_empty()
            {
                let overlap = segment.get_common_prefix_length(&self.grouping_separator);
                maybe_more = maybe_more || (overlap == segment.length());
                if overlap == self.grouping_separator.len() as i32 {
                    is_grouping = true;
                    actual_grouping_string = Some(self.grouping_separator.clone());
                }
            }

            // 3) Attempt to match a decimal separator from the equivalence set.
            // if (we have not seen a decimal separator yet) { ... }
            // The !isGrouping is to confirm that we haven't yet matched the current character.
            if !is_grouping && actual_decimal_string.is_none() && self.decimal_uni_set.get().contains(cp)? {
                is_decimal = true;
                actual_decimal_string = UCharacter::to_string_cp(cp);
            }

            // 4) Attempt to match a grouping separator from the equivalence set.
            // if (we have not seen a grouping or decimal separator yet) { ... }
            if !self.grouping_disabled
                && actual_grouping_string.is_none()
                && actual_decimal_string.is_none()
                && self.grouping_uni_set.contains(cp)?
            {
                is_grouping = true;
                actual_grouping_string = UCharacter::to_string_cp(cp);
            }

            // Leave if we failed to match this as a separator.
            if !is_decimal && !is_grouping {
                break;
            }

            // Check for conditions when we don't want to accept the separator.
            if is_decimal && self.integer_only {
                break;
            } else if curr_group_sep_type == 2 && is_grouping {
                // Fraction grouping
                break;
            }

            // Validate intermediate grouping sizes.
            let prev_valid_secondary = self.validate_group(prev_group_sep_type, prev_group_count, false);
            let curr_valid_primary = self.validate_group(curr_group_sep_type, curr_group_count, true);
            if !prev_valid_secondary || (is_decimal && !curr_valid_primary) {
                // Invalid grouping sizes.
                if is_grouping && curr_group_count == 0 {
                    // Trailing grouping separators: these are taken care of below
                    debug_assert!(curr_group_sep_type == 1);
                } else if self.require_grouping_match {
                    // Strict mode: reject the parse
                    digits_consumed = None;
                }
                break;
            } else if self.require_grouping_match && curr_group_count == 0 && curr_group_sep_type == 1 {
                break;
            } else {
                // Grouping sizes OK so far.
                prev_group_offset = curr_group_offset;
                prev_group_count = curr_group_count;
                if is_decimal {
                    // Do not validate this group any more.
                    prev_group_sep_type = -1;
                } else {
                    prev_group_sep_type = curr_group_sep_type;
                }
            }

            // OK to accept the separator.
            // Special case: don't update currGroup if it is empty; this allows two grouping
            // separators in a row in lenient mode.
            if curr_group_count != 0 {
                curr_group_offset = segment.get_offset();
            }
            curr_group_sep_type = if is_grouping { 1 } else { 2 };
            curr_group_count = 0;
            if is_grouping {
                let len = actual_grouping_string.as_ref().map_or(0, |s| s.len() as i32);
                segment.adjust_offset(len);
            } else {
                let len = actual_decimal_string.as_ref().map_or(0, |s| s.len() as i32);
                segment.adjust_offset(len);
            }
        }

        // End of main loop.
        // Back up if there was a trailing grouping separator.
        // Shift prev -> curr so we can check it as a final group.
        if curr_group_sep_type != 2 && curr_group_count == 0 {
            maybe_more = true;
            segment.set_offset(curr_group_offset);
            curr_group_offset = prev_group_offset;
            curr_group_sep_type = prev_group_sep_type;
            curr_group_count = prev_group_count;
            prev_group_offset = -1;
            prev_group_sep_type = 0;
            prev_group_count = 1;
        }

        // Validate final grouping sizes.
        let mut prev_valid_secondary = self.validate_group(prev_group_sep_type, prev_group_count, false);
        let mut curr_valid_primary = self.validate_group(curr_group_sep_type, curr_group_count, true);
        if !self.require_grouping_match {
            // The cases we need to handle here are lone digits.
            // Examples: "1,1" "1,1," "1,1,1" "1,1,1," ",1" (all parse as 1)
            // See more examples in numberformattestspecification.txt
            let mut digits_to_remove = 0;
            if !prev_valid_secondary {
                segment.set_offset(prev_group_offset);
                digits_to_remove += prev_group_count;
                digits_to_remove += curr_group_count;
            } else if !curr_valid_primary && (prev_group_sep_type != 0 || prev_group_count != 0) {
                maybe_more = true;
                segment.set_offset(curr_group_offset);
                digits_to_remove += curr_group_count;
            }
            if digits_to_remove != 0 {
                let dc = digits_consumed
                    .as_mut()
                    .unwrap_or_else(|| panic!("java.lang.NullPointerException: digitsConsumed"));
                dc.adjust_magnitude(-digits_to_remove)?;
                dc.truncate();
            }
            prev_valid_secondary = true;
            curr_valid_primary = true;
        }
        if curr_group_sep_type != 2 && (!prev_valid_secondary || !curr_valid_primary) {
            // Grouping failure.
            digits_consumed = None;
        }

        // Strings that start with a separator but have no digits,
        // or strings that failed a grouping size check.
        let Some(mut digits_consumed) = digits_consumed else {
            maybe_more = maybe_more || (segment.length() == 0);
            segment.set_offset(initial_offset);
            return Ok(maybe_more);
        };

        // We passed all inspections. Start post-processing.

        // Adjust for fraction part.
        digits_consumed.adjust_magnitude(-digits_after_decimal_place)?;

        // Set the digits, either normal or exponent.
        if exponent_sign != 0 && segment.get_offset() != initial_offset {
            let mut overflow = false;
            if digits_consumed.fits_in_long() {
                let exponent_long = digits_consumed.to_long(false);
                debug_assert!(exponent_long >= 0);
                if exponent_long <= i64::from(i32::MAX) {
                    let exponent_int = exponent_long as i32;
                    let quantity = result
                        .quantity
                        .as_mut()
                        .unwrap_or_else(|| panic!("java.lang.NullPointerException: quantity"));
                    // `adjustMagnitude` throws only ArithmeticException, which
                    // Java catches here.
                    if quantity.adjust_magnitude(exponent_sign * exponent_int).is_err() {
                        overflow = true;
                    }
                } else {
                    overflow = true;
                }
            } else {
                overflow = true;
            }
            if overflow {
                if exponent_sign == -1 {
                    // Set to zero
                    if let Some(q) = result.quantity.as_mut() {
                        q.clear();
                    }
                } else {
                    // Set to infinity
                    result.quantity = None;
                    result.flags |= parsed_number::FLAG_INFINITY;
                }
            }
        } else {
            result.quantity = Some(digits_consumed);
        }

        // Set other information into the result and return.
        if actual_decimal_string.is_some() {
            result.flags |= parsed_number::FLAG_HAS_DECIMAL_SEPARATOR;
        }
        result.set_chars_consumed(segment);
        Ok(segment.length() == 0 || maybe_more)
    }

    /// Return true if the given group is valid.
    fn validate_group(&self, sep_type: i32, count: i32, is_primary: bool) -> bool {
        if self.require_grouping_match {
            if sep_type == -1 {
                // This is a nonexistent or already validated group.
                true
            } else if sep_type == 0 {
                // First group.
                if is_primary {
                    // No grouping separators is OK.
                    true
                } else {
                    count != 0 && count <= self.grouping2
                }
            } else if sep_type == 1 {
                // Middle group.
                if is_primary {
                    count == self.grouping1
                } else {
                    count == self.grouping2
                }
            } else {
                debug_assert!(sep_type == 2);
                // After the decimal separator.
                true
            }
        } else if sep_type == 1 {
            // #11230: don't accept middle groups with only 1 digit.
            count != 1
        } else {
            true
        }
    }
}

impl NumberParseMatcher for DecimalMatcher {
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        self.match_exponent(segment, result, 0)
    }

    fn smoke_test(&self, segment: &StringSegment) -> bool {
        // The common case uses a static leadSet for efficiency.
        if self.digit_strings.is_none() {
            if let Some(lead_set) = self.lead_set {
                return segment.starts_with_set(lead_set);
            }
        }
        if segment.starts_with_set(self.separator_set.get()) || UCharacter::is_digit(segment.get_code_point()) {
            return true;
        }
        let Some(digit_strings) = self.digit_strings.as_ref() else {
            return false;
        };
        digit_strings.iter().any(|d| segment.starts_with(Some(d)))
    }

    fn post_process(&self, _result: &mut ParsedNumber) -> Result<()> {
        // No-op
        Ok(())
    }

    fn to_java_string(&self) -> String {
        "<DecimalMatcher>".to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
