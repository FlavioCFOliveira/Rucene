// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.CharsDictionaryMatcher`.

use std::sync::Arc;

use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::r#impl::breakiter::dictionary_data::DictionaryData;
use crate::r#impl::breakiter::dictionary_matcher::{wrapper_next_code_point, DictionaryMatcher};
use crate::text::u_character_iterator::DONE;
use crate::util::bytes_trie::Result as TrieResult;
use crate::util::chars_trie::CharsTrie;

/// A dictionary over a `CharsTrie` of UTF-16 words.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.impl.breakiter.CharsDictionaryMatcher`.
#[derive(Debug)]
pub(crate) struct CharsDictionaryMatcher {
    characters: Arc<[u16]>,
}

impl CharsDictionaryMatcher {
    /// `CharsDictionaryMatcher(CharSequence chars)`.
    pub(crate) fn new(chars: Vec<u16>) -> Self {
        Self {
            characters: Arc::from(chars),
        }
    }
}

impl DictionaryMatcher for CharsDictionaryMatcher {
    fn matches(
        &self,
        text: &mut dyn CharacterIterator,
        max_length: i32,
        lengths: &mut [i32],
        count_: &mut [i32],
        limit: i32,
        mut values: Option<&mut [i32]>,
    ) -> i32 {
        let mut uct = CharsTrie::new(self.characters.clone(), 0);
        let mut c = wrapper_next_code_point(text);
        if c == DONE {
            return 0;
        }
        let mut result = uct.first_for_code_point(c);
        // TODO: should numChars count Character.charCount?
        let mut num_chars = 1;
        let mut count = 0;
        loop {
            if result.has_value() {
                if count < limit {
                    if let Some(values) = values.as_deref_mut() {
                        values[count as usize] = uct.get_value();
                    }
                    lengths[count as usize] = num_chars;
                    count += 1;
                }

                if result == TrieResult::FinalValue {
                    break;
                }
            } else if result == TrieResult::NoMatch {
                break;
            }

            if num_chars >= max_length {
                break;
            }
            c = wrapper_next_code_point(text);
            if c == DONE {
                break;
            }
            num_chars += 1;
            result = uct.next_for_code_point(c);
        }
        count_[0] = count;
        num_chars
    }

    fn get_type(&self) -> i32 {
        DictionaryData::TRIE_TYPE_UCHARS
    }
}
