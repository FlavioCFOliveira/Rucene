// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UCharacter`.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{index_out_of_bounds, string_index_out_of_bounds};
use crate::lang::u_character_enums::ECharacterCategory;
use crate::lang::u_character_name_iterator::UCharacterNameIterator;
use crate::lang::u_property::{NameChoice, UProperty};
use crate::r#impl::case_map_impl::CaseMapImpl;
use crate::r#impl::emoji_props::EmojiProps;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::trie2::Trie2Iterator;
use crate::r#impl::u_bidi_props::UBiDiProps;
use crate::r#impl::u_case_props::UCaseProps;
use crate::r#impl::u_character_name::UCharacterName;
use crate::r#impl::u_character_name_choice::UCharacterNameChoice;
use crate::r#impl::u_character_property::{self as ucp, UCharacterProperty};
use crate::r#impl::u_character_utility::UCharacterUtility;
use crate::r#impl::u_property_aliases::UPropertyAliases;
use crate::text::break_iterator::BreakIterator;
use crate::text::normalizer2;
use crate::util::range_value_iterator::{Element as RangeElement, RangeValueIterator};
use crate::util::u_locale::ULocale;
use crate::util::value_iterator::{Element as ValueElement, ValueIterator};
use crate::util::version_info::VersionInfo;

/// A Unicode block: a range of code points with a name and an id.
///
/// Equivalent to `UCharacter.UnicodeBlock`, which extends
/// `java.lang.Character.Subset` (whose `toString()` is the name and whose
/// `equals`/`hashCode` are identity; each block is one constant here, so
/// value equality of `(name, id)` is the same).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnicodeBlock {
    name: &'static str,
    id: i32,
}

impl fmt::Display for UnicodeBlock {
    /// `toString()`: the name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// `mref`: the name map `forName` builds once (a soft reference this crate
/// never clears).
static BLOCK_NAME_MAP: Mutex<Option<Arc<HashMap<String, UnicodeBlock>>>> = Mutex::new(None);

impl UnicodeBlock {
    /// `getInstance(int id)`: the block with the id, `INVALID_CODE` when
    /// none.
    pub fn get_instance(id: i32) -> UnicodeBlock {
        if id >= 0 && (id as usize) < BLOCKS.len() {
            return BLOCKS[id as usize];
        }
        Self::INVALID_CODE
    }

    /// `of(int ch)`: the block of a code point.
    pub fn of(ch: i32) -> UnicodeBlock {
        if ch > UCharacter::MAX_VALUE {
            return Self::INVALID_CODE;
        }
        Self::get_instance(
            UCharacterProperty::instance().get_int_property_value(ch, UProperty::BLOCK),
        )
    }

    /// `forName(String blockName)`: the block whose name (ignoring case,
    /// spaces, underscores and hyphens) matches.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when no block matches; the errors of the
    /// property names data.
    pub fn for_name(block_name: &str) -> Result<UnicodeBlock> {
        let m = {
            let mut g = BLOCK_NAME_MAP.lock().unwrap_or_else(|e| e.into_inner());
            match &*g {
                Some(m) => m.clone(),
                None => {
                    let mut m = HashMap::with_capacity(BLOCKS.len());
                    for b in BLOCKS.iter() {
                        let name = UCharacter::get_property_value_name(
                            UProperty::BLOCK,
                            b.get_id(),
                            NameChoice::LONG,
                        )?
                        .unwrap_or_else(|| "null".to_string());
                        m.insert(Self::trim_block_name(&name), *b);
                    }
                    let m = Arc::new(m);
                    *g = Some(m.clone());
                    m
                }
            }
        };
        match m.get(&Self::trim_block_name(block_name)) {
            Some(b) => Ok(*b),
            None => Err(crate::jdk::illegal_argument_no_message()),
        }
    }

    fn trim_block_name(name: &str) -> String {
        let units: Vec<u16> = name.encode_utf16().collect();
        let upper = rucene_analysis_common::jdk::character::to_upper_case_root(&units);
        let result: Vec<u16> = upper
            .into_iter()
            .filter(|&c| c != b' ' as u16 && c != b'_' as u16 && c != b'-' as u16)
            .collect();
        String::from_utf16_lossy(&result)
    }

    /// `getID()`.
    pub fn get_id(&self) -> i32 {
        self.id
    }

    /// `toString()` (`Character.Subset`): the name.
    pub fn get_name(&self) -> &'static str {
        self.name
    }
}

impl UnicodeBlock {
    /// `INVALID_CODE_ID`.
    pub const INVALID_CODE_ID: i32 = -1;
    /// `BASIC_LATIN_ID`.
    pub const BASIC_LATIN_ID: i32 = 1;
    /// `LATIN_1_SUPPLEMENT_ID`.
    pub const LATIN_1_SUPPLEMENT_ID: i32 = 2;
    /// `LATIN_EXTENDED_A_ID`.
    pub const LATIN_EXTENDED_A_ID: i32 = 3;
    /// `LATIN_EXTENDED_B_ID`.
    pub const LATIN_EXTENDED_B_ID: i32 = 4;
    /// `IPA_EXTENSIONS_ID`.
    pub const IPA_EXTENSIONS_ID: i32 = 5;
    /// `SPACING_MODIFIER_LETTERS_ID`.
    pub const SPACING_MODIFIER_LETTERS_ID: i32 = 6;
    /// `COMBINING_DIACRITICAL_MARKS_ID`.
    pub const COMBINING_DIACRITICAL_MARKS_ID: i32 = 7;
    /// `GREEK_ID`.
    pub const GREEK_ID: i32 = 8;
    /// `CYRILLIC_ID`.
    pub const CYRILLIC_ID: i32 = 9;
    /// `ARMENIAN_ID`.
    pub const ARMENIAN_ID: i32 = 10;
    /// `HEBREW_ID`.
    pub const HEBREW_ID: i32 = 11;
    /// `ARABIC_ID`.
    pub const ARABIC_ID: i32 = 12;
    /// `SYRIAC_ID`.
    pub const SYRIAC_ID: i32 = 13;
    /// `THAANA_ID`.
    pub const THAANA_ID: i32 = 14;
    /// `DEVANAGARI_ID`.
    pub const DEVANAGARI_ID: i32 = 15;
    /// `BENGALI_ID`.
    pub const BENGALI_ID: i32 = 16;
    /// `GURMUKHI_ID`.
    pub const GURMUKHI_ID: i32 = 17;
    /// `GUJARATI_ID`.
    pub const GUJARATI_ID: i32 = 18;
    /// `ORIYA_ID`.
    pub const ORIYA_ID: i32 = 19;
    /// `TAMIL_ID`.
    pub const TAMIL_ID: i32 = 20;
    /// `TELUGU_ID`.
    pub const TELUGU_ID: i32 = 21;
    /// `KANNADA_ID`.
    pub const KANNADA_ID: i32 = 22;
    /// `MALAYALAM_ID`.
    pub const MALAYALAM_ID: i32 = 23;
    /// `SINHALA_ID`.
    pub const SINHALA_ID: i32 = 24;
    /// `THAI_ID`.
    pub const THAI_ID: i32 = 25;
    /// `LAO_ID`.
    pub const LAO_ID: i32 = 26;
    /// `TIBETAN_ID`.
    pub const TIBETAN_ID: i32 = 27;
    /// `MYANMAR_ID`.
    pub const MYANMAR_ID: i32 = 28;
    /// `GEORGIAN_ID`.
    pub const GEORGIAN_ID: i32 = 29;
    /// `HANGUL_JAMO_ID`.
    pub const HANGUL_JAMO_ID: i32 = 30;
    /// `ETHIOPIC_ID`.
    pub const ETHIOPIC_ID: i32 = 31;
    /// `CHEROKEE_ID`.
    pub const CHEROKEE_ID: i32 = 32;
    /// `UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_ID`.
    pub const UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_ID: i32 = 33;
    /// `OGHAM_ID`.
    pub const OGHAM_ID: i32 = 34;
    /// `RUNIC_ID`.
    pub const RUNIC_ID: i32 = 35;
    /// `KHMER_ID`.
    pub const KHMER_ID: i32 = 36;
    /// `MONGOLIAN_ID`.
    pub const MONGOLIAN_ID: i32 = 37;
    /// `LATIN_EXTENDED_ADDITIONAL_ID`.
    pub const LATIN_EXTENDED_ADDITIONAL_ID: i32 = 38;
    /// `GREEK_EXTENDED_ID`.
    pub const GREEK_EXTENDED_ID: i32 = 39;
    /// `GENERAL_PUNCTUATION_ID`.
    pub const GENERAL_PUNCTUATION_ID: i32 = 40;
    /// `SUPERSCRIPTS_AND_SUBSCRIPTS_ID`.
    pub const SUPERSCRIPTS_AND_SUBSCRIPTS_ID: i32 = 41;
    /// `CURRENCY_SYMBOLS_ID`.
    pub const CURRENCY_SYMBOLS_ID: i32 = 42;
    /// `COMBINING_MARKS_FOR_SYMBOLS_ID`.
    pub const COMBINING_MARKS_FOR_SYMBOLS_ID: i32 = 43;
    /// `LETTERLIKE_SYMBOLS_ID`.
    pub const LETTERLIKE_SYMBOLS_ID: i32 = 44;
    /// `NUMBER_FORMS_ID`.
    pub const NUMBER_FORMS_ID: i32 = 45;
    /// `ARROWS_ID`.
    pub const ARROWS_ID: i32 = 46;
    /// `MATHEMATICAL_OPERATORS_ID`.
    pub const MATHEMATICAL_OPERATORS_ID: i32 = 47;
    /// `MISCELLANEOUS_TECHNICAL_ID`.
    pub const MISCELLANEOUS_TECHNICAL_ID: i32 = 48;
    /// `CONTROL_PICTURES_ID`.
    pub const CONTROL_PICTURES_ID: i32 = 49;
    /// `OPTICAL_CHARACTER_RECOGNITION_ID`.
    pub const OPTICAL_CHARACTER_RECOGNITION_ID: i32 = 50;
    /// `ENCLOSED_ALPHANUMERICS_ID`.
    pub const ENCLOSED_ALPHANUMERICS_ID: i32 = 51;
    /// `BOX_DRAWING_ID`.
    pub const BOX_DRAWING_ID: i32 = 52;
    /// `BLOCK_ELEMENTS_ID`.
    pub const BLOCK_ELEMENTS_ID: i32 = 53;
    /// `GEOMETRIC_SHAPES_ID`.
    pub const GEOMETRIC_SHAPES_ID: i32 = 54;
    /// `MISCELLANEOUS_SYMBOLS_ID`.
    pub const MISCELLANEOUS_SYMBOLS_ID: i32 = 55;
    /// `DINGBATS_ID`.
    pub const DINGBATS_ID: i32 = 56;
    /// `BRAILLE_PATTERNS_ID`.
    pub const BRAILLE_PATTERNS_ID: i32 = 57;
    /// `CJK_RADICALS_SUPPLEMENT_ID`.
    pub const CJK_RADICALS_SUPPLEMENT_ID: i32 = 58;
    /// `KANGXI_RADICALS_ID`.
    pub const KANGXI_RADICALS_ID: i32 = 59;
    /// `IDEOGRAPHIC_DESCRIPTION_CHARACTERS_ID`.
    pub const IDEOGRAPHIC_DESCRIPTION_CHARACTERS_ID: i32 = 60;
    /// `CJK_SYMBOLS_AND_PUNCTUATION_ID`.
    pub const CJK_SYMBOLS_AND_PUNCTUATION_ID: i32 = 61;
    /// `HIRAGANA_ID`.
    pub const HIRAGANA_ID: i32 = 62;
    /// `KATAKANA_ID`.
    pub const KATAKANA_ID: i32 = 63;
    /// `BOPOMOFO_ID`.
    pub const BOPOMOFO_ID: i32 = 64;
    /// `HANGUL_COMPATIBILITY_JAMO_ID`.
    pub const HANGUL_COMPATIBILITY_JAMO_ID: i32 = 65;
    /// `KANBUN_ID`.
    pub const KANBUN_ID: i32 = 66;
    /// `BOPOMOFO_EXTENDED_ID`.
    pub const BOPOMOFO_EXTENDED_ID: i32 = 67;
    /// `ENCLOSED_CJK_LETTERS_AND_MONTHS_ID`.
    pub const ENCLOSED_CJK_LETTERS_AND_MONTHS_ID: i32 = 68;
    /// `CJK_COMPATIBILITY_ID`.
    pub const CJK_COMPATIBILITY_ID: i32 = 69;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A_ID: i32 = 70;
    /// `CJK_UNIFIED_IDEOGRAPHS_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_ID: i32 = 71;
    /// `YI_SYLLABLES_ID`.
    pub const YI_SYLLABLES_ID: i32 = 72;
    /// `YI_RADICALS_ID`.
    pub const YI_RADICALS_ID: i32 = 73;
    /// `HANGUL_SYLLABLES_ID`.
    pub const HANGUL_SYLLABLES_ID: i32 = 74;
    /// `HIGH_SURROGATES_ID`.
    pub const HIGH_SURROGATES_ID: i32 = 75;
    /// `HIGH_PRIVATE_USE_SURROGATES_ID`.
    pub const HIGH_PRIVATE_USE_SURROGATES_ID: i32 = 76;
    /// `LOW_SURROGATES_ID`.
    pub const LOW_SURROGATES_ID: i32 = 77;
    /// `PRIVATE_USE_AREA_ID`.
    pub const PRIVATE_USE_AREA_ID: i32 = 78;
    /// `PRIVATE_USE_ID`.
    pub const PRIVATE_USE_ID: i32 = Self::PRIVATE_USE_AREA_ID;
    /// `CJK_COMPATIBILITY_IDEOGRAPHS_ID`.
    pub const CJK_COMPATIBILITY_IDEOGRAPHS_ID: i32 = 79;
    /// `ALPHABETIC_PRESENTATION_FORMS_ID`.
    pub const ALPHABETIC_PRESENTATION_FORMS_ID: i32 = 80;
    /// `ARABIC_PRESENTATION_FORMS_A_ID`.
    pub const ARABIC_PRESENTATION_FORMS_A_ID: i32 = 81;
    /// `COMBINING_HALF_MARKS_ID`.
    pub const COMBINING_HALF_MARKS_ID: i32 = 82;
    /// `CJK_COMPATIBILITY_FORMS_ID`.
    pub const CJK_COMPATIBILITY_FORMS_ID: i32 = 83;
    /// `SMALL_FORM_VARIANTS_ID`.
    pub const SMALL_FORM_VARIANTS_ID: i32 = 84;
    /// `ARABIC_PRESENTATION_FORMS_B_ID`.
    pub const ARABIC_PRESENTATION_FORMS_B_ID: i32 = 85;
    /// `SPECIALS_ID`.
    pub const SPECIALS_ID: i32 = 86;
    /// `HALFWIDTH_AND_FULLWIDTH_FORMS_ID`.
    pub const HALFWIDTH_AND_FULLWIDTH_FORMS_ID: i32 = 87;
    /// `OLD_ITALIC_ID`.
    pub const OLD_ITALIC_ID: i32 = 88;
    /// `GOTHIC_ID`.
    pub const GOTHIC_ID: i32 = 89;
    /// `DESERET_ID`.
    pub const DESERET_ID: i32 = 90;
    /// `BYZANTINE_MUSICAL_SYMBOLS_ID`.
    pub const BYZANTINE_MUSICAL_SYMBOLS_ID: i32 = 91;
    /// `MUSICAL_SYMBOLS_ID`.
    pub const MUSICAL_SYMBOLS_ID: i32 = 92;
    /// `MATHEMATICAL_ALPHANUMERIC_SYMBOLS_ID`.
    pub const MATHEMATICAL_ALPHANUMERIC_SYMBOLS_ID: i32 = 93;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_B_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_B_ID: i32 = 94;
    /// `CJK_COMPATIBILITY_IDEOGRAPHS_SUPPLEMENT_ID`.
    pub const CJK_COMPATIBILITY_IDEOGRAPHS_SUPPLEMENT_ID: i32 = 95;
    /// `TAGS_ID`.
    pub const TAGS_ID: i32 = 96;
    /// `CYRILLIC_SUPPLEMENTARY_ID`.
    pub const CYRILLIC_SUPPLEMENTARY_ID: i32 = 97;
    /// `CYRILLIC_SUPPLEMENT_ID`.
    pub const CYRILLIC_SUPPLEMENT_ID: i32 = 97;
    /// `TAGALOG_ID`.
    pub const TAGALOG_ID: i32 = 98;
    /// `HANUNOO_ID`.
    pub const HANUNOO_ID: i32 = 99;
    /// `BUHID_ID`.
    pub const BUHID_ID: i32 = 100;
    /// `TAGBANWA_ID`.
    pub const TAGBANWA_ID: i32 = 101;
    /// `MISCELLANEOUS_MATHEMATICAL_SYMBOLS_A_ID`.
    pub const MISCELLANEOUS_MATHEMATICAL_SYMBOLS_A_ID: i32 = 102;
    /// `SUPPLEMENTAL_ARROWS_A_ID`.
    pub const SUPPLEMENTAL_ARROWS_A_ID: i32 = 103;
    /// `SUPPLEMENTAL_ARROWS_B_ID`.
    pub const SUPPLEMENTAL_ARROWS_B_ID: i32 = 104;
    /// `MISCELLANEOUS_MATHEMATICAL_SYMBOLS_B_ID`.
    pub const MISCELLANEOUS_MATHEMATICAL_SYMBOLS_B_ID: i32 = 105;
    /// `SUPPLEMENTAL_MATHEMATICAL_OPERATORS_ID`.
    pub const SUPPLEMENTAL_MATHEMATICAL_OPERATORS_ID: i32 = 106;
    /// `KATAKANA_PHONETIC_EXTENSIONS_ID`.
    pub const KATAKANA_PHONETIC_EXTENSIONS_ID: i32 = 107;
    /// `VARIATION_SELECTORS_ID`.
    pub const VARIATION_SELECTORS_ID: i32 = 108;
    /// `SUPPLEMENTARY_PRIVATE_USE_AREA_A_ID`.
    pub const SUPPLEMENTARY_PRIVATE_USE_AREA_A_ID: i32 = 109;
    /// `SUPPLEMENTARY_PRIVATE_USE_AREA_B_ID`.
    pub const SUPPLEMENTARY_PRIVATE_USE_AREA_B_ID: i32 = 110;
    /// `LIMBU_ID`.
    pub const LIMBU_ID: i32 = 111;
    /// `TAI_LE_ID`.
    pub const TAI_LE_ID: i32 = 112;
    /// `KHMER_SYMBOLS_ID`.
    pub const KHMER_SYMBOLS_ID: i32 = 113;
    /// `PHONETIC_EXTENSIONS_ID`.
    pub const PHONETIC_EXTENSIONS_ID: i32 = 114;
    /// `MISCELLANEOUS_SYMBOLS_AND_ARROWS_ID`.
    pub const MISCELLANEOUS_SYMBOLS_AND_ARROWS_ID: i32 = 115;
    /// `YIJING_HEXAGRAM_SYMBOLS_ID`.
    pub const YIJING_HEXAGRAM_SYMBOLS_ID: i32 = 116;
    /// `LINEAR_B_SYLLABARY_ID`.
    pub const LINEAR_B_SYLLABARY_ID: i32 = 117;
    /// `LINEAR_B_IDEOGRAMS_ID`.
    pub const LINEAR_B_IDEOGRAMS_ID: i32 = 118;
    /// `AEGEAN_NUMBERS_ID`.
    pub const AEGEAN_NUMBERS_ID: i32 = 119;
    /// `UGARITIC_ID`.
    pub const UGARITIC_ID: i32 = 120;
    /// `SHAVIAN_ID`.
    pub const SHAVIAN_ID: i32 = 121;
    /// `OSMANYA_ID`.
    pub const OSMANYA_ID: i32 = 122;
    /// `CYPRIOT_SYLLABARY_ID`.
    pub const CYPRIOT_SYLLABARY_ID: i32 = 123;
    /// `TAI_XUAN_JING_SYMBOLS_ID`.
    pub const TAI_XUAN_JING_SYMBOLS_ID: i32 = 124;
    /// `VARIATION_SELECTORS_SUPPLEMENT_ID`.
    pub const VARIATION_SELECTORS_SUPPLEMENT_ID: i32 = 125;
    /// `ANCIENT_GREEK_MUSICAL_NOTATION_ID`.
    pub const ANCIENT_GREEK_MUSICAL_NOTATION_ID: i32 = 126;
    /// `ANCIENT_GREEK_NUMBERS_ID`.
    pub const ANCIENT_GREEK_NUMBERS_ID: i32 = 127;
    /// `ARABIC_SUPPLEMENT_ID`.
    pub const ARABIC_SUPPLEMENT_ID: i32 = 128;
    /// `BUGINESE_ID`.
    pub const BUGINESE_ID: i32 = 129;
    /// `CJK_STROKES_ID`.
    pub const CJK_STROKES_ID: i32 = 130;
    /// `COMBINING_DIACRITICAL_MARKS_SUPPLEMENT_ID`.
    pub const COMBINING_DIACRITICAL_MARKS_SUPPLEMENT_ID: i32 = 131;
    /// `COPTIC_ID`.
    pub const COPTIC_ID: i32 = 132;
    /// `ETHIOPIC_EXTENDED_ID`.
    pub const ETHIOPIC_EXTENDED_ID: i32 = 133;
    /// `ETHIOPIC_SUPPLEMENT_ID`.
    pub const ETHIOPIC_SUPPLEMENT_ID: i32 = 134;
    /// `GEORGIAN_SUPPLEMENT_ID`.
    pub const GEORGIAN_SUPPLEMENT_ID: i32 = 135;
    /// `GLAGOLITIC_ID`.
    pub const GLAGOLITIC_ID: i32 = 136;
    /// `KHAROSHTHI_ID`.
    pub const KHAROSHTHI_ID: i32 = 137;
    /// `MODIFIER_TONE_LETTERS_ID`.
    pub const MODIFIER_TONE_LETTERS_ID: i32 = 138;
    /// `NEW_TAI_LUE_ID`.
    pub const NEW_TAI_LUE_ID: i32 = 139;
    /// `OLD_PERSIAN_ID`.
    pub const OLD_PERSIAN_ID: i32 = 140;
    /// `PHONETIC_EXTENSIONS_SUPPLEMENT_ID`.
    pub const PHONETIC_EXTENSIONS_SUPPLEMENT_ID: i32 = 141;
    /// `SUPPLEMENTAL_PUNCTUATION_ID`.
    pub const SUPPLEMENTAL_PUNCTUATION_ID: i32 = 142;
    /// `SYLOTI_NAGRI_ID`.
    pub const SYLOTI_NAGRI_ID: i32 = 143;
    /// `TIFINAGH_ID`.
    pub const TIFINAGH_ID: i32 = 144;
    /// `VERTICAL_FORMS_ID`.
    pub const VERTICAL_FORMS_ID: i32 = 145;
    /// `NKO_ID`.
    pub const NKO_ID: i32 = 146;
    /// `BALINESE_ID`.
    pub const BALINESE_ID: i32 = 147;
    /// `LATIN_EXTENDED_C_ID`.
    pub const LATIN_EXTENDED_C_ID: i32 = 148;
    /// `LATIN_EXTENDED_D_ID`.
    pub const LATIN_EXTENDED_D_ID: i32 = 149;
    /// `PHAGS_PA_ID`.
    pub const PHAGS_PA_ID: i32 = 150;
    /// `PHOENICIAN_ID`.
    pub const PHOENICIAN_ID: i32 = 151;
    /// `CUNEIFORM_ID`.
    pub const CUNEIFORM_ID: i32 = 152;
    /// `CUNEIFORM_NUMBERS_AND_PUNCTUATION_ID`.
    pub const CUNEIFORM_NUMBERS_AND_PUNCTUATION_ID: i32 = 153;
    /// `COUNTING_ROD_NUMERALS_ID`.
    pub const COUNTING_ROD_NUMERALS_ID: i32 = 154;
    /// `SUNDANESE_ID`.
    pub const SUNDANESE_ID: i32 = 155;
    /// `LEPCHA_ID`.
    pub const LEPCHA_ID: i32 = 156;
    /// `OL_CHIKI_ID`.
    pub const OL_CHIKI_ID: i32 = 157;
    /// `CYRILLIC_EXTENDED_A_ID`.
    pub const CYRILLIC_EXTENDED_A_ID: i32 = 158;
    /// `VAI_ID`.
    pub const VAI_ID: i32 = 159;
    /// `CYRILLIC_EXTENDED_B_ID`.
    pub const CYRILLIC_EXTENDED_B_ID: i32 = 160;
    /// `SAURASHTRA_ID`.
    pub const SAURASHTRA_ID: i32 = 161;
    /// `KAYAH_LI_ID`.
    pub const KAYAH_LI_ID: i32 = 162;
    /// `REJANG_ID`.
    pub const REJANG_ID: i32 = 163;
    /// `CHAM_ID`.
    pub const CHAM_ID: i32 = 164;
    /// `ANCIENT_SYMBOLS_ID`.
    pub const ANCIENT_SYMBOLS_ID: i32 = 165;
    /// `PHAISTOS_DISC_ID`.
    pub const PHAISTOS_DISC_ID: i32 = 166;
    /// `LYCIAN_ID`.
    pub const LYCIAN_ID: i32 = 167;
    /// `CARIAN_ID`.
    pub const CARIAN_ID: i32 = 168;
    /// `LYDIAN_ID`.
    pub const LYDIAN_ID: i32 = 169;
    /// `MAHJONG_TILES_ID`.
    pub const MAHJONG_TILES_ID: i32 = 170;
    /// `DOMINO_TILES_ID`.
    pub const DOMINO_TILES_ID: i32 = 171;
    /// `SAMARITAN_ID`.
    pub const SAMARITAN_ID: i32 = 172;
    /// `UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_ID`.
    pub const UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_ID: i32 = 173;
    /// `TAI_THAM_ID`.
    pub const TAI_THAM_ID: i32 = 174;
    /// `VEDIC_EXTENSIONS_ID`.
    pub const VEDIC_EXTENSIONS_ID: i32 = 175;
    /// `LISU_ID`.
    pub const LISU_ID: i32 = 176;
    /// `BAMUM_ID`.
    pub const BAMUM_ID: i32 = 177;
    /// `COMMON_INDIC_NUMBER_FORMS_ID`.
    pub const COMMON_INDIC_NUMBER_FORMS_ID: i32 = 178;
    /// `DEVANAGARI_EXTENDED_ID`.
    pub const DEVANAGARI_EXTENDED_ID: i32 = 179;
    /// `HANGUL_JAMO_EXTENDED_A_ID`.
    pub const HANGUL_JAMO_EXTENDED_A_ID: i32 = 180;
    /// `JAVANESE_ID`.
    pub const JAVANESE_ID: i32 = 181;
    /// `MYANMAR_EXTENDED_A_ID`.
    pub const MYANMAR_EXTENDED_A_ID: i32 = 182;
    /// `TAI_VIET_ID`.
    pub const TAI_VIET_ID: i32 = 183;
    /// `MEETEI_MAYEK_ID`.
    pub const MEETEI_MAYEK_ID: i32 = 184;
    /// `HANGUL_JAMO_EXTENDED_B_ID`.
    pub const HANGUL_JAMO_EXTENDED_B_ID: i32 = 185;
    /// `IMPERIAL_ARAMAIC_ID`.
    pub const IMPERIAL_ARAMAIC_ID: i32 = 186;
    /// `OLD_SOUTH_ARABIAN_ID`.
    pub const OLD_SOUTH_ARABIAN_ID: i32 = 187;
    /// `AVESTAN_ID`.
    pub const AVESTAN_ID: i32 = 188;
    /// `INSCRIPTIONAL_PARTHIAN_ID`.
    pub const INSCRIPTIONAL_PARTHIAN_ID: i32 = 189;
    /// `INSCRIPTIONAL_PAHLAVI_ID`.
    pub const INSCRIPTIONAL_PAHLAVI_ID: i32 = 190;
    /// `OLD_TURKIC_ID`.
    pub const OLD_TURKIC_ID: i32 = 191;
    /// `RUMI_NUMERAL_SYMBOLS_ID`.
    pub const RUMI_NUMERAL_SYMBOLS_ID: i32 = 192;
    /// `KAITHI_ID`.
    pub const KAITHI_ID: i32 = 193;
    /// `EGYPTIAN_HIEROGLYPHS_ID`.
    pub const EGYPTIAN_HIEROGLYPHS_ID: i32 = 194;
    /// `ENCLOSED_ALPHANUMERIC_SUPPLEMENT_ID`.
    pub const ENCLOSED_ALPHANUMERIC_SUPPLEMENT_ID: i32 = 195;
    /// `ENCLOSED_IDEOGRAPHIC_SUPPLEMENT_ID`.
    pub const ENCLOSED_IDEOGRAPHIC_SUPPLEMENT_ID: i32 = 196;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_C_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_C_ID: i32 = 197;
    /// `MANDAIC_ID`.
    pub const MANDAIC_ID: i32 = 198;
    /// `BATAK_ID`.
    pub const BATAK_ID: i32 = 199;
    /// `ETHIOPIC_EXTENDED_A_ID`.
    pub const ETHIOPIC_EXTENDED_A_ID: i32 = 200;
    /// `BRAHMI_ID`.
    pub const BRAHMI_ID: i32 = 201;
    /// `BAMUM_SUPPLEMENT_ID`.
    pub const BAMUM_SUPPLEMENT_ID: i32 = 202;
    /// `KANA_SUPPLEMENT_ID`.
    pub const KANA_SUPPLEMENT_ID: i32 = 203;
    /// `PLAYING_CARDS_ID`.
    pub const PLAYING_CARDS_ID: i32 = 204;
    /// `MISCELLANEOUS_SYMBOLS_AND_PICTOGRAPHS_ID`.
    pub const MISCELLANEOUS_SYMBOLS_AND_PICTOGRAPHS_ID: i32 = 205;
    /// `EMOTICONS_ID`.
    pub const EMOTICONS_ID: i32 = 206;
    /// `TRANSPORT_AND_MAP_SYMBOLS_ID`.
    pub const TRANSPORT_AND_MAP_SYMBOLS_ID: i32 = 207;
    /// `ALCHEMICAL_SYMBOLS_ID`.
    pub const ALCHEMICAL_SYMBOLS_ID: i32 = 208;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_D_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_D_ID: i32 = 209;
    /// `ARABIC_EXTENDED_A_ID`.
    pub const ARABIC_EXTENDED_A_ID: i32 = 210;
    /// `ARABIC_MATHEMATICAL_ALPHABETIC_SYMBOLS_ID`.
    pub const ARABIC_MATHEMATICAL_ALPHABETIC_SYMBOLS_ID: i32 = 211;
    /// `CHAKMA_ID`.
    pub const CHAKMA_ID: i32 = 212;
    /// `MEETEI_MAYEK_EXTENSIONS_ID`.
    pub const MEETEI_MAYEK_EXTENSIONS_ID: i32 = 213;
    /// `MEROITIC_CURSIVE_ID`.
    pub const MEROITIC_CURSIVE_ID: i32 = 214;
    /// `MEROITIC_HIEROGLYPHS_ID`.
    pub const MEROITIC_HIEROGLYPHS_ID: i32 = 215;
    /// `MIAO_ID`.
    pub const MIAO_ID: i32 = 216;
    /// `SHARADA_ID`.
    pub const SHARADA_ID: i32 = 217;
    /// `SORA_SOMPENG_ID`.
    pub const SORA_SOMPENG_ID: i32 = 218;
    /// `SUNDANESE_SUPPLEMENT_ID`.
    pub const SUNDANESE_SUPPLEMENT_ID: i32 = 219;
    /// `TAKRI_ID`.
    pub const TAKRI_ID: i32 = 220;
    /// `BASSA_VAH_ID`.
    pub const BASSA_VAH_ID: i32 = 221;
    /// `CAUCASIAN_ALBANIAN_ID`.
    pub const CAUCASIAN_ALBANIAN_ID: i32 = 222;
    /// `COPTIC_EPACT_NUMBERS_ID`.
    pub const COPTIC_EPACT_NUMBERS_ID: i32 = 223;
    /// `COMBINING_DIACRITICAL_MARKS_EXTENDED_ID`.
    pub const COMBINING_DIACRITICAL_MARKS_EXTENDED_ID: i32 = 224;
    /// `DUPLOYAN_ID`.
    pub const DUPLOYAN_ID: i32 = 225;
    /// `ELBASAN_ID`.
    pub const ELBASAN_ID: i32 = 226;
    /// `GEOMETRIC_SHAPES_EXTENDED_ID`.
    pub const GEOMETRIC_SHAPES_EXTENDED_ID: i32 = 227;
    /// `GRANTHA_ID`.
    pub const GRANTHA_ID: i32 = 228;
    /// `KHOJKI_ID`.
    pub const KHOJKI_ID: i32 = 229;
    /// `KHUDAWADI_ID`.
    pub const KHUDAWADI_ID: i32 = 230;
    /// `LATIN_EXTENDED_E_ID`.
    pub const LATIN_EXTENDED_E_ID: i32 = 231;
    /// `LINEAR_A_ID`.
    pub const LINEAR_A_ID: i32 = 232;
    /// `MAHAJANI_ID`.
    pub const MAHAJANI_ID: i32 = 233;
    /// `MANICHAEAN_ID`.
    pub const MANICHAEAN_ID: i32 = 234;
    /// `MENDE_KIKAKUI_ID`.
    pub const MENDE_KIKAKUI_ID: i32 = 235;
    /// `MODI_ID`.
    pub const MODI_ID: i32 = 236;
    /// `MRO_ID`.
    pub const MRO_ID: i32 = 237;
    /// `MYANMAR_EXTENDED_B_ID`.
    pub const MYANMAR_EXTENDED_B_ID: i32 = 238;
    /// `NABATAEAN_ID`.
    pub const NABATAEAN_ID: i32 = 239;
    /// `OLD_NORTH_ARABIAN_ID`.
    pub const OLD_NORTH_ARABIAN_ID: i32 = 240;
    /// `OLD_PERMIC_ID`.
    pub const OLD_PERMIC_ID: i32 = 241;
    /// `ORNAMENTAL_DINGBATS_ID`.
    pub const ORNAMENTAL_DINGBATS_ID: i32 = 242;
    /// `PAHAWH_HMONG_ID`.
    pub const PAHAWH_HMONG_ID: i32 = 243;
    /// `PALMYRENE_ID`.
    pub const PALMYRENE_ID: i32 = 244;
    /// `PAU_CIN_HAU_ID`.
    pub const PAU_CIN_HAU_ID: i32 = 245;
    /// `PSALTER_PAHLAVI_ID`.
    pub const PSALTER_PAHLAVI_ID: i32 = 246;
    /// `SHORTHAND_FORMAT_CONTROLS_ID`.
    pub const SHORTHAND_FORMAT_CONTROLS_ID: i32 = 247;
    /// `SIDDHAM_ID`.
    pub const SIDDHAM_ID: i32 = 248;
    /// `SINHALA_ARCHAIC_NUMBERS_ID`.
    pub const SINHALA_ARCHAIC_NUMBERS_ID: i32 = 249;
    /// `SUPPLEMENTAL_ARROWS_C_ID`.
    pub const SUPPLEMENTAL_ARROWS_C_ID: i32 = 250;
    /// `TIRHUTA_ID`.
    pub const TIRHUTA_ID: i32 = 251;
    /// `WARANG_CITI_ID`.
    pub const WARANG_CITI_ID: i32 = 252;
    /// `AHOM_ID`.
    pub const AHOM_ID: i32 = 253;
    /// `ANATOLIAN_HIEROGLYPHS_ID`.
    pub const ANATOLIAN_HIEROGLYPHS_ID: i32 = 254;
    /// `CHEROKEE_SUPPLEMENT_ID`.
    pub const CHEROKEE_SUPPLEMENT_ID: i32 = 255;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_E_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_E_ID: i32 = 256;
    /// `EARLY_DYNASTIC_CUNEIFORM_ID`.
    pub const EARLY_DYNASTIC_CUNEIFORM_ID: i32 = 257;
    /// `HATRAN_ID`.
    pub const HATRAN_ID: i32 = 258;
    /// `MULTANI_ID`.
    pub const MULTANI_ID: i32 = 259;
    /// `OLD_HUNGARIAN_ID`.
    pub const OLD_HUNGARIAN_ID: i32 = 260;
    /// `SUPPLEMENTAL_SYMBOLS_AND_PICTOGRAPHS_ID`.
    pub const SUPPLEMENTAL_SYMBOLS_AND_PICTOGRAPHS_ID: i32 = 261;
    /// `SUTTON_SIGNWRITING_ID`.
    pub const SUTTON_SIGNWRITING_ID: i32 = 262;
    /// `ADLAM_ID`.
    pub const ADLAM_ID: i32 = 263;
    /// `BHAIKSUKI_ID`.
    pub const BHAIKSUKI_ID: i32 = 264;
    /// `CYRILLIC_EXTENDED_C_ID`.
    pub const CYRILLIC_EXTENDED_C_ID: i32 = 265;
    /// `GLAGOLITIC_SUPPLEMENT_ID`.
    pub const GLAGOLITIC_SUPPLEMENT_ID: i32 = 266;
    /// `IDEOGRAPHIC_SYMBOLS_AND_PUNCTUATION_ID`.
    pub const IDEOGRAPHIC_SYMBOLS_AND_PUNCTUATION_ID: i32 = 267;
    /// `MARCHEN_ID`.
    pub const MARCHEN_ID: i32 = 268;
    /// `MONGOLIAN_SUPPLEMENT_ID`.
    pub const MONGOLIAN_SUPPLEMENT_ID: i32 = 269;
    /// `NEWA_ID`.
    pub const NEWA_ID: i32 = 270;
    /// `OSAGE_ID`.
    pub const OSAGE_ID: i32 = 271;
    /// `TANGUT_ID`.
    pub const TANGUT_ID: i32 = 272;
    /// `TANGUT_COMPONENTS_ID`.
    pub const TANGUT_COMPONENTS_ID: i32 = 273;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_F_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_F_ID: i32 = 274;
    /// `KANA_EXTENDED_A_ID`.
    pub const KANA_EXTENDED_A_ID: i32 = 275;
    /// `MASARAM_GONDI_ID`.
    pub const MASARAM_GONDI_ID: i32 = 276;
    /// `NUSHU_ID`.
    pub const NUSHU_ID: i32 = 277;
    /// `SOYOMBO_ID`.
    pub const SOYOMBO_ID: i32 = 278;
    /// `SYRIAC_SUPPLEMENT_ID`.
    pub const SYRIAC_SUPPLEMENT_ID: i32 = 279;
    /// `ZANABAZAR_SQUARE_ID`.
    pub const ZANABAZAR_SQUARE_ID: i32 = 280;
    /// `CHESS_SYMBOLS_ID`.
    pub const CHESS_SYMBOLS_ID: i32 = 281;
    /// `DOGRA_ID`.
    pub const DOGRA_ID: i32 = 282;
    /// `GEORGIAN_EXTENDED_ID`.
    pub const GEORGIAN_EXTENDED_ID: i32 = 283;
    /// `GUNJALA_GONDI_ID`.
    pub const GUNJALA_GONDI_ID: i32 = 284;
    /// `HANIFI_ROHINGYA_ID`.
    pub const HANIFI_ROHINGYA_ID: i32 = 285;
    /// `INDIC_SIYAQ_NUMBERS_ID`.
    pub const INDIC_SIYAQ_NUMBERS_ID: i32 = 286;
    /// `MAKASAR_ID`.
    pub const MAKASAR_ID: i32 = 287;
    /// `MAYAN_NUMERALS_ID`.
    pub const MAYAN_NUMERALS_ID: i32 = 288;
    /// `MEDEFAIDRIN_ID`.
    pub const MEDEFAIDRIN_ID: i32 = 289;
    /// `OLD_SOGDIAN_ID`.
    pub const OLD_SOGDIAN_ID: i32 = 290;
    /// `SOGDIAN_ID`.
    pub const SOGDIAN_ID: i32 = 291;
    /// `EGYPTIAN_HIEROGLYPH_FORMAT_CONTROLS_ID`.
    pub const EGYPTIAN_HIEROGLYPH_FORMAT_CONTROLS_ID: i32 = 292;
    /// `ELYMAIC_ID`.
    pub const ELYMAIC_ID: i32 = 293;
    /// `NANDINAGARI_ID`.
    pub const NANDINAGARI_ID: i32 = 294;
    /// `NYIAKENG_PUACHUE_HMONG_ID`.
    pub const NYIAKENG_PUACHUE_HMONG_ID: i32 = 295;
    /// `OTTOMAN_SIYAQ_NUMBERS_ID`.
    pub const OTTOMAN_SIYAQ_NUMBERS_ID: i32 = 296;
    /// `SMALL_KANA_EXTENSION_ID`.
    pub const SMALL_KANA_EXTENSION_ID: i32 = 297;
    /// `SYMBOLS_AND_PICTOGRAPHS_EXTENDED_A_ID`.
    pub const SYMBOLS_AND_PICTOGRAPHS_EXTENDED_A_ID: i32 = 298;
    /// `TAMIL_SUPPLEMENT_ID`.
    pub const TAMIL_SUPPLEMENT_ID: i32 = 299;
    /// `WANCHO_ID`.
    pub const WANCHO_ID: i32 = 300;
    /// `CHORASMIAN_ID`.
    pub const CHORASMIAN_ID: i32 = 301;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_G_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_G_ID: i32 = 302;
    /// `DIVES_AKURU_ID`.
    pub const DIVES_AKURU_ID: i32 = 303;
    /// `KHITAN_SMALL_SCRIPT_ID`.
    pub const KHITAN_SMALL_SCRIPT_ID: i32 = 304;
    /// `LISU_SUPPLEMENT_ID`.
    pub const LISU_SUPPLEMENT_ID: i32 = 305;
    /// `SYMBOLS_FOR_LEGACY_COMPUTING_ID`.
    pub const SYMBOLS_FOR_LEGACY_COMPUTING_ID: i32 = 306;
    /// `TANGUT_SUPPLEMENT_ID`.
    pub const TANGUT_SUPPLEMENT_ID: i32 = 307;
    /// `YEZIDI_ID`.
    pub const YEZIDI_ID: i32 = 308;
    /// `ARABIC_EXTENDED_B_ID`.
    pub const ARABIC_EXTENDED_B_ID: i32 = 309;
    /// `CYPRO_MINOAN_ID`.
    pub const CYPRO_MINOAN_ID: i32 = 310;
    /// `ETHIOPIC_EXTENDED_B_ID`.
    pub const ETHIOPIC_EXTENDED_B_ID: i32 = 311;
    /// `KANA_EXTENDED_B_ID`.
    pub const KANA_EXTENDED_B_ID: i32 = 312;
    /// `LATIN_EXTENDED_F_ID`.
    pub const LATIN_EXTENDED_F_ID: i32 = 313;
    /// `LATIN_EXTENDED_G_ID`.
    pub const LATIN_EXTENDED_G_ID: i32 = 314;
    /// `OLD_UYGHUR_ID`.
    pub const OLD_UYGHUR_ID: i32 = 315;
    /// `TANGSA_ID`.
    pub const TANGSA_ID: i32 = 316;
    /// `TOTO_ID`.
    pub const TOTO_ID: i32 = 317;
    /// `UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_A_ID`.
    pub const UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_A_ID: i32 = 318;
    /// `VITHKUQI_ID`.
    pub const VITHKUQI_ID: i32 = 319;
    /// `ZNAMENNY_MUSICAL_NOTATION_ID`.
    pub const ZNAMENNY_MUSICAL_NOTATION_ID: i32 = 320;
    /// `ARABIC_EXTENDED_C_ID`.
    pub const ARABIC_EXTENDED_C_ID: i32 = 321;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_H_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_H_ID: i32 = 322;
    /// `CYRILLIC_EXTENDED_D_ID`.
    pub const CYRILLIC_EXTENDED_D_ID: i32 = 323;
    /// `DEVANAGARI_EXTENDED_A_ID`.
    pub const DEVANAGARI_EXTENDED_A_ID: i32 = 324;
    /// `KAKTOVIK_NUMERALS_ID`.
    pub const KAKTOVIK_NUMERALS_ID: i32 = 325;
    /// `KAWI_ID`.
    pub const KAWI_ID: i32 = 326;
    /// `NAG_MUNDARI_ID`.
    pub const NAG_MUNDARI_ID: i32 = 327;
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_I_ID`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_I_ID: i32 = 328;
    /// `EGYPTIAN_HIEROGLYPHS_EXTENDED_A_ID`.
    pub const EGYPTIAN_HIEROGLYPHS_EXTENDED_A_ID: i32 = 329;
    /// `GARAY_ID`.
    pub const GARAY_ID: i32 = 330;
    /// `GURUNG_KHEMA_ID`.
    pub const GURUNG_KHEMA_ID: i32 = 331;
    /// `KIRAT_RAI_ID`.
    pub const KIRAT_RAI_ID: i32 = 332;
    /// `MYANMAR_EXTENDED_C_ID`.
    pub const MYANMAR_EXTENDED_C_ID: i32 = 333;
    /// `OL_ONAL_ID`.
    pub const OL_ONAL_ID: i32 = 334;
    /// `SUNUWAR_ID`.
    pub const SUNUWAR_ID: i32 = 335;
    /// `SYMBOLS_FOR_LEGACY_COMPUTING_SUPPLEMENT_ID`.
    pub const SYMBOLS_FOR_LEGACY_COMPUTING_SUPPLEMENT_ID: i32 = 336;
    /// `TODHRI_ID`.
    pub const TODHRI_ID: i32 = 337;
    /// `TULU_TIGALARI_ID`.
    pub const TULU_TIGALARI_ID: i32 = 338;
    /// `COUNT`.
    pub const COUNT: i32 = 339;
    /// `NO_BLOCK`.
    pub const NO_BLOCK: UnicodeBlock = UnicodeBlock {
        name: "NO_BLOCK",
        id: 0,
    };
    /// `BASIC_LATIN`.
    pub const BASIC_LATIN: UnicodeBlock = UnicodeBlock {
        name: "BASIC_LATIN",
        id: Self::BASIC_LATIN_ID,
    };
    /// `LATIN_1_SUPPLEMENT`.
    pub const LATIN_1_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "LATIN_1_SUPPLEMENT",
        id: Self::LATIN_1_SUPPLEMENT_ID,
    };
    /// `LATIN_EXTENDED_A`.
    pub const LATIN_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_A",
        id: Self::LATIN_EXTENDED_A_ID,
    };
    /// `LATIN_EXTENDED_B`.
    pub const LATIN_EXTENDED_B: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_B",
        id: Self::LATIN_EXTENDED_B_ID,
    };
    /// `IPA_EXTENSIONS`.
    pub const IPA_EXTENSIONS: UnicodeBlock = UnicodeBlock {
        name: "IPA_EXTENSIONS",
        id: Self::IPA_EXTENSIONS_ID,
    };
    /// `SPACING_MODIFIER_LETTERS`.
    pub const SPACING_MODIFIER_LETTERS: UnicodeBlock = UnicodeBlock {
        name: "SPACING_MODIFIER_LETTERS",
        id: Self::SPACING_MODIFIER_LETTERS_ID,
    };
    /// `COMBINING_DIACRITICAL_MARKS`.
    pub const COMBINING_DIACRITICAL_MARKS: UnicodeBlock = UnicodeBlock {
        name: "COMBINING_DIACRITICAL_MARKS",
        id: Self::COMBINING_DIACRITICAL_MARKS_ID,
    };
    /// `GREEK`.
    pub const GREEK: UnicodeBlock = UnicodeBlock {
        name: "GREEK",
        id: Self::GREEK_ID,
    };
    /// `CYRILLIC`.
    pub const CYRILLIC: UnicodeBlock = UnicodeBlock {
        name: "CYRILLIC",
        id: Self::CYRILLIC_ID,
    };
    /// `ARMENIAN`.
    pub const ARMENIAN: UnicodeBlock = UnicodeBlock {
        name: "ARMENIAN",
        id: Self::ARMENIAN_ID,
    };
    /// `HEBREW`.
    pub const HEBREW: UnicodeBlock = UnicodeBlock {
        name: "HEBREW",
        id: Self::HEBREW_ID,
    };
    /// `ARABIC`.
    pub const ARABIC: UnicodeBlock = UnicodeBlock {
        name: "ARABIC",
        id: Self::ARABIC_ID,
    };
    /// `SYRIAC`.
    pub const SYRIAC: UnicodeBlock = UnicodeBlock {
        name: "SYRIAC",
        id: Self::SYRIAC_ID,
    };
    /// `THAANA`.
    pub const THAANA: UnicodeBlock = UnicodeBlock {
        name: "THAANA",
        id: Self::THAANA_ID,
    };
    /// `DEVANAGARI`.
    pub const DEVANAGARI: UnicodeBlock = UnicodeBlock {
        name: "DEVANAGARI",
        id: Self::DEVANAGARI_ID,
    };
    /// `BENGALI`.
    pub const BENGALI: UnicodeBlock = UnicodeBlock {
        name: "BENGALI",
        id: Self::BENGALI_ID,
    };
    /// `GURMUKHI`.
    pub const GURMUKHI: UnicodeBlock = UnicodeBlock {
        name: "GURMUKHI",
        id: Self::GURMUKHI_ID,
    };
    /// `GUJARATI`.
    pub const GUJARATI: UnicodeBlock = UnicodeBlock {
        name: "GUJARATI",
        id: Self::GUJARATI_ID,
    };
    /// `ORIYA`.
    pub const ORIYA: UnicodeBlock = UnicodeBlock {
        name: "ORIYA",
        id: Self::ORIYA_ID,
    };
    /// `TAMIL`.
    pub const TAMIL: UnicodeBlock = UnicodeBlock {
        name: "TAMIL",
        id: Self::TAMIL_ID,
    };
    /// `TELUGU`.
    pub const TELUGU: UnicodeBlock = UnicodeBlock {
        name: "TELUGU",
        id: Self::TELUGU_ID,
    };
    /// `KANNADA`.
    pub const KANNADA: UnicodeBlock = UnicodeBlock {
        name: "KANNADA",
        id: Self::KANNADA_ID,
    };
    /// `MALAYALAM`.
    pub const MALAYALAM: UnicodeBlock = UnicodeBlock {
        name: "MALAYALAM",
        id: Self::MALAYALAM_ID,
    };
    /// `SINHALA`.
    pub const SINHALA: UnicodeBlock = UnicodeBlock {
        name: "SINHALA",
        id: Self::SINHALA_ID,
    };
    /// `THAI`.
    pub const THAI: UnicodeBlock = UnicodeBlock {
        name: "THAI",
        id: Self::THAI_ID,
    };
    /// `LAO`.
    pub const LAO: UnicodeBlock = UnicodeBlock {
        name: "LAO",
        id: Self::LAO_ID,
    };
    /// `TIBETAN`.
    pub const TIBETAN: UnicodeBlock = UnicodeBlock {
        name: "TIBETAN",
        id: Self::TIBETAN_ID,
    };
    /// `MYANMAR`.
    pub const MYANMAR: UnicodeBlock = UnicodeBlock {
        name: "MYANMAR",
        id: Self::MYANMAR_ID,
    };
    /// `GEORGIAN`.
    pub const GEORGIAN: UnicodeBlock = UnicodeBlock {
        name: "GEORGIAN",
        id: Self::GEORGIAN_ID,
    };
    /// `HANGUL_JAMO`.
    pub const HANGUL_JAMO: UnicodeBlock = UnicodeBlock {
        name: "HANGUL_JAMO",
        id: Self::HANGUL_JAMO_ID,
    };
    /// `ETHIOPIC`.
    pub const ETHIOPIC: UnicodeBlock = UnicodeBlock {
        name: "ETHIOPIC",
        id: Self::ETHIOPIC_ID,
    };
    /// `CHEROKEE`.
    pub const CHEROKEE: UnicodeBlock = UnicodeBlock {
        name: "CHEROKEE",
        id: Self::CHEROKEE_ID,
    };
    /// `UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS`.
    pub const UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS: UnicodeBlock = UnicodeBlock {
        name: "UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS",
        id: Self::UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_ID,
    };
    /// `OGHAM`.
    pub const OGHAM: UnicodeBlock = UnicodeBlock {
        name: "OGHAM",
        id: Self::OGHAM_ID,
    };
    /// `RUNIC`.
    pub const RUNIC: UnicodeBlock = UnicodeBlock {
        name: "RUNIC",
        id: Self::RUNIC_ID,
    };
    /// `KHMER`.
    pub const KHMER: UnicodeBlock = UnicodeBlock {
        name: "KHMER",
        id: Self::KHMER_ID,
    };
    /// `MONGOLIAN`.
    pub const MONGOLIAN: UnicodeBlock = UnicodeBlock {
        name: "MONGOLIAN",
        id: Self::MONGOLIAN_ID,
    };
    /// `LATIN_EXTENDED_ADDITIONAL`.
    pub const LATIN_EXTENDED_ADDITIONAL: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_ADDITIONAL",
        id: Self::LATIN_EXTENDED_ADDITIONAL_ID,
    };
    /// `GREEK_EXTENDED`.
    pub const GREEK_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "GREEK_EXTENDED",
        id: Self::GREEK_EXTENDED_ID,
    };
    /// `GENERAL_PUNCTUATION`.
    pub const GENERAL_PUNCTUATION: UnicodeBlock = UnicodeBlock {
        name: "GENERAL_PUNCTUATION",
        id: Self::GENERAL_PUNCTUATION_ID,
    };
    /// `SUPERSCRIPTS_AND_SUBSCRIPTS`.
    pub const SUPERSCRIPTS_AND_SUBSCRIPTS: UnicodeBlock = UnicodeBlock {
        name: "SUPERSCRIPTS_AND_SUBSCRIPTS",
        id: Self::SUPERSCRIPTS_AND_SUBSCRIPTS_ID,
    };
    /// `CURRENCY_SYMBOLS`.
    pub const CURRENCY_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "CURRENCY_SYMBOLS",
        id: Self::CURRENCY_SYMBOLS_ID,
    };
    /// `COMBINING_MARKS_FOR_SYMBOLS`.
    pub const COMBINING_MARKS_FOR_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "COMBINING_MARKS_FOR_SYMBOLS",
        id: Self::COMBINING_MARKS_FOR_SYMBOLS_ID,
    };
    /// `LETTERLIKE_SYMBOLS`.
    pub const LETTERLIKE_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "LETTERLIKE_SYMBOLS",
        id: Self::LETTERLIKE_SYMBOLS_ID,
    };
    /// `NUMBER_FORMS`.
    pub const NUMBER_FORMS: UnicodeBlock = UnicodeBlock {
        name: "NUMBER_FORMS",
        id: Self::NUMBER_FORMS_ID,
    };
    /// `ARROWS`.
    pub const ARROWS: UnicodeBlock = UnicodeBlock {
        name: "ARROWS",
        id: Self::ARROWS_ID,
    };
    /// `MATHEMATICAL_OPERATORS`.
    pub const MATHEMATICAL_OPERATORS: UnicodeBlock = UnicodeBlock {
        name: "MATHEMATICAL_OPERATORS",
        id: Self::MATHEMATICAL_OPERATORS_ID,
    };
    /// `MISCELLANEOUS_TECHNICAL`.
    pub const MISCELLANEOUS_TECHNICAL: UnicodeBlock = UnicodeBlock {
        name: "MISCELLANEOUS_TECHNICAL",
        id: Self::MISCELLANEOUS_TECHNICAL_ID,
    };
    /// `CONTROL_PICTURES`.
    pub const CONTROL_PICTURES: UnicodeBlock = UnicodeBlock {
        name: "CONTROL_PICTURES",
        id: Self::CONTROL_PICTURES_ID,
    };
    /// `OPTICAL_CHARACTER_RECOGNITION`.
    pub const OPTICAL_CHARACTER_RECOGNITION: UnicodeBlock = UnicodeBlock {
        name: "OPTICAL_CHARACTER_RECOGNITION",
        id: Self::OPTICAL_CHARACTER_RECOGNITION_ID,
    };
    /// `ENCLOSED_ALPHANUMERICS`.
    pub const ENCLOSED_ALPHANUMERICS: UnicodeBlock = UnicodeBlock {
        name: "ENCLOSED_ALPHANUMERICS",
        id: Self::ENCLOSED_ALPHANUMERICS_ID,
    };
    /// `BOX_DRAWING`.
    pub const BOX_DRAWING: UnicodeBlock = UnicodeBlock {
        name: "BOX_DRAWING",
        id: Self::BOX_DRAWING_ID,
    };
    /// `BLOCK_ELEMENTS`.
    pub const BLOCK_ELEMENTS: UnicodeBlock = UnicodeBlock {
        name: "BLOCK_ELEMENTS",
        id: Self::BLOCK_ELEMENTS_ID,
    };
    /// `GEOMETRIC_SHAPES`.
    pub const GEOMETRIC_SHAPES: UnicodeBlock = UnicodeBlock {
        name: "GEOMETRIC_SHAPES",
        id: Self::GEOMETRIC_SHAPES_ID,
    };
    /// `MISCELLANEOUS_SYMBOLS`.
    pub const MISCELLANEOUS_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "MISCELLANEOUS_SYMBOLS",
        id: Self::MISCELLANEOUS_SYMBOLS_ID,
    };
    /// `DINGBATS`.
    pub const DINGBATS: UnicodeBlock = UnicodeBlock {
        name: "DINGBATS",
        id: Self::DINGBATS_ID,
    };
    /// `BRAILLE_PATTERNS`.
    pub const BRAILLE_PATTERNS: UnicodeBlock = UnicodeBlock {
        name: "BRAILLE_PATTERNS",
        id: Self::BRAILLE_PATTERNS_ID,
    };
    /// `CJK_RADICALS_SUPPLEMENT`.
    pub const CJK_RADICALS_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "CJK_RADICALS_SUPPLEMENT",
        id: Self::CJK_RADICALS_SUPPLEMENT_ID,
    };
    /// `KANGXI_RADICALS`.
    pub const KANGXI_RADICALS: UnicodeBlock = UnicodeBlock {
        name: "KANGXI_RADICALS",
        id: Self::KANGXI_RADICALS_ID,
    };
    /// `IDEOGRAPHIC_DESCRIPTION_CHARACTERS`.
    pub const IDEOGRAPHIC_DESCRIPTION_CHARACTERS: UnicodeBlock = UnicodeBlock {
        name: "IDEOGRAPHIC_DESCRIPTION_CHARACTERS",
        id: Self::IDEOGRAPHIC_DESCRIPTION_CHARACTERS_ID,
    };
    /// `CJK_SYMBOLS_AND_PUNCTUATION`.
    pub const CJK_SYMBOLS_AND_PUNCTUATION: UnicodeBlock = UnicodeBlock {
        name: "CJK_SYMBOLS_AND_PUNCTUATION",
        id: Self::CJK_SYMBOLS_AND_PUNCTUATION_ID,
    };
    /// `HIRAGANA`.
    pub const HIRAGANA: UnicodeBlock = UnicodeBlock {
        name: "HIRAGANA",
        id: Self::HIRAGANA_ID,
    };
    /// `KATAKANA`.
    pub const KATAKANA: UnicodeBlock = UnicodeBlock {
        name: "KATAKANA",
        id: Self::KATAKANA_ID,
    };
    /// `BOPOMOFO`.
    pub const BOPOMOFO: UnicodeBlock = UnicodeBlock {
        name: "BOPOMOFO",
        id: Self::BOPOMOFO_ID,
    };
    /// `HANGUL_COMPATIBILITY_JAMO`.
    pub const HANGUL_COMPATIBILITY_JAMO: UnicodeBlock = UnicodeBlock {
        name: "HANGUL_COMPATIBILITY_JAMO",
        id: Self::HANGUL_COMPATIBILITY_JAMO_ID,
    };
    /// `KANBUN`.
    pub const KANBUN: UnicodeBlock = UnicodeBlock {
        name: "KANBUN",
        id: Self::KANBUN_ID,
    };
    /// `BOPOMOFO_EXTENDED`.
    pub const BOPOMOFO_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "BOPOMOFO_EXTENDED",
        id: Self::BOPOMOFO_EXTENDED_ID,
    };
    /// `ENCLOSED_CJK_LETTERS_AND_MONTHS`.
    pub const ENCLOSED_CJK_LETTERS_AND_MONTHS: UnicodeBlock = UnicodeBlock {
        name: "ENCLOSED_CJK_LETTERS_AND_MONTHS",
        id: Self::ENCLOSED_CJK_LETTERS_AND_MONTHS_ID,
    };
    /// `CJK_COMPATIBILITY`.
    pub const CJK_COMPATIBILITY: UnicodeBlock = UnicodeBlock {
        name: "CJK_COMPATIBILITY",
        id: Self::CJK_COMPATIBILITY_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS`.
    pub const CJK_UNIFIED_IDEOGRAPHS: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_ID,
    };
    /// `YI_SYLLABLES`.
    pub const YI_SYLLABLES: UnicodeBlock = UnicodeBlock {
        name: "YI_SYLLABLES",
        id: Self::YI_SYLLABLES_ID,
    };
    /// `YI_RADICALS`.
    pub const YI_RADICALS: UnicodeBlock = UnicodeBlock {
        name: "YI_RADICALS",
        id: Self::YI_RADICALS_ID,
    };
    /// `HANGUL_SYLLABLES`.
    pub const HANGUL_SYLLABLES: UnicodeBlock = UnicodeBlock {
        name: "HANGUL_SYLLABLES",
        id: Self::HANGUL_SYLLABLES_ID,
    };
    /// `HIGH_SURROGATES`.
    pub const HIGH_SURROGATES: UnicodeBlock = UnicodeBlock {
        name: "HIGH_SURROGATES",
        id: Self::HIGH_SURROGATES_ID,
    };
    /// `HIGH_PRIVATE_USE_SURROGATES`.
    pub const HIGH_PRIVATE_USE_SURROGATES: UnicodeBlock = UnicodeBlock {
        name: "HIGH_PRIVATE_USE_SURROGATES",
        id: Self::HIGH_PRIVATE_USE_SURROGATES_ID,
    };
    /// `LOW_SURROGATES`.
    pub const LOW_SURROGATES: UnicodeBlock = UnicodeBlock {
        name: "LOW_SURROGATES",
        id: Self::LOW_SURROGATES_ID,
    };
    /// `PRIVATE_USE_AREA`.
    pub const PRIVATE_USE_AREA: UnicodeBlock = UnicodeBlock {
        name: "PRIVATE_USE_AREA",
        id: 78,
    };
    /// `PRIVATE_USE`.
    pub const PRIVATE_USE: UnicodeBlock = Self::PRIVATE_USE_AREA;
    /// `CJK_COMPATIBILITY_IDEOGRAPHS`.
    pub const CJK_COMPATIBILITY_IDEOGRAPHS: UnicodeBlock = UnicodeBlock {
        name: "CJK_COMPATIBILITY_IDEOGRAPHS",
        id: Self::CJK_COMPATIBILITY_IDEOGRAPHS_ID,
    };
    /// `ALPHABETIC_PRESENTATION_FORMS`.
    pub const ALPHABETIC_PRESENTATION_FORMS: UnicodeBlock = UnicodeBlock {
        name: "ALPHABETIC_PRESENTATION_FORMS",
        id: Self::ALPHABETIC_PRESENTATION_FORMS_ID,
    };
    /// `ARABIC_PRESENTATION_FORMS_A`.
    pub const ARABIC_PRESENTATION_FORMS_A: UnicodeBlock = UnicodeBlock {
        name: "ARABIC_PRESENTATION_FORMS_A",
        id: Self::ARABIC_PRESENTATION_FORMS_A_ID,
    };
    /// `COMBINING_HALF_MARKS`.
    pub const COMBINING_HALF_MARKS: UnicodeBlock = UnicodeBlock {
        name: "COMBINING_HALF_MARKS",
        id: Self::COMBINING_HALF_MARKS_ID,
    };
    /// `CJK_COMPATIBILITY_FORMS`.
    pub const CJK_COMPATIBILITY_FORMS: UnicodeBlock = UnicodeBlock {
        name: "CJK_COMPATIBILITY_FORMS",
        id: Self::CJK_COMPATIBILITY_FORMS_ID,
    };
    /// `SMALL_FORM_VARIANTS`.
    pub const SMALL_FORM_VARIANTS: UnicodeBlock = UnicodeBlock {
        name: "SMALL_FORM_VARIANTS",
        id: Self::SMALL_FORM_VARIANTS_ID,
    };
    /// `ARABIC_PRESENTATION_FORMS_B`.
    pub const ARABIC_PRESENTATION_FORMS_B: UnicodeBlock = UnicodeBlock {
        name: "ARABIC_PRESENTATION_FORMS_B",
        id: Self::ARABIC_PRESENTATION_FORMS_B_ID,
    };
    /// `SPECIALS`.
    pub const SPECIALS: UnicodeBlock = UnicodeBlock {
        name: "SPECIALS",
        id: Self::SPECIALS_ID,
    };
    /// `HALFWIDTH_AND_FULLWIDTH_FORMS`.
    pub const HALFWIDTH_AND_FULLWIDTH_FORMS: UnicodeBlock = UnicodeBlock {
        name: "HALFWIDTH_AND_FULLWIDTH_FORMS",
        id: Self::HALFWIDTH_AND_FULLWIDTH_FORMS_ID,
    };
    /// `OLD_ITALIC`.
    pub const OLD_ITALIC: UnicodeBlock = UnicodeBlock {
        name: "OLD_ITALIC",
        id: Self::OLD_ITALIC_ID,
    };
    /// `GOTHIC`.
    pub const GOTHIC: UnicodeBlock = UnicodeBlock {
        name: "GOTHIC",
        id: Self::GOTHIC_ID,
    };
    /// `DESERET`.
    pub const DESERET: UnicodeBlock = UnicodeBlock {
        name: "DESERET",
        id: Self::DESERET_ID,
    };
    /// `BYZANTINE_MUSICAL_SYMBOLS`.
    pub const BYZANTINE_MUSICAL_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "BYZANTINE_MUSICAL_SYMBOLS",
        id: Self::BYZANTINE_MUSICAL_SYMBOLS_ID,
    };
    /// `MUSICAL_SYMBOLS`.
    pub const MUSICAL_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "MUSICAL_SYMBOLS",
        id: Self::MUSICAL_SYMBOLS_ID,
    };
    /// `MATHEMATICAL_ALPHANUMERIC_SYMBOLS`.
    pub const MATHEMATICAL_ALPHANUMERIC_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "MATHEMATICAL_ALPHANUMERIC_SYMBOLS",
        id: Self::MATHEMATICAL_ALPHANUMERIC_SYMBOLS_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_B`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_B: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_B",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_B_ID,
    };
    /// `CJK_COMPATIBILITY_IDEOGRAPHS_SUPPLEMENT`.
    pub const CJK_COMPATIBILITY_IDEOGRAPHS_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "CJK_COMPATIBILITY_IDEOGRAPHS_SUPPLEMENT",
        id: Self::CJK_COMPATIBILITY_IDEOGRAPHS_SUPPLEMENT_ID,
    };
    /// `TAGS`.
    pub const TAGS: UnicodeBlock = UnicodeBlock {
        name: "TAGS",
        id: Self::TAGS_ID,
    };
    /// `CYRILLIC_SUPPLEMENTARY`.
    pub const CYRILLIC_SUPPLEMENTARY: UnicodeBlock = UnicodeBlock {
        name: "CYRILLIC_SUPPLEMENTARY",
        id: Self::CYRILLIC_SUPPLEMENTARY_ID,
    };
    /// `CYRILLIC_SUPPLEMENT`.
    pub const CYRILLIC_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "CYRILLIC_SUPPLEMENT",
        id: Self::CYRILLIC_SUPPLEMENT_ID,
    };
    /// `TAGALOG`.
    pub const TAGALOG: UnicodeBlock = UnicodeBlock {
        name: "TAGALOG",
        id: Self::TAGALOG_ID,
    };
    /// `HANUNOO`.
    pub const HANUNOO: UnicodeBlock = UnicodeBlock {
        name: "HANUNOO",
        id: Self::HANUNOO_ID,
    };
    /// `BUHID`.
    pub const BUHID: UnicodeBlock = UnicodeBlock {
        name: "BUHID",
        id: Self::BUHID_ID,
    };
    /// `TAGBANWA`.
    pub const TAGBANWA: UnicodeBlock = UnicodeBlock {
        name: "TAGBANWA",
        id: Self::TAGBANWA_ID,
    };
    /// `MISCELLANEOUS_MATHEMATICAL_SYMBOLS_A`.
    pub const MISCELLANEOUS_MATHEMATICAL_SYMBOLS_A: UnicodeBlock = UnicodeBlock {
        name: "MISCELLANEOUS_MATHEMATICAL_SYMBOLS_A",
        id: Self::MISCELLANEOUS_MATHEMATICAL_SYMBOLS_A_ID,
    };
    /// `SUPPLEMENTAL_ARROWS_A`.
    pub const SUPPLEMENTAL_ARROWS_A: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTAL_ARROWS_A",
        id: Self::SUPPLEMENTAL_ARROWS_A_ID,
    };
    /// `SUPPLEMENTAL_ARROWS_B`.
    pub const SUPPLEMENTAL_ARROWS_B: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTAL_ARROWS_B",
        id: Self::SUPPLEMENTAL_ARROWS_B_ID,
    };
    /// `MISCELLANEOUS_MATHEMATICAL_SYMBOLS_B`.
    pub const MISCELLANEOUS_MATHEMATICAL_SYMBOLS_B: UnicodeBlock = UnicodeBlock {
        name: "MISCELLANEOUS_MATHEMATICAL_SYMBOLS_B",
        id: Self::MISCELLANEOUS_MATHEMATICAL_SYMBOLS_B_ID,
    };
    /// `SUPPLEMENTAL_MATHEMATICAL_OPERATORS`.
    pub const SUPPLEMENTAL_MATHEMATICAL_OPERATORS: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTAL_MATHEMATICAL_OPERATORS",
        id: Self::SUPPLEMENTAL_MATHEMATICAL_OPERATORS_ID,
    };
    /// `KATAKANA_PHONETIC_EXTENSIONS`.
    pub const KATAKANA_PHONETIC_EXTENSIONS: UnicodeBlock = UnicodeBlock {
        name: "KATAKANA_PHONETIC_EXTENSIONS",
        id: Self::KATAKANA_PHONETIC_EXTENSIONS_ID,
    };
    /// `VARIATION_SELECTORS`.
    pub const VARIATION_SELECTORS: UnicodeBlock = UnicodeBlock {
        name: "VARIATION_SELECTORS",
        id: Self::VARIATION_SELECTORS_ID,
    };
    /// `SUPPLEMENTARY_PRIVATE_USE_AREA_A`.
    pub const SUPPLEMENTARY_PRIVATE_USE_AREA_A: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTARY_PRIVATE_USE_AREA_A",
        id: Self::SUPPLEMENTARY_PRIVATE_USE_AREA_A_ID,
    };
    /// `SUPPLEMENTARY_PRIVATE_USE_AREA_B`.
    pub const SUPPLEMENTARY_PRIVATE_USE_AREA_B: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTARY_PRIVATE_USE_AREA_B",
        id: Self::SUPPLEMENTARY_PRIVATE_USE_AREA_B_ID,
    };
    /// `LIMBU`.
    pub const LIMBU: UnicodeBlock = UnicodeBlock {
        name: "LIMBU",
        id: Self::LIMBU_ID,
    };
    /// `TAI_LE`.
    pub const TAI_LE: UnicodeBlock = UnicodeBlock {
        name: "TAI_LE",
        id: Self::TAI_LE_ID,
    };
    /// `KHMER_SYMBOLS`.
    pub const KHMER_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "KHMER_SYMBOLS",
        id: Self::KHMER_SYMBOLS_ID,
    };
    /// `PHONETIC_EXTENSIONS`.
    pub const PHONETIC_EXTENSIONS: UnicodeBlock = UnicodeBlock {
        name: "PHONETIC_EXTENSIONS",
        id: Self::PHONETIC_EXTENSIONS_ID,
    };
    /// `MISCELLANEOUS_SYMBOLS_AND_ARROWS`.
    pub const MISCELLANEOUS_SYMBOLS_AND_ARROWS: UnicodeBlock = UnicodeBlock {
        name: "MISCELLANEOUS_SYMBOLS_AND_ARROWS",
        id: Self::MISCELLANEOUS_SYMBOLS_AND_ARROWS_ID,
    };
    /// `YIJING_HEXAGRAM_SYMBOLS`.
    pub const YIJING_HEXAGRAM_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "YIJING_HEXAGRAM_SYMBOLS",
        id: Self::YIJING_HEXAGRAM_SYMBOLS_ID,
    };
    /// `LINEAR_B_SYLLABARY`.
    pub const LINEAR_B_SYLLABARY: UnicodeBlock = UnicodeBlock {
        name: "LINEAR_B_SYLLABARY",
        id: Self::LINEAR_B_SYLLABARY_ID,
    };
    /// `LINEAR_B_IDEOGRAMS`.
    pub const LINEAR_B_IDEOGRAMS: UnicodeBlock = UnicodeBlock {
        name: "LINEAR_B_IDEOGRAMS",
        id: Self::LINEAR_B_IDEOGRAMS_ID,
    };
    /// `AEGEAN_NUMBERS`.
    pub const AEGEAN_NUMBERS: UnicodeBlock = UnicodeBlock {
        name: "AEGEAN_NUMBERS",
        id: Self::AEGEAN_NUMBERS_ID,
    };
    /// `UGARITIC`.
    pub const UGARITIC: UnicodeBlock = UnicodeBlock {
        name: "UGARITIC",
        id: Self::UGARITIC_ID,
    };
    /// `SHAVIAN`.
    pub const SHAVIAN: UnicodeBlock = UnicodeBlock {
        name: "SHAVIAN",
        id: Self::SHAVIAN_ID,
    };
    /// `OSMANYA`.
    pub const OSMANYA: UnicodeBlock = UnicodeBlock {
        name: "OSMANYA",
        id: Self::OSMANYA_ID,
    };
    /// `CYPRIOT_SYLLABARY`.
    pub const CYPRIOT_SYLLABARY: UnicodeBlock = UnicodeBlock {
        name: "CYPRIOT_SYLLABARY",
        id: Self::CYPRIOT_SYLLABARY_ID,
    };
    /// `TAI_XUAN_JING_SYMBOLS`.
    pub const TAI_XUAN_JING_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "TAI_XUAN_JING_SYMBOLS",
        id: Self::TAI_XUAN_JING_SYMBOLS_ID,
    };
    /// `VARIATION_SELECTORS_SUPPLEMENT`.
    pub const VARIATION_SELECTORS_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "VARIATION_SELECTORS_SUPPLEMENT",
        id: Self::VARIATION_SELECTORS_SUPPLEMENT_ID,
    };
    /// `ANCIENT_GREEK_MUSICAL_NOTATION`.
    pub const ANCIENT_GREEK_MUSICAL_NOTATION: UnicodeBlock = UnicodeBlock {
        name: "ANCIENT_GREEK_MUSICAL_NOTATION",
        id: Self::ANCIENT_GREEK_MUSICAL_NOTATION_ID,
    };
    /// `ANCIENT_GREEK_NUMBERS`.
    pub const ANCIENT_GREEK_NUMBERS: UnicodeBlock = UnicodeBlock {
        name: "ANCIENT_GREEK_NUMBERS",
        id: Self::ANCIENT_GREEK_NUMBERS_ID,
    };
    /// `ARABIC_SUPPLEMENT`.
    pub const ARABIC_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "ARABIC_SUPPLEMENT",
        id: Self::ARABIC_SUPPLEMENT_ID,
    };
    /// `BUGINESE`.
    pub const BUGINESE: UnicodeBlock = UnicodeBlock {
        name: "BUGINESE",
        id: Self::BUGINESE_ID,
    };
    /// `CJK_STROKES`.
    pub const CJK_STROKES: UnicodeBlock = UnicodeBlock {
        name: "CJK_STROKES",
        id: Self::CJK_STROKES_ID,
    };
    /// `COMBINING_DIACRITICAL_MARKS_SUPPLEMENT`.
    pub const COMBINING_DIACRITICAL_MARKS_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "COMBINING_DIACRITICAL_MARKS_SUPPLEMENT",
        id: Self::COMBINING_DIACRITICAL_MARKS_SUPPLEMENT_ID,
    };
    /// `COPTIC`.
    pub const COPTIC: UnicodeBlock = UnicodeBlock {
        name: "COPTIC",
        id: Self::COPTIC_ID,
    };
    /// `ETHIOPIC_EXTENDED`.
    pub const ETHIOPIC_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "ETHIOPIC_EXTENDED",
        id: Self::ETHIOPIC_EXTENDED_ID,
    };
    /// `ETHIOPIC_SUPPLEMENT`.
    pub const ETHIOPIC_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "ETHIOPIC_SUPPLEMENT",
        id: Self::ETHIOPIC_SUPPLEMENT_ID,
    };
    /// `GEORGIAN_SUPPLEMENT`.
    pub const GEORGIAN_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "GEORGIAN_SUPPLEMENT",
        id: Self::GEORGIAN_SUPPLEMENT_ID,
    };
    /// `GLAGOLITIC`.
    pub const GLAGOLITIC: UnicodeBlock = UnicodeBlock {
        name: "GLAGOLITIC",
        id: Self::GLAGOLITIC_ID,
    };
    /// `KHAROSHTHI`.
    pub const KHAROSHTHI: UnicodeBlock = UnicodeBlock {
        name: "KHAROSHTHI",
        id: Self::KHAROSHTHI_ID,
    };
    /// `MODIFIER_TONE_LETTERS`.
    pub const MODIFIER_TONE_LETTERS: UnicodeBlock = UnicodeBlock {
        name: "MODIFIER_TONE_LETTERS",
        id: Self::MODIFIER_TONE_LETTERS_ID,
    };
    /// `NEW_TAI_LUE`.
    pub const NEW_TAI_LUE: UnicodeBlock = UnicodeBlock {
        name: "NEW_TAI_LUE",
        id: Self::NEW_TAI_LUE_ID,
    };
    /// `OLD_PERSIAN`.
    pub const OLD_PERSIAN: UnicodeBlock = UnicodeBlock {
        name: "OLD_PERSIAN",
        id: Self::OLD_PERSIAN_ID,
    };
    /// `PHONETIC_EXTENSIONS_SUPPLEMENT`.
    pub const PHONETIC_EXTENSIONS_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "PHONETIC_EXTENSIONS_SUPPLEMENT",
        id: Self::PHONETIC_EXTENSIONS_SUPPLEMENT_ID,
    };
    /// `SUPPLEMENTAL_PUNCTUATION`.
    pub const SUPPLEMENTAL_PUNCTUATION: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTAL_PUNCTUATION",
        id: Self::SUPPLEMENTAL_PUNCTUATION_ID,
    };
    /// `SYLOTI_NAGRI`.
    pub const SYLOTI_NAGRI: UnicodeBlock = UnicodeBlock {
        name: "SYLOTI_NAGRI",
        id: Self::SYLOTI_NAGRI_ID,
    };
    /// `TIFINAGH`.
    pub const TIFINAGH: UnicodeBlock = UnicodeBlock {
        name: "TIFINAGH",
        id: Self::TIFINAGH_ID,
    };
    /// `VERTICAL_FORMS`.
    pub const VERTICAL_FORMS: UnicodeBlock = UnicodeBlock {
        name: "VERTICAL_FORMS",
        id: Self::VERTICAL_FORMS_ID,
    };
    /// `NKO`.
    pub const NKO: UnicodeBlock = UnicodeBlock {
        name: "NKO",
        id: Self::NKO_ID,
    };
    /// `BALINESE`.
    pub const BALINESE: UnicodeBlock = UnicodeBlock {
        name: "BALINESE",
        id: Self::BALINESE_ID,
    };
    /// `LATIN_EXTENDED_C`.
    pub const LATIN_EXTENDED_C: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_C",
        id: Self::LATIN_EXTENDED_C_ID,
    };
    /// `LATIN_EXTENDED_D`.
    pub const LATIN_EXTENDED_D: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_D",
        id: Self::LATIN_EXTENDED_D_ID,
    };
    /// `PHAGS_PA`.
    pub const PHAGS_PA: UnicodeBlock = UnicodeBlock {
        name: "PHAGS_PA",
        id: Self::PHAGS_PA_ID,
    };
    /// `PHOENICIAN`.
    pub const PHOENICIAN: UnicodeBlock = UnicodeBlock {
        name: "PHOENICIAN",
        id: Self::PHOENICIAN_ID,
    };
    /// `CUNEIFORM`.
    pub const CUNEIFORM: UnicodeBlock = UnicodeBlock {
        name: "CUNEIFORM",
        id: Self::CUNEIFORM_ID,
    };
    /// `CUNEIFORM_NUMBERS_AND_PUNCTUATION`.
    pub const CUNEIFORM_NUMBERS_AND_PUNCTUATION: UnicodeBlock = UnicodeBlock {
        name: "CUNEIFORM_NUMBERS_AND_PUNCTUATION",
        id: Self::CUNEIFORM_NUMBERS_AND_PUNCTUATION_ID,
    };
    /// `COUNTING_ROD_NUMERALS`.
    pub const COUNTING_ROD_NUMERALS: UnicodeBlock = UnicodeBlock {
        name: "COUNTING_ROD_NUMERALS",
        id: Self::COUNTING_ROD_NUMERALS_ID,
    };
    /// `SUNDANESE`.
    pub const SUNDANESE: UnicodeBlock = UnicodeBlock {
        name: "SUNDANESE",
        id: Self::SUNDANESE_ID,
    };
    /// `LEPCHA`.
    pub const LEPCHA: UnicodeBlock = UnicodeBlock {
        name: "LEPCHA",
        id: Self::LEPCHA_ID,
    };
    /// `OL_CHIKI`.
    pub const OL_CHIKI: UnicodeBlock = UnicodeBlock {
        name: "OL_CHIKI",
        id: Self::OL_CHIKI_ID,
    };
    /// `CYRILLIC_EXTENDED_A`.
    pub const CYRILLIC_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "CYRILLIC_EXTENDED_A",
        id: Self::CYRILLIC_EXTENDED_A_ID,
    };
    /// `VAI`.
    pub const VAI: UnicodeBlock = UnicodeBlock {
        name: "VAI",
        id: Self::VAI_ID,
    };
    /// `CYRILLIC_EXTENDED_B`.
    pub const CYRILLIC_EXTENDED_B: UnicodeBlock = UnicodeBlock {
        name: "CYRILLIC_EXTENDED_B",
        id: Self::CYRILLIC_EXTENDED_B_ID,
    };
    /// `SAURASHTRA`.
    pub const SAURASHTRA: UnicodeBlock = UnicodeBlock {
        name: "SAURASHTRA",
        id: Self::SAURASHTRA_ID,
    };
    /// `KAYAH_LI`.
    pub const KAYAH_LI: UnicodeBlock = UnicodeBlock {
        name: "KAYAH_LI",
        id: Self::KAYAH_LI_ID,
    };
    /// `REJANG`.
    pub const REJANG: UnicodeBlock = UnicodeBlock {
        name: "REJANG",
        id: Self::REJANG_ID,
    };
    /// `CHAM`.
    pub const CHAM: UnicodeBlock = UnicodeBlock {
        name: "CHAM",
        id: Self::CHAM_ID,
    };
    /// `ANCIENT_SYMBOLS`.
    pub const ANCIENT_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "ANCIENT_SYMBOLS",
        id: Self::ANCIENT_SYMBOLS_ID,
    };
    /// `PHAISTOS_DISC`.
    pub const PHAISTOS_DISC: UnicodeBlock = UnicodeBlock {
        name: "PHAISTOS_DISC",
        id: Self::PHAISTOS_DISC_ID,
    };
    /// `LYCIAN`.
    pub const LYCIAN: UnicodeBlock = UnicodeBlock {
        name: "LYCIAN",
        id: Self::LYCIAN_ID,
    };
    /// `CARIAN`.
    pub const CARIAN: UnicodeBlock = UnicodeBlock {
        name: "CARIAN",
        id: Self::CARIAN_ID,
    };
    /// `LYDIAN`.
    pub const LYDIAN: UnicodeBlock = UnicodeBlock {
        name: "LYDIAN",
        id: Self::LYDIAN_ID,
    };
    /// `MAHJONG_TILES`.
    pub const MAHJONG_TILES: UnicodeBlock = UnicodeBlock {
        name: "MAHJONG_TILES",
        id: Self::MAHJONG_TILES_ID,
    };
    /// `DOMINO_TILES`.
    pub const DOMINO_TILES: UnicodeBlock = UnicodeBlock {
        name: "DOMINO_TILES",
        id: Self::DOMINO_TILES_ID,
    };
    /// `SAMARITAN`.
    pub const SAMARITAN: UnicodeBlock = UnicodeBlock {
        name: "SAMARITAN",
        id: Self::SAMARITAN_ID,
    };
    /// `UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED`.
    pub const UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED",
        id: Self::UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_ID,
    };
    /// `TAI_THAM`.
    pub const TAI_THAM: UnicodeBlock = UnicodeBlock {
        name: "TAI_THAM",
        id: Self::TAI_THAM_ID,
    };
    /// `VEDIC_EXTENSIONS`.
    pub const VEDIC_EXTENSIONS: UnicodeBlock = UnicodeBlock {
        name: "VEDIC_EXTENSIONS",
        id: Self::VEDIC_EXTENSIONS_ID,
    };
    /// `LISU`.
    pub const LISU: UnicodeBlock = UnicodeBlock {
        name: "LISU",
        id: Self::LISU_ID,
    };
    /// `BAMUM`.
    pub const BAMUM: UnicodeBlock = UnicodeBlock {
        name: "BAMUM",
        id: Self::BAMUM_ID,
    };
    /// `COMMON_INDIC_NUMBER_FORMS`.
    pub const COMMON_INDIC_NUMBER_FORMS: UnicodeBlock = UnicodeBlock {
        name: "COMMON_INDIC_NUMBER_FORMS",
        id: Self::COMMON_INDIC_NUMBER_FORMS_ID,
    };
    /// `DEVANAGARI_EXTENDED`.
    pub const DEVANAGARI_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "DEVANAGARI_EXTENDED",
        id: Self::DEVANAGARI_EXTENDED_ID,
    };
    /// `HANGUL_JAMO_EXTENDED_A`.
    pub const HANGUL_JAMO_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "HANGUL_JAMO_EXTENDED_A",
        id: Self::HANGUL_JAMO_EXTENDED_A_ID,
    };
    /// `JAVANESE`.
    pub const JAVANESE: UnicodeBlock = UnicodeBlock {
        name: "JAVANESE",
        id: Self::JAVANESE_ID,
    };
    /// `MYANMAR_EXTENDED_A`.
    pub const MYANMAR_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "MYANMAR_EXTENDED_A",
        id: Self::MYANMAR_EXTENDED_A_ID,
    };
    /// `TAI_VIET`.
    pub const TAI_VIET: UnicodeBlock = UnicodeBlock {
        name: "TAI_VIET",
        id: Self::TAI_VIET_ID,
    };
    /// `MEETEI_MAYEK`.
    pub const MEETEI_MAYEK: UnicodeBlock = UnicodeBlock {
        name: "MEETEI_MAYEK",
        id: Self::MEETEI_MAYEK_ID,
    };
    /// `HANGUL_JAMO_EXTENDED_B`.
    pub const HANGUL_JAMO_EXTENDED_B: UnicodeBlock = UnicodeBlock {
        name: "HANGUL_JAMO_EXTENDED_B",
        id: Self::HANGUL_JAMO_EXTENDED_B_ID,
    };
    /// `IMPERIAL_ARAMAIC`.
    pub const IMPERIAL_ARAMAIC: UnicodeBlock = UnicodeBlock {
        name: "IMPERIAL_ARAMAIC",
        id: Self::IMPERIAL_ARAMAIC_ID,
    };
    /// `OLD_SOUTH_ARABIAN`.
    pub const OLD_SOUTH_ARABIAN: UnicodeBlock = UnicodeBlock {
        name: "OLD_SOUTH_ARABIAN",
        id: Self::OLD_SOUTH_ARABIAN_ID,
    };
    /// `AVESTAN`.
    pub const AVESTAN: UnicodeBlock = UnicodeBlock {
        name: "AVESTAN",
        id: Self::AVESTAN_ID,
    };
    /// `INSCRIPTIONAL_PARTHIAN`.
    pub const INSCRIPTIONAL_PARTHIAN: UnicodeBlock = UnicodeBlock {
        name: "INSCRIPTIONAL_PARTHIAN",
        id: Self::INSCRIPTIONAL_PARTHIAN_ID,
    };
    /// `INSCRIPTIONAL_PAHLAVI`.
    pub const INSCRIPTIONAL_PAHLAVI: UnicodeBlock = UnicodeBlock {
        name: "INSCRIPTIONAL_PAHLAVI",
        id: Self::INSCRIPTIONAL_PAHLAVI_ID,
    };
    /// `OLD_TURKIC`.
    pub const OLD_TURKIC: UnicodeBlock = UnicodeBlock {
        name: "OLD_TURKIC",
        id: Self::OLD_TURKIC_ID,
    };
    /// `RUMI_NUMERAL_SYMBOLS`.
    pub const RUMI_NUMERAL_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "RUMI_NUMERAL_SYMBOLS",
        id: Self::RUMI_NUMERAL_SYMBOLS_ID,
    };
    /// `KAITHI`.
    pub const KAITHI: UnicodeBlock = UnicodeBlock {
        name: "KAITHI",
        id: Self::KAITHI_ID,
    };
    /// `EGYPTIAN_HIEROGLYPHS`.
    pub const EGYPTIAN_HIEROGLYPHS: UnicodeBlock = UnicodeBlock {
        name: "EGYPTIAN_HIEROGLYPHS",
        id: Self::EGYPTIAN_HIEROGLYPHS_ID,
    };
    /// `ENCLOSED_ALPHANUMERIC_SUPPLEMENT`.
    pub const ENCLOSED_ALPHANUMERIC_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "ENCLOSED_ALPHANUMERIC_SUPPLEMENT",
        id: Self::ENCLOSED_ALPHANUMERIC_SUPPLEMENT_ID,
    };
    /// `ENCLOSED_IDEOGRAPHIC_SUPPLEMENT`.
    pub const ENCLOSED_IDEOGRAPHIC_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "ENCLOSED_IDEOGRAPHIC_SUPPLEMENT",
        id: Self::ENCLOSED_IDEOGRAPHIC_SUPPLEMENT_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_C`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_C: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_C",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_C_ID,
    };
    /// `MANDAIC`.
    pub const MANDAIC: UnicodeBlock = UnicodeBlock {
        name: "MANDAIC",
        id: Self::MANDAIC_ID,
    };
    /// `BATAK`.
    pub const BATAK: UnicodeBlock = UnicodeBlock {
        name: "BATAK",
        id: Self::BATAK_ID,
    };
    /// `ETHIOPIC_EXTENDED_A`.
    pub const ETHIOPIC_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "ETHIOPIC_EXTENDED_A",
        id: Self::ETHIOPIC_EXTENDED_A_ID,
    };
    /// `BRAHMI`.
    pub const BRAHMI: UnicodeBlock = UnicodeBlock {
        name: "BRAHMI",
        id: Self::BRAHMI_ID,
    };
    /// `BAMUM_SUPPLEMENT`.
    pub const BAMUM_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "BAMUM_SUPPLEMENT",
        id: Self::BAMUM_SUPPLEMENT_ID,
    };
    /// `KANA_SUPPLEMENT`.
    pub const KANA_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "KANA_SUPPLEMENT",
        id: Self::KANA_SUPPLEMENT_ID,
    };
    /// `PLAYING_CARDS`.
    pub const PLAYING_CARDS: UnicodeBlock = UnicodeBlock {
        name: "PLAYING_CARDS",
        id: Self::PLAYING_CARDS_ID,
    };
    /// `MISCELLANEOUS_SYMBOLS_AND_PICTOGRAPHS`.
    pub const MISCELLANEOUS_SYMBOLS_AND_PICTOGRAPHS: UnicodeBlock = UnicodeBlock {
        name: "MISCELLANEOUS_SYMBOLS_AND_PICTOGRAPHS",
        id: Self::MISCELLANEOUS_SYMBOLS_AND_PICTOGRAPHS_ID,
    };
    /// `EMOTICONS`.
    pub const EMOTICONS: UnicodeBlock = UnicodeBlock {
        name: "EMOTICONS",
        id: Self::EMOTICONS_ID,
    };
    /// `TRANSPORT_AND_MAP_SYMBOLS`.
    pub const TRANSPORT_AND_MAP_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "TRANSPORT_AND_MAP_SYMBOLS",
        id: Self::TRANSPORT_AND_MAP_SYMBOLS_ID,
    };
    /// `ALCHEMICAL_SYMBOLS`.
    pub const ALCHEMICAL_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "ALCHEMICAL_SYMBOLS",
        id: Self::ALCHEMICAL_SYMBOLS_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_D`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_D: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_D",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_D_ID,
    };
    /// `ARABIC_EXTENDED_A`.
    pub const ARABIC_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "ARABIC_EXTENDED_A",
        id: Self::ARABIC_EXTENDED_A_ID,
    };
    /// `ARABIC_MATHEMATICAL_ALPHABETIC_SYMBOLS`.
    pub const ARABIC_MATHEMATICAL_ALPHABETIC_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "ARABIC_MATHEMATICAL_ALPHABETIC_SYMBOLS",
        id: Self::ARABIC_MATHEMATICAL_ALPHABETIC_SYMBOLS_ID,
    };
    /// `CHAKMA`.
    pub const CHAKMA: UnicodeBlock = UnicodeBlock {
        name: "CHAKMA",
        id: Self::CHAKMA_ID,
    };
    /// `MEETEI_MAYEK_EXTENSIONS`.
    pub const MEETEI_MAYEK_EXTENSIONS: UnicodeBlock = UnicodeBlock {
        name: "MEETEI_MAYEK_EXTENSIONS",
        id: Self::MEETEI_MAYEK_EXTENSIONS_ID,
    };
    /// `MEROITIC_CURSIVE`.
    pub const MEROITIC_CURSIVE: UnicodeBlock = UnicodeBlock {
        name: "MEROITIC_CURSIVE",
        id: Self::MEROITIC_CURSIVE_ID,
    };
    /// `MEROITIC_HIEROGLYPHS`.
    pub const MEROITIC_HIEROGLYPHS: UnicodeBlock = UnicodeBlock {
        name: "MEROITIC_HIEROGLYPHS",
        id: Self::MEROITIC_HIEROGLYPHS_ID,
    };
    /// `MIAO`.
    pub const MIAO: UnicodeBlock = UnicodeBlock {
        name: "MIAO",
        id: Self::MIAO_ID,
    };
    /// `SHARADA`.
    pub const SHARADA: UnicodeBlock = UnicodeBlock {
        name: "SHARADA",
        id: Self::SHARADA_ID,
    };
    /// `SORA_SOMPENG`.
    pub const SORA_SOMPENG: UnicodeBlock = UnicodeBlock {
        name: "SORA_SOMPENG",
        id: Self::SORA_SOMPENG_ID,
    };
    /// `SUNDANESE_SUPPLEMENT`.
    pub const SUNDANESE_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "SUNDANESE_SUPPLEMENT",
        id: Self::SUNDANESE_SUPPLEMENT_ID,
    };
    /// `TAKRI`.
    pub const TAKRI: UnicodeBlock = UnicodeBlock {
        name: "TAKRI",
        id: Self::TAKRI_ID,
    };
    /// `BASSA_VAH`.
    pub const BASSA_VAH: UnicodeBlock = UnicodeBlock {
        name: "BASSA_VAH",
        id: Self::BASSA_VAH_ID,
    };
    /// `CAUCASIAN_ALBANIAN`.
    pub const CAUCASIAN_ALBANIAN: UnicodeBlock = UnicodeBlock {
        name: "CAUCASIAN_ALBANIAN",
        id: Self::CAUCASIAN_ALBANIAN_ID,
    };
    /// `COPTIC_EPACT_NUMBERS`.
    pub const COPTIC_EPACT_NUMBERS: UnicodeBlock = UnicodeBlock {
        name: "COPTIC_EPACT_NUMBERS",
        id: Self::COPTIC_EPACT_NUMBERS_ID,
    };
    /// `COMBINING_DIACRITICAL_MARKS_EXTENDED`.
    pub const COMBINING_DIACRITICAL_MARKS_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "COMBINING_DIACRITICAL_MARKS_EXTENDED",
        id: Self::COMBINING_DIACRITICAL_MARKS_EXTENDED_ID,
    };
    /// `DUPLOYAN`.
    pub const DUPLOYAN: UnicodeBlock = UnicodeBlock {
        name: "DUPLOYAN",
        id: Self::DUPLOYAN_ID,
    };
    /// `ELBASAN`.
    pub const ELBASAN: UnicodeBlock = UnicodeBlock {
        name: "ELBASAN",
        id: Self::ELBASAN_ID,
    };
    /// `GEOMETRIC_SHAPES_EXTENDED`.
    pub const GEOMETRIC_SHAPES_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "GEOMETRIC_SHAPES_EXTENDED",
        id: Self::GEOMETRIC_SHAPES_EXTENDED_ID,
    };
    /// `GRANTHA`.
    pub const GRANTHA: UnicodeBlock = UnicodeBlock {
        name: "GRANTHA",
        id: Self::GRANTHA_ID,
    };
    /// `KHOJKI`.
    pub const KHOJKI: UnicodeBlock = UnicodeBlock {
        name: "KHOJKI",
        id: Self::KHOJKI_ID,
    };
    /// `KHUDAWADI`.
    pub const KHUDAWADI: UnicodeBlock = UnicodeBlock {
        name: "KHUDAWADI",
        id: Self::KHUDAWADI_ID,
    };
    /// `LATIN_EXTENDED_E`.
    pub const LATIN_EXTENDED_E: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_E",
        id: Self::LATIN_EXTENDED_E_ID,
    };
    /// `LINEAR_A`.
    pub const LINEAR_A: UnicodeBlock = UnicodeBlock {
        name: "LINEAR_A",
        id: Self::LINEAR_A_ID,
    };
    /// `MAHAJANI`.
    pub const MAHAJANI: UnicodeBlock = UnicodeBlock {
        name: "MAHAJANI",
        id: Self::MAHAJANI_ID,
    };
    /// `MANICHAEAN`.
    pub const MANICHAEAN: UnicodeBlock = UnicodeBlock {
        name: "MANICHAEAN",
        id: Self::MANICHAEAN_ID,
    };
    /// `MENDE_KIKAKUI`.
    pub const MENDE_KIKAKUI: UnicodeBlock = UnicodeBlock {
        name: "MENDE_KIKAKUI",
        id: Self::MENDE_KIKAKUI_ID,
    };
    /// `MODI`.
    pub const MODI: UnicodeBlock = UnicodeBlock {
        name: "MODI",
        id: Self::MODI_ID,
    };
    /// `MRO`.
    pub const MRO: UnicodeBlock = UnicodeBlock {
        name: "MRO",
        id: Self::MRO_ID,
    };
    /// `MYANMAR_EXTENDED_B`.
    pub const MYANMAR_EXTENDED_B: UnicodeBlock = UnicodeBlock {
        name: "MYANMAR_EXTENDED_B",
        id: Self::MYANMAR_EXTENDED_B_ID,
    };
    /// `NABATAEAN`.
    pub const NABATAEAN: UnicodeBlock = UnicodeBlock {
        name: "NABATAEAN",
        id: Self::NABATAEAN_ID,
    };
    /// `OLD_NORTH_ARABIAN`.
    pub const OLD_NORTH_ARABIAN: UnicodeBlock = UnicodeBlock {
        name: "OLD_NORTH_ARABIAN",
        id: Self::OLD_NORTH_ARABIAN_ID,
    };
    /// `OLD_PERMIC`.
    pub const OLD_PERMIC: UnicodeBlock = UnicodeBlock {
        name: "OLD_PERMIC",
        id: Self::OLD_PERMIC_ID,
    };
    /// `ORNAMENTAL_DINGBATS`.
    pub const ORNAMENTAL_DINGBATS: UnicodeBlock = UnicodeBlock {
        name: "ORNAMENTAL_DINGBATS",
        id: Self::ORNAMENTAL_DINGBATS_ID,
    };
    /// `PAHAWH_HMONG`.
    pub const PAHAWH_HMONG: UnicodeBlock = UnicodeBlock {
        name: "PAHAWH_HMONG",
        id: Self::PAHAWH_HMONG_ID,
    };
    /// `PALMYRENE`.
    pub const PALMYRENE: UnicodeBlock = UnicodeBlock {
        name: "PALMYRENE",
        id: Self::PALMYRENE_ID,
    };
    /// `PAU_CIN_HAU`.
    pub const PAU_CIN_HAU: UnicodeBlock = UnicodeBlock {
        name: "PAU_CIN_HAU",
        id: Self::PAU_CIN_HAU_ID,
    };
    /// `PSALTER_PAHLAVI`.
    pub const PSALTER_PAHLAVI: UnicodeBlock = UnicodeBlock {
        name: "PSALTER_PAHLAVI",
        id: Self::PSALTER_PAHLAVI_ID,
    };
    /// `SHORTHAND_FORMAT_CONTROLS`.
    pub const SHORTHAND_FORMAT_CONTROLS: UnicodeBlock = UnicodeBlock {
        name: "SHORTHAND_FORMAT_CONTROLS",
        id: Self::SHORTHAND_FORMAT_CONTROLS_ID,
    };
    /// `SIDDHAM`.
    pub const SIDDHAM: UnicodeBlock = UnicodeBlock {
        name: "SIDDHAM",
        id: Self::SIDDHAM_ID,
    };
    /// `SINHALA_ARCHAIC_NUMBERS`.
    pub const SINHALA_ARCHAIC_NUMBERS: UnicodeBlock = UnicodeBlock {
        name: "SINHALA_ARCHAIC_NUMBERS",
        id: Self::SINHALA_ARCHAIC_NUMBERS_ID,
    };
    /// `SUPPLEMENTAL_ARROWS_C`.
    pub const SUPPLEMENTAL_ARROWS_C: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTAL_ARROWS_C",
        id: Self::SUPPLEMENTAL_ARROWS_C_ID,
    };
    /// `TIRHUTA`.
    pub const TIRHUTA: UnicodeBlock = UnicodeBlock {
        name: "TIRHUTA",
        id: Self::TIRHUTA_ID,
    };
    /// `WARANG_CITI`.
    pub const WARANG_CITI: UnicodeBlock = UnicodeBlock {
        name: "WARANG_CITI",
        id: Self::WARANG_CITI_ID,
    };
    /// `AHOM`.
    pub const AHOM: UnicodeBlock = UnicodeBlock {
        name: "AHOM",
        id: Self::AHOM_ID,
    };
    /// `ANATOLIAN_HIEROGLYPHS`.
    pub const ANATOLIAN_HIEROGLYPHS: UnicodeBlock = UnicodeBlock {
        name: "ANATOLIAN_HIEROGLYPHS",
        id: Self::ANATOLIAN_HIEROGLYPHS_ID,
    };
    /// `CHEROKEE_SUPPLEMENT`.
    pub const CHEROKEE_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "CHEROKEE_SUPPLEMENT",
        id: Self::CHEROKEE_SUPPLEMENT_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_E`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_E: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_E",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_E_ID,
    };
    /// `EARLY_DYNASTIC_CUNEIFORM`.
    pub const EARLY_DYNASTIC_CUNEIFORM: UnicodeBlock = UnicodeBlock {
        name: "EARLY_DYNASTIC_CUNEIFORM",
        id: Self::EARLY_DYNASTIC_CUNEIFORM_ID,
    };
    /// `HATRAN`.
    pub const HATRAN: UnicodeBlock = UnicodeBlock {
        name: "HATRAN",
        id: Self::HATRAN_ID,
    };
    /// `MULTANI`.
    pub const MULTANI: UnicodeBlock = UnicodeBlock {
        name: "MULTANI",
        id: Self::MULTANI_ID,
    };
    /// `OLD_HUNGARIAN`.
    pub const OLD_HUNGARIAN: UnicodeBlock = UnicodeBlock {
        name: "OLD_HUNGARIAN",
        id: Self::OLD_HUNGARIAN_ID,
    };
    /// `SUPPLEMENTAL_SYMBOLS_AND_PICTOGRAPHS`.
    pub const SUPPLEMENTAL_SYMBOLS_AND_PICTOGRAPHS: UnicodeBlock = UnicodeBlock {
        name: "SUPPLEMENTAL_SYMBOLS_AND_PICTOGRAPHS",
        id: Self::SUPPLEMENTAL_SYMBOLS_AND_PICTOGRAPHS_ID,
    };
    /// `SUTTON_SIGNWRITING`.
    pub const SUTTON_SIGNWRITING: UnicodeBlock = UnicodeBlock {
        name: "SUTTON_SIGNWRITING",
        id: Self::SUTTON_SIGNWRITING_ID,
    };
    /// `ADLAM`.
    pub const ADLAM: UnicodeBlock = UnicodeBlock {
        name: "ADLAM",
        id: Self::ADLAM_ID,
    };
    /// `BHAIKSUKI`.
    pub const BHAIKSUKI: UnicodeBlock = UnicodeBlock {
        name: "BHAIKSUKI",
        id: Self::BHAIKSUKI_ID,
    };
    /// `CYRILLIC_EXTENDED_C`.
    pub const CYRILLIC_EXTENDED_C: UnicodeBlock = UnicodeBlock {
        name: "CYRILLIC_EXTENDED_C",
        id: Self::CYRILLIC_EXTENDED_C_ID,
    };
    /// `GLAGOLITIC_SUPPLEMENT`.
    pub const GLAGOLITIC_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "GLAGOLITIC_SUPPLEMENT",
        id: Self::GLAGOLITIC_SUPPLEMENT_ID,
    };
    /// `IDEOGRAPHIC_SYMBOLS_AND_PUNCTUATION`.
    pub const IDEOGRAPHIC_SYMBOLS_AND_PUNCTUATION: UnicodeBlock = UnicodeBlock {
        name: "IDEOGRAPHIC_SYMBOLS_AND_PUNCTUATION",
        id: Self::IDEOGRAPHIC_SYMBOLS_AND_PUNCTUATION_ID,
    };
    /// `MARCHEN`.
    pub const MARCHEN: UnicodeBlock = UnicodeBlock {
        name: "MARCHEN",
        id: Self::MARCHEN_ID,
    };
    /// `MONGOLIAN_SUPPLEMENT`.
    pub const MONGOLIAN_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "MONGOLIAN_SUPPLEMENT",
        id: Self::MONGOLIAN_SUPPLEMENT_ID,
    };
    /// `NEWA`.
    pub const NEWA: UnicodeBlock = UnicodeBlock {
        name: "NEWA",
        id: Self::NEWA_ID,
    };
    /// `OSAGE`.
    pub const OSAGE: UnicodeBlock = UnicodeBlock {
        name: "OSAGE",
        id: Self::OSAGE_ID,
    };
    /// `TANGUT`.
    pub const TANGUT: UnicodeBlock = UnicodeBlock {
        name: "TANGUT",
        id: Self::TANGUT_ID,
    };
    /// `TANGUT_COMPONENTS`.
    pub const TANGUT_COMPONENTS: UnicodeBlock = UnicodeBlock {
        name: "TANGUT_COMPONENTS",
        id: Self::TANGUT_COMPONENTS_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_F`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_F: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_F",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_F_ID,
    };
    /// `KANA_EXTENDED_A`.
    pub const KANA_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "KANA_EXTENDED_A",
        id: Self::KANA_EXTENDED_A_ID,
    };
    /// `MASARAM_GONDI`.
    pub const MASARAM_GONDI: UnicodeBlock = UnicodeBlock {
        name: "MASARAM_GONDI",
        id: Self::MASARAM_GONDI_ID,
    };
    /// `NUSHU`.
    pub const NUSHU: UnicodeBlock = UnicodeBlock {
        name: "NUSHU",
        id: Self::NUSHU_ID,
    };
    /// `SOYOMBO`.
    pub const SOYOMBO: UnicodeBlock = UnicodeBlock {
        name: "SOYOMBO",
        id: Self::SOYOMBO_ID,
    };
    /// `SYRIAC_SUPPLEMENT`.
    pub const SYRIAC_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "SYRIAC_SUPPLEMENT",
        id: Self::SYRIAC_SUPPLEMENT_ID,
    };
    /// `ZANABAZAR_SQUARE`.
    pub const ZANABAZAR_SQUARE: UnicodeBlock = UnicodeBlock {
        name: "ZANABAZAR_SQUARE",
        id: Self::ZANABAZAR_SQUARE_ID,
    };
    /// `CHESS_SYMBOLS`.
    pub const CHESS_SYMBOLS: UnicodeBlock = UnicodeBlock {
        name: "CHESS_SYMBOLS",
        id: Self::CHESS_SYMBOLS_ID,
    };
    /// `DOGRA`.
    pub const DOGRA: UnicodeBlock = UnicodeBlock {
        name: "DOGRA",
        id: Self::DOGRA_ID,
    };
    /// `GEORGIAN_EXTENDED`.
    pub const GEORGIAN_EXTENDED: UnicodeBlock = UnicodeBlock {
        name: "GEORGIAN_EXTENDED",
        id: Self::GEORGIAN_EXTENDED_ID,
    };
    /// `GUNJALA_GONDI`.
    pub const GUNJALA_GONDI: UnicodeBlock = UnicodeBlock {
        name: "GUNJALA_GONDI",
        id: Self::GUNJALA_GONDI_ID,
    };
    /// `HANIFI_ROHINGYA`.
    pub const HANIFI_ROHINGYA: UnicodeBlock = UnicodeBlock {
        name: "HANIFI_ROHINGYA",
        id: Self::HANIFI_ROHINGYA_ID,
    };
    /// `INDIC_SIYAQ_NUMBERS`.
    pub const INDIC_SIYAQ_NUMBERS: UnicodeBlock = UnicodeBlock {
        name: "INDIC_SIYAQ_NUMBERS",
        id: Self::INDIC_SIYAQ_NUMBERS_ID,
    };
    /// `MAKASAR`.
    pub const MAKASAR: UnicodeBlock = UnicodeBlock {
        name: "MAKASAR",
        id: Self::MAKASAR_ID,
    };
    /// `MAYAN_NUMERALS`.
    pub const MAYAN_NUMERALS: UnicodeBlock = UnicodeBlock {
        name: "MAYAN_NUMERALS",
        id: Self::MAYAN_NUMERALS_ID,
    };
    /// `MEDEFAIDRIN`.
    pub const MEDEFAIDRIN: UnicodeBlock = UnicodeBlock {
        name: "MEDEFAIDRIN",
        id: Self::MEDEFAIDRIN_ID,
    };
    /// `OLD_SOGDIAN`.
    pub const OLD_SOGDIAN: UnicodeBlock = UnicodeBlock {
        name: "OLD_SOGDIAN",
        id: Self::OLD_SOGDIAN_ID,
    };
    /// `SOGDIAN`.
    pub const SOGDIAN: UnicodeBlock = UnicodeBlock {
        name: "SOGDIAN",
        id: Self::SOGDIAN_ID,
    };
    /// `EGYPTIAN_HIEROGLYPH_FORMAT_CONTROLS`.
    pub const EGYPTIAN_HIEROGLYPH_FORMAT_CONTROLS: UnicodeBlock = UnicodeBlock {
        name: "EGYPTIAN_HIEROGLYPH_FORMAT_CONTROLS",
        id: Self::EGYPTIAN_HIEROGLYPH_FORMAT_CONTROLS_ID,
    };
    /// `ELYMAIC`.
    pub const ELYMAIC: UnicodeBlock = UnicodeBlock {
        name: "ELYMAIC",
        id: Self::ELYMAIC_ID,
    };
    /// `NANDINAGARI`.
    pub const NANDINAGARI: UnicodeBlock = UnicodeBlock {
        name: "NANDINAGARI",
        id: Self::NANDINAGARI_ID,
    };
    /// `NYIAKENG_PUACHUE_HMONG`.
    pub const NYIAKENG_PUACHUE_HMONG: UnicodeBlock = UnicodeBlock {
        name: "NYIAKENG_PUACHUE_HMONG",
        id: Self::NYIAKENG_PUACHUE_HMONG_ID,
    };
    /// `OTTOMAN_SIYAQ_NUMBERS`.
    pub const OTTOMAN_SIYAQ_NUMBERS: UnicodeBlock = UnicodeBlock {
        name: "OTTOMAN_SIYAQ_NUMBERS",
        id: Self::OTTOMAN_SIYAQ_NUMBERS_ID,
    };
    /// `SMALL_KANA_EXTENSION`.
    pub const SMALL_KANA_EXTENSION: UnicodeBlock = UnicodeBlock {
        name: "SMALL_KANA_EXTENSION",
        id: Self::SMALL_KANA_EXTENSION_ID,
    };
    /// `SYMBOLS_AND_PICTOGRAPHS_EXTENDED_A`.
    pub const SYMBOLS_AND_PICTOGRAPHS_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "SYMBOLS_AND_PICTOGRAPHS_EXTENDED_A",
        id: Self::SYMBOLS_AND_PICTOGRAPHS_EXTENDED_A_ID,
    };
    /// `TAMIL_SUPPLEMENT`.
    pub const TAMIL_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "TAMIL_SUPPLEMENT",
        id: Self::TAMIL_SUPPLEMENT_ID,
    };
    /// `WANCHO`.
    pub const WANCHO: UnicodeBlock = UnicodeBlock {
        name: "WANCHO",
        id: Self::WANCHO_ID,
    };
    /// `CHORASMIAN`.
    pub const CHORASMIAN: UnicodeBlock = UnicodeBlock {
        name: "CHORASMIAN",
        id: Self::CHORASMIAN_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_G`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_G: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_G",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_G_ID,
    };
    /// `DIVES_AKURU`.
    pub const DIVES_AKURU: UnicodeBlock = UnicodeBlock {
        name: "DIVES_AKURU",
        id: Self::DIVES_AKURU_ID,
    };
    /// `KHITAN_SMALL_SCRIPT`.
    pub const KHITAN_SMALL_SCRIPT: UnicodeBlock = UnicodeBlock {
        name: "KHITAN_SMALL_SCRIPT",
        id: Self::KHITAN_SMALL_SCRIPT_ID,
    };
    /// `LISU_SUPPLEMENT`.
    pub const LISU_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "LISU_SUPPLEMENT",
        id: Self::LISU_SUPPLEMENT_ID,
    };
    /// `SYMBOLS_FOR_LEGACY_COMPUTING`.
    pub const SYMBOLS_FOR_LEGACY_COMPUTING: UnicodeBlock = UnicodeBlock {
        name: "SYMBOLS_FOR_LEGACY_COMPUTING",
        id: Self::SYMBOLS_FOR_LEGACY_COMPUTING_ID,
    };
    /// `TANGUT_SUPPLEMENT`.
    pub const TANGUT_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "TANGUT_SUPPLEMENT",
        id: Self::TANGUT_SUPPLEMENT_ID,
    };
    /// `YEZIDI`.
    pub const YEZIDI: UnicodeBlock = UnicodeBlock {
        name: "YEZIDI",
        id: Self::YEZIDI_ID,
    };
    /// `ARABIC_EXTENDED_B`.
    pub const ARABIC_EXTENDED_B: UnicodeBlock = UnicodeBlock {
        name: "ARABIC_EXTENDED_B",
        id: Self::ARABIC_EXTENDED_B_ID,
    };
    /// `CYPRO_MINOAN`.
    pub const CYPRO_MINOAN: UnicodeBlock = UnicodeBlock {
        name: "CYPRO_MINOAN",
        id: Self::CYPRO_MINOAN_ID,
    };
    /// `ETHIOPIC_EXTENDED_B`.
    pub const ETHIOPIC_EXTENDED_B: UnicodeBlock = UnicodeBlock {
        name: "ETHIOPIC_EXTENDED_B",
        id: Self::ETHIOPIC_EXTENDED_B_ID,
    };
    /// `KANA_EXTENDED_B`.
    pub const KANA_EXTENDED_B: UnicodeBlock = UnicodeBlock {
        name: "KANA_EXTENDED_B",
        id: Self::KANA_EXTENDED_B_ID,
    };
    /// `LATIN_EXTENDED_F`.
    pub const LATIN_EXTENDED_F: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_F",
        id: Self::LATIN_EXTENDED_F_ID,
    };
    /// `LATIN_EXTENDED_G`.
    pub const LATIN_EXTENDED_G: UnicodeBlock = UnicodeBlock {
        name: "LATIN_EXTENDED_G",
        id: Self::LATIN_EXTENDED_G_ID,
    };
    /// `OLD_UYGHUR`.
    pub const OLD_UYGHUR: UnicodeBlock = UnicodeBlock {
        name: "OLD_UYGHUR",
        id: Self::OLD_UYGHUR_ID,
    };
    /// `TANGSA`.
    pub const TANGSA: UnicodeBlock = UnicodeBlock {
        name: "TANGSA",
        id: Self::TANGSA_ID,
    };
    /// `TOTO`.
    pub const TOTO: UnicodeBlock = UnicodeBlock {
        name: "TOTO",
        id: Self::TOTO_ID,
    };
    /// `UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_A`.
    pub const UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_A",
        id: Self::UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_A_ID,
    };
    /// `VITHKUQI`.
    pub const VITHKUQI: UnicodeBlock = UnicodeBlock {
        name: "VITHKUQI",
        id: Self::VITHKUQI_ID,
    };
    /// `ZNAMENNY_MUSICAL_NOTATION`.
    pub const ZNAMENNY_MUSICAL_NOTATION: UnicodeBlock = UnicodeBlock {
        name: "ZNAMENNY_MUSICAL_NOTATION",
        id: Self::ZNAMENNY_MUSICAL_NOTATION_ID,
    };
    /// `ARABIC_EXTENDED_C`.
    pub const ARABIC_EXTENDED_C: UnicodeBlock = UnicodeBlock {
        name: "ARABIC_EXTENDED_C",
        id: Self::ARABIC_EXTENDED_C_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_H`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_H: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_H",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_H_ID,
    };
    /// `CYRILLIC_EXTENDED_D`.
    pub const CYRILLIC_EXTENDED_D: UnicodeBlock = UnicodeBlock {
        name: "CYRILLIC_EXTENDED_D",
        id: Self::CYRILLIC_EXTENDED_D_ID,
    };
    /// `DEVANAGARI_EXTENDED_A`.
    pub const DEVANAGARI_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "DEVANAGARI_EXTENDED_A",
        id: Self::DEVANAGARI_EXTENDED_A_ID,
    };
    /// `KAKTOVIK_NUMERALS`.
    pub const KAKTOVIK_NUMERALS: UnicodeBlock = UnicodeBlock {
        name: "KAKTOVIK_NUMERALS",
        id: Self::KAKTOVIK_NUMERALS_ID,
    };
    /// `KAWI`.
    pub const KAWI: UnicodeBlock = UnicodeBlock {
        name: "KAWI",
        id: Self::KAWI_ID,
    };
    /// `NAG_MUNDARI`.
    pub const NAG_MUNDARI: UnicodeBlock = UnicodeBlock {
        name: "NAG_MUNDARI",
        id: Self::NAG_MUNDARI_ID,
    };
    /// `CJK_UNIFIED_IDEOGRAPHS_EXTENSION_I`.
    pub const CJK_UNIFIED_IDEOGRAPHS_EXTENSION_I: UnicodeBlock = UnicodeBlock {
        name: "CJK_UNIFIED_IDEOGRAPHS_EXTENSION_I",
        id: Self::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_I_ID,
    };
    /// `EGYPTIAN_HIEROGLYPHS_EXTENDED_A`.
    pub const EGYPTIAN_HIEROGLYPHS_EXTENDED_A: UnicodeBlock = UnicodeBlock {
        name: "EGYPTIAN_HIEROGLYPHS_EXTENDED_A",
        id: Self::EGYPTIAN_HIEROGLYPHS_EXTENDED_A_ID,
    };
    /// `GARAY`.
    pub const GARAY: UnicodeBlock = UnicodeBlock {
        name: "GARAY",
        id: Self::GARAY_ID,
    };
    /// `GURUNG_KHEMA`.
    pub const GURUNG_KHEMA: UnicodeBlock = UnicodeBlock {
        name: "GURUNG_KHEMA",
        id: Self::GURUNG_KHEMA_ID,
    };
    /// `KIRAT_RAI`.
    pub const KIRAT_RAI: UnicodeBlock = UnicodeBlock {
        name: "KIRAT_RAI",
        id: Self::KIRAT_RAI_ID,
    };
    /// `MYANMAR_EXTENDED_C`.
    pub const MYANMAR_EXTENDED_C: UnicodeBlock = UnicodeBlock {
        name: "MYANMAR_EXTENDED_C",
        id: Self::MYANMAR_EXTENDED_C_ID,
    };
    /// `OL_ONAL`.
    pub const OL_ONAL: UnicodeBlock = UnicodeBlock {
        name: "OL_ONAL",
        id: Self::OL_ONAL_ID,
    };
    /// `SUNUWAR`.
    pub const SUNUWAR: UnicodeBlock = UnicodeBlock {
        name: "SUNUWAR",
        id: Self::SUNUWAR_ID,
    };
    /// `SYMBOLS_FOR_LEGACY_COMPUTING_SUPPLEMENT`.
    pub const SYMBOLS_FOR_LEGACY_COMPUTING_SUPPLEMENT: UnicodeBlock = UnicodeBlock {
        name: "SYMBOLS_FOR_LEGACY_COMPUTING_SUPPLEMENT",
        id: Self::SYMBOLS_FOR_LEGACY_COMPUTING_SUPPLEMENT_ID,
    };
    /// `TODHRI`.
    pub const TODHRI: UnicodeBlock = UnicodeBlock {
        name: "TODHRI",
        id: Self::TODHRI_ID,
    };
    /// `TULU_TIGALARI`.
    pub const TULU_TIGALARI: UnicodeBlock = UnicodeBlock {
        name: "TULU_TIGALARI",
        id: Self::TULU_TIGALARI_ID,
    };
    /// `INVALID_CODE`.
    pub const INVALID_CODE: UnicodeBlock = UnicodeBlock {
        name: "INVALID_CODE",
        id: Self::INVALID_CODE_ID,
    };
}

/// `BLOCKS_`: the blocks by id.
static BLOCKS: [UnicodeBlock; 339] = [
    UnicodeBlock::NO_BLOCK,
    UnicodeBlock::BASIC_LATIN,
    UnicodeBlock::LATIN_1_SUPPLEMENT,
    UnicodeBlock::LATIN_EXTENDED_A,
    UnicodeBlock::LATIN_EXTENDED_B,
    UnicodeBlock::IPA_EXTENSIONS,
    UnicodeBlock::SPACING_MODIFIER_LETTERS,
    UnicodeBlock::COMBINING_DIACRITICAL_MARKS,
    UnicodeBlock::GREEK,
    UnicodeBlock::CYRILLIC,
    UnicodeBlock::ARMENIAN,
    UnicodeBlock::HEBREW,
    UnicodeBlock::ARABIC,
    UnicodeBlock::SYRIAC,
    UnicodeBlock::THAANA,
    UnicodeBlock::DEVANAGARI,
    UnicodeBlock::BENGALI,
    UnicodeBlock::GURMUKHI,
    UnicodeBlock::GUJARATI,
    UnicodeBlock::ORIYA,
    UnicodeBlock::TAMIL,
    UnicodeBlock::TELUGU,
    UnicodeBlock::KANNADA,
    UnicodeBlock::MALAYALAM,
    UnicodeBlock::SINHALA,
    UnicodeBlock::THAI,
    UnicodeBlock::LAO,
    UnicodeBlock::TIBETAN,
    UnicodeBlock::MYANMAR,
    UnicodeBlock::GEORGIAN,
    UnicodeBlock::HANGUL_JAMO,
    UnicodeBlock::ETHIOPIC,
    UnicodeBlock::CHEROKEE,
    UnicodeBlock::UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS,
    UnicodeBlock::OGHAM,
    UnicodeBlock::RUNIC,
    UnicodeBlock::KHMER,
    UnicodeBlock::MONGOLIAN,
    UnicodeBlock::LATIN_EXTENDED_ADDITIONAL,
    UnicodeBlock::GREEK_EXTENDED,
    UnicodeBlock::GENERAL_PUNCTUATION,
    UnicodeBlock::SUPERSCRIPTS_AND_SUBSCRIPTS,
    UnicodeBlock::CURRENCY_SYMBOLS,
    UnicodeBlock::COMBINING_MARKS_FOR_SYMBOLS,
    UnicodeBlock::LETTERLIKE_SYMBOLS,
    UnicodeBlock::NUMBER_FORMS,
    UnicodeBlock::ARROWS,
    UnicodeBlock::MATHEMATICAL_OPERATORS,
    UnicodeBlock::MISCELLANEOUS_TECHNICAL,
    UnicodeBlock::CONTROL_PICTURES,
    UnicodeBlock::OPTICAL_CHARACTER_RECOGNITION,
    UnicodeBlock::ENCLOSED_ALPHANUMERICS,
    UnicodeBlock::BOX_DRAWING,
    UnicodeBlock::BLOCK_ELEMENTS,
    UnicodeBlock::GEOMETRIC_SHAPES,
    UnicodeBlock::MISCELLANEOUS_SYMBOLS,
    UnicodeBlock::DINGBATS,
    UnicodeBlock::BRAILLE_PATTERNS,
    UnicodeBlock::CJK_RADICALS_SUPPLEMENT,
    UnicodeBlock::KANGXI_RADICALS,
    UnicodeBlock::IDEOGRAPHIC_DESCRIPTION_CHARACTERS,
    UnicodeBlock::CJK_SYMBOLS_AND_PUNCTUATION,
    UnicodeBlock::HIRAGANA,
    UnicodeBlock::KATAKANA,
    UnicodeBlock::BOPOMOFO,
    UnicodeBlock::HANGUL_COMPATIBILITY_JAMO,
    UnicodeBlock::KANBUN,
    UnicodeBlock::BOPOMOFO_EXTENDED,
    UnicodeBlock::ENCLOSED_CJK_LETTERS_AND_MONTHS,
    UnicodeBlock::CJK_COMPATIBILITY,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_A,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS,
    UnicodeBlock::YI_SYLLABLES,
    UnicodeBlock::YI_RADICALS,
    UnicodeBlock::HANGUL_SYLLABLES,
    UnicodeBlock::HIGH_SURROGATES,
    UnicodeBlock::HIGH_PRIVATE_USE_SURROGATES,
    UnicodeBlock::LOW_SURROGATES,
    UnicodeBlock::PRIVATE_USE_AREA,
    UnicodeBlock::CJK_COMPATIBILITY_IDEOGRAPHS,
    UnicodeBlock::ALPHABETIC_PRESENTATION_FORMS,
    UnicodeBlock::ARABIC_PRESENTATION_FORMS_A,
    UnicodeBlock::COMBINING_HALF_MARKS,
    UnicodeBlock::CJK_COMPATIBILITY_FORMS,
    UnicodeBlock::SMALL_FORM_VARIANTS,
    UnicodeBlock::ARABIC_PRESENTATION_FORMS_B,
    UnicodeBlock::SPECIALS,
    UnicodeBlock::HALFWIDTH_AND_FULLWIDTH_FORMS,
    UnicodeBlock::OLD_ITALIC,
    UnicodeBlock::GOTHIC,
    UnicodeBlock::DESERET,
    UnicodeBlock::BYZANTINE_MUSICAL_SYMBOLS,
    UnicodeBlock::MUSICAL_SYMBOLS,
    UnicodeBlock::MATHEMATICAL_ALPHANUMERIC_SYMBOLS,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_B,
    UnicodeBlock::CJK_COMPATIBILITY_IDEOGRAPHS_SUPPLEMENT,
    UnicodeBlock::TAGS,
    UnicodeBlock::CYRILLIC_SUPPLEMENT,
    UnicodeBlock::TAGALOG,
    UnicodeBlock::HANUNOO,
    UnicodeBlock::BUHID,
    UnicodeBlock::TAGBANWA,
    UnicodeBlock::MISCELLANEOUS_MATHEMATICAL_SYMBOLS_A,
    UnicodeBlock::SUPPLEMENTAL_ARROWS_A,
    UnicodeBlock::SUPPLEMENTAL_ARROWS_B,
    UnicodeBlock::MISCELLANEOUS_MATHEMATICAL_SYMBOLS_B,
    UnicodeBlock::SUPPLEMENTAL_MATHEMATICAL_OPERATORS,
    UnicodeBlock::KATAKANA_PHONETIC_EXTENSIONS,
    UnicodeBlock::VARIATION_SELECTORS,
    UnicodeBlock::SUPPLEMENTARY_PRIVATE_USE_AREA_A,
    UnicodeBlock::SUPPLEMENTARY_PRIVATE_USE_AREA_B,
    UnicodeBlock::LIMBU,
    UnicodeBlock::TAI_LE,
    UnicodeBlock::KHMER_SYMBOLS,
    UnicodeBlock::PHONETIC_EXTENSIONS,
    UnicodeBlock::MISCELLANEOUS_SYMBOLS_AND_ARROWS,
    UnicodeBlock::YIJING_HEXAGRAM_SYMBOLS,
    UnicodeBlock::LINEAR_B_SYLLABARY,
    UnicodeBlock::LINEAR_B_IDEOGRAMS,
    UnicodeBlock::AEGEAN_NUMBERS,
    UnicodeBlock::UGARITIC,
    UnicodeBlock::SHAVIAN,
    UnicodeBlock::OSMANYA,
    UnicodeBlock::CYPRIOT_SYLLABARY,
    UnicodeBlock::TAI_XUAN_JING_SYMBOLS,
    UnicodeBlock::VARIATION_SELECTORS_SUPPLEMENT,
    UnicodeBlock::ANCIENT_GREEK_MUSICAL_NOTATION,
    UnicodeBlock::ANCIENT_GREEK_NUMBERS,
    UnicodeBlock::ARABIC_SUPPLEMENT,
    UnicodeBlock::BUGINESE,
    UnicodeBlock::CJK_STROKES,
    UnicodeBlock::COMBINING_DIACRITICAL_MARKS_SUPPLEMENT,
    UnicodeBlock::COPTIC,
    UnicodeBlock::ETHIOPIC_EXTENDED,
    UnicodeBlock::ETHIOPIC_SUPPLEMENT,
    UnicodeBlock::GEORGIAN_SUPPLEMENT,
    UnicodeBlock::GLAGOLITIC,
    UnicodeBlock::KHAROSHTHI,
    UnicodeBlock::MODIFIER_TONE_LETTERS,
    UnicodeBlock::NEW_TAI_LUE,
    UnicodeBlock::OLD_PERSIAN,
    UnicodeBlock::PHONETIC_EXTENSIONS_SUPPLEMENT,
    UnicodeBlock::SUPPLEMENTAL_PUNCTUATION,
    UnicodeBlock::SYLOTI_NAGRI,
    UnicodeBlock::TIFINAGH,
    UnicodeBlock::VERTICAL_FORMS,
    UnicodeBlock::NKO,
    UnicodeBlock::BALINESE,
    UnicodeBlock::LATIN_EXTENDED_C,
    UnicodeBlock::LATIN_EXTENDED_D,
    UnicodeBlock::PHAGS_PA,
    UnicodeBlock::PHOENICIAN,
    UnicodeBlock::CUNEIFORM,
    UnicodeBlock::CUNEIFORM_NUMBERS_AND_PUNCTUATION,
    UnicodeBlock::COUNTING_ROD_NUMERALS,
    UnicodeBlock::SUNDANESE,
    UnicodeBlock::LEPCHA,
    UnicodeBlock::OL_CHIKI,
    UnicodeBlock::CYRILLIC_EXTENDED_A,
    UnicodeBlock::VAI,
    UnicodeBlock::CYRILLIC_EXTENDED_B,
    UnicodeBlock::SAURASHTRA,
    UnicodeBlock::KAYAH_LI,
    UnicodeBlock::REJANG,
    UnicodeBlock::CHAM,
    UnicodeBlock::ANCIENT_SYMBOLS,
    UnicodeBlock::PHAISTOS_DISC,
    UnicodeBlock::LYCIAN,
    UnicodeBlock::CARIAN,
    UnicodeBlock::LYDIAN,
    UnicodeBlock::MAHJONG_TILES,
    UnicodeBlock::DOMINO_TILES,
    UnicodeBlock::SAMARITAN,
    UnicodeBlock::UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED,
    UnicodeBlock::TAI_THAM,
    UnicodeBlock::VEDIC_EXTENSIONS,
    UnicodeBlock::LISU,
    UnicodeBlock::BAMUM,
    UnicodeBlock::COMMON_INDIC_NUMBER_FORMS,
    UnicodeBlock::DEVANAGARI_EXTENDED,
    UnicodeBlock::HANGUL_JAMO_EXTENDED_A,
    UnicodeBlock::JAVANESE,
    UnicodeBlock::MYANMAR_EXTENDED_A,
    UnicodeBlock::TAI_VIET,
    UnicodeBlock::MEETEI_MAYEK,
    UnicodeBlock::HANGUL_JAMO_EXTENDED_B,
    UnicodeBlock::IMPERIAL_ARAMAIC,
    UnicodeBlock::OLD_SOUTH_ARABIAN,
    UnicodeBlock::AVESTAN,
    UnicodeBlock::INSCRIPTIONAL_PARTHIAN,
    UnicodeBlock::INSCRIPTIONAL_PAHLAVI,
    UnicodeBlock::OLD_TURKIC,
    UnicodeBlock::RUMI_NUMERAL_SYMBOLS,
    UnicodeBlock::KAITHI,
    UnicodeBlock::EGYPTIAN_HIEROGLYPHS,
    UnicodeBlock::ENCLOSED_ALPHANUMERIC_SUPPLEMENT,
    UnicodeBlock::ENCLOSED_IDEOGRAPHIC_SUPPLEMENT,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_C,
    UnicodeBlock::MANDAIC,
    UnicodeBlock::BATAK,
    UnicodeBlock::ETHIOPIC_EXTENDED_A,
    UnicodeBlock::BRAHMI,
    UnicodeBlock::BAMUM_SUPPLEMENT,
    UnicodeBlock::KANA_SUPPLEMENT,
    UnicodeBlock::PLAYING_CARDS,
    UnicodeBlock::MISCELLANEOUS_SYMBOLS_AND_PICTOGRAPHS,
    UnicodeBlock::EMOTICONS,
    UnicodeBlock::TRANSPORT_AND_MAP_SYMBOLS,
    UnicodeBlock::ALCHEMICAL_SYMBOLS,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_D,
    UnicodeBlock::ARABIC_EXTENDED_A,
    UnicodeBlock::ARABIC_MATHEMATICAL_ALPHABETIC_SYMBOLS,
    UnicodeBlock::CHAKMA,
    UnicodeBlock::MEETEI_MAYEK_EXTENSIONS,
    UnicodeBlock::MEROITIC_CURSIVE,
    UnicodeBlock::MEROITIC_HIEROGLYPHS,
    UnicodeBlock::MIAO,
    UnicodeBlock::SHARADA,
    UnicodeBlock::SORA_SOMPENG,
    UnicodeBlock::SUNDANESE_SUPPLEMENT,
    UnicodeBlock::TAKRI,
    UnicodeBlock::BASSA_VAH,
    UnicodeBlock::CAUCASIAN_ALBANIAN,
    UnicodeBlock::COPTIC_EPACT_NUMBERS,
    UnicodeBlock::COMBINING_DIACRITICAL_MARKS_EXTENDED,
    UnicodeBlock::DUPLOYAN,
    UnicodeBlock::ELBASAN,
    UnicodeBlock::GEOMETRIC_SHAPES_EXTENDED,
    UnicodeBlock::GRANTHA,
    UnicodeBlock::KHOJKI,
    UnicodeBlock::KHUDAWADI,
    UnicodeBlock::LATIN_EXTENDED_E,
    UnicodeBlock::LINEAR_A,
    UnicodeBlock::MAHAJANI,
    UnicodeBlock::MANICHAEAN,
    UnicodeBlock::MENDE_KIKAKUI,
    UnicodeBlock::MODI,
    UnicodeBlock::MRO,
    UnicodeBlock::MYANMAR_EXTENDED_B,
    UnicodeBlock::NABATAEAN,
    UnicodeBlock::OLD_NORTH_ARABIAN,
    UnicodeBlock::OLD_PERMIC,
    UnicodeBlock::ORNAMENTAL_DINGBATS,
    UnicodeBlock::PAHAWH_HMONG,
    UnicodeBlock::PALMYRENE,
    UnicodeBlock::PAU_CIN_HAU,
    UnicodeBlock::PSALTER_PAHLAVI,
    UnicodeBlock::SHORTHAND_FORMAT_CONTROLS,
    UnicodeBlock::SIDDHAM,
    UnicodeBlock::SINHALA_ARCHAIC_NUMBERS,
    UnicodeBlock::SUPPLEMENTAL_ARROWS_C,
    UnicodeBlock::TIRHUTA,
    UnicodeBlock::WARANG_CITI,
    UnicodeBlock::AHOM,
    UnicodeBlock::ANATOLIAN_HIEROGLYPHS,
    UnicodeBlock::CHEROKEE_SUPPLEMENT,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_E,
    UnicodeBlock::EARLY_DYNASTIC_CUNEIFORM,
    UnicodeBlock::HATRAN,
    UnicodeBlock::MULTANI,
    UnicodeBlock::OLD_HUNGARIAN,
    UnicodeBlock::SUPPLEMENTAL_SYMBOLS_AND_PICTOGRAPHS,
    UnicodeBlock::SUTTON_SIGNWRITING,
    UnicodeBlock::ADLAM,
    UnicodeBlock::BHAIKSUKI,
    UnicodeBlock::CYRILLIC_EXTENDED_C,
    UnicodeBlock::GLAGOLITIC_SUPPLEMENT,
    UnicodeBlock::IDEOGRAPHIC_SYMBOLS_AND_PUNCTUATION,
    UnicodeBlock::MARCHEN,
    UnicodeBlock::MONGOLIAN_SUPPLEMENT,
    UnicodeBlock::NEWA,
    UnicodeBlock::OSAGE,
    UnicodeBlock::TANGUT,
    UnicodeBlock::TANGUT_COMPONENTS,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_F,
    UnicodeBlock::KANA_EXTENDED_A,
    UnicodeBlock::MASARAM_GONDI,
    UnicodeBlock::NUSHU,
    UnicodeBlock::SOYOMBO,
    UnicodeBlock::SYRIAC_SUPPLEMENT,
    UnicodeBlock::ZANABAZAR_SQUARE,
    UnicodeBlock::CHESS_SYMBOLS,
    UnicodeBlock::DOGRA,
    UnicodeBlock::GEORGIAN_EXTENDED,
    UnicodeBlock::GUNJALA_GONDI,
    UnicodeBlock::HANIFI_ROHINGYA,
    UnicodeBlock::INDIC_SIYAQ_NUMBERS,
    UnicodeBlock::MAKASAR,
    UnicodeBlock::MAYAN_NUMERALS,
    UnicodeBlock::MEDEFAIDRIN,
    UnicodeBlock::OLD_SOGDIAN,
    UnicodeBlock::SOGDIAN,
    UnicodeBlock::EGYPTIAN_HIEROGLYPH_FORMAT_CONTROLS,
    UnicodeBlock::ELYMAIC,
    UnicodeBlock::NANDINAGARI,
    UnicodeBlock::NYIAKENG_PUACHUE_HMONG,
    UnicodeBlock::OTTOMAN_SIYAQ_NUMBERS,
    UnicodeBlock::SMALL_KANA_EXTENSION,
    UnicodeBlock::SYMBOLS_AND_PICTOGRAPHS_EXTENDED_A,
    UnicodeBlock::TAMIL_SUPPLEMENT,
    UnicodeBlock::WANCHO,
    UnicodeBlock::CHORASMIAN,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_G,
    UnicodeBlock::DIVES_AKURU,
    UnicodeBlock::KHITAN_SMALL_SCRIPT,
    UnicodeBlock::LISU_SUPPLEMENT,
    UnicodeBlock::SYMBOLS_FOR_LEGACY_COMPUTING,
    UnicodeBlock::TANGUT_SUPPLEMENT,
    UnicodeBlock::YEZIDI,
    UnicodeBlock::ARABIC_EXTENDED_B,
    UnicodeBlock::CYPRO_MINOAN,
    UnicodeBlock::ETHIOPIC_EXTENDED_B,
    UnicodeBlock::KANA_EXTENDED_B,
    UnicodeBlock::LATIN_EXTENDED_F,
    UnicodeBlock::LATIN_EXTENDED_G,
    UnicodeBlock::OLD_UYGHUR,
    UnicodeBlock::TANGSA,
    UnicodeBlock::TOTO,
    UnicodeBlock::UNIFIED_CANADIAN_ABORIGINAL_SYLLABICS_EXTENDED_A,
    UnicodeBlock::VITHKUQI,
    UnicodeBlock::ZNAMENNY_MUSICAL_NOTATION,
    UnicodeBlock::ARABIC_EXTENDED_C,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_H,
    UnicodeBlock::CYRILLIC_EXTENDED_D,
    UnicodeBlock::DEVANAGARI_EXTENDED_A,
    UnicodeBlock::KAKTOVIK_NUMERALS,
    UnicodeBlock::KAWI,
    UnicodeBlock::NAG_MUNDARI,
    UnicodeBlock::CJK_UNIFIED_IDEOGRAPHS_EXTENSION_I,
    UnicodeBlock::EGYPTIAN_HIEROGLYPHS_EXTENDED_A,
    UnicodeBlock::GARAY,
    UnicodeBlock::GURUNG_KHEMA,
    UnicodeBlock::KIRAT_RAI,
    UnicodeBlock::MYANMAR_EXTENDED_C,
    UnicodeBlock::OL_ONAL,
    UnicodeBlock::SUNUWAR,
    UnicodeBlock::SYMBOLS_FOR_LEGACY_COMPUTING_SUPPLEMENT,
    UnicodeBlock::TODHRI,
    UnicodeBlock::TULU_TIGALARI,
];
/// Constants for the `EastAsianWidth` property.
///
/// Equivalent to the interface `UCharacter.EastAsianWidth`.
pub struct EastAsianWidth;

impl EastAsianWidth {
    /// `NEUTRAL`.
    pub const NEUTRAL: i32 = 0;
    /// `AMBIGUOUS`.
    pub const AMBIGUOUS: i32 = 1;
    /// `HALFWIDTH`.
    pub const HALFWIDTH: i32 = 2;
    /// `FULLWIDTH`.
    pub const FULLWIDTH: i32 = 3;
    /// `NARROW`.
    pub const NARROW: i32 = 4;
    /// `WIDE`.
    pub const WIDE: i32 = 5;
    /// `COUNT`.
    pub const COUNT: i32 = 6;
}

/// Constants for the `DecompositionType` property.
///
/// Equivalent to the interface `UCharacter.DecompositionType`.
pub struct DecompositionType;

impl DecompositionType {
    /// `NONE`.
    pub const NONE: i32 = 0;
    /// `CANONICAL`.
    pub const CANONICAL: i32 = 1;
    /// `COMPAT`.
    pub const COMPAT: i32 = 2;
    /// `CIRCLE`.
    pub const CIRCLE: i32 = 3;
    /// `FINAL`.
    pub const FINAL: i32 = 4;
    /// `FONT`.
    pub const FONT: i32 = 5;
    /// `FRACTION`.
    pub const FRACTION: i32 = 6;
    /// `INITIAL`.
    pub const INITIAL: i32 = 7;
    /// `ISOLATED`.
    pub const ISOLATED: i32 = 8;
    /// `MEDIAL`.
    pub const MEDIAL: i32 = 9;
    /// `NARROW`.
    pub const NARROW: i32 = 10;
    /// `NOBREAK`.
    pub const NOBREAK: i32 = 11;
    /// `SMALL`.
    pub const SMALL: i32 = 12;
    /// `SQUARE`.
    pub const SQUARE: i32 = 13;
    /// `SUB`.
    pub const SUB: i32 = 14;
    /// `SUPER`.
    pub const SUPER: i32 = 15;
    /// `VERTICAL`.
    pub const VERTICAL: i32 = 16;
    /// `WIDE`.
    pub const WIDE: i32 = 17;
    /// `COUNT`.
    pub const COUNT: i32 = 18;
}

/// Constants for the `JoiningType` property.
///
/// Equivalent to the interface `UCharacter.JoiningType`.
pub struct JoiningType;

impl JoiningType {
    /// `NON_JOINING`.
    pub const NON_JOINING: i32 = 0;
    /// `JOIN_CAUSING`.
    pub const JOIN_CAUSING: i32 = 1;
    /// `DUAL_JOINING`.
    pub const DUAL_JOINING: i32 = 2;
    /// `LEFT_JOINING`.
    pub const LEFT_JOINING: i32 = 3;
    /// `RIGHT_JOINING`.
    pub const RIGHT_JOINING: i32 = 4;
    /// `TRANSPARENT`.
    pub const TRANSPARENT: i32 = 5;
    /// `COUNT`.
    pub const COUNT: i32 = 6;
}

/// Constants for the `JoiningGroup` property.
///
/// Equivalent to the interface `UCharacter.JoiningGroup`.
pub struct JoiningGroup;

impl JoiningGroup {
    /// `NO_JOINING_GROUP`.
    pub const NO_JOINING_GROUP: i32 = 0;
    /// `AIN`.
    pub const AIN: i32 = 1;
    /// `ALAPH`.
    pub const ALAPH: i32 = 2;
    /// `ALEF`.
    pub const ALEF: i32 = 3;
    /// `BEH`.
    pub const BEH: i32 = 4;
    /// `BETH`.
    pub const BETH: i32 = 5;
    /// `DAL`.
    pub const DAL: i32 = 6;
    /// `DALATH_RISH`.
    pub const DALATH_RISH: i32 = 7;
    /// `E`.
    pub const E: i32 = 8;
    /// `FEH`.
    pub const FEH: i32 = 9;
    /// `FINAL_SEMKATH`.
    pub const FINAL_SEMKATH: i32 = 10;
    /// `GAF`.
    pub const GAF: i32 = 11;
    /// `GAMAL`.
    pub const GAMAL: i32 = 12;
    /// `HAH`.
    pub const HAH: i32 = 13;
    /// `TEH_MARBUTA_GOAL`.
    pub const TEH_MARBUTA_GOAL: i32 = 14;
    /// `HAMZA_ON_HEH_GOAL`.
    pub const HAMZA_ON_HEH_GOAL: i32 = Self::TEH_MARBUTA_GOAL;
    /// `HE`.
    pub const HE: i32 = 15;
    /// `HEH`.
    pub const HEH: i32 = 16;
    /// `HEH_GOAL`.
    pub const HEH_GOAL: i32 = 17;
    /// `HETH`.
    pub const HETH: i32 = 18;
    /// `KAF`.
    pub const KAF: i32 = 19;
    /// `KAPH`.
    pub const KAPH: i32 = 20;
    /// `KNOTTED_HEH`.
    pub const KNOTTED_HEH: i32 = 21;
    /// `LAM`.
    pub const LAM: i32 = 22;
    /// `LAMADH`.
    pub const LAMADH: i32 = 23;
    /// `MEEM`.
    pub const MEEM: i32 = 24;
    /// `MIM`.
    pub const MIM: i32 = 25;
    /// `NOON`.
    pub const NOON: i32 = 26;
    /// `NUN`.
    pub const NUN: i32 = 27;
    /// `PE`.
    pub const PE: i32 = 28;
    /// `QAF`.
    pub const QAF: i32 = 29;
    /// `QAPH`.
    pub const QAPH: i32 = 30;
    /// `REH`.
    pub const REH: i32 = 31;
    /// `REVERSED_PE`.
    pub const REVERSED_PE: i32 = 32;
    /// `SAD`.
    pub const SAD: i32 = 33;
    /// `SADHE`.
    pub const SADHE: i32 = 34;
    /// `SEEN`.
    pub const SEEN: i32 = 35;
    /// `SEMKATH`.
    pub const SEMKATH: i32 = 36;
    /// `SHIN`.
    pub const SHIN: i32 = 37;
    /// `SWASH_KAF`.
    pub const SWASH_KAF: i32 = 38;
    /// `SYRIAC_WAW`.
    pub const SYRIAC_WAW: i32 = 39;
    /// `TAH`.
    pub const TAH: i32 = 40;
    /// `TAW`.
    pub const TAW: i32 = 41;
    /// `TEH_MARBUTA`.
    pub const TEH_MARBUTA: i32 = 42;
    /// `TETH`.
    pub const TETH: i32 = 43;
    /// `WAW`.
    pub const WAW: i32 = 44;
    /// `YEH`.
    pub const YEH: i32 = 45;
    /// `YEH_BARREE`.
    pub const YEH_BARREE: i32 = 46;
    /// `YEH_WITH_TAIL`.
    pub const YEH_WITH_TAIL: i32 = 47;
    /// `YUDH`.
    pub const YUDH: i32 = 48;
    /// `YUDH_HE`.
    pub const YUDH_HE: i32 = 49;
    /// `ZAIN`.
    pub const ZAIN: i32 = 50;
    /// `FE`.
    pub const FE: i32 = 51;
    /// `KHAPH`.
    pub const KHAPH: i32 = 52;
    /// `ZHAIN`.
    pub const ZHAIN: i32 = 53;
    /// `BURUSHASKI_YEH_BARREE`.
    pub const BURUSHASKI_YEH_BARREE: i32 = 54;
    /// `FARSI_YEH`.
    pub const FARSI_YEH: i32 = 55;
    /// `NYA`.
    pub const NYA: i32 = 56;
    /// `ROHINGYA_YEH`.
    pub const ROHINGYA_YEH: i32 = 57;
    /// `MANICHAEAN_ALEPH`.
    pub const MANICHAEAN_ALEPH: i32 = 58;
    /// `MANICHAEAN_AYIN`.
    pub const MANICHAEAN_AYIN: i32 = 59;
    /// `MANICHAEAN_BETH`.
    pub const MANICHAEAN_BETH: i32 = 60;
    /// `MANICHAEAN_DALETH`.
    pub const MANICHAEAN_DALETH: i32 = 61;
    /// `MANICHAEAN_DHAMEDH`.
    pub const MANICHAEAN_DHAMEDH: i32 = 62;
    /// `MANICHAEAN_FIVE`.
    pub const MANICHAEAN_FIVE: i32 = 63;
    /// `MANICHAEAN_GIMEL`.
    pub const MANICHAEAN_GIMEL: i32 = 64;
    /// `MANICHAEAN_HETH`.
    pub const MANICHAEAN_HETH: i32 = 65;
    /// `MANICHAEAN_HUNDRED`.
    pub const MANICHAEAN_HUNDRED: i32 = 66;
    /// `MANICHAEAN_KAPH`.
    pub const MANICHAEAN_KAPH: i32 = 67;
    /// `MANICHAEAN_LAMEDH`.
    pub const MANICHAEAN_LAMEDH: i32 = 68;
    /// `MANICHAEAN_MEM`.
    pub const MANICHAEAN_MEM: i32 = 69;
    /// `MANICHAEAN_NUN`.
    pub const MANICHAEAN_NUN: i32 = 70;
    /// `MANICHAEAN_ONE`.
    pub const MANICHAEAN_ONE: i32 = 71;
    /// `MANICHAEAN_PE`.
    pub const MANICHAEAN_PE: i32 = 72;
    /// `MANICHAEAN_QOPH`.
    pub const MANICHAEAN_QOPH: i32 = 73;
    /// `MANICHAEAN_RESH`.
    pub const MANICHAEAN_RESH: i32 = 74;
    /// `MANICHAEAN_SADHE`.
    pub const MANICHAEAN_SADHE: i32 = 75;
    /// `MANICHAEAN_SAMEKH`.
    pub const MANICHAEAN_SAMEKH: i32 = 76;
    /// `MANICHAEAN_TAW`.
    pub const MANICHAEAN_TAW: i32 = 77;
    /// `MANICHAEAN_TEN`.
    pub const MANICHAEAN_TEN: i32 = 78;
    /// `MANICHAEAN_TETH`.
    pub const MANICHAEAN_TETH: i32 = 79;
    /// `MANICHAEAN_THAMEDH`.
    pub const MANICHAEAN_THAMEDH: i32 = 80;
    /// `MANICHAEAN_TWENTY`.
    pub const MANICHAEAN_TWENTY: i32 = 81;
    /// `MANICHAEAN_WAW`.
    pub const MANICHAEAN_WAW: i32 = 82;
    /// `MANICHAEAN_YODH`.
    pub const MANICHAEAN_YODH: i32 = 83;
    /// `MANICHAEAN_ZAYIN`.
    pub const MANICHAEAN_ZAYIN: i32 = 84;
    /// `STRAIGHT_WAW`.
    pub const STRAIGHT_WAW: i32 = 85;
    /// `AFRICAN_FEH`.
    pub const AFRICAN_FEH: i32 = 86;
    /// `AFRICAN_NOON`.
    pub const AFRICAN_NOON: i32 = 87;
    /// `AFRICAN_QAF`.
    pub const AFRICAN_QAF: i32 = 88;
    /// `MALAYALAM_BHA`.
    pub const MALAYALAM_BHA: i32 = 89;
    /// `MALAYALAM_JA`.
    pub const MALAYALAM_JA: i32 = 90;
    /// `MALAYALAM_LLA`.
    pub const MALAYALAM_LLA: i32 = 91;
    /// `MALAYALAM_LLLA`.
    pub const MALAYALAM_LLLA: i32 = 92;
    /// `MALAYALAM_NGA`.
    pub const MALAYALAM_NGA: i32 = 93;
    /// `MALAYALAM_NNA`.
    pub const MALAYALAM_NNA: i32 = 94;
    /// `MALAYALAM_NNNA`.
    pub const MALAYALAM_NNNA: i32 = 95;
    /// `MALAYALAM_NYA`.
    pub const MALAYALAM_NYA: i32 = 96;
    /// `MALAYALAM_RA`.
    pub const MALAYALAM_RA: i32 = 97;
    /// `MALAYALAM_SSA`.
    pub const MALAYALAM_SSA: i32 = 98;
    /// `MALAYALAM_TTA`.
    pub const MALAYALAM_TTA: i32 = 99;
    /// `HANIFI_ROHINGYA_KINNA_YA`.
    pub const HANIFI_ROHINGYA_KINNA_YA: i32 = 100;
    /// `HANIFI_ROHINGYA_PA`.
    pub const HANIFI_ROHINGYA_PA: i32 = 101;
    /// `THIN_YEH`.
    pub const THIN_YEH: i32 = 102;
    /// `VERTICAL_TAIL`.
    pub const VERTICAL_TAIL: i32 = 103;
    /// `KASHMIRI_YEH`.
    pub const KASHMIRI_YEH: i32 = 104;
    /// `COUNT`.
    pub const COUNT: i32 = 105;
}

/// Constants for the `GraphemeClusterBreak` property.
///
/// Equivalent to the interface `UCharacter.GraphemeClusterBreak`.
pub struct GraphemeClusterBreak;

impl GraphemeClusterBreak {
    /// `OTHER`.
    pub const OTHER: i32 = 0;
    /// `CONTROL`.
    pub const CONTROL: i32 = 1;
    /// `CR`.
    pub const CR: i32 = 2;
    /// `EXTEND`.
    pub const EXTEND: i32 = 3;
    /// `L`.
    pub const L: i32 = 4;
    /// `LF`.
    pub const LF: i32 = 5;
    /// `LV`.
    pub const LV: i32 = 6;
    /// `LVT`.
    pub const LVT: i32 = 7;
    /// `T`.
    pub const T: i32 = 8;
    /// `V`.
    pub const V: i32 = 9;
    /// `SPACING_MARK`.
    pub const SPACING_MARK: i32 = 10;
    /// `PREPEND`.
    pub const PREPEND: i32 = 11;
    /// `REGIONAL_INDICATOR`.
    pub const REGIONAL_INDICATOR: i32 = 12;
    /// `E_BASE`.
    pub const E_BASE: i32 = 13;
    /// `E_BASE_GAZ`.
    pub const E_BASE_GAZ: i32 = 14;
    /// `E_MODIFIER`.
    pub const E_MODIFIER: i32 = 15;
    /// `GLUE_AFTER_ZWJ`.
    pub const GLUE_AFTER_ZWJ: i32 = 16;
    /// `ZWJ`.
    pub const ZWJ: i32 = 17;
    /// `COUNT`.
    pub const COUNT: i32 = 18;
}

/// Constants for the `WordBreak` property.
///
/// Equivalent to the interface `UCharacter.WordBreak`.
pub struct WordBreak;

impl WordBreak {
    /// `OTHER`.
    pub const OTHER: i32 = 0;
    /// `ALETTER`.
    pub const ALETTER: i32 = 1;
    /// `FORMAT`.
    pub const FORMAT: i32 = 2;
    /// `KATAKANA`.
    pub const KATAKANA: i32 = 3;
    /// `MIDLETTER`.
    pub const MIDLETTER: i32 = 4;
    /// `MIDNUM`.
    pub const MIDNUM: i32 = 5;
    /// `NUMERIC`.
    pub const NUMERIC: i32 = 6;
    /// `EXTENDNUMLET`.
    pub const EXTENDNUMLET: i32 = 7;
    /// `CR`.
    pub const CR: i32 = 8;
    /// `EXTEND`.
    pub const EXTEND: i32 = 9;
    /// `LF`.
    pub const LF: i32 = 10;
    /// `MIDNUMLET`.
    pub const MIDNUMLET: i32 = 11;
    /// `NEWLINE`.
    pub const NEWLINE: i32 = 12;
    /// `REGIONAL_INDICATOR`.
    pub const REGIONAL_INDICATOR: i32 = 13;
    /// `HEBREW_LETTER`.
    pub const HEBREW_LETTER: i32 = 14;
    /// `SINGLE_QUOTE`.
    pub const SINGLE_QUOTE: i32 = 15;
    /// `DOUBLE_QUOTE`.
    pub const DOUBLE_QUOTE: i32 = 16;
    /// `E_BASE`.
    pub const E_BASE: i32 = 17;
    /// `E_BASE_GAZ`.
    pub const E_BASE_GAZ: i32 = 18;
    /// `E_MODIFIER`.
    pub const E_MODIFIER: i32 = 19;
    /// `GLUE_AFTER_ZWJ`.
    pub const GLUE_AFTER_ZWJ: i32 = 20;
    /// `ZWJ`.
    pub const ZWJ: i32 = 21;
    /// `WSEGSPACE`.
    pub const WSEGSPACE: i32 = 22;
    /// `COUNT`.
    pub const COUNT: i32 = 23;
}

/// Constants for the `SentenceBreak` property.
///
/// Equivalent to the interface `UCharacter.SentenceBreak`.
pub struct SentenceBreak;

impl SentenceBreak {
    /// `OTHER`.
    pub const OTHER: i32 = 0;
    /// `ATERM`.
    pub const ATERM: i32 = 1;
    /// `CLOSE`.
    pub const CLOSE: i32 = 2;
    /// `FORMAT`.
    pub const FORMAT: i32 = 3;
    /// `LOWER`.
    pub const LOWER: i32 = 4;
    /// `NUMERIC`.
    pub const NUMERIC: i32 = 5;
    /// `OLETTER`.
    pub const OLETTER: i32 = 6;
    /// `SEP`.
    pub const SEP: i32 = 7;
    /// `SP`.
    pub const SP: i32 = 8;
    /// `STERM`.
    pub const STERM: i32 = 9;
    /// `UPPER`.
    pub const UPPER: i32 = 10;
    /// `CR`.
    pub const CR: i32 = 11;
    /// `EXTEND`.
    pub const EXTEND: i32 = 12;
    /// `LF`.
    pub const LF: i32 = 13;
    /// `SCONTINUE`.
    pub const SCONTINUE: i32 = 14;
    /// `COUNT`.
    pub const COUNT: i32 = 15;
}

/// Constants for the `LineBreak` property.
///
/// Equivalent to the interface `UCharacter.LineBreak`.
pub struct LineBreak;

impl LineBreak {
    /// `UNKNOWN`.
    pub const UNKNOWN: i32 = 0;
    /// `AMBIGUOUS`.
    pub const AMBIGUOUS: i32 = 1;
    /// `ALPHABETIC`.
    pub const ALPHABETIC: i32 = 2;
    /// `BREAK_BOTH`.
    pub const BREAK_BOTH: i32 = 3;
    /// `BREAK_AFTER`.
    pub const BREAK_AFTER: i32 = 4;
    /// `BREAK_BEFORE`.
    pub const BREAK_BEFORE: i32 = 5;
    /// `MANDATORY_BREAK`.
    pub const MANDATORY_BREAK: i32 = 6;
    /// `CONTINGENT_BREAK`.
    pub const CONTINGENT_BREAK: i32 = 7;
    /// `CLOSE_PUNCTUATION`.
    pub const CLOSE_PUNCTUATION: i32 = 8;
    /// `COMBINING_MARK`.
    pub const COMBINING_MARK: i32 = 9;
    /// `CARRIAGE_RETURN`.
    pub const CARRIAGE_RETURN: i32 = 10;
    /// `EXCLAMATION`.
    pub const EXCLAMATION: i32 = 11;
    /// `GLUE`.
    pub const GLUE: i32 = 12;
    /// `HYPHEN`.
    pub const HYPHEN: i32 = 13;
    /// `IDEOGRAPHIC`.
    pub const IDEOGRAPHIC: i32 = 14;
    /// `INSEPERABLE`.
    pub const INSEPERABLE: i32 = 15;
    /// `INSEPARABLE`.
    pub const INSEPARABLE: i32 = 15;
    /// `INFIX_NUMERIC`.
    pub const INFIX_NUMERIC: i32 = 16;
    /// `LINE_FEED`.
    pub const LINE_FEED: i32 = 17;
    /// `NONSTARTER`.
    pub const NONSTARTER: i32 = 18;
    /// `NUMERIC`.
    pub const NUMERIC: i32 = 19;
    /// `OPEN_PUNCTUATION`.
    pub const OPEN_PUNCTUATION: i32 = 20;
    /// `POSTFIX_NUMERIC`.
    pub const POSTFIX_NUMERIC: i32 = 21;
    /// `PREFIX_NUMERIC`.
    pub const PREFIX_NUMERIC: i32 = 22;
    /// `QUOTATION`.
    pub const QUOTATION: i32 = 23;
    /// `COMPLEX_CONTEXT`.
    pub const COMPLEX_CONTEXT: i32 = 24;
    /// `SURROGATE`.
    pub const SURROGATE: i32 = 25;
    /// `SPACE`.
    pub const SPACE: i32 = 26;
    /// `BREAK_SYMBOLS`.
    pub const BREAK_SYMBOLS: i32 = 27;
    /// `ZWSPACE`.
    pub const ZWSPACE: i32 = 28;
    /// `NEXT_LINE`.
    pub const NEXT_LINE: i32 = 29;
    /// `WORD_JOINER`.
    pub const WORD_JOINER: i32 = 30;
    /// `H2`.
    pub const H2: i32 = 31;
    /// `H3`.
    pub const H3: i32 = 32;
    /// `JL`.
    pub const JL: i32 = 33;
    /// `JT`.
    pub const JT: i32 = 34;
    /// `JV`.
    pub const JV: i32 = 35;
    /// `CLOSE_PARENTHESIS`.
    pub const CLOSE_PARENTHESIS: i32 = 36;
    /// `CONDITIONAL_JAPANESE_STARTER`.
    pub const CONDITIONAL_JAPANESE_STARTER: i32 = 37;
    /// `HEBREW_LETTER`.
    pub const HEBREW_LETTER: i32 = 38;
    /// `REGIONAL_INDICATOR`.
    pub const REGIONAL_INDICATOR: i32 = 39;
    /// `E_BASE`.
    pub const E_BASE: i32 = 40;
    /// `E_MODIFIER`.
    pub const E_MODIFIER: i32 = 41;
    /// `ZWJ`.
    pub const ZWJ: i32 = 42;
    /// `AKSARA`.
    pub const AKSARA: i32 = 43;
    /// `AKSARA_PREBASE`.
    pub const AKSARA_PREBASE: i32 = 44;
    /// `AKSARA_START`.
    pub const AKSARA_START: i32 = 45;
    /// `VIRAMA_FINAL`.
    pub const VIRAMA_FINAL: i32 = 46;
    /// `VIRAMA`.
    pub const VIRAMA: i32 = 47;
    /// `COUNT`.
    pub const COUNT: i32 = 48;
}

/// Constants for the `NumericType` property.
///
/// Equivalent to the interface `UCharacter.NumericType`.
pub struct NumericType;

impl NumericType {
    /// `NONE`.
    pub const NONE: i32 = 0;
    /// `DECIMAL`.
    pub const DECIMAL: i32 = 1;
    /// `DIGIT`.
    pub const DIGIT: i32 = 2;
    /// `NUMERIC`.
    pub const NUMERIC: i32 = 3;
    /// `COUNT`.
    pub const COUNT: i32 = 4;
}

/// Constants for the `HangulSyllableType` property.
///
/// Equivalent to the interface `UCharacter.HangulSyllableType`.
pub struct HangulSyllableType;

impl HangulSyllableType {
    /// `NOT_APPLICABLE`.
    pub const NOT_APPLICABLE: i32 = 0;
    /// `LEADING_JAMO`.
    pub const LEADING_JAMO: i32 = 1;
    /// `VOWEL_JAMO`.
    pub const VOWEL_JAMO: i32 = 2;
    /// `TRAILING_JAMO`.
    pub const TRAILING_JAMO: i32 = 3;
    /// `LV_SYLLABLE`.
    pub const LV_SYLLABLE: i32 = 4;
    /// `LVT_SYLLABLE`.
    pub const LVT_SYLLABLE: i32 = 5;
    /// `COUNT`.
    pub const COUNT: i32 = 6;
}

/// Constants for the `BidiPairedBracketType` property.
///
/// Equivalent to the interface `UCharacter.BidiPairedBracketType`.
pub struct BidiPairedBracketType;

impl BidiPairedBracketType {
    /// `NONE`.
    pub const NONE: i32 = 0;
    /// `OPEN`.
    pub const OPEN: i32 = 1;
    /// `CLOSE`.
    pub const CLOSE: i32 = 2;
    /// `COUNT`.
    pub const COUNT: i32 = 3;
}

/// Constants for the `IndicPositionalCategory` property.
///
/// Equivalent to the interface `UCharacter.IndicPositionalCategory`.
pub struct IndicPositionalCategory;

impl IndicPositionalCategory {
    /// `NA`.
    pub const NA: i32 = 0;
    /// `BOTTOM`.
    pub const BOTTOM: i32 = 1;
    /// `BOTTOM_AND_LEFT`.
    pub const BOTTOM_AND_LEFT: i32 = 2;
    /// `BOTTOM_AND_RIGHT`.
    pub const BOTTOM_AND_RIGHT: i32 = 3;
    /// `LEFT`.
    pub const LEFT: i32 = 4;
    /// `LEFT_AND_RIGHT`.
    pub const LEFT_AND_RIGHT: i32 = 5;
    /// `OVERSTRUCK`.
    pub const OVERSTRUCK: i32 = 6;
    /// `RIGHT`.
    pub const RIGHT: i32 = 7;
    /// `TOP`.
    pub const TOP: i32 = 8;
    /// `TOP_AND_BOTTOM`.
    pub const TOP_AND_BOTTOM: i32 = 9;
    /// `TOP_AND_BOTTOM_AND_RIGHT`.
    pub const TOP_AND_BOTTOM_AND_RIGHT: i32 = 10;
    /// `TOP_AND_LEFT`.
    pub const TOP_AND_LEFT: i32 = 11;
    /// `TOP_AND_LEFT_AND_RIGHT`.
    pub const TOP_AND_LEFT_AND_RIGHT: i32 = 12;
    /// `TOP_AND_RIGHT`.
    pub const TOP_AND_RIGHT: i32 = 13;
    /// `VISUAL_ORDER_LEFT`.
    pub const VISUAL_ORDER_LEFT: i32 = 14;
    /// `TOP_AND_BOTTOM_AND_LEFT`.
    pub const TOP_AND_BOTTOM_AND_LEFT: i32 = 15;
}

/// Constants for the `IndicSyllabicCategory` property.
///
/// Equivalent to the interface `UCharacter.IndicSyllabicCategory`.
pub struct IndicSyllabicCategory;

impl IndicSyllabicCategory {
    /// `OTHER`.
    pub const OTHER: i32 = 0;
    /// `AVAGRAHA`.
    pub const AVAGRAHA: i32 = 1;
    /// `BINDU`.
    pub const BINDU: i32 = 2;
    /// `BRAHMI_JOINING_NUMBER`.
    pub const BRAHMI_JOINING_NUMBER: i32 = 3;
    /// `CANTILLATION_MARK`.
    pub const CANTILLATION_MARK: i32 = 4;
    /// `CONSONANT`.
    pub const CONSONANT: i32 = 5;
    /// `CONSONANT_DEAD`.
    pub const CONSONANT_DEAD: i32 = 6;
    /// `CONSONANT_FINAL`.
    pub const CONSONANT_FINAL: i32 = 7;
    /// `CONSONANT_HEAD_LETTER`.
    pub const CONSONANT_HEAD_LETTER: i32 = 8;
    /// `CONSONANT_INITIAL_POSTFIXED`.
    pub const CONSONANT_INITIAL_POSTFIXED: i32 = 9;
    /// `CONSONANT_KILLER`.
    pub const CONSONANT_KILLER: i32 = 10;
    /// `CONSONANT_MEDIAL`.
    pub const CONSONANT_MEDIAL: i32 = 11;
    /// `CONSONANT_PLACEHOLDER`.
    pub const CONSONANT_PLACEHOLDER: i32 = 12;
    /// `CONSONANT_PRECEDING_REPHA`.
    pub const CONSONANT_PRECEDING_REPHA: i32 = 13;
    /// `CONSONANT_PREFIXED`.
    pub const CONSONANT_PREFIXED: i32 = 14;
    /// `CONSONANT_SUBJOINED`.
    pub const CONSONANT_SUBJOINED: i32 = 15;
    /// `CONSONANT_SUCCEEDING_REPHA`.
    pub const CONSONANT_SUCCEEDING_REPHA: i32 = 16;
    /// `CONSONANT_WITH_STACKER`.
    pub const CONSONANT_WITH_STACKER: i32 = 17;
    /// `GEMINATION_MARK`.
    pub const GEMINATION_MARK: i32 = 18;
    /// `INVISIBLE_STACKER`.
    pub const INVISIBLE_STACKER: i32 = 19;
    /// `JOINER`.
    pub const JOINER: i32 = 20;
    /// `MODIFYING_LETTER`.
    pub const MODIFYING_LETTER: i32 = 21;
    /// `NON_JOINER`.
    pub const NON_JOINER: i32 = 22;
    /// `NUKTA`.
    pub const NUKTA: i32 = 23;
    /// `NUMBER`.
    pub const NUMBER: i32 = 24;
    /// `NUMBER_JOINER`.
    pub const NUMBER_JOINER: i32 = 25;
    /// `PURE_KILLER`.
    pub const PURE_KILLER: i32 = 26;
    /// `REGISTER_SHIFTER`.
    pub const REGISTER_SHIFTER: i32 = 27;
    /// `SYLLABLE_MODIFIER`.
    pub const SYLLABLE_MODIFIER: i32 = 28;
    /// `TONE_LETTER`.
    pub const TONE_LETTER: i32 = 29;
    /// `TONE_MARK`.
    pub const TONE_MARK: i32 = 30;
    /// `VIRAMA`.
    pub const VIRAMA: i32 = 31;
    /// `VISARGA`.
    pub const VISARGA: i32 = 32;
    /// `VOWEL`.
    pub const VOWEL: i32 = 33;
    /// `VOWEL_DEPENDENT`.
    pub const VOWEL_DEPENDENT: i32 = 34;
    /// `VOWEL_INDEPENDENT`.
    pub const VOWEL_INDEPENDENT: i32 = 35;
    /// `REORDERING_KILLER`.
    pub const REORDERING_KILLER: i32 = 36;
}

/// Constants for the `VerticalOrientation` property.
///
/// Equivalent to the interface `UCharacter.VerticalOrientation`.
pub struct VerticalOrientation;

impl VerticalOrientation {
    /// `ROTATED`.
    pub const ROTATED: i32 = 0;
    /// `TRANSFORMED_ROTATED`.
    pub const TRANSFORMED_ROTATED: i32 = 1;
    /// `TRANSFORMED_UPRIGHT`.
    pub const TRANSFORMED_UPRIGHT: i32 = 2;
    /// `UPRIGHT`.
    pub const UPRIGHT: i32 = 3;
}

/// Indic_Conjunct_Break constants.
///
/// Equivalent to the enum `UCharacter.IndicConjunctBreak`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IndicConjunctBreak {
    /// `NONE`.
    None,
    /// `CONSONANT`.
    Consonant,
    /// `EXTEND`.
    Extend,
    /// `LINKER`.
    Linker,
}

/// Identifier_Status constants (UTS #39).
///
/// Equivalent to the enum `UCharacter.IdentifierStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IdentifierStatus {
    /// `RESTRICTED`.
    Restricted,
    /// `ALLOWED`.
    Allowed,
}

impl IdentifierStatus {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }
}

/// Identifier_Type constants (UTS #39).
///
/// Equivalent to the enum `UCharacter.IdentifierType`; an
/// `EnumSet<IdentifierType>` is a `BTreeSet` (ordinal order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IdentifierType {
    /// `NOT_CHARACTER`.
    NotCharacter,
    /// `DEPRECATED`.
    Deprecated,
    /// `DEFAULT_IGNORABLE`.
    DefaultIgnorable,
    /// `NOT_NFKC`.
    NotNfkc,
    /// `NOT_XID`.
    NotXid,
    /// `EXCLUSION`.
    Exclusion,
    /// `OBSOLETE`.
    Obsolete,
    /// `TECHNICAL`.
    Technical,
    /// `UNCOMMON_USE`.
    UncommonUse,
    /// `LIMITED_USE`.
    LimitedUse,
    /// `INCLUSION`.
    Inclusion,
    /// `RECOMMENDED`.
    Recommended,
}

impl IdentifierType {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }
}

/// The ICU counterpart of `java.lang.Character`: Unicode character
/// properties and case mapping.
///
/// Equivalent to `com.ibm.icu.lang.UCharacter` (which implements
/// `UCharacterEnums.ECharacterCategory` and `ECharacterDirection`; their
/// constants are reached through [`ECharacterCategory`] and
/// `ECharacterDirection`, except `UNASSIGNED` re-exported here). Methods
/// that take a `java.util.Locale` or a `ULocale` that Java allows to be
/// `null` take an `Option`.
pub struct UCharacter;

// private variables -----------------------------------------------------

const LEAD_SURROGATE_BITMASK: i32 = 0xFFFFFC00_u32 as i32;
const TRAIL_SURROGATE_BITMASK: i32 = 0xFFFFFC00_u32 as i32;
const LEAD_SURROGATE_BITS: i32 = 0xD800;
const TRAIL_SURROGATE_BITS: i32 = 0xDC00;
const U16_SURROGATE_OFFSET: i32 = (0xd800 << 10) + 0xdc00 - 0x10000;

/// To get the last character out from a data type
const LAST_CHAR_MASK: i32 = 0xFFFF;
/// No break space code point
const NO_BREAK_SPACE: i32 = 0xA0;
/// Figure space code point
const FIGURE_SPACE: i32 = 0x2007;
/// Narrow no break space code point
const NARROW_NO_BREAK_SPACE: i32 = 0x202F;
/// Ideographic number zero code point
const IDEOGRAPHIC_NUMBER_ZERO: i32 = 0x3007;
/// CJK Ideograph, First code point
const CJK_IDEOGRAPH_FIRST: i32 = 0x4e00;
/// CJK Ideograph, Second code point
const CJK_IDEOGRAPH_SECOND: i32 = 0x4e8c;
/// CJK Ideograph, Third code point
const CJK_IDEOGRAPH_THIRD: i32 = 0x4e09;
/// CJK Ideograph, Fourth code point
const CJK_IDEOGRAPH_FOURTH: i32 = 0x56db;
/// CJK Ideograph, FIFTH code point
const CJK_IDEOGRAPH_FIFTH: i32 = 0x4e94;
/// CJK Ideograph, Sixth code point
const CJK_IDEOGRAPH_SIXTH: i32 = 0x516d;
/// CJK Ideograph, Seventh code point
const CJK_IDEOGRAPH_SEVENTH: i32 = 0x4e03;
/// CJK Ideograph, Eighth code point
const CJK_IDEOGRAPH_EIGHTH: i32 = 0x516b;
/// CJK Ideograph, Nineth code point
const CJK_IDEOGRAPH_NINETH: i32 = 0x4e5d;
/// Application Program command code point
const APPLICATION_PROGRAM_COMMAND: i32 = 0x009F;
/// Unit separator code point
const UNIT_SEPARATOR: i32 = 0x001F;
/// Delete code point
const DELETE: i32 = 0x007F;
/// Han digit characters
const CJK_IDEOGRAPH_COMPLEX_ZERO: i32 = 0x96f6;
const CJK_IDEOGRAPH_COMPLEX_ONE: i32 = 0x58f9;
const CJK_IDEOGRAPH_COMPLEX_TWO: i32 = 0x8cb3;
const CJK_IDEOGRAPH_COMPLEX_THREE: i32 = 0x53c3;
const CJK_IDEOGRAPH_COMPLEX_FOUR: i32 = 0x8086;
const CJK_IDEOGRAPH_COMPLEX_FIVE: i32 = 0x4f0d;
const CJK_IDEOGRAPH_COMPLEX_SIX: i32 = 0x9678;
const CJK_IDEOGRAPH_COMPLEX_SEVEN: i32 = 0x67d2;
const CJK_IDEOGRAPH_COMPLEX_EIGHT: i32 = 0x634c;
const CJK_IDEOGRAPH_COMPLEX_NINE: i32 = 0x7396;
const CJK_IDEOGRAPH_TEN: i32 = 0x5341;
const CJK_IDEOGRAPH_COMPLEX_TEN: i32 = 0x62fe;
const CJK_IDEOGRAPH_HUNDRED: i32 = 0x767e;
const CJK_IDEOGRAPH_COMPLEX_HUNDRED: i32 = 0x4f70;
const CJK_IDEOGRAPH_THOUSAND: i32 = 0x5343;
const CJK_IDEOGRAPH_COMPLEX_THOUSAND: i32 = 0x4edf;
const CJK_IDEOGRAPH_TEN_THOUSAND: i32 = 0x824c;
const CJK_IDEOGRAPH_HUNDRED_MILLION: i32 = 0x5104;

fn mask(c: i8) -> i32 {
    1 << c
}

fn to_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl UCharacter {
    // public data members -----------------------------------------------

    /// `MIN_VALUE`: the lowest Unicode code point value.
    pub const MIN_VALUE: i32 = 0;
    /// `MAX_VALUE`: the highest Unicode code point value (scalar value).
    pub const MAX_VALUE: i32 = 0x10ffff;
    /// `SUPPLEMENTARY_MIN_VALUE`: the minimum value for Supplementary code
    /// points.
    pub const SUPPLEMENTARY_MIN_VALUE: i32 = 0x10000;
    /// `REPLACEMENT_CHAR`: Unicode value used when translating into Unicode
    /// encoding form and there is no existing character.
    pub const REPLACEMENT_CHAR: i32 = 0xFFFD;
    /// `NO_NUMERIC_VALUE`: special value that is returned by
    /// getUnicodeNumericValue(int) when no numeric value is defined for a
    /// code point.
    pub const NO_NUMERIC_VALUE: f64 = -123456789.0;
    /// `MIN_RADIX`: compatibility constant for Java Character's MIN_RADIX.
    pub const MIN_RADIX: i32 = 2;
    /// `MAX_RADIX`: compatibility constant for Java Character's MAX_RADIX.
    pub const MAX_RADIX: i32 = 36;
    /// `TITLECASE_NO_LOWERCASE`: do not lowercase non-initial parts of
    /// words when titlecasing.
    pub const TITLECASE_NO_LOWERCASE: i32 = 0x100;
    /// `TITLECASE_NO_BREAK_ADJUSTMENT`: do not adjust the titlecasing
    /// indexes from BreakIterator::next() indexes.
    pub const TITLECASE_NO_BREAK_ADJUSTMENT: i32 = 0x200;
    /// `FOLD_CASE_DEFAULT`: option value for case folding: use default
    /// mappings defined in CaseFolding.txt.
    pub const FOLD_CASE_DEFAULT: i32 = 0x0000;
    /// `FOLD_CASE_EXCLUDE_SPECIAL_I`: option value for case folding: use
    /// the modified set of mappings provided in CaseFolding.txt to handle
    /// dotted I and dotless i appropriately for Turkic languages (tr, az).
    pub const FOLD_CASE_EXCLUDE_SPECIAL_I: i32 = 0x0001;
    /// `MIN_HIGH_SURROGATE`.
    pub const MIN_HIGH_SURROGATE: u16 = 0xD800;
    /// `MAX_HIGH_SURROGATE`.
    pub const MAX_HIGH_SURROGATE: u16 = 0xDBFF;
    /// `MIN_LOW_SURROGATE`.
    pub const MIN_LOW_SURROGATE: u16 = 0xDC00;
    /// `MAX_LOW_SURROGATE`.
    pub const MAX_LOW_SURROGATE: u16 = 0xDFFF;
    /// `MIN_SURROGATE`.
    pub const MIN_SURROGATE: u16 = 0xD800;
    /// `MAX_SURROGATE`.
    pub const MAX_SURROGATE: u16 = 0xDFFF;
    /// `MIN_SUPPLEMENTARY_CODE_POINT`.
    pub const MIN_SUPPLEMENTARY_CODE_POINT: i32 = 0x10000;
    /// `MAX_CODE_POINT`.
    pub const MAX_CODE_POINT: i32 = 0x10ffff;
    /// `MIN_CODE_POINT`.
    pub const MIN_CODE_POINT: i32 = 0;
    /// `UNASSIGNED` (inherited from `ECharacterCategory`).
    pub const UNASSIGNED: i32 = ECharacterCategory::UNASSIGNED as i32;

    // public methods ----------------------------------------------------

    /// `digit(int ch, int radix)`: the numeric value of a decimal digit or
    /// a Latin letter in the radix, -1 otherwise.
    pub fn digit_radix(ch: i32, radix: i32) -> i32 {
        if (2..=36).contains(&radix) {
            let mut value = Self::digit(ch);
            if value < 0 {
                // ch is not a decimal digit, try latin letters
                value = UCharacterProperty::get_european_digit(ch);
            }
            if value < radix {
                value
            } else {
                -1
            }
        } else {
            -1 // invalid radix
        }
    }

    /// `digit(int ch)`: the decimal digit value, -1 otherwise.
    pub fn digit(ch: i32) -> i32 {
        UCharacterProperty::instance().digit(ch)
    }

    /// `getNumericValue(int ch)`.
    pub fn get_numeric_value(ch: i32) -> i32 {
        UCharacterProperty::instance().get_numeric_value(ch)
    }

    /// `getUnicodeNumericValue(int ch)`.
    pub fn get_unicode_numeric_value(ch: i32) -> f64 {
        UCharacterProperty::instance().get_unicode_numeric_value(ch)
    }

    /// `isSpace(int ch)` (deprecated): Java's ASCII white space.
    pub fn is_space(ch: i32) -> bool {
        ch <= 0x20 && (ch == 0x20 || ch == 0x09 || ch == 0x0a || ch == 0x0c || ch == 0x0d)
    }

    /// `getType(int ch)`: the general category.
    pub fn get_type(ch: i32) -> i32 {
        UCharacterProperty::instance().get_type(ch)
    }

    /// `isDefined(int ch)`.
    pub fn is_defined(ch: i32) -> bool {
        Self::get_type(ch) != 0
    }

    /// `isDigit(int ch)`.
    pub fn is_digit(ch: i32) -> bool {
        Self::get_type(ch) == ECharacterCategory::DECIMAL_DIGIT_NUMBER as i32
    }

    /// `isISOControl(int ch)`.
    pub fn is_iso_control(ch: i32) -> bool {
        (0..=APPLICATION_PROGRAM_COMMAND).contains(&ch)
            && ((ch <= UNIT_SEPARATOR) || (ch >= DELETE))
    }

    /// `isLetter(int ch)`.
    pub fn is_letter(ch: i32) -> bool {
        // if props == 0, it will just fall through and return false
        ((1 << Self::get_type(ch))
            & (mask(ECharacterCategory::UPPERCASE_LETTER)
                | mask(ECharacterCategory::LOWERCASE_LETTER)
                | mask(ECharacterCategory::TITLECASE_LETTER)
                | mask(ECharacterCategory::MODIFIER_LETTER)
                | mask(ECharacterCategory::OTHER_LETTER)))
            != 0
    }

    /// `isLetterOrDigit(int ch)`.
    pub fn is_letter_or_digit(ch: i32) -> bool {
        ((1 << Self::get_type(ch))
            & (mask(ECharacterCategory::UPPERCASE_LETTER)
                | mask(ECharacterCategory::LOWERCASE_LETTER)
                | mask(ECharacterCategory::TITLECASE_LETTER)
                | mask(ECharacterCategory::MODIFIER_LETTER)
                | mask(ECharacterCategory::OTHER_LETTER)
                | mask(ECharacterCategory::DECIMAL_DIGIT_NUMBER)))
            != 0
    }

    /// `isJavaLetter(int cp)` (deprecated).
    pub fn is_java_letter(cp: i32) -> bool {
        Self::is_java_identifier_start(cp)
    }

    /// `isJavaLetterOrDigit(int cp)` (deprecated).
    pub fn is_java_letter_or_digit(cp: i32) -> bool {
        Self::is_java_identifier_part(cp)
    }

    /// `isJavaIdentifierStart(int cp)`: `Character.isJavaIdentifierStart((char) cp)`
    /// (a letter, letter number, currency symbol or connector punctuation).
    pub fn is_java_identifier_start(cp: i32) -> bool {
        // note, downcast to char for jdk 1.4 compatibility
        let ch = cp as u16 as i32;
        use rucene_analysis_common::jdk::character as jc;
        let t = jc::get_type(ch) as i32;
        jc::is_letter(ch)
            || t == ECharacterCategory::LETTER_NUMBER as i32
            || t == ECharacterCategory::CURRENCY_SYMBOL as i32
            || t == ECharacterCategory::CONNECTOR_PUNCTUATION as i32
    }

    /// `isJavaIdentifierPart(int cp)`: `Character.isJavaIdentifierPart((char) cp)`.
    pub fn is_java_identifier_part(cp: i32) -> bool {
        // note, downcast to char for jdk 1.4 compatibility
        rucene_analysis_common::jdk::character::is_java_identifier_part(cp as u16 as i32)
    }

    /// `isLowerCase(int ch)`.
    pub fn is_lower_case(ch: i32) -> bool {
        // if props == 0, it will just fall through and return false
        Self::get_type(ch) == ECharacterCategory::LOWERCASE_LETTER as i32
    }

    /// `isWhitespace(int ch)`: Java's definition of white space.
    pub fn is_whitespace(ch: i32) -> bool {
        // exclude no-break spaces
        // if props == 0, it will just fall through and return false
        (((1 << Self::get_type(ch))
            & (mask(ECharacterCategory::SPACE_SEPARATOR)
                | mask(ECharacterCategory::LINE_SEPARATOR)
                | mask(ECharacterCategory::PARAGRAPH_SEPARATOR)))
            != 0
            && (ch != NO_BREAK_SPACE)
            && (ch != FIGURE_SPACE)
            && (ch != NARROW_NO_BREAK_SPACE))
            // TAB VT LF FF CR FS GS RS US NL are all control characters
            // that are white spaces.
            || (0x9..=0xd).contains(&ch)
            || (0x1c..=0x1f).contains(&ch)
    }

    /// `isSpaceChar(int ch)`.
    pub fn is_space_char(ch: i32) -> bool {
        // if props == 0, it will just fall through and return false
        ((1 << Self::get_type(ch))
            & (mask(ECharacterCategory::SPACE_SEPARATOR)
                | mask(ECharacterCategory::LINE_SEPARATOR)
                | mask(ECharacterCategory::PARAGRAPH_SEPARATOR)))
            != 0
    }

    /// `isTitleCase(int ch)`.
    pub fn is_title_case(ch: i32) -> bool {
        // if props == 0, it will just fall through and return false
        Self::get_type(ch) == ECharacterCategory::TITLECASE_LETTER as i32
    }

    /// `isUnicodeIdentifierPart(int ch)`.
    pub fn is_unicode_identifier_part(ch: i32) -> bool {
        Self::has_binary_property(ch, UProperty::ID_CONTINUE)
    }

    /// `isUnicodeIdentifierStart(int ch)`.
    pub fn is_unicode_identifier_start(ch: i32) -> bool {
        Self::has_binary_property(ch, UProperty::ID_START)
    }

    /// `hasIdentifierType(int c, IdentifierType type)`.
    pub fn has_identifier_type(c: i32, ty: IdentifierType) -> bool {
        UCharacterProperty::instance().has_id_type_enum(c, ty)
    }

    /// `getIdentifierTypes(int c, EnumSet<IdentifierType> types)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` for an unknown data value.
    pub fn get_identifier_types(c: i32, types: &mut BTreeSet<IdentifierType>) -> Result<i32> {
        UCharacterProperty::instance().get_id_types(c, types)
    }

    /// `isIdentifierIgnorable(int ch)`.
    pub fn is_identifier_ignorable(ch: i32) -> bool {
        // see java.lang.Character.isIdentifierIgnorable() on range of
        // ignorable characters.
        if ch <= 0x9f {
            return Self::is_iso_control(ch)
                && !((0x9..=0xd).contains(&ch) || (0x1c..=0x1f).contains(&ch));
        }
        Self::get_type(ch) == ECharacterCategory::FORMAT as i32
    }

    /// `isUpperCase(int ch)`.
    pub fn is_upper_case(ch: i32) -> bool {
        // if props == 0, it will just fall through and return false
        Self::get_type(ch) == ECharacterCategory::UPPERCASE_LETTER as i32
    }

    /// `toLowerCase(int ch)`.
    pub fn to_lower_case_cp(ch: i32) -> i32 {
        UCaseProps::instance().tolower(ch)
    }

    /// `toString(int ch)`: the code point as a string, `None` when invalid.
    pub fn to_string_cp(ch: i32) -> Option<Vec<u16>> {
        if !(Self::MIN_VALUE..=Self::MAX_VALUE).contains(&ch) {
            return None;
        }
        if ch < Self::SUPPLEMENTARY_MIN_VALUE {
            return Some(vec![ch as u16]);
        }
        Some(Self::to_chars(ch).expect("a valid code point"))
    }

    /// `toTitleCase(int ch)`.
    pub fn to_title_case_cp(ch: i32) -> i32 {
        UCaseProps::instance().totitle(ch)
    }

    /// `toUpperCase(int ch)`.
    pub fn to_upper_case_cp(ch: i32) -> i32 {
        UCaseProps::instance().toupper(ch)
    }

    // extra methods not in java.lang.Character --------------------------

    /// `isSupplementary(int ch)`.
    pub fn is_supplementary(ch: i32) -> bool {
        (Self::SUPPLEMENTARY_MIN_VALUE..=Self::MAX_VALUE).contains(&ch)
    }

    /// `isBMP(int ch)`.
    pub fn is_bmp(ch: i32) -> bool {
        (0..=LAST_CHAR_MASK).contains(&ch)
    }

    /// `isPrintable(int ch)`.
    pub fn is_printable(ch: i32) -> bool {
        let cat = Self::get_type(ch);
        // if props == 0, it will just fall through and return false
        cat != ECharacterCategory::UNASSIGNED as i32
            && cat != ECharacterCategory::CONTROL as i32
            && cat != ECharacterCategory::FORMAT as i32
            && cat != ECharacterCategory::PRIVATE_USE as i32
            && cat != ECharacterCategory::SURROGATE as i32
            && cat != ECharacterCategory::GENERAL_OTHER_TYPES as i32
    }

    /// `isBaseForm(int ch)`.
    pub fn is_base_form(ch: i32) -> bool {
        let cat = Self::get_type(ch);
        // if props == 0, it will just fall through and return false
        cat == ECharacterCategory::DECIMAL_DIGIT_NUMBER as i32
            || cat == ECharacterCategory::OTHER_NUMBER as i32
            || cat == ECharacterCategory::LETTER_NUMBER as i32
            || cat == ECharacterCategory::UPPERCASE_LETTER as i32
            || cat == ECharacterCategory::LOWERCASE_LETTER as i32
            || cat == ECharacterCategory::TITLECASE_LETTER as i32
            || cat == ECharacterCategory::MODIFIER_LETTER as i32
            || cat == ECharacterCategory::OTHER_LETTER as i32
            || cat == ECharacterCategory::NON_SPACING_MARK as i32
            || cat == ECharacterCategory::ENCLOSING_MARK as i32
            || cat == ECharacterCategory::COMBINING_SPACING_MARK as i32
    }

    /// `getDirection(int ch)`: the bidi class.
    pub fn get_direction(ch: i32) -> i32 {
        UBiDiProps::instance().get_class(ch)
    }

    /// `isMirrored(int ch)`.
    pub fn is_mirrored(ch: i32) -> bool {
        UBiDiProps::instance().is_mirrored(ch)
    }

    /// `getMirror(int ch)`.
    pub fn get_mirror(ch: i32) -> i32 {
        UBiDiProps::instance().get_mirror(ch)
    }

    /// `getBidiPairedBracket(int c)`.
    pub fn get_bidi_paired_bracket(c: i32) -> i32 {
        UBiDiProps::instance().get_paired_bracket(c)
    }

    /// `getCombiningClass(int ch)`.
    ///
    /// # Errors
    ///
    /// The exception of loading the normalization data.
    pub fn get_combining_class(ch: i32) -> Result<i32> {
        Ok(normalizer2::get_nfd_instance()?.get_combining_class(ch))
    }

    /// `isLegal(int ch)`: a valid code point that is neither a surrogate
    /// nor a noncharacter.
    pub fn is_legal(ch: i32) -> bool {
        if ch < Self::MIN_VALUE {
            return false;
        }
        if ch < Self::MIN_SURROGATE as i32 {
            return true;
        }
        if ch <= Self::MAX_SURROGATE as i32 {
            return false;
        }
        if UCharacterUtility::is_non_character(ch) {
            return false;
        }
        ch <= Self::MAX_VALUE
    }

    /// `isLegal(String str)`.
    pub fn is_legal_str(str: &[u16]) -> bool {
        let size = str.len() as i32;
        let mut i = 0;
        while i < size {
            let codepoint = crate::jdk::code_point_at(str, i);
            if !Self::is_legal(codepoint) {
                return false;
            }
            i += crate::jdk::char_count(codepoint);
        }
        true
    }

    /// `getUnicodeVersion()`.
    pub fn get_unicode_version() -> VersionInfo {
        UCharacterProperty::instance().m_unicode_version
    }

    /// `getName(int ch)`: the Unicode name, `None` when none.
    pub fn get_name_cp(ch: i32) -> Option<Vec<u16>> {
        UCharacterName::instance().get_name(ch, UCharacterNameChoice::UNICODE_CHAR_NAME)
    }

    /// `getName(String s, String separator)`: the names of the code points
    /// joined by `separator` (`null` names append "null").
    pub fn get_name_str(s: &[u16], separator: &[u16]) -> Option<Vec<u16>> {
        if s.len() == 1 {
            // handle common case
            return Self::get_name_cp(s[0] as i32);
        }
        let mut sb: Vec<u16> = Vec::new();
        let mut i = 0;
        while i < s.len() as i32 {
            let cp = crate::jdk::code_point_at(s, i);
            if i != 0 {
                sb.extend_from_slice(separator);
            }
            match Self::get_name_cp(cp) {
                Some(n) => sb.extend_from_slice(&n),
                None => sb.extend(to_units("null")),
            }
            i += crate::jdk::char_count(cp);
        }
        Some(sb)
    }

    /// `getName1_0(int ch)` (deprecated): always `None` (ICU 49).
    pub fn get_name1_0(_ch: i32) -> Option<Vec<u16>> {
        None
    }

    /// `getExtendedName(int ch)`.
    pub fn get_extended_name(ch: i32) -> Option<Vec<u16>> {
        UCharacterName::instance().get_name(ch, UCharacterNameChoice::EXTENDED_CHAR_NAME)
    }

    /// `getNameAlias(int ch)`.
    pub fn get_name_alias(ch: i32) -> Option<Vec<u16>> {
        UCharacterName::instance().get_name(ch, UCharacterNameChoice::CHAR_NAME_ALIAS)
    }

    /// `getISOComment(int ch)` (deprecated): always `None` (ICU 49).
    pub fn get_iso_comment(_ch: i32) -> Option<Vec<u16>> {
        None
    }

    /// `getCharFromName(String name)`: -1 when not found.
    pub fn get_char_from_name(name: Option<&[u16]>) -> i32 {
        UCharacterName::instance().get_char_from_name(UCharacterNameChoice::UNICODE_CHAR_NAME, name)
    }

    /// `getCharFromName1_0(String name)` (deprecated): always -1 (ICU 49).
    pub fn get_char_from_name1_0(_name: Option<&[u16]>) -> i32 {
        -1
    }

    /// `getCharFromExtendedName(String name)`: -1 when not found.
    ///
    /// # Errors
    ///
    /// None in practice; `Result` for the `UnicodeSet` caller's
    /// convenience.
    pub fn get_char_from_extended_name(name: &[u16]) -> Result<i32> {
        Ok(UCharacterName::instance()
            .get_char_from_name(UCharacterNameChoice::EXTENDED_CHAR_NAME, Some(name)))
    }

    /// `getCharFromNameAlias(String name)`.
    pub fn get_char_from_name_alias(name: Option<&[u16]>) -> i32 {
        UCharacterName::instance().get_char_from_name(UCharacterNameChoice::CHAR_NAME_ALIAS, name)
    }

    /// `getPropertyName(int property, int nameChoice)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid property or name choice;
    /// `MissingResourceException` when the data is missing.
    pub fn get_property_name(property: i32, name_choice: i32) -> Result<Option<String>> {
        UPropertyAliases::instance()?.get_property_name(property, name_choice)
    }

    /// `getPropertyEnum(CharSequence propertyAlias)`.
    ///
    /// # Errors
    ///
    /// `IllegalIcuArgumentException("Invalid name: <alias>")` for an unknown
    /// alias; `MissingResourceException` when the data is missing.
    pub fn get_property_enum(property_alias: &[u16]) -> Result<i32> {
        let prop_enum = UPropertyAliases::instance()?.get_property_enum(property_alias);
        if prop_enum == UProperty::UNDEFINED {
            return Err(IllegalIcuArgumentException::with_message(format!(
                "Invalid name: {}",
                String::from_utf16_lossy(property_alias)
            ))
            .error());
        }
        Ok(prop_enum)
    }

    /// `getPropertyValueName(int property, int value, int nameChoice)`: the
    /// name of a property value. For the canonical combining class
    /// properties with a value in the CCC range and a valid name choice,
    /// an invalid name answers `None` instead of throwing.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid property, value or name
    /// choice; `MissingResourceException` when the data is missing.
    pub fn get_property_value_name(
        property: i32,
        value: i32,
        name_choice: i32,
    ) -> Result<Option<String>> {
        if (property == UProperty::CANONICAL_COMBINING_CLASS
            || property == UProperty::LEAD_CANONICAL_COMBINING_CLASS
            || property == UProperty::TRAIL_CANONICAL_COMBINING_CLASS)
            && value >= Self::get_int_property_min_value(UProperty::CANONICAL_COMBINING_CLASS)
            && value <= Self::get_int_property_max_value(UProperty::CANONICAL_COMBINING_CLASS)
            && (0..NameChoice::COUNT).contains(&name_choice)
        {
            // this is hard coded for the valid cc
            // because PropertyValueAliases.txt does not contain all of them
            return match UPropertyAliases::instance()?.get_property_value_name(
                property,
                value,
                name_choice,
            ) {
                Ok(n) => Ok(n),
                Err(e) if e.is_illegal_argument_exception() => Ok(None),
                Err(e) => Err(e),
            };
        }
        UPropertyAliases::instance()?.get_property_value_name(property, value, name_choice)
    }

    /// `getPropertyValueEnum(int property, CharSequence valueAlias)`.
    ///
    /// # Errors
    ///
    /// `IllegalIcuArgumentException("Invalid name: <alias>")` for an unknown
    /// alias; `IllegalArgumentException` for an invalid property.
    pub fn get_property_value_enum(property: i32, value_alias: &[u16]) -> Result<i32> {
        let prop_enum =
            UPropertyAliases::instance()?.get_property_value_enum(property, value_alias)?;
        if prop_enum == UProperty::UNDEFINED {
            return Err(IllegalIcuArgumentException::with_message(format!(
                "Invalid name: {}",
                String::from_utf16_lossy(value_alias)
            ))
            .error());
        }
        Ok(prop_enum)
    }

    /// `getPropertyValueEnumNoThrow(int property, CharSequence valueAlias)`
    /// (internal): `UProperty.UNDEFINED` for an unknown alias.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data is missing.
    pub fn get_property_value_enum_no_throw(property: i32, value_alias: &[u16]) -> Result<i32> {
        Ok(UPropertyAliases::instance()?.get_property_value_enum_no_throw(property, value_alias))
    }

    /// `getCodePoint(int lead, int trail)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Not a valid surrogate pair")`.
    pub fn get_code_point_pair(lead: i32, trail: i32) -> Result<i32> {
        if Self::is_high_surrogate(lead) && Self::is_low_surrogate(trail) {
            return Ok(Self::to_code_point(lead, trail));
        }
        Err(LuceneError::illegal_argument("Not a valid surrogate pair"))
    }

    /// `getCodePoint(char char16)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint")`.
    pub fn get_code_point(char16: u16) -> Result<i32> {
        if Self::is_legal(char16 as i32) {
            return Ok(char16 as i32);
        }
        Err(LuceneError::illegal_argument("Illegal codepoint"))
    }

    /// `toUpperCase(String str)`: with the default locale.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_upper_case_str(str: &[u16]) -> Result<Vec<u16>> {
        CaseMapImpl::to_upper_str(Self::get_default_case_locale(), 0, str)
    }

    /// `toLowerCase(String str)`: with the default locale.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_lower_case_str(str: &[u16]) -> Result<Vec<u16>> {
        CaseMapImpl::to_lower_str(Self::get_default_case_locale(), 0, str)
    }

    /// `toTitleCase(String str, BreakIterator breakiter)`: with the default
    /// locale.
    ///
    /// # Errors
    ///
    /// The errors of the break iterator factory.
    pub fn to_title_case_str(
        str: &[u16],
        breakiter: Option<&mut dyn BreakIterator>,
    ) -> Result<Vec<u16>> {
        Self::to_title_case_locale_options(Some(&Locale::get_default()), str, breakiter, 0)
    }

    fn get_default_case_locale() -> i32 {
        UCaseProps::get_case_locale_jdk(&Locale::get_default())
    }

    fn get_case_locale_jdk(locale: Option<&Locale>) -> i32 {
        match locale {
            Some(l) => UCaseProps::get_case_locale_jdk(l),
            None => UCaseProps::get_case_locale_jdk(&Locale::get_default()),
        }
    }

    fn get_case_locale(locale: Option<&ULocale>) -> i32 {
        match locale {
            Some(l) => UCaseProps::get_case_locale_ulocale(l),
            None => UCaseProps::get_case_locale_ulocale(&ULocale::get_default()),
        }
    }

    /// `toUpperCase(Locale locale, String str)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_upper_case_locale(locale: Option<&Locale>, str: &[u16]) -> Result<Vec<u16>> {
        CaseMapImpl::to_upper_str(Self::get_case_locale_jdk(locale), 0, str)
    }

    /// `toUpperCase(ULocale locale, String str)`.
    pub fn to_upper_case_ulocale(locale: &ULocale, str: &[u16]) -> Vec<u16> {
        CaseMapImpl::to_upper_str(Self::get_case_locale(Some(locale)), 0, str)
            .expect("Edits of a case mapping are well-formed")
    }

    /// `toLowerCase(Locale locale, String str)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_lower_case_locale(locale: Option<&Locale>, str: &[u16]) -> Result<Vec<u16>> {
        CaseMapImpl::to_lower_str(Self::get_case_locale_jdk(locale), 0, str)
    }

    /// `toLowerCase(ULocale locale, String str)`.
    pub fn to_lower_case_ulocale(locale: &ULocale, str: &[u16]) -> Vec<u16> {
        CaseMapImpl::to_lower_str(Self::get_case_locale(Some(locale)), 0, str)
            .expect("Edits of a case mapping are well-formed")
    }

    /// `toTitleCase(Locale locale, String str, BreakIterator breakiter)`.
    ///
    /// # Errors
    ///
    /// The errors of the break iterator factory.
    pub fn to_title_case_locale(
        locale: Option<&Locale>,
        str: &[u16],
        breakiter: Option<&mut dyn BreakIterator>,
    ) -> Result<Vec<u16>> {
        Self::to_title_case_locale_options(locale, str, breakiter, 0)
    }

    /// `toTitleCase(ULocale locale, String str, BreakIterator titleIter)`.
    ///
    /// # Errors
    ///
    /// The errors of the break iterator factory.
    pub fn to_title_case_ulocale(
        locale: &ULocale,
        str: &[u16],
        title_iter: Option<&mut dyn BreakIterator>,
    ) -> Result<Vec<u16>> {
        Self::to_title_case_ulocale_options(Some(locale), str, title_iter, 0)
    }

    /// `toTitleCase(ULocale locale, String str, BreakIterator titleIter, int
    /// options)`. A given iterator is used (and its text set) in place,
    /// where Java passes the reference.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for conflicting options; the errors of
    /// the break iterator factory.
    pub fn to_title_case_ulocale_options(
        locale: Option<&ULocale>,
        str: &[u16],
        title_iter: Option<&mut dyn BreakIterator>,
        options: i32,
    ) -> Result<Vec<u16>> {
        let default;
        let locale = match (locale, title_iter.is_none()) {
            (None, true) => {
                default = ULocale::get_default();
                Some(&default)
            }
            (l, _) => l,
        };
        match title_iter {
            Some(it) => {
                if (options & 0xe0) != 0 {
                    return Err(LuceneError::illegal_argument(
                        "titlecasing iterator option together with an explicit iterator",
                    ));
                }
                it.set_text_string(str);
                CaseMapImpl::to_title_str(Self::get_case_locale(locale), options, it, str)
            }
            None => {
                let l = locale.cloned().unwrap_or_else(ULocale::get_default);
                let mut it = CaseMapImpl::get_title_break_iterator(&l, options, None)?;
                it.set_text_string(str);
                CaseMapImpl::to_title_str(Self::get_case_locale(locale), options, it.as_mut(), str)
            }
        }
    }

    /// `toTitleCase(Locale locale, String str, BreakIterator titleIter, int
    /// options)`.
    ///
    /// # Errors
    ///
    /// As [`to_title_case_ulocale_options`](Self::to_title_case_ulocale_options).
    pub fn to_title_case_locale_options(
        locale: Option<&Locale>,
        str: &[u16],
        title_iter: Option<&mut dyn BreakIterator>,
        options: i32,
    ) -> Result<Vec<u16>> {
        let default;
        let locale = match (locale, title_iter.is_none()) {
            (None, true) => {
                default = Locale::get_default();
                Some(&default)
            }
            (l, _) => l,
        };
        match title_iter {
            Some(it) => {
                if (options & 0xe0) != 0 {
                    return Err(LuceneError::illegal_argument(
                        "titlecasing iterator option together with an explicit iterator",
                    ));
                }
                it.set_text_string(str);
                CaseMapImpl::to_title_str(Self::get_case_locale_jdk(locale), options, it, str)
            }
            None => {
                let l = locale.cloned().unwrap_or_else(Locale::get_default);
                let mut it = CaseMapImpl::get_title_break_iterator_locale(&l, options, None)?;
                it.set_text_string(str);
                CaseMapImpl::to_title_str(
                    Self::get_case_locale_jdk(locale),
                    options,
                    it.as_mut(),
                    str,
                )
            }
        }
    }

    /// `foldCase(int ch, boolean defaultmapping)`.
    pub fn fold_case_cp(ch: i32, defaultmapping: bool) -> i32 {
        Self::fold_case_cp_options(
            ch,
            if defaultmapping {
                Self::FOLD_CASE_DEFAULT
            } else {
                Self::FOLD_CASE_EXCLUDE_SPECIAL_I
            },
        )
    }

    /// `foldCase(String str, boolean defaultmapping)`.
    pub fn fold_case_str(str: &[u16], defaultmapping: bool) -> Vec<u16> {
        Self::fold_case_str_options(
            str,
            if defaultmapping {
                Self::FOLD_CASE_DEFAULT
            } else {
                Self::FOLD_CASE_EXCLUDE_SPECIAL_I
            },
        )
    }

    /// `foldCase(int ch, int options)`.
    pub fn fold_case_cp_options(ch: i32, options: i32) -> i32 {
        UCaseProps::instance().fold(ch, options)
    }

    /// `foldCase(String str, int options)`.
    pub fn fold_case_str_options(str: &[u16], options: i32) -> Vec<u16> {
        CaseMapImpl::fold_str(options, str).expect("Edits of a case folding are well-formed")
    }

    /// `getHanNumericValue(int ch)`: the numeric value of a Han digit or
    /// multiplier, -1 otherwise.
    pub fn get_han_numeric_value(ch: i32) -> i32 {
        match ch {
            IDEOGRAPHIC_NUMBER_ZERO | CJK_IDEOGRAPH_COMPLEX_ZERO => 0, // Han Zero
            CJK_IDEOGRAPH_FIRST | CJK_IDEOGRAPH_COMPLEX_ONE => 1,      // Han One
            CJK_IDEOGRAPH_SECOND | CJK_IDEOGRAPH_COMPLEX_TWO => 2,     // Han Two
            CJK_IDEOGRAPH_THIRD | CJK_IDEOGRAPH_COMPLEX_THREE => 3,    // Han Three
            CJK_IDEOGRAPH_FOURTH | CJK_IDEOGRAPH_COMPLEX_FOUR => 4,    // Han Four
            CJK_IDEOGRAPH_FIFTH | CJK_IDEOGRAPH_COMPLEX_FIVE => 5,     // Han Five
            CJK_IDEOGRAPH_SIXTH | CJK_IDEOGRAPH_COMPLEX_SIX => 6,      // Han Six
            CJK_IDEOGRAPH_SEVENTH | CJK_IDEOGRAPH_COMPLEX_SEVEN => 7,  // Han Seven
            CJK_IDEOGRAPH_EIGHTH | CJK_IDEOGRAPH_COMPLEX_EIGHT => 8,   // Han Eight
            CJK_IDEOGRAPH_NINETH | CJK_IDEOGRAPH_COMPLEX_NINE => 9,    // Han Nine
            CJK_IDEOGRAPH_TEN | CJK_IDEOGRAPH_COMPLEX_TEN => 10,
            CJK_IDEOGRAPH_HUNDRED | CJK_IDEOGRAPH_COMPLEX_HUNDRED => 100,
            CJK_IDEOGRAPH_THOUSAND | CJK_IDEOGRAPH_COMPLEX_THOUSAND => 1000,
            CJK_IDEOGRAPH_TEN_THOUSAND => 10000,
            CJK_IDEOGRAPH_HUNDRED_MILLION => 100000000,
            _ => -1, // no value
        }
    }

    /// `getTypeIterator()`: an iterator over the general category ranges.
    pub fn get_type_iterator() -> Box<dyn RangeValueIterator> {
        Box::new(UCharacterTypeIterator::new())
    }

    /// `getNameIterator()`.
    pub fn get_name_iterator() -> Box<dyn ValueIterator> {
        Box::new(UCharacterNameIterator::new(
            UCharacterName::instance(),
            UCharacterNameChoice::UNICODE_CHAR_NAME,
        ))
    }

    /// `getName1_0Iterator()` (deprecated): an empty iterator (ICU 49).
    pub fn get_name1_0_iterator() -> Box<dyn ValueIterator> {
        Box::new(DummyValueIterator)
    }

    /// `getExtendedNameIterator()`.
    pub fn get_extended_name_iterator() -> Box<dyn ValueIterator> {
        Box::new(UCharacterNameIterator::new(
            UCharacterName::instance(),
            UCharacterNameChoice::EXTENDED_CHAR_NAME,
        ))
    }

    /// `getAge(int ch)`: the Unicode version in which the code point was
    /// first designated.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Codepoint out of bounds")`.
    pub fn get_age(ch: i32) -> Result<VersionInfo> {
        if !(Self::MIN_VALUE..=Self::MAX_VALUE).contains(&ch) {
            return Err(LuceneError::illegal_argument("Codepoint out of bounds"));
        }
        Ok(UCharacterProperty::instance().get_age(ch))
    }

    /// `hasBinaryProperty(int ch, int property)`.
    pub fn has_binary_property(ch: i32, property: i32) -> bool {
        UCharacterProperty::instance().has_binary_property(ch, property)
    }

    /// `hasBinaryProperty(CharSequence s, int property)`: code point or
    /// emoji properties of strings.
    pub fn has_binary_property_str(s: &[u16], property: i32) -> bool {
        let length = s.len();
        if length == 1 {
            return Self::has_binary_property(s[0] as i32, property);
        } else if length == 2 {
            // first code point
            let c = crate::jdk::code_point_at(s, 0);
            if crate::jdk::char_count(c) == length as i32 {
                return Self::has_binary_property(c, property);
            }
        }
        // Only call into EmojiProps for a relevant property,
        // so that we not unnecessarily try to load its data file.
        (UProperty::BASIC_EMOJI..=UProperty::RGI_EMOJI).contains(&property)
            && EmojiProps::instance().has_binary_property_str(s, property)
    }

    /// `isUAlphabetic(int ch)`.
    pub fn is_u_alphabetic(ch: i32) -> bool {
        Self::has_binary_property(ch, UProperty::ALPHABETIC)
    }

    /// `isULowercase(int ch)`.
    pub fn is_u_lowercase(ch: i32) -> bool {
        Self::has_binary_property(ch, UProperty::LOWERCASE)
    }

    /// `isUUppercase(int ch)`.
    pub fn is_u_uppercase(ch: i32) -> bool {
        Self::has_binary_property(ch, UProperty::UPPERCASE)
    }

    /// `isUWhiteSpace(int ch)`.
    pub fn is_u_white_space(ch: i32) -> bool {
        Self::has_binary_property(ch, UProperty::WHITE_SPACE)
    }

    /// `getIntPropertyValue(int ch, int type)`.
    pub fn get_int_property_value(ch: i32, ty: i32) -> i32 {
        UCharacterProperty::instance().get_int_property_value(ch, ty)
    }

    /// `getStringPropertyValue(int propertyEnum, int codepoint, int
    /// nameChoice)` (deprecated internal).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal Property Enum")`; the errors of
    /// the property data.
    pub fn get_string_property_value(
        property_enum: i32,
        codepoint: i32,
        name_choice: i32,
    ) -> Result<Option<Vec<u16>>> {
        if (UProperty::BINARY_START..UProperty::BINARY_LIMIT).contains(&property_enum)
            || (UProperty::INT_START..UProperty::INT_LIMIT).contains(&property_enum)
        {
            return Ok(Self::get_property_value_name(
                property_enum,
                Self::get_int_property_value(codepoint, property_enum),
                name_choice,
            )?
            .map(|s| to_units(&s)));
        }
        if property_enum == UProperty::NUMERIC_VALUE {
            return Ok(Some(to_units(&rucene::util::java_fmt::double_to_string(
                Self::get_unicode_numeric_value(codepoint),
            ))));
        }
        // otherwise must be string property
        Ok(match property_enum {
            UProperty::AGE => Some(to_units(&Self::get_age(codepoint)?.to_string())),
            UProperty::ISO_COMMENT => Self::get_iso_comment(codepoint),
            UProperty::BIDI_MIRRORING_GLYPH => Self::to_string_cp(Self::get_mirror(codepoint)),
            UProperty::CASE_FOLDING => Self::to_string_cp(Self::fold_case_cp(codepoint, true)),
            UProperty::LOWERCASE_MAPPING => Self::to_string_cp(Self::to_lower_case_cp(codepoint)),
            UProperty::NAME => Self::get_name_cp(codepoint),
            UProperty::SIMPLE_CASE_FOLDING => {
                Self::to_string_cp(Self::fold_case_cp(codepoint, true))
            }
            UProperty::SIMPLE_LOWERCASE_MAPPING => {
                Self::to_string_cp(Self::to_lower_case_cp(codepoint))
            }
            UProperty::SIMPLE_TITLECASE_MAPPING => {
                Self::to_string_cp(Self::to_title_case_cp(codepoint))
            }
            UProperty::SIMPLE_UPPERCASE_MAPPING => {
                Self::to_string_cp(Self::to_upper_case_cp(codepoint))
            }
            UProperty::TITLECASE_MAPPING => Self::to_string_cp(Self::to_title_case_cp(codepoint)),
            UProperty::UNICODE_1_NAME => Self::get_name1_0(codepoint),
            UProperty::UPPERCASE_MAPPING => Self::to_string_cp(Self::to_upper_case_cp(codepoint)),
            _ => return Err(LuceneError::illegal_argument("Illegal Property Enum")),
        })
    }

    /// `getIntPropertyMinValue(int type)`: 0 for all properties.
    pub fn get_int_property_min_value(_ty: i32) -> i32 {
        0 // undefined; and: all other properties have a minimum value of 0
    }

    /// `getIntPropertyMaxValue(int type)`.
    pub fn get_int_property_max_value(ty: i32) -> i32 {
        UCharacterProperty::instance().get_int_property_max_value(ty)
    }

    /// `forDigit(int digit, int radix)`: `Character.forDigit`.
    pub fn for_digit(digit: i32, radix: i32) -> u16 {
        if digit >= radix || digit < 0 || !(Self::MIN_RADIX..=Self::MAX_RADIX).contains(&radix) {
            return 0;
        }
        if digit < 10 {
            return (b'0' as i32 + digit) as u16;
        }
        (b'a' as i32 - 10 + digit) as u16
    }

    // JDK 1.5 API coverage

    /// `isValidCodePoint(int cp)`.
    pub fn is_valid_code_point(cp: i32) -> bool {
        (0..=Self::MAX_CODE_POINT).contains(&cp)
    }

    /// `isSupplementaryCodePoint(int cp)`.
    pub fn is_supplementary_code_point(cp: i32) -> bool {
        (Self::MIN_SUPPLEMENTARY_CODE_POINT..=Self::MAX_CODE_POINT).contains(&cp)
    }

    /// `isHighSurrogate(int codePoint)`.
    pub fn is_high_surrogate(code_point: i32) -> bool {
        (code_point & LEAD_SURROGATE_BITMASK) == LEAD_SURROGATE_BITS
    }

    /// `isLowSurrogate(int codePoint)`.
    pub fn is_low_surrogate(code_point: i32) -> bool {
        (code_point & TRAIL_SURROGATE_BITMASK) == TRAIL_SURROGATE_BITS
    }

    /// `isSurrogatePair(int high, int low)`.
    pub fn is_surrogate_pair(high: i32, low: i32) -> bool {
        Self::is_high_surrogate(high) && Self::is_low_surrogate(low)
    }

    /// `charCount(int cp)`.
    pub fn char_count(cp: i32) -> i32 {
        crate::jdk::char_count(cp)
    }

    /// `toCodePoint(int high, int low)`: the code point of a surrogate pair,
    /// in Java's wrapping `int` arithmetic.
    pub fn to_code_point(high: i32, low: i32) -> i32 {
        // see ICU4C U16_GET_SUPPLEMENTARY()
        (high << 10)
            .wrapping_add(low)
            .wrapping_sub(U16_SURROGATE_OFFSET)
    }

    /// `codePointAt(CharSequence seq, int index)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an index outside the text.
    pub fn code_point_at(seq: &[u16], mut index: i32) -> i32 {
        let c1 = seq[index as usize];
        index += 1;
        if Self::is_high_surrogate(c1 as i32) && index < seq.len() as i32 {
            let c2 = seq[index as usize];
            if Self::is_low_surrogate(c2 as i32) {
                return Self::to_code_point(c1 as i32, c2 as i32);
            }
        }
        c1 as i32
    }

    /// `codePointAt(char[] text, int index, int limit)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an invalid index or limit.
    pub fn code_point_at_limit(text: &[u16], mut index: i32, limit: i32) -> i32 {
        if index >= limit || limit > text.len() as i32 {
            index_out_of_bounds(None);
        }
        let c1 = text[index as usize];
        index += 1;
        if Self::is_high_surrogate(c1 as i32) && index < limit {
            let c2 = text[index as usize];
            if Self::is_low_surrogate(c2 as i32) {
                return Self::to_code_point(c1 as i32, c2 as i32);
            }
        }
        c1 as i32
    }

    /// `codePointBefore(CharSequence seq, int index)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an index outside `1..=len`.
    pub fn code_point_before(seq: &[u16], mut index: i32) -> i32 {
        index -= 1;
        if index < 0 {
            string_index_out_of_bounds(index);
        }
        let c2 = seq[index as usize];
        if Self::is_low_surrogate(c2 as i32) && index > 0 {
            index -= 1;
            let c1 = seq[index as usize];
            if Self::is_high_surrogate(c1 as i32) {
                return Self::to_code_point(c1 as i32, c2 as i32);
            }
        }
        c2 as i32
    }

    /// `codePointBefore(char[] text, int index, int limit)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an invalid index or limit.
    pub fn code_point_before_limit(text: &[u16], mut index: i32, limit: i32) -> i32 {
        if index <= limit || limit < 0 {
            index_out_of_bounds(None);
        }
        index -= 1;
        let c2 = text[index as usize];
        if Self::is_low_surrogate(c2 as i32) && index > limit {
            index -= 1;
            let c1 = text[index as usize];
            if Self::is_high_surrogate(c1 as i32) {
                return Self::to_code_point(c1 as i32, c2 as i32);
            }
        }
        c2 as i32
    }

    /// `toChars(int cp, char[] dst, int dstIndex)`: `Character.toChars`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `dst` is too short.
    pub fn to_chars_into(cp: i32, dst: &mut [u16], dst_index: i32) -> Result<i32> {
        let chars = Self::to_chars(cp)?;
        for (i, c) in chars.iter().enumerate() {
            dst[dst_index as usize + i] = *c;
        }
        Ok(chars.len() as i32)
    }

    /// `toChars(int cp)`: `Character.toChars`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Not a valid Unicode code point: 0x<HEX>")`.
    pub fn to_chars(cp: i32) -> Result<Vec<u16>> {
        let mut v = Vec::with_capacity(2);
        crate::jdk::append_code_point(&mut v, cp)?;
        Ok(v)
    }

    /// `getDirectionality(int cp)`.
    pub fn get_directionality(cp: i32) -> i8 {
        Self::get_direction(cp) as i8
    }

    /// `codePointCount(CharSequence text, int start, int limit)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an invalid range.
    pub fn code_point_count(text: &[u16], start: i32, mut limit: i32) -> i32 {
        if start < 0 || limit < start || limit > text.len() as i32 {
            index_out_of_bounds(Some(format!(
                "start ({start}) or limit ({limit}) invalid or out of range 0, {}",
                text.len()
            )));
        }

        let mut len = limit - start;
        while limit > start {
            limit -= 1;
            let mut ch = text[limit as usize];
            while (Self::MIN_LOW_SURROGATE..=Self::MAX_LOW_SURROGATE).contains(&ch) && limit > start
            {
                limit -= 1;
                ch = text[limit as usize];
                if (Self::MIN_HIGH_SURROGATE..=Self::MAX_HIGH_SURROGATE).contains(&ch) {
                    len -= 1;
                    break;
                }
            }
        }
        len
    }

    /// `offsetByCodePoints(CharSequence text, int index, int codePointOffset)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an index outside the text.
    pub fn offset_by_code_points(text: &[u16], mut index: i32, mut code_point_offset: i32) -> i32 {
        if index < 0 || index > text.len() as i32 {
            index_out_of_bounds(Some(format!(
                "index ( {index}) out of range 0, {}",
                text.len()
            )));
        }

        if code_point_offset < 0 {
            loop {
                code_point_offset += 1;
                if code_point_offset > 0 {
                    break;
                }
                index -= 1;
                let mut ch = text[index as usize];
                while (Self::MIN_LOW_SURROGATE..=Self::MAX_LOW_SURROGATE).contains(&ch) && index > 0
                {
                    index -= 1;
                    ch = text[index as usize];
                    if !(Self::MIN_HIGH_SURROGATE..=Self::MAX_HIGH_SURROGATE).contains(&ch) {
                        code_point_offset += 1;
                        if code_point_offset > 0 {
                            return index + 1;
                        }
                    }
                }
            }
        } else {
            let limit = text.len() as i32;
            loop {
                code_point_offset -= 1;
                if code_point_offset < 0 {
                    break;
                }
                let mut ch = text[index as usize];
                index += 1;
                while (Self::MIN_HIGH_SURROGATE..=Self::MAX_HIGH_SURROGATE).contains(&ch)
                    && index < limit
                {
                    ch = text[index as usize];
                    index += 1;
                    if !(Self::MIN_LOW_SURROGATE..=Self::MAX_LOW_SURROGATE).contains(&ch) {
                        code_point_offset -= 1;
                        if code_point_offset < 0 {
                            return index - 1;
                        }
                    }
                }
            }
        }

        index
    }

    /// `offsetByCodePoints(char[] text, int start, int count, int index,
    /// int codePointOffset)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an invalid range or offset.
    pub fn offset_by_code_points_range(
        text: &[u16],
        start: i32,
        count: i32,
        mut index: i32,
        mut code_point_offset: i32,
    ) -> i32 {
        let limit = start + count;
        if start < 0 || limit < start || limit > text.len() as i32 || index < start || index > limit
        {
            index_out_of_bounds(Some(format!(
                "index ( {index}) out of range {start}, {limit} in array 0, {}",
                text.len()
            )));
        }

        if code_point_offset < 0 {
            loop {
                code_point_offset += 1;
                if code_point_offset > 0 {
                    break;
                }
                index -= 1;
                let mut ch = text[index as usize];
                if index < start {
                    index_out_of_bounds(Some(format!("index ( {index}) < start ({start})")));
                }
                while (Self::MIN_LOW_SURROGATE..=Self::MAX_LOW_SURROGATE).contains(&ch)
                    && index > start
                {
                    index -= 1;
                    ch = text[index as usize];
                    if !(Self::MIN_HIGH_SURROGATE..=Self::MAX_HIGH_SURROGATE).contains(&ch) {
                        code_point_offset += 1;
                        if code_point_offset > 0 {
                            return index + 1;
                        }
                    }
                }
            }
        } else {
            loop {
                code_point_offset -= 1;
                if code_point_offset < 0 {
                    break;
                }
                let mut ch = text[index as usize];
                index += 1;
                if index > limit {
                    index_out_of_bounds(Some(format!("index ( {index}) > limit ({limit})")));
                }
                while (Self::MIN_HIGH_SURROGATE..=Self::MAX_HIGH_SURROGATE).contains(&ch)
                    && index < limit
                {
                    ch = text[index as usize];
                    index += 1;
                    if !(Self::MIN_LOW_SURROGATE..=Self::MAX_LOW_SURROGATE).contains(&ch) {
                        code_point_offset -= 1;
                        if code_point_offset < 0 {
                            return index - 1;
                        }
                    }
                }
            }
        }

        index
    }
}

/// `UCharacter.UCharacterTypeIterator` (private): the general category
/// ranges of the main trie.
struct UCharacterTypeIterator {
    trie_iterator: Trie2Iterator<'static, 'static>,
}

/// `MaskType.map`: the general category of a property value.
fn mask_type(value: i32) -> i32 {
    value & ucp::TYPE_MASK
}

impl UCharacterTypeIterator {
    fn new() -> Self {
        Self {
            trie_iterator: UCharacterProperty::instance()
                .m_trie
                .iterator_with(&mask_type),
        }
    }
}

impl RangeValueIterator for UCharacterTypeIterator {
    // implements RangeValueIterator
    fn next(&mut self, element: &mut RangeElement) -> bool {
        if self.trie_iterator.has_next() {
            let range = self.trie_iterator.next().expect("has_next");
            if !range.lead_surrogate {
                element.start = range.start_code_point;
                element.limit = range.end_code_point + 1;
                element.value = range.value;
                return true;
            }
        }
        false
    }

    // implements RangeValueIterator
    fn reset(&mut self) {
        self.trie_iterator = UCharacterProperty::instance()
            .m_trie
            .iterator_with(&mask_type);
    }
}

/// `UCharacter.DummyValueIterator` (private): an empty iterator.
struct DummyValueIterator;

impl ValueIterator for DummyValueIterator {
    fn next(&mut self, _element: &mut ValueElement) -> bool {
        false
    }

    fn reset(&mut self) {}

    fn set_range(&mut self, _start: i32, _limit: i32) -> Result<()> {
        Ok(())
    }
}
