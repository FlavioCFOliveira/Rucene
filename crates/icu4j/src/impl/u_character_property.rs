// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UCharacterProperty`.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use crate::jdk::{char_count, code_point_at};
use crate::lang::u_character::{
    HangulSyllableType, IdentifierStatus, IdentifierType, NumericType, UCharacter,
};
use crate::lang::u_character_enums::ECharacterCategory;
use crate::lang::u_property::UProperty;
use crate::lang::u_script::UScript;
use crate::r#impl::emoji_props::EmojiProps;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::norm2_all_modes::Norm2AllModes;
use crate::r#impl::normalizer2_impl::{ReorderingBuffer, UTF16Plus};
use crate::r#impl::trie2_16::Trie2_16;
use crate::r#impl::u_bidi_props::UBiDiProps;
use crate::r#impl::u_case_props::UCaseProps;
use crate::text::normalizer2;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;
use crate::util::code_point_map::{CodePointMap, Range};
use crate::util::code_point_trie::{CodePointTrie, ValueWidth};
use crate::util::icu_exception::ICUException;
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;
use crate::util::u_resource_bundle::missing_resource;
use crate::util::version_info::VersionInfo;

fn io_exception(message: &str) -> LuceneError {
    LuceneError::from(std::io::Error::other(message.to_string()))
}

/// The message of the panic that stands for a built-in normalization
/// singleton failing to load its embedded data (Java rethrows the stored
/// `RuntimeException`).
const NORM_DATA: &str = "the embedded normalization data loads";

/// Internal class used for Unicode character property database.
///
/// Equivalent to `com.ibm.icu.impl.UCharacterProperty` (reads
/// `uprops.icu`; the nested `LayoutProps` reads `ulayout.icu`). Java's
/// anonymous `BinaryProperty`/`IntProperty` subclasses are the
/// [`BinaryProperty`]/[`IntProperty`] kinds.
#[derive(Debug)]
pub struct UCharacterProperty {
    /// `m_trie_`: trie data.
    pub m_trie: Trie2_16,
    /// `m_unicodeVersion_`: Unicode version.
    pub m_unicode_version: VersionInfo,
    // additional properties ------------------------------------------------
    /// Additional property trie.
    m_additional_trie: Option<Trie2_16>,
    /// Additional property vectors.
    m_additional_vectors: Vec<i32>,
    /// Additional property vector column count.
    m_additional_columns_count: i32,
    /// Maximum values for block, bits used as in vector word 0.
    m_max_block_script_value: i32,
    /// Maximum values for script, bits used as in vector word 0.
    m_max_jtg_value: i32,
    /// Maximum values for other code values.
    m_max_values_other: i32,
    /// `m_scriptExtensions_`: Script_Extensions data.
    pub m_script_extensions: Vec<u16>,
    m_block_trie: CodePointTrie,
    bin_props: Vec<BinaryProperty>,
    int_props: Vec<IntProperty>,
}

/// `INSTANCE`. A failure of Java's static initializer surfaces as
/// `ExceptionInInitializerError` (its `MissingResourceException`), here a
/// panic.
static INSTANCE: LazyLock<UCharacterProperty> = LazyLock::new(|| match UCharacterProperty::new() {
    Ok(p) => p,
    Err(e) => panic!(
        "java.lang.ExceptionInInitializerError: {}",
        missing_resource(e.to_string(), "", "")
    ),
});

// public data members -----------------------------------------------

/// `LATIN_CAPITAL_LETTER_I_WITH_DOT_ABOVE_`.
pub const LATIN_CAPITAL_LETTER_I_WITH_DOT_ABOVE: u16 = 0x130;
/// `LATIN_SMALL_LETTER_DOTLESS_I_`.
pub const LATIN_SMALL_LETTER_DOTLESS_I: u16 = 0x131;
/// `LATIN_SMALL_LETTER_I_`.
pub const LATIN_SMALL_LETTER_I: u16 = 0x69;
/// `TYPE_MASK`: character type mask.
pub const TYPE_MASK: i32 = 0x1F;

// uprops.h enum UPropertySource --------------------------------------- ***

/// `SRC_NONE`: No source, not a supported property.
pub const SRC_NONE: i32 = 0;
/// `SRC_CHAR`: From uchar.c/uprops.icu main trie.
pub const SRC_CHAR: i32 = 1;
/// `SRC_PROPSVEC`: From uchar.c/uprops.icu properties vectors trie.
pub const SRC_PROPSVEC: i32 = 2;
/// `SRC_NAMES`: From unames.c/unames.icu.
pub const SRC_NAMES: i32 = 3;
/// `SRC_CASE`: From ucase.c/ucase.icu.
pub const SRC_CASE: i32 = 4;
/// `SRC_BIDI`: From ubidi_props.c/ubidi.icu.
pub const SRC_BIDI: i32 = 5;
/// `SRC_CHAR_AND_PROPSVEC`: From uchar.c/uprops.icu main trie as well as
/// properties vectors trie.
pub const SRC_CHAR_AND_PROPSVEC: i32 = 6;
/// `SRC_CASE_AND_NORM`: From ucase.c/ucase.icu as well as unorm.cpp/unorm.icu.
pub const SRC_CASE_AND_NORM: i32 = 7;
/// `SRC_NFC`: From normalizer2impl.cpp/nfc.nrm.
pub const SRC_NFC: i32 = 8;
/// `SRC_NFKC`: From normalizer2impl.cpp/nfkc.nrm.
pub const SRC_NFKC: i32 = 9;
/// `SRC_NFKC_CF`: From normalizer2impl.cpp/nfkc_cf.nrm.
pub const SRC_NFKC_CF: i32 = 10;
/// `SRC_NFC_CANON_ITER`: From normalizer2impl.cpp/nfc.nrm canonical iterator data.
pub const SRC_NFC_CANON_ITER: i32 = 11;
// Text layout properties.
/// `SRC_INPC`.
pub const SRC_INPC: i32 = 12;
/// `SRC_INSC`.
pub const SRC_INSC: i32 = 13;
/// `SRC_VO`.
pub const SRC_VO: i32 = 14;
/// `SRC_EMOJI`.
pub const SRC_EMOJI: i32 = 15;
/// `SRC_IDSU`.
pub const SRC_IDSU: i32 = 16;
/// `SRC_ID_COMPAT_MATH`.
pub const SRC_ID_COMPAT_MATH: i32 = 17;
/// `SRC_BLOCK`.
pub const SRC_BLOCK: i32 = 18;
/// `SRC_MCM`.
pub const SRC_MCM: i32 = 19;
/// `SRC_COUNT`: One more than the highest UPropertySource (SRC_) constant.
pub const SRC_COUNT: i32 = 20;

/// `SCRIPT_X_MASK`.
pub const SCRIPT_X_MASK: i32 = 0x00000fff;
// SCRIPT_X_WITH_COMMON must be the lowest value that involves Script_Extensions.
/// `SCRIPT_X_WITH_OTHER`.
pub const SCRIPT_X_WITH_OTHER: i32 = 0xc00;
/// `SCRIPT_X_WITH_INHERITED`.
pub const SCRIPT_X_WITH_INHERITED: i32 = 0x800;
/// `SCRIPT_X_WITH_COMMON`.
pub const SCRIPT_X_WITH_COMMON: i32 = 0x400;
/// `MAX_SCRIPT`.
pub const MAX_SCRIPT: i32 = 0x3ff;

/// `MY_MASK` (package-private).
#[allow(dead_code)] // Java member with no caller in the closure
pub(crate) const MY_MASK: i32 = TYPE_MASK
    & ((1 << ECharacterCategory::UPPERCASE_LETTER)
        | (1 << ECharacterCategory::LOWERCASE_LETTER)
        | (1 << ECharacterCategory::TITLECASE_LETTER)
        | (1 << ECharacterCategory::MODIFIER_LETTER)
        | (1 << ECharacterCategory::OTHER_LETTER));

const GC_CN_MASK: i32 = 1 << ECharacterCategory::UNASSIGNED;
const GC_CC_MASK: i32 = 1 << ECharacterCategory::CONTROL;
const GC_CS_MASK: i32 = 1 << ECharacterCategory::SURROGATE;
const GC_ZS_MASK: i32 = 1 << ECharacterCategory::SPACE_SEPARATOR;
const GC_ZL_MASK: i32 = 1 << ECharacterCategory::LINE_SEPARATOR;
const GC_ZP_MASK: i32 = 1 << ECharacterCategory::PARAGRAPH_SEPARATOR;
/** Mask constant for multiple UCharCategory bits (Z Separators). */
const GC_Z_MASK: i32 = GC_ZS_MASK | GC_ZL_MASK | GC_ZP_MASK;

// ID_Compat_Math_Continue
static ID_COMPAT_MATH_CONTINUE: [i32; 10] = [
    0x00B2,
    0x00B3 + 1,
    0x00B9,
    0x00B9 + 1,
    0x2070,
    0x2070 + 1,
    0x2074,
    0x207E + 1,
    0x2080,
    0x208E + 1,
];

// ID_Compat_Math_Start
static ID_COMPAT_MATH_START: [i32; 13] = [
    0x2202, 0x2207, 0x221E, 0x1D6C1, 0x1D6DB, 0x1D6FB, 0x1D715, 0x1D735, 0x1D74F, 0x1D76F, 0x1D789,
    0x1D7A9, 0x1D7C3,
];

// Modifier_Combining_Mark
static MODIFIER_COMBINING_MARK: [i32; 18] = [
    0x0654,
    0x0655 + 1,
    0x0658,
    0x0658 + 1, // U+0658 was missing from Unicode 16 MCM.txt
    0x06DC,
    0x06DC + 1,
    0x06E3,
    0x06E3 + 1,
    0x06E7,
    0x06E8 + 1,
    0x08CA,
    0x08CB + 1,
    0x08CD,
    0x08CF + 1,
    0x08D3,
    0x08D3 + 1,
    0x08F3,
    0x08F3 + 1,
];

/// Checks whether `c` is graphic in the POSIX sense (`isgraphPOSIX`).
fn isgraph_posix(c: i32) -> bool {
    /* \p{space}\p{gc=Control} == \p{gc=Z}\p{Control} */
    /* comparing ==0 returns false for the categories mentioned */
    (get_mask(UCharacter::get_type(c)) & (GC_CC_MASK | GC_CS_MASK | GC_CN_MASK | GC_Z_MASK)) == 0
}

/// `getMask(int type)`: the mask bit of a general category.
pub fn get_mask(ty: i32) -> i32 {
    1 << ty
}

/// The kinds of Java's `BinaryProperty` and its anonymous subclasses.
#[derive(Debug, Clone, Copy)]
enum BinKind {
    /// `BinaryProperty(int column, int mask)`: a bit of a properties vector.
    Vector,
    BidiControl,
    BidiMirrored,
    FullCompositionExclusion,
    JoinControl,
    /// `CaseBinaryProperty(which)`.
    Case(i32),
    /// `NormInertBinaryProperty(source, which)`.
    NormInert(i32),
    SegmentStarter,
    PosixAlnum,
    PosixBlank,
    PosixGraph,
    PosixPrint,
    PosixXdigit,
    ChangesWhenCasefolded,
    ChangesWhenNfkcCasefolded,
    /// `EmojiBinaryProperty(which)`.
    Emoji(i32),
    RegionalIndicator,
    IdsUnaryOperator,
    /// `MathCompatBinaryProperty(which)`.
    MathCompat(i32),
    /// `MCMBinaryProperty`.
    Mcm,
}

/// `UCharacterProperty.BinaryProperty` (private inner class).
#[derive(Debug, Clone, Copy)]
struct BinaryProperty {
    column: i32, // SRC_PROPSVEC column, or "source" if mask==0
    mask: i32,
    kind: BinKind,
}

impl BinaryProperty {
    const fn vec(column: i32, mask: i32) -> Self {
        Self {
            column,
            mask,
            kind: BinKind::Vector,
        }
    }

    const fn src(source: i32, kind: BinKind) -> Self {
        Self {
            column: source,
            mask: 0,
            kind,
        }
    }

    fn get_source(&self) -> i32 {
        if self.mask == 0 {
            self.column
        } else {
            SRC_PROPSVEC
        }
    }

    fn contains(&self, upc: &UCharacterProperty, c: i32) -> bool {
        match self.kind {
            // systematic, directly stored properties
            BinKind::Vector => (upc.get_additional(c, self.column) & self.mask) != 0,
            BinKind::BidiControl => UBiDiProps::instance().is_bidi_control(c),
            BinKind::BidiMirrored => UBiDiProps::instance().is_mirrored(c),
            BinKind::FullCompositionExclusion => {
                // By definition, Full_Composition_Exclusion is the same as NFC_QC=No.
                let impl_ = &Norm2AllModes::get_nfc_instance().expect(NORM_DATA).impl_;
                impl_.is_comp_no(impl_.get_norm16(c))
            }
            BinKind::JoinControl => UBiDiProps::instance().is_join_control(c),
            BinKind::Case(which) => UCaseProps::instance().has_binary_property(c, which),
            BinKind::NormInert(which) => {
                Norm2AllModes::get_n2_with_impl(which - UProperty::NFD_INERT)
                    .expect(NORM_DATA)
                    .is_inert(c)
            }
            BinKind::SegmentStarter => Norm2AllModes::get_nfc_instance()
                .expect(NORM_DATA)
                .impl_
                .ensure_canon_iter_data()
                .expect(NORM_DATA)
                .is_canon_segment_starter(c),
            BinKind::PosixAlnum => UCharacter::is_u_alphabetic(c) || UCharacter::is_digit(c),
            BinKind::PosixBlank => {
                // "horizontal space"
                if c <= 0x9f {
                    c == 9 || c == 0x20 /* TAB or SPACE */
                } else {
                    /* Zs */
                    UCharacter::get_type(c) == ECharacterCategory::SPACE_SEPARATOR as i32
                }
            }
            BinKind::PosixGraph => isgraph_posix(c),
            BinKind::PosixPrint => {
                // Checks whether ch is printable in the POSIX sense: graph||space
                /* \p{space}\p{gc=Control} == \p{gc=Z}\p{Control} */
                (UCharacter::get_type(c) == ECharacterCategory::SPACE_SEPARATOR as i32)
                    || isgraph_posix(c)
            }
            BinKind::PosixXdigit => {
                /* check ASCII and Fullwidth ASCII a-fA-F */
                if ((0x41..=0x66).contains(&c) && (c <= 0x46 || c >= 0x61))
                    || ((0xff21..=0xff46).contains(&c) && (c <= 0xff26 || c >= 0xff41))
                {
                    return true;
                }
                UCharacter::get_type(c) == ECharacterCategory::DECIMAL_DIGIT_NUMBER as i32
            }
            BinKind::ChangesWhenCasefolded => {
                let nfd = Norm2AllModes::get_nfc_instance()
                    .expect(NORM_DATA)
                    .impl_
                    .get_decomposition(c);
                let mut c = c;
                if let Some(nfd) = &nfd {
                    /* c has a decomposition */
                    c = code_point_at(nfd, 0);
                    if char_count(c) != nfd.len() as i32 {
                        /* multiple code points */
                        c = -1;
                    }
                } else if c < 0 {
                    return false; /* protect against bad input */
                }
                if c >= 0 {
                    /* single code point */
                    let csp = UCaseProps::instance();
                    // Java writes into the shared UCaseProps.dummyStringBuilder.
                    let mut dummy = Vec::new();
                    csp.to_full_folding(c, &mut dummy, UCharacter::FOLD_CASE_DEFAULT) >= 0
                } else {
                    let nfd = nfd.expect("c < 0 only with a decomposition");
                    let folded = UCharacter::fold_case_str(&nfd, true);
                    folded != nfd
                }
            }
            BinKind::ChangesWhenNfkcCasefolded => {
                let kcf = &Norm2AllModes::get_nfkc_cf_instance()
                    .expect(NORM_DATA)
                    .impl_;
                let src = UTF16::value_of(c).expect("a valid code point");
                let mut dest = Vec::new();
                // Small destCapacity for NFKC_CaseFold.
                // NFKC_CaseFold maps the old Hangul syllable to a single
                // new one, so it can grow at most 2x in length.
                {
                    let mut buffer = ReorderingBuffer::new(kcf, &mut dest, 5);
                    kcf.compose(&src, 0, src.len() as i32, false, true, &mut buffer);
                }
                !UTF16Plus::equal(&dest, &src)
            }
            BinKind::Emoji(which) => EmojiProps::instance().has_binary_property(c, which),
            BinKind::RegionalIndicator => {
                // Regional_Indicator
                // Code points are hardcoded.
                (0x1F1E6..=0x1F1FF).contains(&c)
            }
            BinKind::IdsUnaryOperator => {
                // IDS_Unary_Operator
                // Code points are hardcoded.
                (0x2FFE..=0x2FFF).contains(&c)
            }
            BinKind::MathCompat(which) => {
                if which == UProperty::ID_COMPAT_MATH_CONTINUE {
                    let mut i = 0;
                    while i < ID_COMPAT_MATH_CONTINUE.len() {
                        if c < ID_COMPAT_MATH_CONTINUE[i] {
                            return false;
                        } // below range start
                        if c < ID_COMPAT_MATH_CONTINUE[i + 1] {
                            return true;
                        } // below range limit
                        i += 2;
                    }
                }
                if c < ID_COMPAT_MATH_START[0] {
                    return false;
                } // fastpath for common characters
                ID_COMPAT_MATH_START.contains(&c)
            }
            BinKind::Mcm => {
                let mut i = 0;
                while i < MODIFIER_COMBINING_MARK.len() {
                    if c < MODIFIER_COMBINING_MARK[i] {
                        return false;
                    } // below range start
                    if c < MODIFIER_COMBINING_MARK[i + 1] {
                        return true;
                    } // below range limit
                    i += 2;
                }
                false
            }
        }
    }
}

/// The kinds of Java's `IntProperty` and its anonymous subclasses.
///
/// The private subclasses `UCharacterProperty.BiDiIntProperty` (source
/// `SRC_BIDI`, `getMaxValue` from `UBiDiProps`: [`IntKind::BidiClass`],
/// [`IntKind::JoiningGroup`], [`IntKind::JoiningType`],
/// [`IntKind::BidiPairedBracketType`]) and
/// `UCharacterProperty.CombiningClassIntProperty` (`getMaxValue` `0xff`:
/// [`IntKind::CanonicalCombiningClass`],
/// [`IntKind::LeadCanonicalCombiningClass`],
/// [`IntKind::TrailCanonicalCombiningClass`]) are the kinds their anonymous
/// subclasses became (`UCharacterProperty.java:728-746,767-885`); their
/// overrides are the matching arms of `IntProperty::get_max_value`.
#[derive(Debug, Clone, Copy)]
enum IntKind {
    /// `IntProperty(int column, int mask, int shift)`.
    Vector,
    BidiClass,
    Block,
    CanonicalCombiningClass,
    GeneralCategory,
    JoiningGroup,
    JoiningType,
    NumericType,
    Script,
    HangulSyllableType,
    /// `NormQuickCheckIntProperty(source, which, max)`.
    NormQuickCheck(i32, i32),
    LeadCanonicalCombiningClass,
    TrailCanonicalCombiningClass,
    BidiPairedBracketType,
    Inpc,
    Insc,
    Vo,
    IdentifierStatus,
}

/// `UCharacterProperty.IntProperty` (private inner class).
#[derive(Debug, Clone, Copy)]
struct IntProperty {
    column: i32, // SRC_PROPSVEC column, or "source" if mask==0
    mask: i32,
    shift: i32,
    kind: IntKind,
}

impl IntProperty {
    const fn vec(column: i32, mask: i32, shift: i32) -> Self {
        Self {
            column,
            mask,
            shift,
            kind: IntKind::Vector,
        }
    }

    const fn src(source: i32, kind: IntKind) -> Self {
        Self {
            column: source,
            mask: 0,
            shift: 0,
            kind,
        }
    }

    fn get_source(&self) -> i32 {
        if self.mask == 0 {
            self.column
        } else {
            SRC_PROPSVEC
        }
    }

    fn get_value(&self, upc: &UCharacterProperty, c: i32) -> i32 {
        match self.kind {
            // systematic, directly stored properties
            IntKind::Vector => {
                ((upc.get_additional(c, self.column) & self.mask) as u32 >> self.shift) as i32
            }
            IntKind::BidiClass => UBiDiProps::instance().get_class(c),
            IntKind::Block => {
                // We store Block values indexed by the code point shifted right 4 bits
                // and use a "small" UCPTrie=CodePointTrie for minimal data size.
                // This works because blocks have xxx0..xxxF ranges.
                let mut c4 = c;
                // Shift unless out of range, in which case we fetch the trie's error value.
                if c4 <= 0x10ffff {
                    c4 >>= 4;
                }
                upc.m_block_trie.get(c4)
            }
            IntKind::CanonicalCombiningClass => normalizer2::get_nfd_instance()
                .expect(NORM_DATA)
                .get_combining_class(c),
            IntKind::GeneralCategory => upc.get_type(c),
            IntKind::JoiningGroup => UBiDiProps::instance().get_joining_group(c),
            IntKind::JoiningType => UBiDiProps::instance().get_joining_type(c),
            IntKind::NumericType => ntv_get_type(get_numeric_type_value(upc.get_property(c))),
            IntKind::Script => UScript::get_script(c).expect("a code point in range"),
            IntKind::HangulSyllableType => {
                /* see comments on gcbToHst[] above */
                /* see comments on gcbToHst[] above */
                if c > 0xffff {
                    return HangulSyllableType::NOT_APPLICABLE;
                }
                let gcb = ((upc.get_additional(c, 2) & GCB_MASK) as u32 >> GCB_SHIFT) as i32;
                if (gcb as usize) < GCB_TO_HST.len() {
                    GCB_TO_HST[gcb as usize]
                } else {
                    HangulSyllableType::NOT_APPLICABLE
                }
            }
            IntKind::NormQuickCheck(which, _) => {
                Norm2AllModes::get_n2_with_impl(which - UProperty::NFD_QUICK_CHECK)
                    .expect(NORM_DATA)
                    .get_quick_check(c)
            }
            IntKind::LeadCanonicalCombiningClass => {
                Norm2AllModes::get_nfc_instance()
                    .expect(NORM_DATA)
                    .impl_
                    .get_fcd16(c)
                    >> 8
            }
            IntKind::TrailCanonicalCombiningClass => {
                Norm2AllModes::get_nfc_instance()
                    .expect(NORM_DATA)
                    .impl_
                    .get_fcd16(c)
                    & 0xff
            }
            IntKind::BidiPairedBracketType => UBiDiProps::instance().get_paired_bracket_type(c),
            IntKind::Inpc => LayoutProps::instance()
                .inpc_trie
                .as_ref()
                .map_or(0, |t| t.get(c)),
            IntKind::Insc => LayoutProps::instance()
                .insc_trie
                .as_ref()
                .map_or(0, |t| t.get(c)),
            IntKind::Vo => LayoutProps::instance()
                .vo_trie
                .as_ref()
                .map_or(0, |t| t.get(c)),
            IntKind::IdentifierStatus => {
                let value = ((upc.get_additional(c, 2) as u32) >> ID_TYPE_SHIFT) as i32;
                if value >= ID_TYPE_ALLOWED_MIN {
                    IdentifierStatus::Allowed.ordinal()
                } else {
                    IdentifierStatus::Restricted.ordinal()
                }
            }
        }
    }

    fn get_max_value(&self, upc: &UCharacterProperty, which: i32) -> i32 {
        match self.kind {
            IntKind::BidiClass
            | IntKind::JoiningGroup
            | IntKind::JoiningType
            | IntKind::BidiPairedBracketType => UBiDiProps::instance().get_max_value(which),
            IntKind::Block => upc.m_max_values_other & MAX_BLOCK,
            IntKind::CanonicalCombiningClass
            | IntKind::LeadCanonicalCombiningClass
            | IntKind::TrailCanonicalCombiningClass => 0xff,
            IntKind::GeneralCategory => ECharacterCategory::CHAR_CATEGORY_COUNT as i32 - 1,
            IntKind::NumericType => NumericType::COUNT - 1,
            IntKind::Script => upc.get_max_values(0) & MAX_SCRIPT,
            IntKind::HangulSyllableType => HangulSyllableType::COUNT - 1,
            IntKind::NormQuickCheck(_, max) => max,
            IntKind::Inpc => LayoutProps::instance().max_inpc_value,
            IntKind::Insc => LayoutProps::instance().max_insc_value,
            IntKind::Vo => LayoutProps::instance().max_vo_value,
            IntKind::IdentifierStatus => IdentifierStatus::Allowed.ordinal(),
            IntKind::Vector => {
                ((upc.get_max_values(self.column) & self.mask) as u32 >> self.shift) as i32
            }
        }
    }
}

/*
 * Map some of the Grapheme Cluster Break values to Hangul Syllable Types.
 * Hangul_Syllable_Type is fully redundant with a subset of Grapheme_Cluster_Break.
 */
static GCB_TO_HST: [i32; 10] = [
    HangulSyllableType::NOT_APPLICABLE, /* U_GCB_OTHER */
    HangulSyllableType::NOT_APPLICABLE, /* U_GCB_CONTROL */
    HangulSyllableType::NOT_APPLICABLE, /* U_GCB_CR */
    HangulSyllableType::NOT_APPLICABLE, /* U_GCB_EXTEND */
    HangulSyllableType::LEADING_JAMO,   /* U_GCB_L */
    HangulSyllableType::NOT_APPLICABLE, /* U_GCB_LF */
    HangulSyllableType::LV_SYLLABLE,    /* U_GCB_LV */
    HangulSyllableType::LVT_SYLLABLE,   /* U_GCB_LVT */
    HangulSyllableType::TRAILING_JAMO,  /* U_GCB_T */
    HangulSyllableType::VOWEL_JAMO,     /* U_GCB_V */
                                        /*
                                         * Omit GCB values beyond what we need for hst.
                                         * The code below checks for the array length.
                                         */
];

// private variables -------------------------------------------------

/// Default name of the datafile
const DATA_FILE_NAME: &str = "uprops.icu";

/// Shift value for lead surrogate to form a supplementary character.
const NUMERIC_TYPE_VALUE_SHIFT: i32 = 6;
fn get_numeric_type_value(props: i32) -> i32 {
    props >> NUMERIC_TYPE_VALUE_SHIFT
}
/* constants for the storage form of numeric types and values */
/** No numeric value. */
const NTV_NONE: i32 = 0;
/** Decimal digits: nv=0..9 */
const NTV_DECIMAL_START: i32 = 1;
/** Other digits: nv=0..9 */
const NTV_DIGIT_START: i32 = 11;
/** Small integers: nv=0..154 */
const NTV_NUMERIC_START: i32 = 21;
/** Fractions: ((ntv>>4)-12) / ((ntv&0xf)+1) = -1..17 / 1..16 */
const NTV_FRACTION_START: i32 = 0xb0;
/**
 * Large integers:
 * ((ntv>>5)-14) * 10^((ntv&0x1f)+2) = (1..9)*(10^2..10^33)
 * (only one significant decimal digit)
 */
const NTV_LARGE_START: i32 = 0x1e0;
/**
 * Sexagesimal numbers:
 * ((ntv>>2)-0xbf) * 60^((ntv&3)+1) = (1..9)*(60^1..60^4)
 */
const NTV_BASE60_START: i32 = 0x300;
/**
 * Fraction-20 values:
 * frac20 = ntv-0x324 = 0..0x17 -> 1|3|5|7 / 20|40|80|160|320|640
 * numerator: num = 2*(frac20&3)+1
 * denominator: den = 20<<(frac20>>2)
 */
const NTV_FRACTION20_START: i32 = NTV_BASE60_START + 36; // 0x300+9*4=0x324
/**
 * Fraction-32 values:
 * frac32 = ntv-0x34c = 0..15 -> 1|3|5|7 / 32|64|128|256
 * numerator: num = 2*(frac32&3)+1
 * denominator: den = 32<<(frac32>>2)
 */
const NTV_FRACTION32_START: i32 = NTV_FRACTION20_START + 24; // 0x324+6*4=0x34c
/** No numeric value (yet). */
const NTV_RESERVED_START: i32 = NTV_FRACTION32_START + 16; // 0x34c+4*4=0x35c

fn ntv_get_type(ntv: i32) -> i32 {
    if ntv == NTV_NONE {
        NumericType::NONE
    } else if ntv < NTV_DIGIT_START {
        NumericType::DECIMAL
    } else if ntv < NTV_NUMERIC_START {
        NumericType::DIGIT
    } else {
        NumericType::NUMERIC
    }
}

/*
 * Properties in vector word 0
 * Bits
 * 31..24   DerivedAge version major/minor one nibble each
 * 23..22   3..1: Bits 21..20 & 7..0 = Script_Extensions index
 *             3: Script value from Script_Extensions
 *             2: Script=Inherited
 *             1: Script=Common
 *             0: Script=bits 21..20 & 7..0
 * 21..20   Bits 9..8 of the UScriptCode, or index to Script_Extensions
 * 19..17   East Asian Width
 * 16.. 8   UBlockCode
 *  7.. 0   UScriptCode, or index to Script_Extensions
 */

/**
 * Script_Extensions: mask includes Script
 */
/** Effective mask for the East Asian Width property. */
const EAST_ASIAN_MASK: i32 = 0x00007000;
/** Shift for the East Asian Width property. */
const EAST_ASIAN_SHIFT: i32 = 12;

const INCB_MASK: i32 = 0x00018000;
const INCB_SHIFT: i32 = 15;

/**
 * Additional properties used in internal trie data
 */
/*
 * Properties in vector word 1
 * Each bit encodes one binary property.
 * The following constants represent the bit number, use 1<<UPROPS_XYZ.
 * UPROPS_BINARY_1_TOP<=32!
 *
 * Keep this list of property enums in sync with
 * propListNames[] in icu/source/tools/genprops/props2.c!
 *
 * ICU 2.6/uprops format version 3.2 stores full properties instead of "Other_".
 */
const WHITE_SPACE_PROPERTY: i32 = 0;
const DASH_PROPERTY: i32 = 1;
const HYPHEN_PROPERTY: i32 = 2;
const QUOTATION_MARK_PROPERTY: i32 = 3;
const TERMINAL_PUNCTUATION_PROPERTY: i32 = 4;
const MATH_PROPERTY: i32 = 5;
const HEX_DIGIT_PROPERTY: i32 = 6;
const ASCII_HEX_DIGIT_PROPERTY: i32 = 7;
const ALPHABETIC_PROPERTY: i32 = 8;
const IDEOGRAPHIC_PROPERTY: i32 = 9;
const DIACRITIC_PROPERTY: i32 = 10;
const EXTENDER_PROPERTY: i32 = 11;
const NONCHARACTER_CODE_POINT_PROPERTY: i32 = 12;
const GRAPHEME_EXTEND_PROPERTY: i32 = 13;
const GRAPHEME_LINK_PROPERTY: i32 = 14;
const IDS_BINARY_OPERATOR_PROPERTY: i32 = 15;
const IDS_TRINARY_OPERATOR_PROPERTY: i32 = 16;
const RADICAL_PROPERTY: i32 = 17;
const UNIFIED_IDEOGRAPH_PROPERTY: i32 = 18;
const DEFAULT_IGNORABLE_CODE_POINT_PROPERTY: i32 = 19;
const DEPRECATED_PROPERTY: i32 = 20;
const LOGICAL_ORDER_EXCEPTION_PROPERTY: i32 = 21;
const XID_START_PROPERTY: i32 = 22;
const XID_CONTINUE_PROPERTY: i32 = 23;
const ID_START_PROPERTY: i32 = 24;
const ID_CONTINUE_PROPERTY: i32 = 25;
const GRAPHEME_BASE_PROPERTY: i32 = 26;
const S_TERM_PROPERTY: i32 = 27;
const VARIATION_SELECTOR_PROPERTY: i32 = 28;
const PATTERN_SYNTAX: i32 = 29; /* new in ICU 3.4 and Unicode 4.1 */
const PATTERN_WHITE_SPACE: i32 = 30;
const PREPENDED_CONCATENATION_MARK: i32 = 31; // new in ICU 60 and Unicode 10

/*
 * Properties in vector word 2
 * Bits
 * 31..26   ICU 75: Identifier_Type bit set
 *          ICU 70..74: unused
 *          ICU 57..69: emoji properties; moved to uemoji.icu in ICU 70
 * 25..20   Line Break
 * 19..15   Sentence Break
 * 14..10   Word Break
 *  9.. 5   Grapheme Cluster Break
 *  4.. 0   Decomposition Type
 */

//ivate static final int ID_TYPE_MASK = 0xfc000000;
const ID_TYPE_SHIFT: u32 = 26;

// A high bit for use in idTypeToEncoded[] but not used in the data
const ID_TYPE_BIT: i32 = 0x80;

// Identifier_Type bit set
// If ID_TYPE_RESTRICTED is set, then one or more of the other bits identify specific types.
const ID_TYPE_EXCLUSION: i32 = 0x20;
const ID_TYPE_LIMITED_USE: i32 = 0x10;
const ID_TYPE_UNCOMMON_USE: i32 = 8;
const ID_TYPE_TECHNICAL: i32 = 4;
const ID_TYPE_OBSOLETE: i32 = 2;
const ID_TYPE_NOT_XID: i32 = 1;

// Single-value Identifier_Type encodings
const ID_TYPE_NOT_CHARACTER: i32 = 0;
// Bits up to and including ID_TYPE_RESTRICTED are bit sets.
// Higher values are single types.
const ID_TYPE_FORBIDDEN: i32 = ID_TYPE_EXCLUSION | ID_TYPE_LIMITED_USE;
const ID_TYPE_DEPRECATED: i32 = ID_TYPE_FORBIDDEN;
const ID_TYPE_DEFAULT_IGNORABLE: i32 = ID_TYPE_FORBIDDEN + 1;
const ID_TYPE_NOT_NFKC: i32 = ID_TYPE_FORBIDDEN + 2;
// Identifier_Status=Allowed
const ID_TYPE_ALLOWED_MIN: i32 = ID_TYPE_FORBIDDEN + 0xc;
const ID_TYPE_INCLUSION: i32 = ID_TYPE_FORBIDDEN + 0xe;
const ID_TYPE_RECOMMENDED: i32 = ID_TYPE_FORBIDDEN + 0xf;

/**
 * Maps UIdentifierType to encoded value.
 * The encoded value is the Identifier_Type bit set,
 * or a single type value (ID_TYPE_DEPRECATED...),
 * or ID_TYPE_RECOMMENDED.
 */
static ID_TYPE_TO_ENCODED: [i32; 12] = [
    ID_TYPE_NOT_CHARACTER,
    ID_TYPE_DEPRECATED,
    ID_TYPE_DEFAULT_IGNORABLE,
    ID_TYPE_NOT_NFKC,
    ID_TYPE_BIT | ID_TYPE_NOT_XID,
    ID_TYPE_BIT | ID_TYPE_EXCLUSION,
    ID_TYPE_BIT | ID_TYPE_OBSOLETE,
    ID_TYPE_BIT | ID_TYPE_TECHNICAL,
    ID_TYPE_BIT | ID_TYPE_UNCOMMON_USE,
    ID_TYPE_BIT | ID_TYPE_LIMITED_USE,
    ID_TYPE_INCLUSION,
    ID_TYPE_RECOMMENDED,
];

const LB_MASK: i32 = 0x03f00000;
const LB_SHIFT: i32 = 20;

const SB_MASK: i32 = 0x000f8000;
const SB_SHIFT: i32 = 15;

const WB_MASK: i32 = 0x00007c00;
const WB_SHIFT: i32 = 10;

const GCB_MASK: i32 = 0x000003e0;
const GCB_SHIFT: i32 = 5;

/**
 * Integer properties mask for decomposition type.
 * Equivalent to icu4c UPROPS_DT_MASK.
 */
const DECOMPOSITION_TYPE_MASK: i32 = 0x0000001f;

/** First nibble shift */
const AGE_SHIFT: u32 = 24;

// Bits 9..0 in UPROPS_MAX_VALUES_OTHER_INDEX
const MAX_BLOCK: i32 = 0x3ff;

/**
 * Data format "UPro".
 */
const DATA_FORMAT: i32 = 0x5550726F;

/// Whether the data file's format version is supported.
///
/// Equivalent to the private class `com.ibm.icu.impl.UCharacterProperty.IsAcceptable`,
/// whose `UCharacterProperty.IsAcceptable.isDataVersionAcceptable(byte[])` is
/// this function (an `ICUBinary.Authenticate`).
fn is_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 9
}

// private methods -------------------------------------------------------

/*
 * Compare additional properties to see if it has argument type
 * @param property 32 bit properties
 * @param type character type
 * @return true if property has type
 */
/*private boolean compareAdditionalType(int property, int type)
{
    return (property & (1 << type)) != 0;
}*/

// property starts for UnicodeSet -------------------------------------- ***

const TAB: i32 = 0x0009;
//private static final int LF      = 0x000a;
//private static final int FF      = 0x000c;
const CR: i32 = 0x000d;
const U_A: i32 = 0x0041;
const U_F: i32 = 0x0046;
const U_Z: i32 = 0x005a;
const U_A_LOWER: i32 = 0x0061;
const U_F_LOWER: i32 = 0x0066;
const U_Z_LOWER: i32 = 0x007a;
const DEL: i32 = 0x007f;
const NL: i32 = 0x0085;
const NBSP: i32 = 0x00a0;
const CGJ: i32 = 0x034f;
const FIGURESP: i32 = 0x2007;
const HAIRSP: i32 = 0x200a;
//private static final int ZWNJ    = 0x200c;
//private static final int ZWJ     = 0x200d;
const RLM: i32 = 0x200f;
const NNBSP: i32 = 0x202f;
const WJ: i32 = 0x2060;
const INHSWAP: i32 = 0x206a;
const NOMDIG: i32 = 0x206f;
const U_FW_A: i32 = 0xff21;
const U_FW_F: i32 = 0xff26;
const U_FW_Z: i32 = 0xff3a;
const U_FW_A_LOWER: i32 = 0xff41;
const U_FW_F_LOWER: i32 = 0xff46;
const U_FW_Z_LOWER: i32 = 0xff5a;
const ZWNBSP: i32 = 0xfeff;

impl UCharacterProperty {
    // constructor -----------------------------------------------------

    /// Constructor; reads the data file.
    fn new() -> Result<Self> {
        let bin_props = Self::bin_props();
        let int_props = Self::int_props();
        // consistency check
        if bin_props.len() as i32 != UProperty::BINARY_LIMIT {
            return Err(
                ICUException::with_message("binProps.length!=UProperty.BINARY_LIMIT").error(),
            );
        }
        if int_props.len() as i32 != (UProperty::INT_LIMIT - UProperty::INT_START) {
            return Err(ICUException::with_message(
                "intProps.length!=(UProperty.INT_LIMIT-UProperty.INT_START)",
            )
            .error());
        }

        // jar access
        let mut bytes = ICUBinary::get_required_data(DATA_FILE_NAME)?;
        let m_unicode_version = ICUBinary::read_header_and_data_version(
            &mut bytes,
            DATA_FORMAT,
            Some(&is_acceptable as &dyn Fn(&[u8; 4]) -> bool),
        )?;
        // Read or skip the 16 indexes.
        let property_offset = bytes.get_int()?;
        /* exceptionOffset = */
        bytes.get_int()?;
        /* caseOffset = */
        bytes.get_int()?;
        let additional_offset = bytes.get_int()?;
        let additional_vectors_offset = bytes.get_int()?;
        let m_additional_columns_count = bytes.get_int()?;
        let script_extensions_offset = bytes.get_int()?;
        let block_trie_offset = bytes.get_int()?;
        let reserved_offset8 = bytes.get_int()?;
        /* dataTopOffset = */
        bytes.get_int()?;
        let m_max_block_script_value = bytes.get_int()?;
        let m_max_jtg_value = bytes.get_int()?;
        let m_max_values_other = bytes.get_int()?;
        ICUBinary::skip_bytes(&mut bytes, (16 - 13) << 2)?;

        // read the main properties trie
        let m_trie = Trie2_16::create_from_serialized(&mut bytes)?;
        let mut expected_trie_length = (property_offset - 16) * 4;
        let mut trie_length = m_trie.get_serialized_length();
        if trie_length > expected_trie_length {
            return Err(io_exception("uprops.icu: not enough bytes for main trie"));
        }
        // skip padding after trie bytes
        ICUBinary::skip_bytes(&mut bytes, expected_trie_length - trie_length)?;

        // skip unused intervening data structures
        ICUBinary::skip_bytes(&mut bytes, (additional_offset - property_offset) * 4)?;

        let mut m_additional_trie = None;
        let mut m_additional_vectors = Vec::new();
        if m_additional_columns_count > 0 {
            // reads the additional property block
            let t = Trie2_16::create_from_serialized(&mut bytes)?;
            expected_trie_length = (additional_vectors_offset - additional_offset) * 4;
            trie_length = t.get_serialized_length();
            if trie_length > expected_trie_length {
                return Err(io_exception(
                    "uprops.icu: not enough bytes for additional-properties trie",
                ));
            }
            // skip padding after trie bytes
            ICUBinary::skip_bytes(&mut bytes, expected_trie_length - trie_length)?;
            m_additional_trie = Some(t);

            // additional properties
            let size = script_extensions_offset - additional_vectors_offset;
            m_additional_vectors = ICUBinary::get_ints(&mut bytes, size, 0)?;
        }

        // Script_Extensions
        let num_chars = (block_trie_offset - script_extensions_offset) * 2;
        let m_script_extensions = if num_chars > 0 {
            ICUBinary::get_chars(&mut bytes, num_chars, 0)?
        } else {
            Vec::new()
        };

        // Read the blockTrie.
        let part_length = (reserved_offset8 - block_trie_offset) * 4;
        let trie_position = bytes.position();
        let m_block_trie = CodePointTrie::from_binary(None, Some(ValueWidth::Bits16), &mut bytes)?;
        trie_length = bytes.position() - trie_position;
        if trie_length > part_length {
            return Err(ICUUncheckedIOException::with_message(
                "uprops.icu: not enough bytes for blockTrie",
            )
            .error());
        }
        ICUBinary::skip_bytes(&mut bytes, part_length - trie_length)?; // skip padding after trie bytes

        Ok(Self {
            m_trie,
            m_unicode_version,
            m_additional_trie,
            m_additional_vectors,
            m_additional_columns_count,
            m_max_block_script_value,
            m_max_jtg_value,
            m_max_values_other,
            m_script_extensions,
            m_block_trie,
            bin_props,
            int_props,
        })
    }

    /// `INSTANCE`.
    ///
    /// # Panics
    ///
    /// When the data cannot be read (Java's `ExceptionInInitializerError`).
    pub fn instance() -> &'static UCharacterProperty {
        &INSTANCE
    }

    /// `binProps` (Java's instance field initializer, indexed by
    /// `UProperty` binary property).
    fn bin_props() -> Vec<BinaryProperty> {
        use BinKind as K;
        use BinaryProperty as B;
        vec![
            /*
             * Binary-property implementations must be in order of corresponding UProperty,
             * and there must be exactly one entry per binary UProperty.
             */
            B::vec(1, 1 << ALPHABETIC_PROPERTY),
            B::vec(1, 1 << ASCII_HEX_DIGIT_PROPERTY),
            B::src(SRC_BIDI, K::BidiControl),
            B::src(SRC_BIDI, K::BidiMirrored),
            B::vec(1, 1 << DASH_PROPERTY),
            B::vec(1, 1 << DEFAULT_IGNORABLE_CODE_POINT_PROPERTY),
            B::vec(1, 1 << DEPRECATED_PROPERTY),
            B::vec(1, 1 << DIACRITIC_PROPERTY),
            B::vec(1, 1 << EXTENDER_PROPERTY),
            B::src(SRC_NFC, K::FullCompositionExclusion),
            B::vec(1, 1 << GRAPHEME_BASE_PROPERTY),
            B::vec(1, 1 << GRAPHEME_EXTEND_PROPERTY),
            B::vec(1, 1 << GRAPHEME_LINK_PROPERTY),
            B::vec(1, 1 << HEX_DIGIT_PROPERTY),
            B::vec(1, 1 << HYPHEN_PROPERTY),
            B::vec(1, 1 << ID_CONTINUE_PROPERTY),
            B::vec(1, 1 << ID_START_PROPERTY),
            B::vec(1, 1 << IDEOGRAPHIC_PROPERTY),
            B::vec(1, 1 << IDS_BINARY_OPERATOR_PROPERTY),
            B::vec(1, 1 << IDS_TRINARY_OPERATOR_PROPERTY),
            B::src(SRC_BIDI, K::JoinControl),
            B::vec(1, 1 << LOGICAL_ORDER_EXCEPTION_PROPERTY),
            B::src(SRC_CASE, K::Case(UProperty::LOWERCASE)),
            B::vec(1, 1 << MATH_PROPERTY),
            B::vec(1, 1 << NONCHARACTER_CODE_POINT_PROPERTY),
            B::vec(1, 1 << QUOTATION_MARK_PROPERTY),
            B::vec(1, 1 << RADICAL_PROPERTY),
            B::src(SRC_CASE, K::Case(UProperty::SOFT_DOTTED)),
            B::vec(1, 1 << TERMINAL_PUNCTUATION_PROPERTY),
            B::vec(1, 1 << UNIFIED_IDEOGRAPH_PROPERTY),
            B::src(SRC_CASE, K::Case(UProperty::UPPERCASE)),
            B::vec(1, 1 << WHITE_SPACE_PROPERTY),
            B::vec(1, 1 << XID_CONTINUE_PROPERTY),
            B::vec(1, 1 << XID_START_PROPERTY),
            B::src(SRC_CASE, K::Case(UProperty::CASE_SENSITIVE)),
            B::vec(1, 1 << S_TERM_PROPERTY),
            B::vec(1, 1 << VARIATION_SELECTOR_PROPERTY),
            B::src(SRC_NFC, K::NormInert(UProperty::NFD_INERT)),
            B::src(SRC_NFKC, K::NormInert(UProperty::NFKD_INERT)),
            B::src(SRC_NFC, K::NormInert(UProperty::NFC_INERT)),
            B::src(SRC_NFKC, K::NormInert(UProperty::NFKC_INERT)),
            B::src(SRC_NFC_CANON_ITER, K::SegmentStarter),
            B::vec(1, 1 << PATTERN_SYNTAX),
            B::vec(1, 1 << PATTERN_WHITE_SPACE),
            B::src(SRC_CHAR_AND_PROPSVEC, K::PosixAlnum),
            B::src(SRC_CHAR, K::PosixBlank),
            B::src(SRC_CHAR, K::PosixGraph),
            B::src(SRC_CHAR, K::PosixPrint),
            B::src(SRC_CHAR, K::PosixXdigit),
            B::src(SRC_CASE, K::Case(UProperty::CASED)),
            B::src(SRC_CASE, K::Case(UProperty::CASE_IGNORABLE)),
            B::src(SRC_CASE, K::Case(UProperty::CHANGES_WHEN_LOWERCASED)),
            B::src(SRC_CASE, K::Case(UProperty::CHANGES_WHEN_UPPERCASED)),
            B::src(SRC_CASE, K::Case(UProperty::CHANGES_WHEN_TITLECASED)),
            B::src(SRC_CASE_AND_NORM, K::ChangesWhenCasefolded),
            B::src(SRC_CASE, K::Case(UProperty::CHANGES_WHEN_CASEMAPPED)),
            B::src(SRC_NFKC_CF, K::ChangesWhenNfkcCasefolded),
            B::src(SRC_EMOJI, K::Emoji(UProperty::EMOJI)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::EMOJI_PRESENTATION)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::EMOJI_MODIFIER)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::EMOJI_MODIFIER_BASE)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::EMOJI_COMPONENT)),
            B::src(SRC_PROPSVEC, K::RegionalIndicator),
            B::vec(1, 1 << PREPENDED_CONCATENATION_MARK),
            B::src(SRC_EMOJI, K::Emoji(UProperty::EXTENDED_PICTOGRAPHIC)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::BASIC_EMOJI)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::EMOJI_KEYCAP_SEQUENCE)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::RGI_EMOJI_MODIFIER_SEQUENCE)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::RGI_EMOJI_FLAG_SEQUENCE)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::RGI_EMOJI_TAG_SEQUENCE)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::RGI_EMOJI_ZWJ_SEQUENCE)),
            B::src(SRC_EMOJI, K::Emoji(UProperty::RGI_EMOJI)),
            B::src(SRC_IDSU, K::IdsUnaryOperator),
            B::src(
                SRC_ID_COMPAT_MATH,
                K::MathCompat(UProperty::ID_COMPAT_MATH_START),
            ),
            B::src(
                SRC_ID_COMPAT_MATH,
                K::MathCompat(UProperty::ID_COMPAT_MATH_CONTINUE),
            ),
            B::src(SRC_MCM, K::Mcm),
        ]
    }

    /// `intProps` (Java's instance field initializer, indexed by
    /// `which - UProperty.INT_START`).
    fn int_props() -> Vec<IntProperty> {
        use IntKind as K;
        use IntProperty as I;
        vec![
            I::src(SRC_BIDI, K::BidiClass),
            I::src(SRC_BLOCK, K::Block),
            I::src(SRC_NFC, K::CanonicalCombiningClass),
            I::vec(2, DECOMPOSITION_TYPE_MASK, 0),
            I::vec(0, EAST_ASIAN_MASK, EAST_ASIAN_SHIFT),
            I::src(SRC_CHAR, K::GeneralCategory),
            I::src(SRC_BIDI, K::JoiningGroup),
            I::src(SRC_BIDI, K::JoiningType),
            I::vec(2, LB_MASK, LB_SHIFT),
            I::src(SRC_CHAR, K::NumericType),
            I::src(SRC_PROPSVEC, K::Script),
            I::src(SRC_PROPSVEC, K::HangulSyllableType),
            // max=1=YES -- these are never "maybe", only "no" or "yes"
            I::src(SRC_NFC, K::NormQuickCheck(UProperty::NFD_QUICK_CHECK, 1)),
            I::src(SRC_NFKC, K::NormQuickCheck(UProperty::NFKD_QUICK_CHECK, 1)),
            // max=2=MAYBE
            I::src(SRC_NFC, K::NormQuickCheck(UProperty::NFC_QUICK_CHECK, 2)),
            I::src(SRC_NFKC, K::NormQuickCheck(UProperty::NFKC_QUICK_CHECK, 2)),
            I::src(SRC_NFC, K::LeadCanonicalCombiningClass),
            I::src(SRC_NFC, K::TrailCanonicalCombiningClass),
            I::vec(2, GCB_MASK, GCB_SHIFT),
            I::vec(2, SB_MASK, SB_SHIFT),
            I::vec(2, WB_MASK, WB_SHIFT),
            I::src(SRC_BIDI, K::BidiPairedBracketType),
            I::src(SRC_INPC, K::Inpc),
            I::src(SRC_INSC, K::Insc),
            I::src(SRC_VO, K::Vo),
            I::src(SRC_PROPSVEC, K::IdentifierStatus),
            I::vec(0, INCB_MASK, INCB_SHIFT),
        ]
    }

    // public methods ----------------------------------------------------

    /// `getProperty(int ch)`: the main property value of a code point.
    pub fn get_property(&self, ch: i32) -> i32 {
        self.m_trie.get(ch)
    }

    /// `getAdditional(int codepoint, int column)`: the unicode additional
    /// properties; 0 for a column past the vectors.
    pub fn get_additional(&self, codepoint: i32, column: i32) -> i32 {
        debug_assert!(column >= 0);
        if column >= self.m_additional_columns_count {
            return 0;
        }
        let trie = self
            .m_additional_trie
            .as_ref()
            .expect("column count > 0 means the trie exists");
        self.m_additional_vectors[(trie.get(codepoint) + column) as usize]
    }

    /// `getAge(int codepoint)`: the age of the code point (Unicode version
    /// first designated).
    pub fn get_age(&self, codepoint: i32) -> VersionInfo {
        let version = ((self.get_additional(codepoint, 0) as u32) >> AGE_SHIFT) as i32;
        VersionInfo::get_instance4(version >> 2, version & 3, 0, 0)
            .expect("major/minor nibbles are valid")
    }

    /// `hasBinaryProperty(int c, int which)`.
    pub fn has_binary_property(&self, c: i32, which: i32) -> bool {
        if !(UProperty::BINARY_START..UProperty::BINARY_LIMIT).contains(&which) {
            // not a known binary property
            false
        } else {
            self.bin_props[which as usize].contains(self, c)
        }
    }

    /// `getType(int c)`: the general category.
    pub fn get_type(&self, c: i32) -> i32 {
        self.get_property(c) & TYPE_MASK
    }

    /// `getIntPropertyValue(int c, int which)`.
    pub fn get_int_property_value(&self, c: i32, which: i32) -> i32 {
        if which < UProperty::INT_START {
            if (UProperty::BINARY_START..UProperty::BINARY_LIMIT).contains(&which) {
                return if self.bin_props[which as usize].contains(self, c) {
                    1
                } else {
                    0
                };
            }
        } else if which < UProperty::INT_LIMIT {
            return self.int_props[(which - UProperty::INT_START) as usize].get_value(self, c);
        } else if which == UProperty::GENERAL_CATEGORY_MASK {
            return get_mask(self.get_type(c));
        }
        0 // undefined
    }

    /// `getIntPropertyMaxValue(int which)`.
    pub fn get_int_property_max_value(&self, which: i32) -> i32 {
        if which < UProperty::INT_START {
            if (UProperty::BINARY_START..UProperty::BINARY_LIMIT).contains(&which) {
                return 1; // maximum true for all binary properties
            }
        } else if which < UProperty::INT_LIMIT {
            return self.int_props[(which - UProperty::INT_START) as usize]
                .get_max_value(self, which);
        }
        -1 // undefined
    }

    /// `getSource(int which)` (package-private): the `SRC_` data source.
    pub(crate) fn get_source(&self, which: i32) -> i32 {
        if which < UProperty::BINARY_START {
            SRC_NONE /* undefined */
        } else if which < UProperty::BINARY_LIMIT {
            self.bin_props[which as usize].get_source()
        } else if which < UProperty::INT_START {
            SRC_NONE /* undefined */
        } else if which < UProperty::INT_LIMIT {
            self.int_props[(which - UProperty::INT_START) as usize].get_source()
        } else if which < UProperty::STRING_START {
            match which {
                UProperty::GENERAL_CATEGORY_MASK | UProperty::NUMERIC_VALUE => SRC_CHAR,
                _ => SRC_NONE,
            }
        } else if which < UProperty::STRING_LIMIT {
            match which {
                UProperty::AGE => SRC_PROPSVEC,
                UProperty::BIDI_MIRRORING_GLYPH => SRC_BIDI,
                UProperty::CASE_FOLDING
                | UProperty::LOWERCASE_MAPPING
                | UProperty::SIMPLE_CASE_FOLDING
                | UProperty::SIMPLE_LOWERCASE_MAPPING
                | UProperty::SIMPLE_TITLECASE_MAPPING
                | UProperty::SIMPLE_UPPERCASE_MAPPING
                | UProperty::TITLECASE_MAPPING
                | UProperty::UPPERCASE_MAPPING => SRC_CASE,
                UProperty::ISO_COMMENT | UProperty::NAME | UProperty::UNICODE_1_NAME => SRC_NAMES,
                _ => SRC_NONE,
            }
        } else {
            match which {
                UProperty::SCRIPT_EXTENSIONS | UProperty::IDENTIFIER_TYPE => SRC_PROPSVEC,
                _ => SRC_NONE, /* undefined */
            }
        }
    }

    /// `getMaxValues(int column)`: the maximum values of the additional
    /// properties vector words 0 and 2, 0 for other columns.
    pub fn get_max_values(&self, column: i32) -> i32 {
        // return m_maxValues_[column];
        match column {
            0 => self.m_max_block_script_value,
            2 => self.m_max_jtg_value,
            _ => 0,
        }
    }

    /// `getEuropeanDigit(int ch)`: the digit value of a Latin letter
    /// (ASCII or fullwidth, `A`/`a` = 10), -1 otherwise.
    pub fn get_european_digit(ch: i32) -> i32 {
        if (ch > 0x7a && ch < 0xff21)
            || ch < 0x41
            || (ch > 0x5a && ch < 0x61)
            || ch > 0xff5a
            || (ch > 0xff3a && ch < 0xff41)
        {
            return -1;
        }
        if ch <= 0x7a {
            // ch >= 0x41 or ch < 0x61
            return ch + 10 - if ch <= 0x5a { 0x41 } else { 0x61 };
        }
        // ch >= 0xff21
        if ch <= 0xff3a {
            return ch + 10 - 0xff21;
        }
        // ch >= 0xff41 && ch <= 0xff5a
        ch + 10 - 0xff41
    }

    /// `digit(int c)`: the decimal digit value, -1 otherwise.
    pub fn digit(&self, c: i32) -> i32 {
        let value = get_numeric_type_value(self.get_property(c)) - NTV_DECIMAL_START;
        if value <= 9 {
            value
        } else {
            -1
        }
    }

    /// `getNumericValue(int c)`: the integer numeric value, -1 without
    /// one, -2 when it is not a non-negative integer.
    pub fn get_numeric_value(&self, c: i32) -> i32 {
        // slightly pruned version of getUnicodeNumericValue(), plus getEuropeanDigit()
        let ntv = get_numeric_type_value(self.get_property(c));

        if ntv == NTV_NONE {
            Self::get_european_digit(c)
        } else if ntv < NTV_DIGIT_START {
            /* decimal digit */
            ntv - NTV_DECIMAL_START
        } else if ntv < NTV_NUMERIC_START {
            /* other digit */
            ntv - NTV_DIGIT_START
        } else if ntv < NTV_FRACTION_START {
            /* small integer */
            ntv - NTV_NUMERIC_START
        } else if ntv < NTV_LARGE_START {
            /* fraction */
            -2
        } else if ntv < NTV_BASE60_START {
            /* large, single-significant-digit integer */
            let mant = (ntv >> 5) - 14;
            let mut exp = (ntv & 0x1f) + 2;
            if exp < 9 || (exp == 9 && mant <= 2) {
                let mut num_value = mant;
                loop {
                    num_value *= 10;
                    exp -= 1;
                    if exp <= 0 {
                        break;
                    }
                }
                num_value
            } else {
                -2
            }
        } else if ntv < NTV_FRACTION20_START {
            /* sexagesimal (base 60) integer */
            let mut num_value = (ntv >> 2) - 0xbf;
            let exp = (ntv & 3) + 1;

            match exp {
                4 => num_value *= 60 * 60 * 60 * 60,
                3 => num_value *= 60 * 60 * 60,
                2 => num_value *= 60 * 60,
                1 => num_value *= 60,
                _ => {}
            }

            num_value
        } else if ntv < NTV_RESERVED_START {
            // fraction-20 e.g. 3/80
            -2
        } else {
            /* reserved */
            -2
        }
    }

    /// `getUnicodeNumericValue(int c)`: the numeric value as a double,
    /// `UCharacter.NO_NUMERIC_VALUE` without one.
    pub fn get_unicode_numeric_value(&self, c: i32) -> f64 {
        // equivalent to c version double u_getNumericValue(UChar32 c)
        let ntv = get_numeric_type_value(self.get_property(c));

        if ntv == NTV_NONE {
            UCharacter::NO_NUMERIC_VALUE
        } else if ntv < NTV_DIGIT_START {
            /* decimal digit */
            (ntv - NTV_DECIMAL_START) as f64
        } else if ntv < NTV_NUMERIC_START {
            /* other digit */
            (ntv - NTV_DIGIT_START) as f64
        } else if ntv < NTV_FRACTION_START {
            /* small integer */
            (ntv - NTV_NUMERIC_START) as f64
        } else if ntv < NTV_LARGE_START {
            /* fraction */
            let numerator = (ntv >> 4) - 12;
            let denominator = (ntv & 0xf) + 1;
            numerator as f64 / denominator as f64
        } else if ntv < NTV_BASE60_START {
            /* large, single-significant-digit integer */
            let mant = (ntv >> 5) - 14;
            let mut exp = (ntv & 0x1f) + 2;
            let mut num_value = mant as f64;

            /* multiply by 10^exp without math.h */
            while exp >= 4 {
                num_value *= 10000.;
                exp -= 4;
            }
            match exp {
                3 => num_value *= 1000.,
                2 => num_value *= 100.,
                1 => num_value *= 10.,
                _ => {}
            }

            num_value
        } else if ntv < NTV_FRACTION20_START {
            /* sexagesimal (base 60) integer */
            let mut num_value = (ntv >> 2) - 0xbf;
            let exp = (ntv & 3) + 1;

            match exp {
                4 => num_value *= 60 * 60 * 60 * 60,
                3 => num_value *= 60 * 60 * 60,
                2 => num_value *= 60 * 60,
                1 => num_value *= 60,
                _ => {}
            }

            num_value as f64
        } else if ntv < NTV_FRACTION32_START {
            // fraction-20 e.g. 3/80
            let frac20 = ntv - NTV_FRACTION20_START; // 0..0x17
            let numerator = 2 * (frac20 & 3) + 1;
            let denominator = 20 << (frac20 >> 2);
            numerator as f64 / denominator as f64
        } else if ntv < NTV_RESERVED_START {
            // fraction-32 e.g. 3/64
            let frac32 = ntv - NTV_FRACTION32_START; // 0..15
            let numerator = 2 * (frac32 & 3) + 1;
            let denominator = 32 << (frac32 >> 2);
            numerator as f64 / denominator as f64
        } else {
            /* reserved */
            UCharacter::NO_NUMERIC_VALUE
        }
    }

    /// `addPropertyStarts(UnicodeSet set)`: the start code points of the
    /// same-value ranges of the main trie, plus hardcoded property starts.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_property_starts<'s>(&self, set: &'s mut UnicodeSet) -> Result<&'s mut UnicodeSet> {
        /* add the start code point of each same-value range of the main trie */
        let mut trie_iterator = self.m_trie.iterator();
        while trie_iterator.has_next() {
            let range = trie_iterator.next().expect("has_next");
            if range.lead_surrogate {
                break;
            }
            set.add(range.start_code_point)?;
        }

        /* add code points with hardcoded properties, plus the ones following them */

        /* add for u_isblank() */
        set.add(TAB)?;
        set.add(TAB + 1)?;

        /* add for IS_THAT_CONTROL_SPACE() */
        set.add(CR + 1)?; /* range TAB..CR */
        set.add(0x1c)?;
        set.add(0x1f + 1)?;
        set.add(NL)?;
        set.add(NL + 1)?;

        /* add for u_isIDIgnorable() what was not added above */
        set.add(DEL)?; /* range DEL..NBSP-1, NBSP added below */
        set.add(HAIRSP)?;
        set.add(RLM + 1)?;
        set.add(INHSWAP)?;
        set.add(NOMDIG + 1)?;
        set.add(ZWNBSP)?;
        set.add(ZWNBSP + 1)?;

        /* add no-break spaces for u_isWhitespace() what was not added above */
        set.add(NBSP)?;
        set.add(NBSP + 1)?;
        set.add(FIGURESP)?;
        set.add(FIGURESP + 1)?;
        set.add(NNBSP)?;
        set.add(NNBSP + 1)?;

        /* add for u_charDigitValue() */
        // TODO remove when UCharacter.getHanNumericValue() is changed to just return
        // Unicode numeric values
        for c in [
            0x3007, 0x3008, 0x4e00, 0x4e01, 0x4e8c, 0x4e8d, 0x4e09, 0x4e0a, 0x56db, 0x56dc, 0x4e94,
            0x4e95, 0x516d, 0x516e, 0x4e03, 0x4e04, 0x516b, 0x516c, 0x4e5d, 0x4e5e,
        ] {
            set.add(c)?;
        }

        /* add for u_digit() */
        set.add(U_A_LOWER)?;
        set.add(U_Z_LOWER + 1)?;
        set.add(U_A)?;
        set.add(U_Z + 1)?;
        set.add(U_FW_A_LOWER)?;
        set.add(U_FW_Z_LOWER + 1)?;
        set.add(U_FW_A)?;
        set.add(U_FW_Z + 1)?;

        /* add for u_isxdigit() */
        set.add(U_F_LOWER + 1)?;
        set.add(U_F + 1)?;
        set.add(U_FW_F_LOWER + 1)?;
        set.add(U_FW_F + 1)?;

        /* add for UCHAR_DEFAULT_IGNORABLE_CODE_POINT what was not added above */
        set.add(WJ)?; /* range WJ..NOMDIG */
        set.add(0xfff0)?;
        set.add(0xfffb + 1)?;
        set.add(0xe0000)?;
        set.add(0xe0fff + 1)?;

        /* add for UCHAR_GRAPHEME_BASE and others */
        set.add(CGJ)?;
        set.add(CGJ + 1)?;

        Ok(set) // for chaining
    }

    /// `upropsvec_addPropertyStarts(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn upropsvec_add_property_starts(&self, set: &mut UnicodeSet) -> Result<()> {
        /* add the start code point of each same-value range of the properties vectors trie */
        if self.m_additional_columns_count > 0 {
            /* if m_additionalColumnsCount_==0 then the properties vectors trie may not be there at all */
            let trie = self
                .m_additional_trie
                .as_ref()
                .expect("column count > 0 means the trie exists");
            let mut trie_iterator = trie.iterator();
            while trie_iterator.has_next() {
                let range = trie_iterator.next().expect("has_next");
                if range.lead_surrogate {
                    break;
                }
                set.add(range.start_code_point)?;
            }
        }
        Ok(())
    }

    /// `ulayout_addPropertyStarts(int src, UnicodeSet set)` (package-private).
    ///
    /// # Errors
    ///
    /// As `LayoutProps.addPropertyStarts`.
    pub(crate) fn ulayout_add_property_starts(src: i32, set: &mut UnicodeSet) -> Result<()> {
        LayoutProps::instance().add_property_starts(src, set)
    }

    /// `mathCompat_addPropertyStarts(UnicodeSet set)` (package-private).
    pub(crate) fn math_compat_add_property_starts(set: &mut UnicodeSet) -> Result<()> {
        // range limits
        for &c in ID_COMPAT_MATH_CONTINUE.iter() {
            set.add(c)?;
        }
        // single characters
        for &c in ID_COMPAT_MATH_START.iter() {
            set.add(c)?;
            set.add(c + 1)?;
        }
        Ok(())
    }

    /// `mcm_addPropertyStarts(UnicodeSet set)` (package-private).
    pub(crate) fn mcm_add_property_starts(set: &mut UnicodeSet) -> Result<()> {
        // range limits
        for &c in MODIFIER_COMBINING_MARK.iter() {
            set.add(c)?;
        }
        Ok(())
    }

    /// `ublock_addPropertyStarts(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn ublock_add_property_starts(&self, set: &mut UnicodeSet) -> Result<()> {
        // Add the start code point of each same-value range of the trie.
        // We store Block values indexed by the code point shifted right 4 bits;
        // see ublock_getCode().
        let mut range = Range::new();
        let mut start = 0;
        while start < 0x11000 && // limit: (max code point + 1) >> 4
                self.m_block_trie.get_range(start, None, &mut range)
        {
            set.add(start << 4)?;
            start = range.get_end() + 1;
        }
        Ok(())
    }

    /// `hasIDType(int c, int typeIndex)`.
    pub fn has_id_type(&self, c: i32, type_index: i32) -> bool {
        if type_index < 0 || type_index >= ID_TYPE_TO_ENCODED.len() as i32 {
            return false; // not a valid type
        }
        let encoded_type = ID_TYPE_TO_ENCODED[type_index as usize];
        let value = ((self.get_additional(c, 2) as u32) >> ID_TYPE_SHIFT) as i32;
        if (encoded_type & ID_TYPE_BIT) != 0 {
            value < ID_TYPE_FORBIDDEN && (value & encoded_type) != 0
        } else {
            value == encoded_type
        }
    }

    /// `hasIDType(int c, IdentifierType type)`.
    pub fn has_id_type_enum(&self, c: i32, ty: IdentifierType) -> bool {
        self.has_id_type(c, ty.ordinal())
    }

    fn maybe_add_type(
        value: i32,
        bit: i32,
        t: IdentifierType,
        types: &mut BTreeSet<IdentifierType>,
    ) {
        if (value & bit) != 0 {
            types.insert(t);
        }
    }

    /// `getIDTypes(int c, EnumSet<IdentifierType> types)`: fills `types`
    /// (an `EnumSet`, ordered by ordinal) and returns their count.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("unknown IdentifierType data value 0x..")`
    /// for an unknown data value.
    pub fn get_id_types(&self, c: i32, types: &mut BTreeSet<IdentifierType>) -> Result<i32> {
        types.clear();
        let value = ((self.get_additional(c, 2) as u32) >> ID_TYPE_SHIFT) as i32;
        if (value & ID_TYPE_FORBIDDEN) == ID_TYPE_FORBIDDEN || value == ID_TYPE_NOT_CHARACTER {
            // single value
            let t = match value {
                ID_TYPE_NOT_CHARACTER => IdentifierType::NotCharacter,
                ID_TYPE_DEPRECATED => IdentifierType::Deprecated,
                ID_TYPE_DEFAULT_IGNORABLE => IdentifierType::DefaultIgnorable,
                ID_TYPE_NOT_NFKC => IdentifierType::NotNfkc,
                ID_TYPE_INCLUSION => IdentifierType::Inclusion,
                ID_TYPE_RECOMMENDED => IdentifierType::Recommended,
                _ => {
                    return Err(LuceneError::illegal_state(format!(
                        "unknown IdentifierType data value 0x{value:02x}"
                    )));
                }
            };
            types.insert(t);
            Ok(1)
        } else {
            // one or more combinable bits
            Self::maybe_add_type(value, ID_TYPE_NOT_XID, IdentifierType::NotXid, types);
            Self::maybe_add_type(value, ID_TYPE_EXCLUSION, IdentifierType::Exclusion, types);
            Self::maybe_add_type(value, ID_TYPE_OBSOLETE, IdentifierType::Obsolete, types);
            Self::maybe_add_type(value, ID_TYPE_TECHNICAL, IdentifierType::Technical, types);
            Self::maybe_add_type(
                value,
                ID_TYPE_UNCOMMON_USE,
                IdentifierType::UncommonUse,
                types,
            );
            Self::maybe_add_type(
                value,
                ID_TYPE_LIMITED_USE,
                IdentifierType::LimitedUse,
                types,
            );
            Ok(types.len() as i32)
        }
    }
}

/// Text layout properties (`ulayout.icu`).
///
/// Equivalent to `UCharacterProperty.LayoutProps` (private).
#[derive(Debug)]
struct LayoutProps {
    inpc_trie: Option<CodePointTrie>,
    insc_trie: Option<CodePointTrie>,
    vo_trie: Option<CodePointTrie>,
    max_inpc_value: i32,
    max_insc_value: i32,
    max_vo_value: i32,
}

static LAYOUT_INSTANCE: LazyLock<LayoutProps> = LazyLock::new(|| match LayoutProps::new() {
    Ok(p) => p,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

impl LayoutProps {
    const DATA_FORMAT: i32 = 0x4c61796f; // "Layo"

    // indexes into indexes[]
    // Element 0 stores the length of the indexes[] array.
    //ivate static final int IX_INDEXES_LENGTH = 0;
    // Elements 1..7 store the tops (limits, end offsets)
    // of the following arrays in the data.
    const IX_INPC_TRIE_TOP: usize = 1;
    const IX_INSC_TRIE_TOP: usize = 2;
    const IX_VO_TRIE_TOP: usize = 3;
    //ivate static final int IX_RESERVED_TOP = 4;

    //ivate static final int IX_TRIES_TOP = 7;

    const IX_MAX_VALUES: usize = 9;

    // Length of indexes[]. Multiple of 4 to 16-align the tries.
    //ivate static final int IX_COUNT = 12;

    const MAX_INPC_SHIFT: u32 = 24;
    const MAX_INSC_SHIFT: i32 = 16;
    const MAX_VO_SHIFT: i32 = 8;

    fn instance() -> &'static LayoutProps {
        &LAYOUT_INSTANCE
    }

    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    fn new() -> Result<Self> {
        let mut bytes = ICUBinary::get_required_data("ulayout.icu")?;
        let r: Result<Self> = (|| {
            ICUBinary::read_header_and_data_version(
                &mut bytes,
                Self::DATA_FORMAT,
                // `LayoutProps.IsAcceptable` (`IS_ACCEPTABLE`): format version 1.
                Some(&(|v: &[u8; 4]| v[0] == 1) as &dyn Fn(&[u8; 4]) -> bool),
            )?;
            let start_pos = bytes.position();
            let indexes_length = bytes.get_int()?; // inIndexes[IX_INDEXES_LENGTH]
            if indexes_length < 12 {
                return Err(ICUUncheckedIOException::with_message(
                    "Text layout properties data: not enough indexes",
                )
                .error());
            }
            let mut in_indexes = vec![0i32; indexes_length as usize];
            in_indexes[0] = indexes_length;
            for i in 1..indexes_length as usize {
                in_indexes[i] = bytes.get_int()?;
            }

            let mut offset = indexes_length * 4;
            let mut top = in_indexes[Self::IX_INPC_TRIE_TOP];
            let mut trie_size = top - offset;
            let mut inpc_trie = None;
            if trie_size >= 16 {
                inpc_trie = Some(CodePointTrie::from_binary(None, None, &mut bytes)?);
            }
            let mut pos = bytes.position() - start_pos;
            debug_assert!(top >= pos);
            ICUBinary::skip_bytes(&mut bytes, top - pos)?; // skip padding after trie bytes
            offset = top;
            top = in_indexes[Self::IX_INSC_TRIE_TOP];
            trie_size = top - offset;
            let mut insc_trie = None;
            if trie_size >= 16 {
                insc_trie = Some(CodePointTrie::from_binary(None, None, &mut bytes)?);
            }
            pos = bytes.position() - start_pos;
            debug_assert!(top >= pos);
            ICUBinary::skip_bytes(&mut bytes, top - pos)?; // skip padding after trie bytes
            offset = top;
            top = in_indexes[Self::IX_VO_TRIE_TOP];
            trie_size = top - offset;
            let mut vo_trie = None;
            if trie_size >= 16 {
                vo_trie = Some(CodePointTrie::from_binary(None, None, &mut bytes)?);
            }
            pos = bytes.position() - start_pos;
            debug_assert!(top >= pos);
            ICUBinary::skip_bytes(&mut bytes, top - pos)?; // skip padding after trie bytes

            let max_values = in_indexes[Self::IX_MAX_VALUES];
            Ok(Self {
                inpc_trie,
                insc_trie,
                vo_trie,
                max_inpc_value: ((max_values as u32) >> Self::MAX_INPC_SHIFT) as i32,
                max_insc_value: (max_values >> Self::MAX_INSC_SHIFT) & 0xff,
                max_vo_value: (max_values >> Self::MAX_VO_SHIFT) & 0xff,
            })
        })();
        // Java wraps an IOException into ICUUncheckedIOException.
        r.map_err(|e| {
            if e.is_io_exception() {
                ICUUncheckedIOException::with_cause(e).error()
            } else {
                e
            }
        })
    }

    fn add_property_starts(&self, src: i32, set: &mut UnicodeSet) -> Result<()> {
        let trie = match src {
            SRC_INPC => &self.inpc_trie,
            SRC_INSC => &self.insc_trie,
            SRC_VO => &self.vo_trie,
            _ => {
                return Err(crate::jdk::unchecked_no_message(
                    "java.lang.IllegalStateException",
                ))
            }
        };

        let Some(trie) = trie else {
            return Err(missing_resource(
                format!("no data for one of the text layout properties; src={src}"),
                "LayoutProps",
                "",
            ));
        };

        // Add the start code point of each same-value range of the trie.
        let mut range = Range::new();
        let mut start = 0;
        while trie.get_range(start, None, &mut range) {
            set.add(start)?;
            start = range.get_end() + 1;
        }
        Ok(())
    }
}
