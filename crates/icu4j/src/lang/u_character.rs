// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UCharacter`.
//!
//! Only the members block 1 of the port (rmp task #643) calls are here,
//! ported early; the rest of the class is ported by rmp task #645.

/// The ICU counterpart of `java.lang.Character`.
///
/// Equivalent to `com.ibm.icu.lang.UCharacter` (members ported so far:
/// [`to_code_point`](Self::to_code_point)).
pub struct UCharacter;

impl UCharacter {
    /// `toCodePoint(int high, int low)`: the code point of a surrogate pair,
    /// `(high << 10) + low - U16_SURROGATE_OFFSET` (`UCharacter.java:6499-6502`),
    /// in Java's wrapping `int` arithmetic.
    pub fn to_code_point(high: i32, low: i32) -> i32 {
        (high << 10)
            .wrapping_add(low)
            .wrapping_sub(Self::U16_SURROGATE_OFFSET)
    }

    /// `U16_SURROGATE_OFFSET` (private in Java):
    /// `(0xd800 << 10) + 0xdc00 - 0x10000`.
    const U16_SURROGATE_OFFSET: i32 = (0xd800 << 10) + 0xdc00 - 0x10000;
}

impl UCharacter {
    /// `getPropertyValueName(int property, int value, int nameChoice)`: the
    /// name of a property value. For the canonical combining class
    /// properties with a value in `0..=0xff` (`getIntPropertyMinValue` is
    /// `0`, `UCharacter.java:6302-6305`; the maximum is `0xff`,
    /// `CombiningClassIntProperty.getMaxValue`, `UCharacterProperty.java:743-745`)
    /// and a valid name choice, an invalid name answers `None` instead of
    /// throwing. Ported early by rmp task #644 (owner #645).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid property, value or name
    /// choice; `MissingResourceException` when the data is missing.
    pub fn get_property_value_name(property: i32, value: i32, name_choice: i32) -> rucene::error::Result<Option<String>> {
        use crate::lang::u_property::{NameChoice, UProperty};
        use crate::r#impl::u_property_aliases::UPropertyAliases;
        let aliases = UPropertyAliases::instance()?;
        if (property == UProperty::CANONICAL_COMBINING_CLASS
            || property == UProperty::LEAD_CANONICAL_COMBINING_CLASS
            || property == UProperty::TRAIL_CANONICAL_COMBINING_CLASS)
            && value >= 0
            && value <= 0xff
            && (0..NameChoice::COUNT).contains(&name_choice)
        {
            // this is hard coded for the valid cc
            // because PropertyValueAliases.txt does not contain all of them
            return match aliases.get_property_value_name(property, value, name_choice) {
                Ok(n) => Ok(n),
                Err(e) if e.is_illegal_argument_exception() => Ok(None),
                Err(e) => Err(e),
            };
        }
        aliases.get_property_value_name(property, value, name_choice)
    }

    /// `getPropertyValueEnumNoThrow(int property, CharSequence valueAlias)`
    /// (internal): `UProperty.UNDEFINED` for an unknown alias. Ported early
    /// by rmp task #644 (owner #645).
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data is missing.
    pub fn get_property_value_enum_no_throw(property: i32, value_alias: &[u16]) -> rucene::error::Result<i32> {
        Ok(crate::r#impl::u_property_aliases::UPropertyAliases::instance()?
            .get_property_value_enum_no_throw(property, value_alias))
    }
}
