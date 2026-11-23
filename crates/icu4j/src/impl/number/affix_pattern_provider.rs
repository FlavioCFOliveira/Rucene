// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.AffixPatternProvider`.

/// `AffixPatternProvider.Flags`: the bits of the `flags` argument.
pub struct Flags;

impl Flags {
    /// `PLURAL_MASK`.
    pub const PLURAL_MASK: i32 = 0xff;
    /// `PREFIX`.
    pub const PREFIX: i32 = 0x100;
    /// `NEGATIVE_SUBPATTERN`.
    pub const NEGATIVE_SUBPATTERN: i32 = 0x200;
    /// `PADDING`.
    pub const PADDING: i32 = 0x400;
}

/// `FLAG_POS_PREFIX`.
pub const FLAG_POS_PREFIX: i32 = Flags::PREFIX;
/// `FLAG_POS_SUFFIX`.
pub const FLAG_POS_SUFFIX: i32 = 0;
/// `FLAG_NEG_PREFIX`.
pub const FLAG_NEG_PREFIX: i32 = Flags::PREFIX | Flags::NEGATIVE_SUBPATTERN;
/// `FLAG_NEG_SUFFIX`.
pub const FLAG_NEG_SUFFIX: i32 = Flags::NEGATIVE_SUBPATTERN;

/// A provider of the affix patterns of a decimal format (positive and
/// negative prefix and suffix, padding, per plural form).
///
/// Equivalent to the interface
/// `com.ibm.icu.impl.number.AffixPatternProvider`.
pub trait AffixPatternProvider: std::fmt::Debug + Send + Sync {
    /// `charAt(int flags, int i)`.
    fn char_at(&self, flags: i32, i: i32) -> u16;

    /// `length(int flags)`.
    fn length(&self, flags: i32) -> i32;

    /// `getString(int flags)`.
    fn get_string(&self, flags: i32) -> Vec<u16>;

    /// `hasCurrencySign()`.
    fn has_currency_sign(&self) -> bool;

    /// `positiveHasPlusSign()`.
    fn positive_has_plus_sign(&self) -> bool;

    /// `hasNegativeSubpattern()`.
    fn has_negative_subpattern(&self) -> bool;

    /// `negativeHasMinusSign()`.
    fn negative_has_minus_sign(&self) -> bool;

    /// `containsSymbolType(int type)`: whether a pattern contains the
    /// `AffixUtils.TYPE_*` symbol `type`.
    fn contains_symbol_type(&self, symbol_type: i32) -> bool;

    /// `hasBody()`: whether the pattern has a number placeholder.
    fn has_body(&self) -> bool;

    /// `currencyAsDecimal()`: whether the currency symbol is the decimal
    /// separator.
    fn currency_as_decimal(&self) -> bool;
}
