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

//! A char filter that replaces regular expression matches in the whole
//! input, ported from `org.apache.lucene.analysis.pattern.PatternReplaceCharFilter`.

use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::{CharFilter, CharReader};
use rucene::error::Result;

use crate::analysis::charfilter::base_char_filter::BaseCharFilter;

/// CharFilter that uses a regular expression for the target of replace
/// string. The pattern match will be done in each "block" in char stream.
///
/// ex1) source="aa&nbsp;&nbsp;bb&nbsp;aa&nbsp;bb", pattern="(aa)\\s+(bb)"
/// replacement="$1#$2"
/// output="aa#bb&nbsp;aa#bb" NOTE: If you produce a phrase that has
/// different length to source string and the field is used for highlighting
/// for a term of the phrase, you will face a trouble.
///
/// ex2) source="aa123bb", pattern="(aa)\\d+(bb)" replacement="$1&nbsp;$2"
/// output="aa&nbsp;bb"
/// and you want to search bb and highlight it, you will get
/// highlight snippet="aa1&lt;em&gt;23bb&lt;/em&gt;"
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternReplaceCharFilter`
/// (since Solr 1.5), a `BaseCharFilter`: the inherited state is the
/// [`BaseCharFilter`] it holds. Java's `transformedInput`, a `StringReader`
/// over the replaced text, is the text and the position of the next unit to
/// read.
#[derive(Debug)]
pub struct PatternReplaceCharFilter {
    base: BaseCharFilter,
    pattern: JavaPattern,
    replacement: String,
    transformed_input: Option<(Vec<u16>, usize)>,
}

impl PatternReplaceCharFilter {
    /// Equivalent to `PatternReplaceCharFilter(Pattern, String, Reader)`;
    /// Java's `Objects.requireNonNull` checks are the types' here.
    pub fn new(
        pattern: JavaPattern,
        replacement: impl Into<String>,
        input: Box<dyn CharReader>,
    ) -> Self {
        Self {
            base: BaseCharFilter::new(input),
            pattern,
            replacement: replacement.into(),
            transformed_input: None,
        }
    }

    /// Equivalent to the private `fill()`: the whole input, read 1024 units
    /// at a time, replaced by [`process_pattern`](Self::process_pattern).
    fn fill(&mut self) -> Result<()> {
        let mut buffered: Vec<u16> = Vec::new();
        let mut temp = [0u16; 1024];
        loop {
            let cnt = self.base.input_mut().read(&mut temp)?;
            if cnt == 0 {
                break;
            }
            buffered.extend_from_slice(&temp[..cnt]);
        }
        let transformed = self.process_pattern(&buffered)?;
        self.transformed_input = Some((transformed, 0));
        Ok(())
    }

    /// Replace pattern in input and mark correction offsets.
    ///
    /// Equivalent to the package-private
    /// `PatternReplaceCharFilter.processPattern(CharSequence)`.
    ///
    /// # Errors
    ///
    /// What `Matcher.appendReplacement` throws for a malformed replacement.
    pub(crate) fn process_pattern(&mut self, input: &[u16]) -> Result<Vec<u16>> {
        let mut m = self.pattern.matcher(input);

        let mut cumulative_output: Vec<u16> = Vec::new();
        let mut cumulative = 0;
        let mut last_match_end = 0;
        while m.find() {
            let group_size = m.end()? - m.start()?;
            let skipped_size = m.start()? - last_match_end;
            last_match_end = m.end()?;

            let length_before_replacement = cumulative_output.len() as i32 + skipped_size;
            m.append_replacement(&mut cumulative_output, &self.replacement)?;
            // Matcher doesn't tell us how many characters have been appended before the replacement.
            // So we need to calculate it. Skipped characters have been added as part of
            // appendReplacement.
            let replacement_size = cumulative_output.len() as i32 - length_before_replacement;

            if group_size != replacement_size {
                if replacement_size < group_size {
                    // The replacement is smaller.
                    // Add the 'backskip' to the next index after the replacement (this is possibly
                    // after the end of string, but it's fine -- it just means the last character
                    // of the replaced block doesn't reach the end of the original string.
                    cumulative += group_size - replacement_size;
                    let at_index = length_before_replacement + replacement_size;
                    self.base.add_off_correct_map(at_index, cumulative);
                } else {
                    // The replacement is larger. Every new index needs to point to the last
                    // element of the original group (if any).
                    for i in group_size..replacement_size {
                        cumulative -= 1;
                        self.base
                            .add_off_correct_map(length_before_replacement + i, cumulative);
                    }
                }
            }
        }

        // Append the remaining output, no further changes to indices.
        m.append_tail(&mut cumulative_output);
        Ok(cumulative_output)
    }
}

impl CharReader for PatternReplaceCharFilter {
    /// Equivalent to `PatternReplaceCharFilter.read(char[], int, int)` (and
    /// to `read()`, one unit at a time): the whole input is buffered and
    /// replaced on the first call, and then read as Java's `StringReader`
    /// reads (0 here where Java answers `-1`).
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        // Buffer all input on the first call.
        if self.transformed_input.is_none() {
            self.fill()?;
        }

        let (text, next) = self
            .transformed_input
            .as_mut()
            .expect("INVARIANT: filled above");
        if buf.is_empty() || *next >= text.len() {
            return Ok(0);
        }
        let n = buf.len().min(text.len() - *next);
        buf[..n].copy_from_slice(&text[*next..*next + n]);
        *next += n;
        Ok(n)
    }

    /// Equivalent to the inherited `CharFilter.close()`, `input.close()`.
    fn close(&mut self) -> Result<()> {
        self.base.input_mut().close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for PatternReplaceCharFilter {
    /// Equivalent to `PatternReplaceCharFilter.correct(int)`, `Math.max(0,
    /// super.correct(currentOff))`.
    fn correct(&self, current_off: i32) -> i32 {
        0.max(self.base.correct(current_off))
    }

    fn input(&self) -> &dyn CharReader {
        self.base.input()
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        self.base.input_mut()
    }
}
