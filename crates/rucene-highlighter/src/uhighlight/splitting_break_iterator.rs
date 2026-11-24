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

//! Port of `SplittingBreakIterator.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use rucene::error::Result;
use rucene_analysis_common::jdk::break_iterator::{BreakIterator, DONE};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, StringCharacterIterator};

/// Virtually slices the text on both sides of every occurrence of a
/// specified character. If the slice is 0-length which happens for adjacent
/// slice characters or when they are at the beginning or end, that character
/// is reported as a boundary. For every slice between the specified
/// characters, it is further processed with a specified BreakIterator. A
/// consequence is that the enclosed BreakIterator will never "see" the
/// splitting character.
///
/// Note: `setText(CharacterIterator)` is unsupported. Use the string version.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.SplittingBreakIterator`.
#[derive(Debug)]
pub struct SplittingBreakIterator {
    base_iter: Box<dyn BreakIterator>,
    slice_char: u16,

    text: Vec<u16>,
    slice_start_idx: i32,
    slice_end_idx: i32,
    current: i32,
    /// The iterator `getText()` returned last (Java creates one per call).
    text_iter: StringCharacterIterator,
}

fn index_of(text: &[u16], ch: u16, from: i32) -> i32 {
    let from = from.max(0) as usize;
    if from >= text.len() {
        return -1;
    }
    text[from..]
        .iter()
        .position(|&c| c == ch)
        .map_or(-1, |p| (p + from) as i32)
}

fn last_index_of(text: &[u16], ch: u16, from: i32) -> i32 {
    if from < 0 {
        return -1;
    }
    let end = (from as usize).min(text.len().saturating_sub(1));
    if text.is_empty() {
        return -1;
    }
    text[..=end]
        .iter()
        .rposition(|&c| c == ch)
        .map_or(-1, |p| p as i32)
}

impl SplittingBreakIterator {
    /// Equivalent to `new SplittingBreakIterator(BreakIterator, char)`.
    pub fn new(base_iter: Box<dyn BreakIterator>, slice_char: u16) -> Self {
        Self {
            base_iter,
            slice_char,
            text: Vec::new(),
            slice_start_idx: 0,
            slice_end_idx: 0,
            current: 0,
            text_iter: StringCharacterIterator::new(Vec::new()),
        }
    }

    fn set_base_text(&mut self) {
        let slice = self.text[self.slice_start_idx as usize..self.slice_end_idx as usize].to_vec();
        self.base_iter.set_text_units(&slice);
    }

    fn text_length(&self) -> i32 {
        self.text.len() as i32
    }
}

impl BreakIterator for SplittingBreakIterator {
    /// # Panics
    ///
    /// Always, as Java throws `UnsupportedOperationException("unexpected")`.
    fn set_text(&mut self, _new_text: Box<dyn CharacterIterator>) {
        panic!("UnsupportedOperationException: unexpected");
    }

    fn set_text_units(&mut self, new_text: &[u16]) {
        self.text = new_text.to_vec();
        self.first();
    }

    fn get_text(&mut self) -> &dyn CharacterIterator {
        let mut char_iter = StringCharacterIterator::new(self.text.clone());
        // API doesn't say what the state should be but it should probably be
        // at the current index.
        let _ = char_iter.set_index(self.current());
        self.text_iter = char_iter;
        &self.text_iter
    }

    fn current(&self) -> i32 {
        debug_assert!(self.current != DONE);
        self.current // MUST be updated by the other methods when result isn't DONE.
    }

    fn first(&mut self) -> i32 {
        self.slice_start_idx = 0;
        self.slice_end_idx = index_of(&self.text, self.slice_char, 0);
        if self.slice_end_idx == -1 {
            self.slice_end_idx = self.text_length();
        }
        if self.slice_start_idx == self.slice_end_idx {
            self.current = self.slice_start_idx;
            return self.current;
        }
        self.set_base_text();
        self.current = self.slice_start_idx + self.base_iter.current();
        self.current
    }

    fn last(&mut self) -> i32 {
        self.slice_end_idx = self.text_length();
        self.slice_start_idx = last_index_of(&self.text, self.slice_char, self.text_length() - 1);
        if self.slice_start_idx == -1 {
            self.slice_start_idx = 0;
        } else {
            self.slice_start_idx += 1; // past sliceChar
        }
        if self.slice_end_idx == self.slice_start_idx {
            self.current = self.slice_end_idx;
            return self.current;
        }
        self.set_base_text();
        self.current = self.slice_start_idx + self.base_iter.last();
        self.current
    }

    fn next(&mut self) -> i32 {
        let prev_current = self.current;
        self.current = if self.slice_start_idx == self.slice_end_idx {
            DONE
        } else {
            self.base_iter.next()
        };
        if self.current != DONE {
            self.current += self.slice_start_idx;
            return self.current;
        }
        if self.slice_end_idx >= self.text_length() {
            self.current = prev_current; // keep current where it is
            return DONE;
        }
        self.slice_start_idx = self.slice_end_idx + 1;
        self.slice_end_idx = index_of(&self.text, self.slice_char, self.slice_start_idx);
        if self.slice_end_idx == -1 {
            self.slice_end_idx = self.text_length();
        }
        if self.slice_start_idx == self.slice_end_idx {
            self.current = self.slice_start_idx;
            return self.current;
        }
        self.set_base_text();
        self.current = self.slice_start_idx + self.base_iter.current(); // use current() since at first() already
        self.current
    }

    fn previous(&mut self) -> i32 {
        // note: closely follows next() but reversed
        let prev_current = self.current;
        self.current = if self.slice_start_idx == self.slice_end_idx {
            DONE
        } else {
            self.base_iter.previous()
        };
        if self.current != DONE {
            self.current += self.slice_start_idx;
            return self.current;
        }
        if self.slice_start_idx == 0 {
            self.current = prev_current; // keep current where it is
            return DONE;
        }
        self.slice_end_idx = self.slice_start_idx - 1;
        self.slice_start_idx = last_index_of(&self.text, self.slice_char, self.slice_end_idx - 1);
        if self.slice_start_idx == -1 {
            self.slice_start_idx = 0;
        } else {
            self.slice_start_idx += 1; // past sliceChar
        }
        if self.slice_start_idx == self.slice_end_idx {
            self.current = self.slice_start_idx;
            return self.current;
        }
        self.set_base_text();
        self.current = self.slice_start_idx + self.base_iter.last();
        self.current
    }

    fn following(&mut self, offset: i32) -> Result<i32> {
        // if the offset is not in this slice, update the slice
        if offset + 1 < self.slice_start_idx || offset + 1 > self.slice_end_idx {
            if offset == self.text_length() {
                // DONE condition
                self.last(); // because https://bugs.openjdk.java.net/browse/JDK-8015110
                return Ok(DONE);
            }
            self.slice_start_idx = last_index_of(&self.text, self.slice_char, offset); // no +1
            if self.slice_start_idx == -1 {
                self.slice_start_idx = 0;
            } else {
                self.slice_start_idx += 1; // move past separator
            }
            self.slice_end_idx = index_of(
                &self.text,
                self.slice_char,
                (offset + 1).max(self.slice_start_idx),
            );
            if self.slice_end_idx == -1 {
                self.slice_end_idx = self.text_length();
            }
            if self.slice_start_idx != self.slice_end_idx {
                // otherwise, adjacent separator or separator at end
                self.set_base_text();
            }
        }

        // lastly, call following on baseIter (if applicable)
        if self.slice_start_idx == self.slice_end_idx {
            self.current = offset + 1;
            Ok(self.current)
        } else if offset == self.slice_start_idx - 1 {
            self.current = self.slice_start_idx + self.base_iter.first();
            Ok(self.current)
        } else {
            self.current =
                self.slice_start_idx + self.base_iter.following(offset - self.slice_start_idx)?;
            Ok(self.current)
        }
    }

    fn preceding(&mut self, offset: i32) -> Result<i32> {
        // note: closely follows following() but reversed
        if offset - 1 < self.slice_start_idx || offset - 1 > self.slice_end_idx {
            if offset == 0 {
                // DONE condition
                self.first(); // because https://bugs.openjdk.java.net/browse/JDK-8015110
                return Ok(DONE);
            }
            self.slice_end_idx = index_of(&self.text, self.slice_char, offset); // no -1
            if self.slice_end_idx == -1 {
                self.slice_end_idx = self.text_length();
            }
            self.slice_start_idx = last_index_of(&self.text, self.slice_char, offset - 1);
            if self.slice_start_idx == -1 {
                self.slice_start_idx = 0;
            } else {
                self.slice_start_idx = (self.slice_start_idx + 1).min(self.slice_end_idx);
            }
            if self.slice_start_idx != self.slice_end_idx {
                // otherwise, adjacent separator or separator at end
                self.set_base_text();
            }
        }
        // lastly, call preceding on baseIter (if applicable)
        if self.slice_start_idx == self.slice_end_idx {
            self.current = offset - 1;
            Ok(self.current)
        } else if offset == self.slice_end_idx + 1 {
            self.current = self.slice_start_idx + self.base_iter.last();
            Ok(self.current)
        } else {
            self.current =
                self.slice_start_idx + self.base_iter.preceding(offset - self.slice_start_idx)?;
            Ok(self.current)
        }
    }

    fn next_n(&mut self, n: i32) -> i32 {
        if n < 0 {
            for _ in 0..-(n as i64) {
                if self.previous() == DONE {
                    return DONE;
                }
            }
        } else {
            for _ in 0..n {
                if self.next() == DONE {
                    return DONE;
                }
            }
        }
        self.current()
    }

    /// The base class's `isBoundary(int)`.
    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        crate::uhighlight::custom_separator_break_iterator::java_is_boundary(self, offset)
    }

    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(Self {
            base_iter: self.base_iter.clone_box(),
            slice_char: self.slice_char,
            text: self.text.clone(),
            slice_start_idx: self.slice_start_idx,
            slice_end_idx: self.slice_end_idx,
            current: self.current,
            text_iter: self.text_iter.clone(),
        })
    }
}
