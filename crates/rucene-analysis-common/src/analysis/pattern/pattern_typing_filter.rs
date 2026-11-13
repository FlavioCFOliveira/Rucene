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

//! A filter that sets a token's type and flags from the first of several
//! regular expressions it matches, ported from
//! `org.apache.lucene.analysis.pattern.PatternTypingFilter`.

use std::fmt;

use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::{
    CharTermAttribute, FlagsAttribute, TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::chars_to_utf16;

/// The body of [`PatternTypingFilter`]: what Lucene's `PatternTypingFilter`
/// adds to `TokenFilter`.
#[derive(Debug)]
pub struct PatternTypingFilterLogic {
    replacement_and_flag_by_pattern: Vec<PatternTypingRule>,
}

impl TokenFilterLogic for PatternTypingFilterLogic {
    /// Equivalent to the final `PatternTypingFilter.incrementToken()`.
    ///
    /// The type Java sets is the `String` `replaceFirst` returns; an unpaired
    /// surrogate in it becomes U+FFFD here, a Rust string holding none.
    ///
    /// # Errors
    ///
    /// What the input returns, and what `Matcher.replaceFirst` throws for a
    /// malformed type template.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let units = {
                let term = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                chars_to_utf16(&term.buffer()[..term.length()])
            };
            for rule in &self.replacement_and_flag_by_pattern {
                let mut matcher = rule.pattern().matcher(&units);
                if matcher.find() {
                    // allow 2nd reset() and find() that occurs inside replaceFirst to avoid excess string
                    // creation
                    let type_value =
                        String::from_utf16_lossy(&matcher.replace_first(rule.type_template())?);
                    source
                        .get_attribute_interface_mut::<dyn TypeAttribute>()
                        .expect("INVARIANT: the constructor added the TypeAttribute")
                        .set_type(type_value);
                    source
                        .get_attribute_interface_mut::<dyn FlagsAttribute>()
                        .expect("INVARIANT: the constructor added the FlagsAttribute")
                        .set_flags(rule.flags());
                    return Ok(true);
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// Set a type attribute to a parameterized value when tokens are matched by
/// any of a several regex patterns. The value set in the type attribute is
/// parameterized with the match groups of the regex used for matching. In
/// combination with TypeAsSynonymFilter and DropIfFlagged filter this can
/// supply complex synonym patterns that are protected from subsequent
/// analysis, and optionally drop the original term based on the flag set in
/// this filter. See `PatternTypingFilterFactory` for full documentation.
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternTypingFilter`
/// (since 8.8.0).
pub type PatternTypingFilter = TokenFilter<PatternTypingFilterLogic>;

/// Equivalent to `PatternTypingFilter(TokenStream, PatternTypingRule...)`,
/// which adds the `CharTermAttribute`, the `FlagsAttribute` and the
/// `TypeAttribute`. Java's `NullPointerException` for a `null` rule array or
/// rule has no counterpart: a slice of rules holds neither.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_pattern_typing_filter(
    input: Box<dyn TokenStream>,
    replacement_and_flag_by_pattern: &[PatternTypingRule],
) -> Result<PatternTypingFilter> {
    let mut filter = TokenFilter::new(
        input,
        PatternTypingFilterLogic {
            replacement_and_flag_by_pattern: replacement_and_flag_by_pattern.to_vec(),
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn FlagsAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}

/// Value holding class for pattern typing rules.
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternTypingFilter$PatternTypingRule`,
/// a `record`: its components are the accessors, and its `toString()` is
/// [`Display`](fmt::Display). The record's `equals` compares the `Pattern`
/// by identity, which Rust cannot observe on a value, so the type has no
/// equality.
#[derive(Debug, Clone)]
pub struct PatternTypingRule {
    pattern: JavaPattern,
    flags: i32,
    type_template: String,
}

impl PatternTypingRule {
    /// The canonical constructor, `PatternTypingRule(Pattern, int, String)`.
    pub fn new(pattern: JavaPattern, flags: i32, type_template: impl Into<String>) -> Self {
        Self {
            pattern,
            flags,
            type_template: type_template.into(),
        }
    }

    /// The accessor `pattern()`.
    pub fn pattern(&self) -> &JavaPattern {
        &self.pattern
    }

    /// The accessor `flags()`.
    pub fn flags(&self) -> i32 {
        self.flags
    }

    /// The accessor `typeTemplate()`.
    pub fn type_template(&self) -> &str {
        &self.type_template
    }
}

impl fmt::Display for PatternTypingRule {
    /// The record's `toString()`, `Pattern.toString()` being the pattern's
    /// source.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PatternTypingRule[pattern={}, flags={}, typeTemplate={}]",
            self.pattern.pattern(),
            self.flags,
            self.type_template
        )
    }
}
