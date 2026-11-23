// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationRootElements`.

use std::sync::Arc;

use crate::r#impl::coll::collation::Collation;

/// Container and access methods for collation elements and weights that
/// occur in the root collator.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationRootElements`; the
/// elements array is shared with the root `CollationData`, as in Java.
#[derive(Debug, Clone)]
pub struct CollationRootElements {
    elements: Arc<[i64]>,
}

const SEC_TER_DELTA_FLAG_L: i64 = CollationRootElements::SEC_TER_DELTA_FLAG as i64;
const PRIMARY_STEP_MASK_L: i64 = CollationRootElements::PRIMARY_STEP_MASK as i64;

impl CollationRootElements {
    /// `CollationRootElements(long[] rootElements)`.
    pub fn new(root_elements: Arc<[i64]>) -> Self {
        Self { elements: root_elements }
    }

    /// Higher than any root primary.
    pub const PRIMARY_SENTINEL: i64 = 0xffffff00;
    /// Flag in a root element, set if the element contains secondary &
    /// tertiary weights, rather than a primary.
    pub const SEC_TER_DELTA_FLAG: i32 = 0x80;
    /// Mask for getting the primary range step value from a primary-range-end
    /// element.
    pub const PRIMARY_STEP_MASK: i32 = 0x7f;
    /// Index of the first CE with a non-zero tertiary weight.
    pub const IX_FIRST_TERTIARY_INDEX: i32 = 0;
    /// Index of the first CE with a non-zero secondary weight.
    pub const IX_FIRST_SECONDARY_INDEX: i32 = 1;
    /// Index of the first CE with a non-zero primary weight.
    pub const IX_FIRST_PRIMARY_INDEX: i32 = 2;
    /// Must match Collation.COMMON_SEC_AND_TER_CE.
    pub const IX_COMMON_SEC_AND_TER_CE: i32 = 3;
    /// Secondary & tertiary boundaries.
    pub const IX_SEC_TER_BOUNDARIES: i32 = 4;
    /// The current number of indexes.
    pub const IX_COUNT: i32 = 5;

    fn e(&self, i: i32) -> i64 {
        self.elements[i as usize]
    }

    /// Returns the boundary between tertiary weights of primary/secondary
    /// CEs and those of tertiary CEs.
    pub fn get_tertiary_boundary(&self) -> i32 {
        ((self.e(Self::IX_SEC_TER_BOUNDARIES) as i32) << 8) & 0xff00
    }

    /// Returns the first assigned tertiary CE.
    pub fn get_first_tertiary_ce(&self) -> i64 {
        self.e(self.e(Self::IX_FIRST_TERTIARY_INDEX) as i32) & !SEC_TER_DELTA_FLAG_L
    }

    /// Returns the last assigned tertiary CE.
    pub fn get_last_tertiary_ce(&self) -> i64 {
        self.e(self.e(Self::IX_FIRST_SECONDARY_INDEX) as i32 - 1) & !SEC_TER_DELTA_FLAG_L
    }

    /// Returns the last common secondary weight.
    pub fn get_last_common_secondary(&self) -> i32 {
        ((self.e(Self::IX_SEC_TER_BOUNDARIES) as i32) >> 16) & 0xff00
    }

    /// Returns the boundary between secondary weights of primary CEs and
    /// those of secondary CEs.
    pub fn get_secondary_boundary(&self) -> i32 {
        ((self.e(Self::IX_SEC_TER_BOUNDARIES) as i32) >> 8) & 0xff00
    }

    /// Returns the first assigned secondary CE.
    pub fn get_first_secondary_ce(&self) -> i64 {
        self.e(self.e(Self::IX_FIRST_SECONDARY_INDEX) as i32) & !SEC_TER_DELTA_FLAG_L
    }

    /// Returns the last assigned secondary CE.
    pub fn get_last_secondary_ce(&self) -> i64 {
        self.e(self.e(Self::IX_FIRST_PRIMARY_INDEX) as i32 - 1) & !SEC_TER_DELTA_FLAG_L
    }

    /// Returns the first assigned primary weight.
    pub fn get_first_primary(&self) -> i64 {
        self.e(self.e(Self::IX_FIRST_PRIMARY_INDEX) as i32) // step=0: cannot be a range end
    }

    /// Returns the first assigned primary CE.
    pub fn get_first_primary_ce(&self) -> i64 {
        Collation::make_ce(self.get_first_primary())
    }

    /// Returns the last root CE with a primary weight before p.
    pub fn last_ce_with_primary_before(&self, mut p: i64) -> i64 {
        if p == 0 {
            return 0;
        }
        debug_assert!(p > self.e(self.e(Self::IX_FIRST_PRIMARY_INDEX) as i32));
        let mut index = self.find_p(p);
        let mut q = self.e(index);
        let mut sec_ter;
        if p == (q & 0xffffff00) {
            // p == elements[index] is a root primary. Find the CE before it.
            // We must not be in a primary range.
            debug_assert!((q & PRIMARY_STEP_MASK_L) == 0);
            sec_ter = self.e(index - 1);
            if (sec_ter & SEC_TER_DELTA_FLAG_L) == 0 {
                // Primary CE just before p.
                p = sec_ter & 0xffffff00;
                sec_ter = Collation::COMMON_SEC_AND_TER_CE as i64;
            } else {
                // secTer = last secondary & tertiary for the previous primary
                index -= 2;
                loop {
                    p = self.e(index);
                    if (p & SEC_TER_DELTA_FLAG_L) == 0 {
                        p &= 0xffffff00;
                        break;
                    }
                    index -= 1;
                }
            }
        } else {
            // p > elements[index] which is the previous primary.
            // Find the last secondary & tertiary weights for it.
            p = q & 0xffffff00;
            sec_ter = Collation::COMMON_SEC_AND_TER_CE as i64;
            loop {
                index += 1;
                q = self.e(index);
                if (q & SEC_TER_DELTA_FLAG_L) == 0 {
                    // We must not be in a primary range.
                    debug_assert!((q & PRIMARY_STEP_MASK_L) == 0);
                    break;
                }
                sec_ter = q;
            }
        }
        (p << 32) | (sec_ter & !SEC_TER_DELTA_FLAG_L)
    }

    /// Returns the first root CE with a primary weight of at least p.
    pub fn first_ce_with_primary_at_least(&self, mut p: i64) -> i64 {
        if p == 0 {
            return 0;
        }
        let mut index = self.find_p(p);
        if p != (self.e(index) & 0xffffff00) {
            loop {
                index += 1;
                p = self.e(index);
                if (p & SEC_TER_DELTA_FLAG_L) == 0 {
                    // First primary after p. We must not be in a primary range.
                    debug_assert!((p & PRIMARY_STEP_MASK_L) == 0);
                    break;
                }
            }
        }
        // The code above guarantees that p has at most 3 bytes: (p & 0xff) == 0.
        (p << 32) | Collation::COMMON_SEC_AND_TER_CE as i64
    }

    /// Returns the primary weight before p.
    pub fn get_primary_before(&self, mut p: i64, is_compressible: bool) -> i64 {
        let mut index = self.find_primary(p);
        let step;
        let q = self.e(index);
        if p == (q & 0xffffff00) {
            // Found p itself. Return the previous primary.
            // See if p is at the end of a previous range.
            step = q as i32 & Self::PRIMARY_STEP_MASK;
            if step == 0 {
                // p is not at the end of a range. Look for the previous primary.
                loop {
                    index -= 1;
                    p = self.e(index);
                    if (p & SEC_TER_DELTA_FLAG_L) == 0 {
                        break;
                    }
                }
                return p & 0xffffff00;
            }
        } else {
            // p is in a range, and not at the start.
            let next_element = self.e(index + 1);
            debug_assert!(Self::is_end_of_primary_range(next_element));
            step = next_element as i32 & Self::PRIMARY_STEP_MASK;
        }
        // Return the previous range primary.
        if (p & 0xffff) == 0 {
            Collation::dec_two_byte_primary_by_one_step(p, is_compressible, step)
        } else {
            Collation::dec_three_byte_primary_by_one_step(p, is_compressible, step)
        }
    }

    /// Returns the secondary weight before \[p, s\].
    pub fn get_secondary_before(&self, p: i64, s: i32) -> i32 {
        let mut index;
        let mut previous_sec;
        let mut sec;
        if p == 0 {
            index = self.e(Self::IX_FIRST_SECONDARY_INDEX) as i32;
            // Gap at the beginning of the secondary CE range.
            previous_sec = 0;
            sec = (self.e(index) >> 16) as i32;
        } else {
            index = self.find_primary(p) + 1;
            previous_sec = Collation::BEFORE_WEIGHT16;
            sec = ((self.get_first_sec_ter_for_primary(index) as i32) as u32 >> 16) as i32;
        }
        debug_assert!(s >= sec);
        while s > sec {
            previous_sec = sec;
            debug_assert!((self.e(index) & SEC_TER_DELTA_FLAG_L) != 0);
            sec = (self.e(index) >> 16) as i32;
            index += 1;
        }
        debug_assert!(sec == s);
        previous_sec
    }

    /// Returns the tertiary weight before \[p, s, t\].
    pub fn get_tertiary_before(&self, p: i64, s: i32, t: i32) -> i32 {
        debug_assert!((t & !Collation::ONLY_TERTIARY_MASK) == 0);
        let mut index;
        let mut previous_ter;
        let mut sec_ter;
        if p == 0 {
            if s == 0 {
                index = self.e(Self::IX_FIRST_TERTIARY_INDEX) as i32;
                // Gap at the beginning of the tertiary CE range.
                previous_ter = 0;
            } else {
                index = self.e(Self::IX_FIRST_SECONDARY_INDEX) as i32;
                previous_ter = Collation::BEFORE_WEIGHT16;
            }
            sec_ter = self.e(index) & !SEC_TER_DELTA_FLAG_L;
        } else {
            index = self.find_primary(p) + 1;
            previous_ter = Collation::BEFORE_WEIGHT16;
            sec_ter = self.get_first_sec_ter_for_primary(index);
        }
        let st = ((s as i64) << 16) | t as i64;
        while st > sec_ter {
            if (sec_ter >> 16) as i32 == s {
                previous_ter = sec_ter as i32;
            }
            debug_assert!((self.e(index) & SEC_TER_DELTA_FLAG_L) != 0);
            sec_ter = self.e(index) & !SEC_TER_DELTA_FLAG_L;
            index += 1;
        }
        debug_assert!(sec_ter == st);
        previous_ter & 0xffff
    }

    /// Finds the index of the input primary. p must occur as a root primary,
    /// and must not be 0.
    pub fn find_primary(&self, p: i64) -> i32 {
        // Requirement: p must occur as a root primary.
        debug_assert!((p & 0xff) == 0); // at most a 3-byte primary
        let index = self.find_p(p);
        // If p is in a range, then we just assume that p is an actual primary in this range.
        // (Too cumbersome/expensive to check.)
        // Otherwise, it must be an exact match.
        debug_assert!(Self::is_end_of_primary_range(self.e(index + 1)) || p == (self.e(index) & 0xffffff00));
        index
    }

    /// Returns the primary weight after p where index=findPrimary(p).
    pub fn get_primary_after(&self, p: i64, mut index: i32, is_compressible: bool) -> i64 {
        debug_assert!(p == (self.e(index) & 0xffffff00) || Self::is_end_of_primary_range(self.e(index + 1)));
        index += 1;
        let mut q = self.e(index);
        let step;
        if (q & SEC_TER_DELTA_FLAG_L) == 0 && {
            step = q as i32 & Self::PRIMARY_STEP_MASK;
            step != 0
        } {
            // Return the next primary in this range.
            if (p & 0xffff) == 0 {
                Collation::inc_two_byte_primary_by_offset(p, is_compressible, step)
            } else {
                Collation::inc_three_byte_primary_by_offset(p, is_compressible, step)
            }
        } else {
            // Return the next primary in the list.
            while (q & SEC_TER_DELTA_FLAG_L) != 0 {
                index += 1;
                q = self.e(index);
            }
            debug_assert!((q & PRIMARY_STEP_MASK_L) == 0);
            q
        }
    }

    /// Returns the secondary weight after \[p, s\] where
    /// index=findPrimary(p) except use index=0 for p=0.
    pub fn get_secondary_after(&self, mut index: i32, s: i32) -> i32 {
        let mut sec_ter;
        let sec_limit;
        if index == 0 {
            // primary = 0
            debug_assert!(s != 0);
            index = self.e(Self::IX_FIRST_SECONDARY_INDEX) as i32;
            sec_ter = self.e(index);
            // Gap at the end of the secondary CE range.
            sec_limit = 0x10000;
        } else {
            debug_assert!(index >= self.e(Self::IX_FIRST_PRIMARY_INDEX) as i32);
            sec_ter = self.get_first_sec_ter_for_primary(index + 1);
            // If this is an explicit sec/ter unit, then it will be read once more.
            // Gap for secondaries of primary CEs.
            sec_limit = self.get_secondary_boundary();
        }
        loop {
            let sec = (sec_ter >> 16) as i32;
            if sec > s {
                return sec;
            }
            index += 1;
            sec_ter = self.e(index);
            if (sec_ter & SEC_TER_DELTA_FLAG_L) == 0 {
                return sec_limit;
            }
        }
    }

    /// Returns the tertiary weight after \[p, s, t\] where
    /// index=findPrimary(p) except use index=0 for p=0.
    pub fn get_tertiary_after(&self, mut index: i32, s: i32, t: i32) -> i32 {
        let mut sec_ter;
        let ter_limit;
        if index == 0 {
            // primary = 0
            if s == 0 {
                debug_assert!(t != 0);
                index = self.e(Self::IX_FIRST_TERTIARY_INDEX) as i32;
                // Gap at the end of the tertiary CE range.
                ter_limit = 0x4000;
            } else {
                index = self.e(Self::IX_FIRST_SECONDARY_INDEX) as i32;
                // Gap for tertiaries of primary/secondary CEs.
                ter_limit = self.get_tertiary_boundary();
            }
            sec_ter = self.e(index) & !SEC_TER_DELTA_FLAG_L;
        } else {
            debug_assert!(index >= self.e(Self::IX_FIRST_PRIMARY_INDEX) as i32);
            sec_ter = self.get_first_sec_ter_for_primary(index + 1);
            // If this is an explicit sec/ter unit, then it will be read once more.
            ter_limit = self.get_tertiary_boundary();
        }
        let st = (((s as i64) & 0xffffffff) << 16) | t as i64;
        loop {
            if sec_ter > st {
                debug_assert!((sec_ter >> 16) == s as i64);
                return sec_ter as i32 & 0xffff;
            }
            index += 1;
            sec_ter = self.e(index);
            // No tertiary greater than t for this primary+secondary.
            if (sec_ter & SEC_TER_DELTA_FLAG_L) == 0 || (sec_ter >> 16) > s as i64 {
                return ter_limit;
            }
            sec_ter &= !SEC_TER_DELTA_FLAG_L;
        }
    }

    /// Returns the first secondary & tertiary weights for p where
    /// index=findPrimary(p)+1.
    fn get_first_sec_ter_for_primary(&self, index: i32) -> i64 {
        let mut sec_ter = self.e(index);
        if (sec_ter & SEC_TER_DELTA_FLAG_L) == 0 {
            // No sec/ter delta.
            return Collation::COMMON_SEC_AND_TER_CE as i64;
        }
        sec_ter &= !SEC_TER_DELTA_FLAG_L;
        if sec_ter > Collation::COMMON_SEC_AND_TER_CE as i64 {
            // Implied sec/ter.
            return Collation::COMMON_SEC_AND_TER_CE as i64;
        }
        // Explicit sec/ter below common/common.
        sec_ter
    }

    /// Finds the largest index i where elements\[i\]<=p.
    fn find_p(&self, p: i64) -> i32 {
        // p need not occur as a root primary.
        // For example, it might be a reordering group boundary.
        debug_assert!((p >> 24) != Collation::UNASSIGNED_IMPLICIT_BYTE as i64);
        // modified binary search
        let mut start = self.e(Self::IX_FIRST_PRIMARY_INDEX) as i32;
        debug_assert!(p >= self.e(start));
        let mut limit = self.elements.len() as i32 - 1;
        debug_assert!(self.e(limit) >= Self::PRIMARY_SENTINEL);
        debug_assert!(p < self.e(limit));
        while (start + 1) < limit {
            // Invariant: elements[start] and elements[limit] are primaries,
            // and elements[start]<=p<=elements[limit].
            let mut i = ((start as i64 + limit as i64) / 2) as i32;
            let mut q = self.e(i);
            if (q & SEC_TER_DELTA_FLAG_L) != 0 {
                // Find the next primary.
                let mut j = i + 1;
                loop {
                    if j == limit {
                        break;
                    }
                    q = self.e(j);
                    if (q & SEC_TER_DELTA_FLAG_L) == 0 {
                        i = j;
                        break;
                    }
                    j += 1;
                }
                if (q & SEC_TER_DELTA_FLAG_L) != 0 {
                    // Find the preceding primary.
                    j = i - 1;
                    loop {
                        if j == start {
                            break;
                        }
                        q = self.e(j);
                        if (q & SEC_TER_DELTA_FLAG_L) == 0 {
                            i = j;
                            break;
                        }
                        j -= 1;
                    }
                    if (q & SEC_TER_DELTA_FLAG_L) != 0 {
                        // No primary between start and limit.
                        break;
                    }
                }
            }
            if p < (q & 0xffffff00) {
                // Reset the "step" bits of a range end primary.
                limit = i;
            } else {
                start = i;
            }
        }
        start
    }

    fn is_end_of_primary_range(q: i64) -> bool {
        (q & SEC_TER_DELTA_FLAG_L) == 0 && (q & PRIMARY_STEP_MASK_L) != 0
    }
}
