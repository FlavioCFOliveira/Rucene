// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Normalizer2Impl`.
//!
//! Only the nested class `UTF16Plus` is here, ported early because
//! `CodePointTrie` (block 1, rmp task #643) calls it; the rest of the file
//! is ported by rmp task #646.

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
    pub fn equal_range(s1: &[u16], mut start1: i32, limit1: i32, s2: &[u16], mut start2: i32, limit2: i32) -> bool {
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
