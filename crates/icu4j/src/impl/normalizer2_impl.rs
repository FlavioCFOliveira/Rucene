// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Normalizer2Impl`.

use std::sync::{Mutex, OnceLock};

use rucene::error::Result;

use crate::jdk::{
    char_count, code_point_at, code_point_before, is_high_surrogate, is_low_surrogate,
    offset_by_code_points, to_code_point, ByteBuffer,
};
use crate::r#impl::icu_binary::ICUBinary;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;
use crate::util::code_point_map::{CodePointMap, Range, RangeOption};
use crate::util::code_point_trie::{CodePointTrie, Fast, Type, ValueWidth};
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;
use crate::util::mutable_code_point_trie::MutableCodePointTrie;
use crate::util::version_info::VersionInfo;

/// A target of `append(char)`: Java's `Appendable` as the normalization
/// code uses it (a `StringBuilder`, or a [`ReorderingBuffer`], which is
/// itself an `Appendable`).
pub trait Appendable {
    /// `append(char c)`.
    fn append_char(&mut self, c: u16);
}

impl Appendable for Vec<u16> {
    fn append_char(&mut self, c: u16) {
        self.push(c);
    }
}

/// Hangul syllable and Jamo constants and algorithmic decomposition.
///
/// Equivalent to `Normalizer2Impl.Hangul`.
pub struct Hangul;

impl Hangul {
    /* Korean Hangul and Jamo constants */
    /// `JAMO_L_BASE`.
    pub const JAMO_L_BASE: i32 = 0x1100; /* "lead" jamo */
    /// `JAMO_L_END`.
    pub const JAMO_L_END: i32 = 0x1112;
    /// `JAMO_V_BASE`.
    pub const JAMO_V_BASE: i32 = 0x1161; /* "vowel" jamo */
    /// `JAMO_V_END`.
    pub const JAMO_V_END: i32 = 0x1175;
    /// `JAMO_T_BASE`.
    pub const JAMO_T_BASE: i32 = 0x11a7; /* "trail" jamo */
    /// `JAMO_T_END`.
    pub const JAMO_T_END: i32 = 0x11c2;

    /// `HANGUL_BASE`.
    pub const HANGUL_BASE: i32 = 0xac00;
    /// `HANGUL_END`.
    pub const HANGUL_END: i32 = 0xd7a3;

    /// `JAMO_L_COUNT`.
    pub const JAMO_L_COUNT: i32 = 19;
    /// `JAMO_V_COUNT`.
    pub const JAMO_V_COUNT: i32 = 21;
    /// `JAMO_T_COUNT`.
    pub const JAMO_T_COUNT: i32 = 28;

    /// `JAMO_L_LIMIT`.
    pub const JAMO_L_LIMIT: i32 = Self::JAMO_L_BASE + Self::JAMO_L_COUNT;
    /// `JAMO_V_LIMIT`.
    pub const JAMO_V_LIMIT: i32 = Self::JAMO_V_BASE + Self::JAMO_V_COUNT;

    /// `JAMO_VT_COUNT`.
    pub const JAMO_VT_COUNT: i32 = Self::JAMO_V_COUNT * Self::JAMO_T_COUNT;

    /// `HANGUL_COUNT`.
    pub const HANGUL_COUNT: i32 = Self::JAMO_L_COUNT * Self::JAMO_V_COUNT * Self::JAMO_T_COUNT;
    /// `HANGUL_LIMIT`.
    pub const HANGUL_LIMIT: i32 = Self::HANGUL_BASE + Self::HANGUL_COUNT;

    /// `isHangul(int c)`.
    pub fn is_hangul(c: i32) -> bool {
        (Self::HANGUL_BASE..Self::HANGUL_LIMIT).contains(&c)
    }
    /// `isHangulLV(int c)`.
    pub fn is_hangul_lv(mut c: i32) -> bool {
        c -= Self::HANGUL_BASE;
        (0..Self::HANGUL_COUNT).contains(&c) && c % Self::JAMO_T_COUNT == 0
    }
    /// `isJamoL(int c)`.
    pub fn is_jamo_l(c: i32) -> bool {
        (Self::JAMO_L_BASE..Self::JAMO_L_LIMIT).contains(&c)
    }
    /// `isJamoV(int c)`.
    pub fn is_jamo_v(c: i32) -> bool {
        (Self::JAMO_V_BASE..Self::JAMO_V_LIMIT).contains(&c)
    }
    /// `isJamoT(int c)`.
    pub fn is_jamo_t(c: i32) -> bool {
        let t = c - Self::JAMO_T_BASE;
        0 < t && t < Self::JAMO_T_COUNT // not JAMO_T_BASE itself
    }
    /// `isJamo(int c)`.
    pub fn is_jamo(c: i32) -> bool {
        (Self::JAMO_L_BASE..=Self::JAMO_T_END).contains(&c)
            && (c <= Self::JAMO_L_END
                || (Self::JAMO_V_BASE..=Self::JAMO_V_END).contains(&c)
                || Self::JAMO_T_BASE < c)
    }

    /// `decompose(int c, Appendable buffer)`: decomposes c, which must be a
    /// Hangul syllable, into buffer and returns the length of the
    /// decomposition (2 or 3).
    pub fn decompose(mut c: i32, buffer: &mut dyn Appendable) -> i32 {
        c -= Self::HANGUL_BASE;
        let c2 = c % Self::JAMO_T_COUNT;
        c /= Self::JAMO_T_COUNT;
        buffer.append_char((Self::JAMO_L_BASE + c / Self::JAMO_V_COUNT) as u16);
        buffer.append_char((Self::JAMO_V_BASE + c % Self::JAMO_V_COUNT) as u16);
        if c2 == 0 {
            2
        } else {
            buffer.append_char((Self::JAMO_T_BASE + c2) as u16);
            3
        }
    }

    /// `getRawDecomposition(int c, Appendable buffer)`: decomposes c, which
    /// must be a Hangul syllable, into buffer. This is the raw, not
    /// recursive, decomposition. Its length is always 2.
    pub fn get_raw_decomposition(mut c: i32, buffer: &mut dyn Appendable) {
        let orig = c;
        c -= Self::HANGUL_BASE;
        let c2 = c % Self::JAMO_T_COUNT;
        if c2 == 0 {
            c /= Self::JAMO_T_COUNT;
            buffer.append_char((Self::JAMO_L_BASE + c / Self::JAMO_V_COUNT) as u16);
            buffer.append_char((Self::JAMO_V_BASE + c % Self::JAMO_V_COUNT) as u16);
        } else {
            buffer.append_char((orig - c2) as u16); // LV syllable
            buffer.append_char((Self::JAMO_T_BASE + c2) as u16);
        }
    }
}

/// Writable buffer that takes care of canonical ordering. Its Appendable
/// methods behave like the C++ implementation's appendZeroCC() methods.
///
/// If dest is a StringBuilder, then the buffer writes directly to it.
/// Otherwise, the buffer maintains a StringBuilder for intermediate text
/// segments until no further changes are necessary and whole segments can
/// be appended. append() methods that take combining-class values always
/// write to the StringBuilder. Other append() methods flush and append to
/// the Appendable.
///
/// Equivalent to `Normalizer2Impl.ReorderingBuffer`. Java `Appendable`s are
/// `&mut Vec<u16>` in this crate (DECISION on #643), so the destination
/// always takes Java's `StringBuilder` branch (`appIsStringBuilder`):
/// the buffer writes directly to it and `flush` only moves the reorder
/// start.
#[derive(Debug)]
pub struct ReorderingBuffer<'a> {
    impl_: &'a Normalizer2Impl,
    str: &'a mut Vec<u16>,
    reorder_start: i32,
    last_cc: i32,
    // private backward iterator
    code_point_start: i32,
    code_point_limit: i32,
}

impl Appendable for ReorderingBuffer<'_> {
    /// `append(char c)`: appends with zero combining class.
    fn append_char(&mut self, c: u16) {
        self.str.push(c);
        self.last_cc = 0;
        self.reorder_start = self.str.len() as i32;
    }
}

impl<'a> ReorderingBuffer<'a> {
    /// `ReorderingBuffer(Normalizer2Impl ni, Appendable dest, int
    /// destCapacity)`.
    pub fn new(ni: &'a Normalizer2Impl, dest: &'a mut Vec<u16>, dest_capacity: i32) -> Self {
        dest.reserve((dest_capacity.max(0) as usize).saturating_sub(dest.len()));
        let mut b = Self {
            impl_: ni,
            str: dest,
            reorder_start: 0,
            last_cc: 0,
            code_point_start: 0,
            code_point_limit: 0,
        };
        if b.str.is_empty() {
            b.last_cc = 0;
        } else {
            b.set_iterator();
            b.last_cc = b.previous_cc();
            // Set reorderStart after the last code point with cc<=1 if there is one.
            if b.last_cc > 1 {
                while b.previous_cc() > 1 {}
            }
            b.reorder_start = b.code_point_limit;
        }
        b
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.str.is_empty()
    }
    /// `length()`.
    pub fn length(&self) -> i32 {
        self.str.len() as i32
    }
    /// `getLastCC()`.
    pub fn get_last_cc(&self) -> i32 {
        self.last_cc
    }

    /// `getStringBuilder()`.
    pub fn get_string_builder(&mut self) -> &mut Vec<u16> {
        self.str
    }

    /// `equals(CharSequence s, int start, int limit)`.
    pub fn equals(&self, s: &[u16], start: i32, limit: i32) -> bool {
        UTF16Plus::equal_range(self.str, 0, self.str.len() as i32, s, start, limit)
    }

    /// `append(int c, int cc)`.
    pub fn append(&mut self, c: i32, cc: i32) {
        if self.last_cc <= cc || cc == 0 {
            append_code_point(self.str, c);
            self.last_cc = cc;
            if cc <= 1 {
                self.reorder_start = self.str.len() as i32;
            }
        } else {
            self.insert(c, cc);
        }
    }

    /// `append(CharSequence s, int start, int limit, boolean isNFD, int
    /// leadCC, int trailCC)`.
    pub fn append_cc(
        &mut self,
        s: &[u16],
        mut start: i32,
        limit: i32,
        is_nfd: bool,
        mut lead_cc: i32,
        trail_cc: i32,
    ) {
        if start == limit {
            return;
        }
        if self.last_cc <= lead_cc || lead_cc == 0 {
            if trail_cc <= 1 {
                self.reorder_start = self.str.len() as i32 + (limit - start);
            } else if lead_cc <= 1 {
                self.reorder_start = self.str.len() as i32 + 1; // Ok if not a code point boundary.
            }
            self.str
                .extend_from_slice(&s[start as usize..limit as usize]);
            self.last_cc = trail_cc;
        } else {
            let mut c = code_point_at(s, start);
            start += char_count(c);
            self.insert(c, lead_cc); // insert first code point
            while start < limit {
                c = code_point_at(s, start);
                start += char_count(c);
                if start < limit {
                    if is_nfd {
                        lead_cc =
                            Normalizer2Impl::get_cc_from_yes_or_maybe_yes(self.impl_.get_norm16(c));
                    } else {
                        lead_cc = self.impl_.get_cc(self.impl_.get_norm16(c));
                    }
                } else {
                    lead_cc = trail_cc;
                }
                self.append(c, lead_cc);
            }
        }
    }

    /// `appendZeroCC(int c)`.
    pub fn append_zero_cc(&mut self, c: i32) {
        append_code_point(self.str, c);
        self.last_cc = 0;
        self.reorder_start = self.str.len() as i32;
    }

    /// `append(CharSequence s)`: appends with zero combining class.
    pub fn append_str(&mut self, s: &[u16]) -> &mut Self {
        if !s.is_empty() {
            self.str.extend_from_slice(s);
            self.last_cc = 0;
            self.reorder_start = self.str.len() as i32;
        }
        self
    }

    /// `append(CharSequence s, int start, int limit)`: appends with zero
    /// combining class.
    pub fn append_range(&mut self, s: &[u16], start: i32, limit: i32) -> &mut Self {
        if start != limit {
            self.str
                .extend_from_slice(&s[start as usize..limit as usize]);
            self.last_cc = 0;
            self.reorder_start = self.str.len() as i32;
        }
        self
    }

    /// `flush()`: flushes from the intermediate StringBuilder to the
    /// Appendable, if they are different objects.
    pub fn flush(&mut self) {
        self.reorder_start = self.str.len() as i32;
        self.last_cc = 0;
    }

    /// `flushAndAppendZeroCC(CharSequence s, int start, int limit)`.
    pub fn flush_and_append_zero_cc(&mut self, s: &[u16], start: i32, limit: i32) -> &mut Self {
        self.str
            .extend_from_slice(&s[start as usize..limit as usize]);
        self.reorder_start = self.str.len() as i32;
        self.last_cc = 0;
        self
    }

    /// `remove()`.
    pub fn remove(&mut self) {
        self.str.clear();
        self.last_cc = 0;
        self.reorder_start = 0;
    }

    /// `removeSuffix(int suffixLength)`.
    pub fn remove_suffix(&mut self, suffix_length: i32) {
        let old_length = self.str.len() as i32;
        self.str.truncate((old_length - suffix_length) as usize);
        self.last_cc = 0;
        self.reorder_start = self.str.len() as i32;
    }

    /*
     * TODO: Revisit whether it makes sense to track reorderStart.
     * It is set to after the last known character with cc<=1,
     * which stops previousCC() before it reads that character and looks up its cc.
     * previousCC() is normally only called from insert().
     * In other words, reorderStart speeds up the insertion of a combining mark
     * into a multi-combining mark sequence where it does not belong at the end.
     * This might not be worth the trouble.
     * On the other hand, it's not a huge amount of trouble.
     *
     * We probably need it for UNORM_SIMPLE_APPEND.
     */

    // Inserts c somewhere before the last character.
    // Requires 0<cc<lastCC which implies reorderStart<limit.
    fn insert(&mut self, c: i32, cc: i32) {
        self.set_iterator();
        self.skip_previous();
        while self.previous_cc() > cc {}
        // insert c at codePointLimit, after the character with prevCC<=cc
        let at = self.code_point_limit as usize;
        if c <= 0xffff {
            self.str.insert(at, c as u16);
            if cc <= 1 {
                self.reorder_start = self.code_point_limit + 1;
            }
        } else {
            self.str.insert(at, UTF16::get_lead_surrogate(c));
            self.str.insert(at + 1, UTF16::get_trail_surrogate(c));
            if cc <= 1 {
                self.reorder_start = self.code_point_limit + 2;
            }
        }
    }

    fn set_iterator(&mut self) {
        self.code_point_start = self.str.len() as i32;
    }
    fn skip_previous(&mut self) {
        // Requires 0<codePointStart.
        self.code_point_limit = self.code_point_start;
        self.code_point_start = offset_by_code_points(self.str, self.code_point_start, -1);
    }
    fn previous_cc(&mut self) -> i32 {
        // Returns 0 if there is no previous character.
        self.code_point_limit = self.code_point_start;
        if self.reorder_start >= self.code_point_start {
            return 0;
        }
        let c = code_point_before(self.str, self.code_point_start);
        self.code_point_start -= char_count(c);
        self.impl_.get_cc_from_yes_or_maybe_yes_cp(c)
    }
}

/// `StringBuilder.appendCodePoint(int)` for a code point the data yields.
fn append_code_point(sb: &mut Vec<u16>, c: i32) {
    if c <= 0xffff {
        sb.push(c as u16);
    } else {
        sb.push(UTF16::get_lead_surrogate(c));
        sb.push(UTF16::get_trail_surrogate(c));
    }
}

/// The canonical iterator data `ensureCanonIterData()` builds lazily.
#[derive(Debug)]
struct CanonIterData {
    canon_iter_data: CodePointTrie,
    canon_start_sets: Vec<UnicodeSet>,
}

/// Low-level implementation of the Unicode Normalization Algorithm.
///
/// Equivalent to `com.ibm.icu.impl.Normalizer2Impl`. The canonical
/// iterator data that Java builds in the synchronized
/// `ensureCanonIterData()` is set once behind a lock.
#[derive(Debug, Default)]
pub struct Normalizer2Impl {
    data_version: Option<VersionInfo>,

    // BMP code point thresholds for quick check loops looking at single UTF-16 code units.
    min_decomp_no_cp: i32,
    min_comp_no_maybe_cp: i32,
    min_lccc_cp: i32,

    // Norm16 value thresholds for quick check combinations and types of extra data.
    min_yes_no: i32,
    min_yes_no_mappings_only: i32,
    min_no_no: i32,
    min_no_no_comp_boundary_before: i32,
    min_no_no_comp_no_maybe_cc: i32,
    min_no_no_empty: i32,
    limit_no_no: i32,
    center_no_no_delta: i32,
    min_maybe_no: i32,
    min_maybe_no_combines_fwd: i32,
    min_maybe_yes: i32,

    norm_trie: Option<Fast>,
    extra_data: Vec<u16>,
    small_fcd: Vec<i8>,

    canon: OnceLock<CanonIterData>,
    canon_lock: Mutex<()>,
}

const DATA_FORMAT: i32 = 0x4e726d32; // "Nrm2"

fn is_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 5
}

// CanonIterData data structure:
// Bits in canonIterData
const CANON_NOT_SEGMENT_STARTER: i32 = 0x80000000_u32 as i32;
const CANON_HAS_COMPOSITIONS: i32 = 0x40000000;
const CANON_HAS_SET: i32 = 0x200000;
const CANON_VALUE_MASK: i32 = 0x1fffff;

impl Normalizer2Impl {
    /// `MIN_YES_YES_WITH_CC`.
    pub const MIN_YES_YES_WITH_CC: i32 = 0xfe02;
    /// `JAMO_VT`.
    pub const JAMO_VT: i32 = 0xfe00;
    /// `MIN_NORMAL_MAYBE_YES`.
    pub const MIN_NORMAL_MAYBE_YES: i32 = 0xfc00;
    /// `JAMO_L`.
    pub const JAMO_L: i32 = 2; // offset=1 hasCompBoundaryAfter=false
    /// `INERT`.
    pub const INERT: i32 = 1; // offset=0 hasCompBoundaryAfter=true

    // norm16 bit 0 is comp-boundary-after.
    /// `HAS_COMP_BOUNDARY_AFTER`.
    pub const HAS_COMP_BOUNDARY_AFTER: i32 = 1;
    /// `OFFSET_SHIFT`.
    pub const OFFSET_SHIFT: i32 = 1;

    // For algorithmic one-way mappings, norm16 bits 2..1 indicate the
    // tccc (0, 1, >1) for quick FCC boundary-after tests.
    /// `DELTA_TCCC_0`.
    pub const DELTA_TCCC_0: i32 = 0;
    /// `DELTA_TCCC_1`.
    pub const DELTA_TCCC_1: i32 = 2;
    /// `DELTA_TCCC_GT_1`.
    pub const DELTA_TCCC_GT_1: i32 = 4;
    /// `DELTA_TCCC_MASK`.
    pub const DELTA_TCCC_MASK: i32 = 6;
    /// `DELTA_SHIFT`.
    pub const DELTA_SHIFT: i32 = 3;

    /// `MAX_DELTA`.
    pub const MAX_DELTA: i32 = 0x40;

    // Byte offsets from the start of the data, after the generic header.
    /// `IX_NORM_TRIE_OFFSET`.
    pub const IX_NORM_TRIE_OFFSET: usize = 0;
    /// `IX_EXTRA_DATA_OFFSET`.
    pub const IX_EXTRA_DATA_OFFSET: usize = 1;
    /// `IX_SMALL_FCD_OFFSET`.
    pub const IX_SMALL_FCD_OFFSET: usize = 2;
    /// `IX_RESERVED3_OFFSET`.
    pub const IX_RESERVED3_OFFSET: usize = 3;
    /// `IX_TOTAL_SIZE`.
    pub const IX_TOTAL_SIZE: usize = 7;

    // Code point thresholds for quick check codes.
    /// `IX_MIN_DECOMP_NO_CP`.
    pub const IX_MIN_DECOMP_NO_CP: usize = 8;
    /// `IX_MIN_COMP_NO_MAYBE_CP`.
    pub const IX_MIN_COMP_NO_MAYBE_CP: usize = 9;

    // Norm16 value thresholds for quick check combinations and types of extra data.

    /// Mappings & compositions in [minYesNo..minYesNoMappingsOnly[.
    pub const IX_MIN_YES_NO: usize = 10;
    /// Mappings are comp-normalized.
    pub const IX_MIN_NO_NO: usize = 11;
    /// `IX_LIMIT_NO_NO`.
    pub const IX_LIMIT_NO_NO: usize = 12;
    /// `IX_MIN_MAYBE_YES`.
    pub const IX_MIN_MAYBE_YES: usize = 13;

    /// Mappings only in [minYesNoMappingsOnly..minNoNo[.
    pub const IX_MIN_YES_NO_MAPPINGS_ONLY: usize = 14;
    /// Mappings are not comp-normalized but have a comp boundary before.
    pub const IX_MIN_NO_NO_COMP_BOUNDARY_BEFORE: usize = 15;
    /// Mappings do not have a comp boundary before.
    pub const IX_MIN_NO_NO_COMP_NO_MAYBE_CC: usize = 16;
    /// Mappings to the empty string.
    pub const IX_MIN_NO_NO_EMPTY: usize = 17;

    /// `IX_MIN_LCCC_CP`.
    pub const IX_MIN_LCCC_CP: usize = 18;
    //public static final int IX_RESERVED19=19;

    /// Two-way mappings; each starts with a character that combines backward.
    pub const IX_MIN_MAYBE_NO: usize = 20;
    /// Two-way mappings & compositions.
    pub const IX_MIN_MAYBE_NO_COMBINES_FWD: usize = 21;

    //public static final int IX_COUNT=22;

    /// `MAPPING_HAS_CCC_LCCC_WORD`.
    pub const MAPPING_HAS_CCC_LCCC_WORD: i32 = 0x80;
    /// `MAPPING_HAS_RAW_MAPPING`.
    pub const MAPPING_HAS_RAW_MAPPING: i32 = 0x40;
    // unused bit 0x20;
    /// `MAPPING_LENGTH_MASK`.
    pub const MAPPING_LENGTH_MASK: i32 = 0x1f;

    /// `COMP_1_LAST_TUPLE`.
    pub const COMP_1_LAST_TUPLE: i32 = 0x8000;
    /// `COMP_1_TRIPLE`.
    pub const COMP_1_TRIPLE: i32 = 1;
    /// `COMP_1_TRAIL_LIMIT`.
    pub const COMP_1_TRAIL_LIMIT: i32 = 0x3400;
    /// `COMP_1_TRAIL_MASK`.
    pub const COMP_1_TRAIL_MASK: i32 = 0x7ffe;
    /// `COMP_1_TRAIL_SHIFT`: 10-1 for the "triple" bit.
    pub const COMP_1_TRAIL_SHIFT: i32 = 9;
    /// `COMP_2_TRAIL_SHIFT`.
    pub const COMP_2_TRAIL_SHIFT: i32 = 6;
    /// `COMP_2_TRAIL_MASK`.
    pub const COMP_2_TRAIL_MASK: i32 = 0xffc0;

    /// `Normalizer2Impl()`: an empty instance, to be loaded.
    pub fn new() -> Self {
        Self::default()
    }

    fn trie(&self) -> &Fast {
        self.norm_trie
            .as_ref()
            .expect("java.lang.NullPointerException: normTrie is loaded")
    }

    fn ed(&self, i: i32) -> i32 {
        self.extra_data[i as usize] as i32
    }

    /// `load(ByteBuffer bytes)`: reads normalization data.
    ///
    /// # Errors
    ///
    /// `ICUUncheckedIOException` for malformed data (Java wraps the
    /// `IOException`s of the header check).
    pub fn load(&mut self, bytes: &mut ByteBuffer) -> Result<&mut Self> {
        let r = self.load_inner(bytes);
        match r {
            Ok(()) => Ok(self),
            Err(e) if e.is_io_exception() => Err(ICUUncheckedIOException::with_cause(e).error()),
            Err(e) => Err(e),
        }
    }

    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    fn load_inner(&mut self, bytes: &mut ByteBuffer) -> Result<()> {
        self.data_version = Some(ICUBinary::read_header_and_data_version(
            bytes,
            DATA_FORMAT,
            Some(&is_acceptable as &dyn Fn(&[u8; 4]) -> bool),
        )?);
        let indexes_length = bytes.get_int()? / 4; // inIndexes[IX_NORM_TRIE_OFFSET]/4
        if indexes_length <= Self::IX_MIN_LCCC_CP as i32 {
            return Err(ICUUncheckedIOException::with_message(
                "Normalizer2 data: not enough indexes",
            )
            .error());
        }
        let mut in_indexes = vec![0i32; indexes_length as usize];
        in_indexes[0] = indexes_length * 4;
        for i in 1..indexes_length as usize {
            in_indexes[i] = bytes.get_int()?;
        }

        self.min_decomp_no_cp = in_indexes[Self::IX_MIN_DECOMP_NO_CP];
        self.min_comp_no_maybe_cp = in_indexes[Self::IX_MIN_COMP_NO_MAYBE_CP];
        self.min_lccc_cp = in_indexes[Self::IX_MIN_LCCC_CP];

        self.min_yes_no = in_indexes[Self::IX_MIN_YES_NO];
        self.min_yes_no_mappings_only = in_indexes[Self::IX_MIN_YES_NO_MAPPINGS_ONLY];
        self.min_no_no = in_indexes[Self::IX_MIN_NO_NO];
        self.min_no_no_comp_boundary_before = in_indexes[Self::IX_MIN_NO_NO_COMP_BOUNDARY_BEFORE];
        self.min_no_no_comp_no_maybe_cc = in_indexes[Self::IX_MIN_NO_NO_COMP_NO_MAYBE_CC];
        self.min_no_no_empty = in_indexes[Self::IX_MIN_NO_NO_EMPTY];
        self.limit_no_no = in_indexes[Self::IX_LIMIT_NO_NO];
        self.min_maybe_no = in_indexes[Self::IX_MIN_MAYBE_NO];
        self.min_maybe_no_combines_fwd = in_indexes[Self::IX_MIN_MAYBE_NO_COMBINES_FWD];
        self.min_maybe_yes = in_indexes[Self::IX_MIN_MAYBE_YES];
        debug_assert!((self.min_maybe_no & 7) == 0); // 8-aligned for noNoDelta bit fields
        self.center_no_no_delta = (self.min_maybe_no >> Self::DELTA_SHIFT) - Self::MAX_DELTA - 1;

        // Read the normTrie.
        let mut offset = in_indexes[Self::IX_NORM_TRIE_OFFSET];
        let mut next_offset = in_indexes[Self::IX_EXTRA_DATA_OFFSET];
        let trie_position = bytes.position();
        self.norm_trie = Some(Fast::from_binary(Some(ValueWidth::Bits16), bytes)?);
        let trie_length = bytes.position() - trie_position;
        if trie_length > (next_offset - offset) {
            return Err(ICUUncheckedIOException::with_message(
                "Normalizer2 data: not enough bytes for normTrie",
            )
            .error());
        }
        ICUBinary::skip_bytes(bytes, (next_offset - offset) - trie_length)?; // skip padding after trie bytes

        // Read the composition and mapping data.
        offset = next_offset;
        next_offset = in_indexes[Self::IX_SMALL_FCD_OFFSET];
        let num_chars = (next_offset - offset) / 2;
        if num_chars != 0 {
            self.extra_data = ICUBinary::get_string(bytes, num_chars, 0)?;
        }

        // smallFCD: new in formatVersion 2
        let mut small = vec![0u8; 0x100];
        bytes.get_bytes(&mut small)?;
        self.small_fcd = small.into_iter().map(|b| b as i8).collect();
        Ok(())
    }

    /// `load(String name)`: reads normalization data from the ICU data.
    ///
    /// # Errors
    ///
    /// As [`load`](Self::load); `MissingResourceException` when the data
    /// item does not exist.
    pub fn load_name(&mut self, name: &str) -> Result<&mut Self> {
        let mut bytes = ICUBinary::get_required_data(name)?;
        self.load(&mut bytes)
    }

    /// `addLcccChars(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add_range`] (a frozen set).
    pub fn add_lccc_chars(&self, set: &mut UnicodeSet) -> Result<()> {
        let mut start = 0;
        let mut range = Range::new();
        while self.trie().get_range_with_option(
            start,
            RangeOption::FixedLeadSurrogates,
            Self::INERT,
            None,
            &mut range,
        ) {
            let end = range.get_end();
            let norm16 = range.get_value();
            if norm16 > Self::MIN_NORMAL_MAYBE_YES && norm16 != Self::JAMO_VT {
                set.add_range(start, end)?;
            } else if self.min_no_no_comp_no_maybe_cc <= norm16 && norm16 < self.limit_no_no {
                let fcd16 = self.get_fcd16(start);
                if fcd16 > 0xff {
                    set.add_range(start, end)?;
                }
            }
            start = end + 1;
        }
        Ok(())
    }

    /// `addPropertyStarts(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_property_starts(&self, set: &mut UnicodeSet) -> Result<()> {
        // Add the start code point of each same-value range of the trie.
        let mut start = 0;
        let mut range = Range::new();
        while self.trie().get_range_with_option(
            start,
            RangeOption::FixedLeadSurrogates,
            Self::INERT,
            None,
            &mut range,
        ) {
            let end = range.get_end();
            let value = range.get_value();
            set.add(start)?;
            if start != end
                && self.is_algorithmic_no_no(value)
                && (value & Self::DELTA_TCCC_MASK) > Self::DELTA_TCCC_1
            {
                // Range of code points with same-norm16-value algorithmic decompositions.
                // They might have different non-zero FCD16 values.
                let mut prev_fcd16 = self.get_fcd16(start);
                loop {
                    start += 1;
                    if start > end {
                        break;
                    }
                    let fcd16 = self.get_fcd16(start);
                    if fcd16 != prev_fcd16 {
                        set.add(start)?;
                        prev_fcd16 = fcd16;
                    }
                }
            }
            start = end + 1;
        }

        /* add Hangul LV syllables and LV+1 because of skippables */
        let mut c = Hangul::HANGUL_BASE;
        while c < Hangul::HANGUL_LIMIT {
            set.add(c)?;
            set.add(c + 1)?;
            c += Hangul::JAMO_T_COUNT;
        }
        set.add(Hangul::HANGUL_LIMIT)?; /* add Hangul+1 to continue with other properties */
        Ok(())
    }

    /// `addCanonIterPropertyStarts(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set) and of building
    /// the canonical iterator data.
    pub fn add_canon_iter_property_starts(&self, set: &mut UnicodeSet) -> Result<()> {
        /* add the start code point of each same-value range of the canonical iterator data trie */
        self.ensure_canon_iter_data()?;
        // Currently only used for the SEGMENT_STARTER property.
        let canon = self.canon_data();
        let mut start = 0;
        let mut range = Range::new();
        let segment_starter_mapper = |value: i32| value & CANON_NOT_SEGMENT_STARTER;
        while canon
            .canon_iter_data
            .get_range(start, Some(&segment_starter_mapper), &mut range)
        {
            set.add(start)?;
            start = range.get_end() + 1;
        }
        Ok(())
    }

    fn canon_data(&self) -> &CanonIterData {
        self.canon
            .get()
            .expect("java.lang.NullPointerException: canonIterData is built")
    }

    /// `ensureCanonIterData()` (synchronized): builds the canonical
    /// iterator data once.
    ///
    /// # Errors
    ///
    /// The errors of building the tries and sets.
    pub fn ensure_canon_iter_data(&self) -> Result<&Self> {
        if self.canon.get().is_some() {
            return Ok(self);
        }
        let _guard = self.canon_lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.canon.get().is_none() {
            let mut mutable_trie = MutableCodePointTrie::new(0, 0);
            let mut canon_start_sets: Vec<UnicodeSet> = Vec::new();
            let mut start = 0;
            let mut range = Range::new();
            while self.trie().get_range_with_option(
                start,
                RangeOption::FixedLeadSurrogates,
                Self::INERT,
                None,
                &mut range,
            ) {
                let end = range.get_end();
                let norm16 = range.get_value();
                if Self::is_inert(norm16)
                    || (self.min_yes_no <= norm16 && norm16 < self.min_no_no)
                    || (self.min_maybe_no <= norm16 && norm16 < self.min_maybe_yes)
                {
                    // Inert, or 2-way mapping (including Hangul syllable).
                    // We do not write a canonStartSet for any yesNo/maybeNo character.
                    // Composites from 2-way mappings are added at runtime from the
                    // starter's compositions list,
                    // and the other characters in 2-way mappings get CANON_NOT_SEGMENT_STARTER.
                    start = end + 1;
                    continue;
                }
                for c in start..=end {
                    let old_value = mutable_trie.get(c);
                    let mut new_value = old_value;
                    if self.is_maybe_yes_or_non_zero_cc(norm16) {
                        // not a segment starter if it occurs in a decomposition or has cc!=0
                        new_value |= CANON_NOT_SEGMENT_STARTER;
                        if norm16 < Self::MIN_NORMAL_MAYBE_YES {
                            new_value |= CANON_HAS_COMPOSITIONS;
                        }
                    } else if norm16 < self.min_yes_no {
                        new_value |= CANON_HAS_COMPOSITIONS;
                    } else {
                        // c has a one-way decomposition
                        let mut c2 = c;
                        // Do not modify the whole-range norm16 value.
                        let mut norm16_2 = norm16;
                        if self.is_decomp_no_algorithmic(norm16_2) {
                            // Maps to an isCompYesAndZeroCC.
                            c2 = self.map_algorithmic(c2, norm16_2);
                            norm16_2 = self.get_raw_norm16(c2);
                            // No compatibility mappings for the CanonicalIterator.
                            debug_assert!(
                                !(self.is_hangul_lv(norm16_2) || self.is_hangul_lvt(norm16_2))
                            );
                        }
                        if norm16_2 > self.min_yes_no {
                            // c decomposes, get everything from the variable-length extra data
                            let mut mapping = self.get_data_for_yes_or_no(norm16_2);
                            let first_unit = self.ed(mapping);
                            let length = first_unit & Self::MAPPING_LENGTH_MASK;
                            if (first_unit & Self::MAPPING_HAS_CCC_LCCC_WORD) != 0
                                && c == c2
                                && (self.ed(mapping - 1) & 0xff) != 0
                            {
                                new_value |= CANON_NOT_SEGMENT_STARTER; // original c has cc!=0
                            }
                            // Skip empty mappings (no characters in the decomposition).
                            if length != 0 {
                                mapping += 1; // skip over the firstUnit
                                              // add c to first code point's start set
                                let limit = mapping + length;
                                c2 = code_point_at(&self.extra_data, mapping);
                                Self::add_to_start_set(
                                    &mut mutable_trie,
                                    &mut canon_start_sets,
                                    c,
                                    c2,
                                )?;
                                // Set CANON_NOT_SEGMENT_STARTER for each remaining code point of a
                                // one-way mapping. A 2-way mapping is possible here after
                                // intermediate algorithmic mapping.
                                if norm16_2 >= self.min_no_no {
                                    loop {
                                        mapping += char_count(c2);
                                        if mapping >= limit {
                                            break;
                                        }
                                        c2 = code_point_at(&self.extra_data, mapping);
                                        let c2_value = mutable_trie.get(c2);
                                        if (c2_value & CANON_NOT_SEGMENT_STARTER) == 0 {
                                            mutable_trie
                                                .set(c2, c2_value | CANON_NOT_SEGMENT_STARTER)?;
                                        }
                                    }
                                }
                            }
                        } else {
                            // c decomposed to c2 algorithmically; c has cc==0
                            Self::add_to_start_set(
                                &mut mutable_trie,
                                &mut canon_start_sets,
                                c,
                                c2,
                            )?;
                        }
                    }
                    if new_value != old_value {
                        mutable_trie.set(c, new_value)?;
                    }
                }
                start = end + 1;
            }
            let canon_iter_data =
                mutable_trie.build_immutable(Some(Type::Small), Some(ValueWidth::Bits32))?;
            let _ = self.canon.set(CanonIterData {
                canon_iter_data,
                canon_start_sets,
            });
        }
        Ok(self)
    }

    // The trie stores values for lead surrogate code *units*.
    // Surrogate code *points* are inert.
    /// `getNorm16(int c)`.
    pub fn get_norm16(&self, c: i32) -> i32 {
        if UTF16Plus::is_lead_surrogate(c) {
            Self::INERT
        } else {
            self.trie().get(c)
        }
    }
    /// `getRawNorm16(int c)`.
    pub fn get_raw_norm16(&self, c: i32) -> i32 {
        self.trie().get(c)
    }

    /// `getCompQuickCheck(int norm16)`: 0=NO, 1=YES, 2=MAYBE.
    pub fn get_comp_quick_check(&self, norm16: i32) -> i32 {
        if norm16 < self.min_no_no || Self::MIN_YES_YES_WITH_CC <= norm16 {
            1 // yes
        } else if self.min_maybe_no <= norm16 {
            2 // maybe
        } else {
            0 // no
        }
    }
    /// `isAlgorithmicNoNo(int norm16)`.
    pub fn is_algorithmic_no_no(&self, norm16: i32) -> bool {
        self.limit_no_no <= norm16 && norm16 < self.min_maybe_no
    }
    /// `isCompNo(int norm16)`.
    pub fn is_comp_no(&self, norm16: i32) -> bool {
        self.min_no_no <= norm16 && norm16 < self.min_maybe_no
    }
    /// `isDecompYes(int norm16)`.
    pub fn is_decomp_yes(&self, norm16: i32) -> bool {
        norm16 < self.min_yes_no || self.min_maybe_yes <= norm16
    }

    /// `getCC(int norm16)`.
    pub fn get_cc(&self, norm16: i32) -> i32 {
        if norm16 >= Self::MIN_NORMAL_MAYBE_YES {
            return Self::get_cc_from_normal_yes_or_maybe(norm16);
        }
        if norm16 < self.min_no_no || self.limit_no_no <= norm16 {
            return 0;
        }
        self.get_cc_from_no_no(norm16)
    }
    /// `getCCFromNormalYesOrMaybe(int norm16)`.
    pub fn get_cc_from_normal_yes_or_maybe(norm16: i32) -> i32 {
        (norm16 >> Self::OFFSET_SHIFT) & 0xff
    }
    /// `getCCFromYesOrMaybeYes(int norm16)`.
    pub fn get_cc_from_yes_or_maybe_yes(norm16: i32) -> i32 {
        if norm16 >= Self::MIN_NORMAL_MAYBE_YES {
            Self::get_cc_from_normal_yes_or_maybe(norm16)
        } else {
            0
        }
    }
    /// `getCCFromYesOrMaybeYesCP(int c)`.
    pub fn get_cc_from_yes_or_maybe_yes_cp(&self, c: i32) -> i32 {
        if c < self.min_comp_no_maybe_cp {
            return 0;
        }
        Self::get_cc_from_yes_or_maybe_yes(self.get_norm16(c))
    }

    /// `getFCD16(int c)`: the FCD data for code point c.
    #[allow(clippy::if_same_then_else)] // branches mirror the Java source
    pub fn get_fcd16(&self, c: i32) -> i32 {
        if c < self.min_decomp_no_cp {
            return 0;
        } else if c <= 0xffff && !self.single_lead_might_have_non_zero_fcd16(c) {
            return 0;
        }
        self.get_fcd16_from_norm_data(c)
    }
    /// `singleLeadMightHaveNonZeroFCD16(int lead)`: for FCD, a fast pre-test
    /// for a BMP code point or lead surrogate.
    pub fn single_lead_might_have_non_zero_fcd16(&self, lead: i32) -> bool {
        // 0<=lead<=0xffff
        let bits = self.small_fcd[(lead >> 8) as usize] as i32;
        if bits == 0 {
            return false;
        }
        ((bits >> ((lead >> 5) & 7)) & 1) != 0
    }

    /// `getFCD16FromNormData(int c)`: gets the FCD value from the regular
    /// normalization data.
    pub fn get_fcd16_from_norm_data(&self, mut c: i32) -> i32 {
        let mut norm16 = self.get_norm16(c);
        if norm16 >= self.limit_no_no {
            if norm16 >= Self::MIN_NORMAL_MAYBE_YES {
                // combining mark
                norm16 = Self::get_cc_from_normal_yes_or_maybe(norm16);
                return norm16 | (norm16 << 8);
            } else if norm16 >= self.min_maybe_yes {
                return 0;
            } else if norm16 < self.min_maybe_no {
                // isDecompNoAlgorithmic(norm16)
                let delta_trail_cc = norm16 & Self::DELTA_TCCC_MASK;
                if delta_trail_cc <= Self::DELTA_TCCC_1 {
                    return delta_trail_cc >> Self::OFFSET_SHIFT;
                }
                // Maps to an isCompYesAndZeroCC.
                c = self.map_algorithmic(c, norm16);
                norm16 = self.get_raw_norm16(c);
            }
        }
        if norm16 <= self.min_yes_no || self.is_hangul_lvt(norm16) {
            // no decomposition or Hangul syllable, all zeros
            return 0;
        }
        // c decomposes, get everything from the variable-length extra data
        let mapping = self.get_data(norm16);
        let first_unit = self.ed(mapping);
        let mut fcd16 = first_unit >> 8; // tccc
        if (first_unit & Self::MAPPING_HAS_CCC_LCCC_WORD) != 0 {
            fcd16 |= self.ed(mapping - 1) & 0xff00; // lccc
        }
        fcd16
    }

    fn get_fcd16_from_maybe_or_non_zero_cc(&self, mut norm16: i32) -> i32 {
        debug_assert!(norm16 >= self.min_maybe_no);
        if norm16 >= Self::MIN_NORMAL_MAYBE_YES {
            // combining mark
            norm16 = Self::get_cc_from_normal_yes_or_maybe(norm16);
            return norm16 | (norm16 << 8);
        } else if norm16 >= self.min_maybe_yes {
            return 0;
        }
        // c decomposes, get everything from the variable-length extra data
        let mapping = self.get_data_for_maybe(norm16);
        let first_unit = self.ed(mapping);
        // maybeNo has lccc = 0
        debug_assert!(
            (first_unit & Self::MAPPING_HAS_CCC_LCCC_WORD) == 0
                || (self.ed(mapping - 1) & 0xff00) == 0
        );
        first_unit >> 8 // tccc
    }

    /// `getDecomposition(int c)`: the decomposition, `None` when none.
    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    pub fn get_decomposition(&self, mut c: i32) -> Option<Vec<u16>> {
        let mut norm16 = 0;
        if c < self.min_decomp_no_cp || {
            norm16 = self.get_norm16(c);
            self.is_maybe_yes_or_non_zero_cc(norm16)
        } {
            // c does not decompose
            return None;
        }
        let mut decomp = -1;
        if self.is_decomp_no_algorithmic(norm16) {
            // Maps to an isCompYesAndZeroCC.
            c = self.map_algorithmic(c, norm16);
            decomp = c;
            // The mapping might decompose further.
            norm16 = self.get_raw_norm16(c);
        }
        if norm16 < self.min_yes_no {
            if decomp < 0 {
                return None;
            } else {
                return Some(UTF16::value_of(decomp).expect("a valid code point"));
            }
        } else if self.is_hangul_lv(norm16) || self.is_hangul_lvt(norm16) {
            // Hangul syllable: decompose algorithmically
            let mut buffer: Vec<u16> = Vec::new();
            Hangul::decompose(c, &mut buffer);
            return Some(buffer);
        }
        // c decomposes, get everything from the variable-length extra data
        let mut mapping = self.get_data(norm16);
        let length = self.ed(mapping) & Self::MAPPING_LENGTH_MASK;
        mapping += 1;
        Some(self.extra_data[mapping as usize..(mapping + length) as usize].to_vec())
    }

    /// `getRawDecomposition(int c)`: the raw decomposition, `None` when
    /// none.
    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    pub fn get_raw_decomposition(&self, c: i32) -> Option<Vec<u16>> {
        let mut norm16 = 0;
        if c < self.min_decomp_no_cp || {
            norm16 = self.get_norm16(c);
            self.is_decomp_yes(norm16)
        } {
            // c does not decompose
            return None;
        } else if self.is_hangul_lv(norm16) || self.is_hangul_lvt(norm16) {
            // Hangul syllable: decompose algorithmically
            let mut buffer: Vec<u16> = Vec::new();
            Hangul::get_raw_decomposition(c, &mut buffer);
            return Some(buffer);
        } else if self.is_decomp_no_algorithmic(norm16) {
            return Some(
                UTF16::value_of(self.map_algorithmic(c, norm16)).expect("a valid code point"),
            );
        }
        // c decomposes, get everything from the variable-length extra data
        let mut mapping = self.get_data(norm16);
        let first_unit = self.ed(mapping);
        let m_length = first_unit & Self::MAPPING_LENGTH_MASK; // length of normal mapping
        if (first_unit & Self::MAPPING_HAS_RAW_MAPPING) != 0 {
            // Read the raw mapping from before the firstUnit and before the optional ccc/lccc word.
            // Bit 7=MAPPING_HAS_CCC_LCCC_WORD
            let raw_mapping = mapping - ((first_unit >> 7) & 1) - 1;
            let rm0 = self.extra_data[raw_mapping as usize];
            if rm0 as i32 <= Self::MAPPING_LENGTH_MASK {
                Some(
                    self.extra_data[(raw_mapping - rm0 as i32) as usize..raw_mapping as usize]
                        .to_vec(),
                )
            } else {
                // Copy the normal mapping and replace its first two code units with rm0.
                let mut buffer: Vec<u16> = Vec::with_capacity((m_length - 1).max(0) as usize);
                buffer.push(rm0);
                mapping += 1 + 2; // skip over the firstUnit and the first two mapping code units
                buffer.extend_from_slice(
                    &self.extra_data[mapping as usize..(mapping + m_length - 2) as usize],
                );
                Some(buffer)
            }
        } else {
            mapping += 1; // skip over the firstUnit
            Some(self.extra_data[mapping as usize..(mapping + m_length) as usize].to_vec())
        }
    }

    /// `isCanonSegmentStarter(int c)`: whether c is a segment starter
    /// (requires `ensureCanonIterData()`).
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` before `ensureCanonIterData()`.
    pub fn is_canon_segment_starter(&self, c: i32) -> bool {
        self.canon_data().canon_iter_data.get(c) >= 0
    }

    /// `getCanonStartSet(int c, UnicodeSet set)`: fills `set` with the
    /// canonical start set of c; false when it has none.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet`] modification (a frozen set).
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` before `ensureCanonIterData()`.
    pub fn get_canon_start_set(&self, c: i32, set: &mut UnicodeSet) -> Result<bool> {
        let canon = self.canon_data();
        let canon_value = canon.canon_iter_data.get(c) & !CANON_NOT_SEGMENT_STARTER;
        if canon_value == 0 {
            return Ok(false);
        }
        set.clear()?;
        let value = canon_value & CANON_VALUE_MASK;
        if (canon_value & CANON_HAS_SET) != 0 {
            set.add_all(&canon.canon_start_sets[value as usize])?;
        } else if value != 0 {
            set.add(value)?;
        }
        if (canon_value & CANON_HAS_COMPOSITIONS) != 0 {
            let norm16 = self.get_raw_norm16(c);
            if norm16 == Self::JAMO_L {
                let syllable =
                    Hangul::HANGUL_BASE + (c - Hangul::JAMO_L_BASE) * Hangul::JAMO_VT_COUNT;
                set.add_range(syllable, syllable + Hangul::JAMO_VT_COUNT - 1)?;
            } else {
                self.add_composites(self.get_compositions_list(norm16), set)?;
            }
        }
        Ok(true)
    }

    /// `decompose(CharSequence s, StringBuilder dest)`.
    pub fn decompose_into<'d>(&self, s: &[u16], dest: &'d mut Vec<u16>) -> &'d mut Vec<u16> {
        self.decompose_range_into(s, 0, s.len() as i32, dest, s.len() as i32);
        dest
    }

    /// `decompose(CharSequence s, int src, int limit, StringBuilder dest,
    /// int destLengthEstimate)`: decomposes s[src, limit[ and writes the
    /// result to dest.
    pub fn decompose_range_into(
        &self,
        s: &[u16],
        src: i32,
        limit: i32,
        dest: &mut Vec<u16>,
        mut dest_length_estimate: i32,
    ) {
        if dest_length_estimate < 0 {
            dest_length_estimate = limit - src;
        }
        dest.clear();
        let mut buffer = ReorderingBuffer::new(self, dest, dest_length_estimate);
        self.decompose(s, src, limit, Some(&mut buffer));
    }

    // Dual functionality:
    // buffer!=NULL: normalize
    // buffer==NULL: isNormalized/quickCheck/spanQuickCheckYes
    /// `decompose(CharSequence s, int src, int limit, ReorderingBuffer
    /// buffer)`: normalizes into `buffer`, or (without one) returns the
    /// end of the normalized prefix.
    pub fn decompose(
        &self,
        s: &[u16],
        mut src: i32,
        limit: i32,
        mut buffer: Option<&mut ReorderingBuffer<'_>>,
    ) -> i32 {
        let min_no_cp = self.min_decomp_no_cp;

        let mut prev_src;
        let mut c = 0;
        let mut norm16 = 0;

        // only for quick check
        let mut prev_boundary = src;
        let mut prev_cc = 0;

        loop {
            // count code units below the minimum or with irrelevant data for the quick check
            prev_src = src;
            while src != limit {
                c = s[src as usize] as i32;
                if c < min_no_cp || {
                    norm16 = self.trie().bmp_get(c);
                    self.is_most_decomp_yes_and_zero_cc(norm16)
                } {
                    src += 1;
                } else if !UTF16Plus::is_lead_surrogate(c) {
                    break;
                } else {
                    if (src + 1) != limit && is_low_surrogate(s[src as usize + 1]) {
                        let c2 = s[src as usize + 1];
                        c = to_code_point(c as u16, c2);
                        norm16 = self.trie().supp_get(c);
                        if self.is_most_decomp_yes_and_zero_cc(norm16) {
                            src += 2;
                        } else {
                            break;
                        }
                    } else {
                        src += 1; // unpaired lead surrogate: inert
                    }
                }
            }
            // copy these code units all at once
            if src != prev_src {
                if let Some(b) = buffer.as_deref_mut() {
                    b.flush_and_append_zero_cc(s, prev_src, src);
                } else {
                    prev_cc = 0;
                    prev_boundary = src;
                }
            }
            if src == limit {
                break;
            }

            // Check one above-minimum, relevant code point.
            src += char_count(c);
            if let Some(b) = buffer.as_deref_mut() {
                self.decompose_cp(c, norm16, b);
            } else {
                if self.is_decomp_yes(norm16) {
                    let cc = Self::get_cc_from_yes_or_maybe_yes(norm16);
                    if prev_cc <= cc || cc == 0 {
                        prev_cc = cc;
                        if cc <= 1 {
                            prev_boundary = src;
                        }
                        continue;
                    }
                }
                return prev_boundary; // "no" or cc out of order
            }
        }
        src
    }

    /// `decomposeAndAppend(CharSequence s, boolean doDecompose,
    /// ReorderingBuffer buffer)`.
    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    pub fn decompose_and_append(
        &self,
        s: &[u16],
        do_decompose: bool,
        buffer: &mut ReorderingBuffer<'_>,
    ) {
        let limit = s.len() as i32;
        if limit == 0 {
            return;
        }
        if do_decompose {
            self.decompose(s, 0, limit, Some(buffer));
            return;
        }
        // Just merge the strings at the boundary.
        let mut c = code_point_at(s, 0);
        let mut src = 0;
        let first_cc;
        let mut prev_cc;
        let mut cc;
        cc = self.get_cc(self.get_norm16(c));
        first_cc = cc;
        prev_cc = cc;
        while cc != 0 {
            prev_cc = cc;
            src += char_count(c);
            if src >= limit {
                break;
            }
            c = code_point_at(s, src);
            cc = self.get_cc(self.get_norm16(c));
        }
        buffer.append_cc(s, 0, src, false, first_cc, prev_cc);
        buffer.append_range(s, src, limit);
    }

    // Very similar to composeQuickCheck(): Make the same changes in both places if relevant.
    // doCompose: normalize
    // !doCompose: isNormalized (buffer must be empty and initialized)
    /// `compose(CharSequence s, int src, int limit, boolean onlyContiguous,
    /// boolean doCompose, ReorderingBuffer buffer)`.
    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    pub fn compose(
        &self,
        s: &[u16],
        mut src: i32,
        limit: i32,
        only_contiguous: bool,
        do_compose: bool,
        buffer: &mut ReorderingBuffer<'_>,
    ) -> bool {
        let mut prev_boundary = src;
        let min_no_maybe_cp = self.min_comp_no_maybe_cp;

        loop {
            // Fast path: Scan over a sequence of characters below the minimum "no or maybe" code point,
            // or with (compYes && ccc==0) properties.
            let mut prev_src = src;
            let mut c;
            let mut norm16;
            loop {
                if src == limit {
                    if prev_boundary != limit && do_compose {
                        buffer.append_range(s, prev_boundary, limit);
                    }
                    return true;
                }
                c = s[src as usize] as i32;
                norm16 = 0;
                if c < min_no_maybe_cp || {
                    norm16 = self.trie().bmp_get(c);
                    self.is_comp_yes_and_zero_cc(norm16)
                } {
                    src += 1;
                } else {
                    prev_src = src;
                    src += 1;
                    if !UTF16Plus::is_lead_surrogate(c) {
                        break;
                    } else {
                        if src != limit && is_low_surrogate(s[src as usize]) {
                            let c2 = s[src as usize];
                            src += 1;
                            c = to_code_point(c as u16, c2);
                            norm16 = self.trie().supp_get(c);
                            if !self.is_comp_yes_and_zero_cc(norm16) {
                                break;
                            }
                        }
                    }
                }
            }
            // isCompYesAndZeroCC(norm16) is false, that is, norm16>=minNoNo.
            // The current character is either a "noNo" (has a mapping)
            // or a "maybeYes" / "maybeNo" (combines backward)
            // or a "yesYes" with ccc!=0.
            // It is not a Hangul syllable or Jamo L because those have "yes" properties.

            // Medium-fast path: Handle cases that do not require full decomposition and recomposition.
            if norm16 < self.min_maybe_no {
                // minNoNo <= norm16 < minMaybeNo
                if !do_compose {
                    return false;
                }
                // Fast path for mapping a character that is immediately surrounded by boundaries.
                // In this case, we need not decompose around the current character.
                if self.is_decomp_no_algorithmic(norm16) {
                    // Maps to a single isCompYesAndZeroCC character
                    // which also implies hasCompBoundaryBefore.
                    if self.norm16_has_comp_boundary_after(norm16, only_contiguous)
                        || self.has_comp_boundary_before_at(s, src, limit)
                    {
                        if prev_boundary != prev_src {
                            buffer.append_range(s, prev_boundary, prev_src);
                        }
                        buffer.append(self.map_algorithmic(c, norm16), 0);
                        prev_boundary = src;
                        continue;
                    }
                } else if norm16 < self.min_no_no_comp_boundary_before {
                    // The mapping is comp-normalized which also implies hasCompBoundaryBefore.
                    if self.norm16_has_comp_boundary_after(norm16, only_contiguous)
                        || self.has_comp_boundary_before_at(s, src, limit)
                    {
                        if prev_boundary != prev_src {
                            buffer.append_range(s, prev_boundary, prev_src);
                        }
                        let mut mapping = self.get_data_for_yes_or_no(norm16);
                        let length = self.ed(mapping) & Self::MAPPING_LENGTH_MASK;
                        mapping += 1;
                        buffer.append_range(&self.extra_data, mapping, mapping + length);
                        prev_boundary = src;
                        continue;
                    }
                } else if norm16 >= self.min_no_no_empty {
                    // The current character maps to nothing.
                    // Simply omit it from the output if there is a boundary before _or_ after it.
                    // The character itself implies no boundaries.
                    if self.has_comp_boundary_before_at(s, src, limit)
                        || self.has_comp_boundary_after_range(
                            s,
                            prev_boundary,
                            prev_src,
                            only_contiguous,
                        )
                    {
                        if prev_boundary != prev_src {
                            buffer.append_range(s, prev_boundary, prev_src);
                        }
                        prev_boundary = src;
                        continue;
                    }
                }
                // Other "noNo" type, or need to examine more text around this character:
                // Fall through to the slow path.
            } else if Self::is_jamo_vt(norm16) && prev_boundary != prev_src {
                let prev = s[prev_src as usize - 1];
                if c < Hangul::JAMO_T_BASE {
                    // The current character is a Jamo Vowel,
                    // compose with previous Jamo L and following Jamo T.
                    let l = prev.wrapping_sub(Hangul::JAMO_L_BASE as u16);
                    if (l as i32) < Hangul::JAMO_L_COUNT {
                        if !do_compose {
                            return false;
                        }
                        let t;
                        let tt;
                        if src != limit
                            && {
                                tt = s[src as usize] as i32 - Hangul::JAMO_T_BASE;
                                0 < tt
                            }
                            && tt < Hangul::JAMO_T_COUNT
                        {
                            // The next character is a Jamo T.
                            t = tt;
                            src += 1;
                        } else if self.has_comp_boundary_before_at(s, src, limit) {
                            // No Jamo T follows, not even via decomposition.
                            t = 0;
                        } else {
                            t = -1;
                        }
                        if t >= 0 {
                            let syllable = Hangul::HANGUL_BASE
                                + (l as i32 * Hangul::JAMO_V_COUNT + (c - Hangul::JAMO_V_BASE))
                                    * Hangul::JAMO_T_COUNT
                                + t;
                            prev_src -= 1; // Replace the Jamo L as well.
                            if prev_boundary != prev_src {
                                buffer.append_range(s, prev_boundary, prev_src);
                            }
                            buffer.append_char(syllable as u16);
                            prev_boundary = src;
                            continue;
                        }
                        // If we see L+V+x where x!=T then we drop to the slow path,
                        // decompose and recompose.
                        // This is to deal with NFKC finding normal L and V but a
                        // compatibility variant of a T.
                        // We need to either fully compose that combination here
                        // (which would complicate the code and may not work with strange custom data)
                        // or use the slow path.
                    }
                } else if Hangul::is_hangul_lv(prev as i32) {
                    // The current character is a Jamo Trailing consonant,
                    // compose with previous Hangul LV that does not contain a Jamo T.
                    if !do_compose {
                        return false;
                    }
                    let syllable = prev as i32 + c - Hangul::JAMO_T_BASE;
                    prev_src -= 1; // Replace the Hangul LV as well.
                    if prev_boundary != prev_src {
                        buffer.append_range(s, prev_boundary, prev_src);
                    }
                    buffer.append_char(syllable as u16);
                    prev_boundary = src;
                    continue;
                }
                // No matching context, or may need to decompose surrounding text first:
                // Fall through to the slow path.
            } else if norm16 > Self::JAMO_VT {
                // norm16 >= MIN_YES_YES_WITH_CC
                // One or more combining marks that do not combine-back:
                // Check for canonical order, copy unchanged if ok and
                // if followed by a character with a boundary-before.
                let mut cc = Self::get_cc_from_normal_yes_or_maybe(norm16); // cc!=0
                if only_contiguous /* FCC */ && self.get_previous_trail_cc(s, prev_boundary, prev_src) > cc
                {
                    // Fails FCD test, need to decompose and contiguously recompose.
                    if !do_compose {
                        return false;
                    }
                } else {
                    // If !onlyContiguous (not FCC), then we ignore the tccc of
                    // the previous character which passed the quick check "yes && ccc==0" test.
                    let n16;
                    loop {
                        if src == limit {
                            if do_compose {
                                buffer.append_range(s, prev_boundary, limit);
                            }
                            return true;
                        }
                        let prev_cc = cc;
                        c = code_point_at(s, src);
                        let x = self.trie().get(c);
                        if x >= Self::MIN_YES_YES_WITH_CC {
                            cc = Self::get_cc_from_normal_yes_or_maybe(x);
                            if prev_cc > cc {
                                if !do_compose {
                                    return false;
                                }
                                n16 = x;
                                break;
                            }
                        } else {
                            n16 = x;
                            break;
                        }
                        src += char_count(c);
                    }
                    // p is after the last in-order combining mark.
                    // If there is a boundary here, then we continue with no change.
                    if self.norm16_has_comp_boundary_before(n16) {
                        if self.is_comp_yes_and_zero_cc(n16) {
                            src += char_count(c);
                        }
                        continue;
                    }
                    // Use the slow path. There is no boundary in [prevSrc, src[.
                }
            }

            // Slow path: Find the nearest boundaries around the current character,
            // decompose and recompose.
            if prev_boundary != prev_src && !self.norm16_has_comp_boundary_before(norm16) {
                c = code_point_before(s, prev_src);
                norm16 = self.trie().get(c);
                if !self.norm16_has_comp_boundary_after(norm16, only_contiguous) {
                    prev_src -= char_count(c);
                }
            }
            if do_compose && prev_boundary != prev_src {
                buffer.append_range(s, prev_boundary, prev_src);
            }
            let recompose_start_index = buffer.length();
            // We know there is not a boundary here.
            self.decompose_short(
                s,
                prev_src,
                src,
                false, /* !stopAtCompBoundary */
                only_contiguous,
                buffer,
            );
            // Decompose until the next boundary.
            src = self.decompose_short(
                s,
                src,
                limit,
                true, /* stopAtCompBoundary */
                only_contiguous,
                buffer,
            );
            self.recompose(buffer, recompose_start_index, only_contiguous);
            if !do_compose {
                if !buffer.equals(s, prev_src, src) {
                    return false;
                }
                buffer.remove();
            }
            prev_boundary = src;
        }
    }

    /// `composeQuickCheck(CharSequence s, int src, int limit, boolean
    /// onlyContiguous, boolean doSpan)`: the end of the "yes" prefix
    /// shifted left by 1, with bit 0 set for a "maybe" result.
    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    pub fn compose_quick_check(
        &self,
        s: &[u16],
        mut src: i32,
        limit: i32,
        only_contiguous: bool,
        do_span: bool,
    ) -> i32 {
        let mut qc_result = 0;
        let mut prev_boundary = src;
        let min_no_maybe_cp = self.min_comp_no_maybe_cp;

        loop {
            // Fast path: Scan over a sequence of characters below the minimum "no or maybe" code point,
            // or with (compYes && ccc==0) properties.
            let mut prev_src = src;
            let mut c;
            let mut norm16;
            loop {
                if src == limit {
                    return (src << 1) | qc_result; // "yes" or "maybe"
                }
                c = s[src as usize] as i32;
                norm16 = 0;
                if c < min_no_maybe_cp || {
                    norm16 = self.trie().bmp_get(c);
                    self.is_comp_yes_and_zero_cc(norm16)
                } {
                    src += 1;
                } else {
                    prev_src = src;
                    src += 1;
                    if !UTF16Plus::is_lead_surrogate(c) {
                        break;
                    } else if src != limit && is_low_surrogate(s[src as usize]) {
                        let c2 = s[src as usize];
                        src += 1;
                        c = to_code_point(c as u16, c2);
                        norm16 = self.trie().supp_get(c);
                        if !self.is_comp_yes_and_zero_cc(norm16) {
                            break;
                        }
                    }
                }
            }
            // isCompYesAndZeroCC(norm16) is false, that is, norm16>=minNoNo.
            // Check for an earlier boundary-after.
            let mut prev_norm16 = Self::INERT;
            if prev_boundary != prev_src {
                prev_boundary = prev_src;
                if !self.norm16_has_comp_boundary_before(norm16) {
                    c = code_point_before(s, prev_src);
                    let n16 = self.get_norm16(c);
                    if !self.norm16_has_comp_boundary_after(n16, only_contiguous) {
                        prev_boundary -= char_count(c);
                        prev_norm16 = n16;
                    }
                }
            }

            if norm16 >= self.min_maybe_no {
                // The current character is a combining mark or "maybe".
                // Check for canonical order.
                let mut fcd16 = self.get_fcd16_from_maybe_or_non_zero_cc(norm16);
                let mut cc = (fcd16 >> 8) & 0xff;
                if only_contiguous /* FCC */ && cc != 0 && self.get_trail_cc_from_comp_yes_and_zero_cc(prev_norm16) > cc
                {
                    // The [prevBoundary..prevSrc[ character
                    // passed the quick check "yes && ccc==0" test
                    // but is out of canonical order with the current combining mark.
                } else {
                    // If !onlyContiguous (not FCC), then we ignore the tccc of
                    // the previous character which passed the quick check "yes && ccc==0" test.
                    loop {
                        if norm16 < Self::MIN_YES_YES_WITH_CC {
                            if !do_span {
                                qc_result = 1;
                            } else {
                                return prev_boundary << 1; // spanYes does not care to know it's "maybe"
                            }
                        }
                        if src == limit {
                            return (src << 1) | qc_result; // "yes" or "maybe"
                        }
                        let prev_cc = fcd16 & 0xff;
                        c = code_point_at(s, src);
                        norm16 = self.get_norm16(c);
                        if norm16 >= self.min_maybe_no {
                            fcd16 = self.get_fcd16_from_maybe_or_non_zero_cc(norm16);
                            cc = (fcd16 >> 8) & 0xff;
                            if !(prev_cc <= cc || cc == 0) {
                                break;
                            }
                        } else {
                            break;
                        }
                        src += char_count(c);
                    }
                    // src is after the last in-order combining mark.
                    if self.is_comp_yes_and_zero_cc(norm16) {
                        prev_boundary = src;
                        src += char_count(c);
                        continue;
                    }
                }
            }
            return prev_boundary << 1; // "no"
        }
    }

    /// `composeAndAppend(CharSequence s, boolean doCompose, boolean
    /// onlyContiguous, ReorderingBuffer buffer)`.
    pub fn compose_and_append(
        &self,
        s: &[u16],
        do_compose: bool,
        only_contiguous: bool,
        buffer: &mut ReorderingBuffer<'_>,
    ) {
        let mut src = 0;
        let limit = s.len() as i32;
        if !buffer.is_empty() {
            let first_starter_in_src = self.find_next_comp_boundary(s, 0, limit, only_contiguous);
            if 0 != first_starter_in_src {
                let blen = buffer.length();
                let last_starter_in_dest = self.find_previous_comp_boundary(
                    buffer.get_string_builder(),
                    blen,
                    only_contiguous,
                );
                let mut middle: Vec<u16> = Vec::with_capacity(
                    ((blen - last_starter_in_dest) + first_starter_in_src + 16) as usize,
                );
                middle.extend_from_slice(
                    &buffer.get_string_builder()[last_starter_in_dest as usize..blen as usize],
                );
                buffer.remove_suffix(blen - last_starter_in_dest);
                middle.extend_from_slice(&s[..first_starter_in_src as usize]);
                self.compose(
                    &middle,
                    0,
                    middle.len() as i32,
                    only_contiguous,
                    true,
                    buffer,
                );
                src = first_starter_in_src;
            }
        }
        if do_compose {
            self.compose(s, src, limit, only_contiguous, true, buffer);
        } else {
            buffer.append_range(s, src, limit);
        }
    }

    // Dual functionality:
    // buffer!=NULL: normalize
    // buffer==NULL: isNormalized/quickCheck/spanQuickCheckYes
    /// `makeFCD(CharSequence s, int src, int limit, ReorderingBuffer
    /// buffer)`.
    pub fn make_fcd(
        &self,
        s: &[u16],
        mut src: i32,
        limit: i32,
        mut buffer: Option<&mut ReorderingBuffer<'_>>,
    ) -> i32 {
        // Note: In this function we use buffer->appendZeroCC() because we track
        // the lead and trail combining classes here, rather than leaving it to
        // the ReorderingBuffer.
        // The exception is the call to decomposeShort() which uses the buffer
        // in the normal way.

        // Tracks the last FCD-safe boundary, before lccc=0 or after properly-ordered tccc<=1.
        // Similar to the prevBoundary in the compose() implementation.
        let mut prev_boundary = src;
        let mut prev_src;
        let mut c = 0;
        let mut prev_fcd16 = 0;
        let mut fcd16 = 0;

        loop {
            // count code units with lccc==0
            prev_src = src;
            while src != limit {
                c = s[src as usize] as i32;
                if c < self.min_lccc_cp {
                    prev_fcd16 = !c;
                    src += 1;
                } else if !self.single_lead_might_have_non_zero_fcd16(c) {
                    prev_fcd16 = 0;
                    src += 1;
                } else {
                    if UTF16Plus::is_lead_surrogate(c)
                        && (src + 1) != limit
                        && is_low_surrogate(s[src as usize + 1])
                    {
                        c = to_code_point(c as u16, s[src as usize + 1]);
                    }
                    fcd16 = self.get_fcd16_from_norm_data(c);
                    if fcd16 <= 0xff {
                        prev_fcd16 = fcd16;
                        src += char_count(c);
                    } else {
                        break;
                    }
                }
            }
            // copy these code units all at once
            if src != prev_src {
                if src == limit {
                    if let Some(b) = buffer.as_deref_mut() {
                        b.flush_and_append_zero_cc(s, prev_src, src);
                    }
                    break;
                }
                prev_boundary = src;
                // We know that the previous character's lccc==0.
                if prev_fcd16 < 0 {
                    // Fetching the fcd16 value was deferred for this below-minLcccCP code point.
                    let prev = !prev_fcd16;
                    if prev < self.min_decomp_no_cp {
                        prev_fcd16 = 0;
                    } else {
                        prev_fcd16 = self.get_fcd16_from_norm_data(prev);
                        if prev_fcd16 > 1 {
                            prev_boundary -= 1;
                        }
                    }
                } else {
                    let mut p = src - 1;
                    if is_low_surrogate(s[p as usize])
                        && prev_src < p
                        && is_high_surrogate(s[p as usize - 1])
                    {
                        p -= 1;
                        // Need to fetch the previous character's FCD value because
                        // prevFCD16 was just for the trail surrogate code point.
                        prev_fcd16 = self.get_fcd16_from_norm_data(to_code_point(
                            s[p as usize],
                            s[p as usize + 1],
                        ));
                        // Still known to have lccc==0 because its lead surrogate unit had lccc==0.
                    }
                    if prev_fcd16 > 1 {
                        prev_boundary = p;
                    }
                }
                if let Some(b) = buffer.as_deref_mut() {
                    // The last lccc==0 character is excluded from the
                    // flush-and-append call in case it needs to be modified.
                    b.flush_and_append_zero_cc(s, prev_src, prev_boundary);
                    b.append_range(s, prev_boundary, src);
                }
                // The start of the current character (c).
                prev_src = src;
            } else if src == limit {
                break;
            }

            src += char_count(c);
            // The current character (c) at [prevSrc..src[ has a non-zero lead combining class.
            // Check for proper order, and decompose locally if necessary.
            if (prev_fcd16 & 0xff) <= (fcd16 >> 8) {
                // proper order: prev tccc <= current lccc
                if (fcd16 & 0xff) <= 1 {
                    prev_boundary = src;
                }
                if let Some(b) = buffer.as_deref_mut() {
                    b.append_zero_cc(c);
                }
                prev_fcd16 = fcd16;
                continue;
            } else if buffer.is_none() {
                return prev_boundary; // quick check "no"
            } else {
                let b = buffer.as_deref_mut().expect("checked above");
                /*
                 * Back out the part of the source that we copied or appended
                 * already but is now going to be decomposed.
                 * prevSrc is set to after what was copied/appended.
                 */
                b.remove_suffix(prev_src - prev_boundary);
                /*
                 * Find the part of the source that needs to be decomposed,
                 * up to the next safe boundary.
                 */
                src = self.find_next_fcd_boundary(s, src, limit);
                /*
                 * The source text does not fulfill the conditions for FCD.
                 * Decompose and reorder a limited piece of the text.
                 */
                self.decompose_short(s, prev_boundary, src, false, false, b);
                prev_boundary = src;
                prev_fcd16 = 0;
            }
        }
        src
    }

    /// `makeFCDAndAppend(CharSequence s, boolean doMakeFCD,
    /// ReorderingBuffer buffer)`.
    pub fn make_fcd_and_append(
        &self,
        s: &[u16],
        do_make_fcd: bool,
        buffer: &mut ReorderingBuffer<'_>,
    ) {
        let mut src = 0;
        let limit = s.len() as i32;
        if !buffer.is_empty() {
            let first_boundary_in_src = self.find_next_fcd_boundary(s, 0, limit);
            if 0 != first_boundary_in_src {
                let blen = buffer.length();
                let last_boundary_in_dest =
                    self.find_previous_fcd_boundary(buffer.get_string_builder(), blen);
                let mut middle: Vec<u16> = Vec::with_capacity(
                    ((blen - last_boundary_in_dest) + first_boundary_in_src + 16) as usize,
                );
                middle.extend_from_slice(
                    &buffer.get_string_builder()[last_boundary_in_dest as usize..blen as usize],
                );
                buffer.remove_suffix(blen - last_boundary_in_dest);
                middle.extend_from_slice(&s[..first_boundary_in_src as usize]);
                self.make_fcd(&middle, 0, middle.len() as i32, Some(buffer));
                src = first_boundary_in_src;
            }
        }
        if do_make_fcd {
            self.make_fcd(s, src, limit, Some(buffer));
        } else {
            buffer.append_range(s, src, limit);
        }
    }

    /// `hasDecompBoundaryBefore(int c)`.
    pub fn has_decomp_boundary_before(&self, c: i32) -> bool {
        c < self.min_lccc_cp
            || (c <= 0xffff && !self.single_lead_might_have_non_zero_fcd16(c))
            || self.norm16_has_decomp_boundary_before(self.get_norm16(c))
    }
    /// `norm16HasDecompBoundaryBefore(int norm16)`.
    pub fn norm16_has_decomp_boundary_before(&self, norm16: i32) -> bool {
        if norm16 < self.min_no_no_comp_no_maybe_cc {
            return true;
        }
        if norm16 >= self.limit_no_no {
            return norm16 <= Self::MIN_NORMAL_MAYBE_YES || norm16 == Self::JAMO_VT;
        }
        // c decomposes, get everything from the variable-length extra data
        let mapping = self.get_data_for_yes_or_no(norm16);
        let first_unit = self.ed(mapping);
        // true if leadCC==0 (hasFCDBoundaryBefore())
        (first_unit & Self::MAPPING_HAS_CCC_LCCC_WORD) == 0 || (self.ed(mapping - 1) & 0xff00) == 0
    }
    /// `hasDecompBoundaryAfter(int c)`.
    pub fn has_decomp_boundary_after(&self, c: i32) -> bool {
        if c < self.min_decomp_no_cp {
            return true;
        }
        if c <= 0xffff && !self.single_lead_might_have_non_zero_fcd16(c) {
            return true;
        }
        self.norm16_has_decomp_boundary_after(self.get_norm16(c))
    }
    /// `norm16HasDecompBoundaryAfter(int norm16)`.
    pub fn norm16_has_decomp_boundary_after(&self, norm16: i32) -> bool {
        if norm16 <= self.min_yes_no || self.is_hangul_lvt(norm16) {
            return true;
        }
        if norm16 >= self.limit_no_no {
            if self.is_maybe_yes_or_non_zero_cc(norm16) {
                return norm16 <= Self::MIN_NORMAL_MAYBE_YES || norm16 == Self::JAMO_VT;
            } else if norm16 < self.min_maybe_no {
                // Maps to an isCompYesAndZeroCC.
                return (norm16 & Self::DELTA_TCCC_MASK) <= Self::DELTA_TCCC_1;
            }
        }
        // c decomposes, get everything from the variable-length extra data
        let mapping = self.get_data(norm16);
        let first_unit = self.ed(mapping);
        // decomp after-boundary: same as hasFCDBoundaryAfter(),
        // fcd16<=1 || trailCC==0
        if first_unit > 0x1ff {
            return false; // trailCC>1
        }
        if first_unit <= 0xff {
            return true; // trailCC==0
        }
        // if(trailCC==1) test leadCC==0, same as checking for before-boundary
        // true if leadCC==0 (hasFCDBoundaryBefore())
        (first_unit & Self::MAPPING_HAS_CCC_LCCC_WORD) == 0 || (self.ed(mapping - 1) & 0xff00) == 0
    }
    /// `isDecompInert(int c)`.
    pub fn is_decomp_inert(&self, c: i32) -> bool {
        self.is_decomp_yes_and_zero_cc(self.get_norm16(c))
    }

    /// `hasCompBoundaryBefore(int c)`.
    pub fn has_comp_boundary_before(&self, c: i32) -> bool {
        c < self.min_comp_no_maybe_cp || self.norm16_has_comp_boundary_before(self.get_norm16(c))
    }
    /// `hasCompBoundaryAfter(int c, boolean onlyContiguous)`.
    pub fn has_comp_boundary_after(&self, c: i32, only_contiguous: bool) -> bool {
        self.norm16_has_comp_boundary_after(self.get_norm16(c), only_contiguous)
    }
    /// `isCompInert(int c, boolean onlyContiguous)`.
    pub fn is_comp_inert(&self, c: i32, only_contiguous: bool) -> bool {
        let norm16 = self.get_norm16(c);
        self.is_comp_yes_and_zero_cc(norm16)
            && (norm16 & Self::HAS_COMP_BOUNDARY_AFTER) != 0
            && (!only_contiguous
                || Self::is_inert(norm16)
                || self.ed(self.get_data_for_yes_or_no(norm16)) <= 0x1ff)
        // The last check fetches the mapping's first unit and checks tccc<=1.
    }

    /// `hasFCDBoundaryBefore(int c)`.
    pub fn has_fcd_boundary_before(&self, c: i32) -> bool {
        self.has_decomp_boundary_before(c)
    }
    /// `hasFCDBoundaryAfter(int c)`.
    pub fn has_fcd_boundary_after(&self, c: i32) -> bool {
        self.has_decomp_boundary_after(c)
    }
    /// `isFCDInert(int c)`.
    pub fn is_fcd_inert(&self, c: i32) -> bool {
        self.get_fcd16(c) <= 1
    }

    fn is_maybe(&self, norm16: i32) -> bool {
        self.min_maybe_no <= norm16 && norm16 <= Self::JAMO_VT
    }
    fn is_maybe_yes_or_non_zero_cc(&self, norm16: i32) -> bool {
        norm16 >= self.min_maybe_yes
    }
    fn is_inert(norm16: i32) -> bool {
        norm16 == Self::INERT
    }
    fn is_jamo_l(norm16: i32) -> bool {
        norm16 == Self::JAMO_L
    }
    fn is_jamo_vt(norm16: i32) -> bool {
        norm16 == Self::JAMO_VT
    }
    fn hangul_lvt(&self) -> i32 {
        self.min_yes_no_mappings_only | Self::HAS_COMP_BOUNDARY_AFTER
    }
    fn is_hangul_lv(&self, norm16: i32) -> bool {
        norm16 == self.min_yes_no
    }
    fn is_hangul_lvt(&self, norm16: i32) -> bool {
        norm16 == self.hangul_lvt()
    }
    fn is_comp_yes_and_zero_cc(&self, norm16: i32) -> bool {
        norm16 < self.min_no_no
    }
    // UBool isCompYes(uint16_t norm16) const {
    //     return norm16>=MIN_YES_YES_WITH_CC || norm16<minNoNo;
    // }
    // UBool isCompYesOrMaybe(uint16_t norm16) const {
    //     return norm16<minNoNo || minMaybeNo<=norm16;
    // }
    // private boolean hasZeroCCFromDecompYes(int norm16) {
    //     return norm16<=MIN_NORMAL_MAYBE_YES || norm16==JAMO_VT;
    // }
    fn is_decomp_yes_and_zero_cc(&self, norm16: i32) -> bool {
        norm16 < self.min_yes_no
            || norm16 == Self::JAMO_VT
            || (self.min_maybe_yes <= norm16 && norm16 <= Self::MIN_NORMAL_MAYBE_YES)
    }
    /// A little faster and simpler than isDecompYesAndZeroCC() but does not
    /// include the MaybeYes which combine-forward and have ccc=0.
    fn is_most_decomp_yes_and_zero_cc(&self, norm16: i32) -> bool {
        norm16 < self.min_yes_no || norm16 == Self::MIN_NORMAL_MAYBE_YES || norm16 == Self::JAMO_VT
    }
    /// Since formatVersion 5: same as isAlgorithmicNoNo()
    fn is_decomp_no_algorithmic(&self, norm16: i32) -> bool {
        self.limit_no_no <= norm16 && norm16 < self.min_maybe_no
    }

    // For use with isCompYes().
    // Perhaps the compiler can combine the two tests for MIN_YES_YES_WITH_CC.
    // static uint8_t getCCFromYes(uint16_t norm16) {
    //     return norm16>=MIN_YES_YES_WITH_CC ? getCCFromNormalYesOrMaybe(norm16) : 0;
    // }
    fn get_cc_from_no_no(&self, norm16: i32) -> i32 {
        let mapping = self.get_data_for_yes_or_no(norm16);
        if (self.ed(mapping) & Self::MAPPING_HAS_CCC_LCCC_WORD) != 0 {
            self.ed(mapping - 1) & 0xff
        } else {
            0
        }
    }
    /// `getTrailCCFromCompYesAndZeroCC(int norm16)` (package-private).
    pub(crate) fn get_trail_cc_from_comp_yes_and_zero_cc(&self, norm16: i32) -> i32 {
        if norm16 <= self.min_yes_no {
            0 // yesYes and Hangul LV have ccc=tccc=0
        } else {
            // For Hangul LVT we harmlessly fetch a firstUnit with tccc=0 here.
            self.ed(self.get_data_for_yes_or_no(norm16)) >> 8 // tccc from yesNo
        }
    }

    // Requires algorithmic-NoNo.
    fn map_algorithmic(&self, c: i32, norm16: i32) -> i32 {
        c + (norm16 >> Self::DELTA_SHIFT) - self.center_no_no_delta
    }

    // Requires minYesNo<norm16<limitNoNo.
    // private int getMapping(int norm16) { return extraData+(norm16>>OFFSET_SHIFT); }
    fn get_data_for_yes_or_no(&self, norm16: i32) -> i32 {
        norm16 >> Self::OFFSET_SHIFT
    }
    fn get_data_for_maybe(&self, norm16: i32) -> i32 {
        (norm16 - self.min_maybe_no + self.limit_no_no) >> Self::OFFSET_SHIFT
    }
    fn get_data(&self, mut norm16: i32) -> i32 {
        if norm16 >= self.min_maybe_no {
            norm16 = norm16 - self.min_maybe_no + self.limit_no_no;
        }
        norm16 >> Self::OFFSET_SHIFT
    }

    /// @return index into maybeYesCompositions, or -1
    fn get_compositions_list_for_decomp_yes(&self, norm16: i32) -> i32 {
        if !(Self::JAMO_L..Self::MIN_NORMAL_MAYBE_YES).contains(&norm16) {
            -1
        } else {
            // if yesYes: if Jamo L: harmless empty list
            self.get_data(norm16)
        }
    }
    /// @return index into maybeYesCompositions
    fn get_compositions_list_for_composite(&self, norm16: i32) -> i32 {
        // A composite has both mapping & compositions list.
        let list = self.get_data(norm16);
        let first_unit = self.ed(list);
        list + // mapping in maybeYesCompositions
            1 + // +1 to skip the first unit with the mapping length
            (first_unit & Self::MAPPING_LENGTH_MASK) // + mapping length
    }

    /// @param c code point must have compositions
    /// @return index into maybeYesCompositions
    fn get_compositions_list(&self, norm16: i32) -> i32 {
        if self.is_decomp_yes(norm16) {
            self.get_compositions_list_for_decomp_yes(norm16)
        } else {
            self.get_compositions_list_for_composite(norm16)
        }
    }

    // Decompose a short piece of text which is likely to contain characters that
    // fail the quick check loop and/or where the quick check loop's overhead
    // is unlikely to be amortized.
    // Called by the compose() and makeFCD() implementations.
    // Public in Java for collation implementation code.
    fn decompose_short(
        &self,
        s: &[u16],
        mut src: i32,
        limit: i32,
        stop_at_comp_boundary: bool,
        only_contiguous: bool,
        buffer: &mut ReorderingBuffer<'_>,
    ) -> i32 {
        while src < limit {
            let c = code_point_at(s, src);
            if stop_at_comp_boundary && c < self.min_comp_no_maybe_cp {
                return src;
            }
            let norm16 = self.get_norm16(c);
            if stop_at_comp_boundary && self.norm16_has_comp_boundary_before(norm16) {
                return src;
            }
            src += char_count(c);
            self.decompose_cp(c, norm16, buffer);
            if stop_at_comp_boundary && self.norm16_has_comp_boundary_after(norm16, only_contiguous)
            {
                return src;
            }
        }
        src
    }

    /// `decompose(int c, int norm16, ReorderingBuffer buffer)` (private).
    fn decompose_cp(&self, mut c: i32, mut norm16: i32, buffer: &mut ReorderingBuffer<'_>) {
        // get the decomposition and the lead and trail cc's
        if norm16 >= self.limit_no_no {
            if self.is_maybe_yes_or_non_zero_cc(norm16) {
                buffer.append(c, Self::get_cc_from_yes_or_maybe_yes(norm16));
                return;
            } else if norm16 < self.min_maybe_no {
                // Maps to an isCompYesAndZeroCC.
                c = self.map_algorithmic(c, norm16);
                norm16 = self.get_raw_norm16(c);
            }
        }
        if norm16 < self.min_yes_no {
            // c does not decompose
            buffer.append(c, 0);
        } else if self.is_hangul_lv(norm16) || self.is_hangul_lvt(norm16) {
            // Hangul syllable: decompose algorithmically
            Hangul::decompose(c, buffer);
        } else {
            // c decomposes, get everything from the variable-length extra data
            let mut mapping = self.get_data(norm16);
            let first_unit = self.ed(mapping);
            let length = first_unit & Self::MAPPING_LENGTH_MASK;
            let trail_cc = first_unit >> 8;
            let lead_cc = if (first_unit & Self::MAPPING_HAS_CCC_LCCC_WORD) != 0 {
                self.ed(mapping - 1) >> 8
            } else {
                0
            };
            mapping += 1; // skip over the firstUnit
            buffer.append_cc(
                &self.extra_data,
                mapping,
                mapping + length,
                true,
                lead_cc,
                trail_cc,
            );
        }
    }

    /// Finds the recomposition result for
    /// a forward-combining "lead" character,
    /// specified with a pointer to its compositions list,
    /// and a backward-combining "trail" character.
    ///
    /// If the lead and trail characters combine, then this function returns
    /// the following "compositeAndFwd" value:
    /// Bits 21..1  composite character
    /// Bit      0  set if the composite is a forward-combining starter
    /// otherwise it returns -1.
    fn combine(&self, mut list: i32, trail: i32) -> i32 {
        let key1;
        let mut first_unit;
        if trail < Self::COMP_1_TRAIL_LIMIT {
            // trail character is 0..33FF
            // result entry may have 2 or 3 units
            key1 = trail << 1;
            loop {
                first_unit = self.ed(list);
                if key1 > first_unit {
                    list += 2 + (first_unit & Self::COMP_1_TRIPLE);
                } else {
                    break;
                }
            }
            if key1 == (first_unit & Self::COMP_1_TRAIL_MASK) {
                if (first_unit & Self::COMP_1_TRIPLE) != 0 {
                    return (self.ed(list + 1) << 16) | self.ed(list + 2);
                } else {
                    return self.ed(list + 1);
                }
            }
        } else {
            // trail character is 3400..10FFFF
            // result entry has 3 units
            key1 = Self::COMP_1_TRAIL_LIMIT
                + ((trail >> Self::COMP_1_TRAIL_SHIFT) & !Self::COMP_1_TRIPLE);
            let key2 = (trail << Self::COMP_2_TRAIL_SHIFT) & 0xffff;
            loop {
                first_unit = self.ed(list);
                if key1 > first_unit {
                    list += 2 + (first_unit & Self::COMP_1_TRIPLE);
                } else if key1 == (first_unit & Self::COMP_1_TRAIL_MASK) {
                    let second_unit = self.ed(list + 1);
                    if key2 > second_unit {
                        if (first_unit & Self::COMP_1_LAST_TUPLE) != 0 {
                            break;
                        } else {
                            list += 3;
                        }
                    } else if key2 == (second_unit & Self::COMP_2_TRAIL_MASK) {
                        return ((second_unit & !Self::COMP_2_TRAIL_MASK) << 16)
                            | self.ed(list + 2);
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        -1
    }

    /// Adds the composites of the compositions list to the set.
    fn add_composites(&self, mut list: i32, set: &mut UnicodeSet) -> Result<()> {
        let mut first_unit;
        let mut composite_and_fwd;
        loop {
            first_unit = self.ed(list);
            if (first_unit & Self::COMP_1_TRIPLE) == 0 {
                composite_and_fwd = self.ed(list + 1);
                list += 2;
            } else {
                composite_and_fwd =
                    ((self.ed(list + 1) & !Self::COMP_2_TRAIL_MASK) << 16) | self.ed(list + 2);
                list += 3;
            }
            let composite = composite_and_fwd >> 1;
            if (composite_and_fwd & 1) != 0 {
                self.add_composites(
                    self.get_compositions_list_for_composite(self.get_raw_norm16(composite)),
                    set,
                )?;
            }
            set.add(composite)?;
            if (first_unit & Self::COMP_1_LAST_TUPLE) != 0 {
                break;
            }
        }
        Ok(())
    }

    /*
     * Recomposes the buffer text starting at recomposeStartIndex
     * (which is in NFD - decomposed and canonically ordered),
     * and truncates the buffer contents.
     *
     * Note that recomposition never lengthens the text:
     * Any character consists of either one or two code units;
     * a composition may contain at most one more code unit than the original starter,
     * while the combining mark that is removed has at least one code unit.
     */
    fn recompose(
        &self,
        buffer: &mut ReorderingBuffer<'_>,
        recompose_start_index: i32,
        only_contiguous: bool,
    ) {
        {
            let sb = buffer.get_string_builder();
            let mut p = recompose_start_index;
            if p == sb.len() as i32 {
                return;
            }

            let mut starter;
            let mut p_remove;
            let mut compositions_list;
            let mut c;
            let mut composite_and_fwd;
            let mut norm16;
            let mut cc;
            let mut prev_cc;
            let mut starter_is_supplementary;

            // Some of the following variables are not used until we have a forward-combining starter
            // and are only initialized now to avoid compiler warnings.
            compositions_list = -1; // used as indicator for whether we have a forward-combining starter
            starter = -1;
            starter_is_supplementary = false;
            prev_cc = 0;

            loop {
                c = code_point_at(sb, p);
                p += char_count(c);
                norm16 = self.get_norm16(c);
                cc = Self::get_cc_from_yes_or_maybe_yes(norm16);
                if
                // this character combines backward and
                self.is_maybe(norm16) &&
                    // we have seen a starter that combines forward and
                    compositions_list >= 0 &&
                    // the backward-combining character is not blocked
                    (prev_cc < cc || prev_cc == 0)
                {
                    if Self::is_jamo_vt(norm16) {
                        // c is a Jamo V/T, see if we can compose it with the previous character.
                        if c < Hangul::JAMO_T_BASE {
                            // c is a Jamo Vowel, compose with previous Jamo L and following Jamo T.
                            let prev =
                                sb[starter as usize].wrapping_sub(Hangul::JAMO_L_BASE as u16);
                            if (prev as i32) < Hangul::JAMO_L_COUNT {
                                p_remove = p - 1;
                                let mut syllable = (Hangul::HANGUL_BASE
                                    + (prev as i32 * Hangul::JAMO_V_COUNT
                                        + (c - Hangul::JAMO_V_BASE))
                                        * Hangul::JAMO_T_COUNT)
                                    as u16;
                                if p != sb.len() as i32 {
                                    let t = sb[p as usize].wrapping_sub(Hangul::JAMO_T_BASE as u16);
                                    if (t as i32) < Hangul::JAMO_T_COUNT {
                                        // The next character is a Jamo T.
                                        p += 1;
                                        syllable = syllable.wrapping_add(t);
                                    }
                                }
                                sb[starter as usize] = syllable;
                                // remove the Jamo V/T
                                sb.drain(p_remove as usize..p as usize);
                                p = p_remove;
                            }
                        }
                        /*
                         * No "else" for Jamo T:
                         * Since the input is in NFD, there are no Hangul LV syllables that
                         * a Jamo T could combine with.
                         * All Jamo Ts are combined above when handling Jamo Vs.
                         */
                        if p == sb.len() as i32 {
                            break;
                        }
                        compositions_list = -1;
                        continue;
                    } else {
                        composite_and_fwd = self.combine(compositions_list, c);
                        if composite_and_fwd >= 0 {
                            // The starter and the combining mark (c) do combine.
                            let composite = composite_and_fwd >> 1;

                            // Remove the combining mark.
                            p_remove = p - char_count(c); // pRemove & p: start & limit of the combining mark
                            sb.drain(p_remove as usize..p as usize);
                            p = p_remove;
                            // Replace the starter with the composite.
                            if starter_is_supplementary {
                                if composite > 0xffff {
                                    // both are supplementary
                                    sb[starter as usize] = UTF16::get_lead_surrogate(composite);
                                    sb[starter as usize + 1] =
                                        UTF16::get_trail_surrogate(composite);
                                } else {
                                    sb[starter as usize] = c as u16;
                                    sb.remove(starter as usize + 1);
                                    // The composite is shorter than the starter,
                                    // move the intermediate characters forward one.
                                    starter_is_supplementary = false;
                                    p -= 1;
                                }
                            } else if composite > 0xffff {
                                // The composite is longer than the starter,
                                // move the intermediate characters back one.
                                starter_is_supplementary = true;
                                sb[starter as usize] = UTF16::get_lead_surrogate(composite);
                                sb.insert(
                                    starter as usize + 1,
                                    UTF16::get_trail_surrogate(composite),
                                );
                                p += 1;
                            } else {
                                // both are on the BMP
                                sb[starter as usize] = composite as u16;
                            }

                            // Keep prevCC because we removed the combining mark.

                            if p == sb.len() as i32 {
                                break;
                            }
                            // Is the composite a starter that combines forward?
                            if (composite_and_fwd & 1) != 0 {
                                compositions_list = self.get_compositions_list_for_composite(
                                    self.get_raw_norm16(composite),
                                );
                            } else {
                                compositions_list = -1;
                            }

                            // We combined; continue with looking for compositions.
                            continue;
                        }
                    }
                }

                // no combination this time
                prev_cc = cc;
                if p == sb.len() as i32 {
                    break;
                }

                // If c did not combine, then check if it is a starter.
                if cc == 0 {
                    // Found a new starter.
                    compositions_list = self.get_compositions_list_for_decomp_yes(norm16);
                    if compositions_list >= 0 {
                        // It may combine with something, prepare for it.
                        if c <= 0xffff {
                            starter_is_supplementary = false;
                            starter = p - 1;
                        } else {
                            starter_is_supplementary = true;
                            starter = p - 2;
                        }
                    }
                } else if only_contiguous {
                    // FCC: no discontiguous compositions; any intervening character blocks.
                    compositions_list = -1;
                }
            }
        }
        buffer.flush();
    }

    /// `composePair(int a, int b)`: the composite of a and b, -1 when none.
    pub fn compose_pair(&self, a: i32, mut b: i32) -> i32 {
        let norm16 = self.get_norm16(a); // maps an out-of-range 'a' to inert norm16
        let mut list;
        if Self::is_inert(norm16) {
            return -1;
        } else if norm16 < self.min_yes_no_mappings_only {
            // a combines forward.
            if Self::is_jamo_l(norm16) {
                b -= Hangul::JAMO_V_BASE;
                if (0..Hangul::JAMO_V_COUNT).contains(&b) {
                    return Hangul::HANGUL_BASE
                        + ((a - Hangul::JAMO_L_BASE) * Hangul::JAMO_V_COUNT + b)
                            * Hangul::JAMO_T_COUNT;
                } else {
                    return -1;
                }
            } else if self.is_hangul_lv(norm16) {
                b -= Hangul::JAMO_T_BASE;
                if 0 < b && b < Hangul::JAMO_T_COUNT {
                    // not b==0!
                    return a + b;
                } else {
                    return -1;
                }
            } else {
                // 'a' has a compositions list in extraData
                list = self.get_data_for_yes_or_no(norm16);
                if norm16 > self.min_yes_no {
                    // composite 'a' has both mapping & compositions list
                    list += 1 + // +1 to skip the first unit with the mapping length
                        (self.ed(list) & Self::MAPPING_LENGTH_MASK); // + mapping length
                }
            }
        } else if norm16 < self.min_maybe_no_combines_fwd || Self::MIN_NORMAL_MAYBE_YES <= norm16 {
            return -1;
        } else {
            list = self.get_data_for_maybe(norm16);
            if norm16 < self.min_maybe_yes {
                // composite 'a' has both mapping & compositions list
                list += 1 + // +1 to skip the first unit with the mapping length
                    (self.ed(list) & Self::MAPPING_LENGTH_MASK); // + mapping length
            }
        }
        if !(0..=0x10ffff).contains(&b) {
            // combine(list, b) requires a valid code point b
            return -1;
        }
        self.combine(list, b) >> 1
    }

    /// Does c have a composition boundary before it?
    /// True if its decomposition begins with a character that has
    /// ccc=0 && NFC_QC=Yes (isCompYesAndZeroCC()).
    /// As a shortcut, this is true if c itself has ccc=0 && NFC_QC=Yes
    /// (isCompYesAndZeroCC()) so we need not decompose.
    fn has_comp_boundary_before_cp(&self, c: i32, norm16: i32) -> bool {
        c < self.min_comp_no_maybe_cp || self.norm16_has_comp_boundary_before(norm16)
    }
    fn norm16_has_comp_boundary_before(&self, norm16: i32) -> bool {
        norm16 < self.min_no_no_comp_no_maybe_cc || self.is_algorithmic_no_no(norm16)
    }
    fn has_comp_boundary_before_at(&self, s: &[u16], src: i32, limit: i32) -> bool {
        src == limit || self.has_comp_boundary_before(code_point_at(s, src))
    }
    fn norm16_has_comp_boundary_after(&self, norm16: i32, only_contiguous: bool) -> bool {
        (norm16 & Self::HAS_COMP_BOUNDARY_AFTER) != 0
            && (!only_contiguous || self.is_trail_cc01_for_comp_boundary_after(norm16))
    }
    fn has_comp_boundary_after_range(
        &self,
        s: &[u16],
        start: i32,
        p: i32,
        only_contiguous: bool,
    ) -> bool {
        start == p || self.has_comp_boundary_after(code_point_before(s, p), only_contiguous)
    }
    /// For FCC: Given norm16 HAS_COMP_BOUNDARY_AFTER, does it have tccc<=1?
    fn is_trail_cc01_for_comp_boundary_after(&self, norm16: i32) -> bool {
        Self::is_inert(norm16)
            || (if self.is_decomp_no_algorithmic(norm16) {
                (norm16 & Self::DELTA_TCCC_MASK) <= Self::DELTA_TCCC_1
            } else {
                self.ed(self.get_data_for_yes_or_no(norm16)) <= 0x1ff
            })
    }

    fn find_previous_comp_boundary(&self, s: &[u16], mut p: i32, only_contiguous: bool) -> i32 {
        while p > 0 {
            let c = code_point_before(s, p);
            let norm16 = self.get_norm16(c);
            if self.norm16_has_comp_boundary_after(norm16, only_contiguous) {
                break;
            }
            p -= char_count(c);
            if self.has_comp_boundary_before_cp(c, norm16) {
                break;
            }
        }
        p
    }
    fn find_next_comp_boundary(
        &self,
        s: &[u16],
        mut p: i32,
        limit: i32,
        only_contiguous: bool,
    ) -> i32 {
        while p < limit {
            let c = code_point_at(s, p);
            let norm16 = self.trie().get(c);
            if self.has_comp_boundary_before_cp(c, norm16) {
                break;
            }
            p += char_count(c);
            if self.norm16_has_comp_boundary_after(norm16, only_contiguous) {
                break;
            }
        }
        p
    }

    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    fn find_previous_fcd_boundary(&self, s: &[u16], mut p: i32) -> i32 {
        while p > 0 {
            let c = code_point_before(s, p);
            let mut norm16 = 0;
            if c < self.min_decomp_no_cp || {
                norm16 = self.get_norm16(c);
                self.norm16_has_decomp_boundary_after(norm16)
            } {
                break;
            }
            p -= char_count(c);
            if self.norm16_has_decomp_boundary_before(norm16) {
                break;
            }
        }
        p
    }
    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    fn find_next_fcd_boundary(&self, s: &[u16], mut p: i32, limit: i32) -> i32 {
        while p < limit {
            let c = code_point_at(s, p);
            let mut norm16 = 0;
            if c < self.min_lccc_cp || {
                norm16 = self.get_norm16(c);
                self.norm16_has_decomp_boundary_before(norm16)
            } {
                break;
            }
            p += char_count(c);
            if self.norm16_has_decomp_boundary_after(norm16) {
                break;
            }
        }
        p
    }

    fn get_previous_trail_cc(&self, s: &[u16], start: i32, p: i32) -> i32 {
        if start == p {
            return 0;
        }
        self.get_fcd16(code_point_before(s, p))
    }

    fn add_to_start_set(
        mutable_trie: &mut MutableCodePointTrie,
        canon_start_sets: &mut Vec<UnicodeSet>,
        origin: i32,
        decomp_lead: i32,
    ) -> Result<()> {
        let mut canon_value = mutable_trie.get(decomp_lead);
        if (canon_value & (CANON_HAS_SET | CANON_VALUE_MASK)) == 0 && origin != 0 {
            // origin is the first character whose decomposition starts with
            // the character for which we are setting the value.
            mutable_trie.set(decomp_lead, canon_value | origin)?;
        } else {
            // origin is not the first character, or it is U+0000.
            let set;
            if (canon_value & CANON_HAS_SET) == 0 {
                let first_origin = canon_value & CANON_VALUE_MASK;
                canon_value = (canon_value & !CANON_VALUE_MASK)
                    | CANON_HAS_SET
                    | canon_start_sets.len() as i32;
                mutable_trie.set(decomp_lead, canon_value)?;
                canon_start_sets.push(UnicodeSet::new());
                set = canon_start_sets.last_mut().expect("pushed");
                if first_origin != 0 {
                    set.add(first_origin)?;
                }
            } else {
                set = &mut canon_start_sets[(canon_value & CANON_VALUE_MASK) as usize];
            }
            set.add(origin)?;
        }
        Ok(())
    }

    /// The data version (`dataVersion`, private in Java).
    #[allow(dead_code)]
    fn data_version(&self) -> Option<&VersionInfo> {
        self.data_version.as_ref()
    }
}

/// UTF-16 helpers for the normalization code.
///
/// Equivalent to `com.ibm.icu.impl.Normalizer2Impl.UTF16Plus`.
pub struct UTF16Plus;

impl UTF16Plus {
    /// `isLeadSurrogate(int c)`: whether the code unit or code point is a
    /// lead surrogate.
    pub fn is_lead_surrogate(c: i32) -> bool {
        (c & 0xfffffc00_u32 as i32) == 0xd800
    }

    /// `isTrailSurrogate(int c)`.
    pub fn is_trail_surrogate(c: i32) -> bool {
        (c & 0xfffffc00_u32 as i32) == 0xdc00
    }

    /// `isSurrogate(int c)`.
    pub fn is_surrogate(c: i32) -> bool {
        (c & 0xfffff800_u32 as i32) == 0xd800
    }

    /// `isSurrogateLead(int c)`: assuming `c` is a surrogate, whether it is
    /// a lead surrogate.
    pub fn is_surrogate_lead(c: i32) -> bool {
        (c & 0x400) == 0
    }

    /// `equal(CharSequence s1, CharSequence s2)`: equal units.
    pub fn equal(s1: &[u16], s2: &[u16]) -> bool {
        if std::ptr::eq(s1, s2) {
            return true;
        }
        let length = s1.len();
        if length != s2.len() {
            return false;
        }
        for i in 0..length {
            if s1[i] != s2[i] {
                return false;
            }
        }
        true
    }

    /// `equal(CharSequence s1, int start1, int limit1, CharSequence s2, int
    /// start2, int limit2)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when a range is outside its text.
    pub fn equal_range(
        s1: &[u16],
        mut start1: i32,
        limit1: i32,
        s2: &[u16],
        mut start2: i32,
        limit2: i32,
    ) -> bool {
        if (limit1 - start1) != (limit2 - start2) {
            return false;
        }
        if std::ptr::eq(s1, s2) && start1 == start2 {
            return true;
        }
        while start1 < limit1 {
            let a = s1[start1 as usize];
            start1 += 1;
            let b = s2[start2 as usize];
            start2 += 1;
            if a != b {
                return false;
            }
        }
        true
    }
}
