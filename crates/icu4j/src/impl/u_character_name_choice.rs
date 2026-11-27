// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UCharacterNameChoice`.

/// Internal class containing selector constants for the unicode character
/// names.
///
/// Equivalent to the interface `com.ibm.icu.impl.UCharacterNameChoice`.
pub struct UCharacterNameChoice;

impl UCharacterNameChoice {
    /// `UNICODE_CHAR_NAME`.
    pub const UNICODE_CHAR_NAME: i32 = 0;
    /// `OBSOLETE_UNUSED_UNICODE_10_CHAR_NAME`.
    pub const OBSOLETE_UNUSED_UNICODE_10_CHAR_NAME: i32 = 1;
    /// `EXTENDED_CHAR_NAME`.
    pub const EXTENDED_CHAR_NAME: i32 = 2;
    /// `CHAR_NAME_ALIAS`: corrected name from NameAliases.txt.
    pub const CHAR_NAME_ALIAS: i32 = 3;
    /// `CHAR_NAME_CHOICE_COUNT`.
    pub const CHAR_NAME_CHOICE_COUNT: i32 = 4;
    /// `ISO_COMMENT_`: ISO 10646 comment, not a real choice (ICU 49).
    pub const ISO_COMMENT: i32 = Self::CHAR_NAME_CHOICE_COUNT;
}
