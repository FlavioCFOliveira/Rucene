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

//! A filter that replaces regular expression matches in each token, ported
//! from `org.apache.lucene.analysis.pattern.PatternReplaceFilter`.

use rucene::analysis::java_regex::{JavaMatcher, JavaPattern};
use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`PatternReplaceFilter`]: what Lucene's
/// `PatternReplaceFilter` adds to `TokenFilter`.
///
/// Java's matcher reads the `CharTermAttribute` itself, a live
/// `CharSequence`; here `m.reset()` hands it the term's current units.
#[derive(Debug)]
pub struct PatternReplaceFilterLogic {
    replacement: String,
    all: bool,
    m: JavaMatcher,
}

impl TokenFilterLogic for PatternReplaceFilterLogic {
    /// Equivalent to `PatternReplaceFilter.incrementToken()`.
    ///
    /// # Errors
    ///
    /// What the input returns, and what `Matcher.replaceAll` and
    /// `replaceFirst` throw for a malformed replacement.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        let units = {
            let term = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            chars_to_utf16(&term.buffer()[..term.length()])
        };
        self.m.reset_input(&units);
        if self.m.find() {
            // replaceAll/replaceFirst will reset() this previous find.
            let transformed = if self.all {
                self.m.replace_all(&self.replacement)?
            } else {
                self.m.replace_first(&self.replacement)?
            };
            let chars = utf16_to_term_chars(&transformed);
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute")
                .copy_buffer(&chars, 0, chars.len());
        }

        Ok(true)
    }
}

/// A TokenFilter which applies a Pattern to each token in the stream,
/// replacing match occurrences with the specified replacement string.
///
/// **Note:** Depending on the input and the pattern used and the input
/// TokenStream, this TokenFilter may produce Tokens whose text is the empty
/// string.
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternReplaceFilter`.
pub type PatternReplaceFilter = TokenFilter<PatternReplaceFilterLogic>;

/// Constructs an instance to replace either the first, or all occurrences
///
/// * `input` the TokenStream to process
/// * `p` the patterm to apply to each Token
/// * `replacement` the "replacement string" to substitute, if `None` a blank
///   string will be used. Note that this is not the literal string that will
///   be used, '$' and '\' have special meaning.
/// * `all` if true, all matches will be replaced otherwise just the first
///   match.
///
/// Equivalent to `PatternReplaceFilter(TokenStream, Pattern, String,
/// boolean)`, which adds the `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_pattern_replace_filter(
    input: Box<dyn TokenStream>,
    p: &JavaPattern,
    replacement: Option<&str>,
    all: bool,
) -> Result<PatternReplaceFilter> {
    let logic = PatternReplaceFilterLogic {
        replacement: replacement.unwrap_or("").to_string(),
        all,
        m: p.matcher(&[]),
    };
    let mut filter = TokenFilter::new(input, logic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
