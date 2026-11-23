// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UScript`.

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::BitSet;
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::{NameChoice, UProperty};
use crate::r#impl::u_character_property::{self as ucp, UCharacterProperty};
use crate::util::u_locale::ULocale;

/// Constants for ISO 15924 script codes, and related functions.
///
/// Equivalent to `com.ibm.icu.lang.UScript`.
pub struct UScript;

impl UScript {
    /// `INVALID_CODE`.
    pub const INVALID_CODE: i32 = -1;
    /// `COMMON`.
    pub const COMMON: i32 = 0;
    /// `INHERITED`.
    pub const INHERITED: i32 = 1;
    /// `ARABIC`.
    pub const ARABIC: i32 = 2;
    /// `ARMENIAN`.
    pub const ARMENIAN: i32 = 3;
    /// `BENGALI`.
    pub const BENGALI: i32 = 4;
    /// `BOPOMOFO`.
    pub const BOPOMOFO: i32 = 5;
    /// `CHEROKEE`.
    pub const CHEROKEE: i32 = 6;
    /// `COPTIC`.
    pub const COPTIC: i32 = 7;
    /// `CYRILLIC`.
    pub const CYRILLIC: i32 = 8;
    /// `DESERET`.
    pub const DESERET: i32 = 9;
    /// `DEVANAGARI`.
    pub const DEVANAGARI: i32 = 10;
    /// `ETHIOPIC`.
    pub const ETHIOPIC: i32 = 11;
    /// `GEORGIAN`.
    pub const GEORGIAN: i32 = 12;
    /// `GOTHIC`.
    pub const GOTHIC: i32 = 13;
    /// `GREEK`.
    pub const GREEK: i32 = 14;
    /// `GUJARATI`.
    pub const GUJARATI: i32 = 15;
    /// `GURMUKHI`.
    pub const GURMUKHI: i32 = 16;
    /// `HAN`.
    pub const HAN: i32 = 17;
    /// `HANGUL`.
    pub const HANGUL: i32 = 18;
    /// `HEBREW`.
    pub const HEBREW: i32 = 19;
    /// `HIRAGANA`.
    pub const HIRAGANA: i32 = 20;
    /// `KANNADA`.
    pub const KANNADA: i32 = 21;
    /// `KATAKANA`.
    pub const KATAKANA: i32 = 22;
    /// `KHMER`.
    pub const KHMER: i32 = 23;
    /// `LAO`.
    pub const LAO: i32 = 24;
    /// `LATIN`.
    pub const LATIN: i32 = 25;
    /// `MALAYALAM`.
    pub const MALAYALAM: i32 = 26;
    /// `MONGOLIAN`.
    pub const MONGOLIAN: i32 = 27;
    /// `MYANMAR`.
    pub const MYANMAR: i32 = 28;
    /// `OGHAM`.
    pub const OGHAM: i32 = 29;
    /// `OLD_ITALIC`.
    pub const OLD_ITALIC: i32 = 30;
    /// `ORIYA`.
    pub const ORIYA: i32 = 31;
    /// `RUNIC`.
    pub const RUNIC: i32 = 32;
    /// `SINHALA`.
    pub const SINHALA: i32 = 33;
    /// `SYRIAC`.
    pub const SYRIAC: i32 = 34;
    /// `TAMIL`.
    pub const TAMIL: i32 = 35;
    /// `TELUGU`.
    pub const TELUGU: i32 = 36;
    /// `THAANA`.
    pub const THAANA: i32 = 37;
    /// `THAI`.
    pub const THAI: i32 = 38;
    /// `TIBETAN`.
    pub const TIBETAN: i32 = 39;
    /// `CANADIAN_ABORIGINAL`.
    pub const CANADIAN_ABORIGINAL: i32 = 40;
    /// `UCAS`.
    pub const UCAS: i32 = Self::CANADIAN_ABORIGINAL;
    /// `YI`.
    pub const YI: i32 = 41;
    /// `TAGALOG`.
    pub const TAGALOG: i32 = 42;
    /// `HANUNOO`.
    pub const HANUNOO: i32 = 43;
    /// `BUHID`.
    pub const BUHID: i32 = 44;
    /// `TAGBANWA`.
    pub const TAGBANWA: i32 = 45;
    /// `BRAILLE`.
    pub const BRAILLE: i32 = 46;
    /// `CYPRIOT`.
    pub const CYPRIOT: i32 = 47;
    /// `LIMBU`.
    pub const LIMBU: i32 = 48;
    /// `LINEAR_B`.
    pub const LINEAR_B: i32 = 49;
    /// `OSMANYA`.
    pub const OSMANYA: i32 = 50;
    /// `SHAVIAN`.
    pub const SHAVIAN: i32 = 51;
    /// `TAI_LE`.
    pub const TAI_LE: i32 = 52;
    /// `UGARITIC`.
    pub const UGARITIC: i32 = 53;
    /// `KATAKANA_OR_HIRAGANA`.
    pub const KATAKANA_OR_HIRAGANA: i32 = 54;
    /// `BUGINESE`.
    pub const BUGINESE: i32 = 55;
    /// `GLAGOLITIC`.
    pub const GLAGOLITIC: i32 = 56;
    /// `KHAROSHTHI`.
    pub const KHAROSHTHI: i32 = 57;
    /// `SYLOTI_NAGRI`.
    pub const SYLOTI_NAGRI: i32 = 58;
    /// `NEW_TAI_LUE`.
    pub const NEW_TAI_LUE: i32 = 59;
    /// `TIFINAGH`.
    pub const TIFINAGH: i32 = 60;
    /// `OLD_PERSIAN`.
    pub const OLD_PERSIAN: i32 = 61;
    /// `BALINESE`.
    pub const BALINESE: i32 = 62;
    /// `BATAK`.
    pub const BATAK: i32 = 63;
    /// `BLISSYMBOLS`.
    pub const BLISSYMBOLS: i32 = 64;
    /// `BRAHMI`.
    pub const BRAHMI: i32 = 65;
    /// `CHAM`.
    pub const CHAM: i32 = 66;
    /// `CIRTH`.
    pub const CIRTH: i32 = 67;
    /// `OLD_CHURCH_SLAVONIC_CYRILLIC`.
    pub const OLD_CHURCH_SLAVONIC_CYRILLIC: i32 = 68;
    /// `DEMOTIC_EGYPTIAN`.
    pub const DEMOTIC_EGYPTIAN: i32 = 69;
    /// `HIERATIC_EGYPTIAN`.
    pub const HIERATIC_EGYPTIAN: i32 = 70;
    /// `EGYPTIAN_HIEROGLYPHS`.
    pub const EGYPTIAN_HIEROGLYPHS: i32 = 71;
    /// `KHUTSURI`.
    pub const KHUTSURI: i32 = 72;
    /// `SIMPLIFIED_HAN`.
    pub const SIMPLIFIED_HAN: i32 = 73;
    /// `TRADITIONAL_HAN`.
    pub const TRADITIONAL_HAN: i32 = 74;
    /// `PAHAWH_HMONG`.
    pub const PAHAWH_HMONG: i32 = 75;
    /// `OLD_HUNGARIAN`.
    pub const OLD_HUNGARIAN: i32 = 76;
    /// `HARAPPAN_INDUS`.
    pub const HARAPPAN_INDUS: i32 = 77;
    /// `JAVANESE`.
    pub const JAVANESE: i32 = 78;
    /// `KAYAH_LI`.
    pub const KAYAH_LI: i32 = 79;
    /// `LATIN_FRAKTUR`.
    pub const LATIN_FRAKTUR: i32 = 80;
    /// `LATIN_GAELIC`.
    pub const LATIN_GAELIC: i32 = 81;
    /// `LEPCHA`.
    pub const LEPCHA: i32 = 82;
    /// `LINEAR_A`.
    pub const LINEAR_A: i32 = 83;
    /// `MANDAIC`.
    pub const MANDAIC: i32 = 84;
    /// `MANDAEAN`.
    pub const MANDAEAN: i32 = Self::MANDAIC;
    /// `MAYAN_HIEROGLYPHS`.
    pub const MAYAN_HIEROGLYPHS: i32 = 85;
    /// `MEROITIC_HIEROGLYPHS`.
    pub const MEROITIC_HIEROGLYPHS: i32 = 86;
    /// `MEROITIC`.
    pub const MEROITIC: i32 = Self::MEROITIC_HIEROGLYPHS;
    /// `NKO`.
    pub const NKO: i32 = 87;
    /// `ORKHON`.
    pub const ORKHON: i32 = 88;
    /// `OLD_PERMIC`.
    pub const OLD_PERMIC: i32 = 89;
    /// `PHAGS_PA`.
    pub const PHAGS_PA: i32 = 90;
    /// `PHOENICIAN`.
    pub const PHOENICIAN: i32 = 91;
    /// `MIAO`.
    pub const MIAO: i32 = 92;
    /// `PHONETIC_POLLARD`.
    pub const PHONETIC_POLLARD: i32 = Self::MIAO;
    /// `RONGORONGO`.
    pub const RONGORONGO: i32 = 93;
    /// `SARATI`.
    pub const SARATI: i32 = 94;
    /// `ESTRANGELO_SYRIAC`.
    pub const ESTRANGELO_SYRIAC: i32 = 95;
    /// `WESTERN_SYRIAC`.
    pub const WESTERN_SYRIAC: i32 = 96;
    /// `EASTERN_SYRIAC`.
    pub const EASTERN_SYRIAC: i32 = 97;
    /// `TENGWAR`.
    pub const TENGWAR: i32 = 98;
    /// `VAI`.
    pub const VAI: i32 = 99;
    /// `VISIBLE_SPEECH`.
    pub const VISIBLE_SPEECH: i32 = 100;
    /// `CUNEIFORM`.
    pub const CUNEIFORM: i32 = 101;
    /// `UNWRITTEN_LANGUAGES`.
    pub const UNWRITTEN_LANGUAGES: i32 = 102;
    /// `UNKNOWN`.
    pub const UNKNOWN: i32 = 103;
    /// `CARIAN`.
    pub const CARIAN: i32 = 104;
    /// `JAPANESE`.
    pub const JAPANESE: i32 = 105;
    /// `LANNA`.
    pub const LANNA: i32 = 106;
    /// `LYCIAN`.
    pub const LYCIAN: i32 = 107;
    /// `LYDIAN`.
    pub const LYDIAN: i32 = 108;
    /// `OL_CHIKI`.
    pub const OL_CHIKI: i32 = 109;
    /// `REJANG`.
    pub const REJANG: i32 = 110;
    /// `SAURASHTRA`.
    pub const SAURASHTRA: i32 = 111;
    /// `SIGN_WRITING`.
    pub const SIGN_WRITING: i32 = 112;
    /// `SUNDANESE`.
    pub const SUNDANESE: i32 = 113;
    /// `MOON`.
    pub const MOON: i32 = 114;
    /// `MEITEI_MAYEK`.
    pub const MEITEI_MAYEK: i32 = 115;
    /// `IMPERIAL_ARAMAIC`.
    pub const IMPERIAL_ARAMAIC: i32 = 116;
    /// `AVESTAN`.
    pub const AVESTAN: i32 = 117;
    /// `CHAKMA`.
    pub const CHAKMA: i32 = 118;
    /// `KOREAN`.
    pub const KOREAN: i32 = 119;
    /// `KAITHI`.
    pub const KAITHI: i32 = 120;
    /// `MANICHAEAN`.
    pub const MANICHAEAN: i32 = 121;
    /// `INSCRIPTIONAL_PAHLAVI`.
    pub const INSCRIPTIONAL_PAHLAVI: i32 = 122;
    /// `PSALTER_PAHLAVI`.
    pub const PSALTER_PAHLAVI: i32 = 123;
    /// `BOOK_PAHLAVI`.
    pub const BOOK_PAHLAVI: i32 = 124;
    /// `INSCRIPTIONAL_PARTHIAN`.
    pub const INSCRIPTIONAL_PARTHIAN: i32 = 125;
    /// `SAMARITAN`.
    pub const SAMARITAN: i32 = 126;
    /// `TAI_VIET`.
    pub const TAI_VIET: i32 = 127;
    /// `MATHEMATICAL_NOTATION`.
    pub const MATHEMATICAL_NOTATION: i32 = 128;
    /// `SYMBOLS`.
    pub const SYMBOLS: i32 = 129;
    /// `BAMUM`.
    pub const BAMUM: i32 = 130;
    /// `LISU`.
    pub const LISU: i32 = 131;
    /// `NAKHI_GEBA`.
    pub const NAKHI_GEBA: i32 = 132;
    /// `OLD_SOUTH_ARABIAN`.
    pub const OLD_SOUTH_ARABIAN: i32 = 133;
    /// `BASSA_VAH`.
    pub const BASSA_VAH: i32 = 134;
    /// `DUPLOYAN`.
    pub const DUPLOYAN: i32 = 135;
    /// `DUPLOYAN_SHORTAND`.
    pub const DUPLOYAN_SHORTAND: i32 = Self::DUPLOYAN;
    /// `ELBASAN`.
    pub const ELBASAN: i32 = 136;
    /// `GRANTHA`.
    pub const GRANTHA: i32 = 137;
    /// `KPELLE`.
    pub const KPELLE: i32 = 138;
    /// `LOMA`.
    pub const LOMA: i32 = 139;
    /// `MENDE`.
    pub const MENDE: i32 = 140;
    /// `MEROITIC_CURSIVE`.
    pub const MEROITIC_CURSIVE: i32 = 141;
    /// `OLD_NORTH_ARABIAN`.
    pub const OLD_NORTH_ARABIAN: i32 = 142;
    /// `NABATAEAN`.
    pub const NABATAEAN: i32 = 143;
    /// `PALMYRENE`.
    pub const PALMYRENE: i32 = 144;
    /// `KHUDAWADI`.
    pub const KHUDAWADI: i32 = 145;
    /// `SINDHI`.
    pub const SINDHI: i32 = Self::KHUDAWADI;
    /// `WARANG_CITI`.
    pub const WARANG_CITI: i32 = 146;
    /// `AFAKA`.
    pub const AFAKA: i32 = 147;
    /// `JURCHEN`.
    pub const JURCHEN: i32 = 148;
    /// `MRO`.
    pub const MRO: i32 = 149;
    /// `NUSHU`.
    pub const NUSHU: i32 = 150;
    /// `SHARADA`.
    pub const SHARADA: i32 = 151;
    /// `SORA_SOMPENG`.
    pub const SORA_SOMPENG: i32 = 152;
    /// `TAKRI`.
    pub const TAKRI: i32 = 153;
    /// `TANGUT`.
    pub const TANGUT: i32 = 154;
    /// `WOLEAI`.
    pub const WOLEAI: i32 = 155;
    /// `ANATOLIAN_HIEROGLYPHS`.
    pub const ANATOLIAN_HIEROGLYPHS: i32 = 156;
    /// `KHOJKI`.
    pub const KHOJKI: i32 = 157;
    /// `TIRHUTA`.
    pub const TIRHUTA: i32 = 158;
    /// `CAUCASIAN_ALBANIAN`.
    pub const CAUCASIAN_ALBANIAN: i32 = 159;
    /// `MAHAJANI`.
    pub const MAHAJANI: i32 = 160;
    /// `AHOM`.
    pub const AHOM: i32 = 161;
    /// `HATRAN`.
    pub const HATRAN: i32 = 162;
    /// `MODI`.
    pub const MODI: i32 = 163;
    /// `MULTANI`.
    pub const MULTANI: i32 = 164;
    /// `PAU_CIN_HAU`.
    pub const PAU_CIN_HAU: i32 = 165;
    /// `SIDDHAM`.
    pub const SIDDHAM: i32 = 166;
    /// `ADLAM`.
    pub const ADLAM: i32 = 167;
    /// `BHAIKSUKI`.
    pub const BHAIKSUKI: i32 = 168;
    /// `MARCHEN`.
    pub const MARCHEN: i32 = 169;
    /// `NEWA`.
    pub const NEWA: i32 = 170;
    /// `OSAGE`.
    pub const OSAGE: i32 = 171;
    /// `HAN_WITH_BOPOMOFO`.
    pub const HAN_WITH_BOPOMOFO: i32 = 172;
    /// `JAMO`.
    pub const JAMO: i32 = 173;
    /// `SYMBOLS_EMOJI`.
    pub const SYMBOLS_EMOJI: i32 = 174;
    /// `MASARAM_GONDI`.
    pub const MASARAM_GONDI: i32 = 175;
    /// `SOYOMBO`.
    pub const SOYOMBO: i32 = 176;
    /// `ZANABAZAR_SQUARE`.
    pub const ZANABAZAR_SQUARE: i32 = 177;
    /// `DOGRA`.
    pub const DOGRA: i32 = 178;
    /// `GUNJALA_GONDI`.
    pub const GUNJALA_GONDI: i32 = 179;
    /// `MAKASAR`.
    pub const MAKASAR: i32 = 180;
    /// `MEDEFAIDRIN`.
    pub const MEDEFAIDRIN: i32 = 181;
    /// `HANIFI_ROHINGYA`.
    pub const HANIFI_ROHINGYA: i32 = 182;
    /// `SOGDIAN`.
    pub const SOGDIAN: i32 = 183;
    /// `OLD_SOGDIAN`.
    pub const OLD_SOGDIAN: i32 = 184;
    /// `ELYMAIC`.
    pub const ELYMAIC: i32 = 185;
    /// `NYIAKENG_PUACHUE_HMONG`.
    pub const NYIAKENG_PUACHUE_HMONG: i32 = 186;
    /// `NANDINAGARI`.
    pub const NANDINAGARI: i32 = 187;
    /// `WANCHO`.
    pub const WANCHO: i32 = 188;
    /// `CHORASMIAN`.
    pub const CHORASMIAN: i32 = 189;
    /// `DIVES_AKURU`.
    pub const DIVES_AKURU: i32 = 190;
    /// `KHITAN_SMALL_SCRIPT`.
    pub const KHITAN_SMALL_SCRIPT: i32 = 191;
    /// `YEZIDI`.
    pub const YEZIDI: i32 = 192;
    /// `CYPRO_MINOAN`.
    pub const CYPRO_MINOAN: i32 = 193;
    /// `OLD_UYGHUR`.
    pub const OLD_UYGHUR: i32 = 194;
    /// `TANGSA`.
    pub const TANGSA: i32 = 195;
    /// `TOTO`.
    pub const TOTO: i32 = 196;
    /// `VITHKUQI`.
    pub const VITHKUQI: i32 = 197;
    /// `KAWI`.
    pub const KAWI: i32 = 198;
    /// `NAG_MUNDARI`.
    pub const NAG_MUNDARI: i32 = 199;
    /// `ARABIC_NASTALIQ`.
    pub const ARABIC_NASTALIQ: i32 = 200;
    /// `GARAY`.
    pub const GARAY: i32 = 201;
    /// `GURUNG_KHEMA`.
    pub const GURUNG_KHEMA: i32 = 202;
    /// `KIRAT_RAI`.
    pub const KIRAT_RAI: i32 = 203;
    /// `OL_ONAL`.
    pub const OL_ONAL: i32 = 204;
    /// `SUNUWAR`.
    pub const SUNUWAR: i32 = 205;
    /// `TODHRI`.
    pub const TODHRI: i32 = 206;
    /// `TULU_TIGALARI`.
    pub const TULU_TIGALARI: i32 = 207;
    /// `CODE_LIMIT`.
    pub const CODE_LIMIT: i32 = 208;
}

/// Script usage constants (UAX #31 / ScriptMetadata).
///
/// Equivalent to the enum `UScript.ScriptUsage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScriptUsage {
    /// `NOT_ENCODED`: not encoded in Unicode.
    NotEncoded,
    /// `UNKNOWN`: unknown script usage.
    Unknown,
    /// `EXCLUDED`: candidate for exclusion from identifiers.
    Excluded,
    /// `LIMITED_USE`: limited use script.
    LimitedUse,
    /// `ASPIRATIONAL`: aspirational use script.
    Aspirational,
    /// `RECOMMENDED`: recommended script.
    Recommended,
}

/// `usageValues`.
const USAGE_VALUES: [ScriptUsage; 6] = [
    ScriptUsage::NotEncoded,
    ScriptUsage::Unknown,
    ScriptUsage::Excluded,
    ScriptUsage::LimitedUse,
    ScriptUsage::Aspirational,
    ScriptUsage::Recommended,
];

/// Script metadata (script properties).
/// See http://unicode.org/cldr/trac/browser/trunk/common/properties/scriptMetadata.txt
///
/// Equivalent to `UScript.ScriptMetadata` (private).
struct ScriptMetadata;

impl ScriptMetadata {
    // 0 = NOT_ENCODED, no sample character, default false script properties.
    // Bits 20.. 0: sample character

    // Bits 23..21: usage
    #[allow(dead_code)]
    const UNKNOWN: i32 = 1 << 21;
    #[allow(dead_code)]
    const EXCLUSION: i32 = 2 << 21;
    #[allow(dead_code)]
    const LIMITED_USE: i32 = 3 << 21;
    // vate static final int ASPIRATIONAL = 4 << 21; -- not used any more since Unicode 10
    #[allow(dead_code)]
    const RECOMMENDED: i32 = 5 << 21;

    // Bits 31..24: Single-bit flags
    const RTL: i32 = 1 << 24;
    const LB_LETTERS: i32 = 1 << 25;
    const CASED: i32 = 1 << 26;

    fn get_script_props(script: i32) -> i32 {
        if 0 <= script && (script as usize) < SCRIPT_PROPS.len() {
            SCRIPT_PROPS[script as usize]
        } else {
            0
        }
    }
}

// Begin copy-paste output from
// tools/trunk/unicode/py/parsescriptmetadata.py
// or from icu/trunk/source/common/uscript_props.cpp
static SCRIPT_PROPS: [i32; 208] = [
    0x00A00040_u32 as i32, 0x00A00308_u32 as i32, 0x01A00628_u32 as i32, 0x04A00531_u32 as i32,
    0x00A00995_u32 as i32, 0x02A03105_u32 as i32, 0x046013C4_u32 as i32, 0x044003E2_u32 as i32,
    0x04A0042F_u32 as i32, 0x04410414_u32 as i32, 0x00A00905_u32 as i32, 0x00A012A0_u32 as i32,
    0x00A010D3_u32 as i32, 0x00410330_u32 as i32, 0x04A003A9_u32 as i32, 0x00A00A95_u32 as i32,
    0x00A00A15_u32 as i32, 0x02A05B57_u32 as i32, 0x00A0AC00_u32 as i32, 0x01A005D0_u32 as i32,
    0x02A0304B_u32 as i32, 0x00A00C95_u32 as i32, 0x02A030AB_u32 as i32, 0x02A01780_u32 as i32,
    0x02A00EA5_u32 as i32, 0x04A0004C_u32 as i32, 0x00A00D15_u32 as i32, 0x00401826_u32 as i32,
    0x02A01000_u32 as i32, 0x0040168F_u32 as i32, 0x00410300_u32 as i32, 0x00A00B15_u32 as i32,
    0x004016A0_u32 as i32, 0x00A00D85_u32 as i32, 0x01600710_u32 as i32, 0x00A00B95_u32 as i32,
    0x00A00C15_u32 as i32, 0x01A0078C_u32 as i32, 0x02A00E17_u32 as i32, 0x00A00F40_u32 as i32,
    0x006014C0_u32 as i32, 0x0260A288_u32 as i32, 0x00401703_u32 as i32, 0x00401723_u32 as i32,
    0x00401743_u32 as i32, 0x00401763_u32 as i32, 0x0020280E_u32 as i32, 0x01410800_u32 as i32,
    0x00601900_u32 as i32, 0x00410000_u32 as i32, 0x00410480_u32 as i32, 0x00410450_u32 as i32,
    0x02601950_u32 as i32, 0x00410380_u32 as i32, 0x00000000_u32 as i32, 0x00401A00_u32 as i32,
    0x04402C00_u32 as i32, 0x01410A00_u32 as i32, 0x0060A800_u32 as i32, 0x02601980_u32 as i32,
    0x00602D30_u32 as i32, 0x004103A0_u32 as i32, 0x00601B05_u32 as i32, 0x00601BC0_u32 as i32,
    0x00000000_u32 as i32, 0x00411005_u32 as i32, 0x0060AA00_u32 as i32, 0x00000000_u32 as i32,
    0x00000000_u32 as i32, 0x00000000_u32 as i32, 0x00000000_u32 as i32, 0x00413153_u32 as i32,
    0x00000000_u32 as i32, 0x02A05B57_u32 as i32, 0x02A05B57_u32 as i32, 0x00416B1C_u32 as i32,
    0x05410CA1_u32 as i32, 0x00000000_u32 as i32, 0x0060A984_u32 as i32, 0x0060A90A_u32 as i32,
    0x00000000_u32 as i32, 0x00000000_u32 as i32, 0x00601C00_u32 as i32, 0x00410647_u32 as i32,
    0x01600840_u32 as i32, 0x00000000_u32 as i32, 0x01410980_u32 as i32, 0x016007CA_u32 as i32,
    0x01410C00_u32 as i32, 0x0041036B_u32 as i32, 0x0040A840_u32 as i32, 0x01410900_u32 as i32,
    0x00616F00_u32 as i32, 0x00000000_u32 as i32, 0x00000000_u32 as i32, 0x00000000_u32 as i32,
    0x00000000_u32 as i32, 0x00000000_u32 as i32, 0x00000000_u32 as i32, 0x0060A549_u32 as i32,
    0x00000000_u32 as i32, 0x00412000_u32 as i32, 0x00000000_u32 as i32, 0x0020FDD0_u32 as i32,
    0x004102A0_u32 as i32, 0x02A0304B_u32 as i32, 0x02601A20_u32 as i32, 0x00410280_u32 as i32,
    0x01410920_u32 as i32, 0x00601C5A_u32 as i32, 0x0040A930_u32 as i32, 0x0060A882_u32 as i32,
    0x0041D850_u32 as i32, 0x00601B83_u32 as i32, 0x00000000_u32 as i32, 0x0060ABC0_u32 as i32,
    0x01410840_u32 as i32, 0x01410B00_u32 as i32, 0x00611103_u32 as i32, 0x00A0AC00_u32 as i32,
    0x00411083_u32 as i32, 0x01410AD8_u32 as i32, 0x01410B60_u32 as i32, 0x01410B8F_u32 as i32,
    0x00000000_u32 as i32, 0x01410B40_u32 as i32, 0x01400800_u32 as i32, 0x0260AA80_u32 as i32,
    0x00000000_u32 as i32, 0x00000000_u32 as i32, 0x0060A6A0_u32 as i32, 0x0060A4D0_u32 as i32,
    0x00000000_u32 as i32, 0x01410A60_u32 as i32, 0x00416AE6_u32 as i32, 0x0041BC20_u32 as i32,
    0x00410500_u32 as i32, 0x00411315_u32 as i32, 0x00000000_u32 as i32, 0x00000000_u32 as i32,
    0x0141E802_u32 as i32, 0x014109A0_u32 as i32, 0x01410A95_u32 as i32, 0x01410896_u32 as i32,
    0x01410873_u32 as i32, 0x004112BE_u32 as i32, 0x044118B4_u32 as i32, 0x00000000_u32 as i32,
    0x00000000_u32 as i32, 0x00416A4F_u32 as i32, 0x0241B1C4_u32 as i32, 0x00411183_u32 as i32,
    0x004110D0_u32 as i32, 0x00411680_u32 as i32, 0x02418229_u32 as i32, 0x00000000_u32 as i32,
    0x00414400_u32 as i32, 0x00411208_u32 as i32, 0x00411484_u32 as i32, 0x00410537_u32 as i32,
    0x00411152_u32 as i32, 0x02411717_u32 as i32, 0x014108F4_u32 as i32, 0x0041160E_u32 as i32,
    0x0041128F_u32 as i32, 0x00411AC0_u32 as i32, 0x0041158E_u32 as i32, 0x0561E909_u32 as i32,
    0x00411C0E_u32 as i32, 0x00411C72_u32 as i32, 0x00611412_u32 as i32, 0x046104B5_u32 as i32,
    0x02A05B57_u32 as i32, 0x00A01112_u32 as i32, 0x00000000_u32 as i32, 0x00411D10_u32 as i32,
    0x00411A5C_u32 as i32, 0x00411A0B_u32 as i32, 0x0041180B_u32 as i32, 0x00611D71_u32 as i32,
    0x00411EE5_u32 as i32, 0x04416E40_u32 as i32, 0x01610D12_u32 as i32, 0x01410F42_u32 as i32,
    0x01410F19_u32 as i32, 0x01410FF1_u32 as i32, 0x0061E108_u32 as i32, 0x004119CE_u32 as i32,
    0x0061E2E1_u32 as i32, 0x01410FBF_u32 as i32, 0x0041190C_u32 as i32, 0x02418C65_u32 as i32,
    0x01410E88_u32 as i32, 0x00412FE5_u32 as i32, 0x01410F7C_u32 as i32, 0x00416ABC_u32 as i32,
    0x0041E290_u32 as i32, 0x04410582_u32 as i32, 0x02411F1B_u32 as i32, 0x0041E4E6_u32 as i32,
    0x00000000_u32 as i32, 0x05410D5D_u32 as i32, 0x0041611C_u32 as i32, 0x00416D45_u32 as i32,
    0x0041E5D0_u32 as i32, 0x00411BC4_u32 as i32, 0x004105C2_u32 as i32, 0x00411392_u32 as i32,
];

impl UScript {
    fn get_codes_from_locale(locale: &ULocale) -> Result<Option<Vec<i32>>> {
        // Multi-script languages, equivalent to the LocaleScript data
        // that we used to load from locale resource bundles.
        let lang = locale.get_language();
        if lang == "ja" {
            return Ok(Some(vec![UScript::KATAKANA, UScript::HIRAGANA, UScript::HAN]));
        }
        if lang == "ko" {
            return Ok(Some(vec![UScript::HANGUL, UScript::HAN]));
        }
        let script = locale.get_script();
        if lang == "zh" && script == "Hant" {
            return Ok(Some(vec![UScript::HAN, UScript::BOPOMOFO]));
        }
        // Explicit script code.
        if !script.is_empty() {
            let mut script_code = UScript::get_code_from_name(&script)?;
            if script_code != UScript::INVALID_CODE {
                if script_code == UScript::SIMPLIFIED_HAN || script_code == UScript::TRADITIONAL_HAN {
                    script_code = UScript::HAN;
                }
                return Ok(Some(vec![script_code]));
            }
        }
        Ok(None)
    }

    /// Helper function to find the code from locale.
    fn find_code_from_locale(locale: &ULocale) -> Result<Option<Vec<i32>>> {
        let result = Self::get_codes_from_locale(locale)?;
        if result.is_some() {
            return Ok(result);
        }
        let likely = ULocale::add_likely_subtags(locale)?;
        Self::get_codes_from_locale(&likely)
    }

    /// `getCode(Locale locale)`: the script codes associated with the
    /// locale, `None` when none.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_code_locale(locale: &Locale) -> Result<Option<Vec<i32>>> {
        let ul = ULocale::for_locale(Some(locale))?.expect("a non-null Locale maps to a ULocale");
        Self::find_code_from_locale(&ul)
    }

    /// `getCode(ULocale locale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_code_ulocale(locale: &ULocale) -> Result<Option<Vec<i32>>> {
        Self::find_code_from_locale(locale)
    }

    /// `getCode(String nameOrAbbrOrLocale)`: the script codes for a script
    /// name, abbreviation or locale id, `None` when none.
    ///
    /// # Errors
    ///
    /// The errors of the locale and property data.
    pub fn get_code(name_or_abbr_or_locale: &str) -> Result<Option<Vec<i32>>> {
        let units: Vec<u16> = name_or_abbr_or_locale.encode_utf16().collect();
        let mut tried_code = false;
        let mut last_sep_pos = crate::jdk::index_of_unit(&units, b'_' as u16, 0);
        if last_sep_pos < 0 {
            last_sep_pos = crate::jdk::index_of_unit(&units, b'-' as u16, 0);
        }
        // Favor interpretation of nameOrAbbrOrLocale as a script alias if either
        // 1. nameOrAbbrOrLocale does not contain -/_. Handles Han, Mro, Nko, etc.
        // 2. The last instance of -/_ is at offset 3, and the portion after that is
        //    longer than 4 characters (i.e. not a script or region code). This handles
        //    Old_Hungarian, Old_Italic, etc. ("old" is a valid language code)
        // 3. The last instance of -/_ is at offset 7, and the portion after that is
        //    3 characters. This handles New_Tai_Lue ("new" is a valid language code).
        let len = units.len();
        if last_sep_pos < 0 || (last_sep_pos == 3 && len > 8) || (last_sep_pos == 7 && len == 11) {
            let prop_num = UCharacter::get_property_value_enum_no_throw(UProperty::SCRIPT, &units)?;
            if prop_num != UProperty::UNDEFINED {
                return Ok(Some(vec![prop_num]));
            }
            tried_code = true;
        }
        let scripts = Self::find_code_from_locale(&ULocale::new(name_or_abbr_or_locale)?)?;
        if scripts.is_some() {
            return Ok(scripts);
        }
        if !tried_code {
            let prop_num = UCharacter::get_property_value_enum_no_throw(UProperty::SCRIPT, &units)?;
            if prop_num != UProperty::UNDEFINED {
                return Ok(Some(vec![prop_num]));
            }
        }
        Ok(None)
    }

    /// `getCodeFromName(String nameOrAbbr)`: the script code of a script
    /// name or abbreviation, `INVALID_CODE` when unknown.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the property names data is missing.
    pub fn get_code_from_name(name_or_abbr: &str) -> Result<i32> {
        let units: Vec<u16> = name_or_abbr.encode_utf16().collect();
        let prop_num = UCharacter::get_property_value_enum_no_throw(UProperty::SCRIPT, &units)?;
        Ok(if prop_num == UProperty::UNDEFINED { Self::INVALID_CODE } else { prop_num })
    }

    /// `getScript(int codepoint)`: the Script property value of a code
    /// point.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException(<codepoint>)` outside the code point range.
    pub fn get_script(codepoint: i32) -> Result<i32> {
        if (UCharacter::MIN_VALUE..=UCharacter::MAX_VALUE).contains(&codepoint) {
            let upc = UCharacterProperty::instance();
            let script_x = upc.get_additional(codepoint, 0) & ucp::SCRIPT_X_MASK;
            let code_or_index = script_x & ucp::MAX_SCRIPT;
            if script_x < ucp::SCRIPT_X_WITH_COMMON {
                Ok(code_or_index)
            } else if script_x < ucp::SCRIPT_X_WITH_INHERITED {
                Ok(UScript::COMMON)
            } else if script_x < ucp::SCRIPT_X_WITH_OTHER {
                Ok(UScript::INHERITED)
            } else {
                Ok(upc.m_script_extensions[code_or_index as usize] as i32)
            }
        } else {
            Err(LuceneError::illegal_argument(codepoint.to_string()))
        }
    }

    /// `hasScript(int c, int sc)`: whether the Script_Extensions of c
    /// contain sc.
    pub fn has_script(c: i32, sc: i32) -> bool {
        let upc = UCharacterProperty::instance();
        let script_x = upc.get_additional(c, 0) & ucp::SCRIPT_X_MASK;
        let code_or_index = script_x & ucp::MAX_SCRIPT;
        if script_x < ucp::SCRIPT_X_WITH_COMMON {
            return sc == code_or_index;
        }

        let script_extensions = &upc.m_script_extensions;
        let mut scx = code_or_index as usize; // index into scriptExtensions
        if script_x >= ucp::SCRIPT_X_WITH_OTHER {
            scx = script_extensions[scx + 1] as usize;
        }
        if sc > 0x7fff {
            // Guard against bogus input that would
            // make us go past the Script_Extensions terminator.
            return false;
        }
        while sc > script_extensions[scx] as i32 {
            scx += 1;
        }
        sc == (script_extensions[scx] as i32 & 0x7fff)
    }

    /// `getScriptExtensions(int c, BitSet set)`: fills `set` with the
    /// Script_Extensions of c; returns the single script code, or the
    /// negated number of scripts.
    pub fn get_script_extensions(c: i32, set: &mut BitSet) -> i32 {
        set.clear();
        let upc = UCharacterProperty::instance();
        let script_x = upc.get_additional(c, 0) & ucp::SCRIPT_X_MASK;
        let code_or_index = script_x & ucp::MAX_SCRIPT;
        if script_x < ucp::SCRIPT_X_WITH_COMMON {
            set.set(code_or_index);
            return code_or_index;
        }

        let script_extensions = &upc.m_script_extensions;
        let mut scx = code_or_index as usize; // index into scriptExtensions
        if script_x >= ucp::SCRIPT_X_WITH_OTHER {
            scx = script_extensions[scx + 1] as usize;
        }
        let mut length = 0;
        loop {
            let sx = script_extensions[scx] as i32;
            scx += 1;
            set.set(sx & 0x7fff);
            length += 1;
            if sx >= 0x8000 {
                break;
            }
        }
        // length==set.cardinality()
        -length
    }

    /// `getName(int scriptCode)`: the long script name.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid script code;
    /// `MissingResourceException` when the data is missing.
    pub fn get_name(script_code: i32) -> Result<Option<String>> {
        UCharacter::get_property_value_name(UProperty::SCRIPT, script_code, NameChoice::LONG)
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

    /// `getSampleString(int script)`: a representative character of the
    /// script, empty when none.
    pub fn get_sample_string(script: i32) -> Vec<u16> {
        let sample_char = ScriptMetadata::get_script_props(script) & 0x1fffff;
        if sample_char != 0 {
            let mut sb = Vec::new();
            crate::jdk::append_code_point(&mut sb, sample_char).expect("sample characters are code points");
            return sb;
        }
        Vec::new()
    }

    /// `getUsage(int script)`.
    pub fn get_usage(script: i32) -> ScriptUsage {
        USAGE_VALUES[((ScriptMetadata::get_script_props(script) >> 21) & 7) as usize]
    }

    /// `isRightToLeft(int script)`.
    pub fn is_right_to_left(script: i32) -> bool {
        (ScriptMetadata::get_script_props(script) & ScriptMetadata::RTL) != 0
    }

    /// `breaksBetweenLetters(int script)`.
    pub fn breaks_between_letters(script: i32) -> bool {
        (ScriptMetadata::get_script_props(script) & ScriptMetadata::LB_LETTERS) != 0
    }

    /// `isCased(int script)`.
    pub fn is_cased(script: i32) -> bool {
        (ScriptMetadata::get_script_props(script) & ScriptMetadata::CASED) != 0
    }
}
