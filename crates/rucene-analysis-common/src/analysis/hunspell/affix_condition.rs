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

//! The condition of an affix rule, ported from
//! `org.apache.lucene.analysis.hunspell.AffixCondition`.

use std::fmt::Debug;
use std::sync::{Arc, LazyLock};

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::automaton::character_run_automaton::CharacterRunAutomaton;
use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::automaton::reg_exp::RegExp;

use super::affix_kind::AffixKind;
use crate::jdk::string::{
    concat, contains_str, index_of_unit, last_index_of_unit, to_rust_string, units,
};

/// Checks the "condition" part of affix definition, as in
///
/// ```text
/// PFX flag stripping prefix [condition [morphological_fields...]]
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.AffixCondition`. Its
/// constants are [`ALWAYS_TRUE_KEY`], [`always_true`] and [`always_false`];
/// its static methods [`unique_key`] and [`compile`].
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub trait AffixCondition: Debug + Send + Sync {
    /// Equivalent to the default `acceptsStem(String)`.
    fn accepts_stem_string(&self, stem: &[u16]) -> bool {
        self.accepts_stem(stem, 0, stem.len())
    }

    /// Whether the given word matches this condition as a stem with both
    /// "strip" and "affix" removed.
    ///
    /// Equivalent to `acceptsStem(char[], int, int)`.
    fn accepts_stem(&self, word: &[u16], offset: usize, length: usize) -> bool;
}

/// Equivalent to `AffixCondition.ALWAYS_TRUE_KEY`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub const ALWAYS_TRUE_KEY: &str = ".*";

/// The condition every stem meets.
#[derive(Debug)]
struct AlwaysTrue;

impl AffixCondition for AlwaysTrue {
    fn accepts_stem_string(&self, _stem: &[u16]) -> bool {
        true
    }

    fn accepts_stem(&self, _word: &[u16], _offset: usize, _length: usize) -> bool {
        true
    }
}

/// The condition no stem meets.
#[derive(Debug)]
struct AlwaysFalse;

impl AffixCondition for AlwaysFalse {
    fn accepts_stem_string(&self, _stem: &[u16]) -> bool {
        false
    }

    fn accepts_stem(&self, _word: &[u16], _offset: usize, _length: usize) -> bool {
        false
    }
}

/// Equivalent to `AffixCondition.ALWAYS_TRUE`.
pub(crate) fn always_true() -> Arc<dyn AffixCondition> {
    static ALWAYS_TRUE: LazyLock<Arc<dyn AffixCondition>> = LazyLock::new(|| Arc::new(AlwaysTrue));
    Arc::clone(&ALWAYS_TRUE)
}

/// Equivalent to `AffixCondition.ALWAYS_FALSE`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub fn always_false() -> Arc<dyn AffixCondition> {
    static ALWAYS_FALSE: LazyLock<Arc<dyn AffixCondition>> =
        LazyLock::new(|| Arc::new(AlwaysFalse));
    Arc::clone(&ALWAYS_FALSE)
}

/// A key used to deduplicate same condition+strip+kind triples. For trivial
/// conditions that need no check, [`ALWAYS_TRUE_KEY`] is returned.
///
/// Equivalent to `AffixCondition.uniqueKey(AffixKind, String, String)`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub fn unique_key(kind: AffixKind, strip: &[u16], condition: &[u16]) -> Vec<u16> {
    if condition == units(".").as_slice()
        || kind == AffixKind::Prefix && strip.starts_with(condition)
        || kind == AffixKind::Suffix && strip.ends_with(condition) && !is_regexp(condition)
    {
        return units(ALWAYS_TRUE_KEY);
    }
    concat(&[
        condition,
        &units(" "),
        &units(kind.name()),
        &units(" "),
        strip,
    ])
}

/// What escapes the `try` of `compile`: a `PatternSyntaxException`, or any
/// other `Throwable`.
enum CompileFailure {
    PatternSyntax,
    Other(LuceneError),
}

impl From<LuceneError> for CompileFailure {
    fn from(e: LuceneError) -> Self {
        Self::Other(e)
    }
}

/// Analyzes the given affix kind, strip and condition and returns an object
/// able to efficiently check that condition.
///
/// Equivalent to `AffixCondition.compile(AffixKind, String, String, String)`.
/// Java's `String.matches` runs the condition as a `java.util.regex` pattern
/// (the core emulation [`JavaPattern`]); the rest compiles it with Lucene's
/// `RegExp`.
///
/// # Errors
///
/// `IllegalArgumentException("On line: " + line)` with the cause, for any
/// failure but a `PatternSyntaxException` (which makes the condition
/// [`always_false`]).
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub fn compile(
    kind: AffixKind,
    strip: &[u16],
    condition: &[u16],
    line: &[u16],
) -> Result<Arc<dyn AffixCondition>> {
    if !is_regexp(condition) {
        if kind == AffixKind::Suffix && condition.ends_with(strip) {
            return Ok(substring_condition(
                kind,
                condition[..condition.len() - strip.len()].to_vec(),
            ));
        }
        if kind == AffixKind::Prefix && condition.starts_with(strip) {
            return Ok(substring_condition(kind, condition[strip.len()..].to_vec()));
        }
        return Ok(always_false());
    }
    let mut condition = condition.to_vec();
    if let Some(last_bracket) = last_index_of_unit(&condition, u16::from(b'[')) {
        if index_of_unit(&condition, u16::from(b']'), last_bracket + 1).is_none() {
            // unclosed [ is tolerated by Hunspell and occurs in some dictionaries
            condition.push(u16::from(b']'));
        }
    }
    match compile_regexp(kind, strip, &condition) {
        Ok(compiled) => Ok(compiled),
        Err(CompileFailure::PatternSyntax) => Ok(always_false()),
        Err(CompileFailure::Other(e)) => Err(LuceneError::illegal_argument_with_cause(
            format!("On line: {}", to_rust_string(line)),
            Arc::new(e),
        )),
    }
}

/// The body of the `try` of `compile`.
fn compile_regexp(
    kind: AffixKind,
    strip: &[u16],
    condition: &[u16],
) -> std::result::Result<Arc<dyn AffixCondition>, CompileFailure> {
    let condition_chars = count_char_patterns(condition)?;
    if condition_chars <= strip.len() {
        let regex = if kind == AffixKind::Prefix {
            concat(&[&units(".*"), condition])
        } else {
            concat(&[condition, &units(".*")])
        };
        return Ok(if string_matches(strip, &regex)? {
            always_true()
        } else {
            always_false()
        });
    }
    if kind == AffixKind::Prefix {
        let split = skip_char_patterns(condition, strip.len())?;
        if !string_matches(strip, &condition[..split])? {
            return Ok(always_false());
        }
        return Ok(regexp_condition(
            kind,
            &condition[split..],
            condition_chars - strip.len(),
        )?);
    }
    let split = skip_char_patterns(condition, condition_chars - strip.len())?;
    if !string_matches(strip, &condition[split..])? {
        return Ok(always_false());
    }
    Ok(regexp_condition(
        kind,
        &condition[..split],
        condition_chars - strip.len(),
    )?)
}

/// `s.matches(regex)`: `Pattern.compile(regex).matcher(s).matches()`.
fn string_matches(s: &[u16], regex: &[u16]) -> std::result::Result<bool, CompileFailure> {
    match JavaPattern::compile(&to_rust_string(regex), 0) {
        Ok(pattern) => Ok(pattern.matches(&to_rust_string(s))),
        Err(_) => Err(CompileFailure::PatternSyntax),
    }
}

/// Equivalent to `skipCharPatterns(String, int)`.
fn skip_char_patterns(condition: &[u16], count: usize) -> Result<usize> {
    let mut pos = 0;
    for _ in 0..count {
        pos = skip_char_pattern(condition, pos)?;
    }
    Ok(pos)
}

/// Equivalent to `countCharPatterns(String)`.
fn count_char_patterns(condition: &[u16]) -> Result<usize> {
    let mut condition_chars = 0;
    let mut i = 0;
    while i < condition.len() {
        condition_chars += 1;
        i = skip_char_pattern(condition, i)?;
    }
    Ok(condition_chars)
}

/// Equivalent to `skipCharPattern(String, int)`.
///
/// # Errors
///
/// Java's `AssertionError("Malformed condition " + condition)` for an
/// unclosed `[`.
fn skip_char_pattern(condition: &[u16], pos: usize) -> Result<usize> {
    let mut pos = pos;
    if condition[pos] == u16::from(b'[') {
        match index_of_unit(condition, u16::from(b']'), pos + 1) {
            Some(close) => pos = close,
            None => {
                return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                    "Malformed condition {}",
                    to_rust_string(condition)
                ))))
            }
        }
    }
    Ok(pos + 1)
}

/// Equivalent to `isRegexp(String)`.
fn is_regexp(condition: &[u16]) -> bool {
    contains_str(condition, "[") || contains_str(condition, ".") || contains_str(condition, "-")
}

/// The condition that the stem starts (prefix) or ends (suffix) with
/// `stem_condition`.
///
/// Equivalent to the lambda of `substringCondition(AffixKind, String)`.
#[derive(Debug)]
struct SubstringCondition {
    for_suffix: bool,
    stem_condition: Vec<u16>,
}

impl AffixCondition for SubstringCondition {
    fn accepts_stem(&self, word: &[u16], offset: usize, length: usize) -> bool {
        let cond_length = self.stem_condition.len();
        if length < cond_length {
            return false;
        }
        let match_start = if self.for_suffix {
            offset + length - cond_length
        } else {
            offset
        };
        self.stem_condition == word[match_start..match_start + cond_length]
    }
}

/// Equivalent to `substringCondition(AffixKind, String)`.
fn substring_condition(kind: AffixKind, stem_condition: Vec<u16>) -> Arc<dyn AffixCondition> {
    Arc::new(SubstringCondition {
        for_suffix: kind == AffixKind::Suffix,
        stem_condition,
    })
}

/// The condition that the first (prefix) or last (suffix) `char_count`
/// units of the stem are accepted by `automaton`.
///
/// Equivalent to the lambda of `regexpCondition(AffixKind, String, int)`.
#[derive(Debug)]
struct RegexpCondition {
    for_suffix: bool,
    automaton: CharacterRunAutomaton,
    char_count: usize,
}

impl AffixCondition for RegexpCondition {
    fn accepts_stem(&self, word: &[u16], offset: usize, length: usize) -> bool {
        length >= self.char_count
            && self.automaton.run_units(
                word,
                if self.for_suffix {
                    offset + length - self.char_count
                } else {
                    offset
                },
                self.char_count,
            )
    }
}

/// Equivalent to `regexpCondition(AffixKind, String, int)`.
fn regexp_condition(
    kind: AffixKind,
    condition: &[u16],
    char_count: usize,
) -> Result<Arc<dyn AffixCondition>> {
    let regexp = RegExp::with_flags(&to_rust_string(&escape_dash(condition)), RegExp::NONE)?;
    let automaton = CharacterRunAutomaton::new(Operations::determinize(
        &regexp.to_automaton()?,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )?)?;
    Ok(Arc::new(RegexpCondition {
        for_suffix: kind == AffixKind::Suffix,
        automaton,
        char_count,
    }))
}

/// "dash hasn't got special meaning" (we must escape it).
///
/// Equivalent to `escapeDash(String)`.
fn escape_dash(re: &[u16]) -> Vec<u16> {
    let dash = u16::from(b'-');
    if !re.contains(&dash) {
        return re.to_vec();
    }
    // we have to be careful, even though dash doesn't have a special meaning,
    // some dictionaries already escape it (e.g. pt_PT), so we don't want to nullify it
    let mut escaped = Vec::with_capacity(re.len() + 4);
    let mut i = 0;
    while i < re.len() {
        let c = re[i];
        if c == dash {
            escaped.extend_from_slice(&units("\\-"));
        } else {
            escaped.push(c);
            if c == u16::from(b'\\') && i + 1 < re.len() {
                escaped.push(re[i + 1]);
                i += 1;
            }
        }
        i += 1;
    }
    escaped
}
