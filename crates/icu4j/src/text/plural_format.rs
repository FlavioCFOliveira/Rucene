// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.PluralFormat`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::math::{BigDecimal, BigInteger};
use crate::jdk::text::{FieldPosition, Format, ParsedObject};
use crate::jdk::{index_of, units, ParsePosition};
use crate::math::big_decimal::BigDecimal as IcuBigDecimal;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::text::decimal_format::as_decimal_format;
use crate::text::formatted_value::FormattedValue;
use crate::text::message_pattern::{MessagePattern, PartType};
use crate::text::number_format::{self, lock, NumberFormat};
use crate::text::plural_rules::{FixedDecimal, IFixedDecimal, PluralRules, PluralType};
use crate::text::rbnf_lenient_scanner::RbnfLenientScanner;
use crate::text::rule_based_number_format::{npe, substring, RuleBasedNumberFormat};
use crate::text::u_format::UFormatBase;
use crate::util::u_locale::{self, Category, ULocale};

/// `serialVersionUID` (private).
pub const SERIAL_VERSION_UID: i64 = 1;

/// Interface for selecting PluralFormat keywords for numbers. The
/// PluralRules class was intended to implement this interface, but there is
/// no public API that uses a PluralSelector, only MessageFormat and
/// PluralFormat have PluralSelector implementations. Therefore, PluralRules
/// is not marked to implement this non-public interface, to avoid confusing
/// users.
///
/// Equivalent to the package-private interface
/// `PluralFormat.PluralSelector`; Java's `Object context` is the type
/// parameter.
pub(crate) trait PluralSelector<C: ?Sized> {
    /// Given a number, returns the appropriate PluralFormat keyword.
    /// `context` is the worker object for the selector, `number` the number
    /// to be plural-formatted. Equivalent to `select(Object context, double
    /// number)`.
    fn select(&self, context: &C, number: f64) -> String;
}

/// `PluralSelectorAdapter` (private): the selector over the format's plural
/// rules, whose context is the `IFixedDecimal` of the formatted number.
struct PluralSelectorAdapter<'a> {
    plural_rules: &'a PluralRules,
}

impl<'d> PluralSelector<dyn IFixedDecimal + 'd> for PluralSelectorAdapter<'_> {
    fn select(&self, context: &(dyn IFixedDecimal + 'd), _number: f64) -> String {
        self.plural_rules.select(context)
    }
}

/// `PluralFormat` supports the creation of internationalized messages with
/// plural inflection. It is based on plural selection, i.e. the caller
/// specifies messages for each plural case that can appear in the user's
/// language and the `PluralFormat` selects the appropriate message based on
/// the number.
///
/// Equivalent to `com.ibm.icu.text.PluralFormat` (a `UFormat`). Java
/// `null` fields are `None`. The `MessagePattern` is shared with the clones,
/// as Java's shallow `clone()` shares it; the number format is shared by
/// reference, as `setNumberFormat` and the RBNF rules share it in Java. The
/// setters take `&mut self`.
#[derive(Debug, Clone)]
pub struct PluralFormat {
    /// The locales of `UFormat`.
    uformat: UFormatBase,
    /// The locale used for standard number formatting and getting the
    /// predefined plural rules (if they were not defined explicitly).
    /// `ulocale`.
    ulocale: Option<ULocale>,
    /// The plural rules used for plural selection. `pluralRules`.
    plural_rules: Option<Arc<PluralRules>>,
    /// The pattern string or `None` if none was applied. `pattern`.
    pattern: Option<Vec<u16>>,
    /// The MessagePattern which contains the parsed structure of the
    /// pattern string. `msgPattern` (transient).
    msg_pattern: Option<Arc<Mutex<MessagePattern>>>,
    /// Obsolete with use of MessagePattern since ICU 4.8. Used to be: A
    /// map with plural keywords as the keys and the parsed message strings
    /// as values. Kept only for serialization compatibility; always `None`.
    /// `parsedValues`.
    parsed_values: Option<HashMap<String, String>>,
    /// This instance is used to format the numbers in the standard plural
    /// form. `numberFormat`.
    number_format: Option<Arc<dyn NumberFormat>>,
    /// The number of the plural offset. `offset` (transient).
    offset: f64,
}

impl PluralFormat {
    /// The field values before `init`.
    fn empty() -> Self {
        PluralFormat {
            uformat: UFormatBase::new(),
            ulocale: None,
            plural_rules: None,
            pattern: None,
            msg_pattern: None,
            parsed_values: None,
            number_format: None,
            offset: 0.0,
        }
    }

    /// `init(rules, type, locale, numberFormat)` then, when given,
    /// `applyPattern(pattern)`.
    fn create(
        rules: Option<Arc<PluralRules>>,
        plural_type: PluralType,
        locale: ULocale,
        number_format: Option<Arc<dyn NumberFormat>>,
        pattern: Option<&[u16]>,
    ) -> Result<Self> {
        let mut pf = Self::empty();
        pf.init(rules, plural_type, locale, number_format)?;
        if let Some(pattern) = pattern {
            pf.apply_pattern(pattern)?;
        }
        Ok(pf)
    }

    /// Creates a new cardinal-number `PluralFormat` for the default
    /// `FORMAT` locale. Equivalent to `PluralFormat()`.
    ///
    /// # Errors
    ///
    /// The errors of the plural rules and of the number format.
    pub fn new() -> Result<Self> {
        Self::create(
            None,
            PluralType::Cardinal,
            ULocale::get_default_category(Category::Format),
            None,
            None,
        )
    }

    /// Creates a new cardinal-number `PluralFormat` for a given locale.
    /// Equivalent to `PluralFormat(ULocale ulocale)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn for_ulocale(ulocale: &ULocale) -> Result<Self> {
        Self::create(None, PluralType::Cardinal, ulocale.clone(), None, None)
    }

    /// Equivalent to `PluralFormat(Locale locale)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn for_locale(locale: &Locale) -> Result<Self> {
        Self::for_ulocale(&for_locale(locale)?)
    }

    /// Creates a new cardinal-number `PluralFormat` for a given set of rules
    /// and the default `FORMAT` locale. Equivalent to
    /// `PluralFormat(PluralRules rules)`.
    ///
    /// # Errors
    ///
    /// The errors of the number format.
    pub fn with_rules(rules: Arc<PluralRules>) -> Result<Self> {
        Self::create(
            Some(rules),
            PluralType::Cardinal,
            ULocale::get_default_category(Category::Format),
            None,
            None,
        )
    }

    /// Equivalent to `PluralFormat(ULocale ulocale, PluralRules rules)`.
    ///
    /// # Errors
    ///
    /// As [`with_rules`](Self::with_rules).
    pub fn for_ulocale_rules(ulocale: &ULocale, rules: Arc<PluralRules>) -> Result<Self> {
        Self::create(
            Some(rules),
            PluralType::Cardinal,
            ulocale.clone(),
            None,
            None,
        )
    }

    /// Equivalent to `PluralFormat(Locale locale, PluralRules rules)`.
    ///
    /// # Errors
    ///
    /// As [`with_rules`](Self::with_rules).
    pub fn for_locale_rules(locale: &Locale, rules: Arc<PluralRules>) -> Result<Self> {
        Self::for_ulocale_rules(&for_locale(locale)?, rules)
    }

    /// Creates a new `PluralFormat` for the plural type and the locale.
    /// Equivalent to `PluralFormat(ULocale ulocale, PluralType type)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn for_ulocale_type(ulocale: &ULocale, plural_type: PluralType) -> Result<Self> {
        Self::create(None, plural_type, ulocale.clone(), None, None)
    }

    /// Equivalent to `PluralFormat(Locale locale, PluralType type)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn for_locale_type(locale: &Locale, plural_type: PluralType) -> Result<Self> {
        Self::for_ulocale_type(&for_locale(locale)?, plural_type)
    }

    /// Creates a new cardinal-number `PluralFormat` for a given pattern
    /// string and the default `FORMAT` locale. Equivalent to
    /// `PluralFormat(String pattern)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new); the `IllegalArgumentException` of an invalid
    /// pattern.
    pub fn from_pattern(pattern: &[u16]) -> Result<Self> {
        Self::create(
            None,
            PluralType::Cardinal,
            ULocale::get_default_category(Category::Format),
            None,
            Some(pattern),
        )
    }

    /// Equivalent to `PluralFormat(ULocale ulocale, String pattern)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn for_ulocale_pattern(ulocale: &ULocale, pattern: &[u16]) -> Result<Self> {
        Self::create(
            None,
            PluralType::Cardinal,
            ulocale.clone(),
            None,
            Some(pattern),
        )
    }

    /// Equivalent to `PluralFormat(PluralRules rules, String pattern)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn with_rules_pattern(rules: Arc<PluralRules>, pattern: &[u16]) -> Result<Self> {
        Self::create(
            Some(rules),
            PluralType::Cardinal,
            ULocale::get_default_category(Category::Format),
            None,
            Some(pattern),
        )
    }

    /// Equivalent to `PluralFormat(ULocale ulocale, PluralRules rules,
    /// String pattern)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn for_ulocale_rules_pattern(
        ulocale: &ULocale,
        rules: Arc<PluralRules>,
        pattern: &[u16],
    ) -> Result<Self> {
        Self::create(
            Some(rules),
            PluralType::Cardinal,
            ulocale.clone(),
            None,
            Some(pattern),
        )
    }

    /// Equivalent to `PluralFormat(ULocale ulocale, PluralType type, String
    /// pattern)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn for_ulocale_type_pattern(
        ulocale: &ULocale,
        plural_type: PluralType,
        pattern: &[u16],
    ) -> Result<Self> {
        Self::create(None, plural_type, ulocale.clone(), None, Some(pattern))
    }

    /// Creates a new `PluralFormat` for a plural type, a pattern and a
    /// number format (`None`: the locale's). Equivalent to the
    /// package-private `PluralFormat(ULocale ulocale, PluralType type,
    /// String pattern, NumberFormat numberFormat)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub(crate) fn with_number_format(
        ulocale: &ULocale,
        plural_type: PluralType,
        pattern: &[u16],
        number_format: Option<Arc<dyn NumberFormat>>,
    ) -> Result<Self> {
        Self::create(
            None,
            plural_type,
            ulocale.clone(),
            number_format,
            Some(pattern),
        )
    }

    /// Initializes the `PluralRules` object. Postcondition: `ulocale` is
    /// `locale`; `pluralRules` is `rules` when given, otherwise the
    /// predefined plural rule set for the locale; `parsedValues` and
    /// `pattern` are `None`; `numberFormat` is `number_format` when given,
    /// otherwise a `NumberFormat` for the locale. Equivalent to `init(
    /// PluralRules rules, PluralType type, ULocale locale, NumberFormat
    /// numberFormat)` (private).
    fn init(
        &mut self,
        rules: Option<Arc<PluralRules>>,
        plural_type: PluralType,
        locale: ULocale,
        number_format: Option<Arc<dyn NumberFormat>>,
    ) -> Result<()> {
        self.plural_rules = Some(match rules {
            None => PluralRules::for_locale_type(&locale, plural_type)?,
            Some(rules) => rules,
        });
        self.ulocale = Some(locale);
        self.reset_pattern()?;
        self.number_format = Some(match number_format {
            None => Arc::from(number_format::get_instance_ulocale(
                self.ulocale.as_ref().expect("set above"),
            )?),
            Some(nf) => nf,
        });
        Ok(())
    }

    /// `resetPattern()` (private).
    fn reset_pattern(&mut self) -> Result<()> {
        self.pattern = None;
        if let Some(msg_pattern) = &self.msg_pattern {
            lock(msg_pattern).clear()?;
        }
        self.offset = 0.0;
        Ok(())
    }

    /// Sets the pattern used by this plural format. The method parses the
    /// pattern and creates a map of format strings for the plural rules.
    /// Patterns and their interpretation are specified in the class
    /// description. Equivalent to `applyPattern(String pattern)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if the pattern is invalid (the pattern is
    /// then reset).
    pub fn apply_pattern(&mut self, pattern: &[u16]) -> Result<()> {
        self.pattern = Some(pattern.to_vec());
        let msg_pattern = self
            .msg_pattern
            .get_or_insert_with(|| Arc::new(Mutex::new(MessagePattern::new())))
            .clone();
        let parsed = {
            let mut m = lock(&msg_pattern);
            match m.parse_plural_style(pattern) {
                Ok(_) => Ok(m.get_plural_offset(0)),
                Err(e) => Err(e),
            }
        };
        match parsed {
            Ok(offset) => {
                self.offset = offset;
                Ok(())
            }
            Err(e) => {
                self.reset_pattern()?;
                Err(e)
            }
        }
    }

    /// Returns the pattern for this PluralFormat (`None` when none was
    /// applied). Equivalent to `toPattern()`.
    pub fn to_pattern(&self) -> Option<Vec<u16>> {
        self.pattern.clone()
    }

    /// Finds the PluralFormat sub-message for the given number, or the
    /// "other" sub-message: the index of the sub-message's `MSG_START`
    /// part, or 0. `part_index` is the index of the first `PluralFormat`
    /// argument style part. Equivalent to the package-private static
    /// `findSubMessage(MessagePattern pattern, int partIndex, PluralSelector
    /// selector, Object context, double number)`.
    pub(crate) fn find_sub_message<C: ?Sized>(
        pattern: &MessagePattern,
        part_index: i32,
        selector: &dyn PluralSelector<C>,
        context: &C,
        number: f64,
    ) -> i32 {
        let mut part_index = part_index;
        let count = pattern.count_parts();
        let offset;
        let mut part = pattern.get_part(part_index);
        if part.get_type().has_numeric_value() {
            offset = pattern.get_numeric_value(&part);
            part_index += 1;
        } else {
            offset = 0.0;
        }
        // The keyword is null until we need to match against a non-explicit, not-"other" value.
        // Then we get the keyword from the selector.
        // (In other words, we never call the selector if we match against an explicit value,
        // or if the only non-explicit keyword is "other".)
        let mut keyword: Option<String> = None;
        // When we find a match, we set msgStart>0 and also set this boolean to true
        // to avoid matching the keyword again (duplicates are allowed)
        // while we continue to look for an explicit-value match.
        let mut have_keyword_match = false;
        // msgStart is 0 until we find any appropriate sub-message.
        // We remember the first "other" sub-message if we have not seen any
        // appropriate sub-message before.
        // We remember the first matching-keyword sub-message if we have not seen
        // one of those before.
        // (The parser allows [does not check for] duplicate keywords.
        // We just have to make sure to take the first one.)
        // We avoid matching the keyword twice by also setting haveKeywordMatch=true
        // at the first keyword match.
        // We keep going until we find an explicit-value match or reach the end of the plural style.
        let mut msg_start = 0;
        // Iterate over (ARG_SELECTOR [ARG_INT|ARG_DOUBLE] message) tuples
        // until ARG_LIMIT or end of plural-only pattern.
        let other = units("other");
        loop {
            part = pattern.get_part(part_index);
            part_index += 1;
            let type_ = part.get_type();
            if type_ == PartType::ArgLimit {
                break;
            }
            debug_assert!(type_ == PartType::ArgSelector);
            // part is an ARG_SELECTOR followed by an optional explicit value, and then a message
            if pattern.get_part_type(part_index).has_numeric_value() {
                // explicit value like "=2"
                part = pattern.get_part(part_index);
                part_index += 1;
                if number == pattern.get_numeric_value(&part) {
                    // matches explicit value
                    return part_index;
                }
            } else if !have_keyword_match {
                // plural keyword like "few" or "other"
                // Compare "other" first and call the selector if this is not "other".
                if pattern.part_substring_matches(&part, &other) {
                    if msg_start == 0 {
                        msg_start = part_index;
                        if keyword.as_deref() == Some("other") {
                            // This is the first "other" sub-message,
                            // and the selected keyword is also "other".
                            // Do not match "other" again.
                            have_keyword_match = true;
                        }
                    }
                } else {
                    if keyword.is_none() {
                        let k = selector.select(context, number - offset);
                        if msg_start != 0 && k == "other" {
                            // We have already seen an "other" sub-message.
                            // Do not match "other" again.
                            have_keyword_match = true;
                            // Skip keyword matching but do getLimitPartIndex().
                        }
                        keyword = Some(k);
                    }
                    if !have_keyword_match
                        && pattern.part_substring_matches(
                            &part,
                            &units(keyword.as_deref().expect("set above")),
                        )
                    {
                        // keyword matches
                        msg_start = part_index;
                        // Do not match this keyword again.
                        have_keyword_match = true;
                    }
                }
            }
            part_index = pattern.get_limit_part_index(part_index);
            part_index += 1;
            if part_index >= count {
                break;
            }
        }
        msg_start
    }

    /// Formats a plural message for a given number. Equivalent to the final
    /// `format(double number)`.
    ///
    /// # Errors
    ///
    /// The errors of the number format; `NullPointerException` without a
    /// number format.
    pub fn format_f64(&self, number: f64) -> Result<Vec<u16>> {
        self.format_number(&JavaNumber::Double(number), number)
    }

    /// `format(Number numberObject, double number)` (private).
    fn format_number(&self, number_object: &JavaNumber, number: f64) -> Result<Vec<u16>> {
        let number_format = self.number_format.as_ref().ok_or_else(npe)?;
        // If no pattern was applied, return the formatted number.
        let msg_pattern = match &self.msg_pattern {
            Some(m) => m,
            None => return format_number_object(number_format.as_ref(), number_object),
        };
        let msg_pattern = lock(msg_pattern);
        if msg_pattern.count_parts() == 0 {
            return format_number_object(number_format.as_ref(), number_object);
        }

        // Get the appropriate sub-message.
        // Select it based on the formatted number-offset.
        let number_minus_offset = number - self.offset;
        let plural_rules = self.plural_rules.as_ref().ok_or_else(npe)?;
        let plural_rules_wrapper = PluralSelectorAdapter { plural_rules };
        let number_string;
        let mut part_index;
        if let Some(decimal_format) = as_decimal_format(number_format.as_ref()) {
            // Call NumberFormatter to get both the DecimalQuantity and the string.
            let f = decimal_format.to_number_formatter();
            let result = if self.offset == 0.0 {
                // could be BigDecimal etc.
                f.format_number(number_object)?
            } else {
                f.format_double(number_minus_offset)?
            };
            number_string = FormattedValue::to_java_string(&result);
            let dec: &dyn IFixedDecimal = result.get_fixed_decimal();
            part_index =
                Self::find_sub_message(&msg_pattern, 0, &plural_rules_wrapper, dec, number);
        } else {
            if self.offset == 0.0 {
                number_string = format_number_object(number_format.as_ref(), number_object)?;
            } else {
                number_string = number_format.format_f64(number_minus_offset)?;
            }
            let dec = FixedDecimal::from_double(number_minus_offset);
            part_index = Self::find_sub_message(
                &msg_pattern,
                0,
                &plural_rules_wrapper,
                &dec as &dyn IFixedDecimal,
                number,
            );
        }
        let pattern = self.pattern.as_deref().ok_or_else(npe)?;
        // Replace syntactic # signs in the top level of this sub-message
        // (not in nested arguments) with the formatted number-offset.
        let mut result: Option<Vec<u16>> = None;
        let mut prev_index = msg_pattern.get_part(part_index).get_limit();
        loop {
            part_index += 1;
            let part = msg_pattern.get_part(part_index);
            let type_ = part.get_type();
            let mut index = part.get_index();
            if type_ == PartType::MsgLimit {
                return Ok(match result {
                    None => substring(pattern, prev_index, index).to_vec(),
                    Some(mut r) => {
                        r.extend_from_slice(substring(pattern, prev_index, index));
                        r
                    }
                });
            } else if type_ == PartType::ReplaceNumber
                // JDK compatibility mode: Remove SKIP_SYNTAX.
                || (type_ == PartType::SkipSyntax && msg_pattern.jdk_apos_mode())
            {
                let r = result.get_or_insert_with(Vec::new);
                r.extend_from_slice(substring(pattern, prev_index, index));
                if type_ == PartType::ReplaceNumber {
                    r.extend_from_slice(&number_string);
                }
                prev_index = part.get_limit();
            } else if type_ == PartType::ArgStart {
                let r = result.get_or_insert_with(Vec::new);
                r.extend_from_slice(substring(pattern, prev_index, index));
                prev_index = index;
                part_index = msg_pattern.get_limit_part_index(part_index);
                index = msg_pattern.get_part(part_index).get_limit();
                MessagePattern::append_reduced_apostrophes(pattern, prev_index, index, r);
                prev_index = index;
            }
        }
    }

    /// This method is not yet supported by `PluralFormat`. Equivalent to
    /// `parse(String text, ParsePosition parsePosition)`.
    ///
    /// # Errors
    ///
    /// Always `UnsupportedOperationException`.
    pub fn parse(
        &self,
        _text: &[u16],
        _parse_position: &mut ParsePosition,
    ) -> Result<Option<JavaNumber>> {
        // You get number ranges from this. You can't get an exact number.
        Err(unsupported_operation())
    }

    /// This method returns the PluralRules type found from parsing: the
    /// keyword of the longest sub-message found in `source` at or after
    /// `pos`'s begin index, with `pos` set to the match (`-1`, `-1` and
    /// `None` when none). Equivalent to the package-private
    /// `parseType(String source, RbnfLenientScanner scanner, FieldPosition
    /// pos)`.
    pub(crate) fn parse_type(
        &self,
        source: &[u16],
        scanner: Option<&dyn RbnfLenientScanner>,
        pos: &mut FieldPosition,
    ) -> Option<Vec<u16>> {
        // If no pattern was applied, return null.
        let msg_pattern = match &self.msg_pattern {
            Some(m) => lock(m),
            None => {
                pos.set_begin_index(-1);
                pos.set_end_index(-1);
                return None;
            }
        };
        if msg_pattern.count_parts() == 0 {
            pos.set_begin_index(-1);
            pos.set_end_index(-1);
            return None;
        }
        let pattern = self
            .pattern
            .as_deref()
            .unwrap_or_else(|| panic!("{}", npe()));
        let mut part_index = 0;
        let mut curr_match_index;
        let count = msg_pattern.count_parts();
        let mut starting_at = pos.get_begin_index();
        if starting_at < 0 {
            starting_at = 0;
        }

        // The keyword is null until we need to match against a non-explicit, not-"other" value.
        // Then we get the keyword from the selector.
        // (In other words, we never call the selector if we match against an explicit value,
        // or if the only non-explicit keyword is "other".)
        let mut keyword: Option<Vec<u16>> = None;
        let mut matched_word: Option<Vec<u16>> = None;
        let mut matched_index = -1;
        // Iterate over (ARG_SELECTOR ARG_START message ARG_LIMIT) tuples
        // until the end of the plural-only pattern.
        while part_index < count {
            let part_selector = msg_pattern.get_part(part_index);
            part_index += 1;
            if part_selector.get_type() != PartType::ArgSelector {
                // Bad format
                continue;
            }

            let part_start = msg_pattern.get_part(part_index);
            part_index += 1;
            if part_start.get_type() != PartType::MsgStart {
                // Bad format
                continue;
            }

            let part_limit = msg_pattern.get_part(part_index);
            part_index += 1;
            if part_limit.get_type() != PartType::MsgLimit {
                // Bad format
                continue;
            }

            let curr_arg = substring(pattern, part_start.get_limit(), part_limit.get_index());
            if let Some(scanner) = scanner {
                // Check if non-lenient rule finds the text before call lenient parsing
                let temp_pos = index_of(source, curr_arg, starting_at);
                if temp_pos >= 0 {
                    curr_match_index = temp_pos;
                } else {
                    // If lenient parsing is turned ON, we've got some time consuming parsing ahead of us.
                    let scanner_match_result = scanner.find_text(source, curr_arg, starting_at);
                    curr_match_index = scanner_match_result[0];
                }
            } else {
                curr_match_index = index_of(source, curr_arg, starting_at);
            }
            if curr_match_index >= 0
                && curr_match_index >= matched_index
                && matched_word
                    .as_ref()
                    .is_none_or(|w| curr_arg.len() > w.len())
            {
                matched_index = curr_match_index;
                matched_word = Some(curr_arg.to_vec());
                keyword = Some(
                    substring(pattern, part_start.get_limit(), part_limit.get_index()).to_vec(),
                );
            }
        }
        if let Some(keyword) = keyword {
            pos.set_begin_index(matched_index);
            pos.set_end_index(
                matched_index + matched_word.expect("set with the keyword").len() as i32,
            );
            return Some(keyword);
        }

        // Not found!
        pos.set_begin_index(-1);
        pos.set_end_index(-1);
        None
    }

    /// Sets the locale used by this `PluralFormat` object (`None`: the
    /// default `FORMAT` locale). Note: Calling this method resets this
    /// `PluralFormat` object, i.e., a pattern that was applied previously
    /// will be removed, and the NumberFormat is set to the default number
    /// format for the locale. Equivalent to the deprecated
    /// `setLocale(ULocale ulocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the plural rules and of the number format.
    pub fn set_locale(&mut self, ulocale: Option<&ULocale>) -> Result<()> {
        let ulocale = match ulocale {
            Some(l) => l.clone(),
            None => ULocale::get_default_category(Category::Format),
        };
        self.init(None, PluralType::Cardinal, ulocale, None)
    }

    /// Sets the number format used by this formatter (`None` is Java's
    /// `null`). Equivalent to `setNumberFormat(NumberFormat format)`.
    pub fn set_number_format(&mut self, format: Option<Arc<dyn NumberFormat>>) {
        self.number_format = format;
    }

    /// Returns the locale of the `UFormat` (`ACTUAL_LOCALE` or
    /// `VALID_LOCALE`). Equivalent to the inherited `getLocale(ULocale.Type
    /// type)`.
    pub fn get_locale(&self, locale_type: u_locale::Type) -> Option<ULocale> {
        self.uformat.get_locale(locale_type)
    }

    /// Compares locale, plural rules, message pattern and number format.
    /// Equivalent to `equals(Object rhs)` and `equals(PluralFormat rhs)`.
    pub fn java_equals(&self, pf: &PluralFormat) -> bool {
        if std::ptr::eq(self, pf) {
            return true;
        }
        let rules_equal = match (&self.plural_rules, &pf.plural_rules) {
            (None, None) => true,
            (Some(a), Some(b)) => a.equals(Some(b)),
            _ => false,
        };
        let patterns_equal = match (&self.msg_pattern, &pf.msg_pattern) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || *lock(a) == *lock(b),
            _ => false,
        };
        let formats_equal = match (&self.number_format, &pf.number_format) {
            (None, None) => true,
            (Some(a), Some(b)) => a.java_equals(b.as_ref()),
            _ => false,
        };
        self.ulocale == pf.ulocale && rules_equal && patterns_equal && formats_equal
    }

    /// `hashCode()`: `pluralRules.hashCode() ^ parsedValues.hashCode()`.
    ///
    /// # Errors
    ///
    /// `NullPointerException`: `parsedValues` is always `null` in this
    /// version, as in Java.
    pub fn hash_code(&self) -> Result<i32> {
        let rules = self.plural_rules.as_ref().ok_or_else(npe)?.hash_code();
        let parsed_values = self.parsed_values.as_ref().ok_or_else(npe)?;
        // AbstractMap.hashCode(): the sum of the entries' hash codes
        // (key.hashCode() ^ value.hashCode()).
        let map_hash = parsed_values.iter().fold(0i32, |h, (k, v)| {
            h.wrapping_add(java_string_hash(k) ^ java_string_hash(v))
        });
        Ok(rules ^ map_hash)
    }

    /// Equivalent to `toString()`: `locale=<locale>, rules='<rules>',
    /// pattern='<pattern>', format='<number format>'`.
    ///
    /// # Errors
    ///
    /// The errors of the number format's `toString()`.
    pub fn to_java_string(&self) -> Result<Vec<u16>> {
        let mut buf = units("locale=");
        buf.extend_from_slice(&units(
            &self
                .ulocale
                .as_ref()
                .map_or_else(|| "null".to_string(), |l| l.to_string()),
        ));
        buf.extend_from_slice(&units(", rules='"));
        buf.extend_from_slice(&units(
            &self
                .plural_rules
                .as_ref()
                .map_or_else(|| "null".to_string(), |r| r.to_string()),
        ));
        buf.extend_from_slice(&units("'"));
        buf.extend_from_slice(&units(", pattern='"));
        match &self.pattern {
            Some(p) => buf.extend_from_slice(p),
            None => buf.extend_from_slice(&units("null")),
        }
        buf.extend_from_slice(&units("'"));
        buf.extend_from_slice(&units(", format='"));
        match &self.number_format {
            Some(nf) => buf.extend_from_slice(&number_format_to_java_string(nf.as_ref())?),
            None => buf.extend_from_slice(&units("null")),
        }
        buf.extend_from_slice(&units("'"));
        Ok(buf)
    }

    /// `readObject(ObjectInputStream)` after `defaultReadObject()`: ignores
    /// the parsed values of an earlier class version (before ICU 4.8) and
    /// rebuilds the message pattern.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an invalid pattern.
    pub fn read_object(&mut self) -> Result<()> {
        // Ignore the parsedValues from an earlier class version (before ICU 4.8)
        // and rebuild the msgPattern.
        self.parsed_values = None;
        if let Some(pattern) = self.pattern.clone() {
            self.apply_pattern(&pattern)?;
        }
        Ok(())
    }
}

/// `ULocale.forLocale(Locale)` of a non-`null` locale.
fn for_locale(locale: &Locale) -> Result<ULocale> {
    ULocale::for_locale(Some(locale))?.ok_or_else(npe)
}

/// `new UnsupportedOperationException()`.
fn unsupported_operation() -> LuceneError {
    LuceneError::unchecked_with_message("java.lang.UnsupportedOperationException", None)
}

/// `String.hashCode()` of a Rust string (over its UTF-16 units).
fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// `numberFormat.format(Object)` of a boxed number: Java's
/// `Format.format(Object)`, which dispatches `NumberFormat.format(Object,
/// StringBuffer, FieldPosition)` on the number's class.
fn format_number_object(number_format: &dyn NumberFormat, number: &JavaNumber) -> Result<Vec<u16>> {
    let mut buf = Vec::new();
    number_format.format_number_to(number, &mut buf, &mut FieldPosition::new(0))?;
    Ok(buf)
}

/// `NumberFormat.toString()` of the number format: the overrides of the
/// crate's `NumberFormat` classes (`DecimalFormat`, and through it
/// `CompactDecimalFormat`, and `RuleBasedNumberFormat`).
///
/// **Divergence.** Java renders another `NumberFormat` subclass with
/// `Object.toString()`, `getClass().getName() + "@" +
/// Integer.toHexString(hashCode())` (`NumberFormat.hashCode()` is
/// overridden); a Rust implementor of the `NumberFormat` trait has no Java
/// class name, so its Rust type name stands in for it.
fn number_format_to_java_string(number_format: &dyn NumberFormat) -> Result<Vec<u16>> {
    if let Some(df) = as_decimal_format(number_format) {
        return Ok(units(&df.to_java_string()));
    }
    let any = Format::as_any(number_format);
    if let Some(rbnf) = any.downcast_ref::<RuleBasedNumberFormat>() {
        return rbnf.to_java_string();
    }
    Ok(units(&format!(
        "{}@{:x}",
        std::any::type_name_of_val(any),
        number_format.hash_code()
    )))
}

/// The `java.lang.Number` a formatted object stands for, `None` when it is
/// not a number.
fn as_java_number(obj: &dyn Any) -> Option<JavaNumber> {
    if let Some(n) = obj.downcast_ref::<JavaNumber>() {
        Some(n.clone())
    } else if let Some(l) = obj.downcast_ref::<i64>() {
        Some(JavaNumber::Long(*l))
    } else if let Some(i) = obj.downcast_ref::<i32>() {
        Some(JavaNumber::Integer(*i))
    } else if let Some(d) = obj.downcast_ref::<f64>() {
        Some(JavaNumber::Double(*d))
    } else if let Some(f) = obj.downcast_ref::<f32>() {
        Some(JavaNumber::Float(*f))
    } else if let Some(b) = obj.downcast_ref::<BigInteger>() {
        Some(JavaNumber::BigInteger(b.clone()))
    } else if let Some(b) = obj.downcast_ref::<BigDecimal>() {
        Some(JavaNumber::BigDecimal(b.clone()))
    } else {
        obj.downcast_ref::<IcuBigDecimal>()
            .map(|b| JavaNumber::IcuBigDecimal(b.clone()))
    }
}

/// The `String.valueOf(Object)` of a non-number object in the exception
/// message, for the text types; other objects have no Java rendering.
fn object_to_string(obj: &dyn Any) -> String {
    if let Some(s) = obj.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = obj.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = obj.downcast_ref::<Vec<u16>>() {
        String::from_utf16_lossy(s)
    } else {
        "<object>".to_string()
    }
}

impl Format for PluralFormat {
    /// Formats a plural message for a given number and appends the
    /// formatted message to the given `StringBuffer`. Equivalent to
    /// `format(Object number, StringBuffer toAppendTo, FieldPosition pos)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("'<number>' is not a Number")`; the errors
    /// of the number format.
    fn format_object_to(
        &self,
        number: &dyn Any,
        to_append_to: &mut Vec<u16>,
        _pos: &mut FieldPosition,
    ) -> Result<()> {
        let number_object = match as_java_number(number) {
            Some(n) => n,
            None => {
                return Err(LuceneError::illegal_argument(format!(
                    "'{}' is not a Number",
                    object_to_string(number)
                )));
            }
        };
        to_append_to
            .extend_from_slice(&self.format_number(&number_object, number_object.double_value())?);
        Ok(())
    }

    /// This method is not yet supported by `PluralFormat`. Equivalent to
    /// `parseObject(String source, ParsePosition pos)`.
    fn parse_object_at(
        &self,
        _source: &[u16],
        _pos: &mut ParsePosition,
    ) -> Result<Option<ParsedObject>> {
        Err(unsupported_operation())
    }

    fn clone_format(&self) -> Box<dyn Format> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
