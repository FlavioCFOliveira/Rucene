// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.BMPSet`.

use crate::jdk::to_code_point;
use crate::text::unicode_set::SpanCondition;
use crate::util::output_int::OutputInt;

/// `U16_SURROGATE_OFFSET` (a non-final public static in Java, never
/// reassigned).
pub const U16_SURROGATE_OFFSET: i32 = (0xd800 << 10) + 0xdc00 - 0x10000;

/// Java's `int << n`, which masks the shift count to its low 5 bits.
fn shl(a: i32, n: i32) -> i32 {
    a.wrapping_shl(n as u32)
}

/// Helper class for frozen UnicodeSets, implements contains() and span()
/// optimized for BMP code points.
///
/// Equivalent to `com.ibm.icu.impl.BMPSet`. Java shares the parent set's
/// `list` array (frozen, so never changed); here the set keeps a copy of
/// its first `listLength` values.
#[derive(Debug, Clone)]
pub struct BMPSet {
    /// One boolean ('true' or 'false') per Latin-1 character.
    latin1_contains: [bool; 0x100],
    /// One bit per code point from U+0000..U+07FF.
    table7ff: [i32; 64],
    /// One bit per 64 BMP code points.
    bmp_block_bits: [i32; 64],
    /// Inversion list indexes for restricted binary searches in
    /// findCodePoint(), from findCodePoint(U+0800, U+1000, U+2000, ..,
    /// U+F000, U+10000, U+110000).
    list4k_starts: [i32; 18],
    /// The inversion list of the parent set.
    list: Vec<i32>,
    list_length: i32,
}

impl BMPSet {
    /// `BMPSet(int[] parentList, int parentListLength)`.
    pub fn new(parent_list: &[i32], parent_list_length: i32) -> Self {
        let mut s = Self {
            latin1_contains: [false; 0x100],
            table7ff: [0; 64],
            bmp_block_bits: [0; 64],
            list4k_starts: [0; 18],
            list: parent_list[..parent_list_length as usize].to_vec(),
            list_length: parent_list_length,
        };
        // Set the list indexes for binary searches for
        // U+0800, U+1000, U+2000, .., U+F000, U+10000.
        // U+0800 is the first 3-byte-UTF-8 code point. Lower code points are
        // looked up in the bit tables.
        // The last pair of indexes is for finding supplementary code points.
        s.list4k_starts[0] = s.find_code_point(0x800, 0, s.list_length - 1);
        for i in 1..=0x10usize {
            s.list4k_starts[i] =
                s.find_code_point((i as i32) << 12, s.list4k_starts[i - 1], s.list_length - 1);
        }
        s.list4k_starts[0x11] = s.list_length - 1;
        s.init_bits();
        s
    }

    /// `BMPSet(BMPSet otherBMPSet, int[] newParentList, int
    /// newParentListLength)`.
    pub fn from_other(
        other_bmp_set: &BMPSet,
        new_parent_list: &[i32],
        new_parent_list_length: i32,
    ) -> Self {
        Self {
            latin1_contains: other_bmp_set.latin1_contains,
            table7ff: other_bmp_set.table7ff,
            bmp_block_bits: other_bmp_set.bmp_block_bits,
            list4k_starts: other_bmp_set.list4k_starts,
            list: new_parent_list[..new_parent_list_length as usize].to_vec(),
            list_length: new_parent_list_length,
        }
    }

    /// `contains(int c)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for a negative `c`.
    pub fn contains(&self, c: i32) -> bool {
        if c <= 0xff {
            self.latin1_contains[c as usize]
        } else if c <= 0x7ff {
            (self.table7ff[(c & 0x3f) as usize] & shl(1, c >> 6)) != 0
        } else if c < 0xd800 || (0xe000..=0xffff).contains(&c) {
            let lead = c >> 12;
            let two_bits = (self.bmp_block_bits[((c >> 6) & 0x3f) as usize] >> lead) & 0x10001;
            if two_bits <= 1 {
                // All 64 code points with the same bits 15..6
                // are either in the set or not.
                0 != two_bits
            } else {
                // Look up the code point in its 4k block of code points.
                self.contains_slow(
                    c,
                    self.list4k_starts[lead as usize],
                    self.list4k_starts[lead as usize + 1],
                )
            }
        } else if c <= 0x10ffff {
            // surrogate or supplementary code point
            self.contains_slow(c, self.list4k_starts[0xd], self.list4k_starts[0x11])
        } else {
            // Out-of-range code points get false, consistent with long-standing
            // behavior of UnicodeSet.contains(c).
            false
        }
    }

    /// `span(CharSequence s, int start, SpanCondition spanCondition,
    /// OutputInt outCount)`: the end of the span from `start`; `outCount`
    /// receives the number of code points.
    pub fn span(
        &self,
        s: &[u16],
        start: i32,
        span_condition: SpanCondition,
        out_count: Option<&mut OutputInt>,
    ) -> i32 {
        let mut i = start;
        let limit = s.len() as i32;
        let mut num_supplementary = 0;
        let contained = SpanCondition::NotContained != span_condition;
        while i < limit {
            let c = s[i as usize];
            let found;
            if c <= 0xff {
                found = self.latin1_contains[c as usize];
            } else if c <= 0x7ff {
                found = (self.table7ff[(c & 0x3f) as usize] & shl(1, (c >> 6) as i32)) != 0;
            } else if !(0xd800..0xdc00).contains(&c) || (i + 1) == limit || {
                let c2 = s[i as usize + 1];
                !(0xdc00..0xe000).contains(&c2)
            } {
                let lead = (c >> 12) as i32;
                let two_bits = (self.bmp_block_bits[((c >> 6) & 0x3f) as usize] >> lead) & 0x10001;
                if two_bits <= 1 {
                    found = two_bits != 0;
                } else {
                    found = self.contains_slow(
                        c as i32,
                        self.list4k_starts[lead as usize],
                        self.list4k_starts[lead as usize + 1],
                    );
                }
            } else {
                // surrogate pair
                let c2 = s[i as usize + 1];
                let supplementary = to_code_point(c, c2);
                if self.contains_slow(
                    supplementary,
                    self.list4k_starts[0x10],
                    self.list4k_starts[0x11],
                ) != contained
                {
                    break;
                }
                num_supplementary += 1;
                i += 2;
                continue;
            }
            if found != contained {
                break;
            }
            i += 1;
        }
        if let Some(out_count) = out_count {
            let span_length = i - start;
            out_count.value = span_length - num_supplementary; // number of code points
        }
        i
    }

    /// `spanBack(CharSequence s, int limit, SpanCondition spanCondition)`:
    /// the start of the span that ends at `limit`.
    pub fn span_back(&self, s: &[u16], mut limit: i32, span_condition: SpanCondition) -> i32 {
        let contained = SpanCondition::NotContained != span_condition;
        loop {
            limit -= 1;
            let c = s[limit as usize];
            let found;
            if c <= 0xff {
                found = self.latin1_contains[c as usize];
            } else if c <= 0x7ff {
                found = (self.table7ff[(c & 0x3f) as usize] & shl(1, (c >> 6) as i32)) != 0;
            } else if c < 0xd800 || c < 0xdc00 || 0 == limit || {
                let c2 = s[limit as usize - 1];
                !(0xd800..0xdc00).contains(&c2)
            } {
                let lead = (c >> 12) as i32;
                let two_bits = (self.bmp_block_bits[((c >> 6) & 0x3f) as usize] >> lead) & 0x10001;
                if two_bits <= 1 {
                    found = two_bits != 0;
                } else {
                    found = self.contains_slow(
                        c as i32,
                        self.list4k_starts[lead as usize],
                        self.list4k_starts[lead as usize + 1],
                    );
                }
            } else {
                // surrogate pair
                let c2 = s[limit as usize - 1];
                let supplementary = to_code_point(c2, c);
                found = self.contains_slow(
                    supplementary,
                    self.list4k_starts[0x10],
                    self.list4k_starts[0x11],
                );
                if found == contained {
                    limit -= 1;
                }
            }
            if found != contained {
                break;
            }
            if 0 == limit {
                return 0;
            }
        }
        limit + 1
    }

    /// Set bits in a bit rectangle in "vertical" bit organization.
    /// start<limit<=0x800
    fn set32x64_bits(table: &mut [i32; 64], start: i32, limit: i32) {
        let mut lead = start >> 6; // Named for UTF-8 2-byte lead byte with upper 5 bits.
        let mut trail = start & 0x3f; // Named for UTF-8 2-byte trail byte with lower 6 bits.

        // Set one bit indicating an all-one block.
        let mut bits = shl(1, lead);
        if (start + 1) == limit {
            // Single-character shortcut.
            table[trail as usize] |= bits;
            return;
        }

        let limit_lead = limit >> 6;
        let limit_trail = limit & 0x3f;

        if lead == limit_lead {
            // Partial vertical bit column.
            while trail < limit_trail {
                table[trail as usize] |= bits;
                trail += 1;
            }
        } else {
            // Partial vertical bit column,
            // followed by a bit rectangle,
            // followed by another partial vertical bit column.
            if trail > 0 {
                loop {
                    table[trail as usize] |= bits;
                    trail += 1;
                    if trail >= 64 {
                        break;
                    }
                }
                lead += 1;
            }
            if lead < limit_lead {
                bits = !shl(1, lead).wrapping_sub(1);
                if limit_lead < 0x20 {
                    bits &= shl(1, limit_lead).wrapping_sub(1);
                }
                for t in table.iter_mut() {
                    *t |= bits;
                }
            }
            // limit<=0x800. If limit==0x800 then limitLead=32 and limitTrail=0.
            // In that case, bits=1<<limitLead == 1<<0 == 1
            // (because Java << uses only the lower 5 bits of the shift operand)
            // but the bits value is not used because trail<limitTrail is already false.
            bits = shl(1, limit_lead);
            for t in table.iter_mut().take(limit_trail as usize) {
                *t |= bits;
            }
        }
    }

    fn init_bits(&mut self) {
        let mut start;
        let mut limit;
        let mut list_index = 0usize;
        let ll = self.list_length as usize;

        // Set latin1Contains[].
        loop {
            start = self.list[list_index];
            list_index += 1;
            if list_index < ll {
                limit = self.list[list_index];
                list_index += 1;
            } else {
                limit = 0x110000;
            }
            if start >= 0x100 {
                break;
            }
            loop {
                self.latin1_contains[start as usize] = true;
                start += 1;
                if !(start < limit && start < 0x100) {
                    break;
                }
            }
            if limit > 0x100 {
                break;
            }
        }

        // Set table7FF[].
        while start < 0x800 {
            Self::set32x64_bits(
                &mut self.table7ff,
                start,
                if limit <= 0x800 { limit } else { 0x800 },
            );
            if limit > 0x800 {
                start = 0x800;
                break;
            }

            start = self.list[list_index];
            list_index += 1;
            if list_index < ll {
                limit = self.list[list_index];
                list_index += 1;
            } else {
                limit = 0x110000;
            }
        }

        // Set bmpBlockBits[].
        let mut min_start = 0x800;
        while start < 0x10000 {
            if limit > 0x10000 {
                limit = 0x10000;
            }

            if start < min_start {
                start = min_start;
            }
            if start < limit {
                // Else: Another range entirely in a known mixed-value block.
                if 0 != (start & 0x3f) {
                    // Mixed-value block of 64 code points.
                    start >>= 6;
                    self.bmp_block_bits[(start & 0x3f) as usize] |= shl(0x10001, start >> 6);
                    start = (start + 1) << 6; // Round up to the next block boundary.
                    min_start = start; // Ignore further ranges in this block.
                }
                if start < limit {
                    if start < (limit & !0x3f) {
                        // Multiple all-ones blocks of 64 code points each.
                        Self::set32x64_bits(&mut self.bmp_block_bits, start >> 6, limit >> 6);
                    }

                    if 0 != (limit & 0x3f) {
                        // Mixed-value block of 64 code points.
                        limit >>= 6;
                        self.bmp_block_bits[(limit & 0x3f) as usize] |= shl(0x10001, limit >> 6);
                        limit = (limit + 1) << 6; // Round up to the next block boundary.
                        min_start = limit; // Ignore further ranges in this block.
                    }
                }
            }

            if limit == 0x10000 {
                break;
            }

            start = self.list[list_index];
            list_index += 1;
            if list_index < ll {
                limit = self.list[list_index];
                list_index += 1;
            } else {
                limit = 0x110000;
            }
        }
    }

    /// Same as UnicodeSet.findCodePoint(int c) except that the binary search
    /// is restricted for finding code points in a certain range.
    fn find_code_point(&self, c: i32, mut lo: i32, mut hi: i32) -> i32 {
        /* Examples:
                                          findCodePoint(c)
          set              list[]         c=0 1 3 4 7 8
          ===              ==============   ===========
          []               [110000]         0 0 0 0 0 0
          [\u0000-\u0003]  [0, 4, 110000]   1 1 1 2 2 2
          [\u0004-\u0007]  [4, 8, 110000]   0 0 0 1 1 2
          [:Any:]          [0, 110000]      1 1 1 1 1 1
        */

        // Return the smallest i such that c < list[i]. Assume
        // list[len - 1] == HIGH and that c is legal (0..HIGH-1).
        if c < self.list[lo as usize] {
            return lo;
        }
        // High runner test. c is often after the last range, so an
        // initial check for this condition pays off.
        if lo >= hi || c >= self.list[hi as usize - 1] {
            return hi;
        }
        // invariant: c >= list[lo]
        // invariant: c < list[hi]
        loop {
            let i = ((lo + hi) as u32 >> 1) as i32;
            if i == lo {
                break; // Found!
            } else if c < self.list[i as usize] {
                hi = i;
            } else {
                lo = i;
            }
        }
        hi
    }

    fn contains_slow(&self, c: i32, lo: i32, hi: i32) -> bool {
        0 != (self.find_code_point(c, lo, hi) & 1)
    }
}
