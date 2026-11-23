// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.USerializedSet`.

use rucene::error::Result;

use crate::jdk::{illegal_argument_no_message, index_out_of_bounds};

/// Simple class for handling serialized USet/UnicodeSet structures without
/// object creation.
///
/// Equivalent to `com.ibm.icu.impl.USerializedSet`.
#[derive(Debug, Clone)]
pub struct USerializedSet {
    array: Option<Vec<u16>>,
    array_offset: i32,
    bmp_length: i32,
    length: i32,
}

impl Default for USerializedSet {
    fn default() -> Self {
        Self { array: Some(vec![0; 8]), array_offset: 0, bmp_length: 0, length: 0 }
    }
}

impl USerializedSet {
    /// `new USerializedSet()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getSet(char src[], int srcStart)`: fills the set from serialized
    /// data.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `src` is too short.
    pub fn get_set(&mut self, src: &[u16], mut src_start: i32) -> bool {
        // leave most argument checking up to Java exceptions
        self.array = None;
        self.array_offset = 0;
        self.bmp_length = 0;
        self.length = src[src_start as usize] as i32;
        src_start += 1;

        if (self.length & 0x8000) != 0 {
            /* there are supplementary values */
            self.length &= 0x7fff;
            if (src.len() as i32) < (src_start + 1 + self.length) {
                self.length = 0;
                index_out_of_bounds(None);
            }
            self.bmp_length = src[src_start as usize] as i32;
            src_start += 1;
        } else {
            /* only BMP values */
            if (src.len() as i32) < (src_start + self.length) {
                self.length = 0;
                index_out_of_bounds(None);
            }
            self.bmp_length = self.length;
        }
        self.array = Some(src[src_start as usize..(src_start + self.length) as usize].to_vec());
        true
    }

    /// `setToOne(int c)`: a set of one code point.
    ///
    /// # Panics
    ///
    /// `NullPointerException` (as a panic) after a failed `getSet`, as Java's
    /// write to a null array.
    pub fn set_to_one(&mut self, mut c: i32) {
        if 0x10ffff < c {
            return;
        }
        let array = self.array.as_mut().expect("java.lang.NullPointerException");
        if c < 0xffff {
            self.bmp_length = 2;
            self.length = 2;
            array[0] = c as u16;
            array[1] = (c + 1) as u16;
        } else if c == 0xffff {
            self.bmp_length = 1;
            self.length = 3;
            array[0] = 0xffff;
            array[1] = 1;
            array[2] = 0;
        } else if c < 0x10ffff {
            self.bmp_length = 0;
            self.length = 4;
            array[0] = (c >> 16) as u16;
            array[1] = c as u16;
            c += 1;
            array[2] = (c >> 16) as u16;
            array[3] = c as u16;
        } else
        /* c==0x10ffff */
        {
            self.bmp_length = 0;
            self.length = 2;
            array[0] = 0x10;
            array[1] = 0xffff;
        }
    }

    /// `getRange(int rangeIndex, int[] range)`: the code point range
    /// `[range[0], range[1]]`, false when `rangeIndex` is out of range.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `range` has fewer than 2 elements.
    pub fn get_range(&mut self, mut range_index: i32, range: &mut [i32]) -> Result<bool> {
        if range_index < 0 {
            return Ok(false);
        }
        if self.array.is_none() {
            self.array = Some(vec![0; 8]);
        }
        if range.len() < 2 {
            return Err(illegal_argument_no_message());
        }
        let array = self.array.as_ref().expect("set above");
        let at = |i: i32| array[i as usize] as i32;
        range_index *= 2; /* address start/limit pairs */
        if range_index < self.bmp_length {
            range[0] = at(range_index);
            range_index += 1;
            if range_index < self.bmp_length {
                range[1] = at(range_index) - 1;
            } else if range_index < self.length {
                range[1] = ((at(range_index) << 16) | at(range_index + 1)) - 1;
            } else {
                range[1] = 0x10ffff;
            }
            Ok(true)
        } else {
            range_index -= self.bmp_length;
            range_index *= 2; /* address pairs of pairs of units */
            let supp_length = self.length - self.bmp_length;
            if range_index < supp_length {
                let offset = self.array_offset + self.bmp_length;
                range[0] = (at(offset + range_index) << 16) | at(offset + range_index + 1);
                range_index += 2;
                if range_index < supp_length {
                    range[1] = ((at(offset + range_index) << 16) | at(offset + range_index + 1)) - 1;
                } else {
                    range[1] = 0x10ffff;
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    /// `contains(int c)`.
    ///
    /// # Panics
    ///
    /// `NullPointerException` (as a panic) after a failed `getSet`.
    pub fn contains(&self, c: i32) -> bool {
        if c > 0x10ffff {
            return false;
        }
        let array = self.array.as_ref().expect("java.lang.NullPointerException");
        if c <= 0xffff {
            let mut i = 0;
            /* find c in the BMP part */
            while i < self.bmp_length && (c as u16) >= array[i as usize] {
                i += 1;
            }
            (i & 1) != 0
        } else {
            /* find c in the supplementary part */
            let high = (c >> 16) as u16;
            let low = c as u16;
            let mut i = self.bmp_length;
            while i < self.length
                && (high > array[i as usize] || (high == array[i as usize] && low >= array[i as usize + 1]))
            {
                i += 2;
            }
            /* count pairs of 16-bit units even per BMP and check if the number of pairs is odd */
            ((i + self.bmp_length) & 2) != 0
        }
    }

    /// `countRanges()`.
    pub fn count_ranges(&self) -> i32 {
        (self.bmp_length + (self.length - self.bmp_length) / 2 + 1) / 2
    }
}
