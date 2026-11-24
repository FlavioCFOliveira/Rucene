// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationKeys`.

use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_iterator::CollationIterator;
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::text::collator;

/// The state of the abstract class `CollationKeys.SortKeyByteSink`.
///
/// Java porting note: C++ SortKeyByteSink class extends a common class
/// ByteSink, which is not available in Java.
#[derive(Debug, Clone, Default)]
pub struct SortKeyByteSinkBase {
    /// `buffer_`; its length is the capacity.
    pub buffer: Vec<i8>,
    appended: i32,
}

impl SortKeyByteSinkBase {
    /// `SortKeyByteSink(byte[] dest)`.
    pub fn new(dest: Vec<i8>) -> Self {
        Self {
            buffer: dest,
            appended: 0,
        }
    }
}

/// Equivalent to the abstract class `CollationKeys.SortKeyByteSink`: the
/// abstract methods are required, the others provided.
pub trait SortKeyByteSink {
    /// The inherited state.
    fn sink(&self) -> &SortKeyByteSinkBase;

    /// The inherited state, mutably.
    fn sink_mut(&mut self) -> &mut SortKeyByteSinkBase;

    /// `AppendBeyondCapacity(byte[], int, int, int)`.
    fn append_beyond_capacity(&mut self, bytes: &[i8], start: i32, n: i32, length: i32);

    /// `Resize(int, int)`.
    fn resize(&mut self, append_capacity: i32, length: i32) -> bool;

    /// `setBufferAndAppended(byte[], int)`.
    fn set_buffer_and_appended(&mut self, dest: Vec<i8>, app: i32) {
        let s = self.sink_mut();
        s.buffer = dest;
        s.appended = app;
    }

    /// `Append(byte[], int)`; `None` for Java's `null`.
    fn append_bytes(&mut self, bytes: Option<&[i8]>, n: i32) {
        let Some(bytes) = bytes else {
            return;
        };
        if n <= 0 {
            return;
        }
        let s = self.sink_mut();
        let length = s.appended;
        s.appended += n;
        let available = s.buffer.len() as i32 - length;
        if n <= available {
            s.buffer[length as usize..(length + n) as usize].copy_from_slice(&bytes[..n as usize]);
        } else {
            self.append_beyond_capacity(bytes, 0, n, length);
        }
    }

    /// `Append(int)`.
    fn append(&mut self, b: i32) {
        let appended = self.sink().appended;
        if (appended as usize) < self.sink().buffer.len() || self.resize(1, appended) {
            let s = self.sink_mut();
            s.buffer[s.appended as usize] = b as i8;
        }
        self.sink_mut().appended += 1;
    }

    /// `NumberOfBytesAppended()`.
    fn number_of_bytes_appended(&self) -> i32 {
        self.sink().appended
    }

    /// `GetRemainingCapacity()`.
    fn get_remaining_capacity(&self) -> i32 {
        self.sink().buffer.len() as i32 - self.sink().appended
    }

    /// `Overflowed()`.
    fn overflowed(&self) -> bool {
        self.sink().appended > self.sink().buffer.len() as i32
    }
}

/// Collation key level callback.
///
/// Equivalent to `CollationKeys.LevelCallback`.
pub trait LevelCallback {
    /// Returns true if the level will be written.
    fn need_to_write(&mut self, _level: i32) -> bool {
        true
    }
}

/// The plain `LevelCallback` (`SIMPLE_LEVEL_FALLBACK`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SimpleLevelFallback;

impl LevelCallback for SimpleLevelFallback {}

/// `SIMPLE_LEVEL_FALLBACK`.
pub const SIMPLE_LEVEL_FALLBACK: SimpleLevelFallback = SimpleLevelFallback;

const SORT_KEY_LEVEL_INITIAL_CAPACITY: usize = 40;

/// `CollationKeys.SortKeyLevel` (private).
#[derive(Debug, Clone)]
struct SortKeyLevel {
    buffer: Vec<i8>,
    len: i32,
}

impl SortKeyLevel {
    fn new() -> Self {
        Self {
            buffer: vec![0; SORT_KEY_LEVEL_INITIAL_CAPACITY],
            len: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn length(&self) -> i32 {
        self.len
    }

    fn get_at(&self, index: i32) -> i8 {
        self.buffer[index as usize]
    }

    fn append_byte(&mut self, b: i32) {
        if (self.len as usize) < self.buffer.len() || self.ensure_capacity(1) {
            self.buffer[self.len as usize] = b as i8;
            self.len += 1;
        }
    }

    fn append_weight16(&mut self, w: i32) {
        debug_assert!((w & 0xffff) != 0);
        let b0 = ((w as u32) >> 8) as i8;
        let b1 = w as i8;
        let append_length = if b1 == 0 { 1 } else { 2 };
        if (self.len + append_length) as usize <= self.buffer.len()
            || self.ensure_capacity(append_length)
        {
            self.buffer[self.len as usize] = b0;
            self.len += 1;
            if b1 != 0 {
                self.buffer[self.len as usize] = b1;
                self.len += 1;
            }
        }
    }

    fn append_weight32(&mut self, w: i64) {
        debug_assert!(w != 0);
        let w = w as u64;
        let bytes: [i8; 4] = [(w >> 24) as i8, (w >> 16) as i8, (w >> 8) as i8, w as i8];
        let append_length = if bytes[1] == 0 {
            1
        } else if bytes[2] == 0 {
            2
        } else if bytes[3] == 0 {
            3
        } else {
            4
        };
        if (self.len + append_length) as usize <= self.buffer.len()
            || self.ensure_capacity(append_length)
        {
            self.buffer[self.len as usize] = bytes[0];
            self.len += 1;
            if bytes[1] != 0 {
                self.buffer[self.len as usize] = bytes[1];
                self.len += 1;
                if bytes[2] != 0 {
                    self.buffer[self.len as usize] = bytes[2];
                    self.len += 1;
                    if bytes[3] != 0 {
                        self.buffer[self.len as usize] = bytes[3];
                        self.len += 1;
                    }
                }
            }
        }
    }

    fn append_reverse_weight16(&mut self, w: i32) {
        debug_assert!((w & 0xffff) != 0);
        let b0 = ((w as u32) >> 8) as i8;
        let b1 = w as i8;
        let append_length = if b1 == 0 { 1 } else { 2 };
        if (self.len + append_length) as usize <= self.buffer.len()
            || self.ensure_capacity(append_length)
        {
            if b1 == 0 {
                self.buffer[self.len as usize] = b0;
                self.len += 1;
            } else {
                self.buffer[self.len as usize] = b1;
                self.buffer[(self.len + 1) as usize] = b0;
                self.len += 2;
            }
        }
    }

    // Appends all but the last byte to the sink. The last byte should be the 01 terminator.
    fn append_to(&self, sink: &mut dyn SortKeyByteSink) {
        debug_assert!(self.len > 0 && self.buffer[(self.len - 1) as usize] == 1);
        sink.append_bytes(Some(&self.buffer), self.len - 1);
    }

    fn ensure_capacity(&mut self, append_capacity: i32) -> bool {
        let mut new_capacity = 2 * self.buffer.len() as i32;
        let alt_capacity = self.len + 2 * append_capacity;
        if new_capacity < alt_capacity {
            new_capacity = alt_capacity;
        }
        if new_capacity < 200 {
            new_capacity = 200;
        }
        let mut newbuf = vec![0i8; new_capacity as usize];
        newbuf[..self.len as usize].copy_from_slice(&self.buffer[..self.len as usize]);
        self.buffer = newbuf;
        true
    }
}

fn get_sort_key_level(levels: i32, level: i32) -> Option<SortKeyLevel> {
    if (levels & level) != 0 {
        Some(SortKeyLevel::new())
    } else {
        None
    }
}

fn lv(level: &mut Option<SortKeyLevel>) -> &mut SortKeyLevel {
    level
        .as_mut()
        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
}

/// Java's `(int) p >>> 24` for a long p.
fn lead_byte(p: i64) -> i32 {
    ((p as i32 as u32) >> 24) as i32
}

/// Builds sort keys.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationKeys` (static).
#[derive(Debug)]
pub struct CollationKeys;

impl CollationKeys {
    /// Secondary level: Compress up to 33 common weights as 05..25 or 25..45.
    const SEC_COMMON_LOW: i32 = Collation::COMMON_BYTE;
    const SEC_COMMON_MIDDLE: i32 = Self::SEC_COMMON_LOW + 0x20;
    /// read by CollationDataReader
    pub const SEC_COMMON_HIGH: i32 = Self::SEC_COMMON_LOW + 0x40;
    const SEC_COMMON_MAX_COUNT: i32 = 0x21;

    /// Case level, lowerFirst: Compress up to 7 common weights as 1..7 or 7..13.
    const CASE_LOWER_FIRST_COMMON_LOW: i32 = 1;
    const CASE_LOWER_FIRST_COMMON_MIDDLE: i32 = 7;
    const CASE_LOWER_FIRST_COMMON_HIGH: i32 = 13;
    const CASE_LOWER_FIRST_COMMON_MAX_COUNT: i32 = 7;

    /// Case level, upperFirst: Compress up to 13 common weights as 3..15.
    const CASE_UPPER_FIRST_COMMON_LOW: i32 = 3;
    #[allow(dead_code)]
    const CASE_UPPER_FIRST_COMMON_HIGH: i32 = 15;
    const CASE_UPPER_FIRST_COMMON_MAX_COUNT: i32 = 13;

    /// Tertiary level only (no case): Compress up to 97 common weights as 05..65 or 65..C5.
    const TER_ONLY_COMMON_LOW: i32 = Collation::COMMON_BYTE;
    const TER_ONLY_COMMON_MIDDLE: i32 = Self::TER_ONLY_COMMON_LOW + 0x60;
    const TER_ONLY_COMMON_HIGH: i32 = Self::TER_ONLY_COMMON_LOW + 0xc0;
    const TER_ONLY_COMMON_MAX_COUNT: i32 = 0x61;

    /// Tertiary with case, lowerFirst: Compress up to 33 common weights as 05..25 or 25..45.
    const TER_LOWER_FIRST_COMMON_LOW: i32 = Collation::COMMON_BYTE;
    const TER_LOWER_FIRST_COMMON_MIDDLE: i32 = Self::TER_LOWER_FIRST_COMMON_LOW + 0x20;
    const TER_LOWER_FIRST_COMMON_HIGH: i32 = Self::TER_LOWER_FIRST_COMMON_LOW + 0x40;
    const TER_LOWER_FIRST_COMMON_MAX_COUNT: i32 = 0x21;

    /// Tertiary with case, upperFirst: Compress up to 33 common weights as 85..A5 or A5..C5.
    const TER_UPPER_FIRST_COMMON_LOW: i32 = Collation::COMMON_BYTE + 0x80;
    const TER_UPPER_FIRST_COMMON_MIDDLE: i32 = Self::TER_UPPER_FIRST_COMMON_LOW + 0x20;
    const TER_UPPER_FIRST_COMMON_HIGH: i32 = Self::TER_UPPER_FIRST_COMMON_LOW + 0x40;
    const TER_UPPER_FIRST_COMMON_MAX_COUNT: i32 = 0x21;

    /// Quaternary level: Compress up to 113 common weights as 1C..8C or 8C..FC.
    const QUAT_COMMON_LOW: i32 = 0x1c;
    const QUAT_COMMON_MIDDLE: i32 = Self::QUAT_COMMON_LOW + 0x70;
    const QUAT_COMMON_HIGH: i32 = Self::QUAT_COMMON_LOW + 0xE0;
    const QUAT_COMMON_MAX_COUNT: i32 = 0x71;
    /// Primary weights shifted to quaternary level must be encoded with
    /// a lead byte below the common-weight compression range.
    const QUAT_SHIFTED_LIMIT_BYTE: i32 = Self::QUAT_COMMON_LOW - 1; // 0x1b

    /// Map from collation strength (UColAttributeValue) to a mask of
    /// Collation.Level bits up to that strength, excluding the CASE_LEVEL
    /// which is independent of the strength, and excluding IDENTICAL_LEVEL
    /// which this function does not write.
    const LEVEL_MASKS: [i32; 16] = [
        2,    // UCOL_PRIMARY -> PRIMARY_LEVEL
        6,    // UCOL_SECONDARY -> up to SECONDARY_LEVEL
        0x16, // UCOL_TERTIARY -> up to TERTIARY_LEVEL
        0x36, // UCOL_QUATERNARY -> up to QUATERNARY_LEVEL
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x36, // UCOL_IDENTICAL -> up to QUATERNARY_LEVEL
    ];

    /// Writes the sort key bytes for minLevel up to the iterator data's
    /// strength. Optionally writes the case level. Stops writing levels
    /// when callback.needToWrite(level) returns false. Separates levels
    /// with the LEVEL_SEPARATOR_BYTE but does not write a
    /// TERMINATOR_BYTE.
    ///
    /// Equivalent to `writeSortKeyUpToQuaternary(CollationIterator,
    /// boolean[], CollationSettings, SortKeyByteSink, int, LevelCallback,
    /// boolean)`.
    #[allow(clippy::too_many_arguments)]
    pub fn write_sort_key_up_to_quaternary(
        iter: &mut dyn CollationIterator,
        compressible_bytes: &[bool],
        settings: &CollationSettings,
        sink: &mut dyn SortKeyByteSink,
        min_level: i32,
        callback: &mut dyn LevelCallback,
        preflight: bool,
    ) {
        let options = settings.options;
        // Set of levels to process and write.
        let mut levels = Self::LEVEL_MASKS[CollationSettings::get_strength_of(options) as usize];
        if (options & CollationSettings::CASE_LEVEL) != 0 {
            levels |= Collation::CASE_LEVEL_FLAG;
        }
        // Minus the levels below minLevel.
        levels &= !((1 << min_level) - 1);
        if levels == 0 {
            return;
        }

        let variable_top: i64 = if (options & CollationSettings::ALTERNATE_MASK) == 0 {
            0
        } else {
            // +1 so that we can use "<" and primary ignorables test out early.
            settings.variable_top + 1
        };

        let tertiary_mask = CollationSettings::get_tertiary_mask(options);

        let mut p234 = [0i8; 3];
        let mut cases = get_sort_key_level(levels, Collation::CASE_LEVEL_FLAG);
        let mut secondaries = get_sort_key_level(levels, Collation::SECONDARY_LEVEL_FLAG);
        let mut tertiaries = get_sort_key_level(levels, Collation::TERTIARY_LEVEL_FLAG);
        let mut quaternaries = get_sort_key_level(levels, Collation::QUATERNARY_LEVEL_FLAG);

        let mut prev_reordered_primary: i64 = 0; // 0==no compression
        let mut common_cases = 0;
        let mut common_secondaries = 0;
        let mut common_tertiaries = 0;
        let mut common_quaternaries = 0;

        let mut prev_secondary = 0;
        let mut sec_segment_start = 0;

        loop {
            // No need to keep all CEs in the buffer when we write a sort key.
            iter.base_mut().clear_ces_if_none_remaining();
            let mut ce = iter.next_ce();
            let mut p = ((ce as u64) >> 32) as i64;
            if p < variable_top && p > Collation::MERGE_SEPARATOR_PRIMARY {
                // Variable CE, shift it to quaternary level.
                // Ignore all following primary ignorables, and shift further variable CEs.
                if common_quaternaries != 0 {
                    common_quaternaries -= 1;
                    while common_quaternaries >= Self::QUAT_COMMON_MAX_COUNT {
                        lv(&mut quaternaries).append_byte(Self::QUAT_COMMON_MIDDLE);
                        common_quaternaries -= Self::QUAT_COMMON_MAX_COUNT;
                    }
                    // Shifted primary weights are lower than the common weight.
                    lv(&mut quaternaries).append_byte(Self::QUAT_COMMON_LOW + common_quaternaries);
                    common_quaternaries = 0;
                }
                loop {
                    if (levels & Collation::QUATERNARY_LEVEL_FLAG) != 0 {
                        if settings.has_reordering() {
                            p = settings.reorder(p);
                        }
                        if lead_byte(p) >= Self::QUAT_SHIFTED_LIMIT_BYTE {
                            // Prevent shifted primary lead bytes from
                            // overlapping with the common compression range.
                            lv(&mut quaternaries).append_byte(Self::QUAT_SHIFTED_LIMIT_BYTE);
                        }
                        lv(&mut quaternaries).append_weight32(p);
                    }
                    loop {
                        ce = iter.next_ce();
                        p = ((ce as u64) >> 32) as i64;
                        if p != 0 {
                            break;
                        }
                    }
                    if !(p < variable_top && p > Collation::MERGE_SEPARATOR_PRIMARY) {
                        break;
                    }
                }
            }
            // ce could be primary ignorable, or NO_CE, or the merge separator,
            // or a regular primary CE, but it is not variable.
            // If ce==NO_CE, then write nothing for the primary level but
            // terminate compression on all levels and then exit the loop.
            if p > Collation::NO_CE_PRIMARY && (levels & Collation::PRIMARY_LEVEL_FLAG) != 0 {
                // Test the un-reordered primary for compressibility.
                let is_compressible = compressible_bytes[lead_byte(p) as usize];
                if settings.has_reordering() {
                    p = settings.reorder(p);
                }
                let p1 = lead_byte(p);
                if !is_compressible || p1 != lead_byte(prev_reordered_primary) {
                    if prev_reordered_primary != 0 {
                        if p < prev_reordered_primary {
                            // No primary compression terminator
                            // at the end of the level or merged segment.
                            if p1 > Collation::MERGE_SEPARATOR_BYTE {
                                sink.append(Collation::PRIMARY_COMPRESSION_LOW_BYTE);
                            }
                        } else {
                            sink.append(Collation::PRIMARY_COMPRESSION_HIGH_BYTE);
                        }
                    }
                    sink.append(p1);
                    if is_compressible {
                        prev_reordered_primary = p;
                    } else {
                        prev_reordered_primary = 0;
                    }
                }
                let p2 = (p >> 16) as i8;
                if p2 != 0 {
                    p234[0] = p2;
                    p234[1] = (p >> 8) as i8;
                    p234[2] = p as i8;
                    let n = if p234[1] == 0 {
                        1
                    } else if p234[2] == 0 {
                        2
                    } else {
                        3
                    };
                    sink.append_bytes(Some(&p234), n);
                }
                // Optimization for internalNextSortKeyPart():
                // When the primary level overflows we can stop because we need not
                // calculate (preflight) the whole sort key length.
                if !preflight && sink.overflowed() {
                    // not used in Java -- if (!sink.IsOk()) {
                    // Java porting note: U_MEMORY_ALLOCATION_ERROR is set here in
                    // C implementation. IsOk() in Java always returns true, so this
                    // is a dead code.
                    return;
                }
            }

            let lower32 = ce as i32;
            if lower32 == 0 {
                continue;
            } // completely ignorable, no secondary/case/tertiary/quaternary

            if (levels & Collation::SECONDARY_LEVEL_FLAG) != 0 {
                let s = ((lower32 as u32) >> 16) as i32; // 16 bits
                if s == 0 {
                    // secondary ignorable
                } else if s == Collation::COMMON_WEIGHT16
                    && ((options & CollationSettings::BACKWARD_SECONDARY) == 0
                        || p != Collation::MERGE_SEPARATOR_PRIMARY)
                {
                    // s is a common secondary weight, and
                    // backwards-secondary is off or the ce is not the merge separator.
                    common_secondaries += 1;
                } else if (options & CollationSettings::BACKWARD_SECONDARY) == 0 {
                    let secondaries = lv(&mut secondaries);
                    if common_secondaries != 0 {
                        common_secondaries -= 1;
                        while common_secondaries >= Self::SEC_COMMON_MAX_COUNT {
                            secondaries.append_byte(Self::SEC_COMMON_MIDDLE);
                            common_secondaries -= Self::SEC_COMMON_MAX_COUNT;
                        }
                        let b = if s < Collation::COMMON_WEIGHT16 {
                            Self::SEC_COMMON_LOW + common_secondaries
                        } else {
                            Self::SEC_COMMON_HIGH - common_secondaries
                        };
                        secondaries.append_byte(b);
                        common_secondaries = 0;
                    }
                    secondaries.append_weight16(s);
                } else {
                    let secondaries = lv(&mut secondaries);
                    if common_secondaries != 0 {
                        common_secondaries -= 1;
                        // Append reverse weights. The level will be re-reversed later.
                        let remainder = common_secondaries % Self::SEC_COMMON_MAX_COUNT;
                        let b = if prev_secondary < Collation::COMMON_WEIGHT16 {
                            Self::SEC_COMMON_LOW + remainder
                        } else {
                            Self::SEC_COMMON_HIGH - remainder
                        };
                        secondaries.append_byte(b);
                        common_secondaries -= remainder;
                        // commonSecondaries is now a multiple of SEC_COMMON_MAX_COUNT.
                        while common_secondaries > 0 {
                            // same as >= SEC_COMMON_MAX_COUNT
                            secondaries.append_byte(Self::SEC_COMMON_MIDDLE);
                            common_secondaries -= Self::SEC_COMMON_MAX_COUNT;
                        }
                        // commonSecondaries == 0
                    }
                    if 0 < p && p <= Collation::MERGE_SEPARATOR_PRIMARY {
                        // The backwards secondary level compares secondary weights backwards
                        // within segments separated by the merge separator (U+FFFE).
                        let mut last = secondaries.length() - 1;
                        while sec_segment_start < last {
                            secondaries
                                .buffer
                                .swap(sec_segment_start as usize, last as usize);
                            sec_segment_start += 1;
                            last -= 1;
                        }
                        secondaries.append_byte(if p == Collation::NO_CE_PRIMARY {
                            Collation::LEVEL_SEPARATOR_BYTE
                        } else {
                            Collation::MERGE_SEPARATOR_BYTE
                        });
                        prev_secondary = 0;
                        sec_segment_start = secondaries.length();
                    } else {
                        secondaries.append_reverse_weight16(s);
                        prev_secondary = s;
                    }
                }
            }

            if (levels & Collation::CASE_LEVEL_FLAG) != 0 {
                if if CollationSettings::get_strength_of(options) == collator::PRIMARY {
                    p == 0
                } else {
                    ((lower32 as u32) >> 16) == 0
                } {
                    // Primary+caseLevel: Ignore case level weights of primary ignorables.
                    // Otherwise: Ignore case level weights of secondary ignorables.
                    // For details see the comments in the CollationCompare class.
                } else {
                    let cases = lv(&mut cases);
                    let mut c = ((lower32 as u32) >> 8) as i32 & 0xff; // case bits & tertiary lead byte
                    debug_assert!((c & 0xc0) != 0xc0);
                    if (c & 0xc0) == 0 && c > Collation::LEVEL_SEPARATOR_BYTE {
                        common_cases += 1;
                    } else {
                        if (options & CollationSettings::UPPER_FIRST) == 0 {
                            // lowerFirst: Compress common weights to nibbles 1..7..13, mixed=14,
                            // upper=15.
                            // If there are only common (=lowest) weights in the whole level,
                            // then we need not write anything.
                            // Level length differences are handled already on the next-higher level.
                            if common_cases != 0
                                && (c > Collation::LEVEL_SEPARATOR_BYTE || !cases.is_empty())
                            {
                                common_cases -= 1;
                                while common_cases >= Self::CASE_LOWER_FIRST_COMMON_MAX_COUNT {
                                    cases.append_byte(Self::CASE_LOWER_FIRST_COMMON_MIDDLE << 4);
                                    common_cases -= Self::CASE_LOWER_FIRST_COMMON_MAX_COUNT;
                                }
                                let b = if c <= Collation::LEVEL_SEPARATOR_BYTE {
                                    Self::CASE_LOWER_FIRST_COMMON_LOW + common_cases
                                } else {
                                    Self::CASE_LOWER_FIRST_COMMON_HIGH - common_cases
                                };
                                cases.append_byte(b << 4);
                                common_cases = 0;
                            }
                            if c > Collation::LEVEL_SEPARATOR_BYTE {
                                c = (Self::CASE_LOWER_FIRST_COMMON_HIGH + ((c as u32) >> 6) as i32)
                                    << 4; // 14 or 15
                            }
                        } else {
                            // upperFirst: Compress common weights to nibbles 3..15, mixed=2,
                            // upper=1.
                            // The compressed common case weights only go up from the "low" value
                            // because with upperFirst the common weight is the highest one.
                            if common_cases != 0 {
                                common_cases -= 1;
                                while common_cases >= Self::CASE_UPPER_FIRST_COMMON_MAX_COUNT {
                                    cases.append_byte(Self::CASE_UPPER_FIRST_COMMON_LOW << 4);
                                    common_cases -= Self::CASE_UPPER_FIRST_COMMON_MAX_COUNT;
                                }
                                cases.append_byte(
                                    (Self::CASE_UPPER_FIRST_COMMON_LOW + common_cases) << 4,
                                );
                                common_cases = 0;
                            }
                            if c > Collation::LEVEL_SEPARATOR_BYTE {
                                c = (Self::CASE_UPPER_FIRST_COMMON_LOW - ((c as u32) >> 6) as i32)
                                    << 4; // 2 or 1
                            }
                        }
                        // c is a separator byte 01,
                        // or a left-shifted nibble 0x10, 0x20, ... 0xf0.
                        cases.append_byte(c);
                    }
                }
            }

            if (levels & Collation::TERTIARY_LEVEL_FLAG) != 0 {
                let tertiaries = lv(&mut tertiaries);
                let mut t = lower32 & tertiary_mask;
                debug_assert!((lower32 & 0xc000) != 0xc000);
                if t == Collation::COMMON_WEIGHT16 {
                    common_tertiaries += 1;
                } else if (tertiary_mask & 0x8000) == 0 {
                    // Tertiary weights without case bits.
                    // Move lead bytes 06..3F to C6..FF for a large common-weight range.
                    if common_tertiaries != 0 {
                        common_tertiaries -= 1;
                        while common_tertiaries >= Self::TER_ONLY_COMMON_MAX_COUNT {
                            tertiaries.append_byte(Self::TER_ONLY_COMMON_MIDDLE);
                            common_tertiaries -= Self::TER_ONLY_COMMON_MAX_COUNT;
                        }
                        let b = if t < Collation::COMMON_WEIGHT16 {
                            Self::TER_ONLY_COMMON_LOW + common_tertiaries
                        } else {
                            Self::TER_ONLY_COMMON_HIGH - common_tertiaries
                        };
                        tertiaries.append_byte(b);
                        common_tertiaries = 0;
                    }
                    if t > Collation::COMMON_WEIGHT16 {
                        t += 0xc000;
                    }
                    tertiaries.append_weight16(t);
                } else if (options & CollationSettings::UPPER_FIRST) == 0 {
                    // Tertiary weights with caseFirst=lowerFirst.
                    // Move lead bytes 06..BF to 46..FF for the common-weight range.
                    if common_tertiaries != 0 {
                        common_tertiaries -= 1;
                        while common_tertiaries >= Self::TER_LOWER_FIRST_COMMON_MAX_COUNT {
                            tertiaries.append_byte(Self::TER_LOWER_FIRST_COMMON_MIDDLE);
                            common_tertiaries -= Self::TER_LOWER_FIRST_COMMON_MAX_COUNT;
                        }
                        let b = if t < Collation::COMMON_WEIGHT16 {
                            Self::TER_LOWER_FIRST_COMMON_LOW + common_tertiaries
                        } else {
                            Self::TER_LOWER_FIRST_COMMON_HIGH - common_tertiaries
                        };
                        tertiaries.append_byte(b);
                        common_tertiaries = 0;
                    }
                    if t > Collation::COMMON_WEIGHT16 {
                        t += 0x4000;
                    }
                    tertiaries.append_weight16(t);
                } else {
                    // Tertiary weights with caseFirst=upperFirst.
                    // Do not change the artificial uppercase weight of a tertiary CE (0.0.ut),
                    // to keep tertiary CEs well-formed.
                    // Their case+tertiary weights must be greater than those of
                    // primary and secondary CEs.
                    //
                    // Separator         01 -> 01      (unchanged)
                    // Lowercase     02..04 -> 82..84  (includes uncased)
                    // Common weight     05 -> 85..C5  (common-weight compression range)
                    // Lowercase     06..3F -> C6..FF
                    // Mixed case    42..7F -> 42..7F
                    // Uppercase     82..BF -> 02..3F
                    // Tertiary CE   86..BF -> C6..FF
                    if t <= Collation::NO_CE_WEIGHT16 {
                        // Keep separators unchanged.
                    } else if ((lower32 as u32) >> 16) != 0 {
                        // Invert case bits of primary & secondary CEs.
                        t ^= 0xc000;
                        if t < (Self::TER_UPPER_FIRST_COMMON_HIGH << 8) {
                            t -= 0x4000;
                        }
                    } else {
                        // Keep uppercase bits of tertiary CEs.
                        debug_assert!((0x8600..=0xbfff).contains(&t));
                        t += 0x4000;
                    }
                    if common_tertiaries != 0 {
                        common_tertiaries -= 1;
                        while common_tertiaries >= Self::TER_UPPER_FIRST_COMMON_MAX_COUNT {
                            tertiaries.append_byte(Self::TER_UPPER_FIRST_COMMON_MIDDLE);
                            common_tertiaries -= Self::TER_UPPER_FIRST_COMMON_MAX_COUNT;
                        }
                        let b = if t < (Self::TER_UPPER_FIRST_COMMON_LOW << 8) {
                            Self::TER_UPPER_FIRST_COMMON_LOW + common_tertiaries
                        } else {
                            Self::TER_UPPER_FIRST_COMMON_HIGH - common_tertiaries
                        };
                        tertiaries.append_byte(b);
                        common_tertiaries = 0;
                    }
                    tertiaries.append_weight16(t);
                }
            }

            if (levels & Collation::QUATERNARY_LEVEL_FLAG) != 0 {
                let quaternaries = lv(&mut quaternaries);
                let mut q = lower32 & 0xffff;
                if (q & 0xc0) == 0 && q > Collation::NO_CE_WEIGHT16 {
                    common_quaternaries += 1;
                } else if q == Collation::NO_CE_WEIGHT16
                    && (options & CollationSettings::ALTERNATE_MASK) == 0
                    && quaternaries.is_empty()
                {
                    // If alternate=non-ignorable and there are only common quaternary weights,
                    // then we need not write anything.
                    // The only weights greater than the merge separator and less than the common
                    // weight
                    // are shifted primary weights, which are not generated for
                    // alternate=non-ignorable.
                    // There are also exactly as many quaternary weights as tertiary weights,
                    // so level length differences are handled already on tertiary level.
                    // Any above-common quaternary weight will compare greater regardless.
                    quaternaries.append_byte(Collation::LEVEL_SEPARATOR_BYTE);
                } else {
                    if q == Collation::NO_CE_WEIGHT16 {
                        q = Collation::LEVEL_SEPARATOR_BYTE;
                    } else {
                        q = 0xfc + ((q >> 6) & 3);
                    }
                    if common_quaternaries != 0 {
                        common_quaternaries -= 1;
                        while common_quaternaries >= Self::QUAT_COMMON_MAX_COUNT {
                            quaternaries.append_byte(Self::QUAT_COMMON_MIDDLE);
                            common_quaternaries -= Self::QUAT_COMMON_MAX_COUNT;
                        }
                        let b = if q < Self::QUAT_COMMON_LOW {
                            Self::QUAT_COMMON_LOW + common_quaternaries
                        } else {
                            Self::QUAT_COMMON_HIGH - common_quaternaries
                        };
                        quaternaries.append_byte(b);
                        common_quaternaries = 0;
                    }
                    quaternaries.append_byte(q);
                }
            }

            if ((lower32 as u32) >> 24) as i32 == Collation::LEVEL_SEPARATOR_BYTE {
                break;
            } // ce == NO_CE
        }

        // Append the beyond-primary levels.
        // not used in Java -- boolean ok = true;
        if (levels & Collation::SECONDARY_LEVEL_FLAG) != 0 {
            if !callback.need_to_write(Collation::SECONDARY_LEVEL) {
                return;
            }
            // not used in Java -- ok &= secondaries.isOk();
            sink.append(Collation::LEVEL_SEPARATOR_BYTE);
            lv(&mut secondaries).append_to(sink);
        }

        if (levels & Collation::CASE_LEVEL_FLAG) != 0 {
            if !callback.need_to_write(Collation::CASE_LEVEL) {
                return;
            }
            // not used in Java -- ok &= cases.isOk();
            sink.append(Collation::LEVEL_SEPARATOR_BYTE);
            // Write pairs of nibbles as bytes, except separator bytes as themselves.
            let cases = lv(&mut cases);
            let length = cases.length() - 1; // Ignore the trailing NO_CE.
            let mut b: i8 = 0;
            for i in 0..length {
                let c = cases.get_at(i);
                debug_assert!((c & 0xf) == 0 && c != 0);
                if b == 0 {
                    b = c;
                } else {
                    sink.append(b as i32 | ((c as i32 >> 4) & 0xf));
                    b = 0;
                }
            }
            if b != 0 {
                sink.append(b as i32);
            }
        }

        if (levels & Collation::TERTIARY_LEVEL_FLAG) != 0 {
            if !callback.need_to_write(Collation::TERTIARY_LEVEL) {
                return;
            }
            // not used in Java -- ok &= tertiaries.isOk();
            sink.append(Collation::LEVEL_SEPARATOR_BYTE);
            lv(&mut tertiaries).append_to(sink);
        }

        if (levels & Collation::QUATERNARY_LEVEL_FLAG) != 0 {
            if !callback.need_to_write(Collation::QUATERNARY_LEVEL) {
                return;
            }
            // not used in Java -- ok &= quaternaries.isOk();
            sink.append(Collation::LEVEL_SEPARATOR_BYTE);
            lv(&mut quaternaries).append_to(sink);
        }

        // not used in Java -- if (!ok || !sink.IsOk()) {
        // Java porting note: U_MEMORY_ALLOCATION_ERROR is set here in
        // C implementation. IsOk() in Java always returns true, so this
        // is a dead code.
    }
}
