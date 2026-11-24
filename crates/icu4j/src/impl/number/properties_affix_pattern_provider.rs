// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.PropertiesAffixPatternProvider`.

use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::affix_pattern_provider::{AffixPatternProvider, Flags};
use crate::r#impl::number::affix_utils::{AffixUtils, TYPE_MINUS_SIGN, TYPE_PLUS_SIGN};
use crate::r#impl::number::currency_plural_info_affix_provider::CurrencyPluralInfoAffixProvider;
use crate::r#impl::number::decimal_format_properties::{
    empty_literal, identity_hash_code, new_jstring, DecimalFormatProperties, JString,
};

/// An affix pattern provider over the affixes of a property bag.
///
/// Equivalent to `com.ibm.icu.impl.number.PropertiesAffixPatternProvider`.
#[derive(Debug, Clone)]
pub struct PropertiesAffixPatternProvider {
    pos_prefix: JString,
    pos_suffix: JString,
    neg_prefix: JString,
    neg_suffix: JString,
    is_currency_pattern: bool,
    currency_as_decimal: bool,
}

impl PropertiesAffixPatternProvider {
    /// Equivalent to `forProperties(DecimalFormatProperties)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of a malformed affix pattern, and the
    /// errors of `CurrencyPluralInfoAffixProvider`.
    pub fn for_properties(
        properties: &DecimalFormatProperties,
    ) -> Result<Arc<dyn AffixPatternProvider>> {
        match properties.get_currency_plural_info() {
            None => Ok(Arc::new(PropertiesAffixPatternProvider::new(properties)?)),
            Some(cpi) => Ok(Arc::new(CurrencyPluralInfoAffixProvider::new(
                &cpi, properties,
            )?)),
        }
    }

    /// Equivalent to the package-private
    /// `PropertiesAffixPatternProvider(DecimalFormatProperties)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `AffixUtils.hasCurrencySymbols` for
    /// a malformed affix pattern.
    pub(crate) fn new(properties: &DecimalFormatProperties) -> Result<Self> {
        // There are two ways to set affixes in DecimalFormat: via the pattern string (applyPattern), and via the
        // explicit setters (setPositivePrefix and friends).  The way to resolve the settings is as follows:
        //
        // 1) If the explicit setting is present for the field, use it.
        // 2) Otherwise, follows UTS 35 rules based on the pattern string.
        //
        // Importantly, the explicit setters affect only the one field they override.  If you set the positive
        // prefix, that should not affect the negative prefix.

        // Convenience: Extract the properties into local variables.
        // Variables are named with three chars: [p/n][p/s][o/p]
        // [p/n] => p for positive, n for negative
        // [p/s] => p for prefix, s for suffix
        // [o/p] => o for escaped custom override string, p for pattern string
        let ppo = AffixUtils::escape(properties.get_positive_prefix()).map(|s| new_jstring(&s));
        let pso = AffixUtils::escape(properties.get_positive_suffix()).map(|s| new_jstring(&s));
        let npo = AffixUtils::escape(properties.get_negative_prefix()).map(|s| new_jstring(&s));
        let nso = AffixUtils::escape(properties.get_negative_suffix()).map(|s| new_jstring(&s));
        let ppp = properties.get_positive_prefix_pattern_jstring().cloned();
        let psp = properties.get_positive_suffix_pattern_jstring().cloned();
        let npp = properties.get_negative_prefix_pattern_jstring().cloned();
        let nsp = properties.get_negative_suffix_pattern_jstring().cloned();

        let pos_prefix = if let Some(ppo) = ppo {
            ppo
        } else if let Some(ppp) = ppp.clone() {
            ppp
        } else {
            // UTS 35: Default positive prefix is empty string.
            empty_literal()
        };

        let pos_suffix = if let Some(pso) = pso {
            pso
        } else if let Some(psp) = psp.clone() {
            psp
        } else {
            // UTS 35: Default positive prefix is empty string.
            empty_literal()
        };

        let neg_prefix = if let Some(npo) = npo {
            npo
        } else if let Some(npp) = npp.clone() {
            npp
        } else {
            // UTS 35: Default negative prefix is "-" with positive prefix.
            // Important: If the positive prefix is the empty string, don't prepend "-".
            match &ppp {
                None => new_jstring(&[u16::from(b'-')]),
                Some(ppp) => {
                    let mut s = vec![u16::from(b'-')];
                    s.extend_from_slice(ppp);
                    new_jstring(&s)
                }
            }
        };

        let neg_suffix = if let Some(nso) = nso {
            nso
        } else if let Some(nsp) = nsp.clone() {
            nsp
        } else {
            // UTS 35: Default negative prefix is the positive prefix.
            psp.clone().unwrap_or_else(empty_literal)
        };

        // For declaring if this is a currency pattern, we need to look at the
        // original pattern, not at any user-specified overrides.
        let is_currency_pattern = AffixUtils::has_currency_symbols(ppp.as_deref())?
            || AffixUtils::has_currency_symbols(psp.as_deref())?
            || AffixUtils::has_currency_symbols(npp.as_deref())?
            || AffixUtils::has_currency_symbols(nsp.as_deref())?
            || properties.get_currency_as_decimal();

        Ok(PropertiesAffixPatternProvider {
            pos_prefix,
            pos_suffix,
            neg_prefix,
            neg_suffix,
            is_currency_pattern,
            currency_as_decimal: properties.get_currency_as_decimal(),
        })
    }

    /// `getString(int)` as the Java reference.
    fn get_jstring(&self, flags: i32) -> &JString {
        let prefix = (flags & Flags::PREFIX) != 0;
        let negative = (flags & Flags::NEGATIVE_SUBPATTERN) != 0;
        if prefix && negative {
            &self.neg_prefix
        } else if prefix {
            &self.pos_prefix
        } else if negative {
            &self.neg_suffix
        } else {
            &self.pos_suffix
        }
    }

    /// Equivalent to `toString()`: `Object.toString()` (identity hash
    /// stand-in) followed by ` {posPrefix#posSuffix;negPrefix#negSuffix}`.
    pub fn to_java_string(&self) -> String {
        format!(
            "com.ibm.icu.impl.number.PropertiesAffixPatternProvider@{:x} {{{}#{};{}#{}}}",
            identity_hash_code("com.ibm.icu.impl.number.PropertiesAffixPatternProvider"),
            String::from_utf16_lossy(&self.pos_prefix),
            String::from_utf16_lossy(&self.pos_suffix),
            String::from_utf16_lossy(&self.neg_prefix),
            String::from_utf16_lossy(&self.neg_suffix)
        )
    }
}

impl AffixPatternProvider for PropertiesAffixPatternProvider {
    /// Equivalent to `charAt(int, int)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` outside the affix.
    fn char_at(&self, flags: i32, i: i32) -> u16 {
        let s = self.get_jstring(flags);
        if i < 0 || i as usize >= s.len() {
            crate::jdk::string_index_out_of_bounds(i);
        }
        s[i as usize]
    }

    fn length(&self, flags: i32) -> i32 {
        self.get_jstring(flags).len() as i32
    }

    fn get_string(&self, flags: i32) -> Vec<u16> {
        self.get_jstring(flags).to_vec()
    }

    fn positive_has_plus_sign(&self) -> Result<bool> {
        Ok(
            AffixUtils::contains_type(Some(&self.pos_prefix), TYPE_PLUS_SIGN)?
                || AffixUtils::contains_type(Some(&self.pos_suffix), TYPE_PLUS_SIGN)?,
        )
    }

    /// Equivalent to `hasNegativeSubpattern()`; the suffix comparison is the
    /// Java reference comparison `negSuffix != posSuffix`.
    fn has_negative_subpattern(&self) -> bool {
        // See comments in the constructor for more information on why this is necessary.
        !Arc::ptr_eq(&self.neg_suffix, &self.pos_suffix)
            || self.neg_prefix.len() != self.pos_prefix.len() + 1
            || self.neg_prefix[1..] != self.pos_prefix[..]
            || self.neg_prefix[0] != u16::from(b'-')
    }

    fn negative_has_minus_sign(&self) -> Result<bool> {
        Ok(
            AffixUtils::contains_type(Some(&self.neg_prefix), TYPE_MINUS_SIGN)?
                || AffixUtils::contains_type(Some(&self.neg_suffix), TYPE_MINUS_SIGN)?,
        )
    }

    fn has_currency_sign(&self) -> bool {
        self.is_currency_pattern
    }

    fn contains_symbol_type(&self, type_: i32) -> Result<bool> {
        Ok(AffixUtils::contains_type(Some(&self.pos_prefix), type_)?
            || AffixUtils::contains_type(Some(&self.pos_suffix), type_)?
            || AffixUtils::contains_type(Some(&self.neg_prefix), type_)?
            || AffixUtils::contains_type(Some(&self.neg_suffix), type_)?)
    }

    fn has_body(&self) -> bool {
        true
    }

    fn currency_as_decimal(&self) -> bool {
        self.currency_as_decimal
    }
}
