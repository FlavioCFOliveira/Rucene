// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UCharacterEnums`.

/// A container for the different 'enumerated types' used by `UCharacter`.
///
/// Equivalent to `com.ibm.icu.lang.UCharacterEnums` (a class with a private
/// constructor holding the two interfaces below).
pub struct UCharacterEnums;

/// 'Enum' for the CharacterCategory constants.
///
/// Equivalent to the interface `UCharacterEnums.ECharacterCategory`.
pub struct ECharacterCategory;

impl ECharacterCategory {
    /// `UNASSIGNED`.
    pub const UNASSIGNED: i8 = 0;
    /// `GENERAL_OTHER_TYPES`.
    pub const GENERAL_OTHER_TYPES: i8 = 0;
    /// `UPPERCASE_LETTER`.
    pub const UPPERCASE_LETTER: i8 = 1;
    /// `LOWERCASE_LETTER`.
    pub const LOWERCASE_LETTER: i8 = 2;
    /// `TITLECASE_LETTER`.
    pub const TITLECASE_LETTER: i8 = 3;
    /// `MODIFIER_LETTER`.
    pub const MODIFIER_LETTER: i8 = 4;
    /// `OTHER_LETTER`.
    pub const OTHER_LETTER: i8 = 5;
    /// `NON_SPACING_MARK`.
    pub const NON_SPACING_MARK: i8 = 6;
    /// `ENCLOSING_MARK`.
    pub const ENCLOSING_MARK: i8 = 7;
    /// `COMBINING_SPACING_MARK`.
    pub const COMBINING_SPACING_MARK: i8 = 8;
    /// `DECIMAL_DIGIT_NUMBER`.
    pub const DECIMAL_DIGIT_NUMBER: i8 = 9;
    /// `LETTER_NUMBER`.
    pub const LETTER_NUMBER: i8 = 10;
    /// `OTHER_NUMBER`.
    pub const OTHER_NUMBER: i8 = 11;
    /// `SPACE_SEPARATOR`.
    pub const SPACE_SEPARATOR: i8 = 12;
    /// `LINE_SEPARATOR`.
    pub const LINE_SEPARATOR: i8 = 13;
    /// `PARAGRAPH_SEPARATOR`.
    pub const PARAGRAPH_SEPARATOR: i8 = 14;
    /// `CONTROL`.
    pub const CONTROL: i8 = 15;
    /// `FORMAT`.
    pub const FORMAT: i8 = 16;
    /// `PRIVATE_USE`.
    pub const PRIVATE_USE: i8 = 17;
    /// `SURROGATE`.
    pub const SURROGATE: i8 = 18;
    /// `DASH_PUNCTUATION`.
    pub const DASH_PUNCTUATION: i8 = 19;
    /// `START_PUNCTUATION`.
    pub const START_PUNCTUATION: i8 = 20;
    /// `END_PUNCTUATION`.
    pub const END_PUNCTUATION: i8 = 21;
    /// `CONNECTOR_PUNCTUATION`.
    pub const CONNECTOR_PUNCTUATION: i8 = 22;
    /// `OTHER_PUNCTUATION`.
    pub const OTHER_PUNCTUATION: i8 = 23;
    /// `MATH_SYMBOL`.
    pub const MATH_SYMBOL: i8 = 24;
    /// `CURRENCY_SYMBOL`.
    pub const CURRENCY_SYMBOL: i8 = 25;
    /// `MODIFIER_SYMBOL`.
    pub const MODIFIER_SYMBOL: i8 = 26;
    /// `OTHER_SYMBOL`.
    pub const OTHER_SYMBOL: i8 = 27;
    /// `INITIAL_PUNCTUATION`.
    pub const INITIAL_PUNCTUATION: i8 = 28;
    /// `INITIAL_QUOTE_PUNCTUATION`.
    pub const INITIAL_QUOTE_PUNCTUATION: i8 = 28;
    /// `FINAL_PUNCTUATION`.
    pub const FINAL_PUNCTUATION: i8 = 29;
    /// `FINAL_QUOTE_PUNCTUATION`.
    pub const FINAL_QUOTE_PUNCTUATION: i8 = 29;
    /// `CHAR_CATEGORY_COUNT`.
    pub const CHAR_CATEGORY_COUNT: i8 = 30;
    /// `LEFT_TO_RIGHT`.
    pub const LEFT_TO_RIGHT: i32 = 0;
    /// `DIRECTIONALITY_LEFT_TO_RIGHT`.
    pub const DIRECTIONALITY_LEFT_TO_RIGHT: i8 = (Self::LEFT_TO_RIGHT) as i8;
    /// `RIGHT_TO_LEFT`.
    pub const RIGHT_TO_LEFT: i32 = 1;
    /// `DIRECTIONALITY_RIGHT_TO_LEFT`.
    pub const DIRECTIONALITY_RIGHT_TO_LEFT: i8 = (Self::RIGHT_TO_LEFT) as i8;
    /// `EUROPEAN_NUMBER`.
    pub const EUROPEAN_NUMBER: i32 = 2;
    /// `DIRECTIONALITY_EUROPEAN_NUMBER`.
    pub const DIRECTIONALITY_EUROPEAN_NUMBER: i8 = (Self::EUROPEAN_NUMBER) as i8;
    /// `EUROPEAN_NUMBER_SEPARATOR`.
    pub const EUROPEAN_NUMBER_SEPARATOR: i32 = 3;
    /// `DIRECTIONALITY_EUROPEAN_NUMBER_SEPARATOR`.
    pub const DIRECTIONALITY_EUROPEAN_NUMBER_SEPARATOR: i8 = (Self::EUROPEAN_NUMBER_SEPARATOR) as i8;
    /// `EUROPEAN_NUMBER_TERMINATOR`.
    pub const EUROPEAN_NUMBER_TERMINATOR: i32 = 4;
    /// `DIRECTIONALITY_EUROPEAN_NUMBER_TERMINATOR`.
    pub const DIRECTIONALITY_EUROPEAN_NUMBER_TERMINATOR: i8 = (Self::EUROPEAN_NUMBER_TERMINATOR) as i8;
    /// `ARABIC_NUMBER`.
    pub const ARABIC_NUMBER: i32 = 5;
    /// `DIRECTIONALITY_ARABIC_NUMBER`.
    pub const DIRECTIONALITY_ARABIC_NUMBER: i8 = (Self::ARABIC_NUMBER) as i8;
    /// `COMMON_NUMBER_SEPARATOR`.
    pub const COMMON_NUMBER_SEPARATOR: i32 = 6;
    /// `DIRECTIONALITY_COMMON_NUMBER_SEPARATOR`.
    pub const DIRECTIONALITY_COMMON_NUMBER_SEPARATOR: i8 = (Self::COMMON_NUMBER_SEPARATOR) as i8;
    /// `BLOCK_SEPARATOR`.
    pub const BLOCK_SEPARATOR: i32 = 7;
    /// `DIRECTIONALITY_PARAGRAPH_SEPARATOR`.
    pub const DIRECTIONALITY_PARAGRAPH_SEPARATOR: i8 = (Self::BLOCK_SEPARATOR) as i8;
    /// `SEGMENT_SEPARATOR`.
    pub const SEGMENT_SEPARATOR: i32 = 8;
    /// `DIRECTIONALITY_SEGMENT_SEPARATOR`.
    pub const DIRECTIONALITY_SEGMENT_SEPARATOR: i8 = (Self::SEGMENT_SEPARATOR) as i8;
    /// `WHITE_SPACE_NEUTRAL`.
    pub const WHITE_SPACE_NEUTRAL: i32 = 9;
    /// `DIRECTIONALITY_WHITESPACE`.
    pub const DIRECTIONALITY_WHITESPACE: i8 = (Self::WHITE_SPACE_NEUTRAL) as i8;
    /// `OTHER_NEUTRAL`.
    pub const OTHER_NEUTRAL: i32 = 10;
    /// `DIRECTIONALITY_OTHER_NEUTRALS`.
    pub const DIRECTIONALITY_OTHER_NEUTRALS: i8 = (Self::OTHER_NEUTRAL) as i8;
    /// `LEFT_TO_RIGHT_EMBEDDING`.
    pub const LEFT_TO_RIGHT_EMBEDDING: i32 = 11;
    /// `DIRECTIONALITY_LEFT_TO_RIGHT_EMBEDDING`.
    pub const DIRECTIONALITY_LEFT_TO_RIGHT_EMBEDDING: i8 = (Self::LEFT_TO_RIGHT_EMBEDDING) as i8;
    /// `LEFT_TO_RIGHT_OVERRIDE`.
    pub const LEFT_TO_RIGHT_OVERRIDE: i32 = 12;
    /// `DIRECTIONALITY_LEFT_TO_RIGHT_OVERRIDE`.
    pub const DIRECTIONALITY_LEFT_TO_RIGHT_OVERRIDE: i8 = (Self::LEFT_TO_RIGHT_OVERRIDE) as i8;
    /// `RIGHT_TO_LEFT_ARABIC`.
    pub const RIGHT_TO_LEFT_ARABIC: i32 = 13;
    /// `DIRECTIONALITY_RIGHT_TO_LEFT_ARABIC`.
    pub const DIRECTIONALITY_RIGHT_TO_LEFT_ARABIC: i8 = (Self::RIGHT_TO_LEFT_ARABIC) as i8;
    /// `RIGHT_TO_LEFT_EMBEDDING`.
    pub const RIGHT_TO_LEFT_EMBEDDING: i32 = 14;
    /// `DIRECTIONALITY_RIGHT_TO_LEFT_EMBEDDING`.
    pub const DIRECTIONALITY_RIGHT_TO_LEFT_EMBEDDING: i8 = (Self::RIGHT_TO_LEFT_EMBEDDING) as i8;
    /// `RIGHT_TO_LEFT_OVERRIDE`.
    pub const RIGHT_TO_LEFT_OVERRIDE: i32 = 15;
    /// `DIRECTIONALITY_RIGHT_TO_LEFT_OVERRIDE`.
    pub const DIRECTIONALITY_RIGHT_TO_LEFT_OVERRIDE: i8 = (Self::RIGHT_TO_LEFT_OVERRIDE) as i8;
    /// `POP_DIRECTIONAL_FORMAT`.
    pub const POP_DIRECTIONAL_FORMAT: i32 = 16;
    /// `DIRECTIONALITY_POP_DIRECTIONAL_FORMAT`.
    pub const DIRECTIONALITY_POP_DIRECTIONAL_FORMAT: i8 = (Self::POP_DIRECTIONAL_FORMAT) as i8;
    /// `DIR_NON_SPACING_MARK`.
    pub const DIR_NON_SPACING_MARK: i32 = 17;
    /// `DIRECTIONALITY_NONSPACING_MARK`.
    pub const DIRECTIONALITY_NONSPACING_MARK: i8 = (Self::DIR_NON_SPACING_MARK) as i8;
    /// `BOUNDARY_NEUTRAL`.
    pub const BOUNDARY_NEUTRAL: i32 = 18;
    /// `DIRECTIONALITY_BOUNDARY_NEUTRAL`.
    pub const DIRECTIONALITY_BOUNDARY_NEUTRAL: i8 = (Self::BOUNDARY_NEUTRAL) as i8;
    /// `FIRST_STRONG_ISOLATE`.
    pub const FIRST_STRONG_ISOLATE: i8 = 19;
    /// `LEFT_TO_RIGHT_ISOLATE`.
    pub const LEFT_TO_RIGHT_ISOLATE: i8 = 20;
    /// `RIGHT_TO_LEFT_ISOLATE`.
    pub const RIGHT_TO_LEFT_ISOLATE: i8 = 21;
    /// `POP_DIRECTIONAL_ISOLATE`.
    pub const POP_DIRECTIONAL_ISOLATE: i8 = 22;
    /// `CHAR_DIRECTION_COUNT`.
    pub const CHAR_DIRECTION_COUNT: i32 = 23;
}

/// 'Enum' for the CharacterDirection constants.
///
/// Equivalent to the interface `UCharacterEnums.ECharacterDirection`.
pub struct ECharacterDirection;

impl ECharacterDirection {
    /// `DIRECTIONALITY_UNDEFINED`.
    pub const DIRECTIONALITY_UNDEFINED: i8 = -1;
}
