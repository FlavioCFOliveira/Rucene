// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.ThaiBreakEngine`.

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

// Constants for ThaiBreakIterator
/// `THAI_LOOKAHEAD`: how many words in a row are "good enough"?
const THAI_LOOKAHEAD: i32 = 3;
/// `THAI_ROOT_COMBINE_THRESHOLD`: will not combine a non-word with a
/// preceding dictionary word longer than this.
const THAI_ROOT_COMBINE_THRESHOLD: i32 = 3;
/// `THAI_PREFIX_COMBINE_THRESHOLD`: will not combine a non-word that shares
/// at least this much prefix with a dictionary word with a preceding word.
const THAI_PREFIX_COMBINE_THRESHOLD: i32 = 3;
/// `THAI_PAIYANNOI`: ellision character.
const THAI_PAIYANNOI: i32 = 0x0E2F;
/// `THAI_MAIYAMOK`: repeat character.
const THAI_MAIYAMOK: i32 = 0x0E46;
/// `THAI_MIN_WORD`: minimum word size.
const THAI_MIN_WORD: i32 = 2;
/// `THAI_MIN_WORD_SPAN`: minimum number of characters for two words.
const THAI_MIN_WORD_SPAN: i32 = THAI_MIN_WORD * 2;

/// `words[n % THAI_LOOKAHEAD]`'s index.
fn ix(n: i32) -> usize {
    (n % THAI_LOOKAHEAD) as usize
}

/// The dictionary-based engine for Thai.
///
/// Equivalent to the class `com.ibm.icu.impl.breakiter.ThaiBreakEngine`.
#[derive(Debug)]
pub struct ThaiBreakEngine {
    base: DictionaryBreakEngineBase,
    f_dictionary: Option<Box<dyn DictionaryMatcher>>,
    f_end_word_set: UnicodeSet,
    f_begin_word_set: UnicodeSet,
    f_suffix_set: UnicodeSet,
    f_mark_set: UnicodeSet,
}

impl ThaiBreakEngine {
    /// `ThaiBreakEngine()`: builds the Thai sets and loads the `Thai`
    /// dictionary.
    ///
    /// # Errors
    ///
    /// The `IOException`/`MissingResourceException` of the dictionary load,
    /// and the exceptions of the `UnicodeSet` patterns.
    pub fn new() -> Result<Self> {
        // Initialize UnicodeSets
        let mut thai_word_set = UnicodeSet::from_pattern(&units("[[:Thai:]&[:LineBreak=SA:]]"))?;
        let mut f_mark_set = UnicodeSet::from_pattern(&units("[[:Thai:]&[:LineBreak=SA:]&[:M:]]"))?;
        f_mark_set.add(0x0020)?;
        let mut f_begin_word_set = UnicodeSet::from_pairs(&[0x0E01, 0x0E2E, 0x0E40, 0x0E44])?;
        let mut f_suffix_set = UnicodeSet::new();
        f_suffix_set.add(THAI_PAIYANNOI)?;
        f_suffix_set.add(THAI_MAIYAMOK)?;

        // Compact for caching
        thai_word_set.compact()?;

        let mut f_end_word_set = UnicodeSet::from_set(&thai_word_set);
        f_end_word_set.remove(0x0E31)?; // MAI HAN-AKAT
        f_end_word_set.remove_range(0x0E40, 0x0E44)?; // SARA E through SARA AI MAIMALAI

        f_mark_set.compact()?;
        f_end_word_set.compact()?;
        f_begin_word_set.compact()?;
        f_suffix_set.compact()?;

        // Freeze the static UnicodeSet
        thai_word_set.freeze();
        f_mark_set.freeze();
        f_end_word_set.freeze();
        f_begin_word_set.freeze();
        f_suffix_set.freeze();

        let mut base = DictionaryBreakEngineBase::new();
        base.set_characters(&thai_word_set)?;
        // Initialize dictionary
        let f_dictionary = DictionaryData::load_dictionary_for("Thai")?;
        Ok(Self {
            base,
            f_dictionary,
            f_end_word_set,
            f_begin_word_set,
            f_suffix_set,
            f_mark_set,
        })
    }

    /// `equals(Object obj)`: true for any `ThaiBreakEngine`. Normally is a
    /// singleton, but it's possible to have duplicates during
    /// initialization. All are equivalent.
    pub fn java_equals(&self, obj: &dyn Any) -> bool {
        obj.is::<ThaiBreakEngine>()
    }

    /// `hashCode()`: `getClass().hashCode()`.
    ///
    /// **Divergence.** Java answers the identity hash of the `Class` object
    /// (`ThaiBreakEngine.java:96`), which no Rust value has; the port answers
    /// the crate's stable stand-in for it.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.impl.breakiter.ThaiBreakEngine")
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

impl LanguageBreakEngine for ThaiBreakEngine {
    fn handles(&self, c: i32) -> bool {
        let script = UCharacter::get_int_property_value(c, UProperty::SCRIPT);
        script == UScript::THAI
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

impl DictionaryBreakEngine for ThaiBreakEngine {
    fn base(&self) -> &DictionaryBreakEngineBase {
        &self.base
    }

    #[allow(unused_assignments)]
    fn divide_up_dictionary_range(
        &self,
        f_iter: &mut dyn CharacterIterator,
        range_start: i32,
        range_end: i32,
        found_breaks: &mut DequeI,
        _is_phrase_breaking: bool,
    ) -> i32 {
        if (range_end - range_start) < THAI_MIN_WORD_SPAN {
            return 0; // Not enough characters for word
        }
        let dict = self.dictionary();
        let mut words_found = 0;
        let mut word_length;
        let mut words: [PossibleWord; THAI_LOOKAHEAD as usize] =
            std::array::from_fn(|_| PossibleWord::new());

        let mut uc: i32 = 0;
        set_index(f_iter, range_start);
        let mut current;
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
                // If we're already at the end of the range, we're done
                if f_iter.get_index() < range_end {
                    'found_best: loop {
                        if words[ix(words_found + 1)].candidates(f_iter, dict, range_end) > 0 {
                            // Followed by another dictionary word; mark first word as a good candidate
                            words[ix(words_found)].mark_current();

                            // If we're already at the end of the range, we're done
                            if f_iter.get_index() >= range_end {
                                break 'found_best;
                            }

                            // See if any of the possible second words is followed by a third word
                            loop {
                                // If we find a third word, stop right away
                                if words[ix(words_found + 2)].candidates(f_iter, dict, range_end) > 0 {
                                    words[ix(words_found)].mark_current();
                                    break 'found_best;
                                }
                                if !words[ix(words_found + 1)].back_up(f_iter) {
                                    break;
                                }
                            }
                        }
                        if !words[ix(words_found)].back_up(f_iter) {
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
            if f_iter.get_index() < range_end && word_length < THAI_ROOT_COMBINE_THRESHOLD {
                // If it is a dictionary word, do nothing. If it isn't, then if there is
                // no preceding word, or the non-word shares less than the minimum threshold
                // of characters with a dictionary word, then scan to resynchronize
                if words[ix(words_found)].candidates(f_iter, dict, range_end) <= 0
                    && (word_length == 0
                        || words[ix(words_found)].longest_prefix() < THAI_PREFIX_COMBINE_THRESHOLD)
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
                            // NOTE: In the original Apple code, checked that the next
                            // two characters after uc were not 0x0E4C THANTHAKHAT before
                            // checking the dictionary. That is just a performance filter,
                            // but it's not clear it's faster than checking the trie
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
            if f_iter.get_index() < range_end && word_length > 0 {
                if words[ix(words_found)].candidates(f_iter, dict, range_end) <= 0 && {
                    uc = f_iter.current() as i32;
                    uset_contains(&self.f_suffix_set, uc)
                } {
                    if uc == THAI_PAIYANNOI {
                        if !uset_contains(&self.f_suffix_set, f_iter.previous() as i32) {
                            // Skip over previous end and PAIYANNOI
                            f_iter.next();
                            f_iter.next();
                            word_length += 1;
                            uc = f_iter.current() as i32;
                        } else {
                            // Restore prior position
                            f_iter.next();
                        }
                    }
                    if uc == THAI_MAIYAMOK {
                        if f_iter.previous() as i32 != THAI_MAIYAMOK {
                            // Skip over previous end and MAIYAMOK
                            f_iter.next();
                            f_iter.next();
                            word_length += 1;
                        } else {
                            // restore prior position
                            f_iter.next();
                        }
                    }
                } else {
                    set_index(f_iter, current + word_length);
                }
            }

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
