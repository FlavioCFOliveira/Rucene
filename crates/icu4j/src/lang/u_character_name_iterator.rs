// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UCharacterNameIterator`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::lang::u_character::UCharacter;
use crate::r#impl::u_character_name::{UCharacterName, LINES_PER_GROUP};
use crate::r#impl::u_character_name_choice::UCharacterNameChoice;
use crate::util::value_iterator::{Element, ValueIterator};

/// Class enabling iteration of the codepoints and their names.
///
/// Equivalent to `com.ibm.icu.lang.UCharacterNameIterator`
/// (package-private). Java's shared synchronized group buffers are local
/// per call. Each element's value is the name as a `Vec<u16>`.
#[derive(Debug)]
pub struct UCharacterNameIterator {
    m_name: &'static UCharacterName,
    m_choice: i32,
    m_start: i32,
    m_limit: i32,
    m_current: i32,
    m_group_index: i32,
    m_algorithm_index: i32,
}

impl ValueIterator for UCharacterNameIterator {
    #[allow(clippy::if_same_then_else)] // branches mirror the Java source
    fn next(&mut self, element: &mut Element) -> bool {
        if self.m_current >= self.m_limit {
            return false;
        }

        if self.m_choice == UCharacterNameChoice::UNICODE_CHAR_NAME
            || self.m_choice == UCharacterNameChoice::EXTENDED_CHAR_NAME
        {
            let length = self.m_name.get_algorithm_length();
            if self.m_algorithm_index < length {
                while self.m_algorithm_index < length {
                    // find the algorithm range that could contain m_current_
                    if self.m_algorithm_index < 0
                        || self.m_name.get_algorithm_end(self.m_algorithm_index) < self.m_current
                    {
                        self.m_algorithm_index += 1;
                    } else {
                        break;
                    }
                }

                if self.m_algorithm_index < length {
                    // interleave the data-driven ones with the algorithmic ones
                    // iterate over all algorithmic ranges; assume that they are
                    // in ascending order
                    let start = self.m_name.get_algorithm_start(self.m_algorithm_index);
                    if self.m_current < start {
                        // this should get rid of those codepoints that are not
                        // in the algorithmic range
                        let mut end = start;
                        if self.m_limit <= start {
                            end = self.m_limit;
                        }
                        if !self.iterate_group(element, end) {
                            self.m_current += 1;
                            return true;
                        }
                    }

                    if self.m_current >= self.m_limit {
                        // after iterateGroup fails, current codepoint may be
                        // greater than limit
                        return false;
                    }

                    element.integer = self.m_current;
                    element.value = Some(Arc::new(
                        self.m_name
                            .get_algorithm_name(self.m_algorithm_index, self.m_current),
                    ));
                    // reset the group index if we are in the algorithmic names
                    self.m_group_index = -1;
                    self.m_current += 1;
                    return true;
                }
            }
        }
        // enumerate the character names after the last algorithmic range
        if !self.iterate_group(element, self.m_limit) {
            self.m_current += 1;
            return true;
        } else if self.m_choice == UCharacterNameChoice::EXTENDED_CHAR_NAME
            && !self.iterate_extended(element, self.m_limit)
        {
            self.m_current += 1;
            return true;
        }

        false
    }

    fn reset(&mut self) {
        self.m_current = self.m_start;
        self.m_group_index = -1;
        self.m_algorithm_index = -1;
    }

    fn set_range(&mut self, start: i32, limit: i32) -> Result<()> {
        if start >= limit {
            return Err(LuceneError::illegal_argument(
                "start or limit has to be valid Unicode codepoints and start < limit",
            ));
        }
        if start < UCharacter::MIN_VALUE {
            self.m_start = UCharacter::MIN_VALUE;
        } else {
            self.m_start = start;
        }

        if limit > UCharacter::MAX_VALUE + 1 {
            self.m_limit = UCharacter::MAX_VALUE + 1;
        } else {
            self.m_limit = limit;
        }
        self.m_current = self.m_start;
        Ok(())
    }
}

impl UCharacterNameIterator {
    /// `UCharacterNameIterator(UCharacterName name, int choice)`
    /// (protected). Java's check for a null name cannot fail.
    pub(crate) fn new(name: &'static UCharacterName, choice: i32) -> Self {
        Self {
            m_name: name,
            m_choice: choice,
            m_start: UCharacter::MIN_VALUE,
            m_limit: UCharacter::MAX_VALUE + 1,
            m_current: UCharacter::MIN_VALUE,
            m_group_index: -1,
            m_algorithm_index: -1,
        }
    }

    /// Group use: false when a name was found (and stored in `result`).
    fn iterate_single_group(&mut self, result: &mut Element, limit: i32) -> bool {
        let mut group_offsets = [0u16; LINES_PER_GROUP as usize + 1];
        let mut group_lengths = [0u16; LINES_PER_GROUP as usize + 1];
        let index = self.m_name.get_group_lengths(
            self.m_group_index,
            &mut group_offsets,
            &mut group_lengths,
        );
        while self.m_current < limit {
            let offset = UCharacterName::get_group_offset(self.m_current) as usize;
            let mut name = self.m_name.get_group_name(
                index + group_offsets[offset] as i32,
                group_lengths[offset] as i32,
                self.m_choice,
            );
            if name.as_ref().is_none_or(|n| n.is_empty())
                && self.m_choice == UCharacterNameChoice::EXTENDED_CHAR_NAME
            {
                name = self.m_name.get_extended_name(self.m_current);
            }
            if let Some(n) = name {
                if !n.is_empty() {
                    result.integer = self.m_current;
                    result.value = Some(Arc::new(n));
                    return false;
                }
            }
            self.m_current += 1;
        }
        true
    }

    /// Group use: false when a name was found (and stored in `result`).
    fn iterate_group(&mut self, result: &mut Element, limit: i32) -> bool {
        if self.m_group_index < 0 {
            self.m_group_index = self.m_name.get_group(self.m_current);
        }

        while self.m_group_index < self.m_name.m_groupcount && self.m_current < limit {
            // iterate till the last group or the last codepoint
            let start_msb = UCharacterName::get_codepoint_msb(self.m_current);
            let g_msb = self.m_name.get_group_msb(self.m_group_index); // can be -1
            if start_msb == g_msb {
                if start_msb == UCharacterName::get_codepoint_msb(limit - 1) {
                    // if start and limit-1 are in the same group, then enumerate
                    // only in that one
                    return self.iterate_single_group(result, limit);
                }
                // enumerate characters in the partial start group
                // if (m_name_.getGroupOffset(m_current_) != 0) {
                if !self.iterate_single_group(result, UCharacterName::get_group_limit(g_msb)) {
                    return false;
                }
                self.m_group_index += 1; // continue with the next group
            } else if start_msb > g_msb {
                // make sure that we start enumerating with the first group
                // after start
                self.m_group_index += 1;
            } else {
                let mut g_min = UCharacterName::get_group_min(g_msb);
                if g_min > limit {
                    g_min = limit;
                }
                if self.m_choice == UCharacterNameChoice::EXTENDED_CHAR_NAME
                    && !self.iterate_extended(result, g_min)
                {
                    return false;
                }
                self.m_current = g_min;
            }
        }

        true
    }

    /// Iterate extended names: false when a name was found.
    fn iterate_extended(&mut self, result: &mut Element, limit: i32) -> bool {
        while self.m_current < limit {
            let name = self.m_name.get_extended_or10_name(self.m_current);
            if !name.is_empty() {
                result.integer = self.m_current;
                result.value = Some(Arc::new(name));
                return false;
            }
            self.m_current += 1;
        }
        true
    }
}
