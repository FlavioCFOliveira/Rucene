// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UCaseProps`.

use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{code_point_at, ByteBuffer};
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::trie2_16::Trie2_16;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;
use crate::util::u_locale::ULocale;

fn io_exception(message: String) -> LuceneError {
    LuceneError::from(std::io::Error::other(message))
}

/// Iterator for string case mappings, which need to look at the context
/// (surrounding text) of a given character for conditional mappings.
///
/// Equivalent to the interface `UCaseProps.ContextIterator`.
pub trait ContextIterator {
    /// `reset(int dir)`: resets the iterator for forward (`dir > 0`) or
    /// backward (`dir < 0`) iteration; `dir == 0` keeps the direction and
    /// position.
    fn reset(&mut self, dir: i32);

    /// `next()`: the next code point, or a negative value at the end.
    fn next(&mut self) -> i32;
}

static FLAGS_OFFSET: [i8; 256] = [
    0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4,
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5,
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5,
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6,
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5,
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6,
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6,
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7,
    1, 2, 2, 3, 2, 3, 3, 4, 2, 3, 3, 4, 3, 4, 4, 5,
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6,
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6,
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7,
    2, 3, 3, 4, 3, 4, 4, 5, 3, 4, 4, 5, 4, 5, 5, 6,
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7,
    3, 4, 4, 5, 4, 5, 5, 6, 4, 5, 5, 6, 5, 6, 6, 7,
    4, 5, 5, 6, 5, 6, 6, 7, 5, 6, 6, 7, 6, 7, 7, 8,
];

const I_DOT: &[u16] = &[0x69, 0x307]; /* "i̇" */
const J_DOT: &[u16] = &[0x6a, 0x307]; /* "j̇" */
const I_OGONEK_DOT: &[u16] = &[0x12f, 0x307]; /* "į̇" */
const I_DOT_GRAVE: &[u16] = &[0x69, 0x307, 0x300]; /* "i̇̀" */
const I_DOT_ACUTE: &[u16] = &[0x69, 0x307, 0x301]; /* "i̇́" */
const I_DOT_TILDE: &[u16] = &[0x69, 0x307, 0x303]; /* "i̇̃" */

const DATA_FILE_NAME: &str = "ucase.icu";
/* format "cASE" */
const FMT: i32 = 0x63415345;

/* indexes into indexes[] */
//private static final int IX_INDEX_TOP=0;
//private static final int IX_LENGTH=1;
const IX_TRIE_SIZE: usize = 2;
const IX_EXC_LENGTH: usize = 3;
const IX_UNFOLD_LENGTH: usize = 4;
//private static final int IX_MAX_FULL_LENGTH=15;
const IX_TOP: i32 = 16;

const EXCEPTION: i32 = 8;
const SENSITIVE: i32 = 0x10;

const DOT_MASK: i32 = 0x60;
//private static final int NO_DOT=        0;      /* normal characters with cc=0 */
const SOFT_DOTTED: i32 = 0x20; /* soft-dotted characters with cc=0 */
const ABOVE: i32 = 0x40; /* "above" accents with cc=230 */
const OTHER_ACCENT: i32 = 0x60; /* other accent character (0<cc!=230) */

/* no exception: bits 15..7 are a 9-bit signed case mapping delta */
const DELTA_SHIFT: i32 = 7;
//private static final int DELTA_MASK=    0xff80;
//private static final int MAX_DELTA=     0xff;
//private static final int MIN_DELTA=     (-MAX_DELTA-1);

/* exception: bits 15..4 are an unsigned 12-bit index into the exceptions array */
const EXC_SHIFT: i32 = 4;
//private static final int EXC_MASK=      0xfff0;
//private static final int MAX_EXCEPTIONS=((EXC_MASK>>EXC_SHIFT)+1);

/* definitions for 16-bit main exceptions word ------------------------------ */

/* first 8 bits indicate values in optional slots */
const EXC_LOWER: i32 = 0;
const EXC_FOLD: i32 = 1;
const EXC_UPPER: i32 = 2;
const EXC_TITLE: i32 = 3;
const EXC_DELTA: i32 = 4;
//private static final int EXC_5=5;           /* reserved */
const EXC_CLOSURE: i32 = 6;
const EXC_FULL_MAPPINGS: i32 = 7;
//private static final int EXC_ALL_SLOTS=8;   /* one past the last slot */

/* each slot is 2 uint16_t instead of 1 */
const EXC_DOUBLE_SLOTS: i32 = 0x100;

const EXC_NO_SIMPLE_CASE_FOLDING: i32 = 0x200;
const EXC_DELTA_IS_NEGATIVE: i32 = 0x400;
const EXC_SENSITIVE: i32 = 0x800;

/* EXC_DOT_MASK=DOT_MASK<<EXC_DOT_SHIFT */
const EXC_DOT_SHIFT: i32 = 7;

/* EXC_CONDITIONAL_SPECIAL and EXC_CONDITIONAL_FOLD are for internal use */
const EXC_CONDITIONAL_SPECIAL: i32 = 0x4000;
const EXC_CONDITIONAL_FOLD: i32 = 0x8000;

/* definitions for lengths word for full case mappings */
const FULL_LOWER: i32 = 0xf;
//private static final int FULL_FOLDING=  0xf0;
//private static final int FULL_UPPER=    0xf00;
//private static final int FULL_TITLE=    0xf000;

/* maximum lengths */
//private static final int FULL_MAPPINGS_MAX_LENGTH=4*0xf;
const CLOSURE_MAX_LENGTH: i32 = 0xf;

/* constants for reverse case folding ("unfold") data */
const UNFOLD_ROWS: usize = 0;
const UNFOLD_ROW_WIDTH: usize = 1;
const UNFOLD_STRING_WIDTH: usize = 2;

fn get_exceptions_offset(props: i32) -> i32 {
    props >> EXC_SHIFT
}

fn has_slot(flags: i32, index: i32) -> bool {
    (flags & (1 << index)) != 0
}

fn slot_offset(flags: i32, index: i32) -> i32 {
    FLAGS_OFFSET[(flags & ((1 << index) - 1)) as usize] as i32
}

fn is_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 4
}

/// Reborrows an optional context iterator for one call.
fn reborrow<'s>(iter: &'s mut Option<&mut dyn ContextIterator>) -> Option<&'s mut dyn ContextIterator> {
    match iter {
        Some(i) => Some(&mut **i),
        None => None,
    }
}

/// Low-level Unicode character/string case mapping code.
///
/// Equivalent to `com.ibm.icu.impl.UCaseProps` (reads `ucase.icu`).
/// Java's shared `dummyStringBuilder` (a never-read scratch output) is a
/// local buffer per call here.
#[derive(Debug)]
pub struct UCaseProps {
    indexes: Vec<i32>,
    exceptions: Vec<u16>,
    unfold: Option<Vec<u16>>,
    trie: Trie2_16,
}

/// `INSTANCE`. A failure of Java's static initializer surfaces as
/// `ExceptionInInitializerError`, here a panic.
static INSTANCE: LazyLock<UCaseProps> = LazyLock::new(|| match UCaseProps::new() {
    Ok(p) => p,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

/// Fast case mapping data for ASCII/Latin.
/// Linear arrays of delta bytes: 0=no mapping; EXC=exception.
/// Deltas must not cross the ASCII boundary, or else they cannot be easily used
/// in simple UTF-8 code.
///
/// Equivalent to `UCaseProps.LatinCase` (package-private).
pub(crate) struct LatinCase;

impl LatinCase {
    /// Case mapping/folding data for code points up to U+017F.
    pub(crate) const LIMIT: u16 = 0x180;
    /// U+017F case-folds and uppercases crossing the ASCII boundary.
    pub(crate) const LONG_S: u16 = 0x17f;
    /// Exception: Complex mapping, or too-large delta.
    pub(crate) const EXC: i8 = -0x80;

    /// `TO_LOWER_NORMAL`.
    pub(crate) const TO_LOWER_NORMAL: [i8; 0x180] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, Self::EXC, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 0, 32, 32, 32, 32, 32, 32, 32, Self::EXC,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        Self::EXC, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 1, 0, 1,
        0, 1, 0, 1, 0, 1, 0, 1, 0, Self::EXC, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, -121, 1, 0, 1, 0, 1, 0, Self::EXC,
    ];

    /// `TO_LOWER_TR_LT`.
    pub(crate) const TO_LOWER_TR_LT: [i8; 0x180] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 32, 32, 32, 32, 32, 32, 32, 32, Self::EXC, Self::EXC, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, Self::EXC, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, Self::EXC, Self::EXC, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 0, 32, 32, 32, 32, 32, 32, 32, Self::EXC,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, Self::EXC, 0, 1, 0, 1, 0, Self::EXC, 0,
        Self::EXC, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 1, 0, 1,
        0, 1, 0, 1, 0, 1, 0, 1, 0, Self::EXC, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0,
        1, 0, 1, 0, 1, 0, 1, 0, -121, 1, 0, 1, 0, 1, 0, Self::EXC,
    ];

    /// `TO_UPPER_NORMAL`.
    pub(crate) const TO_UPPER_NORMAL: [i8; 0x180] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32,
        -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, Self::EXC, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, Self::EXC,
        -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32,
        -32, -32, -32, -32, -32, -32, -32, 0, -32, -32, -32, -32, -32, -32, -32, 121,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, Self::EXC, 0, -1, 0, -1, 0, -1, 0, 0, -1, 0, -1, 0, -1, 0,
        -1, 0, -1, 0, -1, 0, -1, 0, -1, Self::EXC, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, 0, -1, 0, -1, 0, -1, Self::EXC,
    ];

    /// `TO_UPPER_TR`.
    pub(crate) const TO_UPPER_TR: [i8; 0x180] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, -32, -32, -32, -32, -32, -32, -32, -32, Self::EXC, -32, -32, -32, -32, -32, -32,
        -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, Self::EXC, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, Self::EXC,
        -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32,
        -32, -32, -32, -32, -32, -32, -32, 0, -32, -32, -32, -32, -32, -32, -32, 121,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, Self::EXC, 0, -1, 0, -1, 0, -1, 0, 0, -1, 0, -1, 0, -1, 0,
        -1, 0, -1, 0, -1, 0, -1, 0, -1, Self::EXC, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1,
        0, -1, 0, -1, 0, -1, 0, -1, 0, 0, -1, 0, -1, 0, -1, Self::EXC,
    ];
}

impl UCaseProps {
    /// `MAX_STRING_LENGTH`: the maximum length of a full case mapping string.
    pub const MAX_STRING_LENGTH: i32 = 0x1f;

    /// `LOC_ROOT`.
    pub const LOC_ROOT: i32 = 1;
    /// `LOC_TURKISH`.
    pub const LOC_TURKISH: i32 = 2;
    /// `LOC_LITHUANIAN`.
    pub const LOC_LITHUANIAN: i32 = 3;
    /// `LOC_GREEK`.
    pub const LOC_GREEK: i32 = 4;
    /// `LOC_DUTCH`.
    pub const LOC_DUTCH: i32 = 5;
    /// `LOC_ARMENIAN`.
    pub const LOC_ARMENIAN: i32 = 6;

    /// `FOLD_CASE_OPTIONS_MASK`: bit mask for getting just the options from
    /// a string compare options word that are relevant for case folding.
    pub const FOLD_CASE_OPTIONS_MASK: i32 = 7;

    /// `TYPE_MASK`.
    pub const TYPE_MASK: i32 = 3;
    /// `NONE`.
    pub const NONE: i32 = 0;
    /// `LOWER`.
    pub const LOWER: i32 = 1;
    /// `UPPER`.
    pub const UPPER: i32 = 2;
    /// `TITLE`.
    pub const TITLE: i32 = 3;
    /// `IGNORABLE`.
    pub const IGNORABLE: i32 = 4;

    // constructors etc. --------------------------------------------------- ***

    // port of ucase_openProps()
    fn new() -> Result<Self> {
        let mut bytes = ICUBinary::get_required_data(DATA_FILE_NAME)?;
        Self::read_data(&mut bytes)
    }

    fn read_data(bytes: &mut ByteBuffer) -> Result<Self> {
        // read the header
        ICUBinary::read_header(bytes, FMT, Some(&is_acceptable as &dyn Fn(&[u8; 4]) -> bool))?;

        // read indexes[]
        let count = bytes.get_int()?;
        if count < IX_TOP {
            return Err(io_exception(format!("indexes[0] too small in {DATA_FILE_NAME}")));
        }
        let mut indexes = vec![0i32; count as usize];

        indexes[0] = count;
        for i in 1..count as usize {
            indexes[i] = bytes.get_int()?;
        }

        // read the trie
        let trie = Trie2_16::create_from_serialized(bytes)?;
        let expected_trie_length = indexes[IX_TRIE_SIZE];
        let trie_length = trie.get_serialized_length();
        if trie_length > expected_trie_length {
            return Err(io_exception(format!("{DATA_FILE_NAME}: not enough bytes for the trie")));
        }
        // skip padding after trie bytes
        ICUBinary::skip_bytes(bytes, expected_trie_length - trie_length)?;

        // read exceptions[]
        let count = indexes[IX_EXC_LENGTH];
        let exceptions = if count > 0 { ICUBinary::get_string(bytes, count, 0)? } else { Vec::new() };

        // read unfold[]
        let count = indexes[IX_UNFOLD_LENGTH];
        let unfold = if count > 0 { Some(ICUBinary::get_chars(bytes, count, 0)?) } else { None };
        Ok(Self { indexes, exceptions, unfold, trie })
    }

    /// `INSTANCE`.
    ///
    /// # Panics
    ///
    /// When the data cannot be read (Java's `ExceptionInInitializerError`).
    pub fn instance() -> &'static UCaseProps {
        &INSTANCE
    }

    /// `getTrie()` (package-private).
    pub(crate) fn get_trie() -> &'static Trie2_16 {
        &INSTANCE.trie
    }

    // set of property starts for UnicodeSet ------------------------------- ***

    /// `addPropertyStarts(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_property_starts(&self, set: &mut UnicodeSet) -> Result<()> {
        /* add the start code point of each same-value range of the trie */
        let mut trie_iterator = self.trie.iterator();
        while trie_iterator.has_next() {
            let range = trie_iterator.next().expect("has_next");
            if range.lead_surrogate {
                break;
            }
            set.add(range.start_code_point)?;
        }

        /* add code points with hardcoded properties, plus the ones following them */

        /* (none right now, see comment below) */

        /*
         * Omit code points with hardcoded specialcasing properties
         * because we do not build property UnicodeSets for them right now.
         */
        Ok(())
    }

    // data access primitives ---------------------------------------------- ***

    fn exc(&self, i: i32) -> i32 {
        self.exceptions[i as usize] as i32
    }

    /// `propsHasException(int props)` (package-private).
    pub(crate) fn props_has_exception(props: i32) -> bool {
        (props & EXCEPTION) != 0
    }

    /// Get the value of an optional-value slot where hasSlot(excWord,
    /// index), together with the offset of its last unit (bits 63..32).
    fn get_slot_value_and_offset(&self, exc_word: i32, index: i32, mut exc_offset: i32) -> i64 {
        let mut value: i64;
        if (exc_word & EXC_DOUBLE_SLOTS) == 0 {
            exc_offset += slot_offset(exc_word, index);
            value = self.exc(exc_offset) as i64;
        } else {
            exc_offset += 2 * slot_offset(exc_word, index);
            value = self.exc(exc_offset) as i64;
            exc_offset += 1;
            value = (value << 16) | self.exc(exc_offset) as i64;
        }
        value | ((exc_offset as i64) << 32)
    }

    /* same as getSlotValueAndOffset() but does not return the slot offset */
    fn get_slot_value(&self, exc_word: i32, index: i32, mut exc_offset: i32) -> i32 {
        let mut value: i32;
        if (exc_word & EXC_DOUBLE_SLOTS) == 0 {
            exc_offset += slot_offset(exc_word, index);
            value = self.exc(exc_offset);
        } else {
            exc_offset += 2 * slot_offset(exc_word, index);
            value = self.exc(exc_offset);
            exc_offset += 1;
            value = (value << 16) | self.exc(exc_offset);
        }
        value
    }

    // simple case mappings ------------------------------------------------ ***

    /// `tolower(int c)`.
    pub fn tolower(&self, mut c: i32) -> i32 {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::is_upper_or_title_from_props(props) {
                c += Self::get_delta(props);
            }
        } else {
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;
            if has_slot(exc_word, EXC_DELTA) && Self::is_upper_or_title_from_props(props) {
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset);
                return if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
            }
            if has_slot(exc_word, EXC_LOWER) {
                c = self.get_slot_value(exc_word, EXC_LOWER, exc_offset);
            }
        }
        c
    }

    /// `toupper(int c)`.
    pub fn toupper(&self, mut c: i32) -> i32 {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::get_type_from_props(props) == Self::LOWER {
                c += Self::get_delta(props);
            }
        } else {
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;
            if has_slot(exc_word, EXC_DELTA) && Self::get_type_from_props(props) == Self::LOWER {
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset);
                return if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
            }
            if has_slot(exc_word, EXC_UPPER) {
                c = self.get_slot_value(exc_word, EXC_UPPER, exc_offset);
            }
        }
        c
    }

    /// `totitle(int c)`.
    pub fn totitle(&self, mut c: i32) -> i32 {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::get_type_from_props(props) == Self::LOWER {
                c += Self::get_delta(props);
            }
        } else {
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;
            if has_slot(exc_word, EXC_DELTA) && Self::get_type_from_props(props) == Self::LOWER {
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset);
                return if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
            }
            let index = if has_slot(exc_word, EXC_TITLE) {
                EXC_TITLE
            } else if has_slot(exc_word, EXC_UPPER) {
                EXC_UPPER
            } else {
                return c;
            };
            c = self.get_slot_value(exc_word, index, exc_offset);
        }
        c
    }

    /// `addCaseClosure(int c, UnicodeSet set)`: adds all simple case mappings
    /// and the full case folding of `c` to `set`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_case_closure(&self, c: i32, set: &mut UnicodeSet) -> Result<()> {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::get_type_from_props(props) != Self::NONE {
                /* add the one simple case mapping, no matter what type it is */
                let delta = Self::get_delta(props);
                if delta != 0 {
                    set.add(c + delta)?;
                }
            }
        } else {
            /*
             * c has exceptions, so there may be multiple simple and/or
             * full case mappings. Add them all.
             */
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;
            let exc_offset0 = exc_offset;

            // Hardcode the case closure of i and its relatives and ignore the
            // data file data for these characters.
            // The Turkic dotless i and dotted I with their case mapping conditions
            // and case folding option make the related characters behave specially.
            // This code matches their closure behavior to their case folding behavior.
            if (exc_word & EXC_CONDITIONAL_FOLD) != 0 {
                // Only the default (not Turkic) mappings are included.
                if c == 0x49 {
                    // regular i and I are in one equivalence class
                    set.add(0x69)?;
                    return Ok(());
                } else if c == 0x130 {
                    // Dotted I is in a class with <0069 0307> (for canonical equivalence with <0049 0307>)
                    set.add_str(I_DOT)?;
                    return Ok(());
                }
            } else if c == 0x69 {
                set.add(0x49)?;
                return Ok(());
            } else if c == 0x131 {
                // dotless i is in a class by itself
                return Ok(());
            }

            /* add all simple case mappings */
            for index in EXC_LOWER..=EXC_TITLE {
                if has_slot(exc_word, index) {
                    exc_offset = exc_offset0;
                    let mapping = self.get_slot_value(exc_word, index, exc_offset);
                    set.add(mapping)?;
                }
            }
            if has_slot(exc_word, EXC_DELTA) {
                exc_offset = exc_offset0;
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset);
                set.add(if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta })?;
            }

            /* get the closure string pointer & length */
            let mut closure_offset;
            let closure_length;
            if has_slot(exc_word, EXC_CLOSURE) {
                exc_offset = exc_offset0;
                let value = self.get_slot_value_and_offset(exc_word, EXC_CLOSURE, exc_offset);
                closure_length = (value as i32) & CLOSURE_MAX_LENGTH; /* higher bits are reserved */
                closure_offset = (value >> 32) as i32 + 1; /* behind this slot, unless there are full case mappings */
            } else {
                closure_length = 0;
                closure_offset = 0;
            }

            /* add the full case folding */
            if has_slot(exc_word, EXC_FULL_MAPPINGS) {
                exc_offset = exc_offset0;
                let value = self.get_slot_value_and_offset(exc_word, EXC_FULL_MAPPINGS, exc_offset);
                let mut full_length = value as i32;

                /* start of full case mapping strings */
                exc_offset = (value >> 32) as i32 + 1;

                full_length &= 0xffff; /* bits 16 and higher are reserved */

                /* skip the lowercase result string */
                exc_offset += full_length & FULL_LOWER;
                full_length >>= 4;

                /* add the full case folding string */
                let length = full_length & 0xf;
                if length != 0 {
                    set.add_str(&self.exceptions[exc_offset as usize..(exc_offset + length) as usize])?;
                    exc_offset += length;
                }

                /* skip the uppercase and titlecase strings */
                full_length >>= 4;
                exc_offset += full_length & 0xf;
                full_length >>= 4;
                exc_offset += full_length;

                closure_offset = exc_offset; /* behind full case mappings */
            }

            /* add each code point in the closure string */
            let limit = closure_offset + closure_length;
            let mut index = closure_offset;
            while index < limit {
                let mapping = code_point_at(&self.exceptions, index);
                set.add(mapping)?;
                index += UTF16::get_char_count(c);
            }
        }
        Ok(())
    }

    /// `addSimpleCaseClosure(int c, UnicodeSet set)`: adds all simple case
    /// mappings and the simple case folding of `c` to `set`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_simple_case_closure(&self, c: i32, set: &mut UnicodeSet) -> Result<()> {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::get_type_from_props(props) != Self::NONE {
                /* add the one simple case mapping, no matter what type it is */
                let delta = Self::get_delta(props);
                if delta != 0 {
                    set.add(c + delta)?;
                }
            }
        } else {
            // c has exceptions. Add the mappings relevant for scf=Simple_Case_Folding.
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;
            let exc_offset0 = exc_offset;

            // Hardcode the case closure of i and its relatives and ignore the
            // data file data for these characters, like in ucase_addCaseClosure().
            if (exc_word & EXC_CONDITIONAL_FOLD) != 0 {
                // These two are folded to each other, and don't fold to anything else.
                if c == 0x49 {
                    // regular i and I are in one equivalence class
                    set.add(0x69)?;
                    return Ok(());
                } else if c == 0x130 {
                    // dotted I is in a class by itself
                    return Ok(());
                }
            } else if c == 0x69 {
                set.add(0x49)?;
                return Ok(());
            } else if c == 0x131 {
                // dotless i is in a class by itself
                return Ok(());
            }

            // Add all simple case mappings.
            for index in EXC_LOWER..=EXC_TITLE {
                if has_slot(exc_word, index) {
                    exc_offset = exc_offset0;
                    let mapping = self.get_slot_value(exc_word, index, exc_offset);
                    set.add(mapping)?;
                }
            }
            if has_slot(exc_word, EXC_DELTA) {
                exc_offset = exc_offset0;
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset);
                let mapping = if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
                set.add(mapping)?;
            }

            // Get the closure string pointer & length.
            let mut closure_offset;
            let closure_length;
            if has_slot(exc_word, EXC_CLOSURE) {
                exc_offset = exc_offset0;
                let value = self.get_slot_value_and_offset(exc_word, EXC_CLOSURE, exc_offset);
                closure_length = (value as i32) & CLOSURE_MAX_LENGTH; /* higher bits are reserved */
                closure_offset = (value >> 32) as i32 + 1; /* behind this slot, unless there are full case mappings */
            } else {
                closure_length = 0;
                closure_offset = 0;
            }

            // Skip the full case mappings.
            if closure_length > 0 && has_slot(exc_word, EXC_FULL_MAPPINGS) {
                exc_offset = exc_offset0;
                let value = self.get_slot_value_and_offset(exc_word, EXC_FULL_MAPPINGS, exc_offset);
                let mut full_length = value as i32;

                /* start of full case mapping strings */
                exc_offset = (value >> 32) as i32 + 1;

                full_length &= 0xffff; /* bits 16 and higher are reserved */

                // Skip all 4 full case mappings.
                exc_offset += full_length & FULL_LOWER;
                full_length >>= 4;
                exc_offset += full_length & 0xf;
                full_length >>= 4;
                exc_offset += full_length & 0xf;
                full_length >>= 4;
                exc_offset += full_length;

                closure_offset = exc_offset; /* behind full case mappings */
            }

            // Add each code point in the closure string whose scf maps back to c.
            let limit = closure_offset + closure_length;
            let mut index = closure_offset;
            while index < limit {
                let mapping = code_point_at(&self.exceptions, index);
                set.add(mapping)?;
                index += UTF16::get_char_count(c);
            }
        }
        Ok(())
    }

    /*
     * compare s, which has a length, with t=unfold[unfoldOffset..], which has a maximum length or is NUL-terminated
     * must be s.length()>0 and max>0 and s.length()<=max
     */
    fn strcmp_max(&self, s: &[u16], mut unfold_offset: usize, mut max: i32) -> i32 {
        let unfold = self.unfold.as_ref().expect("unfold data present");
        let mut length = s.len() as i32;
        max -= length; /* we require length<=max, so no need to decrement max in the loop */
        let mut i1 = 0usize;
        loop {
            let mut c1 = s[i1] as i32;
            i1 += 1;
            let c2 = unfold[unfold_offset] as i32;
            unfold_offset += 1;
            if c2 == 0 {
                return 1; /* reached the end of t but not of s */
            }
            c1 -= c2;
            if c1 != 0 {
                return c1; /* return difference result */
            }
            length -= 1;
            if length <= 0 {
                break;
            }
        }
        /* ends with length==0 */

        if max == 0 || unfold[unfold_offset] == 0 {
            0 /* equal to length of both strings */
        } else {
            -max /* return length difference */
        }
    }

    /// `addStringCaseClosure(String s, UnicodeSet set)`: maps the string
    /// to single code points and adds the associated case closure mappings;
    /// false when the string does not fold to code points.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_string_case_closure(&self, s: &[u16], set: &mut UnicodeSet) -> Result<bool> {
        let Some(unfold) = &self.unfold else { return Ok(false) };

        let length = s.len() as i32;
        if length <= 1 {
            /* the string is too short to find any match */
            /*
             * more precise would be:
             * if(!u_strHasMoreChar32Than(s, length, 1))
             * but this does not make much practical difference because
             * a single supplementary code point would just not be found
             */
            return Ok(false);
        }

        let unfold_rows = unfold[UNFOLD_ROWS] as i32;
        let unfold_row_width = unfold[UNFOLD_ROW_WIDTH] as i32;
        let unfold_string_width = unfold[UNFOLD_STRING_WIDTH] as i32;
        //unfoldCPWidth=unfoldRowWidth-unfoldStringWidth;

        if length > unfold_string_width {
            /* the string is too long to find any match */
            return Ok(false);
        }

        /* do a binary search for the string */
        let mut start = 0;
        let mut limit = unfold_rows;
        while start < limit {
            let i = (start + limit) / 2;
            let unfold_offset = (i + 1) * unfold_row_width; // +1 to skip the header values above
            let result = self.strcmp_max(s, unfold_offset as usize, unfold_string_width);

            if result == 0 {
                /* found the string: add each code point, and its case closure */
                let mut i = unfold_string_width;
                while i < unfold_row_width && unfold[(unfold_offset + i) as usize] != 0 {
                    let c = UTF16::char_at_chars(unfold, unfold_offset, unfold.len() as i32, i);
                    set.add(c)?;
                    self.add_case_closure(c, set)?;
                    i += UTF16::get_char_count(c);
                }
                return Ok(true);
            } else if result < 0 {
                limit = i;
            } else {
                /* result>0 */
                start = i + 1;
            }
        }

        Ok(false) /* string not found */
    }

    /// `getType(int c)`: `NONE`, `LOWER`, `UPPER` or `TITLE`.
    pub fn get_type(&self, c: i32) -> i32 {
        Self::get_type_from_props(self.trie.get(c))
    }

    /// `getTypeOrIgnorable(int c)`: like getType() but also sets
    /// `IGNORABLE` if c is case-ignorable.
    pub fn get_type_or_ignorable(&self, c: i32) -> i32 {
        Self::get_type_and_ignorable_from_props(self.trie.get(c))
    }

    /// `getDotType(int c)`: `NO_DOT`, `SOFT_DOTTED`, ...
    pub fn get_dot_type(&self, c: i32) -> i32 {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            props & DOT_MASK
        } else {
            (self.exc(get_exceptions_offset(props)) >> EXC_DOT_SHIFT) & DOT_MASK
        }
    }

    /// `isSoftDotted(int c)`.
    pub fn is_soft_dotted(&self, c: i32) -> bool {
        self.get_dot_type(c) == SOFT_DOTTED
    }

    /// `isCaseSensitive(int c)`.
    pub fn is_case_sensitive(&self, c: i32) -> bool {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            (props & SENSITIVE) != 0
        } else {
            (self.exc(get_exceptions_offset(props)) & EXC_SENSITIVE) != 0
        }
    }

    // string casing ------------------------------------------------------- ***

    /// `getCaseLocale(Locale locale)`.
    pub fn get_case_locale_jdk(locale: &Locale) -> i32 {
        Self::get_case_locale(locale.get_language())
    }

    /// `getCaseLocale(ULocale locale)`.
    pub fn get_case_locale_ulocale(locale: &ULocale) -> i32 {
        Self::get_case_locale(&locale.get_language())
    }

    /// Accepts both 2- and 3-letter language subtags.
    fn get_case_locale(language: &str) -> i32 {
        // Check the subtag length to reduce the number of comparisons
        // for locales without special behavior.
        // Fastpath for English "en" which is often used for default (=root locale) case mappings,
        // and for Chinese "zh": Very common but no special case mapping behavior.
        let b = language.as_bytes();
        if crate::r#impl::locale::ascii_util::len16(language) == 2 {
            if language == "en" || b[0] > b't' {
                return Self::LOC_ROOT;
            } else if language == "tr" || language == "az" {
                return Self::LOC_TURKISH;
            } else if language == "el" {
                return Self::LOC_GREEK;
            } else if language == "lt" {
                return Self::LOC_LITHUANIAN;
            } else if language == "nl" {
                return Self::LOC_DUTCH;
            } else if language == "hy" {
                return Self::LOC_ARMENIAN;
            }
        } else if crate::r#impl::locale::ascii_util::len16(language) == 3 {
            if language == "tur" || language == "aze" {
                return Self::LOC_TURKISH;
            } else if language == "ell" {
                return Self::LOC_GREEK;
            } else if language == "lit" {
                return Self::LOC_LITHUANIAN;
            } else if language == "nld" {
                return Self::LOC_DUTCH;
            } else if language == "hye" {
                return Self::LOC_ARMENIAN;
            }
        }
        Self::LOC_ROOT
    }

    /* Is followed by {case-ignorable}* cased  ? (dir determines looking forward/backward) */
    fn is_followed_by_cased_letter(&self, iter: Option<&mut dyn ContextIterator>, dir: i32) -> bool {
        let Some(iter) = iter else { return false };

        iter.reset(dir);
        loop {
            let c = iter.next();
            if c < 0 {
                break;
            }
            let ty = self.get_type_or_ignorable(c);
            if (ty & 4) != 0 {
                /* case-ignorable, continue with the loop */
            } else if ty != Self::NONE {
                return true; /* followed by cased letter */
            } else {
                return false; /* uncased and not case-ignorable */
            }
        }

        false /* not followed by cased letter */
    }

    /* Is preceded by Soft_Dotted character with no intervening cc=230 ? */
    fn is_preceded_by_soft_dotted(&self, iter: Option<&mut dyn ContextIterator>) -> bool {
        let Some(iter) = iter else { return false };

        iter.reset(-1);
        loop {
            let c = iter.next();
            if c < 0 {
                break;
            }
            let dot_type = self.get_dot_type(c);
            if dot_type == SOFT_DOTTED {
                return true; /* preceded by TYPE_i */
            } else if dot_type != OTHER_ACCENT {
                return false; /* preceded by different base character (not TYPE_i), or intervening cc==230 */
            }
        }

        false /* not preceded by TYPE_i */
    }

    /*
     * See Jitterbug 2344:
     * The condition After_I for Turkic-lowercasing of U+0307 combining dot above
     * is checked in ICU 2.0, 2.1, 2.6 but was not in 2.2 & 2.4 because
     * we made those releases compatible with Unicode 3.2 which had not fixed
     * a related bug in SpecialCasing.txt.
     *
     * From the Jitterbug 2344 text:
     * ... this bug is listed as a Unicode erratum
     * from 2002-10-31 at http://www.unicode.org/uni2errata/UnicodeErrata.html
     * <quote>
     * There are two errors in SpecialCasing.txt.
     * 1. Missing semicolons on two lines. ... [irrelevant for ICU]
     * 2. An incorrect context definition. Correct as follows:
     * < 0307; ; 0307; 0307; tr After_Soft_Dotted; # COMBINING DOT ABOVE
     * < 0307; ; 0307; 0307; az After_Soft_Dotted; # COMBINING DOT ABOVE
     * ---
     * > 0307; ; 0307; 0307; tr After_I; # COMBINING DOT ABOVE
     * > 0307; ; 0307; 0307; az After_I; # COMBINING DOT ABOVE
     * where the context After_I is defined as:
     * The last preceding base character was an uppercase I, and there is no
     * intervening combining character class 230 (Above).
     * </quote>
     *
     * Note that SpecialCasing.txt even in Unicode 3.2 described the condition as:
     *
     * # When lowercasing, remove dot_above in the sequence I + dot_above, which will turn into i.
     * # This matches the behavior of the canonically equivalent I-dot_above
     *
     * See also the description in this place in older versions of uchar.c (revision 1.100).
     *
     * Markus W. Scherer 2003-feb-15
     */

    /* Is preceded by base character 'I' with no intervening cc=230 ? */
    fn is_preceded_by_i(&self, iter: Option<&mut dyn ContextIterator>) -> bool {
        let Some(iter) = iter else { return false };

        iter.reset(-1);
        loop {
            let c = iter.next();
            if c < 0 {
                break;
            }
            if c == 0x49 {
                return true; /* preceded by I */
            }
            let dot_type = self.get_dot_type(c);
            if dot_type != OTHER_ACCENT {
                return false; /* preceded by different base character (not I), or intervening cc==230 */
            }
        }

        false /* not preceded by I */
    }

    /* Is followed by one or more cc==230 ? */
    fn is_followed_by_more_above(&self, iter: Option<&mut dyn ContextIterator>) -> bool {
        let Some(iter) = iter else { return false };

        iter.reset(1);
        loop {
            let c = iter.next();
            if c < 0 {
                break;
            }
            let dot_type = self.get_dot_type(c);
            if dot_type == ABOVE {
                return true; /* at least one cc==230 following */
            } else if dot_type != OTHER_ACCENT {
                return false; /* next base character, no more cc==230 following */
            }
        }

        false /* no more cc==230 following */
    }

    /* Is followed by a dot above (without cc==230 in between) ? */
    fn is_followed_by_dot_above(&self, iter: Option<&mut dyn ContextIterator>) -> bool {
        let Some(iter) = iter else { return false };

        iter.reset(1);
        loop {
            let c = iter.next();
            if c < 0 {
                break;
            }
            if c == 0x307 {
                return true;
            }
            let dot_type = self.get_dot_type(c);
            if dot_type != OTHER_ACCENT {
                return false; /* next base character or cc==230 in between */
            }
        }

        false /* no dot above following */
    }

    /// `toFullLower(int c, ContextIterator iter, Appendable out, int
    /// caseLocale)`: the full lowercase mapping of `c`. Returns the code
    /// point (or `~c` for an unchanged one), or the length of the string it
    /// appended to `out` (at most `MAX_STRING_LENGTH`).
    pub fn to_full_lower(&self, c: i32, mut iter: Option<&mut dyn ContextIterator>, out: &mut Vec<u16>, case_locale: i32) -> i32 {
        let mut result = c;
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::is_upper_or_title_from_props(props) {
                result = c + Self::get_delta(props);
            }
        } else {
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;

            let exc_offset2 = exc_offset;

            if (exc_word & EXC_CONDITIONAL_SPECIAL) != 0 {
                /* use hardcoded conditions and mappings */
                /*
                 * Test for conditional mappings first
                 *   (otherwise the unconditional default mappings are always taken),
                 * then test for characters that have unconditional mappings in SpecialCasing.txt,
                 * then get the UnicodeData.txt mappings.
                 */
                if case_locale == Self::LOC_LITHUANIAN &&
                        /* base characters, find accents above */
                        (((c == 0x49 || c == 0x4a || c == 0x12e) &&
                            self.is_followed_by_more_above(reborrow(&mut iter))) ||
                        /* precomposed with accent above, no need to find one */
                        (c == 0xcc || c == 0xcd || c == 0x128))
                {
                    /*
                        # Lithuanian

                        # Lithuanian retains the dot in a lowercase i when followed by accents.

                        # Introduce an explicit dot above when lowercasing capital I's and J's
                        # whenever there are more accents above.
                        # (of the accents used in Lithuanian: grave, acute, tilde above, and ogonek)

                        0049; 0069 0307; 0049; 0049; lt More_Above; # LATIN CAPITAL LETTER I
                        004A; 006A 0307; 004A; 004A; lt More_Above; # LATIN CAPITAL LETTER J
                        012E; 012F 0307; 012E; 012E; lt More_Above; # LATIN CAPITAL LETTER I WITH OGONEK
                        00CC; 0069 0307 0300; 00CC; 00CC; lt; # LATIN CAPITAL LETTER I WITH GRAVE
                        00CD; 0069 0307 0301; 00CD; 00CD; lt; # LATIN CAPITAL LETTER I WITH ACUTE
                        0128; 0069 0307 0303; 0128; 0128; lt; # LATIN CAPITAL LETTER I WITH TILDE
                     */
                    return match c {
                        0x49 => {
                            /* LATIN CAPITAL LETTER I */
                            out.extend_from_slice(I_DOT);
                            2
                        }
                        0x4a => {
                            /* LATIN CAPITAL LETTER J */
                            out.extend_from_slice(J_DOT);
                            2
                        }
                        0x12e => {
                            /* LATIN CAPITAL LETTER I WITH OGONEK */
                            out.extend_from_slice(I_OGONEK_DOT);
                            2
                        }
                        0xcc => {
                            /* LATIN CAPITAL LETTER I WITH GRAVE */
                            out.extend_from_slice(I_DOT_GRAVE);
                            3
                        }
                        0xcd => {
                            /* LATIN CAPITAL LETTER I WITH ACUTE */
                            out.extend_from_slice(I_DOT_ACUTE);
                            3
                        }
                        0x128 => {
                            /* LATIN CAPITAL LETTER I WITH TILDE */
                            out.extend_from_slice(I_DOT_TILDE);
                            3
                        }
                        _ => 0, /* will not occur */
                    };
                /* # Turkish and Azeri */
                } else if case_locale == Self::LOC_TURKISH && c == 0x130 {
                    /*
                        # I and i-dotless; I-dot and i are case pairs in Turkish and Azeri
                        # The following rules handle those cases.

                        0130; 0069; 0130; 0130; tr # LATIN CAPITAL LETTER I WITH DOT ABOVE
                        0130; 0069; 0130; 0130; az # LATIN CAPITAL LETTER I WITH DOT ABOVE
                     */
                    return 0x69;
                } else if case_locale == Self::LOC_TURKISH && c == 0x307 && self.is_preceded_by_i(reborrow(&mut iter)) {
                    /*
                        # When lowercasing, remove dot_above in the sequence I + dot_above, which will turn into i.
                        # This matches the behavior of the canonically equivalent I-dot_above

                        0307; ; 0307; 0307; tr After_I; # COMBINING DOT ABOVE
                        0307; ; 0307; 0307; az After_I; # COMBINING DOT ABOVE
                     */
                    return 0; /* remove the dot (continue without output) */
                } else if case_locale == Self::LOC_TURKISH && c == 0x49 && !self.is_followed_by_dot_above(reborrow(&mut iter)) {
                    /*
                        # When lowercasing, unless an I is before a dot_above, it turns into a dotless i.

                        0049; 0131; 0049; 0049; tr Not_Before_Dot; # LATIN CAPITAL LETTER I
                        0049; 0131; 0049; 0049; az Not_Before_Dot; # LATIN CAPITAL LETTER I
                     */
                    return 0x131;
                } else if c == 0x130 {
                    /*
                        # Preserve canonical equivalence for I with dot. Turkic is handled below.

                        0130; 0069 0307; 0130; 0130; # LATIN CAPITAL LETTER I WITH DOT ABOVE
                     */
                    out.extend_from_slice(I_DOT);
                    return 2;
                } else if c == 0x3a3
                    && !self.is_followed_by_cased_letter(reborrow(&mut iter), 1)
                    && self.is_followed_by_cased_letter(reborrow(&mut iter), -1) /* -1=preceded */
                {
                    /* greek capital sigma maps depending on surrounding cased letters (see SpecialCasing.txt) */
                    /*
                        # Special case for final form of sigma

                        03A3; 03C2; 03A3; 03A3; Final_Sigma; # GREEK CAPITAL LETTER SIGMA
                     */
                    return 0x3c2; /* greek small final sigma */
                } else {
                    /* no known conditional special case mapping, use a normal mapping */
                }
            } else if has_slot(exc_word, EXC_FULL_MAPPINGS) {
                let value = self.get_slot_value_and_offset(exc_word, EXC_FULL_MAPPINGS, exc_offset);
                let full = (value as i32) & FULL_LOWER;
                if full != 0 {
                    /* start of full case mapping strings */
                    exc_offset = (value >> 32) as i32 + 1;

                    // append the lowercase mapping
                    out.extend_from_slice(&self.exceptions[exc_offset as usize..(exc_offset + full) as usize]);

                    /* return the string length */
                    return full;
                }
            }

            if has_slot(exc_word, EXC_DELTA) && Self::is_upper_or_title_from_props(props) {
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset2);
                return if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
            }
            if has_slot(exc_word, EXC_LOWER) {
                result = self.get_slot_value(exc_word, EXC_LOWER, exc_offset2);
            }
        }

        if result == c {
            !result
        } else {
            result
        }
    }

    /* internal */
    fn to_upper_or_title(&self, c: i32, iter: Option<&mut dyn ContextIterator>, out: &mut Vec<u16>, loc: i32, upper_not_title: bool) -> i32 {
        let mut result = c;
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::get_type_from_props(props) == Self::LOWER {
                result = c + Self::get_delta(props);
            }
        } else {
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;

            let exc_offset2 = exc_offset;

            if (exc_word & EXC_CONDITIONAL_SPECIAL) != 0 {
                /* use hardcoded conditions and mappings */
                if loc == Self::LOC_TURKISH && c == 0x69 {
                    /*
                        # Turkish and Azeri

                        # I and i-dotless; I-dot and i are case pairs in Turkish and Azeri
                        # The following rules handle those cases.

                        # When uppercasing, i turns into a dotted capital I

                        0069; 0069; 0130; 0130; tr; # LATIN SMALL LETTER I
                        0069; 0069; 0130; 0130; az; # LATIN SMALL LETTER I
                     */
                    return 0x130;
                } else if loc == Self::LOC_LITHUANIAN && c == 0x307 && self.is_preceded_by_soft_dotted(iter) {
                    /*
                        # Lithuanian

                        # Lithuanian retains the dot in a lowercase i when followed by accents.

                        # Remove DOT ABOVE after "i" with upper or titlecase

                        0307; 0307; ; ; lt After_Soft_Dotted; # COMBINING DOT ABOVE
                     */
                    return 0; /* remove the dot (continue without output) */
                } else if c == 0x0587 {
                    // See ICU-13416:
                    // և ligature ech-yiwn
                    // uppercases to ԵՒ=ech+yiwn by default and in Western Armenian,
                    // but to ԵՎ=ech+vew in Eastern Armenian.
                    if loc == Self::LOC_ARMENIAN {
                        out.extend_from_slice(if upper_not_title { &[0x535, 0x54e] } else { &[0x535, 0x57e] });
                    } else {
                        out.extend_from_slice(if upper_not_title { &[0x535, 0x552] } else { &[0x535, 0x582] });
                    }
                    return 2;
                } else {
                    /* no known conditional special case mapping, use a normal mapping */
                }
            } else if has_slot(exc_word, EXC_FULL_MAPPINGS) {
                let value = self.get_slot_value_and_offset(exc_word, EXC_FULL_MAPPINGS, exc_offset);
                let mut full = (value as i32) & 0xffff;

                /* start of full case mapping strings */
                exc_offset = (value >> 32) as i32 + 1;

                /* skip the lowercase and case-folding result strings */
                exc_offset += full & FULL_LOWER;
                full >>= 4;
                exc_offset += full & 0xf;
                full >>= 4;

                if upper_not_title {
                    full &= 0xf;
                } else {
                    /* skip the uppercase result string */
                    exc_offset += full & 0xf;
                    full = (full >> 4) & 0xf;
                }

                if full != 0 {
                    // append the result string
                    out.extend_from_slice(&self.exceptions[exc_offset as usize..(exc_offset + full) as usize]);

                    /* return the string length */
                    return full;
                }
            }

            if has_slot(exc_word, EXC_DELTA) && Self::get_type_from_props(props) == Self::LOWER {
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset2);
                return if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
            }
            let index = if !upper_not_title && has_slot(exc_word, EXC_TITLE) {
                EXC_TITLE
            } else if has_slot(exc_word, EXC_UPPER) {
                /* here, titlecase is same as uppercase */
                EXC_UPPER
            } else {
                return !c;
            };
            result = self.get_slot_value(exc_word, index, exc_offset2);
        }

        if result == c {
            !result
        } else {
            result
        }
    }

    /// `toFullUpper(int c, ContextIterator iter, Appendable out, int
    /// caseLocale)`: as [`to_full_lower`](Self::to_full_lower), uppercase.
    pub fn to_full_upper(&self, c: i32, iter: Option<&mut dyn ContextIterator>, out: &mut Vec<u16>, case_locale: i32) -> i32 {
        self.to_upper_or_title(c, iter, out, case_locale, true)
    }

    /// `toFullTitle(int c, ContextIterator iter, Appendable out, int
    /// caseLocale)`: as [`to_full_lower`](Self::to_full_lower), titlecase.
    pub fn to_full_title(&self, c: i32, iter: Option<&mut dyn ContextIterator>, out: &mut Vec<u16>, case_locale: i32) -> i32 {
        self.to_upper_or_title(c, iter, out, case_locale, false)
    }

    /* case folding ------------------------------------------------------------- */

    /*
     * Case folding is similar to lowercasing.
     * The result may be a simple mapping, i.e., a single code point, or
     * a full mapping, i.e., a string.
     * If the case folding for a code point is the same as its simple (1:1) lowercase mapping,
     * then only the lowercase mapping is stored.
     *
     * Some special cases are hardcoded because their conditions cannot be
     * parsed and processed from CaseFolding.txt.
     *
     * Unicode 3.2 CaseFolding.txt specifies for its status field:

    # C: common case folding, common mappings shared by both simple and full mappings.
    # F: full case folding, mappings that cause strings to grow in length. Multiple characters are separated by spaces.
    # S: simple case folding, mappings to single characters where different from F.
    # T: special case for uppercase I and dotted uppercase I
    #    - For non-Turkic languages, this mapping is normally not used.
    #    - For Turkic languages (tr, az), this mapping can be used instead of the normal mapping for these characters.
    #
    # Usage:
    #  A. To do a simple case folding, use the mappings with status C + S.
    #  B. To do a full case folding, use the mappings with status C + F.
    #
    #    The mappings with status T can be used or omitted depending on the desired case-folding
    #    behavior. (The default option is to exclude them.)

     * Unicode 3.2 has 'T' mappings as follows:

    0049; T; 0131; # LATIN CAPITAL LETTER I
    0130; T; 0069; # LATIN CAPITAL LETTER I WITH DOT ABOVE

     * while the default mappings for these code points are:

    0049; C; 0069; # LATIN CAPITAL LETTER I
    0130; F; 0069 0307; # LATIN CAPITAL LETTER I WITH DOT ABOVE

     * U+0130 has no simple case folding (simple-case-folds to itself).
     */

    /// `fold(int c, int options)`: the simple case folding mapping of `c`.
    pub fn fold(&self, mut c: i32, options: i32) -> i32 {
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::is_upper_or_title_from_props(props) {
                c += Self::get_delta(props);
            }
        } else {
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;
            if (exc_word & EXC_CONDITIONAL_FOLD) != 0 {
                /* special case folding mappings, hardcoded */
                if (options & Self::FOLD_CASE_OPTIONS_MASK) == UCharacter::FOLD_CASE_DEFAULT {
                    /* default mappings */
                    if c == 0x49 {
                        /* 0049; C; 0069; # LATIN CAPITAL LETTER I */
                        return 0x69;
                    } else if c == 0x130 {
                        /* no simple case folding for U+0130 */
                        return c;
                    }
                } else {
                    /* Turkic mappings */
                    if c == 0x49 {
                        /* 0049; T; 0131; # LATIN CAPITAL LETTER I */
                        return 0x131;
                    } else if c == 0x130 {
                        /* 0130; T; 0069; # LATIN CAPITAL LETTER I WITH DOT ABOVE */
                        return 0x69;
                    }
                }
            }
            if (exc_word & EXC_NO_SIMPLE_CASE_FOLDING) != 0 {
                return c;
            }
            if has_slot(exc_word, EXC_DELTA) && Self::is_upper_or_title_from_props(props) {
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset);
                return if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
            }
            let index = if has_slot(exc_word, EXC_FOLD) {
                EXC_FOLD
            } else if has_slot(exc_word, EXC_LOWER) {
                EXC_LOWER
            } else {
                return c;
            };
            c = self.get_slot_value(exc_word, index, exc_offset);
        }
        c
    }

    /*
     * Issue for canonical caseless match (UAX #21):
     * Turkic casefolding (using "T" mappings in CaseFolding.txt) does not preserve
     * canonical equivalence, unlike default-option casefolding.
     * For example, I-grave and I + grave fold to strings that are not canonically
     * equivalent.
     * For more details, see the comment in unorm_compare() in unorm.cpp
     * and the intermediate prototype changes for Jitterbug 2021.
     * (For example, revision 1.104 of uchar.c and 1.4 of CaseFolding.txt.)
     *
     * This did not get fixed because it appears that it is not possible to fix
     * it for uppercase and lowercase characters (I-grave vs. i-grave)
     * together in a way that they still fold to common result strings.
     */

    /// `toFullFolding(int c, Appendable out, int options)`: the full case
    /// folding of `c`, with the return convention of
    /// [`to_full_lower`](Self::to_full_lower).
    pub fn to_full_folding(&self, c: i32, out: &mut Vec<u16>, options: i32) -> i32 {
        let mut result = c;
        let props = self.trie.get(c);
        if !Self::props_has_exception(props) {
            if Self::is_upper_or_title_from_props(props) {
                result = c + Self::get_delta(props);
            }
        } else {
            let mut exc_offset = get_exceptions_offset(props);
            let exc_word = self.exc(exc_offset);
            exc_offset += 1;

            let exc_offset2 = exc_offset;

            if (exc_word & EXC_CONDITIONAL_FOLD) != 0 {
                /* use hardcoded conditions and mappings */
                if (options & Self::FOLD_CASE_OPTIONS_MASK) == UCharacter::FOLD_CASE_DEFAULT {
                    /* default mappings */
                    if c == 0x49 {
                        /* 0049; C; 0069; # LATIN CAPITAL LETTER I */
                        return 0x69;
                    } else if c == 0x130 {
                        /* 0130; F; 0069 0307; # LATIN CAPITAL LETTER I WITH DOT ABOVE */
                        out.extend_from_slice(I_DOT);
                        return 2;
                    }
                } else {
                    /* Turkic mappings */
                    if c == 0x49 {
                        /* 0049; T; 0131; # LATIN CAPITAL LETTER I */
                        return 0x131;
                    } else if c == 0x130 {
                        /* 0130; T; 0069; # LATIN CAPITAL LETTER I WITH DOT ABOVE */
                        return 0x69;
                    }
                }
            } else if has_slot(exc_word, EXC_FULL_MAPPINGS) {
                let value = self.get_slot_value_and_offset(exc_word, EXC_FULL_MAPPINGS, exc_offset);
                let mut full = (value as i32) & 0xffff;

                /* start of full case mapping strings */
                exc_offset = (value >> 32) as i32 + 1;

                /* skip the lowercase result string */
                exc_offset += full & FULL_LOWER;
                full = (full >> 4) & 0xf;

                if full != 0 {
                    // append the result string
                    out.extend_from_slice(&self.exceptions[exc_offset as usize..(exc_offset + full) as usize]);

                    /* return the string length */
                    return full;
                }
            }

            if (exc_word & EXC_NO_SIMPLE_CASE_FOLDING) != 0 {
                return !c;
            }
            if has_slot(exc_word, EXC_DELTA) && Self::is_upper_or_title_from_props(props) {
                let delta = self.get_slot_value(exc_word, EXC_DELTA, exc_offset2);
                return if (exc_word & EXC_DELTA_IS_NEGATIVE) == 0 { c + delta } else { c - delta };
            }
            let index = if has_slot(exc_word, EXC_FOLD) {
                EXC_FOLD
            } else if has_slot(exc_word, EXC_LOWER) {
                EXC_LOWER
            } else {
                return !c;
            };
            result = self.get_slot_value(exc_word, index, exc_offset2);
        }

        if result == c {
            !result
        } else {
            result
        }
    }

    /// `hasBinaryProperty(int c, int which)`: the case-related binary
    /// properties.
    pub fn has_binary_property(&self, c: i32, which: i32) -> bool {
        // Java writes the discarded results into the shared dummyStringBuilder.
        let mut dummy: Vec<u16> = Vec::new();
        match which {
            UProperty::LOWERCASE => Self::LOWER == self.get_type(c),
            UProperty::UPPERCASE => Self::UPPER == self.get_type(c),
            UProperty::SOFT_DOTTED => self.is_soft_dotted(c),
            UProperty::CASE_SENSITIVE => self.is_case_sensitive(c),
            UProperty::CASED => Self::NONE != self.get_type(c),
            UProperty::CASE_IGNORABLE => (self.get_type_or_ignorable(c) >> 2) != 0,
            /*
             * Note: The following Changes_When_Xyz are defined as testing whether
             * the NFD form of the input changes when Xyz-case-mapped.
             * However, this simpler implementation of these properties,
             * ignoring NFD, passes the tests.
             * The implementation needs to be changed if the tests start failing.
             * When that happens, optimizations should be used to work with the
             * per-single-code point ucase_toFullXyz() functions unless
             * the NFD form has more than one code point,
             * and the property starts set needs to be the union of the
             * case and NFC/canonical-iteration starts.
             */
            UProperty::CHANGES_WHEN_LOWERCASED => self.to_full_lower(c, None, &mut dummy, Self::LOC_ROOT) >= 0,
            UProperty::CHANGES_WHEN_UPPERCASED => self.to_full_upper(c, None, &mut dummy, Self::LOC_ROOT) >= 0,
            UProperty::CHANGES_WHEN_TITLECASED => self.to_full_title(c, None, &mut dummy, Self::LOC_ROOT) >= 0,
            /* case UProperty.CHANGES_WHEN_CASEFOLDED: -- in UCharacterProperty.java */
            UProperty::CHANGES_WHEN_CASEMAPPED => {
                self.to_full_lower(c, None, &mut dummy, Self::LOC_ROOT) >= 0
                    || self.to_full_upper(c, None, &mut dummy, Self::LOC_ROOT) >= 0
                    || self.to_full_title(c, None, &mut dummy, Self::LOC_ROOT) >= 0
            }
            _ => false,
        }
    }

    // definitions for 16-bit case properties word ------------------------- ***

    /// `getTypeFromProps(int props)` (package-private).
    pub(crate) fn get_type_from_props(props: i32) -> i32 {
        props & Self::TYPE_MASK
    }

    /* get the same type and ignorable bits, without exception bits */
    fn get_type_and_ignorable_from_props(props: i32) -> i32 {
        props & 7
    }

    /// `isUpperOrTitleFromProps(int props)` (package-private).
    pub(crate) fn is_upper_or_title_from_props(props: i32) -> bool {
        (props & 2) != 0
    }

    /// `getDelta(int props)` (package-private).
    pub(crate) fn get_delta(props: i32) -> i32 {
        (props as i16 as i32) >> DELTA_SHIFT
    }
}
