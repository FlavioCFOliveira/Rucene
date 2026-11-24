// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CaseMapImpl`.

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{
    char_count, code_point_at, code_point_before, is_high_surrogate, is_low_surrogate,
    to_code_point, unsupported_operation,
};
use crate::lang::u_character::UCharacter;
use crate::lang::u_character_enums::ECharacterCategory;
use crate::r#impl::u_case_props::{ContextIterator, LatinCase, UCaseProps};
use crate::r#impl::u_character_property::UCharacterProperty;
use crate::text::break_iterator::{self, BreakIterator, BreakIteratorLocales, DONE};
use crate::text::edits::Edits;
use crate::util::u_locale::ULocale;

/// Implementation of [`ContextIterator`] for a string.
///
/// Equivalent to `CaseMapImpl.StringContextIterator`.
#[derive(Debug, Clone)]
pub struct StringContextIterator<'a> {
    s: &'a [u16],
    index: i32,
    limit: i32,
    cp_start: i32,
    cp_limit: i32,
    dir: i32,
}

impl<'a> StringContextIterator<'a> {
    /// `StringContextIterator(CharSequence src)`.
    pub fn new(src: &'a [u16]) -> Self {
        Self {
            s: src,
            limit: src.len() as i32,
            cp_start: 0,
            cp_limit: 0,
            index: 0,
            dir: 0,
        }
    }

    /// `StringContextIterator(CharSequence src, int cpStart, int cpLimit)`:
    /// the code point is at `[cp_start, cp_limit[`.
    pub fn new_cp(src: &'a [u16], cp_start: i32, cp_limit: i32) -> Self {
        Self {
            s: src,
            index: 0,
            limit: src.len() as i32,
            cp_start,
            cp_limit,
            dir: 0,
        }
    }

    /// `setLimit(int lim)`: sets the end of the iteration range.
    pub fn set_limit(&mut self, lim: i32) {
        if 0 <= lim && lim <= self.s.len() as i32 {
            self.limit = lim;
        } else {
            self.limit = self.s.len() as i32;
        }
    }

    /// `moveToLimit()`: moves the iteration limit to the end of the string.
    pub fn move_to_limit(&mut self) {
        self.cp_start = self.limit;
        self.cp_limit = self.limit;
    }

    /// `moveTo(int i)`.
    pub fn move_to(&mut self, i: i32) {
        self.cp_start = i;
        self.cp_limit = i;
    }

    /// `nextCaseMapCP()`: the next code point to be case-mapped, or -1.
    pub fn next_case_map_cp(&mut self) -> i32 {
        self.cp_start = self.cp_limit;
        if self.cp_limit < self.limit {
            let c = code_point_at(self.s, self.cp_limit);
            self.cp_limit += char_count(c);
            c
        } else {
            -1
        }
    }

    /// `setCPStartAndLimit(int s, int l)`.
    pub fn set_cp_start_and_limit(&mut self, s: i32, l: i32) {
        self.cp_start = s;
        self.cp_limit = l;
        self.dir = 0;
    }

    /// `getCPStart()`.
    pub fn get_cp_start(&self) -> i32 {
        self.cp_start
    }

    /// `getCPLimit()`.
    pub fn get_cp_limit(&self) -> i32 {
        self.cp_limit
    }

    /// `getCPLength()`.
    pub fn get_cp_length(&self) -> i32 {
        self.cp_limit - self.cp_start
    }
}

impl ContextIterator for StringContextIterator<'_> {
    fn reset(&mut self, direction: i32) {
        if direction > 0 {
            /* reset for forward iteration */
            self.dir = 1;
            self.index = self.cp_limit;
        } else if direction < 0 {
            /* reset for backward iteration */
            self.dir = -1;
            self.index = self.cp_start;
        } else {
            // not a valid direction
            self.dir = 0;
            self.index = 0;
        }
    }

    fn next(&mut self) -> i32 {
        if self.dir > 0 && self.index < self.s.len() as i32 {
            let c = code_point_at(self.s, self.index);
            self.index += char_count(c);
            return c;
        } else if self.dir < 0 && self.index > 0 {
            let c = code_point_before(self.s, self.index);
            self.index -= char_count(c);
            return c;
        }
        -1
    }
}

/// `TITLECASE_WHOLE_STRING`: bit mask for titlecasing the whole string.
pub const TITLECASE_WHOLE_STRING: i32 = 0x20;
/// `TITLECASE_SENTENCES`: bit mask for titlecasing by sentence.
pub const TITLECASE_SENTENCES: i32 = 0x40;
/// Bit mask for which titlecasing iterator to use.
const TITLECASE_ITERATOR_MASK: i32 = 0xe0;
/// `TITLECASE_ADJUST_TO_CASED`: adjust the titlecasing index to the next
/// cased character.
pub const TITLECASE_ADJUST_TO_CASED: i32 = 0x400;
/// Bit mask for titlecasing index adjustment options.
const TITLECASE_ADJUSTMENT_MASK: i32 = 0x600;
/// `OMIT_UNCHANGED_TEXT`: omit unchanged text when recording Edits.
pub const OMIT_UNCHANGED_TEXT: i32 = 0x4000;

const ACUTE: u16 = 0x301;

const U_GC_M_MASK: i32 = (1 << ECharacterCategory::NON_SPACING_MARK)
    | (1 << ECharacterCategory::COMBINING_SPACING_MARK)
    | (1 << ECharacterCategory::ENCLOSING_MARK);

const LNS: i32 = (1 << ECharacterCategory::UPPERCASE_LETTER)
    | (1 << ECharacterCategory::LOWERCASE_LETTER)
    | (1 << ECharacterCategory::TITLECASE_LETTER)
    // Not MODIFIER_LETTER: We count only cased modifier letters.
    | (1 << ECharacterCategory::OTHER_LETTER)
    | (1 << ECharacterCategory::DECIMAL_DIGIT_NUMBER)
    | (1 << ECharacterCategory::LETTER_NUMBER)
    | (1 << ECharacterCategory::OTHER_NUMBER)
    | (1 << ECharacterCategory::MATH_SYMBOL)
    | (1 << ECharacterCategory::CURRENCY_SYMBOL)
    | (1 << ECharacterCategory::MODIFIER_SYMBOL)
    | (1 << ECharacterCategory::OTHER_SYMBOL)
    | (1 << ECharacterCategory::PRIVATE_USE);

fn is_lns(c: i32) -> bool {
    // Letter, number, symbol,
    // or a private use code point because those are typically used as letters or numbers.
    // Consider modifier letters only if they are cased.
    let gc = UCharacterProperty::instance().get_type(c);
    ((1 << gc) & LNS) != 0
        || (gc == ECharacterCategory::MODIFIER_LETTER as i32
            && UCaseProps::instance().get_type(c) != UCaseProps::NONE)
}

/// Implementation of `CaseMap`, `UCharacter` case mapping and
/// `Transliterator` case mapping.
///
/// Equivalent to `com.ibm.icu.impl.CaseMapImpl`. Java's generic
/// `Appendable` destinations are `&mut Vec<u16>`.
pub struct CaseMapImpl;

impl CaseMapImpl {
    /// `addTitleAdjustmentOption(int options, int newOption)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("multiple titlecasing index adjustment
    /// options")`.
    pub fn add_title_adjustment_option(options: i32, new_option: i32) -> Result<i32> {
        let adj_options = options & TITLECASE_ADJUSTMENT_MASK;
        if adj_options != 0 && adj_options != new_option {
            return Err(LuceneError::illegal_argument(
                "multiple titlecasing index adjustment options",
            ));
        }
        Ok(options | new_option)
    }

    /// `addTitleIteratorOption(int options, int newOption)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("multiple titlecasing iterator options")`.
    pub fn add_title_iterator_option(options: i32, new_option: i32) -> Result<i32> {
        let iter_options = options & TITLECASE_ITERATOR_MASK;
        if iter_options != 0 && iter_options != new_option {
            return Err(LuceneError::illegal_argument(
                "multiple titlecasing iterator options",
            ));
        }
        Ok(options | new_option)
    }

    /// `getTitleBreakIterator(Locale locale, int options, BreakIterator
    /// iter)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for conflicting or unknown options; the
    /// errors of the break iterator factory.
    pub fn get_title_break_iterator_locale(
        locale: &Locale,
        mut options: i32,
        iter: Option<Box<dyn BreakIterator>>,
    ) -> Result<Box<dyn BreakIterator>> {
        options &= TITLECASE_ITERATOR_MASK;
        if options != 0 && iter.is_some() {
            return Err(LuceneError::illegal_argument(
                "titlecasing iterator option together with an explicit iterator",
            ));
        }
        match iter {
            Some(i) => Ok(i),
            None => match options {
                0 => break_iterator::get_word_instance_locale(locale),
                TITLECASE_WHOLE_STRING => Ok(Box::new(WholeStringBreakIterator::default())),
                TITLECASE_SENTENCES => break_iterator::get_sentence_instance_locale(locale),
                _ => Err(LuceneError::illegal_argument(
                    "unknown titlecasing iterator option",
                )),
            },
        }
    }

    /// `getTitleBreakIterator(ULocale locale, int options, BreakIterator
    /// iter)`.
    ///
    /// # Errors
    ///
    /// As [`get_title_break_iterator_locale`](Self::get_title_break_iterator_locale).
    pub fn get_title_break_iterator(
        locale: &ULocale,
        mut options: i32,
        iter: Option<Box<dyn BreakIterator>>,
    ) -> Result<Box<dyn BreakIterator>> {
        options &= TITLECASE_ITERATOR_MASK;
        if options != 0 && iter.is_some() {
            return Err(LuceneError::illegal_argument(
                "titlecasing iterator option together with an explicit iterator",
            ));
        }
        match iter {
            Some(i) => Ok(i),
            None => match options {
                0 => break_iterator::get_word_instance_ulocale(locale),
                TITLECASE_WHOLE_STRING => Ok(Box::new(WholeStringBreakIterator::default())),
                TITLECASE_SENTENCES => break_iterator::get_sentence_instance_ulocale(locale),
                _ => Err(LuceneError::illegal_argument(
                    "unknown titlecasing iterator option",
                )),
            },
        }
    }
}

/// Omit unchanged text when case-mapping with Edits.
///
/// Equivalent to `CaseMapImpl.WholeStringBreakIterator` (private): the
/// whole string is one segment.
///
/// **Divergence.** Its unsupported methods throw
/// `UnsupportedOperationException("should not occur")` in Java
/// (`CaseMapImpl.java`, `notImplemented()`); the `BreakIterator` trait
/// methods return plain values, so they panic with that text here. ICU
/// never calls them.
#[derive(Debug, Clone, Default)]
struct WholeStringBreakIterator {
    length: i32,
    locales: BreakIteratorLocales,
}

fn not_implemented() -> ! {
    panic!("{}", unsupported_operation("should not occur"))
}

impl BreakIterator for WholeStringBreakIterator {
    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn first(&mut self) -> i32 {
        0
    }
    fn last(&mut self) -> i32 {
        not_implemented()
    }
    fn next_n(&mut self, _n: i32) -> i32 {
        not_implemented()
    }
    fn next(&mut self) -> i32 {
        self.length
    }
    fn previous(&mut self) -> i32 {
        not_implemented()
    }
    fn following(&mut self, _offset: i32) -> i32 {
        not_implemented()
    }
    fn current(&self) -> i32 {
        not_implemented()
    }
    fn get_text(&self) -> Option<&dyn CharacterIterator> {
        not_implemented()
    }
    fn set_text(&mut self, new_text: Option<Box<dyn CharacterIterator>>) {
        // Java dereferences the argument (NullPointerException for null).
        self.length = new_text
            .expect("java.lang.NullPointerException")
            .get_end_index();
    }
    fn set_text_char_sequence(&mut self, new_text: std::sync::Arc<[u16]>) {
        self.length = new_text.len() as i32;
    }
    fn set_text_string(&mut self, new_text: &[u16]) {
        self.length = new_text.len() as i32;
    }
    fn locales(&self) -> &BreakIteratorLocales {
        &self.locales
    }
    fn locales_mut(&mut self) -> &mut BreakIteratorLocales {
        &mut self.locales
    }
}

fn append_code_point(a: &mut Vec<u16>, c: i32) -> i32 {
    if c <= 0xffff {
        a.push(c as u16);
        1
    } else {
        a.push((0xd7c0 + (c >> 10)) as u16);
        a.push((0xdc00 + (c & 0x3ff)) as u16);
        2
    }
}

/// Appends a full case mapping result, see `UCaseProps.MAX_STRING_LENGTH`.
fn append_result(
    result: i32,
    dest: &mut Vec<u16>,
    cp_length: i32,
    options: i32,
    edits: Option<&mut Edits>,
) -> Result<()> {
    // Decode the result.
    if result < 0 {
        // (not) original code point
        if let Some(e) = edits {
            e.add_unchanged(cp_length)?;
        }
        if (options & OMIT_UNCHANGED_TEXT) != 0 {
            return Ok(());
        }
        append_code_point(dest, !result);
    } else if result <= UCaseProps::MAX_STRING_LENGTH {
        // The mapping has already been appended to result.
        if let Some(e) = edits {
            e.add_replace(cp_length, result)?;
        }
    } else {
        // Append the single-code point mapping.
        let length = append_code_point(dest, result);
        if let Some(e) = edits {
            e.add_replace(cp_length, length)?;
        }
    }
    Ok(())
}

fn append_unchanged(
    src: &[u16],
    start: i32,
    length: i32,
    dest: &mut Vec<u16>,
    options: i32,
    edits: Option<&mut Edits>,
) -> Result<()> {
    if length > 0 {
        if let Some(e) = edits {
            e.add_unchanged(length)?;
        }
        if (options & OMIT_UNCHANGED_TEXT) != 0 {
            return Ok(());
        }
        dest.extend_from_slice(&src[start as usize..(start + length) as usize]);
    }
    Ok(())
}

fn apply_edits(src: &[u16], replacement_chars: &[u16], edits: &Edits) -> Vec<u16> {
    if !edits.has_changes() {
        return src.to_vec();
    }
    let mut result: Vec<u16> =
        Vec::with_capacity((src.len() as i32 + edits.length_delta()).max(0) as usize);
    let mut ei = edits.get_coarse_iterator();
    while ei.next() {
        if ei.has_change() {
            let i = ei.replacement_index();
            result
                .extend_from_slice(&replacement_chars[i as usize..(i + ei.new_length()) as usize]);
        } else {
            let i = ei.source_index();
            result.extend_from_slice(&src[i as usize..(i + ei.old_length()) as usize]);
        }
    }
    result
}

/// `CASE_TRIE`.
fn case_trie() -> &'static crate::r#impl::trie2_16::Trie2_16 {
    UCaseProps::get_trie()
}

/// caseLocale >= 0: Lowercases [srcStart, srcLimit[ but takes context [0, src.length()[ into account.
/// caseLocale < 0: Case-folds [srcStart, srcLimit[.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
fn internal_to_lower(
    case_locale: i32,
    options: i32,
    src: &[u16],
    src_start: i32,
    src_limit: i32,
    mut iter: Option<&mut StringContextIterator<'_>>,
    dest: &mut Vec<u16>,
    mut edits: Option<&mut Edits>,
) -> Result<()> {
    let latin_to_lower: &[i8; 0x180] = if case_locale == UCaseProps::LOC_ROOT
        || (if case_locale >= 0 {
            !(case_locale == UCaseProps::LOC_TURKISH || case_locale == UCaseProps::LOC_LITHUANIAN)
        } else {
            (options & UCaseProps::FOLD_CASE_OPTIONS_MASK) == UCharacter::FOLD_CASE_DEFAULT
        }) {
        &LatinCase::TO_LOWER_NORMAL
    } else {
        &LatinCase::TO_LOWER_TR_LT
    };
    let mut prev = src_start;
    let mut src_index = src_start;
    let mut local_iter: Option<StringContextIterator<'_>> = None;
    'outer: loop {
        // fast path for simple cases
        let mut lead: u16;
        loop {
            if src_index >= src_limit {
                break 'outer;
            }
            lead = src[src_index as usize];
            let delta: i32;
            if lead < LatinCase::LONG_S {
                let d = latin_to_lower[lead as usize];
                if d == LatinCase::EXC {
                    break;
                }
                src_index += 1;
                if d == 0 {
                    continue;
                }
                delta = d as i32;
            } else if lead >= 0xd800 {
                break; // surrogate or higher
            } else {
                let props = case_trie().get_from_u16_single_lead(lead);
                if UCaseProps::props_has_exception(props) {
                    break;
                }
                src_index += 1;
                if !UCaseProps::is_upper_or_title_from_props(props) || {
                    let d = UCaseProps::get_delta(props);
                    d == 0
                } {
                    continue;
                }
                delta = UCaseProps::get_delta(props);
            }
            lead = lead.wrapping_add(delta as u16);
            append_unchanged(
                src,
                prev,
                src_index - 1 - prev,
                dest,
                options,
                edits.as_deref_mut(),
            )?;
            dest.push(lead);
            if let Some(e) = edits.as_deref_mut() {
                e.add_replace(1, 1)?;
            }
            prev = src_index;
        }
        // slow path
        let cp_start = src_index;
        src_index += 1;
        let c;
        if is_high_surrogate(lead)
            && src_index < src_limit
            && is_low_surrogate(src[src_index as usize])
        {
            c = to_code_point(lead, src[src_index as usize]);
            src_index += 1;
        } else {
            c = lead as i32;
        }
        // We need to append unchanged text before calling the UCaseProps.toFullXyz() methods
        // because they will sometimes append their mapping to dest,
        // and that must be after copying the previous text.
        append_unchanged(
            src,
            prev,
            cp_start - prev,
            dest,
            options,
            edits.as_deref_mut(),
        )?;
        prev = cp_start;
        let r;
        if case_locale >= 0 {
            if let Some(i) = iter.as_deref_mut() {
                i.set_cp_start_and_limit(cp_start, src_index);
                r = UCaseProps::instance().to_full_lower(
                    c,
                    Some(i as &mut dyn ContextIterator),
                    dest,
                    case_locale,
                );
            } else {
                let i = match local_iter.as_mut() {
                    Some(i) => {
                        i.set_cp_start_and_limit(cp_start, src_index);
                        i
                    }
                    None => {
                        local_iter.insert(StringContextIterator::new_cp(src, cp_start, src_index))
                    }
                };
                r = UCaseProps::instance().to_full_lower(
                    c,
                    Some(i as &mut dyn ContextIterator),
                    dest,
                    case_locale,
                );
            }
        } else {
            r = UCaseProps::instance().to_full_folding(c, dest, options);
        }
        if r >= 0 {
            append_result(r, dest, src_index - cp_start, options, edits.as_deref_mut())?;
            prev = src_index;
        }
    }
    append_unchanged(src, prev, src_index - prev, dest, options, edits)
}

#[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
fn internal_to_upper(
    case_locale: i32,
    options: i32,
    src: &[u16],
    dest: &mut Vec<u16>,
    mut edits: Option<&mut Edits>,
) -> Result<()> {
    let mut iter: Option<StringContextIterator<'_>> = None;
    let latin_to_upper: &[i8; 0x180] = if case_locale == UCaseProps::LOC_TURKISH {
        &LatinCase::TO_UPPER_TR
    } else {
        &LatinCase::TO_UPPER_NORMAL
    };
    let mut prev = 0;
    let mut src_index = 0;
    let src_length = src.len() as i32;
    'outer: loop {
        // fast path for simple cases
        let mut lead: u16;
        loop {
            if src_index >= src_length {
                break 'outer;
            }
            lead = src[src_index as usize];
            let delta: i32;
            if lead < LatinCase::LONG_S {
                let d = latin_to_upper[lead as usize];
                if d == LatinCase::EXC {
                    break;
                }
                src_index += 1;
                if d == 0 {
                    continue;
                }
                delta = d as i32;
            } else if lead >= 0xd800 {
                break; // surrogate or higher
            } else {
                let props = case_trie().get_from_u16_single_lead(lead);
                if UCaseProps::props_has_exception(props) {
                    break;
                }
                src_index += 1;
                if UCaseProps::get_type_from_props(props) != UCaseProps::LOWER
                    || UCaseProps::get_delta(props) == 0
                {
                    continue;
                }
                delta = UCaseProps::get_delta(props);
            }
            lead = lead.wrapping_add(delta as u16);
            append_unchanged(
                src,
                prev,
                src_index - 1 - prev,
                dest,
                options,
                edits.as_deref_mut(),
            )?;
            dest.push(lead);
            if let Some(e) = edits.as_deref_mut() {
                e.add_replace(1, 1)?;
            }
            prev = src_index;
        }
        // slow path
        let cp_start = src_index;
        src_index += 1;
        let c;
        if is_high_surrogate(lead)
            && src_index < src_length
            && is_low_surrogate(src[src_index as usize])
        {
            c = to_code_point(lead, src[src_index as usize]);
            src_index += 1;
        } else {
            c = lead as i32;
        }
        let it = match iter.as_mut() {
            Some(i) => {
                i.set_cp_start_and_limit(cp_start, src_index);
                i
            }
            None => iter.insert(StringContextIterator::new_cp(src, cp_start, src_index)),
        };
        // We need to append unchanged text before calling the UCaseProps.toFullXyz() methods
        // because they will sometimes append their mapping to dest,
        // and that must be after copying the previous text.
        append_unchanged(
            src,
            prev,
            cp_start - prev,
            dest,
            options,
            edits.as_deref_mut(),
        )?;
        prev = cp_start;
        let r = UCaseProps::instance().to_full_upper(
            c,
            Some(it as &mut dyn ContextIterator),
            dest,
            case_locale,
        );
        if r >= 0 {
            append_result(r, dest, src_index - cp_start, options, edits.as_deref_mut())?;
            prev = src_index;
        }
    }
    append_unchanged(src, prev, src_index - prev, dest, options, edits)
}

impl CaseMapImpl {
    /// `toLower(int caseLocale, int options, CharSequence src)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_lower_str(case_locale: i32, options: i32, src: &[u16]) -> Result<Vec<u16>> {
        if src.len() <= 100 && (options & OMIT_UNCHANGED_TEXT) == 0 {
            if src.is_empty() {
                return Ok(src.to_vec());
            }
            // Collect and apply only changes.
            // Good if no or few changes. Bad (slow) if many changes.
            let mut edits = Edits::new();
            let mut replacement_chars = Vec::new();
            Self::to_lower(
                case_locale,
                options | OMIT_UNCHANGED_TEXT,
                src,
                &mut replacement_chars,
                Some(&mut edits),
            )?;
            Ok(apply_edits(src, &replacement_chars, &edits))
        } else {
            let mut dest = Vec::with_capacity(src.len());
            Self::to_lower(case_locale, options, src, &mut dest, None)?;
            Ok(dest)
        }
    }

    /// `toLower(int caseLocale, int options, CharSequence src, A dest,
    /// Edits edits)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_lower<'d>(
        case_locale: i32,
        options: i32,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        mut edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        if let Some(e) = edits.as_deref_mut() {
            e.reset();
        }
        internal_to_lower(
            case_locale,
            options,
            src,
            0,
            src.len() as i32,
            None,
            dest,
            edits,
        )?;
        Ok(dest)
    }

    /// `toUpper(int caseLocale, int options, CharSequence src)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_upper_str(case_locale: i32, options: i32, src: &[u16]) -> Result<Vec<u16>> {
        if src.len() <= 100 && (options & OMIT_UNCHANGED_TEXT) == 0 {
            if src.is_empty() {
                return Ok(src.to_vec());
            }
            // Collect and apply only changes.
            // Good if no or few changes. Bad (slow) if many changes.
            let mut edits = Edits::new();
            let mut replacement_chars = Vec::new();
            Self::to_upper(
                case_locale,
                options | OMIT_UNCHANGED_TEXT,
                src,
                &mut replacement_chars,
                Some(&mut edits),
            )?;
            Ok(apply_edits(src, &replacement_chars, &edits))
        } else {
            let mut dest = Vec::with_capacity(src.len());
            Self::to_upper(case_locale, options, src, &mut dest, None)?;
            Ok(dest)
        }
    }

    /// `toUpper(int caseLocale, int options, CharSequence src, A dest,
    /// Edits edits)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_upper<'d>(
        case_locale: i32,
        options: i32,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        mut edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        if let Some(e) = edits.as_deref_mut() {
            e.reset();
        }
        if case_locale == UCaseProps::LOC_GREEK {
            return GreekUpper::to_upper(options, src, dest, edits);
        }
        internal_to_upper(case_locale, options, src, dest, edits)?;
        Ok(dest)
    }

    /// `toTitle(int caseLocale, int options, BreakIterator iter,
    /// CharSequence src)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_title_str(
        case_locale: i32,
        options: i32,
        iter: &mut dyn BreakIterator,
        src: &[u16],
    ) -> Result<Vec<u16>> {
        if src.len() <= 100 && (options & OMIT_UNCHANGED_TEXT) == 0 {
            if src.is_empty() {
                return Ok(src.to_vec());
            }
            // Collect and apply only changes.
            // Good if no or few changes. Bad (slow) if many changes.
            let mut edits = Edits::new();
            let mut replacement_chars = Vec::new();
            Self::to_title(
                case_locale,
                options | OMIT_UNCHANGED_TEXT,
                iter,
                src,
                &mut replacement_chars,
                Some(&mut edits),
            )?;
            Ok(apply_edits(src, &replacement_chars, &edits))
        } else {
            let mut dest = Vec::with_capacity(src.len());
            Self::to_title(case_locale, options, iter, src, &mut dest, None)?;
            Ok(dest)
        }
    }

    /// `toTitle(int caseLocale, int options, BreakIterator titleIter,
    /// CharSequence src, A dest, Edits edits)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn to_title<'d>(
        case_locale: i32,
        options: i32,
        title_iter: &mut dyn BreakIterator,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        mut edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        if let Some(e) = edits.as_deref_mut() {
            e.reset();
        }

        /* set up local variables */
        let mut iter = StringContextIterator::new(src);
        let src_length = src.len() as i32;
        let mut prev = 0;
        let mut is_first_index = true;

        /* titlecasing loop */
        while prev < src_length {
            /* find next index where to titlecase */
            let mut index;
            if is_first_index {
                is_first_index = false;
                index = title_iter.first();
            } else {
                index = title_iter.next();
            }
            if index == DONE || index > src_length {
                index = src_length;
            }

            /*
             * Segment [prev..index[ into 3 parts:
             * a) skipped characters (copy as-is) [prev..titleStart[
             * b) first letter (titlecase)         [titleStart..titleLimit[
             * c) subsequent characters (lowercase)                 [titleLimit..index[
             */
            if prev < index {
                // Find and copy skipped characters [prev..titleStart[
                let mut title_start = prev;
                iter.set_limit(index);
                let mut c = iter.next_case_map_cp();
                if (options & UCharacter::TITLECASE_NO_BREAK_ADJUSTMENT) == 0 {
                    // Adjust the titlecasing index to the next cased character,
                    // or to the next letter/number/symbol/private use.
                    // Stop with titleStart<titleLimit<=index
                    // if there is a character to be titlecased,
                    // or else stop with titleStart==titleLimit==index.
                    let to_cased = (options & TITLECASE_ADJUST_TO_CASED) != 0;
                    while (if to_cased {
                        UCaseProps::NONE == UCaseProps::instance().get_type(c)
                    } else {
                        !is_lns(c)
                    }) && {
                        c = iter.next_case_map_cp();
                        c >= 0
                    } {}
                    // If c<0 then we have only uncased characters in [prev..index[
                    // and stopped with titleStart==titleLimit==index.
                    title_start = iter.get_cp_start();
                    if prev < title_start {
                        append_unchanged(
                            src,
                            prev,
                            title_start - prev,
                            dest,
                            options,
                            edits.as_deref_mut(),
                        )?;
                    }
                }

                if title_start < index {
                    // titlecase c which is from [titleStart..titleLimit[
                    c = UCaseProps::instance().to_full_title(
                        c,
                        Some(&mut iter as &mut dyn ContextIterator),
                        dest,
                        case_locale,
                    );
                    append_result(c, dest, iter.get_cp_length(), options, edits.as_deref_mut())?;

                    // Special case Dutch IJ titlecasing
                    let title_limit;
                    if title_start + 1 < index && case_locale == UCaseProps::LOC_DUTCH {
                        if c < 0 {
                            c = !c;
                        }
                        if c == b'I' as i32 || c == 0xCD {
                            title_limit = maybe_title_dutch_ij(
                                src,
                                c,
                                title_start + 1,
                                index,
                                dest,
                                options,
                                edits.as_deref_mut(),
                            )?;
                            iter.move_to(title_limit);
                        } else {
                            title_limit = iter.get_cp_limit();
                        }
                    } else {
                        title_limit = iter.get_cp_limit();
                    }

                    // lowercase [titleLimit..index[
                    if title_limit < index {
                        if (options & UCharacter::TITLECASE_NO_LOWERCASE) == 0 {
                            // Normal operation: Lowercase the rest of the word.
                            internal_to_lower(
                                case_locale,
                                options,
                                src,
                                title_limit,
                                index,
                                Some(&mut iter),
                                dest,
                                edits.as_deref_mut(),
                            )?;
                        } else {
                            // Optionally just copy the rest of the word unchanged.
                            append_unchanged(
                                src,
                                title_limit,
                                index - title_limit,
                                dest,
                                options,
                                edits.as_deref_mut(),
                            )?;
                        }
                        iter.move_to_limit();
                    }
                }
            }

            prev = index;
        }
        Ok(dest)
    }

    /// `fold(int options, CharSequence src)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn fold_str(options: i32, src: &[u16]) -> Result<Vec<u16>> {
        if src.len() <= 100 && (options & OMIT_UNCHANGED_TEXT) == 0 {
            if src.is_empty() {
                return Ok(src.to_vec());
            }
            // Collect and apply only changes.
            // Good if no or few changes. Bad (slow) if many changes.
            let mut edits = Edits::new();
            let mut replacement_chars = Vec::new();
            Self::fold(
                options | OMIT_UNCHANGED_TEXT,
                src,
                &mut replacement_chars,
                Some(&mut edits),
            )?;
            Ok(apply_edits(src, &replacement_chars, &edits))
        } else {
            let mut dest = Vec::with_capacity(src.len());
            Self::fold(options, src, &mut dest, None)?;
            Ok(dest)
        }
    }

    /// `fold(int options, CharSequence src, A dest, Edits edits)`.
    ///
    /// # Errors
    ///
    /// The errors of `Edits`.
    pub fn fold<'d>(
        options: i32,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        mut edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        if let Some(e) = edits.as_deref_mut() {
            e.reset();
        }
        internal_to_lower(-1, options, src, 0, src.len() as i32, None, dest, edits)?;
        Ok(dest)
    }
}

/// Input: c is a letter I with or without acute accent.
/// start is the index in src after c, and is less than segmentLimit.
/// If a plain i/I is followed by a plain j/J,
/// or an i/I with acute (precomposed or decomposed) is followed by a j/J with acute,
/// then we output accordingly.
///
/// Returns the src index after the titlecased j/J, or `start` if none was
/// found.
fn maybe_title_dutch_ij(
    src: &[u16],
    c: i32,
    start: i32,
    segment_limit: i32,
    dest: &mut Vec<u16>,
    options: i32,
    mut edits: Option<&mut Edits>,
) -> Result<i32> {
    debug_assert!(start < segment_limit);

    let mut start = start;
    let mut index = start;
    let mut with_acute = false;

    // If the conditions are met, then the following variables tell us what to output.
    let mut unchanged1 = 0; // code units before the j, or the whole sequence (0..3)
    let mut do_title_j = false; // true if the j needs to be titlecased
    let mut unchanged2 = 0; // after the j (0 or 1)

    // next character after the first letter
    let mut c2 = src[index as usize];
    index += 1;

    // Is the first letter an i/I with accent?
    if c == b'I' as i32 {
        if c2 == ACUTE {
            with_acute = true;
            unchanged1 = 1;
            if index == segment_limit {
                return Ok(start);
            }
            c2 = src[index as usize];
            index += 1;
        }
    } else {
        // Í
        with_acute = true;
    }

    // Is the next character a j/J?
    if c2 == b'j' as u16 {
        do_title_j = true;
    } else if c2 == b'J' as u16 {
        unchanged1 += 1;
    } else {
        return Ok(start);
    }

    // A plain i/I must be followed by a plain j/J.
    // An i/I with acute must be followed by a j/J with acute.
    if with_acute {
        if index == segment_limit || src[index as usize] != ACUTE {
            return Ok(start);
        }
        index += 1;
        if do_title_j {
            unchanged2 = 1;
        } else {
            unchanged1 += 1;
        }
    }

    // There must not be another combining mark.
    if index < segment_limit {
        let cp = code_point_at(src, index);
        let bit = 1 << UCharacter::get_type(cp);
        if (bit & U_GC_M_MASK) != 0 {
            return Ok(start);
        }
    }

    // Output the rest of the Dutch IJ.
    append_unchanged(src, start, unchanged1, dest, options, edits.as_deref_mut())?;
    start += unchanged1;
    if do_title_j {
        dest.push(b'J' as u16);
        if let Some(e) = edits.as_deref_mut() {
            e.add_replace(1, 1)?;
        }
        start += 1;
    }
    append_unchanged(src, start, unchanged2, dest, options, edits)?;
    debug_assert!(start + unchanged2 == index);
    Ok(index)
}

/// Greek string uppercasing with a state machine.
/// Similar to a Unicode normalization Normalizer2.
///
/// Equivalent to `CaseMapImpl.GreekUpper` (private).
struct GreekUpper;

// Data bits.
const UPPER_MASK: i32 = 0x3ff;
const HAS_VOWEL: i32 = 0x1000;
const HAS_YPOGEGRAMMENI: i32 = 0x2000;
const HAS_ACCENT: i32 = 0x4000;
const HAS_DIALYTIKA: i32 = 0x8000;
// Further bits during data processing, not stored in the data map.
const HAS_COMBINING_DIALYTIKA: i32 = 0x10000;
const HAS_OTHER_GREEK_DIACRITIC: i32 = 0x20000;

const HAS_VOWEL_AND_ACCENT: i32 = HAS_VOWEL | HAS_ACCENT;
const HAS_VOWEL_AND_ACCENT_AND_DIALYTIKA: i32 = HAS_VOWEL_AND_ACCENT | HAS_DIALYTIKA;
const HAS_EITHER_DIALYTIKA: i32 = HAS_DIALYTIKA | HAS_COMBINING_DIALYTIKA;

// State bits.
const AFTER_CASED: i32 = 1;
const AFTER_VOWEL_WITH_COMBINING_ACCENT: i32 = 2;
const AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT: i32 = 4;

// Data generated by prototype code, see
// https://icu.unicode.org/design/case/greek-upper
// TODO: Move this data into ucase.icu.
/// `data0370`.
static DATA0370: [u16; 144] = [
    0x0370, 0x0370, 0x0372, 0x0372, 0x0000, 0x0000, 0x0376, 0x0376, 0x0000, 0x0000, 0x037A, 0x03FD,
    0x03FE, 0x03FF, 0x0000, 0x037F, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x5391, 0x0000,
    0x5395, 0x5397, 0x5399, 0x0000, 0x539F, 0x0000, 0x53A5, 0x53A9, 0xD399, 0x1391, 0x0392, 0x0393,
    0x0394, 0x1395, 0x0396, 0x1397, 0x0398, 0x1399, 0x039A, 0x039B, 0x039C, 0x039D, 0x039E, 0x139F,
    0x03A0, 0x03A1, 0x0000, 0x03A3, 0x03A4, 0x13A5, 0x03A6, 0x03A7, 0x03A8, 0x13A9, 0x9399, 0x93A5,
    0x5391, 0x5395, 0x5397, 0x5399, 0xD3A5, 0x1391, 0x0392, 0x0393, 0x0394, 0x1395, 0x0396, 0x1397,
    0x0398, 0x1399, 0x039A, 0x039B, 0x039C, 0x039D, 0x039E, 0x139F, 0x03A0, 0x03A1, 0x03A3, 0x03A3,
    0x03A4, 0x13A5, 0x03A6, 0x03A7, 0x03A8, 0x13A9, 0x9399, 0x93A5, 0x539F, 0x53A5, 0x53A9, 0x03CF,
    0x0392, 0x0398, 0x03D2, 0x43D2, 0x83D2, 0x03A6, 0x03A0, 0x03CF, 0x03D8, 0x03D8, 0x03DA, 0x03DA,
    0x03DC, 0x03DC, 0x03DE, 0x03DE, 0x03E0, 0x03E0, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x039A, 0x03A1, 0x03F9, 0x037F,
    0x03F4, 0x1395, 0x0000, 0x03F7, 0x03F7, 0x03F9, 0x03FA, 0x03FA, 0x03FC, 0x03FD, 0x03FE, 0x03FF,
];

/// `data1F00`.
static DATA1F00: [u16; 256] = [
    0x1391, 0x1391, 0x5391, 0x5391, 0x5391, 0x5391, 0x5391, 0x5391, 0x1391, 0x1391, 0x5391, 0x5391,
    0x5391, 0x5391, 0x5391, 0x5391, 0x1395, 0x1395, 0x5395, 0x5395, 0x5395, 0x5395, 0x0000, 0x0000,
    0x1395, 0x1395, 0x5395, 0x5395, 0x5395, 0x5395, 0x0000, 0x0000, 0x1397, 0x1397, 0x5397, 0x5397,
    0x5397, 0x5397, 0x5397, 0x5397, 0x1397, 0x1397, 0x5397, 0x5397, 0x5397, 0x5397, 0x5397, 0x5397,
    0x1399, 0x1399, 0x5399, 0x5399, 0x5399, 0x5399, 0x5399, 0x5399, 0x1399, 0x1399, 0x5399, 0x5399,
    0x5399, 0x5399, 0x5399, 0x5399, 0x139F, 0x139F, 0x539F, 0x539F, 0x539F, 0x539F, 0x0000, 0x0000,
    0x139F, 0x139F, 0x539F, 0x539F, 0x539F, 0x539F, 0x0000, 0x0000, 0x13A5, 0x13A5, 0x53A5, 0x53A5,
    0x53A5, 0x53A5, 0x53A5, 0x53A5, 0x0000, 0x13A5, 0x0000, 0x53A5, 0x0000, 0x53A5, 0x0000, 0x53A5,
    0x13A9, 0x13A9, 0x53A9, 0x53A9, 0x53A9, 0x53A9, 0x53A9, 0x53A9, 0x13A9, 0x13A9, 0x53A9, 0x53A9,
    0x53A9, 0x53A9, 0x53A9, 0x53A9, 0x5391, 0x5391, 0x5395, 0x5395, 0x5397, 0x5397, 0x5399, 0x5399,
    0x539F, 0x539F, 0x53A5, 0x53A5, 0x53A9, 0x53A9, 0x0000, 0x0000, 0x3391, 0x3391, 0x7391, 0x7391,
    0x7391, 0x7391, 0x7391, 0x7391, 0x3391, 0x3391, 0x7391, 0x7391, 0x7391, 0x7391, 0x7391, 0x7391,
    0x3397, 0x3397, 0x7397, 0x7397, 0x7397, 0x7397, 0x7397, 0x7397, 0x3397, 0x3397, 0x7397, 0x7397,
    0x7397, 0x7397, 0x7397, 0x7397, 0x33A9, 0x33A9, 0x73A9, 0x73A9, 0x73A9, 0x73A9, 0x73A9, 0x73A9,
    0x33A9, 0x33A9, 0x73A9, 0x73A9, 0x73A9, 0x73A9, 0x73A9, 0x73A9, 0x1391, 0x1391, 0x7391, 0x3391,
    0x7391, 0x0000, 0x5391, 0x7391, 0x1391, 0x1391, 0x5391, 0x5391, 0x3391, 0x0000, 0x1399, 0x0000,
    0x0000, 0x0000, 0x7397, 0x3397, 0x7397, 0x0000, 0x5397, 0x7397, 0x5395, 0x5395, 0x5397, 0x5397,
    0x3397, 0x0000, 0x0000, 0x0000, 0x1399, 0x1399, 0xD399, 0xD399, 0x0000, 0x0000, 0x5399, 0xD399,
    0x1399, 0x1399, 0x5399, 0x5399, 0x0000, 0x0000, 0x0000, 0x0000, 0x13A5, 0x13A5, 0xD3A5, 0xD3A5,
    0x03A1, 0x03A1, 0x53A5, 0xD3A5, 0x13A5, 0x13A5, 0x53A5, 0x53A5, 0x03A1, 0x0000, 0x0000, 0x0000,
    0x0000, 0x0000, 0x73A9, 0x33A9, 0x73A9, 0x0000, 0x53A9, 0x73A9, 0x539F, 0x539F, 0x53A9, 0x53A9,
    0x33A9, 0x0000, 0x0000, 0x0000,
];
/// `data2126`.
const DATA2126: i32 = 0x03A9 | HAS_VOWEL; // Ω

impl GreekUpper {
    fn get_letter_data(c: i32) -> i32 {
        if !(0x370..=0x2126).contains(&c) || (0x3ff < c && c < 0x1f00) {
            0
        } else if c <= 0x3ff {
            DATA0370[(c - 0x370) as usize] as i32
        } else if c <= 0x1fff {
            DATA1F00[(c - 0x1f00) as usize] as i32
        } else if c == 0x2126 {
            DATA2126
        } else {
            0
        }
    }

    /// Returns a non-zero value for each of the Greek combining diacritics
    /// listed in The Unicode Standard, version 8, chapter 7.2 Greek,
    /// plus some perispomeni look-alikes.
    fn get_diacritic_data(c: u16) -> i32 {
        match c {
            0x0300 | 0x0301 | 0x0342 | 0x0302 | 0x0303 | 0x0311 => HAS_ACCENT,
            0x0308 => HAS_COMBINING_DIALYTIKA,
            0x0344 => HAS_COMBINING_DIALYTIKA | HAS_ACCENT,
            0x0345 => HAS_YPOGEGRAMMENI,
            0x0304 | 0x0306 | 0x0313 | 0x0314 | 0x0343 => HAS_OTHER_GREEK_DIACRITIC,
            _ => 0,
        }
    }

    fn is_followed_by_cased_letter(s: &[u16], mut i: i32) -> bool {
        while i < s.len() as i32 {
            let c = code_point_at(s, i);
            let ty = UCaseProps::instance().get_type_or_ignorable(c);
            if (ty & UCaseProps::IGNORABLE) != 0 {
                // Case-ignorable, continue with the loop.
                i += char_count(c);
            } else if ty != UCaseProps::NONE {
                return true; // Followed by cased letter.
            } else {
                return false; // Uncased and not case-ignorable.
            }
        }
        false // Not followed by cased letter.
    }

    /// `toUpper(int options, CharSequence src, A dest, Edits edits)`.
    ///
    /// Several Greek letters and diacritics are uppercased and/or have
    /// their accents removed; see `CaseMapImpl.java`.
    fn to_upper<'d>(
        options: i32,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        mut edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        let mut state = 0;
        let mut i = 0;
        while i < src.len() as i32 {
            let c = code_point_at(src, i);
            let mut next_index = i + char_count(c);
            let mut next_state = 0;
            let ty = UCaseProps::instance().get_type_or_ignorable(c);
            if (ty & UCaseProps::IGNORABLE) != 0 {
                // c is case-ignorable
                next_state |= state & AFTER_CASED;
            } else if ty != UCaseProps::NONE {
                // c is cased
                next_state |= AFTER_CASED;
            }
            let mut data = Self::get_letter_data(c);
            if data > 0 {
                let mut upper = data & UPPER_MASK;
                // Add a dialytika to this iota or ypsilon vowel
                // if we removed a tonos from the previous vowel,
                // and that previous vowel did not also have (or gain) a dialytika.
                // Adding one only to the final vowel in a longer sequence
                // (which does not occur in normal writing) would require lookahead.
                // Set the same flag as for preserving an existing dialytika.
                if (data & HAS_VOWEL) != 0
                    && (state
                        & (AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT | AFTER_VOWEL_WITH_COMBINING_ACCENT))
                        != 0
                    && (upper == 0x399 || upper == 0x3A5)
                {
                    data |= if (state & AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT) != 0 {
                        HAS_DIALYTIKA
                    } else {
                        HAS_COMBINING_DIALYTIKA
                    };
                }
                let mut num_ypogegrammeni = 0; // Map each one to a trailing, spacing, capital iota.
                if (data & HAS_YPOGEGRAMMENI) != 0 {
                    num_ypogegrammeni = 1;
                }
                let has_precomposed_accent = (data & HAS_ACCENT) != 0;
                // Skip combining diacritics after this Greek letter.
                while next_index < src.len() as i32 {
                    let diacritic_data = Self::get_diacritic_data(src[next_index as usize]);
                    if diacritic_data != 0 {
                        data |= diacritic_data;
                        if (diacritic_data & HAS_YPOGEGRAMMENI) != 0 {
                            num_ypogegrammeni += 1;
                        }
                        next_index += 1;
                    } else {
                        break; // not a Greek diacritic
                    }
                }
                if (data & HAS_VOWEL_AND_ACCENT_AND_DIALYTIKA) == HAS_VOWEL_AND_ACCENT {
                    next_state |= if has_precomposed_accent {
                        AFTER_VOWEL_WITH_PRECOMPOSED_ACCENT
                    } else {
                        AFTER_VOWEL_WITH_COMBINING_ACCENT
                    };
                }
                // Map according to Greek rules.
                let mut add_tonos = false;
                if upper == 0x397
                    && (data & HAS_ACCENT) != 0
                    && num_ypogegrammeni == 0
                    && (state & AFTER_CASED) == 0
                    && !Self::is_followed_by_cased_letter(src, next_index)
                {
                    // Keep disjunctive "or" with (only) a tonos.
                    // We use the same "word boundary" conditions as for the Final_Sigma test.
                    if has_precomposed_accent {
                        upper = 0x389; // Preserve the precomposed form.
                    } else {
                        add_tonos = true;
                    }
                } else if (data & HAS_DIALYTIKA) != 0 {
                    // Preserve a vowel with dialytika in precomposed form if it exists.
                    if upper == 0x399 {
                        upper = 0x3AA;
                        data &= !HAS_EITHER_DIALYTIKA;
                    } else if upper == 0x3A5 {
                        upper = 0x3AB;
                        data &= !HAS_EITHER_DIALYTIKA;
                    }
                }

                let mut change;
                if edits.is_none() && (options & OMIT_UNCHANGED_TEXT) == 0 {
                    change = true; // common, simple usage
                } else {
                    // Find out first whether we are changing the text.
                    change = src[i as usize] as i32 != upper || num_ypogegrammeni > 0;
                    let mut i2 = i + 1;
                    if (data & HAS_EITHER_DIALYTIKA) != 0 {
                        change |= i2 >= next_index || src[i2 as usize] != 0x308;
                        i2 += 1;
                    }
                    if add_tonos {
                        change |= i2 >= next_index || src[i2 as usize] != 0x301;
                        i2 += 1;
                    }
                    let old_length = next_index - i;
                    let new_length = (i2 - i) + num_ypogegrammeni;
                    change |= old_length != new_length;
                    if change {
                        if let Some(e) = edits.as_deref_mut() {
                            e.add_replace(old_length, new_length)?;
                        }
                    } else {
                        if let Some(e) = edits.as_deref_mut() {
                            e.add_unchanged(old_length)?;
                        }
                        // Write unchanged text?
                        change = (options & OMIT_UNCHANGED_TEXT) == 0;
                    }
                }

                if change {
                    dest.push(upper as u16);
                    if (data & HAS_EITHER_DIALYTIKA) != 0 {
                        dest.push(0x308); // restore or add a dialytika
                    }
                    if add_tonos {
                        dest.push(0x301);
                    }
                    while num_ypogegrammeni > 0 {
                        dest.push(0x399);
                        num_ypogegrammeni -= 1;
                    }
                }
            } else {
                let r = UCaseProps::instance().to_full_upper(c, None, dest, UCaseProps::LOC_GREEK);
                append_result(r, dest, next_index - i, options, edits.as_deref_mut())?;
            }
            i = next_index;
            state = next_state;
        }
        Ok(dest)
    }
}
