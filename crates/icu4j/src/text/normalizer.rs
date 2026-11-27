// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Normalizer`.

use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::jdk::{
    char_count, code_point_at, code_point_before, illegal_argument_no_message, index_out_of_bounds,
    to_code_point,
};
use crate::lang::u_character::UCharacter;
use crate::r#impl::norm2_all_modes::{Norm2AllModes, NOOP_NORMALIZER2};
use crate::r#impl::normalizer2_impl::UTF16Plus;
use crate::r#impl::u_case_props::UCaseProps;
use crate::text::filtered_normalizer2::FilteredNormalizer2;
use crate::text::normalizer2::{self, Normalizer2};
use crate::text::u_character_iterator::{self as uci, UCharacterIterator};
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// Result values for normalization quick check functions.
///
/// Equivalent to `Normalizer.QuickCheckResult` and its three instances
/// `Normalizer.NO`, `YES` and `MAYBE` (compared by identity in Java).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuickCheckResult {
    /// `NO`: the text is not normalized.
    No,
    /// `YES`: the text is normalized.
    Yes,
    /// `MAYBE`: it cannot be determined without more work.
    Maybe,
}

/// Constants for normalization modes.
///
/// Equivalent to the abstract class `Normalizer.Mode` and its singleton
/// subclasses (`NONE`, `NFD`, `NFKD`, `NFC`, `NFKC`, `FCD`; the aliases
/// `DEFAULT`, `NO_OP`, `COMPOSE`, `COMPOSE_COMPAT`, `DECOMP`,
/// `DECOMP_COMPAT` are the same values). Each private singleton subclass
/// (`Normalizer.java:248-286`) is a variant, and its
/// `getNormalizer2(int options)` override is the variant's arm of
/// [`Mode::get_normalizer2`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// `NONE` (`NO_OP`), the private class `Normalizer.NONEMode`: no
    /// decomposition/composition.
    None,
    /// `NFD` (`DECOMP`), the private class `Normalizer.NFDMode`: canonical
    /// decomposition.
    Nfd,
    /// `NFKD` (`DECOMP_COMPAT`), the private class `Normalizer.NFKDMode`:
    /// compatibility decomposition.
    Nfkd,
    /// `NFC` (`DEFAULT`, `COMPOSE`), the private class `Normalizer.NFCMode`:
    /// canonical decomposition followed by canonical composition.
    Nfc,
    /// `NFKC` (`COMPOSE_COMPAT`), the private class `Normalizer.NFKCMode`:
    /// compatibility decomposition followed by canonical composition.
    Nfkc,
    /// `FCD`, the private class `Normalizer.FCDMode`: "fast C or D" form.
    Fcd,
}

/// `Unicode32.INSTANCE`: `[:age=3.2:]`, frozen.
static UNICODE32: LazyLock<std::result::Result<Arc<UnicodeSet>, Arc<LuceneError>>> =
    LazyLock::new(|| {
        let pattern: Vec<u16> = "[:age=3.2:]".encode_utf16().collect();
        match UnicodeSet::from_pattern(&pattern) {
            Ok(mut s) => {
                s.freeze();
                Ok(Arc::new(s))
            }
            Err(e) => Err(Arc::new(e)),
        }
    });

/// A lazily created `ModeImpl.INSTANCE`: the normalizer of a mode, or the
/// exception of its static initializer.
///
/// Each private holder class of `Normalizer` (`Normalizer.java:156-199`),
/// whose only member is the lazily initialized `INSTANCE`, is one static
/// of this type, named after it.
type ModeImpl = LazyLock<std::result::Result<Arc<dyn Normalizer2>, Arc<LuceneError>>>;

fn mode_impl(
    f: fn() -> Result<Arc<dyn Normalizer2>>,
) -> std::result::Result<Arc<dyn Normalizer2>, Arc<LuceneError>> {
    f().map_err(Arc::new)
}

fn filtered(n2: Result<Arc<dyn Normalizer2>>) -> Result<Arc<dyn Normalizer2>> {
    let set = UNICODE32.as_ref().map_err(|e| e.duplicate())?.clone();
    Ok(Arc::new(FilteredNormalizer2::new(n2?, set)))
}

/// `Normalizer.NFDModeImpl.INSTANCE`: `Normalizer2.getNFDInstance()`.
static NFD_MODE_IMPL: ModeImpl =
    LazyLock::new(|| mode_impl(|| Ok(normalizer2::get_nfd_instance()? as Arc<dyn Normalizer2>)));
/// `Normalizer.NFKDModeImpl.INSTANCE`: `Normalizer2.getNFKDInstance()`.
static NFKD_MODE_IMPL: ModeImpl =
    LazyLock::new(|| mode_impl(|| Ok(normalizer2::get_nfkd_instance()? as Arc<dyn Normalizer2>)));
/// `Normalizer.NFCModeImpl.INSTANCE`: `Normalizer2.getNFCInstance()`.
static NFC_MODE_IMPL: ModeImpl =
    LazyLock::new(|| mode_impl(|| Ok(normalizer2::get_nfc_instance()? as Arc<dyn Normalizer2>)));
/// `Normalizer.NFKCModeImpl.INSTANCE`: `Normalizer2.getNFKCInstance()`.
static NFKC_MODE_IMPL: ModeImpl =
    LazyLock::new(|| mode_impl(|| Ok(normalizer2::get_nfkc_instance()? as Arc<dyn Normalizer2>)));
/// `Normalizer.FCDModeImpl.INSTANCE`: `Norm2AllModes.getFCDNormalizer2()`.
static FCD_MODE_IMPL: ModeImpl = LazyLock::new(|| {
    mode_impl(|| Ok(Norm2AllModes::get_fcd_normalizer2()? as Arc<dyn Normalizer2>))
});
/// `Normalizer.NFD32ModeImpl.INSTANCE`: `Normalizer2.getNFDInstance()`
/// filtered by `Unicode32.INSTANCE`.
static NFD32_MODE_IMPL: ModeImpl = LazyLock::new(|| {
    mode_impl(|| filtered(normalizer2::get_nfd_instance().map(|n| n as Arc<dyn Normalizer2>)))
});
/// `Normalizer.NFKD32ModeImpl.INSTANCE`: `Normalizer2.getNFKDInstance()`
/// filtered by `Unicode32.INSTANCE`.
static NFKD32_MODE_IMPL: ModeImpl = LazyLock::new(|| {
    mode_impl(|| filtered(normalizer2::get_nfkd_instance().map(|n| n as Arc<dyn Normalizer2>)))
});
/// `Normalizer.NFC32ModeImpl.INSTANCE`: `Normalizer2.getNFCInstance()`
/// filtered by `Unicode32.INSTANCE`.
static NFC32_MODE_IMPL: ModeImpl = LazyLock::new(|| {
    mode_impl(|| filtered(normalizer2::get_nfc_instance().map(|n| n as Arc<dyn Normalizer2>)))
});
/// `Normalizer.NFKC32ModeImpl.INSTANCE`: `Normalizer2.getNFKCInstance()`
/// filtered by `Unicode32.INSTANCE`.
static NFKC32_MODE_IMPL: ModeImpl = LazyLock::new(|| {
    mode_impl(|| filtered(normalizer2::get_nfkc_instance().map(|n| n as Arc<dyn Normalizer2>)))
});
/// `Normalizer.FCD32ModeImpl.INSTANCE`: `Norm2AllModes.getFCDNormalizer2()`
/// filtered by `Unicode32.INSTANCE`.
static FCD32_MODE_IMPL: ModeImpl = LazyLock::new(|| {
    mode_impl(|| filtered(Norm2AllModes::get_fcd_normalizer2().map(|n| n as Arc<dyn Normalizer2>)))
});

fn get(m: &ModeImpl) -> Result<Arc<dyn Normalizer2>> {
    match &**m {
        Ok(n) => Ok(n.clone()),
        Err(e) => Err(e.duplicate()),
    }
}

impl Mode {
    /// `getNormalizer2(int options)` (protected).
    ///
    /// # Errors
    ///
    /// The exception of loading the normalization data.
    pub fn get_normalizer2(self, options: i32) -> Result<Arc<dyn Normalizer2>> {
        let u32_ = (options & Normalizer::UNICODE_3_2) != 0;
        match self {
            Mode::None => Ok(NOOP_NORMALIZER2.clone() as Arc<dyn Normalizer2>),
            Mode::Nfd => get(if u32_ {
                &NFD32_MODE_IMPL
            } else {
                &NFD_MODE_IMPL
            }),
            Mode::Nfkd => get(if u32_ {
                &NFKD32_MODE_IMPL
            } else {
                &NFKD_MODE_IMPL
            }),
            Mode::Nfc => get(if u32_ {
                &NFC32_MODE_IMPL
            } else {
                &NFC_MODE_IMPL
            }),
            Mode::Nfkc => get(if u32_ {
                &NFKC32_MODE_IMPL
            } else {
                &NFKC_MODE_IMPL
            }),
            Mode::Fcd => get(if u32_ {
                &FCD32_MODE_IMPL
            } else {
                &FCD_MODE_IMPL
            }),
        }
    }
}

/// Old Unicode normalization API.
///
/// Equivalent to `com.ibm.icu.text.Normalizer` (deprecated in ICU, kept
/// whole). The `char[]` overloads that write into a destination array
/// normalize into a buffer and copy it through Java's `CharsAppendable`
/// rules (Java appends piece by piece; the result written to a large
/// enough destination is the same).
#[derive(Debug, Clone)]
pub struct Normalizer {
    // The input text and our position in it
    text: Box<dyn UCharacterIterator>,
    norm2: Arc<dyn Normalizer2>,
    mode: Mode,
    options: i32,

    // The normalization buffer is the result of normalization
    // of the source in [currentIndex..nextIndex[ .
    current_index: i32,
    next_index: i32,

    // A buffer for holding intermediate results
    buffer: Vec<u16>,
    buffer_pos: i32,
}

/// `COMPARE_EQUIV`: internal option for unorm_cmpEquivFold() for strings
/// in FCD.
const COMPARE_EQUIV: i32 = 0x80000;

impl Normalizer {
    /// `UNICODE_3_2`: options bit set value to select Unicode 3.2
    /// normalization (except NormalizationCorrections).
    pub const UNICODE_3_2: i32 = 0x20;

    /// `DONE`: constant indicating that the end of the iteration has been
    /// reached.
    pub const DONE: i32 = uci::DONE;

    /// `NONE`.
    pub const NONE: Mode = Mode::None;
    /// `NFD`.
    pub const NFD: Mode = Mode::Nfd;
    /// `NFKD`.
    pub const NFKD: Mode = Mode::Nfkd;
    /// `NFC`.
    pub const NFC: Mode = Mode::Nfc;
    /// `NFKC`.
    pub const NFKC: Mode = Mode::Nfkc;
    /// `FCD`.
    pub const FCD: Mode = Mode::Fcd;
    /// `DEFAULT`.
    pub const DEFAULT: Mode = Mode::Nfc;
    /// `NO_OP`.
    pub const NO_OP: Mode = Mode::None;
    /// `COMPOSE`.
    pub const COMPOSE: Mode = Mode::Nfc;
    /// `COMPOSE_COMPAT`.
    pub const COMPOSE_COMPAT: Mode = Mode::Nfkc;
    /// `DECOMP`.
    pub const DECOMP: Mode = Mode::Nfd;
    /// `DECOMP_COMPAT`.
    pub const DECOMP_COMPAT: Mode = Mode::Nfkd;

    /// `IGNORE_HANGUL`: option to disable Hangul/Jamo composition and
    /// decomposition (deprecated, no effect).
    pub const IGNORE_HANGUL: i32 = 0x0001;

    /// `NO`.
    pub const NO: QuickCheckResult = QuickCheckResult::No;
    /// `YES`.
    pub const YES: QuickCheckResult = QuickCheckResult::Yes;
    /// `MAYBE`.
    pub const MAYBE: QuickCheckResult = QuickCheckResult::Maybe;

    /// `FOLD_CASE_DEFAULT`: option bit for compare: case sensitively
    /// compare the strings.
    pub const FOLD_CASE_DEFAULT: i32 = UCharacter::FOLD_CASE_DEFAULT;

    /// `INPUT_IS_FCD`: option bit for compare: both input strings are
    /// assumed to fulfill FCD conditions.
    pub const INPUT_IS_FCD: i32 = 0x20000;

    /// `COMPARE_IGNORE_CASE`: option bit for compare: perform case-insensitive
    /// comparison.
    pub const COMPARE_IGNORE_CASE: i32 = 0x10000;

    /// `COMPARE_CODE_POINT_ORDER`: option bit for compare: compare strings
    /// in code point order instead of code unit order.
    pub const COMPARE_CODE_POINT_ORDER: i32 = 0x8000;

    /// `FOLD_CASE_EXCLUDE_SPECIAL_I`: option value for case folding: use
    /// the modified set of mappings provided in CaseFolding.txt to handle
    /// dotted I and dotless i appropriately for Turkic languages (tr, az).
    pub const FOLD_CASE_EXCLUDE_SPECIAL_I: i32 = UCharacter::FOLD_CASE_EXCLUDE_SPECIAL_I;

    /// `COMPARE_NORM_OPTIONS_SHIFT`: lowest-order bit number of compare()
    /// options bits corresponding to normalization options bits.
    pub const COMPARE_NORM_OPTIONS_SHIFT: i32 = 20;

    //-------------------------------------------------------------------------
    // Iterator constructors
    //-------------------------------------------------------------------------

    /// `Normalizer(String str, Mode mode, int opt)`.
    ///
    /// # Errors
    ///
    /// The exception of loading the normalization data.
    pub fn new_string(str: &[u16], mode: Mode, opt: i32) -> Result<Self> {
        Ok(Self {
            text: uci::get_instance_string(str),
            mode,
            options: opt,
            norm2: mode.get_normalizer2(opt)?,
            current_index: 0,
            next_index: 0,
            buffer: Vec::new(),
            buffer_pos: 0,
        })
    }

    /// `Normalizer(CharacterIterator iter, Mode mode, int opt)`: over a
    /// clone of `iter`.
    ///
    /// # Errors
    ///
    /// The exception of loading the normalization data.
    pub fn new_character_iterator(
        iter: &dyn CharacterIterator,
        mode: Mode,
        opt: i32,
    ) -> Result<Self> {
        Ok(Self {
            text: uci::get_instance_character_iterator(iter.clone_box()),
            mode,
            options: opt,
            norm2: mode.get_normalizer2(opt)?,
            current_index: 0,
            next_index: 0,
            buffer: Vec::new(),
            buffer_pos: 0,
        })
    }

    /// `Normalizer(UCharacterIterator iter, Mode mode, int options)`: over a
    /// clone of `iter`.
    ///
    /// # Errors
    ///
    /// The exception of loading the normalization data.
    pub fn new_u_character_iterator(
        iter: &dyn UCharacterIterator,
        mode: Mode,
        options: i32,
    ) -> Result<Self> {
        Ok(Self {
            text: iter.clone_box(),
            mode,
            options,
            norm2: mode.get_normalizer2(options)?,
            current_index: 0,
            next_index: 0,
            buffer: Vec::new(),
            buffer_pos: 0,
        })
    }

    //--------------------------------------------------------------------------
    // Static Utility methods
    //--------------------------------------------------------------------------

    fn get_compose_normalizer2(compat: bool, options: i32) -> Result<Arc<dyn Normalizer2>> {
        (if compat { Mode::Nfkc } else { Mode::Nfc }).get_normalizer2(options)
    }

    fn get_decompose_normalizer2(compat: bool, options: i32) -> Result<Arc<dyn Normalizer2>> {
        (if compat { Mode::Nfkd } else { Mode::Nfd }).get_normalizer2(options)
    }

    /// `compose(String str, boolean compat)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn compose_str(str: &[u16], compat: bool) -> Result<Vec<u16>> {
        Self::compose_str_options(str, compat, 0)
    }

    /// `compose(String str, boolean compat, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn compose_str_options(str: &[u16], compat: bool, options: i32) -> Result<Vec<u16>> {
        Self::get_compose_normalizer2(compat, options)?.normalize_str(str)
    }

    /// `compose(char[] source, char[] target, boolean compat, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException(<length>)` when `target` is too small.
    pub fn compose_chars(
        source: &[u16],
        target: &mut [u16],
        compat: bool,
        options: i32,
    ) -> Result<i32> {
        let tl = target.len() as i32;
        Self::compose_chars_range(
            source,
            0,
            source.len() as i32,
            target,
            0,
            tl,
            compat,
            options,
        )
    }

    /// `compose(char[] src, int srcStart, int srcLimit, char[] dest, int
    /// destStart, int destLimit, boolean compat, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException(<length>)` when the destination range is
    /// too small.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_chars_range(
        src: &[u16],
        src_start: i32,
        src_limit: i32,
        dest: &mut [u16],
        dest_start: i32,
        dest_limit: i32,
        compat: bool,
        options: i32,
    ) -> Result<i32> {
        let src_buffer = &src[src_start as usize..src_limit as usize];
        let mut app = CharsAppendable::new(dest, dest_start, dest_limit);
        let mut out = Vec::new();
        Self::get_compose_normalizer2(compat, options)?.normalize_append(src_buffer, &mut out)?;
        app.append_range(&out, 0, out.len() as i32);
        Ok(app.length())
    }

    /// `decompose(String str, boolean compat)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn decompose_str(str: &[u16], compat: bool) -> Result<Vec<u16>> {
        Self::decompose_str_options(str, compat, 0)
    }

    /// `decompose(String str, boolean compat, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn decompose_str_options(str: &[u16], compat: bool, options: i32) -> Result<Vec<u16>> {
        Self::get_decompose_normalizer2(compat, options)?.normalize_str(str)
    }

    /// `decompose(char[] source, char[] target, boolean compat, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException(<length>)` when `target` is too small.
    pub fn decompose_chars(
        source: &[u16],
        target: &mut [u16],
        compat: bool,
        options: i32,
    ) -> Result<i32> {
        let tl = target.len() as i32;
        Self::decompose_chars_range(
            source,
            0,
            source.len() as i32,
            target,
            0,
            tl,
            compat,
            options,
        )
    }

    /// `decompose(char[] src, int srcStart, int srcLimit, char[] dest, int
    /// destStart, int destLimit, boolean compat, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException(<length>)` when the destination range is
    /// too small.
    #[allow(clippy::too_many_arguments)]
    pub fn decompose_chars_range(
        src: &[u16],
        src_start: i32,
        src_limit: i32,
        dest: &mut [u16],
        dest_start: i32,
        dest_limit: i32,
        compat: bool,
        options: i32,
    ) -> Result<i32> {
        let src_buffer = &src[src_start as usize..src_limit as usize];
        let mut app = CharsAppendable::new(dest, dest_start, dest_limit);
        let mut out = Vec::new();
        Self::get_decompose_normalizer2(compat, options)?.normalize_append(src_buffer, &mut out)?;
        app.append_range(&out, 0, out.len() as i32);
        Ok(app.length())
    }

    /// `normalize(String str, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn normalize_str(str: &[u16], mode: Mode, options: i32) -> Result<Vec<u16>> {
        mode.get_normalizer2(options)?.normalize_str(str)
    }

    /// `normalize(String src, Mode mode)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn normalize_str_mode(src: &[u16], mode: Mode) -> Result<Vec<u16>> {
        Self::normalize_str(src, mode, 0)
    }

    /// `normalize(char[] source, char[] target, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException(<length>)` when `target` is too small.
    pub fn normalize_chars(
        source: &[u16],
        target: &mut [u16],
        mode: Mode,
        options: i32,
    ) -> Result<i32> {
        let tl = target.len() as i32;
        Self::normalize_chars_range(source, 0, source.len() as i32, target, 0, tl, mode, options)
    }

    /// `normalize(char[] src, int srcStart, int srcLimit, char[] dest, int
    /// destStart, int destLimit, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException(<length>)` when the destination range is
    /// too small.
    #[allow(clippy::too_many_arguments)]
    pub fn normalize_chars_range(
        src: &[u16],
        src_start: i32,
        src_limit: i32,
        dest: &mut [u16],
        dest_start: i32,
        dest_limit: i32,
        mode: Mode,
        options: i32,
    ) -> Result<i32> {
        let src_buffer = &src[src_start as usize..src_limit as usize];
        let mut app = CharsAppendable::new(dest, dest_start, dest_limit);
        let mut out = Vec::new();
        mode.get_normalizer2(options)?
            .normalize_append(src_buffer, &mut out)?;
        app.append_range(&out, 0, out.len() as i32);
        Ok(app.length())
    }

    /// `normalize(int char32, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer; `IllegalArgumentException` for an
    /// invalid code point.
    pub fn normalize_cp(char32: i32, mode: Mode, options: i32) -> Result<Vec<u16>> {
        if mode == Mode::Nfd && options == 0 {
            let decomposition = normalizer2::get_nfc_instance()?.get_decomposition(char32)?;
            return match decomposition {
                Some(d) => Ok(d),
                None => UTF16::value_of(char32),
            };
        }
        Self::normalize_str(&UTF16::value_of(char32)?, mode, options)
    }

    /// `normalize(int char32, Mode mode)`.
    ///
    /// # Errors
    ///
    /// As [`normalize_cp`](Self::normalize_cp).
    pub fn normalize_cp_mode(char32: i32, mode: Mode) -> Result<Vec<u16>> {
        Self::normalize_cp(char32, mode, 0)
    }

    /// `quickCheck(String source, Mode mode)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn quick_check_str_mode(source: &[u16], mode: Mode) -> Result<QuickCheckResult> {
        Self::quick_check_str(source, mode, 0)
    }

    /// `quickCheck(String source, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn quick_check_str(source: &[u16], mode: Mode, options: i32) -> Result<QuickCheckResult> {
        mode.get_normalizer2(options)?.quick_check(source)
    }

    /// `quickCheck(char[] source, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn quick_check_chars(source: &[u16], mode: Mode, options: i32) -> Result<QuickCheckResult> {
        Self::quick_check_chars_range(source, 0, source.len() as i32, mode, options)
    }

    /// `quickCheck(char[] source, int start, int limit, Mode mode, int
    /// options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn quick_check_chars_range(
        source: &[u16],
        start: i32,
        limit: i32,
        mode: Mode,
        options: i32,
    ) -> Result<QuickCheckResult> {
        mode.get_normalizer2(options)?
            .quick_check(&source[start as usize..limit as usize])
    }

    /// `isNormalized(char[] src, int start, int limit, Mode mode, int
    /// options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn is_normalized_chars(
        src: &[u16],
        start: i32,
        limit: i32,
        mode: Mode,
        options: i32,
    ) -> Result<bool> {
        mode.get_normalizer2(options)?
            .is_normalized(&src[start as usize..limit as usize])
    }

    /// `isNormalized(String str, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn is_normalized_str(str: &[u16], mode: Mode, options: i32) -> Result<bool> {
        mode.get_normalizer2(options)?.is_normalized(str)
    }

    /// `isNormalized(int char32, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer; `IllegalArgumentException` for an
    /// invalid code point.
    pub fn is_normalized_cp(char32: i32, mode: Mode, options: i32) -> Result<bool> {
        Self::is_normalized_str(&UTF16::value_of(char32)?, mode, options)
    }

    /// `compare(char[] s1, int s1Start, int s1Limit, char[] s2, int s2Start,
    /// int s2Limit, int options)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for negative or reversed ranges; the
    /// errors of the normalizer.
    pub fn compare_chars_range(
        s1: &[u16],
        s1_start: i32,
        s1_limit: i32,
        s2: &[u16],
        s2_start: i32,
        s2_limit: i32,
        options: i32,
    ) -> Result<i32> {
        if s1_start < 0
            || s1_limit < 0
            || s2_start < 0
            || s2_limit < 0
            || s1_limit < s1_start
            || s2_limit < s2_start
        {
            return Err(illegal_argument_no_message());
        }
        Self::internal_compare(
            &s1[s1_start as usize..s1_limit as usize],
            &s2[s2_start as usize..s2_limit as usize],
            options,
        )
    }

    /// `compare(String s1, String s2, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn compare_str(s1: &[u16], s2: &[u16], options: i32) -> Result<i32> {
        Self::internal_compare(s1, s2, options)
    }

    /// `compare(char[] s1, char[] s2, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn compare_chars(s1: &[u16], s2: &[u16], options: i32) -> Result<i32> {
        Self::internal_compare(s1, s2, options)
    }

    /// `compare(int char32a, int char32b, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer; `IllegalArgumentException` for an
    /// invalid code point.
    pub fn compare_cps(char32a: i32, char32b: i32, options: i32) -> Result<i32> {
        Self::internal_compare(
            &UTF16::value_of(char32a)?,
            &UTF16::value_of(char32b)?,
            options | Self::INPUT_IS_FCD,
        )
    }

    /// `compare(int char32a, String str2, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer; `IllegalArgumentException` for an
    /// invalid code point.
    pub fn compare_cp_str(char32a: i32, str2: &[u16], options: i32) -> Result<i32> {
        Self::internal_compare(&UTF16::value_of(char32a)?, str2, options)
    }

    /// `concatenate(char[] left, int leftStart, int leftLimit, char[] right,
    /// int rightStart, int rightLimit, char[] dest, int destStart, int
    /// destLimit, Normalizer.Mode mode, int options)`: the length of the
    /// result written to `dest`. Java's check for overlapping right and
    /// destination arrays cannot fail (Rust does not allow the aliasing).
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException(<length>)` when the destination range is
    /// too small.
    #[allow(clippy::too_many_arguments)]
    pub fn concatenate_chars_range(
        left: &[u16],
        left_start: i32,
        left_limit: i32,
        right: &[u16],
        right_start: i32,
        right_limit: i32,
        dest: &mut [u16],
        dest_start: i32,
        dest_limit: i32,
        mode: Mode,
        options: i32,
    ) -> Result<i32> {
        // allow left==dest
        let mut dest_builder: Vec<u16> = Vec::with_capacity(
            (left_limit - left_start + right_limit - right_start + 16).max(0) as usize,
        );
        dest_builder.extend_from_slice(&left[left_start as usize..left_limit as usize]);
        let right_buffer = &right[right_start as usize..right_limit as usize];
        mode.get_normalizer2(options)?
            .append(&mut dest_builder, right_buffer)?;
        let dest_length = dest_builder.len() as i32;
        if dest_length <= (dest_limit - dest_start) {
            dest[dest_start as usize..(dest_start + dest_length) as usize]
                .copy_from_slice(&dest_builder);
            Ok(dest_length)
        } else {
            index_out_of_bounds(Some(dest_length.to_string()));
        }
    }

    /// `concatenate(char[] left, char[] right, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn concatenate_chars(
        left: &[u16],
        right: &[u16],
        mode: Mode,
        options: i32,
    ) -> Result<Vec<u16>> {
        let mut dest: Vec<u16> = Vec::with_capacity(left.len() + right.len() + 16);
        dest.extend_from_slice(left);
        mode.get_normalizer2(options)?.append(&mut dest, right)?;
        Ok(dest)
    }

    /// `concatenate(String left, String right, Mode mode, int options)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn concatenate_str(
        left: &[u16],
        right: &[u16],
        mode: Mode,
        options: i32,
    ) -> Result<Vec<u16>> {
        Self::concatenate_chars(left, right, mode, options)
    }

    /// `getFC_NFKC_Closure(int c, char[] dest)`: the length of the closure,
    /// copied into `dest` when it fits.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn get_fc_nfkc_closure_into(c: i32, dest: Option<&mut [u16]>) -> Result<i32> {
        let closure = Self::get_fc_nfkc_closure(c)?;
        let length = closure.len();
        if let Some(dest) = dest {
            if length != 0 && length <= dest.len() {
                dest[..length].copy_from_slice(&closure);
            }
        }
        Ok(length as i32)
    }

    /// `getFC_NFKC_Closure(int c)`: the FC_NFKC_Closure of c, empty when
    /// it is the code point itself.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn get_fc_nfkc_closure(c: i32) -> Result<Vec<u16>> {
        // Compute the FC_NFKC_Closure on the fly:
        // We have the API for complete coverage of Unicode properties, although
        // this value by itself is not useful via API.
        // (What could be useful is a custom normalization table that combines
        // case folding and NFKC.)
        // For the derivation, see Unicode's DerivedNormalizationProps.txt.
        let nfkc = get(&NFKC_MODE_IMPL)?;
        let csp = UCaseProps::instance();
        // first: b = NFKC(Fold(a))
        let mut folded: Vec<u16> = Vec::new();
        let folded1_length = csp.to_full_folding(c, &mut folded, 0);
        if folded1_length < 0 {
            let nfkc_impl = &nfkc
                .as_with_impl()
                .expect("the NFKC normalizer is a Normalizer2WithImpl")
                .impl_;
            if nfkc_impl.get_comp_quick_check(nfkc_impl.get_norm16(c)) != 0 {
                return Ok(Vec::new()); // c does not change at all under CaseFolding+NFKC
            }
            crate::jdk::append_code_point(&mut folded, c)?;
        } else if folded1_length > UCaseProps::MAX_STRING_LENGTH {
            crate::jdk::append_code_point(&mut folded, folded1_length)?;
        }
        let kc1 = nfkc.normalize_str(&folded)?;
        // second: c = NFKC(Fold(b))
        let kc2 = nfkc.normalize_str(&UCharacter::fold_case_str_options(&kc1, 0))?;
        // if (c != b) add the mapping from a to c
        if kc1 == kc2 {
            Ok(Vec::new())
        } else {
            Ok(kc2)
        }
    }

    //-------------------------------------------------------------------------
    // Iteration API
    //-------------------------------------------------------------------------

    /// `current()`: the current code point, `DONE` at the end.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn current(&mut self) -> Result<i32> {
        if self.buffer_pos < self.buffer.len() as i32 || self.next_normalize()? {
            Ok(code_point_at(&self.buffer, self.buffer_pos))
        } else {
            Ok(Self::DONE)
        }
    }

    /// `next()`: the next code point, `DONE` at the end.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    #[allow(clippy::should_implement_trait)] // Java method name
    pub fn next(&mut self) -> Result<i32> {
        if self.buffer_pos < self.buffer.len() as i32 || self.next_normalize()? {
            let c = code_point_at(&self.buffer, self.buffer_pos);
            self.buffer_pos += char_count(c);
            Ok(c)
        } else {
            Ok(Self::DONE)
        }
    }

    /// `previous()`: the previous code point, `DONE` at the start.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn previous(&mut self) -> Result<i32> {
        if self.buffer_pos > 0 || self.previous_normalize()? {
            let c = code_point_before(&self.buffer, self.buffer_pos);
            self.buffer_pos -= char_count(c);
            Ok(c)
        } else {
            Ok(Self::DONE)
        }
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.text.set_to_start();
        self.current_index = 0;
        self.next_index = 0;
        self.clear_buffer();
    }

    /// `setIndexOnly(int index)` (deprecated).
    pub fn set_index_only(&mut self, index: i32) {
        self.text.set_index(index);
        self.current_index = index;
        self.next_index = index;
        self.clear_buffer();
    }

    /// `setIndex(int index)` (deprecated): sets the index and returns the
    /// current code point.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn set_index(&mut self, index: i32) -> Result<i32> {
        self.set_index_only(index);
        self.current()
    }

    /// `getBeginIndex()` (deprecated).
    pub fn get_begin_index(&self) -> i32 {
        0
    }

    /// `getEndIndex()` (deprecated).
    pub fn get_end_index(&self) -> i32 {
        self.end_index()
    }

    /// `first()`: resets and returns the first code point.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn first(&mut self) -> Result<i32> {
        self.reset();
        self.next()
    }

    /// `last()`: moves to the end and returns the last code point.
    ///
    /// # Errors
    ///
    /// The errors of the normalizer.
    pub fn last(&mut self) -> Result<i32> {
        self.text.set_to_limit();
        self.current_index = self.text.get_index();
        self.next_index = self.current_index;
        self.clear_buffer();
        self.previous()
    }

    /// `getIndex()`: the index in the input text of the current position.
    pub fn get_index(&self) -> i32 {
        if self.buffer_pos < self.buffer.len() as i32 {
            self.current_index
        } else {
            self.next_index
        }
    }

    /// `startIndex()` (deprecated).
    pub fn start_index(&self) -> i32 {
        0
    }

    /// `endIndex()` (deprecated).
    pub fn end_index(&self) -> i32 {
        self.text.get_length()
    }

    //-------------------------------------------------------------------------
    // Iterator attributes
    //-------------------------------------------------------------------------

    /// `setMode(Mode newMode)`.
    ///
    /// # Errors
    ///
    /// The exception of loading the normalization data.
    pub fn set_mode(&mut self, new_mode: Mode) -> Result<()> {
        self.mode = new_mode;
        self.norm2 = self.mode.get_normalizer2(self.options)?;
        Ok(())
    }

    /// `getMode()`.
    pub fn get_mode(&self) -> Mode {
        self.mode
    }

    /// `setOption(int option, boolean value)` (deprecated).
    ///
    /// # Errors
    ///
    /// The exception of loading the normalization data.
    pub fn set_option(&mut self, option: i32, value: bool) -> Result<()> {
        if value {
            self.options |= option;
        } else {
            self.options &= !option;
        }
        self.norm2 = self.mode.get_normalizer2(self.options)?;
        Ok(())
    }

    /// `getOption(int option)` (deprecated): 1 when set, 0 otherwise.
    pub fn get_option(&self, option: i32) -> i32 {
        if (self.options & option) != 0 {
            1
        } else {
            0
        }
    }

    /// `getText(char[] fillIn)` (deprecated).
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `fill_in` is too short.
    pub fn get_text_into(&mut self, fill_in: &mut [u16]) -> i32 {
        self.text.get_text_fill(fill_in)
    }

    /// `getLength()` (deprecated).
    pub fn get_length(&self) -> i32 {
        self.text.get_length()
    }

    /// `getText()` (deprecated).
    pub fn get_text(&mut self) -> Vec<u16> {
        self.text.get_text()
    }

    /// `setText(StringBuffer newText)` (deprecated).
    pub fn set_text_string_buffer(&mut self, new_text: Arc<Mutex<Vec<u16>>>) {
        self.text = uci::get_instance_string_buffer(new_text);
        self.reset();
    }

    /// `setText(char[] newText)` (deprecated).
    pub fn set_text_chars(&mut self, new_text: Arc<[u16]>) {
        self.text = uci::get_instance_chars(new_text);
        self.reset();
    }

    /// `setText(String newText)` (deprecated).
    pub fn set_text_string(&mut self, new_text: &[u16]) {
        self.text = uci::get_instance_string(new_text);
        self.reset();
    }

    /// `setText(CharacterIterator newText)` (deprecated).
    pub fn set_text_character_iterator(&mut self, new_text: Box<dyn CharacterIterator>) {
        self.text = uci::get_instance_character_iterator(new_text);
        self.reset();
    }

    /// `setText(UCharacterIterator newText)` (deprecated): over a clone.
    pub fn set_text_u_character_iterator(&mut self, new_text: &dyn UCharacterIterator) {
        self.text = new_text.clone_box();
        self.reset();
    }

    //-------------------------------------------------------------------------
    // Private utility methods
    //-------------------------------------------------------------------------

    fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.buffer_pos = 0;
    }

    fn next_normalize(&mut self) -> Result<bool> {
        self.clear_buffer();
        self.current_index = self.next_index;
        self.text.set_index(self.next_index);
        // Skip at least one character so we make progress.
        let mut c = self.text.next_code_point();
        if c < 0 {
            return Ok(false);
        }
        let mut segment: Vec<u16> = Vec::new();
        crate::jdk::append_code_point(&mut segment, c)?;
        loop {
            c = self.text.next_code_point();
            if c < 0 {
                break;
            }
            if self.norm2.has_boundary_before(c)? {
                self.text.move_code_point_index(-1);
                break;
            }
            crate::jdk::append_code_point(&mut segment, c)?;
        }
        self.next_index = self.text.get_index();
        self.norm2.normalize(&segment, &mut self.buffer)?;
        Ok(!self.buffer.is_empty())
    }

    fn previous_normalize(&mut self) -> Result<bool> {
        self.clear_buffer();
        self.next_index = self.current_index;
        self.text.set_index(self.current_index);
        let mut segment: Vec<u16> = Vec::new();
        loop {
            let c = self.text.previous_code_point();
            if c < 0 {
                break;
            }
            if c <= 0xffff {
                segment.insert(0, c as u16);
            } else {
                segment.insert(0, UTF16::get_trail_surrogate(c));
                segment.insert(0, UTF16::get_lead_surrogate(c));
            }
            if self.norm2.has_boundary_before(c)? {
                break;
            }
        }
        self.current_index = self.text.get_index();
        self.norm2.normalize(&segment, &mut self.buffer)?;
        self.buffer_pos = self.buffer.len() as i32;
        Ok(!self.buffer.is_empty())
    }

    /* compare canonically equivalent ------------------------------------------- */
    fn internal_compare(s1: &[u16], s2: &[u16], mut options: i32) -> Result<i32> {
        let norm_options = ((options as u32) >> Self::COMPARE_NORM_OPTIONS_SHIFT) as i32;
        options |= COMPARE_EQUIV;

        /*
         * UAX #21 Case Mappings, as fixed for Unicode version 4
         * (see Jitterbug 2021), defines a canonical caseless match as
         *
         * A string X is a canonical caseless match
         * for a string Y if and only if
         * NFD(toCasefold(NFD(X))) = NFD(toCasefold(NFD(Y)))
         *
         * For better performance, we check for FCD (or let the caller tell us that
         * both strings are in FCD) for the inner normalization.
         * BasicNormalizerTest::FindFoldFCDExceptions() makes sure that
         * case-folding preserves the FCD-ness of a string.
         * The outer normalization is then only performed by NormalizerImpl.cmpEquivFold()
         * when there is a difference.
         *
         * Exception: When using the Turkic case-folding option, we do perform
         * full NFD first. This is because in the Turkic case precomposed characters
         * with 0049 capital I or 0069 small i fold differently whether they
         * are first decomposed or not, so an FCD check - a check only for
         * canonical order - is not sufficient.
         */
        let mut f1: Option<Vec<u16>> = None;
        let mut f2: Option<Vec<u16>> = None;
        if (options & Self::INPUT_IS_FCD) == 0 || (options & Self::FOLD_CASE_EXCLUDE_SPECIAL_I) != 0
        {
            let n2 = if (options & Self::FOLD_CASE_EXCLUDE_SPECIAL_I) != 0 {
                Mode::Nfd.get_normalizer2(norm_options)?
            } else {
                Mode::Fcd.get_normalizer2(norm_options)?
            };

            // check if s1 and/or s2 fulfill the FCD conditions
            let span_qc_yes1 = n2.span_quick_check_yes(s1)?;
            let span_qc_yes2 = n2.span_quick_check_yes(s2)?;

            /*
             * ICU 2.4 had a further optimization:
             * If both strings were not in FCD, then they were both NFD'ed,
             * and the COMPARE_EQUIV option was turned off.
             * It is not entirely clear that this is valid with the current
             * definition of the canonical caseless match.
             * Therefore, ICU 2.6 removes that optimization.
             */

            if span_qc_yes1 < s1.len() as i32 {
                let mut fcd1: Vec<u16> = Vec::with_capacity(s1.len() + 16);
                fcd1.extend_from_slice(&s1[..span_qc_yes1 as usize]);
                n2.normalize_second_and_append(&mut fcd1, &s1[span_qc_yes1 as usize..])?;
                f1 = Some(fcd1);
            }
            if span_qc_yes2 < s2.len() as i32 {
                let mut fcd2: Vec<u16> = Vec::with_capacity(s2.len() + 16);
                fcd2.extend_from_slice(&s2[..span_qc_yes2 as usize]);
                n2.normalize_second_and_append(&mut fcd2, &s2[span_qc_yes2 as usize..])?;
                f2 = Some(fcd2);
            }
        }

        Self::cmp_equiv_fold(
            f1.as_deref().unwrap_or(s1),
            f2.as_deref().unwrap_or(s2),
            options,
        )
    }

    /*
     * Compare two strings for canonical equivalence.
     * Further options include case-insensitive comparison and
     * code point order (as opposed to code unit order).
     *
     * In this function, canonical equivalence is optional as well.
     * If canonical equivalence is tested, then both strings must fulfill
     * the FCD check.
     *
     * Semantically, this is equivalent to
     *   strcmp[CodePointOrder](NFD(foldCase(s1)), NFD(foldCase(s2)))
     * where code point order, NFD and foldCase are all optional.
     *
     * String comparisons almost always yield results before processing both strings
     * completely.
     * They are generally more efficient working incrementally instead of
     * performing the sub-processing (strlen, normalization, case-folding)
     * on the entire strings first.
     *
     * It is also unnecessary to not normalize identical characters.
     *
     * This function works in principle as follows:
     *
     * loop {
     *   get one code unit c1 from s1 (-1 if end of source)
     *   get one code unit c2 from s2 (-1 if end of source)
     *
     *   if(either string finished) {
     *     return result;
     *   }
     *   if(c1==c2) {
     *     continue;
     *   }
     *
     *   // c1!=c2
     *   try to decompose/case-fold c1/c2, and continue if one does;
     *
     *   // still c1!=c2 and neither decomposes/case-folds, return result
     *   return c1-c2;
     * }
     *
     * When a character decomposes, then the pointer for that source changes to
     * the decomposition, pushing the previous pointer onto a stack.
     * When the end of the decomposition is reached, then the code unit reader
     * pops the previous source from the stack.
     * (Same for case-folding.)
     *
     * This is complicated further by operating on variable-width UTF-16.
     * The top part of the loop works on code units, while lookups for decomposition
     * and case-folding need code points.
     * Code points are assembled after the equality/end-of-source part.
     * The source pointer is only advanced beyond all code units when the code point
     * actually decomposes/case-folds.
     *
     * If we were on a trail surrogate unit when assembling a code point,
     * and the code point decomposes/case-folds, then the decomposition/folding
     * result must be compared with the part of the other string that corresponds to
     * this string's lead surrogate.
     * Since we only assemble a code point when hitting a trail unit when the
     * preceding lead units were identical, we back up the other string by one unit
     * in such a case.
     *
     * The optional code point order comparison at the end works with
     * the same fix-up as the other code point order comparison functions.
     * See ustring.c and the comment near the end of this function.
     *
     * Assumption: A decomposition or case-folding result string never contains
     * a single surrogate. This is a safe assumption in the Unicode Standard.
     * Therefore, we do not need to check for surrogate pairs across
     * decomposition/case-folding boundaries.
     *
     * Further assumptions (see verifications tstnorm.cpp):
     * The API function checks for FCD first, while the core function
     * first case-folds and then decomposes. This requires that case-folding does not
     * un-FCD any strings.
     *
     * The API function may also NFD the input and turn off decomposition.
     * This requires that case-folding does not un-NFD strings either.
     *
     * TODO If any of the above two assumptions is violated,
     * then this entire code must be re-thought.
     * If this happens, then a simple solution is to case-fold both strings up front
     * and to turn off UNORM_INPUT_IS_FCD.
     * We already do this when not both strings are in FCD because makeFCD
     * would be a partial NFD before the case folding, which does not work.
     * Note that all of this is only a problem when case-folding _and_
     * canonical equivalence come together.
     * (Comments in unorm_compare() are more up to date than this TODO.)
     */
    /// `cmpEquivFold(CharSequence cs1, CharSequence cs2, int options)`
    /// (package-private): internal option for unorm_cmpEquivFold() for
    /// decomposing.
    ///
    /// # Errors
    ///
    /// The exception of loading the NFC data.
    pub(crate) fn cmp_equiv_fold(cs1_in: &[u16], cs2_in: &[u16], options: i32) -> Result<i32> {
        // The text of each level: the input, a decomposition or a case folding.
        #[derive(Clone)]
        enum Src {
            In1,
            In2,
            Fold1,
            Fold2,
            Owned(Vec<u16>),
        }
        // A `Normalizer.CmpEquivLevel` (`cs`, `s`) is a `(Option<Src>, i32)`:
        // `cs` (`None` for an unused level) and `s`; `createCmpEquivLevelStack()`
        // is the two-element array the stacks below are created with.
        let nfc_all;
        let nfc_impl = if (options & COMPARE_EQUIV) != 0 {
            nfc_all = Norm2AllModes::get_nfc_instance()?;
            Some(&nfc_all.impl_)
        } else {
            None
        };
        let csp = if (options & Self::COMPARE_IGNORE_CASE) != 0 {
            Some(UCaseProps::instance())
        } else {
            None
        };
        let mut fold1: Vec<u16> = Vec::new();
        let mut fold2: Vec<u16> = Vec::new();

        // stacks of previous-level start/current/limit
        let mut stack1: Option<[(Option<Src>, i32); 2]> = None;
        let mut stack2: Option<[(Option<Src>, i32); 2]> = None;

        // current-level start/limit - s1/s2 as current
        let mut cs1 = Src::In1;
        let mut cs2 = Src::In2;
        let mut s1 = 0;
        let mut limit1 = cs1_in.len() as i32;
        let mut s2 = 0;
        let mut limit2 = cs2_in.len() as i32;

        let mut level1 = 0;
        let mut level2 = 0;
        let mut c1 = -1;
        let mut c2 = -1;
        let mut cp1;
        let mut cp2;
        let mut length;

        macro_rules! text {
            ($src:expr) => {
                match &$src {
                    Src::In1 => cs1_in,
                    Src::In2 => cs2_in,
                    Src::Fold1 => &fold1[..],
                    Src::Fold2 => &fold2[..],
                    Src::Owned(v) => &v[..],
                }
            };
        }

        loop {
            /*
             * here a code unit value of -1 means "get another code unit"
             * below it will mean "this source is finished"
             */

            if c1 < 0 {
                // get next code unit from string 1, post-increment
                loop {
                    if s1 == limit1 {
                        if level1 == 0 {
                            c1 = -1;
                            break;
                        }
                    } else {
                        c1 = text!(cs1)[s1 as usize] as i32;
                        s1 += 1;
                        break;
                    }

                    // reached end of level buffer, pop one level
                    let st = stack1.as_ref().expect("level1 > 0 means the stack exists");
                    loop {
                        level1 -= 1;
                        if let Some(cs) = &st[level1 as usize].0 {
                            cs1 = cs.clone();
                            break;
                        }
                    }
                    s1 = st[level1 as usize].1;
                    limit1 = text!(cs1).len() as i32;
                }
            }

            if c2 < 0 {
                // get next code unit from string 2, post-increment
                loop {
                    if s2 == limit2 {
                        if level2 == 0 {
                            c2 = -1;
                            break;
                        }
                    } else {
                        c2 = text!(cs2)[s2 as usize] as i32;
                        s2 += 1;
                        break;
                    }

                    // reached end of level buffer, pop one level
                    let st = stack2.as_ref().expect("level2 > 0 means the stack exists");
                    loop {
                        level2 -= 1;
                        if let Some(cs) = &st[level2 as usize].0 {
                            cs2 = cs.clone();
                            break;
                        }
                    }
                    s2 = st[level2 as usize].1;
                    limit2 = text!(cs2).len() as i32;
                }
            }

            /*
             * compare c1 and c2
             * either variable c1, c2 is -1 only if the corresponding string is finished
             */
            if c1 == c2 {
                if c1 < 0 {
                    return Ok(0); // c1==c2==-1 indicating end of strings
                }
                c1 = -1;
                c2 = -1;
                continue;
            } else if c1 < 0 {
                return Ok(-1); // string 1 ends before string 2
            } else if c2 < 0 {
                return Ok(1); // string 2 ends before string 1
            }
            // c1!=c2 && c1>=0 && c2>=0

            // get complete code points for c1, c2 for lookups if either is a surrogate
            cp1 = c1;
            if UTF16::is_surrogate(c1) {
                let t = text!(cs1);
                if UTF16Plus::is_surrogate_lead(c1) {
                    if s1 != limit1 && crate::jdk::is_low_surrogate(t[s1 as usize]) {
                        // advance ++s1; only below if cp1 decomposes/case-folds
                        cp1 = to_code_point(c1 as u16, t[s1 as usize]);
                    }
                } else
                /* isTrail(c1) */
                {
                    if 0 <= (s1 - 2) && crate::jdk::is_high_surrogate(t[(s1 - 2) as usize]) {
                        cp1 = to_code_point(t[(s1 - 2) as usize], c1 as u16);
                    }
                }
            }

            cp2 = c2;
            if UTF16::is_surrogate(c2) {
                let t = text!(cs2);
                if UTF16Plus::is_surrogate_lead(c2) {
                    if s2 != limit2 && crate::jdk::is_low_surrogate(t[s2 as usize]) {
                        // advance ++s2; only below if cp2 decomposes/case-folds
                        cp2 = to_code_point(c2 as u16, t[s2 as usize]);
                    }
                } else
                /* isTrail(c2) */
                {
                    if 0 <= (s2 - 2) && crate::jdk::is_high_surrogate(t[(s2 - 2) as usize]) {
                        cp2 = to_code_point(t[(s2 - 2) as usize], c2 as u16);
                    }
                }
            }

            /*
             * go down one level for each string
             * continue with the main loop as soon as there is a real change
             */

            if level1 == 0 && (options & Self::COMPARE_IGNORE_CASE) != 0 && {
                length = csp
                    .expect("COMPARE_IGNORE_CASE")
                    .to_full_folding(cp1, &mut fold1, options);
                length >= 0
            } {
                // cp1 case-folds to the code point "length" or to p[length]
                if UTF16::is_surrogate(c1) {
                    if UTF16Plus::is_surrogate_lead(c1) {
                        // advance beyond source surrogate pair if it case-folds
                        s1 += 1;
                    } else
                    /* isTrail(c1) */
                    {
                        // we got a supplementary code point when hitting its trail surrogate,
                        // therefore the lead surrogate must have been the same as in the other string;
                        // compare this decomposition with the lead surrogate in the other string
                        // remember that this simulates bulk text replacement:
                        // the decomposition would replace the entire code point
                        s2 -= 1;
                        c2 = text!(cs2)[(s2 - 1) as usize] as i32;
                    }
                }

                // push current level pointers
                let st = stack1.get_or_insert([(None, 0), (None, 0)]);
                st[0] = (Some(cs1.clone()), s1);
                level1 += 1;

                // copy the folding result to fold1[]
                // Java has no pointers: copy the folding result to fold1[]
                if length <= UCaseProps::MAX_STRING_LENGTH {
                    let n = fold1.len() - length as usize;
                    fold1.drain(..n);
                } else {
                    fold1.clear();
                    crate::jdk::append_code_point(&mut fold1, length)?;
                }

                // set next level pointers to case folding
                cs1 = Src::Fold1;
                s1 = 0;
                limit1 = fold1.len() as i32;

                // get ready to read from decomposition, continue with loop
                c1 = -1;
                continue;
            }

            if level2 == 0 && (options & Self::COMPARE_IGNORE_CASE) != 0 && {
                length = csp
                    .expect("COMPARE_IGNORE_CASE")
                    .to_full_folding(cp2, &mut fold2, options);
                length >= 0
            } {
                // cp2 case-folds to the code point "length" or to p[length]
                if UTF16::is_surrogate(c2) {
                    if UTF16Plus::is_surrogate_lead(c2) {
                        // advance beyond source surrogate pair if it case-folds
                        s2 += 1;
                    } else
                    /* isTrail(c2) */
                    {
                        // we got a supplementary code point when hitting its trail surrogate,
                        // therefore the lead surrogate must have been the same as in the other string;
                        // compare this decomposition with the lead surrogate in the other string
                        // remember that this simulates bulk text replacement:
                        // the decomposition would replace the entire code point
                        s1 -= 1;
                        c1 = text!(cs1)[(s1 - 1) as usize] as i32;
                    }
                }

                // push current level pointers
                let st = stack2.get_or_insert([(None, 0), (None, 0)]);
                st[0] = (Some(cs2.clone()), s2);
                level2 += 1;

                // copy the folding result to fold2[]
                // Java has no pointers: copy the folding result to fold2[]
                if length <= UCaseProps::MAX_STRING_LENGTH {
                    let n = fold2.len() - length as usize;
                    fold2.drain(..n);
                } else {
                    fold2.clear();
                    crate::jdk::append_code_point(&mut fold2, length)?;
                }

                // set next level pointers to case folding
                cs2 = Src::Fold2;
                s2 = 0;
                limit2 = fold2.len() as i32;

                // get ready to read from decomposition, continue with loop
                c2 = -1;
                continue;
            }

            if level1 < 2 && (options & COMPARE_EQUIV) != 0 {
                if let Some(decomp1) = nfc_impl.expect("COMPARE_EQUIV").get_decomposition(cp1) {
                    // cp1 decomposes into p[length]
                    if UTF16::is_surrogate(c1) {
                        if UTF16Plus::is_surrogate_lead(c1) {
                            // advance beyond source surrogate pair if it decomposes
                            s1 += 1;
                        } else
                        /* isTrail(c1) */
                        {
                            // we got a supplementary code point when hitting its trail surrogate,
                            // therefore the lead surrogate must have been the same as in the other string;
                            // compare this decomposition with the lead surrogate in the other string
                            // remember that this simulates bulk text replacement:
                            // the decomposition would replace the entire code point
                            s2 -= 1;
                            c2 = text!(cs2)[(s2 - 1) as usize] as i32;
                        }
                    }

                    // push current level pointers
                    let st = stack1.get_or_insert([(None, 0), (None, 0)]);
                    st[level1 as usize] = (Some(cs1.clone()), s1);
                    level1 += 1;

                    // set empty intermediate level if skipped
                    if level1 < 2 {
                        st[level1 as usize].0 = None;
                        level1 += 1;
                    }

                    // set next level pointers to decomposition
                    limit1 = decomp1.len() as i32;
                    cs1 = Src::Owned(decomp1);
                    s1 = 0;

                    // get ready to read from decomposition, continue with loop
                    c1 = -1;
                    continue;
                }
            }

            if level2 < 2 && (options & COMPARE_EQUIV) != 0 {
                if let Some(decomp2) = nfc_impl.expect("COMPARE_EQUIV").get_decomposition(cp2) {
                    // cp2 decomposes into p[length]
                    if UTF16::is_surrogate(c2) {
                        if UTF16Plus::is_surrogate_lead(c2) {
                            // advance beyond source surrogate pair if it decomposes
                            s2 += 1;
                        } else
                        /* isTrail(c2) */
                        {
                            // we got a supplementary code point when hitting its trail surrogate,
                            // therefore the lead surrogate must have been the same as in the other string;
                            // compare this decomposition with the lead surrogate in the other string
                            // remember that this simulates bulk text replacement:
                            // the decomposition would replace the entire code point
                            s1 -= 1;
                            c1 = text!(cs1)[(s1 - 1) as usize] as i32;
                        }
                    }

                    // push current level pointers
                    let st = stack2.get_or_insert([(None, 0), (None, 0)]);
                    st[level2 as usize] = (Some(cs2.clone()), s2);
                    level2 += 1;

                    // set empty intermediate level if skipped
                    if level2 < 2 {
                        st[level2 as usize].0 = None;
                        level2 += 1;
                    }

                    // set next level pointers to decomposition
                    limit2 = decomp2.len() as i32;
                    cs2 = Src::Owned(decomp2);
                    s2 = 0;

                    // get ready to read from decomposition, continue with loop
                    c2 = -1;
                    continue;
                }
            }

            /*
             * no decomposition/case folding, max level for both sides:
             * return difference result
             *
             * code point order comparison must not just return cp1-cp2
             * because when single surrogates are present then the surrogate pairs
             * that formed cp1 and cp2 may be from different string indexes
             *
             * example: { d800 d800 dc01 } vs. { d800 dc00 }, compare at second code units
             * c1=d800 cp1=10001 c2=dc00 cp2=10000
             * cp1-cp2>0 but c1-c2<0 and in fact in UTF-32 it is { d800 10001 } < { 10000 }
             *
             * therefore, use same fix-up as in ustring.c/uprv_strCompare()
             * except: uprv_strCompare() fetches c=*s while this functions fetches c=*s++
             * so we have slightly different pointer/start/limit comparisons here
             */

            if c1 >= 0xd800 && c2 >= 0xd800 && (options & Self::COMPARE_CODE_POINT_ORDER) != 0 {
                /* subtract 0x2800 from BMP code points to make them smaller than supplementary ones */
                let t1 = text!(cs1);
                if (c1 <= 0xdbff && s1 != limit1 && crate::jdk::is_low_surrogate(t1[s1 as usize]))
                    || (crate::jdk::is_low_surrogate(c1 as u16)
                        && 0 != (s1 - 1)
                        && crate::jdk::is_high_surrogate(t1[(s1 - 2) as usize]))
                {
                    /* part of a surrogate pair, leave >=d800 */
                } else {
                    /* BMP code point - may be surrogate code point - make <d800 */
                    c1 -= 0x2800;
                }

                let t2 = text!(cs2);
                if (c2 <= 0xdbff && s2 != limit2 && crate::jdk::is_low_surrogate(t2[s2 as usize]))
                    || (crate::jdk::is_low_surrogate(c2 as u16)
                        && 0 != (s2 - 1)
                        && crate::jdk::is_high_surrogate(t2[(s2 - 2) as usize]))
                {
                    /* part of a surrogate pair, leave >=d800 */
                } else {
                    /* BMP code point - may be surrogate code point - make <d800 */
                    c2 -= 0x2800;
                }
            }

            return Ok(c1 - c2);
        }
    }
}

/// An Appendable that writes into a char array with a capacity that may be
/// less than array.length. (By contrast, CharBuffer will write beyond
/// destLimit all the way up to array.length.)
///
/// An overflow is only reported at the end, for the old Normalizer API
/// functions that write to char arrays.
///
/// Equivalent to `Normalizer.CharsAppendable` (private).
struct CharsAppendable<'a> {
    chars: &'a mut [u16],
    start: i32,
    limit: i32,
    offset: i32,
}

impl<'a> CharsAppendable<'a> {
    fn new(dest: &'a mut [u16], dest_start: i32, dest_limit: i32) -> Self {
        Self {
            chars: dest,
            start: dest_start,
            offset: dest_start,
            limit: dest_limit,
        }
    }

    fn length(&self) -> i32 {
        let len = self.offset - self.start;
        if self.offset <= self.limit {
            len
        } else {
            index_out_of_bounds(Some(len.to_string()));
        }
    }

    #[allow(dead_code)]
    fn append_char(&mut self, c: u16) -> &mut Self {
        if self.offset < self.limit {
            self.chars[self.offset as usize] = c;
        }
        self.offset += 1;
        self
    }

    fn append_range(&mut self, s: &[u16], mut s_start: i32, s_limit: i32) -> &mut Self {
        let len = s_limit - s_start;
        if len <= (self.limit - self.offset) {
            while s_start < s_limit {
                self.chars[self.offset as usize] = s[s_start as usize];
                self.offset += 1;
                s_start += 1;
            }
        } else {
            self.offset += len;
        }
        self
    }
}
