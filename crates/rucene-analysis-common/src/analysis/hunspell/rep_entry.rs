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

//! A `REP` replacement, ported from
//! `org.apache.lucene.analysis.hunspell.RepEntry`.

use std::fmt;

use crate::jdk::string::{concat, ends_with_str, index_of, replace_unit, starts_with_str};

/// A replacement rule of the `REP` table: a pattern, optionally anchored at
/// the start (`^`) or the end (`$`) of the word, and its replacement.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.RepEntry`.
#[derive(Debug, Clone)]
pub(crate) struct RepEntry {
    pattern: Vec<u16>,
    replacement: Vec<u16>,
    must_start: bool,
    must_end: bool,
    pattern_len: usize,
}

impl RepEntry {
    /// Equivalent to `RepEntry(String, String)`: `_` in the replacement
    /// stands for a space.
    pub(crate) fn new(raw_pattern: &[u16], raw_replacement: &[u16]) -> Self {
        let must_start = starts_with_str(raw_pattern, "^");
        let must_end = ends_with_str(raw_pattern, "$");
        let start = usize::from(must_start);
        let end = raw_pattern.len() - usize::from(must_end);
        // a raw pattern of fewer than two units cannot both start with `^`
        // and end with `$`, so `start <= end`
        let pattern = raw_pattern[start..end].to_vec();
        let replacement = replace_unit(raw_replacement, u16::from(b'_'), u16::from(b' '));
        let pattern_len = pattern.len();
        Self {
            pattern,
            replacement,
            must_start,
            must_end,
            pattern_len,
        }
    }

    /// Equivalent to `isMiddle()`.
    pub(crate) fn is_middle(&self) -> bool {
        !self.must_start && !self.must_end
    }

    /// The words this rule makes of `word`, one per place it applies.
    ///
    /// Equivalent to `substitute(String)`.
    pub(crate) fn substitute(&self, word: &[u16]) -> Vec<Vec<u16>> {
        if self.must_start {
            let matches = if self.must_end {
                word == self.pattern.as_slice()
            } else {
                word.starts_with(&self.pattern)
            };
            return if matches {
                vec![concat(&[&self.replacement, &word[self.pattern_len..]])]
            } else {
                Vec::new()
            };
        }
        if self.must_end {
            return if word.ends_with(&self.pattern) {
                vec![concat(&[
                    &word[..word.len() - self.pattern_len],
                    &self.replacement,
                ])]
            } else {
                Vec::new()
            };
        }
        let Some(mut pos) = index_of(word, &self.pattern, 0) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        loop {
            result.push(concat(&[
                &word[..pos],
                &self.replacement,
                &word[pos + self.pattern_len..],
            ]));
            match index_of(word, &self.pattern, pos + 1) {
                Some(next) => pos = next,
                None => break,
            }
        }
        result
    }
}

impl fmt::Display for RepEntry {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}->{}",
            if self.must_start { "^" } else { "" },
            String::from_utf16_lossy(&self.pattern),
            if self.must_end { "$" } else { "" },
            String::from_utf16_lossy(&self.replacement)
        )
    }
}
