// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.CurrencyPluralInfoAffixProvider`.

use rucene::error::Result;

use crate::r#impl::number::affix_pattern_provider::{AffixPatternProvider, Flags};
use crate::r#impl::number::decimal_format_properties::DecimalFormatProperties;
use crate::r#impl::number::pattern_string_parser::PatternStringParser;
use crate::r#impl::number::properties_affix_pattern_provider::PropertiesAffixPatternProvider;
use crate::r#impl::standard_plural::StandardPlural;
use crate::text::currency_plural_info::CurrencyPluralInfo;

/// An affix pattern provider with one set of affixes per plural form, from a
/// `CurrencyPluralInfo`.
///
/// Equivalent to `com.ibm.icu.impl.number.CurrencyPluralInfoAffixProvider`.
#[derive(Debug, Clone)]
pub struct CurrencyPluralInfoAffixProvider {
    affixes_by_plural: Vec<PropertiesAffixPatternProvider>,
}

impl CurrencyPluralInfoAffixProvider {
    /// Equivalent to `CurrencyPluralInfoAffixProvider(CurrencyPluralInfo,
    /// DecimalFormatProperties)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of a malformed plural pattern.
    pub fn new(cpi: &CurrencyPluralInfo, properties: &DecimalFormatProperties) -> Result<Self> {
        // We need to use a PropertiesAffixPatternProvider, not the simpler version ParsedPatternInfo,
        // because user-specified affix overrides still need to work.
        let mut affixes_by_plural = Vec::with_capacity(StandardPlural::COUNT as usize);
        let mut plural_properties = DecimalFormatProperties::new();
        plural_properties.copy_from(properties);
        for plural in StandardPlural::VALUES {
            let pattern = cpi.get_currency_plural_pattern(plural.get_keyword());
            PatternStringParser::parse_to_existing_properties(Some(&pattern), &mut plural_properties)?;
            affixes_by_plural.push(PropertiesAffixPatternProvider::new(&plural_properties)?);
        }
        Ok(CurrencyPluralInfoAffixProvider { affixes_by_plural })
    }

    fn other(&self) -> &PropertiesAffixPatternProvider {
        &self.affixes_by_plural[StandardPlural::Other as usize]
    }
}

impl AffixPatternProvider for CurrencyPluralInfoAffixProvider {
    fn char_at(&self, flags: i32, i: i32) -> u16 {
        let plural_ordinal = (flags & Flags::PLURAL_MASK) as usize;
        self.affixes_by_plural[plural_ordinal].char_at(flags, i)
    }

    fn length(&self, flags: i32) -> i32 {
        let plural_ordinal = (flags & Flags::PLURAL_MASK) as usize;
        self.affixes_by_plural[plural_ordinal].length(flags)
    }

    fn get_string(&self, flags: i32) -> Vec<u16> {
        let plural_ordinal = (flags & Flags::PLURAL_MASK) as usize;
        self.affixes_by_plural[plural_ordinal].get_string(flags)
    }

    fn positive_has_plus_sign(&self) -> Result<bool> {
        self.other().positive_has_plus_sign()
    }

    fn has_negative_subpattern(&self) -> bool {
        self.other().has_negative_subpattern()
    }

    fn negative_has_minus_sign(&self) -> Result<bool> {
        self.other().negative_has_minus_sign()
    }

    fn has_currency_sign(&self) -> bool {
        self.other().has_currency_sign()
    }

    fn contains_symbol_type(&self, type_: i32) -> Result<bool> {
        self.other().contains_symbol_type(type_)
    }

    fn has_body(&self) -> bool {
        self.other().has_body()
    }

    fn currency_as_decimal(&self) -> bool {
        self.other().currency_as_decimal()
    }
}

