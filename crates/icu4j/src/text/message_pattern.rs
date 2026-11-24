// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 *   Copyright (C) 2010-2016, International Business Machines
 *   Corporation and others.  All Rights Reserved.
 *******************************************************************************
 *   created on: 2010aug21
 *   created by: Markus W. Scherer
 */

//! `com.ibm.icu.text.MessagePattern`.

use std::fmt;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::jdk::{index_of_unit, is_high_surrogate, to_rust_string};
use crate::r#impl::icu_config::ICUConfig;
use crate::r#impl::pattern_props::PatternProps;
use crate::util::freezable::Freezable;

/// Mode for when an apostrophe starts quoted literal text for
/// `MessageFormat` output.
///
/// Equivalent to the enum `MessagePattern.ApostropheMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApostropheMode {
    /// `DOUBLE_OPTIONAL`: a single apostrophe only starts quoted literal
    /// text if it immediately precedes a syntax character (`{}`, `#` in
    /// plural style, `|` in choice style). This is the default behavior
    /// starting with ICU 4.8.
    DoubleOptional,
    /// `DOUBLE_REQUIRED`: a single apostrophe always starts quoted literal
    /// text (the behavior of the JDK `MessageFormat`).
    DoubleRequired,
}

impl ApostropheMode {
    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            ApostropheMode::DoubleOptional => "DOUBLE_OPTIONAL",
            ApostropheMode::DoubleRequired => "DOUBLE_REQUIRED",
        }
    }

    /// `ApostropheMode.valueOf(String name)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("No enum constant
    /// com.ibm.icu.text.MessagePattern.ApostropheMode.<name>")`.
    pub fn value_of(name: &str) -> Result<Self> {
        match name {
            "DOUBLE_OPTIONAL" => Ok(ApostropheMode::DoubleOptional),
            "DOUBLE_REQUIRED" => Ok(ApostropheMode::DoubleRequired),
            _ => Err(LuceneError::illegal_argument(format!(
                "No enum constant com.ibm.icu.text.MessagePattern.ApostropheMode.{name}"
            ))),
        }
    }
}

/// The type of a [`Part`].
///
/// Equivalent to the enum `MessagePattern.Part.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartType {
    /// `MSG_START`: start of a message pattern (main or nested).
    MsgStart,
    /// `MSG_LIMIT`: end of a message pattern (main or nested).
    MsgLimit,
    /// `SKIP_SYNTAX`: indicates a substring of the pattern string which is
    /// to be skipped when formatting.
    SkipSyntax,
    /// `INSERT_CHAR`: indicates that a syntax character needs to be
    /// inserted for auto-quoting.
    InsertChar,
    /// `REPLACE_NUMBER`: indicates a syntactic (non-escaped) `#` symbol in
    /// a plural variant.
    ReplaceNumber,
    /// `ARG_START`: start of an argument.
    ArgStart,
    /// `ARG_LIMIT`: end of an argument.
    ArgLimit,
    /// `ARG_NUMBER`: the argument number, provided by the value.
    ArgNumber,
    /// `ARG_NAME`: the argument name.
    ArgName,
    /// `ARG_TYPE`: the argument type identifier.
    ArgType,
    /// `ARG_STYLE`: the argument style text.
    ArgStyle,
    /// `ARG_SELECTOR`: a selector substring in a "complex" argument style.
    ArgSelector,
    /// `ARG_INT`: an integer value, for example the offset or an explicit
    /// selector value in a `PluralFormat` style.
    ArgInt,
    /// `ARG_DOUBLE`: a numeric value, for example the offset or an explicit
    /// selector value in a `PluralFormat` style. The part value is an index
    /// into an internal array of numeric values.
    ArgDouble,
}

impl PartType {
    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            PartType::MsgStart => "MSG_START",
            PartType::MsgLimit => "MSG_LIMIT",
            PartType::SkipSyntax => "SKIP_SYNTAX",
            PartType::InsertChar => "INSERT_CHAR",
            PartType::ReplaceNumber => "REPLACE_NUMBER",
            PartType::ArgStart => "ARG_START",
            PartType::ArgLimit => "ARG_LIMIT",
            PartType::ArgNumber => "ARG_NUMBER",
            PartType::ArgName => "ARG_NAME",
            PartType::ArgType => "ARG_TYPE",
            PartType::ArgStyle => "ARG_STYLE",
            PartType::ArgSelector => "ARG_SELECTOR",
            PartType::ArgInt => "ARG_INT",
            PartType::ArgDouble => "ARG_DOUBLE",
        }
    }

    /// `hasNumericValue()`: whether the part has a numeric value
    /// (`ARG_INT` or `ARG_DOUBLE`).
    pub fn has_numeric_value(self) -> bool {
        self == PartType::ArgInt || self == PartType::ArgDouble
    }
}

/// Argument type constants.
///
/// Equivalent to the enum `MessagePattern.ArgType` (declaration order is
/// the ordinal stored in `ARG_START`/`ARG_LIMIT` part values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArgType {
    /// `NONE`: the argument has no specified type.
    None,
    /// `SIMPLE`: the argument has a "simple" type which is provided by the
    /// `ARG_TYPE` part.
    Simple,
    /// `CHOICE`: the argument is a `ChoiceFormat` with one or more
    /// `((ARG_INT | ARG_DOUBLE), ARG_SELECTOR, message)` tuples.
    Choice,
    /// `PLURAL`: the argument is a cardinal-number `PluralFormat`.
    Plural,
    /// `SELECT`: the argument is a `SelectFormat`.
    Select,
    /// `SELECTORDINAL`: the argument is an ordinal-number `PluralFormat`.
    SelectOrdinal,
}

/// `argTypes`: `ArgType.values()`.
const ARG_TYPES: [ArgType; 6] = [
    ArgType::None,
    ArgType::Simple,
    ArgType::Choice,
    ArgType::Plural,
    ArgType::Select,
    ArgType::SelectOrdinal,
];

impl ArgType {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    /// `name()` (and `toString()`).
    pub fn name(self) -> &'static str {
        match self {
            ArgType::None => "NONE",
            ArgType::Simple => "SIMPLE",
            ArgType::Choice => "CHOICE",
            ArgType::Plural => "PLURAL",
            ArgType::Select => "SELECT",
            ArgType::SelectOrdinal => "SELECTORDINAL",
        }
    }

    /// `hasPluralStyle()`: whether the type is `PLURAL` or
    /// `SELECTORDINAL`.
    pub fn has_plural_style(self) -> bool {
        self == ArgType::Plural || self == ArgType::SelectOrdinal
    }

    /// `toString().toLowerCase(Locale.ENGLISH)`.
    fn lower_name(self) -> String {
        self.name().to_ascii_lowercase()
    }
}

/// A message pattern "part", representing a pattern parsing event.
///
/// Equivalent to `MessagePattern.Part` (Java's `char length` is `u16`,
/// `short value` is `i16`); `equals` is the derived `PartialEq` over all
/// fields, including `limitPartIndex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    part_type: PartType,
    index: i32,
    length: u16,
    value: i16,
    limit_part_index: i32,
}

const MAX_LENGTH: i32 = 0xffff;
const MAX_VALUE: i32 = i16::MAX as i32;

impl Part {
    fn new(t: PartType, i: i32, l: i32, v: i32) -> Self {
        Self {
            part_type: t,
            index: i,
            length: l as u16,
            value: v as i16,
            limit_part_index: 0,
        }
    }

    /// `getType()`.
    pub fn get_type(&self) -> PartType {
        self.part_type
    }

    /// `getIndex()`: the pattern string index associated with this part.
    pub fn get_index(&self) -> i32 {
        self.index
    }

    /// `getLength()`: the length of the pattern substring associated with
    /// this part.
    pub fn get_length(&self) -> i32 {
        self.length as i32
    }

    /// `getLimit()`: `getIndex() + getLength()`.
    pub fn get_limit(&self) -> i32 {
        self.index + self.length as i32
    }

    /// `getValue()`: a value associated with this part.
    pub fn get_value(&self) -> i32 {
        self.value as i32
    }

    /// `getArgType()`: the argument type if this part is `ARG_START` or
    /// `ARG_LIMIT`, otherwise `NONE`.
    pub fn get_arg_type(&self) -> ArgType {
        let t = self.get_type();
        if t == PartType::ArgStart || t == PartType::ArgLimit {
            ARG_TYPES[self.value as usize]
        } else {
            ArgType::None
        }
    }

    /// `hashCode()`: `((type.hashCode()*37+index)*37+length)*37+value`;
    /// the enum's identity hash is
    /// [`java_identity_hash_code_stand_in`] of
    /// `com.ibm.icu.text.MessagePattern$Part$Type.<NAME>`.
    pub fn hash_code(&self) -> i32 {
        let t = java_identity_hash_code_stand_in(&format!(
            "com.ibm.icu.text.MessagePattern$Part$Type.{}",
            self.part_type.name()
        ));
        ((t.wrapping_mul(37).wrapping_add(self.index))
            .wrapping_mul(37)
            .wrapping_add(self.length as i32))
        .wrapping_mul(37)
        .wrapping_add(self.value as i32)
    }
}

impl fmt::Display for Part {
    /// `toString()`: `TYPE(value)@index`, the value being the argument
    /// type name for `ARG_START`/`ARG_LIMIT`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value_string =
            if self.part_type == PartType::ArgStart || self.part_type == PartType::ArgLimit {
                self.get_arg_type().name().to_string()
            } else {
                self.value.to_string()
            };
        write!(
            f,
            "{}({})@{}",
            self.part_type.name(),
            value_string,
            self.index
        )
    }
}

/// `defaultAposMode`: from the ICU configuration property
/// `com.ibm.icu.text.MessagePattern.ApostropheMode` (default
/// `DOUBLE_OPTIONAL`); a bad value fails the class initialization (Java's
/// `ExceptionInInitializerError`, a panic here, crate precedent).
static DEFAULT_APOS_MODE: LazyLock<ApostropheMode> = LazyLock::new(|| {
    let name = ICUConfig::get_or(
        "com.ibm.icu.text.MessagePattern.ApostropheMode",
        Some("DOUBLE_OPTIONAL"),
    )
    .unwrap_or_default();
    ApostropheMode::value_of(&name)
        .unwrap_or_else(|e| panic!("java.lang.ExceptionInInitializerError: {e}"))
});

/// `ARG_NAME_NOT_NUMBER`: [`MessagePattern::validate_argument_name`]
/// result for an argument name that is a valid pattern identifier but not
/// a number.
pub const ARG_NAME_NOT_NUMBER: i32 = -1;

/// `ARG_NAME_NOT_VALID`: [`MessagePattern::validate_argument_name`] result
/// for an argument name that is neither an identifier nor a number.
pub const ARG_NAME_NOT_VALID: i32 = -2;

/// `NO_NUMERIC_VALUE`: special value returned by
/// [`MessagePattern::get_numeric_value`] for a part without a numeric
/// value.
pub const NO_NUMERIC_VALUE: f64 = -123456789.0;

const MAX_PREFIX_LENGTH: i32 = 24;

/// Parses and represents ICU `MessageFormat` patterns, and the
/// sub-patterns of `PluralFormat`, `SelectFormat` and `ChoiceFormat`, as a
/// list of [`Part`]s.
///
/// Equivalent to `com.ibm.icu.text.MessagePattern`. The pattern is UTF-16
/// (`msg`, Java's `null` before a parse or after `clear()` is `None`).
/// `IllegalArgumentException`, `NumberFormatException` and
/// `UnsupportedOperationException` are `Err`; the
/// `IndexOutOfBoundsException`s ICU throws for oversized patterns are
/// panics (crate convention).
///
/// **Divergence.** Java throws `NullPointerException` when a method that
/// reads the pattern runs on a cleared object; the port panics with that
/// text.
#[derive(Debug, Clone)]
pub struct MessagePattern {
    apos_mode: ApostropheMode,
    msg: Option<Vec<u16>>,
    parts: Vec<Part>,
    numeric_values: Option<Vec<f64>>,
    has_arg_names: bool,
    has_arg_numbers: bool,
    needs_auto_quoting: bool,
    frozen: bool,
}

impl PartialEq for MessagePattern {
    /// `equals(Object other)`: same apostrophe mode, pattern and parts.
    fn eq(&self, o: &Self) -> bool {
        self.apos_mode == o.apos_mode && self.msg == o.msg && self.parts == o.parts
    }
}

impl Default for MessagePattern {
    fn default() -> Self {
        Self::new()
    }
}

impl MessagePattern {
    /// `new MessagePattern()`: the default apostrophe mode.
    pub fn new() -> Self {
        Self::with_mode(*DEFAULT_APOS_MODE)
    }

    /// `new MessagePattern(ApostropheMode mode)`.
    pub fn with_mode(mode: ApostropheMode) -> Self {
        Self {
            apos_mode: mode,
            msg: None,
            parts: Vec::new(),
            numeric_values: None,
            has_arg_names: false,
            has_arg_numbers: false,
            needs_auto_quoting: false,
            frozen: false,
        }
    }

    /// `new MessagePattern(String pattern)`: parses a `MessageFormat`
    /// pattern with the default apostrophe mode.
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse).
    pub fn from_pattern(pattern: &[u16]) -> Result<Self> {
        let mut m = Self::new();
        m.parse(pattern)?;
        Ok(m)
    }

    /// `parse(String pattern)`: parses a `MessageFormat` pattern string.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a syntax error, `NumberFormatException`
    /// for a bad number, `UnsupportedOperationException` when frozen.
    pub fn parse(&mut self, pattern: &[u16]) -> Result<&mut Self> {
        self.pre_parse(pattern)?;
        self.parse_message(0, 0, 0, ArgType::None)?;
        self.post_parse();
        Ok(self)
    }

    /// `parseChoiceStyle(String pattern)`: parses a `ChoiceFormat` pattern
    /// string.
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse).
    pub fn parse_choice_style(&mut self, pattern: &[u16]) -> Result<&mut Self> {
        self.pre_parse(pattern)?;
        self.parse_choice_style_at(0, 0)?;
        self.post_parse();
        Ok(self)
    }

    /// `parsePluralStyle(String pattern)`: parses a `PluralFormat` pattern
    /// string.
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse).
    pub fn parse_plural_style(&mut self, pattern: &[u16]) -> Result<&mut Self> {
        self.pre_parse(pattern)?;
        self.parse_plural_or_select_style(ArgType::Plural, 0, 0)?;
        self.post_parse();
        Ok(self)
    }

    /// `parseSelectStyle(String pattern)`: parses a `SelectFormat` pattern
    /// string.
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse).
    pub fn parse_select_style(&mut self, pattern: &[u16]) -> Result<&mut Self> {
        self.pre_parse(pattern)?;
        self.parse_plural_or_select_style(ArgType::Select, 0, 0)?;
        self.post_parse();
        Ok(self)
    }

    /// `clear()`: clears this object, so that it represents no pattern.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException("Attempt to clear() a frozen
    /// MessagePattern instance.")`.
    pub fn clear(&mut self) -> Result<()> {
        // Mostly the same as preParse().
        if self.is_frozen() {
            return Err(LuceneError::unsupported_operation(
                "Attempt to clear() a frozen MessagePattern instance.",
            ));
        }
        self.msg = None;
        self.has_arg_names = false;
        self.has_arg_numbers = false;
        self.needs_auto_quoting = false;
        self.parts.clear();
        if let Some(n) = self.numeric_values.as_mut() {
            n.clear();
        }
        Ok(())
    }

    /// `clearPatternAndSetApostropheMode(ApostropheMode mode)`.
    ///
    /// # Errors
    ///
    /// As [`clear`](Self::clear).
    pub fn clear_pattern_and_set_apostrophe_mode(&mut self, mode: ApostropheMode) -> Result<()> {
        self.clear()?;
        self.apos_mode = mode;
        Ok(())
    }

    /// `hashCode()`: `(aposMode.hashCode()*37 + msg.hashCode())*37 +
    /// parts.hashCode()`; enum identity hashes are
    /// [`java_identity_hash_code_stand_in`] values.
    pub fn hash_code(&self) -> i32 {
        let apos = java_identity_hash_code_stand_in(&format!(
            "com.ibm.icu.text.MessagePattern$ApostropheMode.{}",
            self.apos_mode.name()
        ));
        let msg_hash = self.msg.as_ref().map_or(0, |m| {
            m.iter()
                .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(c as i32))
        });
        let parts_hash = self
            .parts
            .iter()
            .fold(1i32, |h, p| h.wrapping_mul(31).wrapping_add(p.hash_code()));
        (apos.wrapping_mul(37).wrapping_add(msg_hash))
            .wrapping_mul(37)
            .wrapping_add(parts_hash)
    }

    /// `getApostropheMode()`.
    pub fn get_apostrophe_mode(&self) -> ApostropheMode {
        self.apos_mode
    }

    /// `jdkAposMode()` (package-private): whether the mode is
    /// `DOUBLE_REQUIRED`.
    pub fn jdk_apos_mode(&self) -> bool {
        self.apos_mode == ApostropheMode::DoubleRequired
    }

    /// `getPatternString()`: the parsed pattern string, `None` if none was
    /// parsed.
    pub fn get_pattern_string(&self) -> Option<&[u16]> {
        self.msg.as_deref()
    }

    /// `hasNamedArguments()`.
    pub fn has_named_arguments(&self) -> bool {
        self.has_arg_names
    }

    /// `hasNumberedArguments()`.
    pub fn has_numbered_arguments(&self) -> bool {
        self.has_arg_numbers
    }

    /// `validateArgumentName(String name)`: the argument number (>= 0) if
    /// `name` is a number, [`ARG_NAME_NOT_NUMBER`] if it is another
    /// identifier, [`ARG_NAME_NOT_VALID`] otherwise.
    pub fn validate_argument_name(name: &[u16]) -> i32 {
        if !PatternProps::is_identifier(name) {
            return ARG_NAME_NOT_VALID;
        }
        Self::parse_arg_number_of(name, 0, name.len() as i32)
    }

    /// `autoQuoteApostropheDeep()`: the pattern with the apostrophes
    /// auto-quoting inserted, recursively in all sub-messages.
    pub fn auto_quote_apostrophe_deep(&self) -> Option<Vec<u16>> {
        if !self.needs_auto_quoting {
            return self.msg.clone();
        }
        let mut modified: Option<Vec<u16>> = None;
        // Iterate backward so that the insertion indexes do not change.
        let count = self.count_parts();
        let mut i = count;
        while i > 0 {
            i -= 1;
            let part = self.get_part(i);
            if part.get_type() == PartType::InsertChar {
                let m = modified.get_or_insert_with(|| self.m().to_vec());
                m.insert(part.index as usize, part.value as u16);
            }
        }
        match modified {
            None => self.msg.clone(),
            Some(m) => Some(m),
        }
    }

    /// `countParts()`.
    pub fn count_parts(&self) -> i32 {
        self.parts.len() as i32
    }

    /// `getPart(int i)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for `i` outside the parts.
    pub fn get_part(&self, i: i32) -> Part {
        self.parts[i as usize]
    }

    /// `getPartType(int i)`.
    pub fn get_part_type(&self, i: i32) -> PartType {
        self.parts[i as usize].part_type
    }

    /// `getPatternIndex(int partIndex)`.
    pub fn get_pattern_index(&self, part_index: i32) -> i32 {
        self.parts[part_index as usize].index
    }

    /// `getSubstring(Part part)`: the substring of the pattern string
    /// indicated by the part.
    pub fn get_substring(&self, part: &Part) -> Vec<u16> {
        let index = part.index as usize;
        self.m()[index..index + part.length as usize].to_vec()
    }

    /// `partSubstringMatches(Part part, String s)`.
    pub fn part_substring_matches(&self, part: &Part, s: &[u16]) -> bool {
        part.length as usize == s.len()
            && region_matches(self.m(), part.index, s, 0, part.length as i32)
    }

    /// `getNumericValue(Part part)`: the part's numeric value, or
    /// [`NO_NUMERIC_VALUE`].
    pub fn get_numeric_value(&self, part: &Part) -> f64 {
        let t = part.part_type;
        if t == PartType::ArgInt {
            part.value as f64
        } else if t == PartType::ArgDouble {
            self.numeric_values
                .as_ref()
                .map_or(0.0, |n| n[part.value as usize])
        } else {
            NO_NUMERIC_VALUE
        }
    }

    /// `getPluralOffset(int pluralStart)`: the "offset:" value of a
    /// `PluralFormat` argument, or 0 if none is specified.
    pub fn get_plural_offset(&self, plural_start: i32) -> f64 {
        let part = self.parts[plural_start as usize];
        if part.part_type.has_numeric_value() {
            self.get_numeric_value(&part)
        } else {
            0.0
        }
    }

    /// `getLimitPartIndex(int start)`: the index of the matching
    /// `ARG_LIMIT`/`MSG_LIMIT` part.
    pub fn get_limit_part_index(&self, start: i32) -> i32 {
        let limit = self.parts[start as usize].limit_part_index;
        if limit < start {
            return start;
        }
        limit
    }

    /// The pattern (`msg`), Java's `NullPointerException` when there is
    /// none.
    fn m(&self) -> &[u16] {
        match &self.msg {
            Some(m) => m,
            None => panic!("java.lang.NullPointerException"),
        }
    }

    fn pre_parse(&mut self, pattern: &[u16]) -> Result<()> {
        if self.is_frozen() {
            return Err(LuceneError::unsupported_operation(format!(
                "Attempt to parse({}) on frozen MessagePattern instance.",
                prefix_of(pattern, 0)
            )));
        }
        self.msg = Some(pattern.to_vec());
        self.has_arg_names = false;
        self.has_arg_numbers = false;
        self.needs_auto_quoting = false;
        self.parts.clear();
        if let Some(n) = self.numeric_values.as_mut() {
            n.clear();
        }
        Ok(())
    }

    fn post_parse(&mut self) {
        // Nothing to be done currently.
    }

    fn len(&self) -> i32 {
        self.m().len() as i32
    }

    fn char_at(&self, i: i32) -> u16 {
        match self.m().get(i as usize) {
            Some(&c) if i >= 0 => c,
            _ => crate::jdk::string_index_out_of_bounds(i),
        }
    }

    fn parse_message(
        &mut self,
        mut index: i32,
        msg_start_length: i32,
        nesting_level: i32,
        parent_type: ArgType,
    ) -> Result<i32> {
        if nesting_level > MAX_VALUE {
            crate::jdk::index_out_of_bounds(None);
        }
        let msg_start = self.parts.len() as i32;
        self.add_part(PartType::MsgStart, index, msg_start_length, nesting_level);
        index += msg_start_length;
        while index < self.len() {
            let mut c = self.char_at(index);
            index += 1;
            if c == b'\'' as u16 {
                if index == self.len() {
                    // The apostrophe is the last character in the pattern.
                    // Add a Part for auto-quoting.
                    self.add_part(PartType::InsertChar, index, 0, b'\'' as i32); // value=char to be inserted
                    self.needs_auto_quoting = true;
                } else {
                    c = self.char_at(index);
                    if c == b'\'' as u16 {
                        // double apostrophe, skip the second one
                        self.add_part(PartType::SkipSyntax, index, 1, 0);
                        index += 1;
                    } else if self.apos_mode == ApostropheMode::DoubleRequired
                        || c == b'{' as u16
                        || c == b'}' as u16
                        || (parent_type == ArgType::Choice && c == b'|' as u16)
                        || (parent_type.has_plural_style() && c == b'#' as u16)
                    {
                        // skip the quote-starting apostrophe
                        self.add_part(PartType::SkipSyntax, index - 1, 1, 0);
                        // find the end of the quoted literal text
                        loop {
                            index = index_of_unit(self.m(), b'\'' as u16, index + 1);
                            if index >= 0 {
                                if (index + 1) < self.len()
                                    && self.char_at(index + 1) == b'\'' as u16
                                {
                                    // double apostrophe inside quoted literal text
                                    // still encodes a single apostrophe, skip the second one
                                    index += 1;
                                    self.add_part(PartType::SkipSyntax, index, 1, 0);
                                } else {
                                    // skip the quote-ending apostrophe
                                    self.add_part(PartType::SkipSyntax, index, 1, 0);
                                    index += 1;
                                    break;
                                }
                            } else {
                                // The quoted text reaches to the end of the of the message.
                                index = self.len();
                                // Add a Part for auto-quoting.
                                self.add_part(PartType::InsertChar, index, 0, b'\'' as i32); // value=char to be inserted
                                self.needs_auto_quoting = true;
                                break;
                            }
                        }
                    } else {
                        // Interpret the apostrophe as literal text.
                        // Add a Part for auto-quoting.
                        self.add_part(PartType::InsertChar, index, 0, b'\'' as i32); // value=char to be inserted
                        self.needs_auto_quoting = true;
                    }
                }
            } else if parent_type.has_plural_style() && c == b'#' as u16 {
                // The unquoted # in a plural message fragment will be replaced
                // with the (number-offset).
                self.add_part(PartType::ReplaceNumber, index - 1, 1, 0);
            } else if c == b'{' as u16 {
                index = self.parse_arg(index - 1, 1, nesting_level)?;
            } else if (nesting_level > 0 && c == b'}' as u16)
                || (parent_type == ArgType::Choice && c == b'|' as u16)
            {
                // Finish the message before the terminator.
                // In a choice style, report the "}" substring only for the following ARG_LIMIT,
                // not for this MSG_LIMIT.
                let limit_length = if parent_type == ArgType::Choice && c == b'}' as u16 {
                    0
                } else {
                    1
                };
                self.add_limit_part(
                    msg_start,
                    PartType::MsgLimit,
                    index - 1,
                    limit_length,
                    nesting_level,
                );
                if parent_type == ArgType::Choice {
                    // Let the choice style parser see the '}' or '|'.
                    return Ok(index - 1);
                } else {
                    // continue parsing after the '}'
                    return Ok(index);
                }
            } // else: c is part of literal text
        }
        if nesting_level > 0 && !self.in_top_level_choice_message(nesting_level, parent_type) {
            return Err(LuceneError::illegal_argument(format!(
                "Unmatched '{{' braces in message {}",
                self.prefix()
            )));
        }
        self.add_limit_part(msg_start, PartType::MsgLimit, index, 0, nesting_level);
        Ok(index)
    }

    fn parse_arg(
        &mut self,
        mut index: i32,
        arg_start_length: i32,
        nesting_level: i32,
    ) -> Result<i32> {
        let arg_start = self.parts.len() as i32;
        let mut arg_type = ArgType::None;
        self.add_part(
            PartType::ArgStart,
            index,
            arg_start_length,
            arg_type.ordinal(),
        );
        index = self.skip_white_space(index + arg_start_length);
        let name_index = index;
        if index == self.len() {
            return Err(LuceneError::illegal_argument(format!(
                "Unmatched '{{' braces in message {}",
                self.prefix()
            )));
        }
        // parse argument name or number
        index = self.skip_identifier(index);
        let number = self.parse_arg_number(name_index, index);
        if number >= 0 {
            let length = index - name_index;
            if length > MAX_LENGTH || number > MAX_VALUE {
                crate::jdk::index_out_of_bounds(Some(format!(
                    "Argument number too large: {}",
                    self.prefix_at(name_index)
                )));
            }
            self.has_arg_numbers = true;
            self.add_part(PartType::ArgNumber, name_index, length, number);
        } else if number == ARG_NAME_NOT_NUMBER {
            let length = index - name_index;
            if length > MAX_LENGTH {
                crate::jdk::index_out_of_bounds(Some(format!(
                    "Argument name too long: {}",
                    self.prefix_at(name_index)
                )));
            }
            self.has_arg_names = true;
            self.add_part(PartType::ArgName, name_index, length, 0);
        } else {
            // number<-1 (ARG_NAME_NOT_VALID)
            return Err(LuceneError::illegal_argument(format!(
                "Bad argument syntax: {}",
                self.prefix_at(name_index)
            )));
        }
        index = self.skip_white_space(index);
        if index == self.len() {
            return Err(LuceneError::illegal_argument(format!(
                "Unmatched '{{' braces in message {}",
                self.prefix()
            )));
        }
        let mut c = self.char_at(index);
        if c == b'}' as u16 {
            // all done
        } else if c != b',' as u16 {
            return Err(LuceneError::illegal_argument(format!(
                "Bad argument syntax: {}",
                self.prefix_at(name_index)
            )));
        } else
        /* ',' */
        {
            // parse argument type: case-sensitive a-zA-Z
            index = self.skip_white_space(index + 1);
            let type_index = index;
            while index < self.len() && is_arg_type_char(self.char_at(index) as i32) {
                index += 1;
            }
            let length = index - type_index;
            index = self.skip_white_space(index);
            if index == self.len() {
                return Err(LuceneError::illegal_argument(format!(
                    "Unmatched '{{' braces in message {}",
                    self.prefix()
                )));
            }
            if length == 0 || {
                c = self.char_at(index);
                c != b',' as u16 && c != b'}' as u16
            } {
                return Err(LuceneError::illegal_argument(format!(
                    "Bad argument syntax: {}",
                    self.prefix_at(name_index)
                )));
            }
            if length > MAX_LENGTH {
                crate::jdk::index_out_of_bounds(Some(format!(
                    "Argument type name too long: {}",
                    self.prefix_at(name_index)
                )));
            }
            arg_type = ArgType::Simple;
            if length == 6 {
                // case-insensitive comparisons for complex-type names
                if self.is_choice(type_index) {
                    arg_type = ArgType::Choice;
                } else if self.is_plural(type_index) {
                    arg_type = ArgType::Plural;
                } else if self.is_select(type_index) {
                    arg_type = ArgType::Select;
                }
            } else if length == 13 && self.is_select(type_index) && self.is_ordinal(type_index + 6)
            {
                arg_type = ArgType::SelectOrdinal;
            }
            // change the ARG_START type from NONE to argType
            self.parts[arg_start as usize].value = arg_type.ordinal() as i16;
            if arg_type == ArgType::Simple {
                self.add_part(PartType::ArgType, type_index, length, 0);
            }
            // look for an argument style (pattern)
            if c == b'}' as u16 {
                if arg_type != ArgType::Simple {
                    return Err(LuceneError::illegal_argument(format!(
                        "No style field for complex argument: {}",
                        self.prefix_at(name_index)
                    )));
                }
            } else
            /* ',' */
            {
                index += 1;
                if arg_type == ArgType::Simple {
                    index = self.parse_simple_style(index)?;
                } else if arg_type == ArgType::Choice {
                    index = self.parse_choice_style_at(index, nesting_level)?;
                } else {
                    index = self.parse_plural_or_select_style(arg_type, index, nesting_level)?;
                }
            }
        }
        // Argument parsing stopped on the '}'.
        self.add_limit_part(arg_start, PartType::ArgLimit, index, 1, arg_type.ordinal());
        Ok(index + 1)
    }

    fn parse_simple_style(&mut self, mut index: i32) -> Result<i32> {
        let start = index;
        let mut nested_braces = 0;
        while index < self.len() {
            let c = self.char_at(index);
            index += 1;
            if c == b'\'' as u16 {
                // Treat apostrophe as quoting but include it in the style part.
                // Find the end of the quoted literal text.
                index = index_of_unit(self.m(), b'\'' as u16, index);
                if index < 0 {
                    return Err(LuceneError::illegal_argument(format!(
                        "Quoted literal argument style text reaches to the end of the message: {}",
                        self.prefix_at(start)
                    )));
                }
                // skip the quote-ending apostrophe
                index += 1;
            } else if c == b'{' as u16 {
                nested_braces += 1;
            } else if c == b'}' as u16 {
                if nested_braces > 0 {
                    nested_braces -= 1;
                } else {
                    index -= 1;
                    let length = index - start;
                    if length > MAX_LENGTH {
                        crate::jdk::index_out_of_bounds(Some(format!(
                            "Argument style text too long: {}",
                            self.prefix_at(start)
                        )));
                    }
                    self.add_part(PartType::ArgStyle, start, length, 0);
                    return Ok(index);
                }
            } // c is part of literal text
        }
        Err(LuceneError::illegal_argument(format!(
            "Unmatched '{{' braces in message {}",
            self.prefix()
        )))
    }

    fn parse_choice_style_at(&mut self, mut index: i32, nesting_level: i32) -> Result<i32> {
        let start = index;
        index = self.skip_white_space(index);
        if index == self.len() || self.char_at(index) == b'}' as u16 {
            return Err(LuceneError::illegal_argument(format!(
                "Missing choice argument pattern in {}",
                self.prefix()
            )));
        }
        loop {
            // The choice argument style contains |-separated (number, separator, message) triples.
            // Parse the number.
            let number_index = index;
            index = self.skip_double(index);
            let length = index - number_index;
            if length == 0 {
                return Err(LuceneError::illegal_argument(format!(
                    "Bad choice pattern syntax: {}",
                    self.prefix_at(start)
                )));
            }
            if length > MAX_LENGTH {
                crate::jdk::index_out_of_bounds(Some(format!(
                    "Choice number too long: {}",
                    self.prefix_at(number_index)
                )));
            }
            self.parse_double(number_index, index, true)?; // adds ARG_INT or ARG_DOUBLE
                                                           // Parse the separator.
            index = self.skip_white_space(index);
            if index == self.len() {
                return Err(LuceneError::illegal_argument(format!(
                    "Bad choice pattern syntax: {}",
                    self.prefix_at(start)
                )));
            }
            let c = self.char_at(index);
            if !(c == b'#' as u16 || c == b'<' as u16 || c == 0x2264) {
                // U+2264 is <=
                return Err(LuceneError::illegal_argument(format!(
                    "Expected choice separator (#<\u{2264}) instead of '{}' in choice pattern {}",
                    to_rust_string(&[c]),
                    self.prefix_at(start)
                )));
            }
            self.add_part(PartType::ArgSelector, index, 1, 0);
            // Parse the message fragment.
            index += 1;
            index = self.parse_message(index, 0, nesting_level + 1, ArgType::Choice)?;
            // parseMessage(..., CHOICE) returns the index of the terminator, or msg.length().
            if index == self.len() {
                return Ok(index);
            }
            if self.char_at(index) == b'}' as u16 {
                if !self.in_message_format_pattern(nesting_level) {
                    return Err(LuceneError::illegal_argument(format!(
                        "Bad choice pattern syntax: {}",
                        self.prefix_at(start)
                    )));
                }
                return Ok(index);
            } // else the terminator is '|'
            index = self.skip_white_space(index + 1);
        }
    }

    fn parse_plural_or_select_style(
        &mut self,
        arg_type: ArgType,
        mut index: i32,
        nesting_level: i32,
    ) -> Result<i32> {
        let start = index;
        let mut is_empty = true;
        let mut has_other = false;
        loop {
            // First, collect the selector looking for a small set of terminators.
            // It would be a little faster to consider the syntax of each possible
            // token right here, but that makes the code too complicated.
            index = self.skip_white_space(index);
            let eos = index == self.len();
            if eos || self.char_at(index) == b'}' as u16 {
                if eos == self.in_message_format_pattern(nesting_level) {
                    return Err(LuceneError::illegal_argument(format!(
                        "Bad {} pattern syntax: {}",
                        arg_type.lower_name(),
                        self.prefix_at(start)
                    )));
                }
                if !has_other {
                    return Err(LuceneError::illegal_argument(format!(
                        "Missing 'other' keyword in {} pattern in {}",
                        arg_type.lower_name(),
                        self.prefix()
                    )));
                }
                return Ok(index);
            }
            let selector_index = index;
            if arg_type.has_plural_style() && self.char_at(selector_index) == b'=' as u16 {
                // explicit-value plural selector: =double
                index = self.skip_double(index + 1);
                let length = index - selector_index;
                if length == 1 {
                    return Err(LuceneError::illegal_argument(format!(
                        "Bad {} pattern syntax: {}",
                        arg_type.lower_name(),
                        self.prefix_at(start)
                    )));
                }
                if length > MAX_LENGTH {
                    crate::jdk::index_out_of_bounds(Some(format!(
                        "Argument selector too long: {}",
                        self.prefix_at(selector_index)
                    )));
                }
                self.add_part(PartType::ArgSelector, selector_index, length, 0);
                self.parse_double(selector_index + 1, index, false)?; // adds ARG_INT or ARG_DOUBLE
            } else {
                index = self.skip_identifier(index);
                let length = index - selector_index;
                if length == 0 {
                    return Err(LuceneError::illegal_argument(format!(
                        "Bad {} pattern syntax: {}",
                        arg_type.lower_name(),
                        self.prefix_at(start)
                    )));
                }
                // Note: The ':' in "offset:" is just beyond the skipIdentifier() range.
                if arg_type.has_plural_style()
                    && length == 6
                    && index < self.len()
                    && region_matches(
                        self.m(),
                        selector_index,
                        &crate::jdk::units("offset:"),
                        0,
                        7,
                    )
                {
                    // plural offset, not a selector
                    if !is_empty {
                        return Err(LuceneError::illegal_argument(format!(
                            "Plural argument 'offset:' (if present) must precede key-message pairs: {}",
                            self.prefix_at(start)
                        )));
                    }
                    // allow whitespace between offset: and its value
                    let value_index = self.skip_white_space(index + 1); // The ':' is at index.
                    index = self.skip_double(value_index);
                    if index == value_index {
                        return Err(LuceneError::illegal_argument(format!(
                            "Missing value for plural 'offset:' {}",
                            self.prefix_at(start)
                        )));
                    }
                    if (index - value_index) > MAX_LENGTH {
                        crate::jdk::index_out_of_bounds(Some(format!(
                            "Plural offset value too long: {}",
                            self.prefix_at(value_index)
                        )));
                    }
                    self.parse_double(value_index, index, false)?; // adds ARG_INT or ARG_DOUBLE
                    is_empty = false;
                    continue; // no message fragment after the offset
                } else {
                    // normal selector word
                    if length > MAX_LENGTH {
                        crate::jdk::index_out_of_bounds(Some(format!(
                            "Argument selector too long: {}",
                            self.prefix_at(selector_index)
                        )));
                    }
                    self.add_part(PartType::ArgSelector, selector_index, length, 0);
                    if region_matches(
                        self.m(),
                        selector_index,
                        &crate::jdk::units("other"),
                        0,
                        length,
                    ) {
                        has_other = true;
                    }
                }
            }

            // parse the message fragment following the selector
            index = self.skip_white_space(index);
            if index == self.len() || self.char_at(index) != b'{' as u16 {
                return Err(LuceneError::illegal_argument(format!(
                    "No message fragment after {} selector: {}",
                    arg_type.lower_name(),
                    self.prefix_at(selector_index)
                )));
            }
            index = self.parse_message(index, 1, nesting_level + 1, arg_type)?;
            is_empty = false;
        }
    }

    /// `parseArgNumber(CharSequence s, int start, int limit)`: the
    /// non-negative argument number, [`ARG_NAME_NOT_NUMBER`] or
    /// [`ARG_NAME_NOT_VALID`].
    fn parse_arg_number_of(s: &[u16], mut start: i32, limit: i32) -> i32 {
        // If the identifier contains only ASCII digits, then it is an argument _number_
        // and must not have leading zeros (except "0" itself).
        // Otherwise it is an argument _name_.
        if start >= limit {
            return ARG_NAME_NOT_VALID;
        }
        let mut number: i32;
        // Defer numeric errors until we know there are only digits.
        let mut bad_number;
        let mut c = s[start as usize];
        start += 1;
        if c == b'0' as u16 {
            if start == limit {
                return 0;
            } else {
                number = 0;
                bad_number = true; // leading zero
            }
        } else if (b'1' as u16..=b'9' as u16).contains(&c) {
            number = (c - b'0' as u16) as i32;
            bad_number = false;
        } else {
            return ARG_NAME_NOT_NUMBER;
        }
        while start < limit {
            c = s[start as usize];
            start += 1;
            if (b'0' as u16..=b'9' as u16).contains(&c) {
                if number >= i32::MAX / 10 {
                    bad_number = true; // overflow
                }
                number = number
                    .wrapping_mul(10)
                    .wrapping_add((c - b'0' as u16) as i32);
            } else {
                return ARG_NAME_NOT_NUMBER;
            }
        }
        // There are only ASCII digits.
        if bad_number {
            ARG_NAME_NOT_VALID
        } else {
            number
        }
    }

    fn parse_arg_number(&self, start: i32, limit: i32) -> i32 {
        Self::parse_arg_number_of(self.m(), start, limit)
    }

    /// `parseDouble(int start, int limit, boolean allowInfinity)`: adds an
    /// `ARG_INT` or `ARG_DOUBLE` part for the number at `start..limit`.
    fn parse_double(&mut self, start: i32, limit: i32, allow_infinity: bool) -> Result<()> {
        debug_assert!(start < limit);
        // fake loop for easy exit and single throw statement
        'bad: {
            // fast path for small integers and infinity
            let mut value: i32 = 0;
            let mut is_negative = 0; // not boolean so that we can easily add it to value
            let mut index = start;
            let mut c = self.char_at(index);
            index += 1;
            if c == b'-' as u16 {
                is_negative = 1;
                if index == limit {
                    break 'bad; // no number
                }
                c = self.char_at(index);
                index += 1;
            } else if c == b'+' as u16 {
                if index == limit {
                    break 'bad; // no number
                }
                c = self.char_at(index);
                index += 1;
            }
            if c == 0x221e {
                // infinity
                if allow_infinity && index == limit {
                    self.add_arg_double_part(
                        if is_negative != 0 {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        },
                        start,
                        limit - start,
                    );
                    return Ok(());
                } else {
                    break 'bad;
                }
            }
            // try to parse the number as a small integer but fall back to a double
            while (b'0' as u16..=b'9' as u16).contains(&c) {
                value = value * 10 + (c - b'0' as u16) as i32;
                if value > (MAX_VALUE + is_negative) {
                    break; // not a small-enough integer
                }
                if index == limit {
                    self.add_part(
                        PartType::ArgInt,
                        start,
                        limit - start,
                        if is_negative != 0 { -value } else { value },
                    );
                    return Ok(());
                }
                c = self.char_at(index);
                index += 1;
            }
            // Let Double.parseDouble() throw a NumberFormatException.
            let text = to_rust_string(&self.m()[start as usize..limit as usize]);
            let numeric_value = crate::jdk::parse_double(&text)?;
            self.add_arg_double_part(numeric_value, start, limit - start);
            return Ok(());
        }
        Err(LuceneError::number_format(format!(
            "Bad syntax for numeric value: {}",
            to_rust_string(&self.m()[start as usize..limit as usize])
        )))
    }

    /// `appendReducedApostrophes(String s, int start, int limit,
    /// StringBuilder sb)` (package-private): appends `s[start, limit)`
    /// with apostrophe pairs reduced to single ones.
    pub fn append_reduced_apostrophes(s: &[u16], mut start: i32, limit: i32, sb: &mut Vec<u16>) {
        let mut double_apos = -1;
        loop {
            let i = index_of_unit(s, b'\'' as u16, start);
            if i < 0 || i >= limit {
                sb.extend_from_slice(&s[start as usize..limit as usize]);
                break;
            }
            if i == double_apos {
                // Double apostrophe at start-1 and start==i, append one.
                sb.push(b'\'' as u16);
                start += 1;
                double_apos = -1;
            } else {
                // Append text between apostrophes and skip this one.
                sb.extend_from_slice(&s[start as usize..i as usize]);
                start = i + 1;
                double_apos = start;
            }
        }
    }

    fn skip_white_space(&self, index: i32) -> i32 {
        PatternProps::skip_white_space(self.m(), index)
    }

    fn skip_identifier(&self, index: i32) -> i32 {
        PatternProps::skip_identifier(self.m(), index)
    }

    /// `skipDouble(int index)`: skips a sequence of characters that could
    /// occur in a double value.
    fn skip_double(&self, mut index: i32) -> i32 {
        while index < self.len() {
            let c = self.char_at(index);
            // U+221E: Allow the infinity symbol, for ChoiceFormat patterns.
            if (c < b'0' as u16 && !(c == b'+' as u16 || c == b'-' as u16 || c == b'.' as u16))
                || (c > b'9' as u16 && c != b'e' as u16 && c != b'E' as u16 && c != 0x221e)
            {
                break;
            }
            index += 1;
        }
        index
    }

    fn matches_ci(&self, index: i32, word: &str) -> bool {
        word.bytes().enumerate().all(|(k, w)| {
            let c = self.char_at(index + k as i32);
            c == w as u16 || c == w.to_ascii_uppercase() as u16
        })
    }

    fn is_choice(&self, index: i32) -> bool {
        self.matches_ci(index, "choice")
    }

    fn is_plural(&self, index: i32) -> bool {
        self.matches_ci(index, "plural")
    }

    fn is_select(&self, index: i32) -> bool {
        self.matches_ci(index, "select")
    }

    fn is_ordinal(&self, index: i32) -> bool {
        self.matches_ci(index, "ordinal")
    }

    /// `inMessageFormatPattern(int nestingLevel)`: whether we are inside a
    /// `MessageFormat` (sub-)pattern, as opposed to "top-level"
    /// choice/plural/select patterns.
    fn in_message_format_pattern(&self, nesting_level: i32) -> bool {
        nesting_level > 0 || self.parts[0].part_type == PartType::MsgStart
    }

    /// `inTopLevelChoiceMessage(int nestingLevel, ArgType parentType)`.
    fn in_top_level_choice_message(&self, nesting_level: i32, parent_type: ArgType) -> bool {
        nesting_level == 1
            && parent_type == ArgType::Choice
            && self.parts[0].part_type != PartType::MsgStart
    }

    fn add_part(&mut self, part_type: PartType, index: i32, length: i32, value: i32) {
        self.parts.push(Part::new(part_type, index, length, value));
    }

    fn add_limit_part(
        &mut self,
        start: i32,
        part_type: PartType,
        index: i32,
        length: i32,
        value: i32,
    ) {
        let size = self.parts.len() as i32;
        self.parts[start as usize].limit_part_index = size;
        self.add_part(part_type, index, length, value);
    }

    fn add_arg_double_part(&mut self, numeric_value: f64, start: i32, length: i32) {
        let numeric_index;
        match &self.numeric_values {
            None => {
                self.numeric_values = Some(Vec::new());
                numeric_index = 0;
            }
            Some(n) => {
                numeric_index = n.len() as i32;
                if numeric_index > MAX_VALUE {
                    crate::jdk::index_out_of_bounds(Some("Too many numeric values".to_string()));
                }
            }
        }
        if let Some(n) = self.numeric_values.as_mut() {
            n.push(numeric_value);
        }
        self.add_part(PartType::ArgDouble, start, length, numeric_index);
    }

    fn prefix_at(&self, start: i32) -> String {
        prefix_of(self.m(), start)
    }

    fn prefix(&self) -> String {
        prefix_of(self.m(), 0)
    }
}

/// `String.regionMatches(int toffset, String other, int ooffset, int len)`.
fn region_matches(s: &[u16], toffset: i32, other: &[u16], ooffset: i32, len: i32) -> bool {
    if ooffset < 0
        || toffset < 0
        || toffset as i64 > s.len() as i64 - len as i64
        || ooffset as i64 > other.len() as i64 - len as i64
    {
        return false;
    }
    (0..len).all(|k| s[(toffset + k) as usize] == other[(ooffset + k) as usize])
}

/// `prefix(String s, int start)`: a quoted prefix of the pattern from
/// `start` for error messages, at most `MAX_PREFIX_LENGTH` characters.
fn prefix_of(s: &[u16], start: i32) -> String {
    let mut prefix = String::with_capacity((MAX_PREFIX_LENGTH + 20) as usize);
    if start == 0 {
        prefix.push('"');
    } else {
        prefix.push_str(&format!("[at pattern index {start}] \""));
    }
    let substring_length = s.len() as i32 - start;
    if substring_length <= MAX_PREFIX_LENGTH {
        prefix.push_str(&to_rust_string(&s[start as usize..]));
    } else {
        let mut limit = start + MAX_PREFIX_LENGTH - 4;
        if is_high_surrogate(s[(limit - 1) as usize]) {
            // remove lead surrogate from the end of the prefix
            limit -= 1;
        }
        prefix.push_str(&to_rust_string(&s[start as usize..limit as usize]));
        prefix.push_str(" ...");
    }
    prefix.push('"');
    prefix
}

fn is_arg_type_char(c: i32) -> bool {
    (b'a' as i32..=b'z' as i32).contains(&c) || (b'A' as i32..=b'Z' as i32).contains(&c)
}

impl Freezable<MessagePattern> for MessagePattern {
    /// `isFrozen()`.
    fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// `freeze()`.
    fn freeze(&mut self) -> &mut MessagePattern {
        self.frozen = true;
        self
    }

    /// `cloneAsThawed()`: an unfrozen copy.
    fn clone_as_thawed(&self) -> MessagePattern {
        let mut new_msg = self.clone();
        new_msg.frozen = false;
        new_msg
    }
}

impl fmt::Display for MessagePattern {
    /// `toString()`: the pattern string (`null` when none).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.msg {
            Some(m) => f.write_str(&to_rust_string(m)),
            None => f.write_str("null"),
        }
    }
}
