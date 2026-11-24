// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.TransliteratorParser`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, LuceneErrorKind, Result};

use crate::jdk::{
    index_of, index_of_unit, string_index_out_of_bounds, to_rust_string, units, ParsePosition,
};
use crate::lang::u_character::UCharacter;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::utility::Utility;
use crate::text::function_replacer::FunctionReplacer;
use crate::text::normalizer::{Mode, Normalizer};
use crate::text::quantifier::Quantifier;
use crate::text::rule_based_transliterator::{Data, Variable};
use crate::text::string_matcher::StringMatcher;
use crate::text::string_replacer::StringReplacer;
use crate::text::symbol_table::{SymbolTable, SYMBOL_REF};
use crate::text::transliteration_rule::TransliterationRule;
use crate::text::transliterator::{FORWARD, REVERSE};
use crate::text::transliterator_id_parser::TransliteratorIDParser;
use crate::text::unicode_matcher::UnicodeMatcher;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

//----------------------------------------------------------------------
// Constants
//----------------------------------------------------------------------

// Indicator for ID blocks
const ID_TOKEN: &[u16] = &[b':' as u16, b':' as u16];
const ID_TOKEN_LEN: i32 = 2;

// Operators
const VARIABLE_DEF_OP: u16 = b'=' as u16;
const FORWARD_RULE_OP: u16 = b'>' as u16;
const REVERSE_RULE_OP: u16 = b'<' as u16;
const FWDREV_RULE_OP: u16 = b'~' as u16; // internal rep of <> op

const OPERATORS: &[u16] = &[0x3D, 0x3E, 0x3C, 0x2190, 0x2192, 0x2194]; // "=><←→↔"
const HALF_ENDERS: &[u16] = &[0x3D, 0x3E, 0x3C, 0x2190, 0x2192, 0x2194, 0x3B]; // "=><←→↔;"

// Other special characters
const QUOTE: u16 = b'\'' as u16;
const ESCAPE: u16 = b'\\' as u16;
const END_OF_RULE: u16 = b';' as u16;
const RULE_COMMENT_CHAR: u16 = b'#' as u16;

const CONTEXT_ANTE: u16 = b'{' as u16; // ante{key
const CONTEXT_POST: u16 = b'}' as u16; // key}post
const CURSOR_POS: u16 = b'|' as u16;
const CURSOR_OFFSET: u16 = b'@' as u16;
const ANCHOR_START: u16 = b'^' as u16;

const KLEENE_STAR: u16 = b'*' as u16;
const ONE_OR_MORE: u16 = b'+' as u16;
const ZERO_OR_ONE: u16 = b'?' as u16;

const DOT: u16 = b'.' as u16;
const DOT_SET: &str = "[^[:Zp:][:Zl:]\\r\\n$]";

// By definition, the ANCHOR_END special character is a
// trailing SymbolTable.SYMBOL_REF character.
// private static final char ANCHOR_END       = '$';

// Segments of the input string are delimited by "(" and ")".  In the
// output string these segments are referenced as "$1", "$2", etc.
const SEGMENT_OPEN: u16 = b'(' as u16;
const SEGMENT_CLOSE: u16 = b')' as u16;

// A function is denoted &Source-Target/Variant(text)
const FUNCTION: u16 = b'&' as u16;

// Aliases for some of the syntax characters. These are provided so
// transliteration rules can be expressed in XML without clashing with
// XML syntax characters '<', '>', and '&'.
const ALT_REVERSE_RULE_OP: u16 = 0x2190; // Left Arrow
const ALT_FORWARD_RULE_OP: u16 = 0x2192; // Right Arrow
const ALT_FWDREV_RULE_OP: u16 = 0x2194; // Left Right Arrow
const ALT_FUNCTION: u16 = 0x2206; // Increment (~Greek Capital Delta)

// Special characters disallowed at the top level
static ILLEGAL_TOP: LazyLock<UnicodeSet> = LazyLock::new(|| static_set("[\\)]"));

// Special characters disallowed within a segment
static ILLEGAL_SEG: LazyLock<UnicodeSet> = LazyLock::new(|| static_set("[\\{\\}\\|\\@]"));

// Special characters disallowed within a function argument
static ILLEGAL_FUNC: LazyLock<UnicodeSet> =
    LazyLock::new(|| static_set("[\\^\\(\\.\\*\\+\\?\\{\\}\\|\\@]"));

fn static_set(pattern: &str) -> UnicodeSet {
    match UnicodeSet::from_pattern(&units(pattern)) {
        Ok(set) => set,
        Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
    }
}

/// `String.charAt(int index)`.
///
/// # Panics
///
/// Java's `StringIndexOutOfBoundsException` (uncaught by ICU).
fn char_at(s: &[u16], index: i32) -> u16 {
    if index < 0 || index as usize >= s.len() {
        string_index_out_of_bounds(index);
    }
    s[index as usize]
}

/// `String.substring(int begin, int end)`.
///
/// # Panics
///
/// Java's `StringIndexOutOfBoundsException` (uncaught by ICU).
fn substring(s: &[u16], begin: i32, end: i32) -> Vec<u16> {
    assert!(
        begin >= 0 && begin <= end && end as usize <= s.len(),
        "java.lang.StringIndexOutOfBoundsException: begin {begin}, end {end}, length {}",
        s.len()
    );
    s[begin as usize..end as usize].to_vec()
}

/// The symbol table of a parse: the variables defined so far.
///
/// Equivalent to the inner class `TransliteratorParser.ParseData`, which
/// reads the parser's `variableNames`, `variablesVector` and
/// `curData.variablesBase`; here it borrows them.
struct ParseData<'a> {
    variable_names: &'a HashMap<Vec<u16>, Vec<u16>>,
    variables_vector: &'a [Variable],
    variables_base: Option<u16>,
}

impl ParseData<'_> {
    fn variables_base(&self) -> i32 {
        match self.variables_base {
            Some(b) => b as i32,
            None => panic!("java.lang.NullPointerException: Cannot read field \"variablesBase\" because \"this.curData\" is null"),
        }
    }

    /// `parseReference(String text, ParsePosition pos, int limit)`.
    fn parse_reference_impl(text: &[u16], pos: &mut ParsePosition, limit: i32) -> Option<Vec<u16>> {
        let start = pos.get_index();
        let mut i = start;
        while i < limit {
            let c = char_at(text, i);
            if (i == start && !UCharacter::is_unicode_identifier_start(c as i32))
                || !UCharacter::is_unicode_identifier_part(c as i32)
            {
                break;
            }
            i += 1;
        }
        if i == start {
            // No valid name chars
            return None;
        }
        pos.set_index(i);
        Some(substring(text, start, i))
    }

    /// `isMatcher(int ch)`: return true if the given character is a
    /// matcher standin or a plain character (non standin).
    fn is_matcher(&self, ch: i32) -> bool {
        // Note that we cannot use data.lookup() because the
        // set array has not been constructed yet.
        let i = ch - self.variables_base();
        if i >= 0 && i < self.variables_vector.len() as i32 {
            return self.variables_vector[i as usize].is_matcher();
        }
        true
    }

    /// `isReplacer(int ch)`: return true if the given character is a
    /// replacer standin or a plain character (non standin).
    fn is_replacer(&self, ch: i32) -> bool {
        // Note that we cannot use data.lookup() because the
        // set array has not been constructed yet.
        let i = ch - self.variables_base();
        if i >= 0 && i < self.variables_vector.len() as i32 {
            return self.variables_vector[i as usize].is_replacer();
        }
        true
    }
}

impl SymbolTable for ParseData<'_> {
    /// `lookup(String name)`.
    fn lookup(&self, name: &[u16]) -> Option<Vec<u16>> {
        self.variable_names.get(name).cloned()
    }

    /// `lookupMatcher(int ch)`.
    fn lookup_matcher(&self, ch: i32) -> Option<Arc<dyn UnicodeMatcher>> {
        // Note that we cannot use data.lookup() because the
        // set array has not been constructed yet.
        let i = ch - self.variables_base();
        if i >= 0 && i < self.variables_vector.len() as i32 {
            return self.variables_vector[i as usize].as_matcher_arc();
        }
        None
    }

    /// `parseReference(String text, ParsePosition pos, int limit)`: only
    /// the identifier is parsed; the caller looks it up.
    fn parse_reference(
        &self,
        text: &[u16],
        pos: &mut ParsePosition,
        limit: i32,
    ) -> Result<Option<Vec<u16>>> {
        Ok(Self::parse_reference_impl(text, pos, limit))
    }
}

/// A source of rule lines.
///
/// Equivalent to the nested abstract class `TransliteratorParser.RuleBody`.
trait RuleBody {
    /// `nextLine()`: retrieve the next line of the source, or return `None`
    /// if none. Folds lines terminated by a backslash into the next line,
    /// without regard for intervening comments.
    fn next_line(&mut self) -> Option<Vec<u16>> {
        let mut s = self.handle_next_line();
        if let Some(line) = &s {
            if !line.is_empty() && line[line.len() - 1] == b'\\' as u16 {
                let mut b = line.clone();
                loop {
                    b.pop();
                    s = self.handle_next_line();
                    let Some(next) = &s else {
                        break;
                    };
                    b.extend_from_slice(next);
                    if !(!next.is_empty() && next[next.len() - 1] == b'\\' as u16) {
                        break;
                    }
                }
                s = Some(b);
            }
        }
        s
    }

    /// `reset()`: reset to the first line of the source.
    fn reset(&mut self);

    /// `handleNextLine()`: subclass method to return the next line of the
    /// source.
    fn handle_next_line(&mut self) -> Option<Vec<u16>>;
}

/// `RuleBody` subclass for a `String[]` array.
///
/// Equivalent to the nested class `TransliteratorParser.RuleArray`.
struct RuleArray {
    array: Vec<Vec<u16>>,
    i: usize,
}

impl RuleArray {
    /// `RuleArray(String[] array)`.
    fn new(array: Vec<Vec<u16>>) -> Self {
        Self { array, i: 0 }
    }
}

impl RuleBody for RuleArray {
    fn handle_next_line(&mut self) -> Option<Vec<u16>> {
        if self.i < self.array.len() {
            let s = self.array[self.i].clone();
            self.i += 1;
            Some(s)
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.i = 0;
    }
}

/// A class representing one side of a rule. This class knows how to parse
/// half of a rule. It is tightly coupled to the method
/// `TransliteratorParser.parseRule()`.
///
/// Equivalent to the nested class `TransliteratorParser.RuleHalf`.
struct RuleHalf {
    text: Vec<u16>,
    cursor: i32, // position of cursor in text
    ante: i32,   // position of ante context marker '{' in text
    post: i32,   // position of post context marker '}' in text
    // Record the offset to the cursor either to the left or to the
    // right of the key.  This is indicated by characters on the output
    // side that allow the cursor to be positioned arbitrarily within
    // the matching text.  For example, abc{def} > | @@@ xyz; changes
    // def to xyz and moves the cursor to before abc.  Offset characters
    // must be at the start or end, and they cannot move the cursor past
    // the ante- or postcontext text.  Placeholders are only valid in
    // output text.  The length of the ante and post context is
    // determined at runtime, because of supplementals and quantifiers.
    cursor_offset: i32, // only nonzero on output side
    // Position of first CURSOR_OFFSET on _right_.  This will be -1
    // for |@, -2 for |@@, etc., and 1 for @|, 2 for @@|, etc.
    cursor_offset_pos: i32,
    anchor_start: bool,
    anchor_end: bool,
    // The segment number from 1..n of the next '(' we see
    // during parsing; 1-based.
    next_segment_number: i32,
}

impl RuleHalf {
    fn new() -> Self {
        Self {
            text: Vec::new(),
            cursor: -1,
            ante: -1,
            post: -1,
            cursor_offset: 0,
            cursor_offset_pos: 0,
            anchor_start: false,
            anchor_end: false,
            next_segment_number: 1,
        }
    }

    /// `parse(String rule, int pos, int limit, TransliteratorParser
    /// parser)`: parse one side of a rule, stopping at either the limit,
    /// the END_OF_RULE character, or an operator. Returns the index after
    /// the terminating character, or if limit was reached, limit.
    fn parse(
        &mut self,
        rule: &[u16],
        pos: i32,
        limit: i32,
        parser: &mut TransliteratorParser,
    ) -> Result<i32> {
        let start = pos;
        let mut buf = Vec::new();
        let pos = self.parse_section(rule, pos, limit, parser, &mut buf, &ILLEGAL_TOP, false)?;
        self.text = buf;

        if self.cursor_offset > 0 && self.cursor != self.cursor_offset_pos {
            return Err(syntax_error(
                &format!("Misplaced {}", CURSOR_POS as u8 as char),
                rule,
                start,
            ));
        }

        Ok(pos)
    }

    /// `parseSection(String rule, int pos, int limit, TransliteratorParser
    /// parser, StringBuffer buf, UnicodeSet illegal, boolean isSegment)`:
    /// parse a section of one side of a rule, stopping at either the limit,
    /// the END_OF_RULE character, an operator, or a segment close
    /// character. This method parses both a top-level rule half and a
    /// segment within such a rule half. It calls itself recursively to
    /// parse segments and nested segments.
    #[allow(clippy::too_many_arguments)]
    fn parse_section(
        &mut self,
        rule: &[u16],
        mut pos: i32,
        limit: i32,
        parser: &mut TransliteratorParser,
        buf: &mut Vec<u16>,
        illegal: &UnicodeSet,
        is_segment: bool,
    ) -> Result<i32> {
        let start = pos;
        let mut pp: Option<ParsePosition> = None;
        let mut quote_start = -1; // Most recent 'single quoted string'
        let mut quote_limit = -1;
        let mut var_start = -1; // Most recent $variableReference
        let mut var_limit = -1;
        let mut iref = [0i32; 1];
        let buf_start = buf.len() as i32;

        'main: while pos < limit {
            // Since all syntax characters are in the BMP, fetching
            // 16-bit code units suffices here.
            let mut c = char_at(rule, pos);
            pos += 1;
            if PatternProps::is_white_space(c as i32) {
                continue;
            }
            // HALF_ENDERS is all chars that end a rule half: "<>=;"
            if HALF_ENDERS.contains(&c) {
                // /CLOVER:OFF
                // isSegment is always false
                if is_segment {
                    return Err(syntax_error("Unclosed segment", rule, start));
                }
                // /CLOVER:ON
                break 'main;
            }
            if self.anchor_end {
                // Text after a presumed end anchor is a syntax err
                return Err(syntax_error("Malformed variable reference", rule, start));
            }
            if UnicodeSet::resembles_pattern(rule, pos - 1) {
                let p = pp.get_or_insert_with(|| ParsePosition::new(0));
                p.set_index(pos - 1); // Backup to opening '['
                buf.push(parser.parse_set(rule, p)?);
                pos = p.get_index();
                continue;
            }
            // Handle escapes
            if c == ESCAPE {
                if pos == limit {
                    return Err(syntax_error("Trailing backslash", rule, start));
                }
                let cp_and_length = Utility::unescape_and_length_at(rule, pos);
                if cp_and_length < 0 {
                    return Err(syntax_error("Malformed escape", rule, start));
                }
                let escaped = Utility::cp_from_code_point_and_length(cp_and_length);
                pos += Utility::length_from_code_point_and_length(cp_and_length);
                parser.check_variable_range(escaped, rule, start)?;
                UTF16::append(buf, escaped)?;
                continue;
            }
            // Handle quoted matter
            if c == QUOTE {
                let mut iq = index_of_unit(rule, QUOTE, pos);
                if iq == pos {
                    buf.push(c); // Parse [''] outside quotes as ['] iq == pos
                    pos += 1;
                } else {
                    // This loop picks up a run of quoted text of the
                    // form 'aaaa' each time through.  If this run
                    // hasn't really ended ('aaaa''bbbb') then it keeps
                    // looping, each time adding on a new run.  When it
                    // reaches the final quote it breaks.
                    quote_start = buf.len() as i32;
                    loop {
                        if iq < 0 {
                            return Err(syntax_error("Unterminated quote", rule, start));
                        }
                        buf.extend_from_slice(&substring(rule, pos, iq));
                        pos = iq + 1;
                        if pos < limit && char_at(rule, pos) == QUOTE {
                            // Parse [''] inside quotes as [']
                            iq = index_of_unit(rule, QUOTE, pos + 1);
                            // Continue looping
                        } else {
                            break;
                        }
                    }
                    quote_limit = buf.len() as i32;

                    for iq in quote_start..quote_limit {
                        parser.check_variable_range(buf[iq as usize] as i32, rule, start)?;
                    }
                }
                continue;
            }

            parser.check_variable_range(c as i32, rule, start)?;

            if illegal.contains_valid(c as i32) {
                return Err(syntax_error(
                    &format!("Illegal character '{}'", String::from_utf16_lossy(&[c])),
                    rule,
                    start,
                ));
            }

            match c {
                //------------------------------------------------------
                // Elements allowed within and out of segments
                //------------------------------------------------------
                ANCHOR_START => {
                    if buf.is_empty() && !self.anchor_start {
                        self.anchor_start = true;
                    } else {
                        return Err(syntax_error("Misplaced anchor start", rule, start));
                    }
                }
                SEGMENT_OPEN => {
                    // bufSegStart is the offset in buf to the first
                    // character of the segment we are parsing.
                    let buf_seg_start = buf.len();

                    // Record segment number now, since nextSegmentNumber
                    // will be incremented during the call to parseSection
                    // if there are nested segments.
                    let segment_number = self.next_segment_number;
                    self.next_segment_number += 1; // 1-based

                    // Parse the segment
                    pos = self.parse_section(rule, pos, limit, parser, buf, &ILLEGAL_SEG, true)?;

                    // After parsing a segment, the relevant characters are
                    // in buf, starting at offset bufSegStart.  Extract them
                    // into a string matcher, and replace them with a
                    // standin for that matcher.
                    let m = Arc::new(StringMatcher::new(
                        buf[buf_seg_start..].to_vec(),
                        segment_number,
                        parser.cur_data(),
                    ));

                    // Record and associate object and segment number
                    parser.set_segment_object(segment_number, m)?;
                    buf.truncate(buf_seg_start);
                    buf.push(parser.get_segment_standin(segment_number)?);
                }
                FUNCTION | ALT_FUNCTION => {
                    iref[0] = pos;
                    let single = TransliteratorIDParser::parse_filter_id(rule, &mut iref)?;
                    // The next character MUST be a segment open
                    let Some(single) =
                        single.filter(|_| Utility::parse_char(rule, &mut iref, SEGMENT_OPEN))
                    else {
                        return Err(syntax_error("Invalid function", rule, start));
                    };

                    let Some(t) = single.get_instance()? else {
                        return Err(syntax_error("Invalid function ID", rule, start));
                    };

                    // bufSegStart is the offset in buf to the first
                    // character of the segment we are parsing.
                    let buf_seg_start = buf.len();

                    // Parse the segment
                    pos =
                        self.parse_section(rule, iref[0], limit, parser, buf, &ILLEGAL_FUNC, true)?;

                    // After parsing a segment, the relevant characters are
                    // in buf, starting at offset bufSegStart.
                    let r = FunctionReplacer::new(
                        t,
                        Box::new(StringReplacer::new(
                            buf[buf_seg_start..].to_vec(),
                            parser.cur_data(),
                        )),
                    );

                    // Replace the buffer contents with a stand-in
                    buf.truncate(buf_seg_start);
                    buf.push(
                        parser.generate_stand_in_for(Variable::FunctionReplacer(Arc::new(r)))?,
                    );
                }
                SYMBOL_REF => {
                    // Handle variable references and segment references "$1" .. "$9"
                    // A variable reference must be followed immediately
                    // by a Unicode identifier start and zero or more
                    // Unicode identifier part characters, or by a digit
                    // 1..9 if it is a segment reference.
                    if pos == limit {
                        // A variable ref character at the end acts as
                        // an anchor to the context limit, as in perl.
                        self.anchor_end = true;
                    } else {
                        // Parse "$1" "$2" .. "$9" .. (no upper limit)
                        c = char_at(rule, pos);
                        let mut r = UCharacter::digit_radix(c as i32, 10);
                        if (1..=9).contains(&r) {
                            iref[0] = pos;
                            r = Utility::parse_number(rule, &mut iref, 10);
                            if r < 0 {
                                return Err(syntax_error(
                                    "Undefined segment reference",
                                    rule,
                                    start,
                                ));
                            }
                            pos = iref[0];
                            buf.push(parser.get_segment_standin(r)?);
                        } else {
                            let p = pp.get_or_insert_with(|| ParsePosition::new(0));
                            p.set_index(pos);
                            match ParseData::parse_reference_impl(rule, p, limit) {
                                None => {
                                    // This means the '$' was not followed by a
                                    // valid name.  Try to interpret it as an
                                    // end anchor then.  If this also doesn't work
                                    // (if we see a following character) then signal
                                    // an error.
                                    self.anchor_end = true;
                                }
                                Some(name) => {
                                    pos = p.get_index();
                                    // If this is a variable definition statement,
                                    // then the LHS variable will be undefined.  In
                                    // that case appendVariableDef() will append the
                                    // special placeholder char variableLimit-1.
                                    var_start = buf.len() as i32;
                                    parser.append_variable_def(&name, buf)?;
                                    var_limit = buf.len() as i32;
                                }
                            }
                        }
                    }
                }
                DOT => {
                    buf.push(parser.get_dot_stand_in()?);
                }
                KLEENE_STAR | ONE_OR_MORE | ZERO_OR_ONE => {
                    // Quantifiers.  We handle single characters, quoted strings,
                    // variable references, and segments.
                    //  a+      matches  aaa
                    //  'foo'+  matches  foofoofoo
                    //  $v+     matches  xyxyxy if $v == xy
                    //  (seg)+  matches  segsegseg
                    if is_segment && buf.len() as i32 == buf_start {
                        // The */+ immediately follows '('
                        return Err(syntax_error("Misplaced quantifier", rule, start));
                    }

                    let qstart;
                    let qlimit;
                    // The */+ follows an isolated character or quote
                    // or variable reference
                    if buf.len() as i32 == quote_limit {
                        // The */+ follows a 'quoted string'
                        qstart = quote_start;
                        qlimit = quote_limit;
                    } else if buf.len() as i32 == var_limit {
                        // The */+ follows a $variableReference
                        qstart = var_start;
                        qlimit = var_limit;
                    } else {
                        // The */+ follows a single character, possibly
                        // a segment standin
                        qstart = buf.len() as i32 - 1;
                        qlimit = qstart + 1;
                    }

                    // new StringMatcher(buf.toString(), qstart, qlimit, ...)
                    // catches the RuntimeException of substring: the
                    // range check reproduces it.
                    let m: Arc<dyn UnicodeMatcher> =
                        if qstart >= 0 && qstart <= qlimit && qlimit as usize <= buf.len() {
                            Arc::new(StringMatcher::new_range(
                                buf,
                                qstart,
                                qlimit,
                                0,
                                parser.cur_data(),
                            ))
                        } else {
                            let cause = LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
                                format!("begin {qstart}, end {qlimit}, length {}", buf.len()),
                            ));
                            let precontext = if pos < 50 {
                                to_rust_string(&substring(rule, 0, pos))
                            } else {
                                format!("...{}", to_rust_string(&substring(rule, pos - 50, pos)))
                            };
                            let post_context = if limit - pos <= 50 {
                                to_rust_string(&substring(rule, pos, limit))
                            } else {
                                format!("{}...", to_rust_string(&substring(rule, pos, pos + 50)))
                            };
                            return Err(IllegalIcuArgumentException::with_message_and_cause(
                                format!("Failure in rule: {precontext}$$${post_context}"),
                                cause,
                            )
                            .error());
                        };
                    let mut min = 0;
                    let mut max = Quantifier::MAX;
                    match c {
                        ONE_OR_MORE => {
                            min = 1;
                        }
                        ZERO_OR_ONE => {
                            min = 0;
                            max = 1;
                        }
                        // case KLEENE_STAR:
                        //    do nothing -- min, max already set
                        _ => {}
                    }
                    let m = Quantifier::new(m, min, max)?;
                    buf.truncate(qstart as usize);
                    buf.push(parser.generate_stand_in_for(Variable::Quantifier(Arc::new(m)))?);
                }

                //------------------------------------------------------
                // Elements allowed ONLY WITHIN segments
                //------------------------------------------------------
                SEGMENT_CLOSE => {
                    // assert(isSegment);
                    // We're done parsing a segment.
                    break 'main;
                }

                //------------------------------------------------------
                // Elements allowed ONLY OUTSIDE segments
                //------------------------------------------------------
                CONTEXT_ANTE => {
                    if self.ante >= 0 {
                        return Err(syntax_error("Multiple ante contexts", rule, start));
                    }
                    self.ante = buf.len() as i32;
                }
                CONTEXT_POST => {
                    if self.post >= 0 {
                        return Err(syntax_error("Multiple post contexts", rule, start));
                    }
                    self.post = buf.len() as i32;
                }
                CURSOR_POS => {
                    if self.cursor >= 0 {
                        return Err(syntax_error("Multiple cursors", rule, start));
                    }
                    self.cursor = buf.len() as i32;
                }
                CURSOR_OFFSET => {
                    if self.cursor_offset < 0 {
                        if !buf.is_empty() {
                            return Err(syntax_error(
                                &format!("Misplaced {}", c as u8 as char),
                                rule,
                                start,
                            ));
                        }
                        self.cursor_offset -= 1;
                    } else if self.cursor_offset > 0 {
                        if buf.len() as i32 != self.cursor_offset_pos || self.cursor >= 0 {
                            return Err(syntax_error(
                                &format!("Misplaced {}", c as u8 as char),
                                rule,
                                start,
                            ));
                        }
                        self.cursor_offset += 1;
                    } else if self.cursor == 0 && buf.is_empty() {
                        self.cursor_offset = -1;
                    } else if self.cursor < 0 {
                        self.cursor_offset_pos = buf.len() as i32;
                        self.cursor_offset = 1;
                    } else {
                        return Err(syntax_error(
                            &format!("Misplaced {}", c as u8 as char),
                            rule,
                            start,
                        ));
                    }
                }

                //------------------------------------------------------
                // Non-special characters
                //------------------------------------------------------
                _ => {
                    // Disallow unquoted characters other than [0-9A-Za-z]
                    // in the printable ASCII range.  These characters are
                    // reserved for possible future use.
                    if (0x0021..=0x007E).contains(&c)
                        && !((b'0' as u16..=b'9' as u16).contains(&c)
                            || (b'A' as u16..=b'Z' as u16).contains(&c)
                            || (b'a' as u16..=b'z' as u16).contains(&c))
                    {
                        return Err(syntax_error(
                            &format!("Unquoted {}", c as u8 as char),
                            rule,
                            start,
                        ));
                    }
                    buf.push(c);
                }
            }
        }
        Ok(pos)
    }

    /// `removeContext()`: remove context.
    fn remove_context(&mut self) {
        //text = text.substring(ante < 0 ? 0 : ante,
        //                      post < 0 ? text.length() : post);
        self.text = substring(
            &self.text,
            if self.ante < 0 { 0 } else { self.ante },
            if self.post < 0 {
                self.text.len() as i32
            } else {
                self.post
            },
        );
        self.ante = -1;
        self.post = -1;
        self.anchor_start = false;
        self.anchor_end = false;
    }

    /// `isValidOutput(TransliteratorParser parser)`: return true if this
    /// half looks like valid output, that is, does not contain quantifiers
    /// or other special input-only elements.
    fn is_valid_output(&self, parser: &TransliteratorParser) -> bool {
        let parse_data = parser.parse_data();
        let mut i = 0;
        while i < self.text.len() as i32 {
            let c = UTF16::char_at(&self.text, i);
            i += UTF16::get_char_count(c);
            if !parse_data.is_replacer(c) {
                return false;
            }
        }
        true
    }

    /// `isValidInput(TransliteratorParser parser)`: return true if this
    /// half looks like valid input, that is, does not contain functions or
    /// other special output-only elements.
    fn is_valid_input(&self, parser: &TransliteratorParser) -> bool {
        let parse_data = parser.parse_data();
        let mut i = 0;
        while i < self.text.len() as i32 {
            let c = UTF16::char_at(&self.text, i);
            i += UTF16::get_char_count(c);
            if !parse_data.is_matcher(c) {
                return false;
            }
        }
        true
    }
}

/// The parser of transliteration rules: compiles rule source into
/// `RuleBasedTransliterator.Data` objects and ID blocks.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.TransliteratorParser`.
pub(crate) struct TransliteratorParser {
    //----------------------------------------------------------------------
    // Data members
    //----------------------------------------------------------------------
    /// PUBLIC data member. A Vector of `RuleBasedTransliterator.Data`
    /// objects, one for each discrete group of rules in the rule set.
    pub data_vector: Vec<Arc<Data>>,

    /// PUBLIC data member. A Vector of Strings containing all of the ID
    /// blocks in the rule set.
    pub id_block_vector: Vec<Vec<u16>>,

    /// The current data object for which we are parsing rules.
    cur_data: Option<Arc<Data>>,

    /// PUBLIC data member containing the parsed compound filter, if any.
    pub compound_filter: Option<UnicodeSet>,

    direction: i32,

    /// Temporary vector of set variables. When parsing is complete, this is
    /// copied into the array data.variables. As with data.variables,
    /// element 0 corresponds to character data.variablesBase.
    variables_vector: Vec<Variable>,

    /// Temporary table of variable names. When parsing is complete, this is
    /// copied into data.variableNames.
    variable_names: HashMap<Vec<u16>, Vec<u16>>,

    /// String of standins for segments. Used during the parsing of a single
    /// rule. `segmentStandins.charAt(0)` is the standin for "$1" and
    /// corresponds to StringMatcher object `segmentObjects.elementAt(0)`,
    /// etc.
    segment_standins: Vec<u16>,

    /// Vector of StringMatcher objects for segments. Used during the
    /// parsing of a single rule. `segmentStandins.charAt(0)` is the standin
    /// for "$1" and corresponds to StringMatcher object
    /// `segmentObjects.elementAt(0)`, etc.
    segment_objects: Vec<Option<Arc<StringMatcher>>>,

    /// The next available stand-in for variables. This starts at some point
    /// in the private use area (discovered dynamically) and increments up
    /// toward `variableLimit`. At any point during parsing, available
    /// variables are `variableNext..variableLimit-1`.
    variable_next: u16,

    /// The last available stand-in for variables. This is discovered
    /// dynamically. At any point during parsing, available variables are
    /// `variableNext..variableLimit-1`. During variable definition we use
    /// the special value `variableLimit-1` as a placeholder.
    variable_limit: u16,

    /// When we encounter an undefined variable, we do not immediately
    /// signal an error, in case we are defining this variable, e.g.,
    /// "$a = [a-z];". Instead, we save the name of the undefined variable,
    /// and substitute in the placeholder char `variableLimit - 1`, and
    /// decrement `variableLimit`.
    undefined_variable_name: Option<Vec<u16>>,

    /// The stand-in character for the 'dot' set, represented by '.' in
    /// patterns. This is allocated the first time it is needed, and reused
    /// thereafter.
    dot_stand_in: i32,

    /// The data objects created by this parse, in creation order (see
    /// [`Data`]'s divergence note).
    all_data: Vec<Arc<Data>>,
}

impl Default for TransliteratorParser {
    fn default() -> Self {
        Self::new()
    }
}

/// `syntaxError(String msg, String rule, int start)`: the
/// `IllegalIcuArgumentException` reporting a syntax error: the message,
/// then the escaped rule from `start` to its end.
fn syntax_error(msg: &str, rule: &[u16], start: i32) -> LuceneError {
    let end = TransliteratorParser::rule_end(rule, start, rule.len() as i32);
    IllegalIcuArgumentException::with_message(format!(
        "{msg} in \"{}\"",
        to_rust_string(&Utility::escape_string(&substring(rule, start, end)))
    ))
    .error()
}

/// `previous.initCause(cause)` on the root cause of `error` (the last
/// exception of its cause chain): `error` rebuilt with `cause` appended to
/// its chain. The parser collects only `IllegalArgumentException`s, whose
/// kinds are rebuilt here; any other kind is returned unchanged.
fn init_root_cause(error: LuceneError, cause: LuceneError) -> LuceneError {
    fn chain(existing: Option<&Arc<LuceneError>>, cause: LuceneError) -> LuceneError {
        match existing {
            None => cause,
            Some(c) => init_root_cause(c.duplicate(), cause),
        }
    }
    if let Some(icu) = IllegalIcuArgumentException::of(&error) {
        use rucene::error::JavaThrowable;
        let message = icu.get_message().unwrap_or_default().to_string();
        let new_cause = chain(icu.get_cause(), cause);
        return IllegalIcuArgumentException::with_message_and_cause(message, new_cause).error();
    }
    match error.kind() {
        LuceneErrorKind::IllegalArgument {
            message,
            cause: existing,
        } => {
            let new_cause = chain(existing.as_ref(), cause);
            LuceneError::illegal_argument_with_cause(message.clone(), Arc::new(new_cause))
        }
        LuceneErrorKind::Unchecked {
            class,
            message,
            cause: existing,
        } => {
            let new_cause = chain(existing.as_ref(), cause);
            LuceneError::from(LuceneErrorKind::Unchecked {
                class,
                message: message.clone(),
                cause: Some(Arc::new(new_cause)),
            })
        }
        _ => error,
    }
}

impl TransliteratorParser {
    //----------------------------------------------------------------------
    // Top-level methods
    //----------------------------------------------------------------------

    /// `TransliteratorParser()`: constructor.
    pub fn new() -> Self {
        Self {
            data_vector: Vec::new(),
            id_block_vector: Vec::new(),
            cur_data: None,
            compound_filter: None,
            direction: FORWARD,
            variables_vector: Vec::new(),
            variable_names: HashMap::new(),
            segment_standins: Vec::new(),
            segment_objects: Vec::new(),
            variable_next: 0,
            variable_limit: 0,
            undefined_variable_name: None,
            dot_stand_in: -1,
            all_data: Vec::new(),
        }
    }

    /// `parse(String rules, int dir)`: parse a set of rules. After the
    /// parse completes, examine the public data members for results.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`IllegalIcuArgumentException`) for
    /// syntax errors, the first one with the following ones as its causes;
    /// `RuntimeException` when the variable range is exhausted.
    pub fn parse(&mut self, rules: &[u16], dir: i32) -> Result<()> {
        self.parse_rules(&mut RuleArray::new(vec![rules.to_vec()]), dir)
    }

    /// The symbol table over the current parse state.
    fn parse_data(&self) -> ParseData<'_> {
        ParseData {
            variable_names: &self.variable_names,
            variables_vector: &self.variables_vector,
            variables_base: self.cur_data.as_ref().map(|d| d.variables_base()),
        }
    }

    /// `curData`.
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` outside a rule section (does not
    /// occur: rules are only parsed inside one).
    fn cur_data(&self) -> &Arc<Data> {
        self.cur_data.as_ref().unwrap_or_else(|| {
            panic!(
                "java.lang.NullPointerException: Cannot read field \"curData\" because it is null"
            )
        })
    }

    //----------------------------------------------------------------------
    // Private methods
    //----------------------------------------------------------------------

    /// `parseRules(RuleBody ruleArray, int dir)`: parse an array of zero or
    /// more rules. The strings in the array are treated as if they were
    /// concatenated together, with rule terminators inserted between array
    /// elements if not present already.
    ///
    /// Any previous rules are discarded. Typically this method is called
    /// exactly once, during construction.
    ///
    /// The member this.data will be set to null if there are no rules.
    fn parse_rules(&mut self, rule_array: &mut dyn RuleBody, dir: i32) -> Result<()> {
        let mut parsing_ids = true;
        let mut rule_count = 0;

        self.data_vector = Vec::new();
        self.id_block_vector = Vec::new();
        self.cur_data = None;
        self.direction = dir;
        self.compound_filter = None;
        self.variables_vector = Vec::new();
        self.variable_names = HashMap::new();
        self.all_data = Vec::new();

        let mut errors: Vec<LuceneError> = Vec::new();
        let mut error_count = 0;

        rule_array.reset();

        let mut id_block_result: Vec<u16> = Vec::new();

        // The compound filter offset is an index into idBlockResult.
        // If it is 0, then the compound filter occurred at the start,
        // and it is the offset to the _start_ of the compound filter
        // pattern.  Otherwise it is the offset to the _limit_ of the
        // compound filter pattern within idBlockResult.
        self.compound_filter = None;
        let mut compound_filter_offset = -1;

        'main: while let Some(rule) = rule_array.next_line() {
            let mut pos = 0i32;
            let limit = rule.len() as i32;
            while pos < limit {
                let c = char_at(&rule, pos);
                pos += 1;
                if PatternProps::is_white_space(c as i32) {
                    continue;
                }
                // Skip lines starting with the comment character
                if c == RULE_COMMENT_CHAR {
                    pos = index_of(&rule, &[b'\n' as u16], pos) + 1;
                    if pos == 0 {
                        break; // No "\n" found; rest of rule is a comment
                    }
                    continue; // Either fall out or restart with next line
                }

                // skip empty rules
                if c == END_OF_RULE {
                    continue;
                }

                // Often a rule file contains multiple errors.  It's
                // convenient to the rule author if these are all reported
                // at once.  We keep parsing rules even after a failure, up
                // to a specified limit, and report all errors at once.
                let result = (|| -> Result<()> {
                    rule_count += 1;

                    // We've found the start of a rule or ID.  c is its first
                    // character, and pos points past c.
                    pos -= 1;
                    // Look for an ID token.  Must have at least ID_TOKEN_LEN + 1
                    // chars left.
                    if (pos + ID_TOKEN_LEN + 1) <= limit
                        && rule[pos as usize..(pos + ID_TOKEN_LEN) as usize] == *ID_TOKEN
                    {
                        pos += ID_TOKEN_LEN;
                        let mut c = char_at(&rule, pos);
                        while PatternProps::is_white_space(c as i32) && pos < limit {
                            pos += 1;
                            c = char_at(&rule, pos);
                        }
                        let mut p = [pos];

                        if !parsing_ids {
                            if let Some(cur_data) = self.cur_data.take() {
                                if self.direction == FORWARD {
                                    self.data_vector.push(cur_data);
                                } else {
                                    self.data_vector.insert(0, cur_data);
                                }
                            }
                            parsing_ids = true;
                        }

                        let id =
                            TransliteratorIDParser::parse_single_id(&rule, &mut p, self.direction)?;
                        if p[0] != pos && Utility::parse_char(&rule, &mut p, END_OF_RULE) {
                            let id = id.expect("a single ID was parsed");
                            if self.direction == FORWARD {
                                id_block_result.extend_from_slice(&id.canon_id);
                                id_block_result.push(END_OF_RULE);
                            } else {
                                let mut s = id.canon_id.clone();
                                s.push(END_OF_RULE);
                                id_block_result.splice(0..0, s);
                            }
                        } else {
                            // Couldn't parse an ID.  Try to parse a global filter
                            let mut with_parens = [-1i32];
                            let f = TransliteratorIDParser::parse_global_filter(
                                &rule,
                                &mut p,
                                self.direction,
                                &mut with_parens,
                                None,
                            )?;
                            if f.is_some() && Utility::parse_char(&rule, &mut p, END_OF_RULE) {
                                if (self.direction == FORWARD) == (with_parens[0] == 0) {
                                    if self.compound_filter.is_some() {
                                        // Multiple compound filters
                                        return Err(syntax_error(
                                            "Multiple global filters",
                                            &rule,
                                            pos,
                                        ));
                                    }
                                    self.compound_filter = f;
                                    compound_filter_offset = rule_count;
                                }
                            } else {
                                // Invalid ::id
                                // Can be parsed as neither an ID nor a global filter
                                return Err(syntax_error("Invalid ::ID", &rule, pos));
                            }
                        }

                        pos = p[0];
                    } else {
                        if parsing_ids {
                            if self.direction == FORWARD {
                                self.id_block_vector.push(id_block_result.clone());
                            } else {
                                self.id_block_vector.insert(0, id_block_result.clone());
                            }
                            id_block_result.clear();
                            parsing_ids = false;
                            let cur_data = Arc::new(Data::new_after(self.all_data.clone()));
                            self.all_data.push(cur_data.clone());
                            self.cur_data = Some(cur_data);

                            // By default, rules use part of the private use area
                            // E000..F8FF for variables and other stand-ins.  Currently
                            // the range F000..F8FF is typically sufficient.  The 'use
                            // variable range' pragma allows rule sets to modify this.
                            self.set_variable_range(0xF000, 0xF8FF)?;
                        }

                        if Self::resembles_pragma(&rule, pos, limit) {
                            let ppp = self.parse_pragma(&rule, pos, limit)?;
                            if ppp < 0 {
                                return Err(syntax_error("Unrecognized pragma", &rule, pos));
                            }
                            pos = ppp;
                            // Parse a rule
                        } else {
                            pos = self.parse_rule(&rule, pos, limit)?;
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    // catch (IllegalArgumentException e)
                    if !e.is_illegal_argument_exception() {
                        return Err(e);
                    }
                    if error_count == 30 {
                        errors.push(
                            IllegalIcuArgumentException::with_message_and_cause(
                                "\nMore than 30 errors; further messages squelched",
                                e,
                            )
                            .error(),
                        );
                        break 'main;
                    }
                    errors.push(e);
                    error_count += 1;
                    pos = Self::rule_end(&rule, pos, limit) + 1; // +1 advances past ';'
                }
            }
        }
        if parsing_ids && !id_block_result.is_empty() {
            if self.direction == FORWARD {
                self.id_block_vector.push(id_block_result);
            } else {
                self.id_block_vector.insert(0, id_block_result);
            }
        } else if !parsing_ids {
            if let Some(cur_data) = self.cur_data.clone() {
                if self.direction == FORWARD {
                    self.data_vector.push(cur_data);
                } else {
                    self.data_vector.insert(0, cur_data);
                }
            }
        }

        // Convert the set vector to an array
        for data in &self.data_vector {
            data.set_variables(self.variables_vector.clone());
            data.variable_names().extend(
                self.variable_names
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
        }
        self.variables_vector = Vec::new();

        // Do more syntax checking and index the rules
        let result = (|| -> Result<()> {
            if self.compound_filter.is_some()
                && ((self.direction == FORWARD && compound_filter_offset != 1)
                    || (self.direction == REVERSE && compound_filter_offset != rule_count))
            {
                return Err(IllegalIcuArgumentException::with_message(
                    "Compound filters misplaced",
                )
                .error());
            }

            for data in &self.data_vector {
                data.rule_set().freeze()?;
            }

            if self.id_block_vector.len() == 1 && self.id_block_vector[0].is_empty() {
                self.id_block_vector.remove(0);
            }
            Ok(())
        })();
        if let Err(e) = result {
            if !e.is_illegal_argument_exception() {
                return Err(e);
            }
            errors.push(e);
        }

        if !errors.is_empty() {
            // for (int i = errors.size()-1; i > 0; --i) {
            //     RuntimeException previous = errors.get(i-1);
            //     while (previous.getCause() != null) {
            //         previous = (RuntimeException) previous.getCause(); // chain specially
            //     }
            //     previous.initCause(errors.get(i));
            // }
            let mut chained = errors.pop().expect("not empty");
            while let Some(previous) = errors.pop() {
                chained = init_root_cause(previous, chained);
            }
            return Err(chained);
            // if initCause not supported: throw new IllegalArgumentException(errors.toString());
        }
        Ok(())
    }

    /// `parseRule(String rule, int pos, int limit)`: MAIN PARSER. Parse the
    /// next rule in the given rule string, starting at pos. Return the
    /// index after the last character parsed. Do not parse characters at
    /// or after limit.
    ///
    /// Important: The character at pos must be a non-whitespace character
    /// that is not the comment character.
    ///
    /// This method handles quoting, escaping, and whitespace removal. It
    /// parses the end-of-rule character. It recognizes context and cursor
    /// indicators. Once it does a lexical breakdown of the rule at pos, it
    /// creates a rule object and adds it to our rule list.
    ///
    /// This method is tightly coupled to the inner class RuleHalf.
    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    fn parse_rule(&mut self, rule: &[u16], mut pos: i32, limit: i32) -> Result<i32> {
        let start = pos;
        let mut operator: u16 = 0;

        // Set up segments data
        self.segment_standins = Vec::new();
        self.segment_objects = Vec::new();

        let mut left = RuleHalf::new();
        let mut right = RuleHalf::new();

        self.undefined_variable_name = None;
        pos = left.parse(rule, pos, limit, self)?;

        if pos == limit || {
            pos -= 1;
            operator = char_at(rule, pos);
            !OPERATORS.contains(&operator)
        } {
            return Err(syntax_error(&format!("No operator pos={pos}"), rule, start));
        }
        pos += 1;

        // Found an operator char.  Check for forward-reverse operator.
        if operator == REVERSE_RULE_OP && (pos < limit && char_at(rule, pos) == FORWARD_RULE_OP) {
            pos += 1;
            operator = FWDREV_RULE_OP;
        }

        // Translate alternate op characters.
        match operator {
            ALT_FORWARD_RULE_OP => operator = FORWARD_RULE_OP,
            ALT_REVERSE_RULE_OP => operator = REVERSE_RULE_OP,
            ALT_FWDREV_RULE_OP => operator = FWDREV_RULE_OP,
            _ => {}
        }

        pos = right.parse(rule, pos, limit, self)?;

        if pos < limit {
            pos -= 1;
            if char_at(rule, pos) == END_OF_RULE {
                pos += 1;
            } else {
                // RuleHalf parser must have terminated at an operator
                return Err(syntax_error("Unquoted operator", rule, start));
            }
        }

        if operator == VARIABLE_DEF_OP {
            // LHS is the name.  RHS is a single character, either a literal
            // or a set (already parsed).  If RHS is longer than one
            // character, it is either a multi-character string, or multiple
            // sets, or a mixture of chars and sets -- syntax error.

            // We expect to see a single undefined variable (the one being
            // defined).
            let Some(undefined) = self.undefined_variable_name.clone() else {
                return Err(syntax_error(
                    "Missing '$' or duplicate definition",
                    rule,
                    start,
                ));
            };
            if left.text.len() != 1 || left.text[0] != self.variable_limit {
                return Err(syntax_error("Malformed LHS", rule, start));
            }
            if left.anchor_start || left.anchor_end || right.anchor_start || right.anchor_end {
                return Err(syntax_error("Malformed variable def", rule, start));
            }
            // We allow anything on the right, including an empty string.
            let value = right.text.clone();
            self.variable_names.insert(undefined, value);

            self.variable_limit = self.variable_limit.wrapping_add(1);
            return Ok(pos);
        }

        // If this is not a variable definition rule, we shouldn't have
        // any undefined variable names.
        if let Some(name) = &self.undefined_variable_name {
            return Err(syntax_error(
                &format!("Undefined variable ${}", to_rust_string(name)),
                rule,
                start,
            ));
        }

        // Verify segments
        if self.segment_standins.len() > self.segment_objects.len() {
            return Err(syntax_error("Undefined segment reference", rule, start));
        }
        for &c in &self.segment_standins {
            if c == 0 {
                return Err(syntax_error("Internal error", rule, start)); // will never happen
            }
        }
        for o in &self.segment_objects {
            if o.is_none() {
                return Err(syntax_error("Internal error", rule, start)); // will never happen
            }
        }

        // If the direction we want doesn't match the rule
        // direction, do nothing.
        if operator != FWDREV_RULE_OP
            && ((self.direction == FORWARD) != (operator == FORWARD_RULE_OP))
        {
            return Ok(pos);
        }

        // Transform the rule into a forward rule by swapping the
        // sides if necessary.
        if self.direction == REVERSE {
            std::mem::swap(&mut left, &mut right);
        }

        // Remove non-applicable elements in forward-reverse
        // rules.  Bidirectional rules ignore elements that do not
        // apply.
        if operator == FWDREV_RULE_OP {
            right.remove_context();
            left.cursor = -1;
            left.cursor_offset = 0;
        }

        // Normalize context
        if left.ante < 0 {
            left.ante = 0;
        }
        if left.post < 0 {
            left.post = left.text.len() as i32;
        }

        // Context is only allowed on the input side.  Cursors are only
        // allowed on the output side.  Segment delimiters can only appear
        // on the left, and references on the right.  Cursor offset
        // cannot appear without an explicit cursor.  Cursor offset
        // cannot place the cursor outside the limits of the context.
        // Anchors are only allowed on the input side.
        if right.ante >= 0
            || right.post >= 0
            || left.cursor >= 0
            || (right.cursor_offset != 0 && right.cursor < 0)
            // - The following two checks were used to ensure that the
            // - the cursor offset stayed within the ante- or postcontext.
            // - However, with the addition of quantifiers, we have to
            // - allow arbitrary cursor offsets and do runtime checking.
            //(right.cursorOffset > (left.text.length() - left.post)) ||
            //(-right.cursorOffset > left.ante) ||
            || right.anchor_start
            || right.anchor_end
            || !left.is_valid_input(self)
            || !right.is_valid_output(self)
            || left.ante > left.post
        {
            return Err(syntax_error("Malformed rule", rule, start));
        }

        // Flatten segment objects vector to an array
        let segments_array = if !self.segment_objects.is_empty() {
            Some(
                self.segment_objects
                    .iter()
                    .map(|o| o.clone().expect("checked above"))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };

        let cur_data = self.cur_data().clone();
        let r = TransliterationRule::new(
            left.text,
            left.ante,
            left.post,
            &right.text,
            right.cursor,
            right.cursor_offset,
            segments_array,
            left.anchor_start,
            left.anchor_end,
            &cur_data,
        )?;
        cur_data.rule_set().add_rule(r);

        Ok(pos)
    }

    /// `setVariableRange(int start, int end)`: set the variable range to
    /// `[start, end]` (inclusive).
    fn set_variable_range(&mut self, start: i32, end: i32) -> Result<()> {
        if start > end || start < 0 || end > 0xFFFF {
            return Err(IllegalIcuArgumentException::with_message(format!(
                "Invalid variable range {start}, {end}"
            ))
            .error());
        }

        self.cur_data().set_variables_base(start as u16);

        if self.data_vector.is_empty() {
            self.variable_next = start as u16;
            self.variable_limit = (end + 1) as u16;
        }
        Ok(())
    }

    /// `checkVariableRange(int ch, String rule, int start)`: assert that the
    /// given character is NOT within the variable range. If it is, signal
    /// an error. This is necessary to ensure that the variable range does
    /// not overlap characters used in a rule.
    fn check_variable_range(&self, ch: i32, rule: &[u16], start: i32) -> Result<()> {
        if ch >= self.cur_data().variables_base() as i32 && ch < self.variable_limit as i32 {
            return Err(syntax_error(
                "Variable range character in rule",
                rule,
                start,
            ));
        }
        Ok(())
    }

    // (The following method is part of an unimplemented feature.
    // Remove this clover pragma after the feature is implemented.
    // 2003-06-11 ICU 2.6 Alan)
    //CLOVER:OFF
    /// `pragmaMaximumBackup(int backup)`: set the maximum backup to
    /// 'backup', in response to a pragma statement.
    fn pragma_maximum_backup(&self, _backup: i32) -> Result<()> {
        //TODO Finish
        Err(IllegalIcuArgumentException::with_message(
            "use maximum backup pragma not implemented yet",
        )
        .error())
    }
    //CLOVER:ON

    // (The following method is part of an unimplemented feature.
    // Remove this clover pragma after the feature is implemented.
    // 2003-06-11 ICU 2.6 Alan)
    //CLOVER:OFF
    /// `pragmaNormalizeRules(Normalizer.Mode mode)`: begin normalizing all
    /// rules using the given mode, in response to a pragma statement.
    fn pragma_normalize_rules(&self, _mode: Mode) -> Result<()> {
        //TODO Finish
        Err(IllegalIcuArgumentException::with_message(
            "use normalize rules pragma not implemented yet",
        )
        .error())
    }
    //CLOVER:ON

    /// `resemblesPragma(String rule, int pos, int limit)`: return true if
    /// the given rule looks like a pragma.
    fn resembles_pragma(rule: &[u16], pos: i32, limit: i32) -> bool {
        // Must start with /use\s/i
        Utility::parse_pattern(rule, pos, limit, &units("use "), &mut []) >= 0
    }

    /// `parsePragma(String rule, int pos, int limit)`: parse a pragma. This
    /// method assumes `resembles_pragma()` has already returned true.
    /// Returns the position immediately after the last character parsed,
    /// or -1 if the pragma is not recognized.
    fn parse_pragma(&mut self, rule: &[u16], mut pos: i32, limit: i32) -> Result<i32> {
        let mut array = [0i32; 2];

        // resemblesPragma() has already returned true, so we
        // know that pos points to /use\s/i; we can skip 4 characters
        // immediately
        pos += 4;

        // Here are the pragmas we recognize:
        // use variable range 0xE000 0xEFFF;
        // use maximum backup 16;
        // use nfd rules;
        let p = Utility::parse_pattern(
            rule,
            pos,
            limit,
            &units("~variable range # #~;"),
            &mut array,
        );
        if p >= 0 {
            self.set_variable_range(array[0], array[1])?;
            return Ok(p);
        }

        let p = Utility::parse_pattern(rule, pos, limit, &units("~maximum backup #~;"), &mut array);
        if p >= 0 {
            self.pragma_maximum_backup(array[0])?;
            return Ok(p);
        }

        let p = Utility::parse_pattern(rule, pos, limit, &units("~nfd rules~;"), &mut []);
        if p >= 0 {
            self.pragma_normalize_rules(Normalizer::NFD)?;
            return Ok(p);
        }

        let p = Utility::parse_pattern(rule, pos, limit, &units("~nfc rules~;"), &mut []);
        if p >= 0 {
            self.pragma_normalize_rules(Normalizer::NFC)?;
            return Ok(p);
        }

        // Syntax error: unable to parse pragma
        Ok(-1)
    }

    /// `ruleEnd(String rule, int start, int limit)`: locate the end of the
    /// rule: the next unquoted `;`, or `limit`.
    fn rule_end(rule: &[u16], start: i32, limit: i32) -> i32 {
        let mut end = Utility::quoted_index_of(rule, start, limit, &[b';' as u16]);
        if end < 0 {
            end = limit;
        }
        end
    }

    /// `parseSet(String rule, ParsePosition pos)`: parse a UnicodeSet out,
    /// store it, and return the stand-in character used to represent it.
    fn parse_set(&mut self, rule: &[u16], pos: &mut ParsePosition) -> Result<u16> {
        let mut set = UnicodeSet::from_pattern_pos(rule, pos, Some(&self.parse_data()))?;
        if self.variable_next >= self.variable_limit {
            return Err(crate::jdk::runtime_exception(Some(
                "Private use variables exhausted".to_string(),
            )));
        }
        set.compact()?;
        self.generate_stand_in_for(Variable::Set(Arc::new(set)))
    }

    /// `generateStandInFor(Object obj)`: generate and return a stand-in for
    /// a new UnicodeMatcher or UnicodeReplacer. Store the object
    /// internally.
    fn generate_stand_in_for(&mut self, obj: Variable) -> Result<u16> {
        // assert(obj != null);

        // Look up previous stand-in, if any.  This is a short list
        // (typical n is 0, 1, or 2); linear search is optimal.
        for (i, v) in self.variables_vector.iter().enumerate() {
            if v.same_object(&obj) {
                // [sic] pointer comparison
                return Ok(self.cur_data().variables_base().wrapping_add(i as u16));
            }
        }

        if self.variable_next >= self.variable_limit {
            return Err(crate::jdk::runtime_exception(Some(
                "Variable range exhausted".to_string(),
            )));
        }
        self.variables_vector.push(obj);
        let c = self.variable_next;
        self.variable_next = self.variable_next.wrapping_add(1);
        Ok(c)
    }

    /// `getSegmentStandin(int seg)`: return the standin for segment seg
    /// (1-based).
    fn get_segment_standin(&mut self, seg: i32) -> Result<u16> {
        if (self.segment_standins.len() as i32) < seg {
            self.segment_standins.resize(seg as usize, 0);
        }
        let mut c = char_at(&self.segment_standins, seg - 1);
        if c == 0 {
            if self.variable_next >= self.variable_limit {
                return Err(crate::jdk::runtime_exception(Some(
                    "Variable range exhausted".to_string(),
                )));
            }
            c = self.variable_next;
            self.variable_next = self.variable_next.wrapping_add(1);
            // Set a placeholder in the primary variables vector that will be
            // filled in later by setSegmentObject().  We know that we will get
            // called first because setSegmentObject() will call us.
            self.variables_vector.push(Variable::Null);
            self.segment_standins[(seg - 1) as usize] = c;
        }
        Ok(c)
    }

    /// `setSegmentObject(int seg, StringMatcher obj)`: set the object for
    /// segment seg (1-based).
    fn set_segment_object(&mut self, seg: i32, obj: Arc<StringMatcher>) -> Result<()> {
        // Since we call parseSection() recursively, nested
        // segments will result in segment i+1 getting parsed
        // and stored before segment i; be careful with the
        // vector handling here.
        while (self.segment_objects.len() as i32) < seg {
            self.segment_objects.push(None);
        }
        let index = self.get_segment_standin(seg)? as i32 - self.cur_data().variables_base() as i32;
        if self.segment_objects[(seg - 1) as usize].is_some()
            || !matches!(self.variables_vector[index as usize], Variable::Null)
        {
            return Err(crate::jdk::runtime_exception(None)); // should never happen
        }
        self.segment_objects[(seg - 1) as usize] = Some(obj.clone());
        self.variables_vector[index as usize] = Variable::StringMatcher(obj);
        Ok(())
    }

    /// `getDotStandIn()`: return the stand-in for the dot set. It is
    /// allocated the first time and reused thereafter.
    fn get_dot_stand_in(&mut self) -> Result<u16> {
        if self.dot_stand_in == -1 {
            let set = UnicodeSet::from_pattern(&units(DOT_SET))?;
            self.dot_stand_in = self.generate_stand_in_for(Variable::Set(Arc::new(set)))? as i32;
        }
        Ok(self.dot_stand_in as u16)
    }

    /// `appendVariableDef(String name, StringBuffer buf)`: append the value
    /// of the given variable name to the given StringBuffer.
    ///
    /// # Errors
    ///
    /// `IllegalIcuArgumentException` if the name is unknown and another
    /// undefined name was already seen.
    fn append_variable_def(&mut self, name: &[u16], buf: &mut Vec<u16>) -> Result<()> {
        match self.variable_names.get(name) {
            None => {
                // We allow one undefined variable so that variable definition
                // statements work.  For the first undefined variable we return
                // the special placeholder variableLimit-1, and save the variable
                // name.
                if self.undefined_variable_name.is_none() {
                    self.undefined_variable_name = Some(name.to_vec());
                    if self.variable_next >= self.variable_limit {
                        return Err(crate::jdk::runtime_exception(Some(
                            "Private use variables exhausted".to_string(),
                        )));
                    }
                    self.variable_limit = self.variable_limit.wrapping_sub(1);
                    buf.push(self.variable_limit);
                } else {
                    return Err(IllegalIcuArgumentException::with_message(format!(
                        "Undefined variable ${}",
                        to_rust_string(name)
                    ))
                    .error());
                }
            }
            Some(ch) => buf.extend_from_slice(ch),
        }
        Ok(())
    }
}
