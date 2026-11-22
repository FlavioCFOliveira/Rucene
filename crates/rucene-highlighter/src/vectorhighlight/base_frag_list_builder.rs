/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `BaseFragListBuilder.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use rucene::error::{LuceneError, Result};

use crate::vectorhighlight::field_frag_list::FieldFragList;
use crate::vectorhighlight::field_phrase_list::{FieldPhraseList, WeightedPhraseInfo};

/// A abstract implementation of `FragListBuilder`.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.vectorhighlight.BaseFragListBuilder`: its state
/// and the protected `createFieldFragList(FieldPhraseList, FieldFragList,
/// int)`, which its subclasses reach through `Deref`.
#[derive(Debug, Clone)]
pub struct BaseFragListBuilder {
    pub(crate) margin: i32,
    pub(crate) min_frag_char_size: i32,
}

impl BaseFragListBuilder {
    /// Equivalent to `BaseFragListBuilder.MARGIN_DEFAULT`.
    pub const MARGIN_DEFAULT: i32 = 6;
    /// Equivalent to `BaseFragListBuilder.MIN_FRAG_CHAR_SIZE_FACTOR`.
    pub const MIN_FRAG_CHAR_SIZE_FACTOR: i32 = 3;

    /// Equivalent to `new BaseFragListBuilder(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `margin` is negative.
    pub fn with_margin(margin: i32) -> Result<Self> {
        if margin < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "margin({margin}) is too small. It must be 0 or higher."
            )));
        }

        Ok(Self {
            margin,
            min_frag_char_size: 1.max(margin.wrapping_mul(Self::MIN_FRAG_CHAR_SIZE_FACTOR)),
        })
    }

    /// Equivalent to `new BaseFragListBuilder()`.
    pub fn new() -> Self {
        Self::with_margin(Self::MARGIN_DEFAULT).expect("INVARIANT: the default margin is valid")
    }

    /// Equivalent to the protected `createFieldFragList(FieldPhraseList,
    /// FieldFragList, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `frag_char_size` is below the minimum.
    pub fn create_field_frag_list_into(
        &self,
        field_phrase_list: &FieldPhraseList,
        mut field_frag_list: Box<dyn FieldFragList>,
        frag_char_size: i32,
    ) -> Result<Box<dyn FieldFragList>> {
        if frag_char_size < self.min_frag_char_size {
            return Err(LuceneError::illegal_argument(format!(
                "fragCharSize({frag_char_size}) is too small. It must be {} or higher.",
                self.min_frag_char_size
            )));
        }

        let mut wpil: Vec<WeightedPhraseInfo> = Vec::new();
        let mut queue = IteratorQueue::new(field_phrase_list.get_phrase_list().iter());
        let mut start_offset = 0;
        while let Some(phrase_info) = queue.top() {
            // if the phrase violates the border of previous fragment, discard
            // it and try next phrase
            if phrase_info.get_start_offset() < start_offset {
                queue.remove_top();
                continue;
            }

            wpil.clear();
            let current_phrase_start_offset = phrase_info.get_start_offset();
            let mut current_phrase_end_offset = phrase_info.get_end_offset();
            let mut span_start = (current_phrase_start_offset - self.margin).max(start_offset);
            let mut span_end = current_phrase_end_offset.max(span_start + frag_char_size);
            let removed = queue.remove_top().expect("INVARIANT: the top is set");
            if self.accept_phrase(
                removed,
                current_phrase_end_offset - current_phrase_start_offset,
                frag_char_size,
            ) {
                wpil.push(removed.clone());
            }
            while let Some(phrase_info) = queue.top() {
                // pull until we crossed the current spanEnd
                if phrase_info.get_end_offset() <= span_end {
                    current_phrase_end_offset = phrase_info.get_end_offset();
                    let removed = queue.remove_top().expect("INVARIANT: the top is set");
                    if self.accept_phrase(
                        removed,
                        current_phrase_end_offset - current_phrase_start_offset,
                        frag_char_size,
                    ) {
                        wpil.push(removed.clone());
                    }
                } else {
                    break;
                }
            }
            if wpil.is_empty() {
                continue;
            }

            let match_len = current_phrase_end_offset - current_phrase_start_offset;
            // now recalculate the start and end position to "center" the
            // result
            let new_margin = 0.max((frag_char_size - match_len) / 2); // matchLen can be > fragCharSize prevent IAOOB here
            span_start = current_phrase_start_offset - new_margin;
            if span_start < start_offset {
                span_start = start_offset;
            }
            // whatever is bigger matchLen or fragCharSize - we use it as
            // the end of span
            span_end = span_start + match_len.max(frag_char_size);
            start_offset = span_end;
            field_frag_list.add(span_start, span_end, &wpil);
        }
        Ok(field_frag_list)
    }

    /// A predicate to decide if the given `WeightedPhraseInfo` should be
    /// accepted as a highlighted phrase or if it should be discarded.
    ///
    /// The default implementation discards phrases that are composed of
    /// more than one term and where the matchLength exceeds the fragment
    /// character size.
    ///
    /// Equivalent to the protected `acceptPhrase(WeightedPhraseInfo, int,
    /// int)`.
    pub fn accept_phrase(&self, info: &WeightedPhraseInfo, match_length: i32, frag_char_size: i32) -> bool {
        info.get_terms_offsets().len() <= 1 || match_length <= frag_char_size
    }
}

impl Default for BaseFragListBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Equivalent to the private nested class `BaseFragListBuilder.IteratorQueue<T>`.
pub(crate) struct IteratorQueue<I: Iterator> {
    iter: I,
    top: Option<I::Item>,
}

impl<I: Iterator> IteratorQueue<I>
where
    I::Item: Copy,
{
    pub(crate) fn new(iter: I) -> Self {
        let mut this = Self { iter, top: None };
        let remove_top = this.remove_top();
        debug_assert!(remove_top.is_none());
        this
    }

    pub(crate) fn top(&self) -> Option<I::Item> {
        self.top
    }

    pub(crate) fn remove_top(&mut self) -> Option<I::Item> {
        let current_top = self.top;
        self.top = self.iter.next();
        current_top
    }
}