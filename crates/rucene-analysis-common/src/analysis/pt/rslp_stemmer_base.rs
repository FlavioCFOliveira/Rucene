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

//! Base class for stemmers that use a set of RSLP-like stemming steps,
//! ported from `org.apache.lucene.analysis.pt.RSLPStemmerBase`.

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::LazyLock;

use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::CharArraySet;
use rucene::error::{LuceneError, Result};

use crate::analysis::util::stemmer_util::StemmerUtil;
use crate::jdk::io::{InputStreamReader, LineNumberReader, ReadChars};
use crate::jdk::string::{ends_with_str, parse_int, to_rust_string, trim};
use crate::resources::get_resource_as_stream;

/// Base class for stemmers that use a set of RSLP-like stemming steps.
///
/// RSLP (Removedor de Sufixos da Lingua Portuguesa) is an algorithm designed
/// originally for stemming the Portuguese language, described in the paper
/// *A Stemming Algorithm for the Portuguese Language*, Orengo et. al.
///
/// Since this time a plural-only modification (RSLP-S) as well as a
/// modification for the Galician language have been implemented. This class
/// parses a configuration file that describes [`Step`]s, where each Step
/// contains a set of [`Rule`]s.
///
/// The general rule format is:
///
/// > { "suffix", N, "replacement", { "exception1", "exception2", ...}}
///
/// where:
///
/// * `suffix` is the suffix to be removed (such as "inho").
/// * `N` is the min stem size, where stem is defined as the candidate stem
///   after removing the suffix (but before appending the replacement!)
/// * `replacement` is an optimal string to append after removing the
///   suffix. This can be the empty string.
/// * `exceptions` is an optional list of exceptions, patterns that should
///   not be stemmed. These patterns can be specified as whole word or suffix
///   (ends-with) patterns, depending upon the exceptions format flag in the
///   step header.
///
/// A step is an ordered list of rules, with a structure in this format:
///
/// > { "name", N, B, { "cond1", "cond2", ... } ... rules ... };
///
/// where:
///
/// * `name` is a name for the step (such as "Plural").
/// * `N` is the min word size. Words that are less than this length bypass
///   the step completely, as an optimization. Note: N can be zero, in this
///   case this implementation will automatically calculate the appropriate
///   value from the underlying rules.
/// * `B` is a "boolean" flag specifying how exceptions in the rules are
///   matched. A value of 1 indicates whole-word pattern matching, a value of
///   0 indicates that exceptions are actually suffixes and should be matched
///   with ends-with.
/// * `conds` are an optional list of conditions to enter the step at all.
///   If the list is non-empty, then a word must end with one of these
///   conditions or it will bypass the step completely as an optimization.
///
/// * see: [RSLP description](http://www.inf.ufrgs.br/~viviane/rslp/index.htm)
///
/// Equivalent to `org.apache.lucene.analysis.pt.RSLPStemmerBase`
/// (`@lucene.internal`), an abstract class with only static members: the
/// type has no value, and its subclasses (`PortugueseStemmer`,
/// `PortugueseMinimalStemmer`, `GalicianStemmer`,
/// `GalicianMinimalStemmer`) call `RSLPStemmerBase::parse`. The strings
/// it handles are UTF-16 units, Java's `char[]`.
#[derive(Debug)]
pub enum RSLPStemmerBase {}

/// What a rule of a [`Step`] does: Java's virtual methods of `Rule`, which
/// `RuleWithSetExceptions` and `RuleWithSuffixExceptions` override.
pub trait RslpRule: Debug + Send + Sync {
    /// The members the rule inherits from `Rule`.
    fn rule(&self) -> &Rule;

    /// true if the word matches this rule.
    ///
    /// Equivalent to `Rule.matches(char[], int)` and its overrides.
    fn matches(&self, s: &[u16], len: usize) -> bool;

    /// new valid length of the string after firing this rule.
    ///
    /// Equivalent to `Rule.replace(char[], int)`.
    fn replace(&self, s: &mut [u16], len: usize) -> usize {
        self.rule().replace(s, len)
    }
}

/// A basic rule, with no exceptions.
///
/// Equivalent to `org.apache.lucene.analysis.pt.RSLPStemmerBase.Rule`.
#[derive(Debug, Clone)]
pub struct Rule {
    pub(crate) suffix: Vec<u16>,
    pub(crate) replacement: Vec<u16>,
    pub(crate) min: i32,
}

impl Rule {
    /// Create a rule.
    ///
    /// * `suffix`: suffix to remove
    /// * `min`: minimum stem length
    /// * `replacement`: replacement string
    ///
    /// Equivalent to `Rule(String, int, String)`.
    pub(crate) fn new(suffix: &[u16], min: i32, replacement: &[u16]) -> Self {
        Self {
            suffix: suffix.to_vec(),
            replacement: replacement.to_vec(),
            min,
        }
    }

    /// true if the word matches this rule.
    ///
    /// Equivalent to `Rule.matches(char[], int)`, the implementation the
    /// subclasses reach as `super.matches`.
    pub fn matches(&self, s: &[u16], len: usize) -> bool {
        len as i64 - self.suffix.len() as i64 >= i64::from(self.min)
            && StemmerUtil::ends_with_chars(s, len, &self.suffix)
    }

    /// new valid length of the string after firing this rule.
    ///
    /// Equivalent to `Rule.replace(char[], int)`.
    pub fn replace(&self, s: &mut [u16], len: usize) -> usize {
        if !self.replacement.is_empty() {
            let at = len - self.suffix.len();
            s[at..at + self.replacement.len()].copy_from_slice(&self.replacement);
        }
        len - self.suffix.len() + self.replacement.len()
    }
}

impl RslpRule for Rule {
    fn rule(&self) -> &Rule {
        self
    }

    fn matches(&self, s: &[u16], len: usize) -> bool {
        Rule::matches(self, s, len)
    }
}

/// A rule with a set of whole-word exceptions.
///
/// Equivalent to
/// `org.apache.lucene.analysis.pt.RSLPStemmerBase.RuleWithSetExceptions`,
/// which extends `Rule`.
#[derive(Debug)]
pub struct RuleWithSetExceptions {
    rule: Rule,
    pub(crate) exceptions: CharArraySet,
}

impl RuleWithSetExceptions {
    /// Equivalent to `RuleWithSetExceptions(String, int, String, String[])`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` "useless exception '<e>' does not end with
    /// '<suffix>'" when an exception does not end with the suffix.
    pub(crate) fn new(
        suffix: &[u16],
        min: i32,
        replacement: &[u16],
        exceptions: &[Vec<u16>],
    ) -> Result<Self> {
        let rule = Rule::new(suffix, min, replacement);
        for exception in exceptions {
            if !exception.ends_with(suffix) {
                return Err(runtime_exception(format!(
                    "useless exception '{}' does not end with '{}'",
                    to_rust_string(exception),
                    to_rust_string(suffix)
                )));
            }
        }
        let exceptions =
            CharArraySet::from_iter(exceptions.iter().map(|e| to_rust_string(e)), false);
        Ok(Self { rule, exceptions })
    }
}

impl RslpRule for RuleWithSetExceptions {
    fn rule(&self) -> &Rule {
        &self.rule
    }

    /// Equivalent to `RuleWithSetExceptions.matches(char[], int)`.
    fn matches(&self, s: &[u16], len: usize) -> bool {
        self.rule.matches(s, len) && !self.exceptions.contains_chars(Some(s), 0, len)
    }
}

/// A rule with a set of exceptional suffixes.
///
/// Equivalent to
/// `org.apache.lucene.analysis.pt.RSLPStemmerBase.RuleWithSuffixExceptions`,
/// which extends `Rule`.
#[derive(Debug)]
pub struct RuleWithSuffixExceptions {
    rule: Rule,
    // TODO: use a more efficient datastructure: automaton?
    pub(crate) exceptions: Vec<Vec<u16>>,
}

impl RuleWithSuffixExceptions {
    /// Equivalent to `RuleWithSuffixExceptions(String, int, String,
    /// String[])`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` "warning: useless exception '<e>' does not end
    /// with '<suffix>'" when an exception does not end with the suffix.
    pub(crate) fn new(
        suffix: &[u16],
        min: i32,
        replacement: &[u16],
        exceptions: &[Vec<u16>],
    ) -> Result<Self> {
        let rule = Rule::new(suffix, min, replacement);
        for exception in exceptions {
            if !exception.ends_with(suffix) {
                return Err(runtime_exception(format!(
                    "warning: useless exception '{}' does not end with '{}'",
                    to_rust_string(exception),
                    to_rust_string(suffix)
                )));
            }
        }
        Ok(Self {
            rule,
            exceptions: exceptions.to_vec(),
        })
    }
}

impl RslpRule for RuleWithSuffixExceptions {
    fn rule(&self) -> &Rule {
        &self.rule
    }

    /// Equivalent to `RuleWithSuffixExceptions.matches(char[], int)`.
    fn matches(&self, s: &[u16], len: usize) -> bool {
        if !self.rule.matches(s, len) {
            return false;
        }

        for exception in &self.exceptions {
            if StemmerUtil::ends_with_chars(s, len, exception) {
                return false;
            }
        }

        true
    }
}

/// A step containing a list of rules.
///
/// Equivalent to `org.apache.lucene.analysis.pt.RSLPStemmerBase.Step`.
#[derive(Debug)]
pub struct Step {
    pub(crate) name: String,
    /// Java's `rules`, null (`None`) when the step's rule list ends the
    /// file without its closing `;`.
    pub(crate) rules: Option<Vec<Box<dyn RslpRule>>>,
    pub(crate) min: i32,
    pub(crate) suffixes: Option<Vec<Vec<u16>>>,
}

impl Step {
    /// Create a new step
    ///
    /// * `name`: Step's name.
    /// * `rules`: an ordered list of rules.
    /// * `min`: minimum word size. if this is 0 it is automatically
    ///   calculated.
    /// * `suffixes`: optional list of conditional suffixes. may be null.
    ///
    /// Equivalent to `Step(String, Rule[], int, String[])`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` when `min` is 0 and `rules` is null, as
    /// Java's loop over the rules throws.
    pub(crate) fn new(
        name: String,
        rules: Option<Vec<Box<dyn RslpRule>>>,
        mut min: i32,
        suffixes: Option<Vec<Vec<u16>>>,
    ) -> Result<Self> {
        if min == 0 {
            min = i32::MAX;
            let Some(rules) = &rules else {
                return Err(LuceneError::null_pointer());
            };
            for r in rules {
                let r = r.rule();
                min = min.min(r.min.wrapping_add(r.suffix.len() as i32));
            }
        }

        let suffixes = match suffixes {
            Some(suffixes) if !suffixes.is_empty() => Some(suffixes),
            _ => None,
        };
        Ok(Self {
            name,
            rules,
            min,
            suffixes,
        })
    }

    /// new valid length of the string after applying the entire step.
    ///
    /// Equivalent to `Step.apply(char[], int)`.
    pub fn apply(&self, s: &mut [u16], len: usize) -> usize {
        if (len as i64) < i64::from(self.min) {
            return len;
        }

        if let Some(suffixes) = &self.suffixes {
            let mut found = false;

            for suffix in suffixes {
                if StemmerUtil::ends_with_chars(s, len, suffix) {
                    found = true;
                    break;
                }
            }

            if !found {
                return len;
            }
        }

        // Java dereferences `rules` here, throwing `NullPointerException`
        // for a step without them; `parse` is the only caller that builds
        // one (`Step::new` is crate-private), and the crate's two RSLP
        // resources close every step with `;`, so every step has its rules.
        let rules = self
            .rules
            .as_ref()
            .expect("INVARIANT: the crate's RSLP resources close every step");
        for rule in rules {
            if rule.matches(s, len) {
                return rule.replace(s, len);
            }
        }

        len
    }
}

/// `new RuntimeException(message)`.
fn runtime_exception(message: String) -> LuceneError {
    LuceneError::unchecked("java.lang.RuntimeException", message)
}

/// `RSLPStemmerBase.headerPattern`.
static HEADER_PATTERN: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile(
        r#"^\{\s*"([^"]*)",\s*([0-9]+),\s*(0|1),\s*\{(.*)\},\s*$"#,
        0,
    )
    .expect("INVARIANT: the pattern compiles")
});

/// `RSLPStemmerBase.stripPattern`.
static STRIP_PATTERN: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile(r#"^\{\s*"([^"]*)",\s*([0-9]+)\s*\}\s*(,|(\}\s*;))$"#, 0)
        .expect("INVARIANT: the pattern compiles")
});

/// `RSLPStemmerBase.repPattern`.
static REP_PATTERN: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile(
        r#"^\{\s*"([^"]*)",\s*([0-9]+),\s*"([^"]*)"\}\s*(,|(\}\s*;))$"#,
        0,
    )
    .expect("INVARIANT: the pattern compiles")
});

/// `RSLPStemmerBase.excPattern`.
static EXC_PATTERN: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile(
        r#"^\{\s*"([^"]*)",\s*([0-9]+),\s*"([^"]*)",\s*\{(.*)\}\s*\}\s*(,|(\}\s*;))$"#,
        0,
    )
    .expect("INVARIANT: the pattern compiles")
});

/// The text of group `group` of `matcher`'s match: Java's
/// `matcher.group(group)` of a group that took part in it.
fn group(matcher: &rucene::analysis::java_regex::JavaMatcher, group: i32) -> Vec<u16> {
    matcher
        .group(group)
        .expect("INVARIANT: the pattern has the group and has matched")
        .expect("INVARIANT: the group takes part in every match")
}

impl RSLPStemmerBase {
    /// Parse a resource file into an RSLP stemmer description.
    ///
    /// * returns: a Map containing the named Steps in this description.
    ///
    /// Equivalent to `parse(Class<? extends RSLPStemmerBase>, String)`:
    /// `class_name` is the binary name of the class whose
    /// `getResourceAsStream(resource)` Java reads, resolved against the
    /// crate's resources. Java wraps an `IOException` in a
    /// `RuntimeException`; the other exceptions pass unchanged.
    ///
    /// # Errors
    ///
    /// `RuntimeException` for an I/O error or a malformed step header or
    /// rule; `NullPointerException` when the resource is missing (Java's
    /// `new InputStreamReader(null, …)`); `NumberFormatException` for a
    /// number out of range.
    pub(crate) fn parse(class_name: &str, resource: &str) -> Result<HashMap<String, Step>> {
        // TODO: this parser is ugly, but works. use a jflex grammar instead.
        let io = |e: LuceneError| {
            if e.is_io_exception() {
                LuceneError::runtime_exception(e)
            } else {
                e
            }
        };
        let Some(is) = get_resource_as_stream(class_name, resource) else {
            return Err(LuceneError::null_pointer());
        };
        let mut r = LineNumberReader::new(InputStreamReader::new_utf8(is));
        let mut steps = HashMap::new();
        while let Some(step) = read_line(&mut r).map_err(io)? {
            let s = parse_step(&mut r, &step).map_err(io)?;
            steps.insert(s.name.clone(), s);
        }
        r.close().map_err(io)?;
        Ok(steps)
    }
}

/// Equivalent to `parseStep(LineNumberReader, String)`.
fn parse_step<S: ReadChars>(r: &mut LineNumberReader<S>, header: &[u16]) -> Result<Step> {
    let mut matcher = HEADER_PATTERN.matcher(header);
    if !matcher.find() {
        return Err(runtime_exception(format!(
            "Illegal Step header specified at line {}",
            r.get_line_number()
        )));
    }
    debug_assert_eq!(matcher.group_count(), 4);
    let name = to_rust_string(&group(&matcher, 1));
    let min = parse_int(&group(&matcher, 2))?;
    let r#type = parse_int(&group(&matcher, 3))?;
    let suffixes = parse_list(&group(&matcher, 4))?;
    let rules = parse_rules(r, r#type)?;
    Step::new(name, rules, min, suffixes)
}

/// Equivalent to `parseRules(LineNumberReader, int)`: `None` is Java's
/// `null`, when the input ends before a rule line ending with `;`.
fn parse_rules<S: ReadChars>(
    r: &mut LineNumberReader<S>,
    r#type: i32,
) -> Result<Option<Vec<Box<dyn RslpRule>>>> {
    let mut rules: Vec<Box<dyn RslpRule>> = Vec::new();
    while let Some(line) = read_line(r)? {
        let mut matcher = STRIP_PATTERN.matcher(&line);
        if matcher.matches() {
            rules.push(Box::new(Rule::new(
                &group(&matcher, 1),
                parse_int(&group(&matcher, 2))?,
                &[],
            )));
        } else {
            matcher = REP_PATTERN.matcher(&line);
            if matcher.matches() {
                rules.push(Box::new(Rule::new(
                    &group(&matcher, 1),
                    parse_int(&group(&matcher, 2))?,
                    &group(&matcher, 3),
                )));
            } else {
                matcher = EXC_PATTERN.matcher(&line);
                if matcher.matches() {
                    // Java evaluates the arguments left to right: the min,
                    // then the exception list.
                    let suffix = group(&matcher, 1);
                    let min = parse_int(&group(&matcher, 2))?;
                    let replacement = group(&matcher, 3);
                    let exceptions = parse_list(&group(&matcher, 4))?;
                    // Java passes a null array when the list is empty, and
                    // the constructors' loop over it throws.
                    let Some(exceptions) = exceptions else {
                        return Err(LuceneError::null_pointer());
                    };
                    if r#type == 0 {
                        rules.push(Box::new(RuleWithSuffixExceptions::new(
                            &suffix,
                            min,
                            &replacement,
                            &exceptions,
                        )?));
                    } else {
                        rules.push(Box::new(RuleWithSetExceptions::new(
                            &suffix,
                            min,
                            &replacement,
                            &exceptions,
                        )?));
                    }
                } else {
                    return Err(runtime_exception(format!(
                        "Illegal Step rule specified at line {}",
                        r.get_line_number()
                    )));
                }
            }
        }
        if ends_with_str(&line, ";") {
            return Ok(Some(rules));
        }
    }
    Ok(None)
}

/// Equivalent to `parseList(String)`: `None` is Java's `null`, for an
/// empty string; the list is `s.split(",")` (trailing empty strings
/// removed), each element trimmed and unquoted.
fn parse_list(s: &[u16]) -> Result<Option<Vec<Vec<u16>>>> {
    if s.is_empty() {
        return Ok(None);
    }
    let mut list: Vec<&[u16]> = s.split(|&c| c == u16::from(b',')).collect();
    while list.last().is_some_and(|last| last.is_empty()) {
        list.pop();
    }
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        out.push(parse_string(trim(item))?);
    }
    Ok(Some(out))
}

/// Equivalent to `parseString(String)`, `s.substring(1, s.length() - 1)`.
///
/// # Errors
///
/// `StringIndexOutOfBoundsException` for a string shorter than two units,
/// as `substring` throws.
fn parse_string(s: &[u16]) -> Result<Vec<u16>> {
    if s.len() < 2 {
        return Err(LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin 1, end {}, length {}", s.len() as i64 - 1, s.len()),
        ));
    }
    Ok(s[1..s.len() - 1].to_vec())
}

/// Equivalent to `readLine(LineNumberReader)`: the next line that is not
/// blank or a `#` comment, trimmed.
fn read_line<S: ReadChars>(r: &mut LineNumberReader<S>) -> Result<Option<Vec<u16>>> {
    while let Some(line) = r.read_line()? {
        let line = trim(&line);
        if !line.is_empty() && line[0] != u16::from(b'#') {
            return Ok(Some(line.to_vec()));
        }
    }
    Ok(None)
}
