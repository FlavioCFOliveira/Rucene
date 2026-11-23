// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UProperty`.

/// Selection constants for Unicode properties.
///
/// Equivalent to the interface `com.ibm.icu.lang.UProperty`.
pub struct UProperty;

impl UProperty {
    /// `UNDEFINED`.
    pub const UNDEFINED: i32 = -1;
    /// `ALPHABETIC`.
    pub const ALPHABETIC: i32 = 0;
    /// `BINARY_START`.
    pub const BINARY_START: i32 = Self::ALPHABETIC;
    /// `ASCII_HEX_DIGIT`.
    pub const ASCII_HEX_DIGIT: i32 = 1;
    /// `BIDI_CONTROL`.
    pub const BIDI_CONTROL: i32 = 2;
    /// `BIDI_MIRRORED`.
    pub const BIDI_MIRRORED: i32 = 3;
    /// `DASH`.
    pub const DASH: i32 = 4;
    /// `DEFAULT_IGNORABLE_CODE_POINT`.
    pub const DEFAULT_IGNORABLE_CODE_POINT: i32 = 5;
    /// `DEPRECATED`.
    pub const DEPRECATED: i32 = 6;
    /// `DIACRITIC`.
    pub const DIACRITIC: i32 = 7;
    /// `EXTENDER`.
    pub const EXTENDER: i32 = 8;
    /// `FULL_COMPOSITION_EXCLUSION`.
    pub const FULL_COMPOSITION_EXCLUSION: i32 = 9;
    /// `GRAPHEME_BASE`.
    pub const GRAPHEME_BASE: i32 = 10;
    /// `GRAPHEME_EXTEND`.
    pub const GRAPHEME_EXTEND: i32 = 11;
    /// `GRAPHEME_LINK`.
    pub const GRAPHEME_LINK: i32 = 12;
    /// `HEX_DIGIT`.
    pub const HEX_DIGIT: i32 = 13;
    /// `HYPHEN`.
    pub const HYPHEN: i32 = 14;
    /// `ID_CONTINUE`.
    pub const ID_CONTINUE: i32 = 15;
    /// `ID_START`.
    pub const ID_START: i32 = 16;
    /// `IDEOGRAPHIC`.
    pub const IDEOGRAPHIC: i32 = 17;
    /// `IDS_BINARY_OPERATOR`.
    pub const IDS_BINARY_OPERATOR: i32 = 18;
    /// `IDS_TRINARY_OPERATOR`.
    pub const IDS_TRINARY_OPERATOR: i32 = 19;
    /// `JOIN_CONTROL`.
    pub const JOIN_CONTROL: i32 = 20;
    /// `LOGICAL_ORDER_EXCEPTION`.
    pub const LOGICAL_ORDER_EXCEPTION: i32 = 21;
    /// `LOWERCASE`.
    pub const LOWERCASE: i32 = 22;
    /// `MATH`.
    pub const MATH: i32 = 23;
    /// `NONCHARACTER_CODE_POINT`.
    pub const NONCHARACTER_CODE_POINT: i32 = 24;
    /// `QUOTATION_MARK`.
    pub const QUOTATION_MARK: i32 = 25;
    /// `RADICAL`.
    pub const RADICAL: i32 = 26;
    /// `SOFT_DOTTED`.
    pub const SOFT_DOTTED: i32 = 27;
    /// `TERMINAL_PUNCTUATION`.
    pub const TERMINAL_PUNCTUATION: i32 = 28;
    /// `UNIFIED_IDEOGRAPH`.
    pub const UNIFIED_IDEOGRAPH: i32 = 29;
    /// `UPPERCASE`.
    pub const UPPERCASE: i32 = 30;
    /// `WHITE_SPACE`.
    pub const WHITE_SPACE: i32 = 31;
    /// `XID_CONTINUE`.
    pub const XID_CONTINUE: i32 = 32;
    /// `XID_START`.
    pub const XID_START: i32 = 33;
    /// `CASE_SENSITIVE`.
    pub const CASE_SENSITIVE: i32 = 34;
    /// `S_TERM`.
    pub const S_TERM: i32 = 35;
    /// `VARIATION_SELECTOR`.
    pub const VARIATION_SELECTOR: i32 = 36;
    /// `NFD_INERT`.
    pub const NFD_INERT: i32 = 37;
    /// `NFKD_INERT`.
    pub const NFKD_INERT: i32 = 38;
    /// `NFC_INERT`.
    pub const NFC_INERT: i32 = 39;
    /// `NFKC_INERT`.
    pub const NFKC_INERT: i32 = 40;
    /// `SEGMENT_STARTER`.
    pub const SEGMENT_STARTER: i32 = 41;
    /// `PATTERN_SYNTAX`.
    pub const PATTERN_SYNTAX: i32 = 42;
    /// `PATTERN_WHITE_SPACE`.
    pub const PATTERN_WHITE_SPACE: i32 = 43;
    /// `POSIX_ALNUM`.
    pub const POSIX_ALNUM: i32 = 44;
    /// `POSIX_BLANK`.
    pub const POSIX_BLANK: i32 = 45;
    /// `POSIX_GRAPH`.
    pub const POSIX_GRAPH: i32 = 46;
    /// `POSIX_PRINT`.
    pub const POSIX_PRINT: i32 = 47;
    /// `POSIX_XDIGIT`.
    pub const POSIX_XDIGIT: i32 = 48;
    /// `CASED`.
    pub const CASED: i32 = 49;
    /// `CASE_IGNORABLE`.
    pub const CASE_IGNORABLE: i32 = 50;
    /// `CHANGES_WHEN_LOWERCASED`.
    pub const CHANGES_WHEN_LOWERCASED: i32 = 51;
    /// `CHANGES_WHEN_UPPERCASED`.
    pub const CHANGES_WHEN_UPPERCASED: i32 = 52;
    /// `CHANGES_WHEN_TITLECASED`.
    pub const CHANGES_WHEN_TITLECASED: i32 = 53;
    /// `CHANGES_WHEN_CASEFOLDED`.
    pub const CHANGES_WHEN_CASEFOLDED: i32 = 54;
    /// `CHANGES_WHEN_CASEMAPPED`.
    pub const CHANGES_WHEN_CASEMAPPED: i32 = 55;
    /// `CHANGES_WHEN_NFKC_CASEFOLDED`.
    pub const CHANGES_WHEN_NFKC_CASEFOLDED: i32 = 56;
    /// `EMOJI`.
    pub const EMOJI: i32 = 57;
    /// `EMOJI_PRESENTATION`.
    pub const EMOJI_PRESENTATION: i32 = 58;
    /// `EMOJI_MODIFIER`.
    pub const EMOJI_MODIFIER: i32 = 59;
    /// `EMOJI_MODIFIER_BASE`.
    pub const EMOJI_MODIFIER_BASE: i32 = 60;
    /// `EMOJI_COMPONENT`.
    pub const EMOJI_COMPONENT: i32 = 61;
    /// `REGIONAL_INDICATOR`.
    pub const REGIONAL_INDICATOR: i32 = 62;
    /// `PREPENDED_CONCATENATION_MARK`.
    pub const PREPENDED_CONCATENATION_MARK: i32 = 63;
    /// `EXTENDED_PICTOGRAPHIC`.
    pub const EXTENDED_PICTOGRAPHIC: i32 = 64;
    /// `BASIC_EMOJI`.
    pub const BASIC_EMOJI: i32 = 65;
    /// `EMOJI_KEYCAP_SEQUENCE`.
    pub const EMOJI_KEYCAP_SEQUENCE: i32 = 66;
    /// `RGI_EMOJI_MODIFIER_SEQUENCE`.
    pub const RGI_EMOJI_MODIFIER_SEQUENCE: i32 = 67;
    /// `RGI_EMOJI_FLAG_SEQUENCE`.
    pub const RGI_EMOJI_FLAG_SEQUENCE: i32 = 68;
    /// `RGI_EMOJI_TAG_SEQUENCE`.
    pub const RGI_EMOJI_TAG_SEQUENCE: i32 = 69;
    /// `RGI_EMOJI_ZWJ_SEQUENCE`.
    pub const RGI_EMOJI_ZWJ_SEQUENCE: i32 = 70;
    /// `RGI_EMOJI`.
    pub const RGI_EMOJI: i32 = 71;
    /// `IDS_UNARY_OPERATOR`.
    pub const IDS_UNARY_OPERATOR: i32 = 72;
    /// `ID_COMPAT_MATH_START`.
    pub const ID_COMPAT_MATH_START: i32 = 73;
    /// `ID_COMPAT_MATH_CONTINUE`.
    pub const ID_COMPAT_MATH_CONTINUE: i32 = 74;
    /// `MODIFIER_COMBINING_MARK`.
    pub const MODIFIER_COMBINING_MARK: i32 = 75;
    /// `BINARY_LIMIT`.
    pub const BINARY_LIMIT: i32 = 76;
    /// `BIDI_CLASS`.
    pub const BIDI_CLASS: i32 = 0x1000;
    /// `INT_START`.
    pub const INT_START: i32 = Self::BIDI_CLASS;
    /// `BLOCK`.
    pub const BLOCK: i32 = 0x1001;
    /// `CANONICAL_COMBINING_CLASS`.
    pub const CANONICAL_COMBINING_CLASS: i32 = 0x1002;
    /// `DECOMPOSITION_TYPE`.
    pub const DECOMPOSITION_TYPE: i32 = 0x1003;
    /// `EAST_ASIAN_WIDTH`.
    pub const EAST_ASIAN_WIDTH: i32 = 0x1004;
    /// `GENERAL_CATEGORY`.
    pub const GENERAL_CATEGORY: i32 = 0x1005;
    /// `JOINING_GROUP`.
    pub const JOINING_GROUP: i32 = 0x1006;
    /// `JOINING_TYPE`.
    pub const JOINING_TYPE: i32 = 0x1007;
    /// `LINE_BREAK`.
    pub const LINE_BREAK: i32 = 0x1008;
    /// `NUMERIC_TYPE`.
    pub const NUMERIC_TYPE: i32 = 0x1009;
    /// `SCRIPT`.
    pub const SCRIPT: i32 = 0x100A;
    /// `HANGUL_SYLLABLE_TYPE`.
    pub const HANGUL_SYLLABLE_TYPE: i32 = 0x100B;
    /// `NFD_QUICK_CHECK`.
    pub const NFD_QUICK_CHECK: i32 = 0x100C;
    /// `NFKD_QUICK_CHECK`.
    pub const NFKD_QUICK_CHECK: i32 = 0x100D;
    /// `NFC_QUICK_CHECK`.
    pub const NFC_QUICK_CHECK: i32 = 0x100E;
    /// `NFKC_QUICK_CHECK`.
    pub const NFKC_QUICK_CHECK: i32 = 0x100F;
    /// `LEAD_CANONICAL_COMBINING_CLASS`.
    pub const LEAD_CANONICAL_COMBINING_CLASS: i32 = 0x1010;
    /// `TRAIL_CANONICAL_COMBINING_CLASS`.
    pub const TRAIL_CANONICAL_COMBINING_CLASS: i32 = 0x1011;
    /// `GRAPHEME_CLUSTER_BREAK`.
    pub const GRAPHEME_CLUSTER_BREAK: i32 = 0x1012;
    /// `SENTENCE_BREAK`.
    pub const SENTENCE_BREAK: i32 = 0x1013;
    /// `WORD_BREAK`.
    pub const WORD_BREAK: i32 = 0x1014;
    /// `BIDI_PAIRED_BRACKET_TYPE`.
    pub const BIDI_PAIRED_BRACKET_TYPE: i32 = 0x1015;
    /// `INDIC_POSITIONAL_CATEGORY`.
    pub const INDIC_POSITIONAL_CATEGORY: i32 = 0x1016;
    /// `INDIC_SYLLABIC_CATEGORY`.
    pub const INDIC_SYLLABIC_CATEGORY: i32 = 0x1017;
    /// `VERTICAL_ORIENTATION`.
    pub const VERTICAL_ORIENTATION: i32 = 0x1018;
    /// `IDENTIFIER_STATUS`.
    pub const IDENTIFIER_STATUS: i32 = 0x1019;
    /// `INDIC_CONJUNCT_BREAK`.
    pub const INDIC_CONJUNCT_BREAK: i32 = 0x101A;
    /// `INT_LIMIT`.
    pub const INT_LIMIT: i32 = 0x101B;
    /// `GENERAL_CATEGORY_MASK`.
    pub const GENERAL_CATEGORY_MASK: i32 = 0x2000;
    /// `MASK_START`.
    pub const MASK_START: i32 = Self::GENERAL_CATEGORY_MASK;
    /// `MASK_LIMIT`.
    pub const MASK_LIMIT: i32 = 0x2001;
    /// `NUMERIC_VALUE`.
    pub const NUMERIC_VALUE: i32 = 0x3000;
    /// `DOUBLE_START`.
    pub const DOUBLE_START: i32 = Self::NUMERIC_VALUE;
    /// `DOUBLE_LIMIT`.
    pub const DOUBLE_LIMIT: i32 = 0x3001;
    /// `AGE`.
    pub const AGE: i32 = 0x4000;
    /// `STRING_START`.
    pub const STRING_START: i32 = Self::AGE;
    /// `BIDI_MIRRORING_GLYPH`.
    pub const BIDI_MIRRORING_GLYPH: i32 = 0x4001;
    /// `CASE_FOLDING`.
    pub const CASE_FOLDING: i32 = 0x4002;
    /// `ISO_COMMENT`.
    pub const ISO_COMMENT: i32 = 0x4003;
    /// `LOWERCASE_MAPPING`.
    pub const LOWERCASE_MAPPING: i32 = 0x4004;
    /// `NAME`.
    pub const NAME: i32 = 0x4005;
    /// `SIMPLE_CASE_FOLDING`.
    pub const SIMPLE_CASE_FOLDING: i32 = 0x4006;
    /// `SIMPLE_LOWERCASE_MAPPING`.
    pub const SIMPLE_LOWERCASE_MAPPING: i32 = 0x4007;
    /// `SIMPLE_TITLECASE_MAPPING`.
    pub const SIMPLE_TITLECASE_MAPPING: i32 = 0x4008;
    /// `SIMPLE_UPPERCASE_MAPPING`.
    pub const SIMPLE_UPPERCASE_MAPPING: i32 = 0x4009;
    /// `TITLECASE_MAPPING`.
    pub const TITLECASE_MAPPING: i32 = 0x400A;
    /// `UNICODE_1_NAME`.
    pub const UNICODE_1_NAME: i32 = 0x400B;
    /// `UPPERCASE_MAPPING`.
    pub const UPPERCASE_MAPPING: i32 = 0x400C;
    /// `BIDI_PAIRED_BRACKET`.
    pub const BIDI_PAIRED_BRACKET: i32 = 0x400D;
    /// `STRING_LIMIT`.
    pub const STRING_LIMIT: i32 = 0x400E;
    /// `SCRIPT_EXTENSIONS`.
    pub const SCRIPT_EXTENSIONS: i32 = 0x7000;
    /// `OTHER_PROPERTY_START`.
    pub const OTHER_PROPERTY_START: i32 = Self::SCRIPT_EXTENSIONS;
    /// `IDENTIFIER_TYPE`.
    pub const IDENTIFIER_TYPE: i32 = 0x7001;
    /// `OTHER_PROPERTY_LIMIT`.
    pub const OTHER_PROPERTY_LIMIT: i32 = 0x7002;
    /// `SHORT`.
    pub const SHORT: i32 = 0;
    /// `LONG`.
    pub const LONG: i32 = 1;
    /// `COUNT`.
    pub const COUNT: i32 = 2;
}

/// Selector constants for property names and property value names.
///
/// Equivalent to the interface `UProperty.NameChoice`.
pub struct NameChoice;

impl NameChoice {
    /// `SHORT`.
    pub const SHORT: i32 = 0;
    /// `LONG`.
    pub const LONG: i32 = 1;
    /// `COUNT` (deprecated).
    pub const COUNT: i32 = 2;
}
