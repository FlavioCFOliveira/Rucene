// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.CurrencySpacingEnabledModifier`.

use std::any::Any;
use std::sync::{Arc, LazyLock};

use rucene::error::Result;

use crate::jdk::text::FieldRef;
use crate::jdk::units;
use crate::r#impl::formatted_string_builder::{is_field, FormattedStringBuilder};
use crate::r#impl::number::constant_multi_field_modifier::ConstantMultiFieldModifier;
use crate::r#impl::number::modifier::{Modifier, Parameters};
use crate::text::decimal_format_symbols::{
    DecimalFormatSymbols, CURRENCY_SPC_CURRENCY_MATCH, CURRENCY_SPC_INSERT, CURRENCY_SPC_SURROUNDING_MATCH,
};
use crate::text::number_format::field as nf_field;
use crate::text::unicode_set::UnicodeSet;

// These are the default currency spacing UnicodeSets in CLDR.
// Pre-compute them for performance.
// The unit test testCurrencySpacingPatternStability() will start failing if
// these change in CLDR.
/// `UNISET_DIGIT`.
static UNISET_DIGIT: LazyLock<Arc<UnicodeSet>> = LazyLock::new(|| frozen("[:digit:]"));
/// `UNISET_NOTSZ`.
static UNISET_NOTSZ: LazyLock<Arc<UnicodeSet>> = LazyLock::new(|| frozen("[[:^S:]&[:^Z:]]"));

fn frozen(pattern: &str) -> Arc<UnicodeSet> {
    let mut s = UnicodeSet::from_pattern(&units(pattern)).expect("the CLDR default spacing sets are valid");
    s.freeze();
    Arc::new(s)
}

// Constants for better readability. Types are for compiler checking.
/// `PREFIX`.
pub(crate) const PREFIX: i8 = 0;
/// `SUFFIX`.
pub(crate) const SUFFIX: i8 = 1;
/// `IN_CURRENCY`.
pub(crate) const IN_CURRENCY: i16 = 0;
/// `IN_NUMBER`.
pub(crate) const IN_NUMBER: i16 = 1;

/// Identical to `ConstantMultiFieldModifier`, but supports currency spacing.
///
/// Equivalent to `com.ibm.icu.impl.number.CurrencySpacingEnabledModifier`
/// (a `ConstantMultiFieldModifier` subclass): it holds its base
/// ([`base`](Self::base)) and overrides `apply`; the other `Modifier`
/// members are the base's. The frozen sets are shared (`Arc`), as Java
/// shares its static sets.
#[derive(Debug, Clone)]
pub struct CurrencySpacingEnabledModifier {
    base: ConstantMultiFieldModifier,
    after_prefix_unicode_set: Option<Arc<UnicodeSet>>,
    after_prefix_insert: Option<Vec<u16>>,
    before_suffix_unicode_set: Option<Arc<UnicodeSet>>,
    before_suffix_insert: Option<Vec<u16>>,
}

impl CurrencySpacingEnabledModifier {
    /// Safe code path. Equivalent to
    /// `CurrencySpacingEnabledModifier(FormattedStringBuilder,
    /// FormattedStringBuilder, boolean, boolean, DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an invalid spacing pattern.
    pub fn new(
        prefix: &FormattedStringBuilder,
        suffix: &FormattedStringBuilder,
        overwrite: bool,
        strong: bool,
        symbols: &DecimalFormatSymbols,
    ) -> Result<Self> {
        let base = ConstantMultiFieldModifier::new(prefix, suffix, overwrite, strong);

        // Check for currency spacing. Do not build the UnicodeSets unless
        // there is a currency code point at a boundary.
        let (after_prefix_unicode_set, after_prefix_insert) =
            if prefix.length() > 0 && is_field(prefix.field_at(prefix.length() - 1), &nf_field::CURRENCY as FieldRef) {
                let prefix_cp = prefix.get_last_code_point();
                let prefix_unicode_set = Self::get_unicode_set(symbols, IN_CURRENCY, PREFIX)?;
                if prefix_unicode_set.contains(prefix_cp)? {
                    // freeze(): the sets this answers are frozen.
                    (Some(Self::get_unicode_set(symbols, IN_NUMBER, PREFIX)?), Some(Self::get_insert_string(symbols, PREFIX)?))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
        let (before_suffix_unicode_set, before_suffix_insert) =
            if suffix.length() > 0 && is_field(suffix.field_at(0), &nf_field::CURRENCY as FieldRef) {
                let suffix_cp = suffix.get_first_code_point();
                let suffix_unicode_set = Self::get_unicode_set(symbols, IN_CURRENCY, SUFFIX)?;
                if suffix_unicode_set.contains(suffix_cp)? {
                    (Some(Self::get_unicode_set(symbols, IN_NUMBER, SUFFIX)?), Some(Self::get_insert_string(symbols, SUFFIX)?))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
        Ok(Self { base, after_prefix_unicode_set, after_prefix_insert, before_suffix_unicode_set, before_suffix_insert })
    }

    /// The `ConstantMultiFieldModifier` this modifier extends.
    pub fn base(&self) -> &ConstantMultiFieldModifier {
        &self.base
    }

    /// Unsafe code path. Equivalent to
    /// `applyCurrencySpacing(FormattedStringBuilder, int, int, int, int,
    /// DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an invalid spacing pattern.
    pub fn apply_currency_spacing(
        output: &mut FormattedStringBuilder,
        prefix_start: i32,
        prefix_len: i32,
        suffix_start: i32,
        suffix_len: i32,
        symbols: &DecimalFormatSymbols,
    ) -> Result<i32> {
        let mut length = 0;
        let has_prefix = prefix_len > 0;
        let has_suffix = suffix_len > 0;
        let has_number = suffix_start - prefix_start - prefix_len > 0; // could be empty string
        if has_prefix && has_number {
            length += Self::apply_currency_spacing_affix(output, prefix_start + prefix_len, PREFIX, symbols)?;
        }
        if has_suffix && has_number {
            length += Self::apply_currency_spacing_affix(output, suffix_start + length, SUFFIX, symbols)?;
        }
        Ok(length)
    }

    /// Unsafe code path. Equivalent to
    /// `applyCurrencySpacingAffix(FormattedStringBuilder, int, byte,
    /// DecimalFormatSymbols)`.
    fn apply_currency_spacing_affix(
        output: &mut FormattedStringBuilder,
        index: i32,
        affix: i8,
        symbols: &DecimalFormatSymbols,
    ) -> Result<i32> {
        // NOTE: For prefix, output.fieldAt(index-1) gets the last field
        // type in the prefix.
        // This works even if the last code point in the prefix is 2 code
        // units because the field value gets populated to both indices in
        // the field array.
        let affix_field = if affix == PREFIX { output.field_at(index - 1) } else { output.field_at(index) };
        if !is_field(affix_field, &nf_field::CURRENCY as FieldRef) {
            return Ok(0);
        }
        let affix_cp = if affix == PREFIX { output.code_point_before(index) } else { output.code_point_at(index) };
        let affix_uniset = Self::get_unicode_set(symbols, IN_CURRENCY, affix)?;
        if !affix_uniset.contains(affix_cp)? {
            return Ok(0);
        }
        let number_cp = if affix == PREFIX { output.code_point_at(index) } else { output.code_point_before(index) };
        let number_uniset = Self::get_unicode_set(symbols, IN_NUMBER, affix)?;
        if !number_uniset.contains(number_cp)? {
            return Ok(0);
        }
        let spacing_string = Self::get_insert_string(symbols, affix)?;

        // NOTE: This next line *inserts* the spacing string, triggering an
        // arraycopy. It would be more efficient if this could be done
        // before affixes were attached, so that it could be prepended/
        // appended instead of inserted. However, the change would add more
        // complexity than it is worth, and would also be in conflict with
        // the TODO in the C++ code.
        Ok(output.insert(index, &spacing_string, None))
    }

    /// `getUnicodeSet(DecimalFormatSymbols, short, byte)`.
    fn get_unicode_set(symbols: &DecimalFormatSymbols, position: i16, affix: i8) -> Result<Arc<UnicodeSet>> {
        let pattern = symbols
            .get_pattern_for_currency_spacing(
                if position == IN_CURRENCY { CURRENCY_SPC_CURRENCY_MATCH } else { CURRENCY_SPC_SURROUNDING_MATCH },
                affix == SUFFIX,
            )?
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: currency spacing pattern"));
        if pattern == units("[:digit:]").as_slice() {
            Ok(UNISET_DIGIT.clone())
        } else if pattern == units("[[:^S:]&[:^Z:]]").as_slice() {
            Ok(UNISET_NOTSZ.clone())
        } else {
            let mut set = UnicodeSet::from_pattern(pattern)?;
            set.freeze();
            Ok(Arc::new(set))
        }
    }

    /// `getInsertString(DecimalFormatSymbols, byte)`.
    fn get_insert_string(symbols: &DecimalFormatSymbols, affix: i8) -> Result<Vec<u16>> {
        Ok(symbols
            .get_pattern_for_currency_spacing(CURRENCY_SPC_INSERT, affix == SUFFIX)?
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: currency spacing insert"))
            .to_vec())
    }
}

impl Modifier for CurrencySpacingEnabledModifier {
    /// Safe code path.
    fn apply(&self, output: &mut FormattedStringBuilder, left_index: i32, right_index: i32) -> Result<i32> {
        // Currency spacing logic
        let mut length = 0;
        if right_index - left_index > 0 {
            if let Some(set) = &self.after_prefix_unicode_set {
                if set.contains(output.code_point_at(left_index)).unwrap_or(false) {
                    // TODO: Should we use the CURRENCY field here?
                    let insert = self.after_prefix_insert.as_deref().unwrap_or(&[]);
                    length += output.insert(left_index, insert, None);
                }
            }
        }
        if right_index - left_index > 0 {
            if let Some(set) = &self.before_suffix_unicode_set {
                if set.contains(output.code_point_before(right_index)).unwrap_or(false) {
                    // TODO: Should we use the CURRENCY field here?
                    let insert = self.before_suffix_insert.as_deref().unwrap_or(&[]);
                    length += output.insert(right_index + length, insert, None);
                }
            }
        }

        // Call super for the remaining logic
        length += self.base.apply(output, left_index, right_index + length)?;
        Ok(length)
    }

    fn get_prefix_length(&self) -> Result<i32> {
        self.base.get_prefix_length()
    }

    fn get_code_point_count(&self) -> Result<i32> {
        self.base.get_code_point_count()
    }

    fn is_strong(&self) -> bool {
        self.base.is_strong()
    }

    fn contains_field(&self, field: FieldRef) -> bool {
        self.base.contains_field(field)
    }

    fn get_parameters(&self) -> Option<&Parameters> {
        self.base.get_parameters()
    }

    fn strict_equals(&self, other: &dyn Modifier) -> bool {
        self.base.strict_equals(other)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
