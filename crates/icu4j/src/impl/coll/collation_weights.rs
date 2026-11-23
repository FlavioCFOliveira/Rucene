// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationWeights`.

use crate::r#impl::coll::collation::Collation;

/// `CollationWeights.WeightRange` (private): a range of weights with the
/// same length.
///
/// Java's ranges are objects that `ranges[]` references; each is
/// referenced by at most one slot, so they are values here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WeightRange {
    start: i64,
    end: i64,
    length: i32,
    count: i32,
}

/// Allocates n collation element weights between two exclusive limits.
/// Used only internally by the collation tailoring builder.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationWeights`.
#[derive(Debug, Clone, Default)]
pub struct CollationWeights {
    middle_length: i32,
    /// for byte 1, 2, 3, 4
    min_bytes: [i32; 5],
    max_bytes: [i32; 5],
    ranges: [Option<WeightRange>; 7],
    range_index: i32,
    range_count: i32,
}

fn get_weight_trail(weight: i64, length: i32) -> i32 {
    (weight >> (8 * (4 - length))) as i32 & 0xff
}

fn set_weight_trail(weight: i64, mut length: i32, trail: i32) -> i64 {
    length = 8 * (4 - length);
    (weight & (0xffffff00i64 << length)) | ((trail as i64) << length)
}

fn get_weight_byte(weight: i64, idx: i32) -> i32 {
    get_weight_trail(weight, idx)
}

fn set_weight_byte(weight: i64, mut idx: i32, b: i32) -> i64 {
    let mut mask: i64;
    idx *= 8;
    if idx < 32 {
        mask = 0xffffffffi64 >> idx;
    } else {
        mask = 0;
    }
    idx = 32 - idx;
    mask |= 0xffffff00i64 << idx;
    (weight & mask) | ((b as i64) << idx)
}

fn truncate_weight(weight: i64, length: i32) -> i64 {
    weight & (0xffffffffi64 << (8 * (4 - length)))
}

fn inc_weight_trail(weight: i64, length: i32) -> i64 {
    weight + (1i64 << (8 * (4 - length)))
}

fn dec_weight_trail(weight: i64, length: i32) -> i64 {
    weight - (1i64 << (8 * (4 - length)))
}

impl CollationWeights {
    /// `CollationWeights()`.
    pub fn new() -> Self {
        Self::default()
    }

    fn r(&self, i: i32) -> &WeightRange {
        self.ranges[i as usize].as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    fn rm(&mut self, i: i32) -> &mut WeightRange {
        self.ranges[i as usize].as_mut().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `initForPrimary(boolean)`.
    pub fn init_for_primary(&mut self, compressible: bool) {
        self.middle_length = 1;
        self.min_bytes[1] = Collation::MERGE_SEPARATOR_BYTE + 1;
        self.max_bytes[1] = Collation::TRAIL_WEIGHT_BYTE;
        if compressible {
            self.min_bytes[2] = Collation::PRIMARY_COMPRESSION_LOW_BYTE + 1;
            self.max_bytes[2] = Collation::PRIMARY_COMPRESSION_HIGH_BYTE - 1;
        } else {
            self.min_bytes[2] = 2;
            self.max_bytes[2] = 0xff;
        }
        self.min_bytes[3] = 2;
        self.max_bytes[3] = 0xff;
        self.min_bytes[4] = 2;
        self.max_bytes[4] = 0xff;
    }

    /// `initForSecondary()`.
    pub fn init_for_secondary(&mut self) {
        // We use only the lower 16 bits for secondary weights.
        self.middle_length = 3;
        self.min_bytes[1] = 0;
        self.max_bytes[1] = 0;
        self.min_bytes[2] = 0;
        self.max_bytes[2] = 0;
        self.min_bytes[3] = Collation::LEVEL_SEPARATOR_BYTE + 1;
        self.max_bytes[3] = 0xff;
        self.min_bytes[4] = 2;
        self.max_bytes[4] = 0xff;
    }

    /// `initForTertiary()`.
    pub fn init_for_tertiary(&mut self) {
        // We use only the lower 16 bits for tertiary weights.
        self.middle_length = 3;
        self.min_bytes[1] = 0;
        self.max_bytes[1] = 0;
        self.min_bytes[2] = 0;
        self.max_bytes[2] = 0;
        // We use only 6 bits per byte.
        // The other bits are used for case & quaternary weights.
        self.min_bytes[3] = Collation::LEVEL_SEPARATOR_BYTE + 1;
        self.max_bytes[3] = 0x3f;
        self.min_bytes[4] = 2;
        self.max_bytes[4] = 0x3f;
    }

    /// Determine heuristically what ranges to use for a given number of
    /// weights between (excluding) two limits.
    ///
    /// Equivalent to `allocWeights(long, long, int)`.
    pub fn alloc_weights(&mut self, lower_limit: i64, upper_limit: i64, n: i32) -> bool {
        // Call getWeightRanges() and then determine heuristically
        // which ranges to use for a given number of weights between (excluding)
        // two limits.
        if !self.get_weight_ranges(lower_limit, upper_limit) {
            return false;
        }

        // try until we find suitably large ranges
        loop {
            // get the smallest number of bytes in a range
            let min_length = self.r(0).length;

            if self.alloc_weights_in_short_ranges(n, min_length) {
                break;
            }

            if min_length == 4 {
                return false;
            }

            if self.alloc_weights_in_min_length_ranges(n, min_length) {
                break;
            }

            // no good match, lengthen all minLength ranges and iterate
            let mut i = 0;
            while i < self.range_count && self.r(i).length == min_length {
                let mut range = *self.r(i);
                self.lengthen_range(&mut range);
                *self.rm(i) = range;
                i += 1;
            }
        }

        // set the first range for nextWeight()
        self.range_index = 0;
        if (self.range_count as usize) < self.ranges.len() {
            self.ranges[self.range_count as usize] = None; // force a crash when going out of bounds
        }
        true
    }

    /// Given a set of ranges calculated by allocWeights(), iterate through
    /// the weights. The ranges are modified to keep the current iteration
    /// state.
    ///
    /// Equivalent to `nextWeight()`.
    pub fn next_weight(&mut self) -> i64 {
        if self.range_index >= self.range_count {
            0xffffffff
        } else {
            // get the next weight
            let idx = self.range_index;
            let weight = self.r(idx).start;
            self.rm(idx).count -= 1;
            if self.r(idx).count == 0 {
                // this range is finished
                self.range_index += 1;
            } else {
                // increment the weight for the next value
                let length = self.r(idx).length;
                let start = self.inc_weight(weight, length);
                self.rm(idx).start = start;
                debug_assert!(self.r(idx).start <= self.r(idx).end);
            }

            weight
        }
    }

    /// `lengthOfWeight(long)`.
    pub fn length_of_weight(weight: i64) -> i32 {
        if (weight & 0xffffff) == 0 {
            1
        } else if (weight & 0xffff) == 0 {
            2
        } else if (weight & 0xff) == 0 {
            3
        } else {
            4
        }
    }

    fn count_bytes(&self, idx: i32) -> i32 {
        self.max_bytes[idx as usize] - self.min_bytes[idx as usize] + 1
    }

    fn inc_weight(&self, mut weight: i64, mut length: i32) -> i64 {
        loop {
            let b = get_weight_byte(weight, length);
            if b < self.max_bytes[length as usize] {
                return set_weight_byte(weight, length, b + 1);
            } else {
                // Roll over, set this byte to the minimum and increment the previous one.
                weight = set_weight_byte(weight, length, self.min_bytes[length as usize]);
                length -= 1;
                debug_assert!(length > 0);
            }
        }
    }

    fn inc_weight_by_offset(&self, mut weight: i64, mut length: i32, mut offset: i32) -> i64 {
        loop {
            offset += get_weight_byte(weight, length);
            if offset <= self.max_bytes[length as usize] {
                return set_weight_byte(weight, length, offset);
            } else {
                // Split the offset between this byte and the previous one.
                offset -= self.min_bytes[length as usize];
                weight =
                    set_weight_byte(weight, length, self.min_bytes[length as usize] + offset % self.count_bytes(length));
                offset /= self.count_bytes(length);
                length -= 1;
                debug_assert!(length > 0);
            }
        }
    }

    fn lengthen_range(&self, range: &mut WeightRange) {
        let length = range.length + 1;
        range.start = set_weight_trail(range.start, length, self.min_bytes[length as usize]);
        range.end = set_weight_trail(range.end, length, self.max_bytes[length as usize]);
        range.count *= self.count_bytes(length);
        range.length = length;
    }

    /// Takes two CE weights and calculates the possible ranges of weights
    /// between the two limits, excluding them.
    fn get_weight_ranges(&mut self, lower_limit: i64, upper_limit: i64) -> bool {
        debug_assert!(lower_limit != 0);
        debug_assert!(upper_limit != 0);

        // get the lengths of the limits
        let lower_length = Self::length_of_weight(lower_limit);
        let upper_length = Self::length_of_weight(upper_limit);

        debug_assert!(lower_length >= self.middle_length);
        // Permit upperLength<middleLength: The upper limit for secondaries is 0x10000.

        if lower_limit >= upper_limit {
            return false;
        }

        // check that neither is a prefix of the other
        if lower_length < upper_length && lower_limit == truncate_weight(upper_limit, lower_length) {
            return false;
        }
        // if the upper limit is a prefix of the lower limit then the earlier test lowerLimit>=upperLimit has caught it

        let mut lower: [Option<WeightRange>; 5] = [None; 5]; // [0] and [1] are not used - this simplifies indexing
        let mut middle = WeightRange::default();
        let mut upper: [Option<WeightRange>; 5] = [None; 5];

        // With the limit lengths of 1..4, there are up to 7 ranges for allocation:
        // range     minimum length
        // lower[4]  4
        // lower[3]  3
        // lower[2]  2
        // middle    1
        // upper[2]  2
        // upper[3]  3
        // upper[4]  4
        //
        // We are now going to calculate up to 7 ranges.
        // Some of them will typically overlap, so we will then have to merge and eliminate ranges.
        let mut weight = lower_limit;
        let mut length = lower_length;
        while length > self.middle_length {
            let trail = get_weight_trail(weight, length);
            if trail < self.max_bytes[length as usize] {
                lower[length as usize] = Some(WeightRange {
                    start: inc_weight_trail(weight, length),
                    end: set_weight_trail(weight, length, self.max_bytes[length as usize]),
                    length,
                    count: self.max_bytes[length as usize] - trail,
                });
            }
            weight = truncate_weight(weight, length - 1);
            length -= 1;
        }
        if weight < 0xff000000 {
            middle.start = inc_weight_trail(weight, self.middle_length);
        } else {
            // Prevent overflow for primary lead byte FF
            // which would yield a middle range starting at 0.
            middle.start = 0xffffffff; // no middle range
        }

        weight = upper_limit;
        length = upper_length;
        while length > self.middle_length {
            let trail = get_weight_trail(weight, length);
            if trail > self.min_bytes[length as usize] {
                upper[length as usize] = Some(WeightRange {
                    start: set_weight_trail(weight, length, self.min_bytes[length as usize]),
                    end: dec_weight_trail(weight, length),
                    length,
                    count: trail - self.min_bytes[length as usize],
                });
            }
            weight = truncate_weight(weight, length - 1);
            length -= 1;
        }
        middle.end = dec_weight_trail(weight, self.middle_length);

        // set the middle range
        middle.length = self.middle_length;
        if middle.end >= middle.start {
            middle.count = ((middle.end - middle.start) >> (8 * (4 - self.middle_length))) as i32 + 1;
        } else {
            // Merge overlapping ranges, or join adjacent ones.
            let mut length = 4;
            while length > self.middle_length {
                let li = length as usize;
                if let (Some(lo), Some(up)) = (lower[li], upper[li]) {
                    if lo.count > 0 && up.count > 0 {
                        // Note: The lowerEnd and upperStart weights are versions of
                        // lowerLimit and upperLimit (which are lowerLimit<upperLimit),
                        // truncated (still less-or-equal)
                        // and then with their last bytes changed to the
                        // maxByte (for lowerEnd) or minByte (for upperStart).
                        let lower_end = lo.end;
                        let upper_start = up.start;
                        let mut merged = false;

                        if lower_end > upper_start {
                            // These two lower and upper ranges collide.
                            // Since lowerLimit<upperLimit and lowerEnd and upperStart
                            // are versions with only their last bytes modified
                            // (and following ones removed/reset to 0),
                            // lowerEnd>upperStart is only possible
                            // if the leading bytes are equal
                            // and lastByte(lowerEnd)>lastByte(upperStart).
                            debug_assert!(truncate_weight(lower_end, length - 1) == truncate_weight(upper_start, length - 1));
                            // Intersect these two ranges.
                            let lo_m = lower[li].as_mut().expect("checked");
                            lo_m.end = up.end;
                            lo_m.count = get_weight_trail(lo_m.end, length) - get_weight_trail(lo_m.start, length) + 1;
                            // count might be <= 0 in which case there is no room,
                            // and the range-collecting code below will ignore this range.
                            merged = true;
                        } else if lower_end == upper_start {
                            // Not possible, unless minByte==maxByte which is not allowed.
                            debug_assert!(self.min_bytes[li] < self.max_bytes[li]);
                        } else {
                            /* lowerEnd<upperStart */
                            if self.inc_weight(lower_end, length) == upper_start {
                                // Join adjacent ranges.
                                let lo_m = lower[li].as_mut().expect("checked");
                                lo_m.end = up.end;
                                lo_m.count += up.count; // might be >countBytes
                                merged = true;
                            }
                        }
                        if merged {
                            // Remove all shorter ranges.
                            // There was no room available for them between the ranges.
                            upper[li].as_mut().expect("checked").count = 0;
                            length -= 1;
                            while length > self.middle_length {
                                lower[length as usize] = None;
                                upper[length as usize] = None;
                                length -= 1;
                            }
                            break;
                        }
                    }
                }
                length -= 1;
            }
        }

        // Copy the ranges, shortest first, into the result array.
        self.range_count = 0;
        if middle.count > 0 {
            self.ranges[0] = Some(middle);
            self.range_count = 1;
        }
        for length in self.middle_length + 1..=4 {
            // copy upper first so that later the middle range is more likely the first one to use
            if let Some(up) = upper[length as usize].filter(|r| r.count > 0) {
                self.ranges[self.range_count as usize] = Some(up);
                self.range_count += 1;
            }
            if let Some(lo) = lower[length as usize].filter(|r| r.count > 0) {
                self.ranges[self.range_count as usize] = Some(lo);
                self.range_count += 1;
            }
        }
        self.range_count > 0
    }

    fn alloc_weights_in_short_ranges(&mut self, mut n: i32, min_length: i32) -> bool {
        // See if the first few minLength and minLength+1 ranges have enough weights.
        let mut i = 0;
        while i < self.range_count && self.r(i).length <= (min_length + 1) {
            if n <= self.r(i).count {
                // Use the first few minLength and minLength+1 ranges.
                if self.r(i).length > min_length {
                    // Reduce the number of weights from the last minLength+1 range
                    // which might sort before some minLength ranges,
                    // so that we use all weights in the minLength ranges.
                    self.rm(i).count = n;
                }
                self.range_count = i + 1;
                // Sort the ranges by weight values.
                if self.range_count > 1 {
                    self.ranges[..self.range_count as usize]
                        .sort_by_key(|r| r.unwrap_or_else(|| panic!("java.lang.NullPointerException")).start);
                }
                return true;
            }
            n -= self.r(i).count; // still >0
            i += 1;
        }
        false
    }

    fn alloc_weights_in_min_length_ranges(&mut self, n: i32, min_length: i32) -> bool {
        // See if the minLength ranges have enough weights
        // when we split one and lengthen the following ones.
        let mut count = 0;
        let mut min_length_range_count = 0;
        while min_length_range_count < self.range_count && self.r(min_length_range_count).length == min_length {
            count += self.r(min_length_range_count).count;
            min_length_range_count += 1;
        }

        let next_count_bytes = self.count_bytes(min_length + 1);
        if n > count * next_count_bytes {
            return false;
        }

        // Use the minLength ranges. Merge them, and then split again as necessary.
        let mut start = self.r(0).start;
        let mut end = self.r(0).end;
        for i in 1..min_length_range_count {
            if self.r(i).start < start {
                start = self.r(i).start;
            }
            if self.r(i).end > end {
                end = self.r(i).end;
            }
        }

        // Calculate how to split the range between minLength (count1) and minLength+1 (count2).
        // Goal:
        //   count1 + count2 * nextCountBytes = n
        //   count1 + count2 = count
        // These turn into
        //   (count - count2) + count2 * nextCountBytes = n
        // and then into the following count1 & count2 computations.
        let mut count2 = (n - count) / (next_count_bytes - 1); // number of weights to be lengthened
        let mut count1 = count - count2; // number of minLength weights
        if count2 == 0 || (count1 + count2 * next_count_bytes) < n {
            // round up
            count2 += 1;
            count1 -= 1;
            debug_assert!((count1 + count2 * next_count_bytes) >= n);
        }

        self.rm(0).start = start;

        if count1 == 0 {
            // Make one long range.
            self.rm(0).end = end;
            self.rm(0).count = count;
            let mut r0 = *self.r(0);
            self.lengthen_range(&mut r0);
            self.ranges[0] = Some(r0);
            self.range_count = 1;
        } else {
            // Split the range, lengthen the second part.
            let end0 = self.inc_weight_by_offset(start, min_length, count1 - 1);
            self.rm(0).end = end0;
            self.rm(0).count = count1;

            if self.ranges[1].is_none() {
                self.ranges[1] = Some(WeightRange::default());
            }
            let start1 = self.inc_weight(end0, min_length);
            let mut r1 = WeightRange {
                start: start1,
                end,
                length: min_length, // +1 when lengthened
                count: count2,      // *countBytes when lengthened
            };
            self.lengthen_range(&mut r1);
            self.ranges[1] = Some(r1);
            self.range_count = 2;
        }
        true
    }
}
