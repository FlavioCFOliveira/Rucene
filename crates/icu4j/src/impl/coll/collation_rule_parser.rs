// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationRuleParser`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::extra::ParseException;
use rucene_queryparser::jdk::string::equals_ignore_case;

use crate::jdk::{char_count, code_point_at, is_high_surrogate, is_low_surrogate, to_rust_string, units};
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::norm2_all_modes::Normalizer2WithImpl;
use crate::r#impl::pattern_props::PatternProps;
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::text::collator::{self, ReorderCodes};
use crate::text::normalizer2;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;
use crate::util::u_locale;

/// Special reset positions.
///
/// Equivalent to the enum `CollationRuleParser.Position`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Position {
    /// `FIRST_TERTIARY_IGNORABLE`.
    FirstTertiaryIgnorable,
    /// `LAST_TERTIARY_IGNORABLE`.
    LastTertiaryIgnorable,
    /// `FIRST_SECONDARY_IGNORABLE`.
    FirstSecondaryIgnorable,
    /// `LAST_SECONDARY_IGNORABLE`.
    LastSecondaryIgnorable,
    /// `FIRST_PRIMARY_IGNORABLE`.
    FirstPrimaryIgnorable,
    /// `LAST_PRIMARY_IGNORABLE`.
    LastPrimaryIgnorable,
    /// `FIRST_VARIABLE`.
    FirstVariable,
    /// `LAST_VARIABLE`.
    LastVariable,
    /// `FIRST_REGULAR`.
    FirstRegular,
    /// `LAST_REGULAR`.
    LastRegular,
    /// `FIRST_IMPLICIT`.
    FirstImplicit,
    /// `LAST_IMPLICIT`.
    LastImplicit,
    /// `FIRST_TRAILING`.
    FirstTrailing,
    /// `LAST_TRAILING`.
    LastTrailing,
}

/// `POSITION_VALUES`.
pub const POSITION_VALUES: [Position; 14] = [
    Position::FirstTertiaryIgnorable,
    Position::LastTertiaryIgnorable,
    Position::FirstSecondaryIgnorable,
    Position::LastSecondaryIgnorable,
    Position::FirstPrimaryIgnorable,
    Position::LastPrimaryIgnorable,
    Position::FirstVariable,
    Position::LastVariable,
    Position::FirstRegular,
    Position::LastRegular,
    Position::FirstImplicit,
    Position::LastImplicit,
    Position::FirstTrailing,
    Position::LastTrailing,
];

impl Position {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }
}

/// First character of contractions that encode special reset positions.
/// U+FFFE cannot be tailored via rule syntax.
///
/// The second contraction character is POS_BASE + Position.
pub const POS_LEAD: u16 = 0xfffe;
/// Base for the second character of contractions that encode special
/// reset positions. Braille characters U+28xx are printable and normalization-inert.
pub const POS_BASE: u16 = 0x2800;

/// Receives the parsed rules.
///
/// Equivalent to the abstract class `CollationRuleParser.Sink`.
pub trait Sink {
    /// Adds a reset. strength=UCOL_IDENTICAL for &str. strength=UCOL_PRIMARY/UCOL_SECONDARY/UCOL_TERTIARY for &[before n]str where n=1/2/3.
    fn add_reset(&mut self, strength: i32, str: &[u16]) -> Result<()>;
    /// Adds a relation with strength and prefix | str / extension.
    fn add_relation(&mut self, strength: i32, prefix: &[u16], str: &[u16], extension: &[u16]) -> Result<()>;
    /// `suppressContractions(UnicodeSet)`.
    fn suppress_contractions(&mut self, _set: &UnicodeSet) -> Result<()> {
        Ok(())
    }
    /// `optimize(UnicodeSet)`.
    fn optimize(&mut self, _set: &UnicodeSet) -> Result<()> {
        Ok(())
    }
}

/// Imports the rules of another collation.
///
/// Equivalent to the interface `CollationRuleParser.Importer`.
pub trait Importer: Send + Sync {
    /// `getRules(String, String)`.
    fn get_rules(&self, locale_id: &str, collation_type: &str) -> Result<Vec<u16>>;
}

const UCOL_DEFAULT: i32 = -1;
const UCOL_OFF: i32 = 0;
const UCOL_ON: i32 = 1;

/// UCOL_PRIMARY=0 .. UCOL_IDENTICAL=15
const STRENGTH_MASK: i32 = 0xf;
const STARRED_FLAG: i32 = 0x10;
const OFFSET_SHIFT: i32 = 8;

const BEFORE: &str = "[before";

const POSITIONS: [&str; 14] = [
    "first tertiary ignorable",
    "last tertiary ignorable",
    "first secondary ignorable",
    "last secondary ignorable",
    "first primary ignorable",
    "last primary ignorable",
    "first variable",
    "last variable",
    "first regular",
    "last regular",
    "first implicit",
    "last implicit",
    "first trailing",
    "last trailing",
];

const G_SPECIAL_REORDER_CODES: [&str; 5] = ["space", "punct", "symbol", "currency", "digit"];

const U_PARSE_CONTEXT_LEN: i32 = 16;

/// `Throwable.getMessage()` of an error a sink or a callee threw.
fn java_get_message(e: &LuceneError) -> String {
    match e.kind() {
        LuceneErrorKind::IllegalArgument { message, .. }
        | LuceneErrorKind::IllegalState { message, .. }
        | LuceneErrorKind::UnsupportedOperation { message, .. } => message.clone(),
        LuceneErrorKind::Unchecked { message, .. } => message.clone().unwrap_or_else(|| "null".to_string()),
        LuceneErrorKind::Throwable(t) => t.get_message().map(str::to_string).unwrap_or_else(|| "null".to_string()),
        _ => e.to_string(),
    }
}

/// The out-parameters of a parse: Java's `settings` and `sink` fields.
struct Cx<'c> {
    settings: &'c mut CollationSettings,
    sink: &'c mut dyn Sink,
}

/// Parses collation rules.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationRuleParser`. Java keeps
/// the sink and the out-settings in fields set before `parse()`; here
/// [`parse`](Self::parse) receives them, because the sink (the builder)
/// owns this parser for the duration of the parse.
pub struct CollationRuleParser {
    raw_builder: Vec<u16>,
    nfd: Arc<Normalizer2WithImpl>,
    nfc: Arc<Normalizer2WithImpl>,
    rules: Vec<u16>,
    base_data: Arc<CollationData>,
    importer: Option<Arc<dyn Importer>>,
    rule_index: i32,
}

impl std::fmt::Debug for CollationRuleParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollationRuleParser").field("rule_index", &self.rule_index).finish_non_exhaustive()
    }
}

fn is_surrogate(c: i32) -> bool {
    (c as u32 & 0xfffff800) == 0xd800
}

fn is_syntax_char(c: i32) -> bool {
    (0x21..=0x7e).contains(&c) && (c <= 0x2f || (0x3a..=0x40).contains(&c) || (0x5b..=0x60).contains(&c) || (0x7b <= c))
}

fn get_on_off_value(s: &str) -> i32 {
    if s == "on" {
        UCOL_ON
    } else if s == "off" {
        UCOL_OFF
    } else {
        UCOL_DEFAULT
    }
}

impl CollationRuleParser {
    /// `CollationRuleParser(CollationData base)`.
    ///
    /// # Errors
    ///
    /// The exception of loading the NFD/NFC data
    /// (`Normalizer2.getNFDInstance()` in the field initializers).
    pub fn new(base: Arc<CollationData>) -> Result<Self> {
        Ok(Self {
            raw_builder: Vec::new(),
            nfd: normalizer2::get_nfd_instance()?,
            nfc: normalizer2::get_nfc_instance()?,
            rules: Vec::new(),
            base_data: base,
            importer: None,
            rule_index: 0,
        })
    }

    /// `setImporter(Importer)`.
    pub fn set_importer(&mut self, importer_alias: Option<Arc<dyn Importer>>) {
        self.importer = importer_alias;
    }

    /// `parse(String, CollationSettings)` with the sink of `setSink(Sink)`.
    ///
    /// # Errors
    ///
    /// `ParseException` for invalid rules.
    pub fn parse(&mut self, rule_string: &[u16], out_settings: &mut CollationSettings, sink: &mut dyn Sink) -> Result<()> {
        let mut cx = Cx { settings: out_settings, sink };
        self.parse_rules(rule_string, &mut cx)
    }

    fn len(&self) -> i32 {
        self.rules.len() as i32
    }

    fn char_at(&self, i: i32) -> u16 {
        self.rules[i as usize]
    }

    fn parse_rules(&mut self, rule_string: &[u16], cx: &mut Cx<'_>) -> Result<()> {
        self.rules = rule_string.to_vec();
        self.rule_index = 0;
        while self.rule_index < self.len() {
            let c = self.char_at(self.rule_index);
            if PatternProps::is_white_space(c as i32) {
                self.rule_index += 1;
                continue;
            }
            match c {
                0x26 => self.parse_rule_chain(cx)?, // '&'
                0x5b => self.parse_setting(cx)?,    // '['
                0x23 => {
                    // '#' starts a comment, until the end of the line
                    self.rule_index = self.skip_comment(self.rule_index + 1);
                }
                0x40 => {
                    // '@' is equivalent to [backwards 2]
                    cx.settings.set_flag(CollationSettings::BACKWARD_SECONDARY, true);
                    self.rule_index += 1;
                }
                0x21 => {
                    // '!' used to turn on Thai/Lao character reversal
                    // Accept but ignore. The root collator has contractions
                    // that are equivalent to the character reversal, where appropriate.
                    self.rule_index += 1;
                }
                _ => return Err(self.set_parse_error("expected a reset or setting or comment")),
            }
        }
        Ok(())
    }

    fn parse_rule_chain(&mut self, cx: &mut Cx<'_>) -> Result<()> {
        let reset_strength = self.parse_reset_and_position(cx)?;
        let mut is_first_relation = true;
        loop {
            let result = self.parse_relation_operator();
            if result < 0 {
                if self.rule_index < self.len() && self.char_at(self.rule_index) == 0x23 {
                    // '#' starts a comment, until the end of the line
                    self.rule_index = self.skip_comment(self.rule_index + 1);
                    continue;
                }
                if is_first_relation {
                    return Err(self.set_parse_error("reset not followed by a relation"));
                }
                return Ok(());
            }
            let strength = result & STRENGTH_MASK;
            if reset_strength < collator::IDENTICAL {
                // reset-before rule chain
                if is_first_relation {
                    if strength != reset_strength {
                        return Err(self.set_parse_error("reset-before strength differs from its first relation"));
                    }
                } else if strength < reset_strength {
                    return Err(self.set_parse_error("reset-before strength followed by a stronger relation"));
                }
            }
            let i = self.rule_index + (result >> OFFSET_SHIFT); // skip over the relation operator
            if (result & STARRED_FLAG) == 0 {
                self.parse_relation_strings(strength, i, cx)?;
            } else {
                self.parse_starred_characters(strength, i, cx)?;
            }
            is_first_relation = false;
        }
    }

    fn region_matches(&self, i: i32, s: &str) -> bool {
        let u = units(s);
        if i < 0 || i as usize + u.len() > self.rules.len() {
            return false;
        }
        self.rules[i as usize..i as usize + u.len()] == u[..]
    }

    fn parse_reset_and_position(&mut self, cx: &mut Cx<'_>) -> Result<i32> {
        let mut i = self.skip_white_space(self.rule_index + 1);
        let mut j;
        let reset_strength;
        let mut before_strength = None;
        if self.region_matches(i, BEFORE) {
            j = i + BEFORE.len() as i32;
            if j < self.len() && PatternProps::is_white_space(self.char_at(j) as i32) {
                j = self.skip_white_space(j + 1);
                if j + 1 < self.len() {
                    let c = self.char_at(j) as i32;
                    if (0x31..=0x33).contains(&c) && self.char_at(j + 1) == 0x5d {
                        before_strength = Some((c, j));
                    }
                }
            }
        }
        if let Some((c, j)) = before_strength {
            // &[before n] with n=1 or 2 or 3
            reset_strength = collator::PRIMARY + (c - 0x31);
            i = self.skip_white_space(j + 2);
        } else {
            reset_strength = collator::IDENTICAL;
        }
        if i >= self.len() {
            return Err(self.set_parse_error("reset without position"));
        }
        let mut raw = std::mem::take(&mut self.raw_builder);
        let r = if self.char_at(i) == 0x5b {
            // '['
            self.parse_special_position(i, &mut raw)
        } else {
            self.parse_tailoring_string(i, &mut raw)
        };
        let i = match r {
            Ok(i) => i,
            Err(e) => {
                self.raw_builder = raw;
                return Err(e);
            }
        };
        let added = cx.sink.add_reset(reset_strength, &raw);
        self.raw_builder = raw;
        if let Err(e) = added {
            return Err(self.set_parse_error_with("adding reset failed", e));
        }
        self.rule_index = i;
        Ok(reset_strength)
    }

    fn parse_relation_operator(&mut self) -> i32 {
        self.rule_index = self.skip_white_space(self.rule_index);
        if self.rule_index >= self.len() {
            return UCOL_DEFAULT;
        }
        let mut strength;
        let mut i = self.rule_index;
        let c = self.char_at(i);
        i += 1;
        match c {
            0x3c => {
                // '<'
                if i < self.len() && self.char_at(i) == 0x3c {
                    // <<
                    i += 1;
                    if i < self.len() && self.char_at(i) == 0x3c {
                        // <<<
                        i += 1;
                        if i < self.len() && self.char_at(i) == 0x3c {
                            // <<<<
                            i += 1;
                            strength = collator::QUATERNARY;
                        } else {
                            strength = collator::TERTIARY;
                        }
                    } else {
                        strength = collator::SECONDARY;
                    }
                } else {
                    strength = collator::PRIMARY;
                }
                if i < self.len() && self.char_at(i) == 0x2a {
                    // '*'
                    i += 1;
                    strength |= STARRED_FLAG;
                }
            }
            0x3b => {
                // ';' same as <<
                strength = collator::SECONDARY;
            }
            0x2c => {
                // ',' same as <<<
                strength = collator::TERTIARY;
            }
            0x3d => {
                // '='
                strength = collator::IDENTICAL;
                if i < self.len() && self.char_at(i) == 0x2a {
                    // '*'
                    i += 1;
                    strength |= STARRED_FLAG;
                }
            }
            _ => return UCOL_DEFAULT,
        }
        ((i - self.rule_index) << OFFSET_SHIFT) | strength
    }

    fn parse_relation_strings(&mut self, strength: i32, mut i: i32, cx: &mut Cx<'_>) -> Result<()> {
        // Parse
        //     prefix | str / extension
        // where prefix and extension are optional.
        let mut prefix: Vec<u16> = Vec::new();
        let mut extension: Vec<u16> = Vec::new();
        let mut raw = std::mem::take(&mut self.raw_builder);
        let result = (|| -> Result<i32> {
            i = self.parse_tailoring_string(i, &mut raw)?;
            let mut next = if i < self.len() { self.char_at(i) } else { 0 };
            if next == 0x7c {
                // '|' separates the context prefix from the string.
                prefix = raw.clone();
                i = self.parse_tailoring_string(i + 1, &mut raw)?;
                next = if i < self.len() { self.char_at(i) } else { 0 };
            }
            // str = rawBuilder (do not modify rawBuilder any more in this function)
            if next == 0x2f {
                // '/' separates the string from the extension.
                let mut ext_builder = Vec::new();
                i = self.parse_tailoring_string(i + 1, &mut ext_builder)?;
                extension = ext_builder;
            }
            if !prefix.is_empty() {
                let prefix0 = code_point_at(&prefix, 0);
                let c = code_point_at(&raw, 0);
                if !self.nfc.has_boundary_before(prefix0) || !self.nfc.has_boundary_before(c) {
                    return Err(self.set_parse_error("in 'prefix|str', prefix and str must each start with an NFC boundary"));
                }
            }
            if let Err(e) = cx.sink.add_relation(strength, &prefix, &raw, &extension) {
                return Err(self.set_parse_error_with("adding relation failed", e));
            }
            Ok(i)
        })();
        self.raw_builder = raw;
        self.rule_index = result?;
        Ok(())
    }

    fn parse_starred_characters(&mut self, strength: i32, mut i: i32, cx: &mut Cx<'_>) -> Result<()> {
        let empty: [u16; 0] = [];
        let mut raw = std::mem::take(&mut self.raw_builder);
        let result = (|| -> Result<i32> {
            i = self.parse_string(self.skip_white_space(i), &mut raw)?;
            if raw.is_empty() {
                return Err(self.set_parse_error("missing starred-relation string"));
            }
            let mut prev = -1;
            let mut j = 0;
            loop {
                while j < raw.len() as i32 {
                    let c = code_point_at(&raw, j);
                    if !self.nfd.is_inert(c) {
                        return Err(self.set_parse_error("starred-relation string is not all NFD-inert"));
                    }
                    let s = UTF16::value_of(c)?;
                    if let Err(e) = cx.sink.add_relation(strength, &empty, &s, &empty) {
                        return Err(self.set_parse_error_with("adding relation failed", e));
                    }
                    j += char_count(c);
                    prev = c;
                }
                if i >= self.len() || self.char_at(i) != 0x2d {
                    // '-'
                    break;
                }
                if prev < 0 {
                    return Err(self.set_parse_error("range without start in starred-relation string"));
                }
                i = self.parse_string(i + 1, &mut raw)?;
                if raw.is_empty() {
                    return Err(self.set_parse_error("range without end in starred-relation string"));
                }
                let c = code_point_at(&raw, 0);
                if c < prev {
                    return Err(self.set_parse_error("range start greater than end in starred-relation string"));
                }
                // range prev-c
                loop {
                    prev += 1;
                    if prev > c {
                        break;
                    }
                    if !self.nfd.is_inert(prev) {
                        return Err(self.set_parse_error("starred-relation string range is not all NFD-inert"));
                    }
                    if is_surrogate(prev) {
                        return Err(self.set_parse_error("starred-relation string range contains a surrogate"));
                    }
                    if (0xfffd..=0xffff).contains(&prev) {
                        return Err(self.set_parse_error("starred-relation string range contains U+FFFD, U+FFFE or U+FFFF"));
                    }
                    let s = UTF16::value_of(prev)?;
                    if let Err(e) = cx.sink.add_relation(strength, &empty, &s, &empty) {
                        return Err(self.set_parse_error_with("adding relation failed", e));
                    }
                }
                prev = -1;
                j = char_count(c);
            }
            Ok(self.skip_white_space(i))
        })();
        self.raw_builder = raw;
        self.rule_index = result?;
        Ok(())
    }

    fn parse_tailoring_string(&mut self, i: i32, raw: &mut Vec<u16>) -> Result<i32> {
        let i = self.parse_string(self.skip_white_space(i), raw)?;
        if raw.is_empty() {
            return Err(self.set_parse_error("missing relation string"));
        }
        Ok(self.skip_white_space(i))
    }

    fn parse_string(&mut self, mut i: i32, raw: &mut Vec<u16>) -> Result<i32> {
        raw.clear();
        while i < self.len() {
            let mut c = self.char_at(i);
            i += 1;
            if is_syntax_char(c as i32) {
                if c == 0x27 {
                    // apostrophe
                    if i < self.len() && self.char_at(i) == 0x27 {
                        // Double apostrophe, encodes a single one.
                        raw.push(0x27);
                        i += 1;
                        continue;
                    }
                    // Quote literal text until the next single apostrophe.
                    loop {
                        if i == self.len() {
                            return Err(self.set_parse_error("quoted literal text missing terminating apostrophe"));
                        }
                        c = self.char_at(i);
                        i += 1;
                        if c == 0x27 {
                            if i < self.len() && self.char_at(i) == 0x27 {
                                // Double apostrophe inside quoted literal text,
                                // still encodes a single apostrophe.
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        raw.push(c);
                    }
                } else if c == 0x5c {
                    // backslash
                    if i == self.len() {
                        return Err(self.set_parse_error("backslash escape at the end of the rule string"));
                    }
                    let cp = code_point_at(&self.rules, i);
                    crate::jdk::append_code_point(raw, cp)?;
                    i += char_count(cp);
                } else {
                    // Any other syntax character terminates a string.
                    i -= 1;
                    break;
                }
            } else if PatternProps::is_white_space(c as i32) {
                // Unquoted white space terminates a string.
                i -= 1;
                break;
            } else {
                raw.push(c);
            }
        }
        let mut j = 0;
        while j < raw.len() as i32 {
            let c = code_point_at(raw, j);
            if is_surrogate(c) {
                return Err(self.set_parse_error("string contains an unpaired surrogate"));
            }
            if (0xfffd..=0xffff).contains(&c) {
                return Err(self.set_parse_error("string contains U+FFFD, U+FFFE or U+FFFF"));
            }
            j += char_count(c);
        }
        Ok(i)
    }

    fn parse_special_position(&mut self, i: i32, str: &mut Vec<u16>) -> Result<i32> {
        let mut words = Vec::new();
        let mut j = self.read_words(i + 1, &mut words);
        if j > i && self.char_at(j) == 0x5d && !words.is_empty() {
            // words end with ]
            j += 1;
            let raw = to_rust_string(&words);
            str.clear();
            for (pos, p) in POSITIONS.iter().enumerate() {
                if raw == *p {
                    str.push(POS_LEAD);
                    str.push(POS_BASE + pos as u16);
                    return Ok(j);
                }
            }
            if raw == "top" {
                str.push(POS_LEAD);
                str.push(POS_BASE + Position::LastRegular.ordinal() as u16);
                return Ok(j);
            }
            if raw == "variable top" {
                str.push(POS_LEAD);
                str.push(POS_BASE + Position::LastVariable.ordinal() as u16);
                return Ok(j);
            }
        }
        Err(self.set_parse_error("not a valid special reset position"))
    }

    fn parse_setting(&mut self, cx: &mut Cx<'_>) -> Result<()> {
        let i = self.rule_index + 1;
        let mut words = Vec::new();
        let mut j = self.read_words(i, &mut words);
        if j <= i || words.is_empty() {
            return Err(self.set_parse_error("expected a setting/option at '['"));
        }
        // startsWith() etc. are available for String but not CharSequence/StringBuilder.
        let mut raw_u = words.clone();
        if self.char_at(j) == 0x5d {
            // words end with ]
            j += 1;
            if raw_u.starts_with(&units("reorder")) && (raw_u.len() == 7 || raw_u[7] == 0x20) {
                self.parse_reordering(&raw_u, cx)?;
                self.rule_index = j;
                return Ok(());
            }
            if raw_u == units("backwards 2") {
                cx.settings.set_flag(CollationSettings::BACKWARD_SECONDARY, true);
                self.rule_index = j;
                return Ok(());
            }
            let v_u: Vec<u16>;
            let value_index = raw_u.iter().rposition(|&c| c == 0x20);
            if let Some(value_index) = value_index {
                v_u = raw_u[value_index + 1..].to_vec();
                raw_u.truncate(value_index);
            } else {
                v_u = Vec::new();
            }
            let raw = to_rust_string(&raw_u);
            let v = to_rust_string(&v_u);
            if raw == "strength" && v_u.len() == 1 {
                let mut value = UCOL_DEFAULT;
                let c = v_u[0] as i32;
                if (0x31..=0x34).contains(&c) {
                    // 1..4
                    value = collator::PRIMARY + (c - 0x31);
                } else if c == 0x49 {
                    // 'I'
                    value = collator::IDENTICAL;
                }
                if value != UCOL_DEFAULT {
                    cx.settings.set_strength(value)?;
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "alternate" {
                let mut value = UCOL_DEFAULT;
                if v == "non-ignorable" {
                    value = 0; // UCOL_NON_IGNORABLE
                } else if v == "shifted" {
                    value = 1; // UCOL_SHIFTED
                }
                if value != UCOL_DEFAULT {
                    cx.settings.set_alternate_handling_shifted(value > 0);
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "maxVariable" {
                let mut value = UCOL_DEFAULT;
                if v == "space" {
                    value = CollationSettings::MAX_VAR_SPACE;
                } else if v == "punct" {
                    value = CollationSettings::MAX_VAR_PUNCT;
                } else if v == "symbol" {
                    value = CollationSettings::MAX_VAR_SYMBOL;
                } else if v == "currency" {
                    value = CollationSettings::MAX_VAR_CURRENCY;
                }
                if value != UCOL_DEFAULT {
                    cx.settings.set_max_variable(value, 0)?;
                    cx.settings.variable_top = self.base_data.get_last_primary_for_group(ReorderCodes::FIRST + value);
                    debug_assert!(cx.settings.variable_top != 0);
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "caseFirst" {
                let mut value = UCOL_DEFAULT;
                if v == "off" {
                    value = UCOL_OFF;
                } else if v == "lower" {
                    value = CollationSettings::CASE_FIRST; // UCOL_LOWER_FIRST
                } else if v == "upper" {
                    value = CollationSettings::CASE_FIRST_AND_UPPER_MASK; // UCOL_UPPER_FIRST
                }
                if value != UCOL_DEFAULT {
                    cx.settings.set_case_first(value);
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "caseLevel" {
                let value = get_on_off_value(&v);
                if value != UCOL_DEFAULT {
                    cx.settings.set_flag(CollationSettings::CASE_LEVEL, value > 0);
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "normalization" {
                let value = get_on_off_value(&v);
                if value != UCOL_DEFAULT {
                    cx.settings.set_flag(CollationSettings::CHECK_FCD, value > 0);
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "numericOrdering" {
                let value = get_on_off_value(&v);
                if value != UCOL_DEFAULT {
                    cx.settings.set_flag(CollationSettings::NUMERIC, value > 0);
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "hiraganaQ" {
                let value = get_on_off_value(&v);
                if value != UCOL_DEFAULT {
                    if value == UCOL_ON {
                        return Err(self.set_parse_error("[hiraganaQ on] is not supported"));
                    }
                    self.rule_index = j;
                    return Ok(());
                }
            } else if raw == "import" {
                // BCP 47 language tag -> ICU locale ID
                let locale_id = match (|| {
                    let mut b = u_locale::Builder::default();
                    b.set_language_tag(&v)?;
                    b.build()
                })() {
                    Ok(l) => l,
                    Err(e) => return Err(self.set_parse_error_with("expected language tag in [import langTag]", e)),
                };
                // localeID minus all keywords
                let base_id = locale_id.get_base_name();
                // @collation=type, or length=0 if not specified
                let collation_type = locale_id.get_keyword_value("collation");
                match self.importer.clone() {
                    None => return Err(self.set_parse_error("[import langTag] is not supported")),
                    Some(importer) => {
                        let imported_rules = match importer
                            .get_rules(&base_id, collation_type.as_deref().unwrap_or("standard"))
                        {
                            Ok(r) => r,
                            Err(e) => return Err(self.set_parse_error_with("[import langTag] failed", e)),
                        };
                        let outer_rules = std::mem::take(&mut self.rules);
                        let outer_rule_index = self.rule_index;
                        if let Err(e) = self.parse_rules(&imported_rules, cx) {
                            self.rule_index = outer_rule_index; // Restore the original index for error reporting.
                            let err = self.set_parse_error_with_rules("parsing imported rules failed", e, &outer_rules);
                            self.rules = outer_rules;
                            return Err(err);
                        }
                        self.rules = outer_rules;
                        self.rule_index = j;
                    }
                }
                return Ok(());
            }
        } else if self.char_at(j) == 0x5b {
            // words end with [
            let raw = to_rust_string(&raw_u);
            let mut set = UnicodeSet::new();
            j = self.parse_unicode_set(j, &mut set)?;
            if raw == "optimize" {
                if let Err(e) = cx.sink.optimize(&set) {
                    return Err(self.set_parse_error_with("[optimize set] failed", e));
                }
                self.rule_index = j;
                return Ok(());
            } else if raw == "suppressContractions" {
                if let Err(e) = cx.sink.suppress_contractions(&set) {
                    return Err(self.set_parse_error_with("[suppressContractions set] failed", e));
                }
                self.rule_index = j;
                return Ok(());
            }
        }
        Err(self.set_parse_error("not a valid setting/option"))
    }

    fn parse_reordering(&mut self, raw: &[u16], cx: &mut Cx<'_>) -> Result<()> {
        let mut i = 7usize; // after "reorder"
        if i == raw.len() {
            // empty [reorder] with no codes
            cx.settings.reset_reordering();
            return Ok(());
        }
        // Parse the codes in [reorder aa bb cc].
        let mut reorder_codes: Vec<i32> = Vec::new();
        while i < raw.len() {
            i += 1; // skip the word-separating space
            let mut limit = i;
            while limit < raw.len() && raw[limit] != 0x20 {
                limit += 1;
            }
            let word = to_rust_string(&raw[i..limit]);
            let code = Self::get_reorder_code(&word)?;
            if code < 0 {
                return Err(self.set_parse_error("unknown script or reorder code"));
            }
            reorder_codes.push(code);
            i = limit;
        }
        if reorder_codes.is_empty() {
            cx.settings.reset_reordering();
        } else {
            cx.settings.set_reordering(&self.base_data, &reorder_codes)?;
        }
        Ok(())
    }

    /// Gets a script or reorder code from its string representation.
    ///
    /// Equivalent to `getReorderCode(String)`; returns the script or reorder
    /// code, or <0 if not recognized.
    ///
    /// # Errors
    ///
    /// Exceptions of the property lookup other than the
    /// `IllegalIcuArgumentException` Java catches.
    pub fn get_reorder_code(word: &str) -> Result<i32> {
        for (i, code) in G_SPECIAL_REORDER_CODES.iter().enumerate() {
            if equals_ignore_case(word, code) {
                return Ok(ReorderCodes::FIRST + i as i32);
            }
        }
        match UCharacter::get_property_value_enum(UProperty::SCRIPT, &units(word)) {
            Ok(script) if script >= 0 => return Ok(script),
            Ok(_) => {}
            Err(e) if IllegalIcuArgumentException::of(&e).is_some() => {
                // fall through
            }
            Err(e) => return Err(e),
        }
        if equals_ignore_case(word, "others") {
            return Ok(ReorderCodes::OTHERS); // same as Zzzz = USCRIPT_UNKNOWN
        }
        Ok(-1)
    }

    fn parse_unicode_set(&mut self, i: i32, set: &mut UnicodeSet) -> Result<i32> {
        // Collect a UnicodeSet pattern between a balanced pair of [brackets].
        let mut level = 0;
        let mut j = i;
        loop {
            if j == self.len() {
                return Err(self.set_parse_error("unbalanced UnicodeSet pattern brackets"));
            }
            let c = self.char_at(j);
            j += 1;
            if c == 0x5b {
                // '['
                level += 1;
            } else if c == 0x5d {
                // ']'
                level -= 1;
                if level == 0 {
                    break;
                }
            }
        }
        let pattern = self.rules[i as usize..j as usize].to_vec();
        if let Err(e) = set.apply_pattern(&pattern) {
            return Err(self.set_parse_error(&format!("not a valid UnicodeSet pattern: {}", java_get_message(&e))));
        }
        j = self.skip_white_space(j);
        if j == self.len() || self.char_at(j) != 0x5d {
            return Err(self.set_parse_error("missing option-terminating ']' after UnicodeSet pattern"));
        }
        j += 1;
        Ok(j)
    }

    fn read_words(&self, mut i: i32, raw: &mut Vec<u16>) -> i32 {
        raw.clear();
        i = self.skip_white_space(i);
        loop {
            if i >= self.len() {
                return 0;
            }
            let c = self.char_at(i);
            if is_syntax_char(c as i32) && c != 0x2d && c != 0x5f {
                // syntax except -_
                if raw.is_empty() {
                    return i;
                }
                let last_index = raw.len() - 1;
                if raw[last_index] == 0x20 {
                    // remove trailing space
                    raw.truncate(last_index);
                }
                return i;
            }
            if PatternProps::is_white_space(c as i32) {
                raw.push(0x20);
                i = self.skip_white_space(i + 1);
            } else {
                raw.push(c);
                i += 1;
            }
        }
    }

    fn skip_comment(&self, mut i: i32) -> i32 {
        // skip to past the newline
        while i < self.len() {
            let c = self.char_at(i);
            i += 1;
            // LF or FF or CR or NEL or LS or PS
            if c == 0xa || c == 0xc || c == 0xd || c == 0x85 || c == 0x2028 || c == 0x2029 {
                // Unicode Newline Guidelines: "A readline function should stop at NLF, LS, FF, or PS."
                // NLF (new line function) = CR or LF or CR+LF or NEL.
                // No need to collect all of CR+LF because a following LF will be ignored anyway.
                break;
            }
        }
        i
    }

    /// `setParseError(String)`: the exception Java throws.
    fn set_parse_error(&self, reason: &str) -> LuceneError {
        self.make_parse_exception(reason, None)
    }

    /// `setParseError(String, Exception)`.
    fn set_parse_error_with(&self, reason: &str, e: LuceneError) -> LuceneError {
        let msg = format!("{reason}: {}", java_get_message(&e));
        self.make_parse_exception(&msg, Some(e))
    }

    /// `setParseError(String, Exception)` with `rules` restored to the
    /// outer rules first (Java restores the field after the call; the
    /// context is computed from the imported rules still in the field).
    fn set_parse_error_with_rules(&self, reason: &str, e: LuceneError, _outer_rules: &[u16]) -> LuceneError {
        self.set_parse_error_with(reason, e)
    }

    fn make_parse_exception(&self, reason: &str, cause: Option<LuceneError>) -> LuceneError {
        let message = self.append_error_context(reason);
        match cause {
            None => ParseException::error(message, self.rule_index),
            Some(c) => ParseException::error_with_cause(message, self.rule_index, c),
        }
    }

    /// C++ setErrorContext().
    ///
    /// The message is a Rust `String` (the crate's error messages are), so
    /// an unpaired surrogate of the rules in the context window reads as
    /// U+FFFD; the error offset is exact.
    fn append_error_context(&self, reason: &str) -> String {
        // Note: This relies on the calling code maintaining the ruleIndex
        // at a position that is useful for debugging.
        // For example, at the beginning of a reset or relation etc.
        let mut msg: Vec<u16> = units(reason);
        msg.extend(units(&format!(" at index {}", self.rule_index)));
        // We are not counting line numbers.

        msg.extend(units(" near \""));
        // before ruleIndex
        let mut start = self.rule_index - (U_PARSE_CONTEXT_LEN - 1);
        if start < 0 {
            start = 0;
        } else if start > 0 && is_low_surrogate(self.char_at(start)) {
            start += 1;
        }
        msg.extend_from_slice(&self.rules[start as usize..self.rule_index as usize]);

        msg.push(0x21); // '!'
        // starting from ruleIndex
        let mut length = self.len() - self.rule_index;
        if length >= U_PARSE_CONTEXT_LEN {
            length = U_PARSE_CONTEXT_LEN - 1;
            if is_high_surrogate(self.char_at(self.rule_index + length - 1)) {
                length -= 1;
            }
        }
        msg.extend_from_slice(&self.rules[self.rule_index as usize..(self.rule_index + length) as usize]);
        msg.push(0x22); // '"'
        String::from_utf16_lossy(&msg)
    }

    fn skip_white_space(&self, mut i: i32) -> i32 {
        while i < self.len() && PatternProps::is_white_space(self.char_at(i) as i32) {
            i += 1;
        }
        i
    }
}
