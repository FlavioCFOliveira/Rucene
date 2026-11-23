// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UScript`.
//!
//! Only the members block 2 of the port (rmp task #644) calls are here,
//! ported early (`LSR` reads script codes and names); the rest of the class
//! is ported by rmp task #645.

use rucene::error::Result;

use crate::lang::u_character::UCharacter;
use crate::lang::u_property::{NameChoice, UProperty};

/// Constants for ISO 15924 script codes, and related functions.
///
/// Equivalent to `com.ibm.icu.lang.UScript` (members ported so far).
pub struct UScript;

impl UScript {
    /// `INVALID_CODE`.
    pub const INVALID_CODE: i32 = -1;

    /// `getCodeFromName(String nameOrAbbr)`: the script code of a script
    /// name or abbreviation, `INVALID_CODE` when unknown.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the property names data is missing.
    pub fn get_code_from_name(name_or_abbr: &str) -> Result<i32> {
        let units: Vec<u16> = name_or_abbr.encode_utf16().collect();
        let prop_num = UCharacter::get_property_value_enum_no_throw(UProperty::SCRIPT, &units)?;
        Ok(if prop_num == UProperty::UNDEFINED {
            Self::INVALID_CODE
        } else {
            prop_num
        })
    }

    /// `getShortName(int scriptCode)`: the script abbreviation.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid script code;
    /// `MissingResourceException` when the data is missing.
    pub fn get_short_name(script_code: i32) -> Result<Option<String>> {
        UCharacter::get_property_value_name(UProperty::SCRIPT, script_code, NameChoice::SHORT)
    }
}
