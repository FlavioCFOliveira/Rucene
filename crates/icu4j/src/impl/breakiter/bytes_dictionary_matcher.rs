// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.BytesDictionaryMatcher`.

use std::sync::Arc;

use rucene::error::Result;
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::r#impl::assert::Assert;
use crate::r#impl::breakiter::dictionary_data::DictionaryData;
use crate::r#impl::breakiter::dictionary_matcher::{wrapper_next_code_point, DictionaryMatcher};
use crate::text::u_character_iterator::DONE;
use crate::util::bytes_trie::{BytesTrie, Result as TrieResult};

/// A dictionary over a `BytesTrie` whose bytes are code points shifted by a
/// script offset.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.impl.breakiter.BytesDictionaryMatcher`.
#[derive(Debug)]
pub(crate) struct BytesDictionaryMatcher {
    characters: Arc<[i8]>,
    transform: i32,
}

impl BytesDictionaryMatcher {
    /// `BytesDictionaryMatcher(byte[] chars, int transform)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) unless `transform` is an
    /// offset transform.
    pub(crate) fn new(chars: Vec<i8>, transform: i32) -> Result<Self> {
        Assert::assrt(
            (transform & DictionaryData::TRANSFORM_TYPE_MASK)
                == DictionaryData::TRANSFORM_TYPE_OFFSET,
        )?;
        // while there is only one transform type so far, save the entire transform constant so that
        // if we add any others, we need only change code in transform() and the assert above rather
        // than adding a "transform type" variable
        Ok(Self {
            characters: Arc::from(chars),
            transform,
        })
    }

    /// `transform(int c)`.
    fn transform(&self, c: i32) -> i32 {
        if c == 0x200D {
            return 0xFF;
        } else if c == 0x200C {
            return 0xFE;
        }

        let delta = c - (self.transform & DictionaryData::TRANSFORM_OFFSET_MASK);
        if !(0..=0xFD).contains(&delta) {
            return -1;
        }
        delta
    }
}

impl DictionaryMatcher for BytesDictionaryMatcher {
    fn matches(
        &self,
        text: &mut dyn CharacterIterator,
        max_length: i32,
        lengths: &mut [i32],
        count_: &mut [i32],
        limit: i32,
        mut values: Option<&mut [i32]>,
    ) -> i32 {
        let mut bt = BytesTrie::new(self.characters.clone(), 0);
        let mut c = wrapper_next_code_point(text);
        if c == DONE {
            return 0;
        }
        let mut result = bt.first(self.transform(c));
        // TODO: should numChars count Character.charCount() ?
        let mut num_chars = 1;
        let mut count = 0;
        loop {
            if result.has_value() {
                if count < limit {
                    if let Some(values) = values.as_deref_mut() {
                        values[count as usize] = bt.get_value();
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
            result = bt.next(self.transform(c));
        }
        count_[0] = count;
        num_chars
    }

    fn get_type(&self) -> i32 {
        DictionaryData::TRIE_TYPE_BYTES
    }
}
