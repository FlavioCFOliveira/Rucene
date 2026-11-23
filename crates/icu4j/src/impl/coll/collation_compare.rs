// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationCompare`.

use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_iterator::CollationIterator;
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::text::collator;

/// Java's `long >>> 32`.
fn ushr32(x: i64) -> i64 {
    ((x as u64) >> 32) as i64
}

/// Java's `int >>> 16`.
fn ushr16(x: i32) -> i32 {
    ((x as u32) >> 16) as i32
}

/// Compares two strings by their collation elements.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationCompare` (static).
#[derive(Debug)]
pub struct CollationCompare;

impl CollationCompare {
    /// `compareUpToQuaternary(CollationIterator, CollationIterator, CollationSettings)`.
    pub fn compare_up_to_quaternary(
        left: &mut dyn CollationIterator,
        right: &mut dyn CollationIterator,
        settings: &CollationSettings,
    ) -> i32 {
        let options = settings.options;
        let variable_top: i64 = if (options & CollationSettings::ALTERNATE_MASK) == 0 {
            0
        } else {
            // +1 so that we can use "<" and primary ignorables test out early.
            settings.variable_top + 1
        };
        let mut any_variable = false;

        // Fetch CEs, compare primaries, store secondary & tertiary weights.
        loop {
            // We fetch CEs until we get a non-ignorable primary or reach the end.
            let mut left_primary: i64;
            loop {
                let mut ce = left.next_ce();
                left_primary = ushr32(ce);
                if left_primary < variable_top && left_primary > Collation::MERGE_SEPARATOR_PRIMARY {
                    // Variable CE, shift it to quaternary level.
                    // Ignore all following primary ignorables, and shift further variable CEs.
                    any_variable = true;
                    loop {
                        // Store only the primary of the variable CE.
                        left.base_mut().set_current_ce(ce & (0xffffffff00000000u64 as i64));
                        loop {
                            ce = left.next_ce();
                            left_primary = ushr32(ce);
                            if left_primary == 0 {
                                left.base_mut().set_current_ce(0);
                            } else {
                                break;
                            }
                        }
                        if !(left_primary < variable_top && left_primary > Collation::MERGE_SEPARATOR_PRIMARY) {
                            break;
                        }
                    }
                }
                if left_primary != 0 {
                    break;
                }
            }

            let mut right_primary: i64;
            loop {
                let mut ce = right.next_ce();
                right_primary = ushr32(ce);
                if right_primary < variable_top && right_primary > Collation::MERGE_SEPARATOR_PRIMARY {
                    // Variable CE, shift it to quaternary level.
                    // Ignore all following primary ignorables, and shift further variable CEs.
                    any_variable = true;
                    loop {
                        // Store only the primary of the variable CE.
                        right.base_mut().set_current_ce(ce & (0xffffffff00000000u64 as i64));
                        loop {
                            ce = right.next_ce();
                            right_primary = ushr32(ce);
                            if right_primary == 0 {
                                right.base_mut().set_current_ce(0);
                            } else {
                                break;
                            }
                        }
                        if !(right_primary < variable_top && right_primary > Collation::MERGE_SEPARATOR_PRIMARY) {
                            break;
                        }
                    }
                }
                if right_primary != 0 {
                    break;
                }
            }

            if left_primary != right_primary {
                // Return the primary difference, with script reordering.
                if settings.has_reordering() {
                    left_primary = settings.reorder(left_primary);
                    right_primary = settings.reorder(right_primary);
                }
                return if left_primary < right_primary { Collation::LESS } else { Collation::GREATER };
            }
            if left_primary == Collation::NO_CE_PRIMARY {
                break;
            }
        }

        // Compare the buffered secondary & tertiary weights.
        // We might skip the secondary level but continue with the case level
        // which is turned on separately.
        if CollationSettings::get_strength_of(options) >= collator::SECONDARY {
            if (options & CollationSettings::BACKWARD_SECONDARY) == 0 {
                let mut left_index = 0;
                let mut right_index = 0;
                loop {
                    let mut left_secondary;
                    loop {
                        left_secondary = ushr16(left.base().get_ce(left_index) as i32);
                        left_index += 1;
                        if left_secondary != 0 {
                            break;
                        }
                    }
                    let mut right_secondary;
                    loop {
                        right_secondary = ushr16(right.base().get_ce(right_index) as i32);
                        right_index += 1;
                        if right_secondary != 0 {
                            break;
                        }
                    }
                    if left_secondary != right_secondary {
                        return if left_secondary < right_secondary { Collation::LESS } else { Collation::GREATER };
                    }
                    if left_secondary == Collation::NO_CE_WEIGHT16 {
                        break;
                    }
                }
            } else {
                // The backwards secondary level compares secondary weights backwards
                // within segments separated by the merge separator (U+FFFE, weight 02).
                let mut left_start = 0;
                let mut right_start = 0;
                loop {
                    // Find the merge separator or the NO_CE terminator.
                    let mut p;
                    let mut left_limit = left_start;
                    loop {
                        p = ushr32(left.base().get_ce(left_limit));
                        if p > Collation::MERGE_SEPARATOR_PRIMARY || p == 0 {
                            left_limit += 1;
                        } else {
                            break;
                        }
                    }
                    let mut right_limit = right_start;
                    loop {
                        p = ushr32(right.base().get_ce(right_limit));
                        if p > Collation::MERGE_SEPARATOR_PRIMARY || p == 0 {
                            right_limit += 1;
                        } else {
                            break;
                        }
                    }

                    // Compare the segments.
                    let mut left_index = left_limit;
                    let mut right_index = right_limit;
                    loop {
                        let mut left_secondary = 0;
                        while left_secondary == 0 && left_index > left_start {
                            left_index -= 1;
                            left_secondary = ushr16(left.base().get_ce(left_index) as i32);
                        }

                        let mut right_secondary = 0;
                        while right_secondary == 0 && right_index > right_start {
                            right_index -= 1;
                            right_secondary = ushr16(right.base().get_ce(right_index) as i32);
                        }

                        if left_secondary != right_secondary {
                            return if left_secondary < right_secondary { Collation::LESS } else { Collation::GREATER };
                        }
                        if left_secondary == 0 {
                            break;
                        }
                    }

                    // Did we reach the end of either string?
                    // Both strings have the same number of merge separators,
                    // or else there would have been a primary-level difference.
                    debug_assert!(left.base().get_ce(left_limit) == right.base().get_ce(right_limit));
                    if p == Collation::NO_CE_PRIMARY {
                        break;
                    }
                    // Skip both merge separators and continue.
                    left_start = left_limit + 1;
                    right_start = right_limit + 1;
                }
            }
        }

        if (options & CollationSettings::CASE_LEVEL) != 0 {
            let strength = CollationSettings::get_strength_of(options);
            let mut left_index = 0;
            let mut right_index = 0;
            loop {
                let mut left_case: i32;
                let left_lower32: i32;
                let mut right_case: i32;
                if strength == collator::PRIMARY {
                    // Primary+caseLevel: Ignore case level weights of primary ignorables.
                    // Otherwise we would get a-umlaut > a
                    // which is not desirable for accent-insensitive sorting.
                    // Check for (lower 32 bits) == 0 as well because variable CEs are stored
                    // with only primary weights.
                    let mut ce;
                    loop {
                        ce = left.base().get_ce(left_index);
                        left_index += 1;
                        left_case = ce as i32;
                        if !(ushr32(ce) == 0 || left_case == 0) {
                            break;
                        }
                    }
                    left_lower32 = left_case;
                    left_case &= 0xc000;

                    loop {
                        ce = right.base().get_ce(right_index);
                        right_index += 1;
                        right_case = ce as i32;
                        if !(ushr32(ce) == 0 || right_case == 0) {
                            break;
                        }
                    }
                    right_case &= 0xc000;
                } else {
                    // Secondary+caseLevel: By analogy with the above,
                    // ignore case level weights of secondary ignorables.
                    //
                    // Note: A tertiary CE has uppercase case bits (0.0.ut)
                    // to keep tertiary+caseFirst well-formed.
                    //
                    // Tertiary+caseLevel: Also ignore case level weights of secondary ignorables.
                    // Otherwise a tertiary CE's uppercase would be no greater than
                    // a primary/secondary CE's uppercase.
                    // (See UCA well-formedness condition 2.)
                    // We could construct a special case weight higher than uppercase,
                    // but it's simpler to always ignore case weights of secondary ignorables,
                    // turning 0.0.ut into 0.0.0.t.
                    // (See LDML Collation, Case Parameters.)
                    loop {
                        left_case = left.base().get_ce(left_index) as i32;
                        left_index += 1;
                        if (left_case as u32 & 0xffff0000) != 0 {
                            break;
                        }
                    }
                    left_lower32 = left_case;
                    left_case &= 0xc000;

                    loop {
                        right_case = right.base().get_ce(right_index) as i32;
                        right_index += 1;
                        if (right_case as u32 & 0xffff0000) != 0 {
                            break;
                        }
                    }
                    right_case &= 0xc000;
                }

                // No need to handle NO_CE and MERGE_SEPARATOR specially:
                // There is one case weight for each previous-level weight,
                // so level length differences were handled there.
                if left_case != right_case {
                    if (options & CollationSettings::UPPER_FIRST) == 0 {
                        return if left_case < right_case { Collation::LESS } else { Collation::GREATER };
                    } else {
                        return if left_case < right_case { Collation::GREATER } else { Collation::LESS };
                    }
                }
                if ushr16(left_lower32) == Collation::NO_CE_WEIGHT16 {
                    break;
                }
            }
        }
        if CollationSettings::get_strength_of(options) <= collator::SECONDARY {
            return Collation::EQUAL;
        }

        let tertiary_mask = CollationSettings::get_tertiary_mask(options);

        let mut left_index = 0;
        let mut right_index = 0;
        let mut any_quaternaries = 0;
        loop {
            let mut left_lower32: i32;
            let mut left_tertiary: i32;
            loop {
                left_lower32 = left.base().get_ce(left_index) as i32;
                left_index += 1;
                any_quaternaries |= left_lower32;
                debug_assert!((left_lower32 & Collation::ONLY_TERTIARY_MASK) != 0 || (left_lower32 & 0xc0c0) == 0);
                left_tertiary = left_lower32 & tertiary_mask;
                if left_tertiary != 0 {
                    break;
                }
            }

            let mut right_lower32: i32;
            let mut right_tertiary: i32;
            loop {
                right_lower32 = right.base().get_ce(right_index) as i32;
                right_index += 1;
                any_quaternaries |= right_lower32;
                debug_assert!((right_lower32 & Collation::ONLY_TERTIARY_MASK) != 0 || (right_lower32 & 0xc0c0) == 0);
                right_tertiary = right_lower32 & tertiary_mask;
                if right_tertiary != 0 {
                    break;
                }
            }

            if left_tertiary != right_tertiary {
                if CollationSettings::sorts_tertiary_upper_case_first(options) {
                    // Pass through NO_CE and keep real tertiary weights larger than that.
                    // Do not change the artificial uppercase weight of a tertiary CE (0.0.ut),
                    // to keep tertiary CEs well-formed.
                    // Their case+tertiary weights must be greater than those of
                    // primary and secondary CEs.
                    if left_tertiary > Collation::NO_CE_WEIGHT16 {
                        if (left_lower32 as u32 & 0xffff0000) != 0 {
                            left_tertiary ^= 0xc000;
                        } else {
                            left_tertiary += 0x4000;
                        }
                    }
                    if right_tertiary > Collation::NO_CE_WEIGHT16 {
                        if (right_lower32 as u32 & 0xffff0000) != 0 {
                            right_tertiary ^= 0xc000;
                        } else {
                            right_tertiary += 0x4000;
                        }
                    }
                }
                return if left_tertiary < right_tertiary { Collation::LESS } else { Collation::GREATER };
            }
            if left_tertiary == Collation::NO_CE_WEIGHT16 {
                break;
            }
        }
        if CollationSettings::get_strength_of(options) <= collator::TERTIARY {
            return Collation::EQUAL;
        }

        if !any_variable && (any_quaternaries & 0xc0) == 0 {
            // If there are no "variable" CEs and no non-zero quaternary weights,
            // then there are no quaternary differences.
            return Collation::EQUAL;
        }

        left_index = 0;
        right_index = 0;
        loop {
            let mut left_quaternary: i64;
            loop {
                let ce = left.base().get_ce(left_index);
                left_index += 1;
                left_quaternary = ce & 0xffff;
                if left_quaternary <= Collation::NO_CE_WEIGHT16 as i64 {
                    // Variable primary or completely ignorable or NO_CE.
                    left_quaternary = ushr32(ce);
                } else {
                    // Regular CE, not tertiary ignorable.
                    // Preserve the quaternary weight in bits 7..6.
                    left_quaternary |= 0xffffff3f;
                }
                if left_quaternary != 0 {
                    break;
                }
            }

            let mut right_quaternary: i64;
            loop {
                let ce = right.base().get_ce(right_index);
                right_index += 1;
                right_quaternary = ce & 0xffff;
                if right_quaternary <= Collation::NO_CE_WEIGHT16 as i64 {
                    // Variable primary or completely ignorable or NO_CE.
                    right_quaternary = ushr32(ce);
                } else {
                    // Regular CE, not tertiary ignorable.
                    // Preserve the quaternary weight in bits 7..6.
                    right_quaternary |= 0xffffff3f;
                }
                if right_quaternary != 0 {
                    break;
                }
            }

            if left_quaternary != right_quaternary {
                // Return the difference, with script reordering.
                if settings.has_reordering() {
                    left_quaternary = settings.reorder(left_quaternary);
                    right_quaternary = settings.reorder(right_quaternary);
                }
                return if left_quaternary < right_quaternary { Collation::LESS } else { Collation::GREATER };
            }
            if left_quaternary == Collation::NO_CE_PRIMARY {
                break;
            }
        }
        Collation::EQUAL
    }
}
