// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.LaoBreakEngine`.

use std::any::Any;

use rucene::error::Result;
use rucene::util::java_collections::java_identity_hash_code_stand_in;
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::lang::u_script::UScript;
use crate::r#impl::breakiter::dictionary_break_engine::{uset_contains, 
    dictionary_find_breaks, DequeI, DictionaryBreakEngine, DictionaryBreakEngineBase, PossibleWord,
};
use crate::r#impl::breakiter::dictionary_data::DictionaryData;
use crate::r#impl::breakiter::dictionary_matcher::DictionaryMatcher;
use crate::r#impl::breakiter::language_break_engine::LanguageBreakEngine;
use crate::r#impl::character_iteration::set_index;
use crate::text::unicode_set::UnicodeSet;

// Constants for LaoBreakIterator
/// `LAO_LOOKAHEAD`: how many words in a row are "good enough"?
const LAO_LOOKAHEAD: i32 = 3;
/// `LAO_ROOT_COMBINE_THRESHOLD`: will not combine a non-word with a
/// preceding dictionary word longer than this.
const LAO_ROOT_COMBINE_THRESHOLD: i32 = 3;
/// `LAO_PREFIX_COMBINE_THRESHOLD`: will not combine a non-word that shares
/// at least this much prefix with a dictionary word with a preceding word.
const LAO_PREFIX_COMBINE_THRESHOLD: i32 = 3;
/// `LAO_MIN_WORD`: minimum word size.
const LAO_MIN_WORD: i32 = 2;

/// `words[n % LAO_LOOKAHEAD]`'s index.
fn ix(n: i32) -> usize {
    (n % LAO_LOOKAHEAD) as usize
}

/// The dictionary-based engine for Lao.
///
/// Equivalent to the class `com.ibm.icu.impl.breakiter.LaoBreakEngine`.
#[derive(Debug)]
pub struct LaoBreakEngine {
    base: DictionaryBreakEngineBase,
    f_dictionary: Option<Box<dyn DictionaryMatcher>>,
    f_end_word_set: UnicodeSet,
    f_begin_word_set: UnicodeSet,
    f_mark_set: UnicodeSet,
}

impl LaoBreakEngine {
    /// `LaoBreakEngine()`: builds the Lao sets and loads the `Laoo`
    /// dictionary.
    ///
    /// # Errors
    ///
    /// The `IOException`/`MissingResourceException` of the dictionary load,
    /// and the exceptions of the `UnicodeSet` patterns.
    pub fn new() -> Result<Self> {
        // Initialize UnicodeSets
        let mut lao_word_set = UnicodeSet::from_pattern(&units("[[:Laoo:]&[:LineBreak=SA:]]"))?;
        let mut f_mark_set = UnicodeSet::from_pattern(&units("[[:Laoo:]&[:LineBreak=SA:]&[:M:]]"))?;
        f_mark_set.add(0x0020)?;
        let mut f_begin_word_set = UnicodeSet::from_pairs(&[
            0x0E81, 0x0EAE, // basic consonants (including holes for corresponding Thai characters)
            0x0EC0, 0x0EC4, // prefix vowels
            0x0EDC, 0x0EDD, // digraph consonants (no Thai equivalent)
        ])?;

        // Compact for caching
        lao_word_set.compact()?;

        let mut f_end_word_set = UnicodeSet::from_set(&lao_word_set);
        f_end_word_set.remove_range(0x0EC0, 0x0EC4)?; // prefix vowels

        f_mark_set.compact()?;
        f_end_word_set.compact()?;
        f_begin_word_set.compact()?;

        // Freeze the static UnicodeSet
        lao_word_set.freeze();
        f_mark_set.freeze();
        f_end_word_set.freeze();
        f_begin_word_set.freeze();

        let mut base = DictionaryBreakEngineBase::new();
        base.set_characters(&lao_word_set)?;
        // Initialize dictionary
        let f_dictionary = DictionaryData::load_dictionary_for("Laoo")?;
        Ok(Self {
            base,
            f_dictionary,
            f_end_word_set,
            f_begin_word_set,
            f_mark_set,
        })
    }

    /// `equals(Object obj)`: true for any `LaoBreakEngine`. Normally is a
    /// singleton, but it's possible to have duplicates during
    /// initialization. All are equivalent.
    pub fn java_equals(&self, obj: &dyn Any) -> bool {
        obj.is::<LaoBreakEngine>()
    }

    /// `hashCode()`: `getClass().hashCode()`.
    ///
    /// **Divergence.** Java answers the identity hash of the `Class` object
    /// (`LaoBreakEngine.java`, `hashCode`), which no Rust value has; the port
    /// answers the crate's stable stand-in for it.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.impl.breakiter.LaoBreakEngine")
    }

    /// `fDictionary`, dereferenced as Java does on each use.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the dictionary data had an unknown trie
    /// type (`DictionaryData.loadDictionaryFor` answered `null`).
    fn dictionary(&self) -> &dyn DictionaryMatcher {
        self.f_dictionary
            .as_deref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fDictionary"))
    }
}

impl LanguageBreakEngine for LaoBreakEngine {
    fn handles(&self, c: i32) -> bool {
        let script = UCharacter::get_int_property_value(c, UProperty::SCRIPT);
        script == UScript::LAO
    }

    fn find_breaks(
        &self,
        text: &mut dyn CharacterIterator,
        start_pos: i32,
        end_pos: i32,
        found_breaks: &mut DequeI,
        is_phrase_breaking: bool,
    ) -> i32 {
        dictionary_find_breaks(self, text, start_pos, end_pos, found_breaks, is_phrase_breaking)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl DictionaryBreakEngine for LaoBreakEngine {
    fn base(&self) -> &DictionaryBreakEngineBase {
        &self.base
    }

    fn divide_up_dictionary_range(
        &self,
        f_iter: &mut dyn CharacterIterator,
        range_start: i32,
        range_end: i32,
        found_breaks: &mut DequeI,
        _is_phrase_breaking: bool,
    ) -> i32 {
        if (range_end - range_start) < LAO_MIN_WORD {
            return 0; // Not enough characters for word
        }
        let dict = self.dictionary();
        let mut words_found = 0;
        let mut word_length;
        let mut current;
        let mut words: [PossibleWord; LAO_LOOKAHEAD as usize] =
            std::array::from_fn(|_| PossibleWord::new());
        let mut uc;

        set_index(f_iter, range_start);

        loop {
            current = f_iter.get_index();
            if current >= range_end {
                break;
            }
            word_length = 0;

            //Look for candidate words at the current position
            let candidates = words[ix(words_found)].candidates(f_iter, dict, range_end);

            // If we found exactly one, use that
            if candidates == 1 {
                word_length = words[ix(words_found)].accept_marked(f_iter);
                words_found += 1;
            }

            // If there was more than one, see which one can take us forward the most words
            else if candidates > 1 {
                let mut found_best = false;
                // If we're already at the end of the range, we're done
                if f_iter.get_index() < range_end {
                    loop {
                        if words[ix(words_found + 1)].candidates(f_iter, dict, range_end) > 0 {
                            // Followed by another dictionary word; mark first word as a good candidate
                            words[ix(words_found)].mark_current();

                            // If we're already at the end of the range, we're done
                            if f_iter.get_index() >= range_end {
                                break;
                            }

                            // See if any of the possible second words is followed by a third word
                            loop {
                                // If we find a third word, stop right away
                                if words[ix(words_found + 2)].candidates(f_iter, dict, range_end) > 0 {
                                    words[ix(words_found)].mark_current();
                                    found_best = true;
                                    break;
                                }
                                if !words[ix(words_found + 1)].back_up(f_iter) {
                                    break;
                                }
                            }
                        }
                        // Java's `while (backUp(fIter) && !foundBest)` calls backUp first.
                        if !(words[ix(words_found)].back_up(f_iter) && !found_best) {
                            break;
                        }
                    }
                }
                word_length = words[ix(words_found)].accept_marked(f_iter);
                words_found += 1;
            }

            // We come here after having either found a word or not. We look ahead to the
            // next word. If it's not a dictionary word, we will combine it with the word we
            // just found (if there is one), but only if the preceding word does not exceed
            // the threshold.
            // The text iterator should now be positioned at the end of the word we found.
            if f_iter.get_index() < range_end && word_length < LAO_ROOT_COMBINE_THRESHOLD {
                // If it is a dictionary word, do nothing. If it isn't, then if there is
                // no preceding word, or the non-word shares less than the minimum threshold
                // of characters with a dictionary word, then scan to resynchronize
                if words[ix(words_found)].candidates(f_iter, dict, range_end) <= 0
                    && (word_length == 0
                        || words[ix(words_found)].longest_prefix() < LAO_PREFIX_COMBINE_THRESHOLD)
                {
                    // Look for a plausible word boundary
                    let mut remaining = range_end - (current + word_length);
                    let mut pc = f_iter.current() as i32;
                    let mut chars = 0;
                    loop {
                        f_iter.next();
                        uc = f_iter.current() as i32;
                        chars += 1;
                        remaining -= 1;
                        if remaining <= 0 {
                            break;
                        }
                        if uset_contains(&self.f_end_word_set, pc) && uset_contains(&self.f_begin_word_set, uc) {
                            // Maybe. See if it's in the dictionary.
                            let candidate = words[ix(words_found + 1)].candidates(f_iter, dict, range_end);
                            set_index(f_iter, current + word_length + chars);
                            if candidate > 0 {
                                break;
                            }
                        }
                        pc = uc;
                    }

                    // Bump the word count if there wasn't already one
                    if word_length <= 0 {
                        words_found += 1;
                    }

                    // Update the length with the passed-over characters
                    word_length += chars;
                } else {
                    // Backup to where we were for next iteration
                    set_index(f_iter, current + word_length);
                }
            }

            // Never stop before a combining mark.
            loop {
                let curr_pos = f_iter.get_index();
                if !(curr_pos < range_end && uset_contains(&self.f_mark_set, f_iter.current() as i32)) {
                    break;
                }
                f_iter.next();
                word_length += f_iter.get_index() - curr_pos;
            }

            // Look ahead for possible suffixes if a dictionary word does not follow.
            // We do this in code rather than using a rule so that the heuristic
            // resynch continues to function. For example, one of the suffix characters
            // could be a typo in the middle of a word.
            // NOT CURRENTLY APPLICABLE TO LAO

            // Did we find a word on this iteration? If so, push it on the break stack
            if word_length > 0 {
                found_breaks.push(current + word_length);
            }
        }

        // Don't return a break for the end of the dictionary range if there is one there
        if found_breaks.peek() >= range_end {
            found_breaks.pop();
            words_found -= 1;
        }

        words_found
    }
}
