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

//! A filter that emits one token per capture group of one or more regular
//! expressions, ported from
//! `org.apache.lucene.analysis.pattern.PatternCaptureGroupTokenFilter`.

use rucene::analysis::java_regex::{JavaMatcher, JavaPattern};
use rucene::analysis::{
    CharTermAttribute, PositionIncrementAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`PatternCaptureGroupTokenFilter`]: what Lucene's
/// `PatternCaptureGroupTokenFilter` adds to `TokenFilter`.
///
/// Java's `CharsRefBuilder spare` holds the current token as UTF-16 units,
/// which the matchers search; a capture copied into the term has its
/// unpaired surrogates turned into U+FFFD (`jdk::character::utf16_to_term_chars`).
#[derive(Debug)]
pub struct PatternCaptureGroupTokenFilterLogic {
    state: Option<CapturedState>,
    matchers: Vec<JavaMatcher>,
    spare: Vec<u16>,
    group_counts: Vec<i32>,
    preserve_original: bool,
    current_group: Vec<i32>,
    current_matcher: i32,
}

/// Sets the term to `units`.
fn set_term(source: &AttributeSource, units: &[u16]) {
    let chars = utf16_to_term_chars(units);
    source
        .get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("INVARIANT: the constructor added the CharTermAttribute")
        .copy_buffer(&chars, 0, chars.len());
}

impl PatternCaptureGroupTokenFilterLogic {
    /// Equivalent to the private `nextCapture()`.
    fn next_capture(&mut self) -> Result<bool> {
        let mut min_offset = i32::MAX;
        self.current_matcher = -1;

        let mut i: i32 = 0;
        while i < self.matchers.len() as i32 {
            let iu = i as usize;
            let matcher = &mut self.matchers[iu];
            if self.current_group[iu] == -1 {
                self.current_group[iu] = if matcher.find() { 1 } else { 0 };
            }
            if self.current_group[iu] != 0 {
                while self.current_group[iu] < self.group_counts[iu] + 1 {
                    let start = matcher.start_group(self.current_group[iu])?;
                    let end = matcher.end_group(self.current_group[iu])?;
                    if start == end
                        || self.preserve_original && start == 0 && self.spare.len() as i32 == end
                    {
                        self.current_group[iu] += 1;
                        continue;
                    }
                    if start < min_offset {
                        min_offset = start;
                        self.current_matcher = i;
                    }
                    break;
                }
                if self.current_group[iu] == self.group_counts[iu] + 1 {
                    self.current_group[iu] = -1;
                    i -= 1;
                }
            }
            i += 1;
        }
        Ok(self.current_matcher != -1)
    }

    /// The bounds of the current capture of the current matcher.
    fn current_bounds(&self) -> Result<(i32, i32)> {
        let matcher = &self.matchers[self.current_matcher as usize];
        let group = self.current_group[self.current_matcher as usize];
        Ok((matcher.start_group(group)?, matcher.end_group(group)?))
    }
}

impl TokenFilterLogic for PatternCaptureGroupTokenFilterLogic {
    /// Equivalent to `PatternCaptureGroupTokenFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.current_matcher != -1 && self.next_capture()? {
            debug_assert!(self.state.is_some());
            source.clear_attributes();
            if let Some(state) = &self.state {
                source.restore_state(state)?;
            }
            let (start, end) = self.current_bounds()?;

            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(0);
            set_term(source, &self.spare[start as usize..end as usize]);
            self.current_group[self.current_matcher as usize] += 1;
            return Ok(true);
        }

        if !input.increment_token()? {
            return Ok(false);
        }

        self.spare = {
            let term = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            chars_to_utf16(&term.buffer()[..term.length()])
        };
        self.state = source.capture_state();

        for i in 0..self.matchers.len() {
            self.matchers[i].reset_input(&self.spare);
            self.current_group[i] = -1;
        }

        if self.preserve_original {
            self.current_matcher = 0;
        } else if self.next_capture()? {
            let (start, end) = self.current_bounds()?;

            // if we start at 0 we can simply set the length and save the copy
            // (`charTermAttr.setLength(end)` keeps the first `end` units of
            // the term, which are `spare`'s)
            set_term(source, &self.spare[start as usize..end as usize]);
            self.current_group[self.current_matcher as usize] += 1;
        }
        Ok(true)
    }

    /// Equivalent to `PatternCaptureGroupTokenFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.state = None;
        self.current_matcher = -1;
        Ok(())
    }
}

/// CaptureGroup uses Java regexes to emit multiple tokens - one for each
/// capture group in one or more patterns.
///
/// For example, a pattern like:
///
/// `"(https?://([a-zA-Z\-_0-9.]+))"`
///
/// when matched against the string `"http://www.foo.com/index"` would return
/// the tokens `"https://www.foo.com"` and `"www.foo.com"`.
///
/// If none of the patterns match, or if preserveOriginal is true, the
/// original token will be preserved.
///
/// Each pattern is matched as often as it can be, so the pattern
/// `"(...)"`, when matched against `"abcdefghi"` would produce
/// `["abc","def","ghi"]`
///
/// A camelCaseFilter could be written as:
///
/// ```text
///   "([A-Z]{2,})",
///   "(?<![A-Z])([A-Z][a-z]+)",
///   "(?:^|\\b|(?<=[0-9_])|(?<=[A-Z]{2}))([a-z]+)",
///   "([0-9]+)"
/// ```
///
/// plus if preserveOriginal is true, it would also return
/// `"camelCaseFilter"`
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternCaptureGroupTokenFilter`.
pub type PatternCaptureGroupTokenFilter = TokenFilter<PatternCaptureGroupTokenFilterLogic>;

/// * `input` the input `TokenStream`
/// * `preserve_original` set to true to return the original token even if
///   one of the patterns matches
/// * `patterns` an array of `Pattern` objects to match against each token
///
/// Equivalent to `PatternCaptureGroupTokenFilter(TokenStream, boolean,
/// Pattern...)`, which adds the `CharTermAttribute` and the
/// `PositionIncrementAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_pattern_capture_group_token_filter(
    input: Box<dyn TokenStream>,
    preserve_original: bool,
    patterns: &[JavaPattern],
) -> Result<PatternCaptureGroupTokenFilter> {
    let mut matchers = Vec::with_capacity(patterns.len());
    let mut group_counts = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let matcher = pattern.matcher(&[]);
        group_counts.push(matcher.group_count() as i32);
        matchers.push(matcher);
    }
    let logic = PatternCaptureGroupTokenFilterLogic {
        state: None,
        current_group: vec![-1; patterns.len()],
        matchers,
        spare: Vec::new(),
        group_counts,
        preserve_original,
        // Java's field is never initialized, so it starts at 0.
        current_matcher: 0,
    };
    let mut filter = TokenFilter::new(input, logic);
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    Ok(filter)
}
