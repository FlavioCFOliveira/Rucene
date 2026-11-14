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

//! A CharFilter that wraps another Reader and attempts to strip out HTML
//! constructs; ported from
//! `org.apache.lucene.analysis.charfilter.HTMLStripCharFilter`.
//!
//! Lucene 10.5.0 generates the Java class with JFlex 1.8.2 from
//! `HTMLStripCharFilter.jflex` (its character entity names come from
//! `HTMLCharacterEntities.jflex`). `tools/translate/jflex_java_to_rust.py`
//! translates the generated parts: the DFA tables (into
//! `html_strip_char_filter_tables.rs`) and the JFlex skeleton with the
//! actions made of simple statements (into the `@generated` regions of this
//! file). The user code of the grammar, the `%eof{ ... %}` code and the other
//! actions (`action_<N>`, each the body of the Java `case N:`) are translated
//! by hand, outside the regions.
//!
//! The JFlex buffer holds the UTF-16 code units the input
//! [`CharReader`] hands out, as Java's `char[]` holds those of a
//! `java.io.Reader`; the filter outputs UTF-16 code units, and the offsets
//! count them, as Java's do.

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::sync::LazyLock;

use rucene::analysis::factories::java_equals_ignore_case;
use rucene::analysis::{CharArrayMap, CharArraySet, CharFilter, CharReader};
use rucene::error::Result;

use super::base_char_filter::BaseCharFilter;
use crate::analysis::util::open_string_builder::OpenStringBuilder;

// @generated begin constants by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
// From Lucene 10.5.0's lucene/analysis/common/src/java/org/apache/lucene/analysis/charfilter/HTMLStripCharFilter.java
// (sha256 3994c43178169b93510b399376d163fdf2c12c2fa6d96e23454f24a6d9416236).

/// This character denotes the end of file.
///
/// Equivalent to `HTMLStripCharFilter.YYEOF`.
const YYEOF: i32 = -1;

/// Initial size of the lookahead buffer.
///
/// Equivalent to `HTMLStripCharFilter.ZZ_BUFFERSIZE`.
const ZZ_BUFFERSIZE: usize = 16384;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.YYINITIAL`.
const YYINITIAL: i32 = 0;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.AMPERSAND`.
const AMPERSAND: i32 = 2;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.NUMERIC_CHARACTER`.
const NUMERIC_CHARACTER: i32 = 4;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.CHARACTER_REFERENCE_TAIL`.
const CHARACTER_REFERENCE_TAIL: i32 = 6;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.LEFT_ANGLE_BRACKET`.
const LEFT_ANGLE_BRACKET: i32 = 8;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.BANG`.
const BANG: i32 = 10;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.COMMENT`.
const COMMENT: i32 = 12;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.SCRIPT`.
const SCRIPT: i32 = 14;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.SCRIPT_COMMENT`.
const SCRIPT_COMMENT: i32 = 16;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.LEFT_ANGLE_BRACKET_SLASH`.
const LEFT_ANGLE_BRACKET_SLASH: i32 = 18;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.LEFT_ANGLE_BRACKET_SPACE`.
const LEFT_ANGLE_BRACKET_SPACE: i32 = 20;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.CDATA`.
const CDATA: i32 = 22;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.SERVER_SIDE_INCLUDE`.
const SERVER_SIDE_INCLUDE: i32 = 24;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.SINGLE_QUOTED_STRING`.
const SINGLE_QUOTED_STRING: i32 = 26;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.DOUBLE_QUOTED_STRING`.
const DOUBLE_QUOTED_STRING: i32 = 28;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.END_TAG_TAIL_INCLUDE`.
const END_TAG_TAIL_INCLUDE: i32 = 30;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.END_TAG_TAIL_EXCLUDE`.
const END_TAG_TAIL_EXCLUDE: i32 = 32;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.END_TAG_TAIL_SUBSTITUTE`.
const END_TAG_TAIL_SUBSTITUTE: i32 = 34;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.START_TAG_TAIL_INCLUDE`.
const START_TAG_TAIL_INCLUDE: i32 = 36;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.START_TAG_TAIL_EXCLUDE`.
const START_TAG_TAIL_EXCLUDE: i32 = 38;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.START_TAG_TAIL_SUBSTITUTE`.
const START_TAG_TAIL_SUBSTITUTE: i32 = 40;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.STYLE`.
const STYLE: i32 = 42;

/// Lexical state.
///
/// Equivalent to `HTMLStripCharFilter.STYLE_COMMENT`.
const STYLE_COMMENT: i32 = 44;

/// `ZZ_LEXSTATE[l]` is the state in the DFA for the lexical state `l`;
/// `ZZ_LEXSTATE[l+1]` is the state in the DFA for the lexical state `l` at the
/// beginning of a line; `l` is of the form `l = 2*k`, `k` a non negative integer.
const ZZ_LEXSTATE: [i32; 46] = [
    0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14,
    14, 15, 15, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22,
];

/// Error code for "Unknown internal scanner error".
const ZZ_UNKNOWN_ERROR: usize = 0;
/// Error code for "could not match input".
const ZZ_NO_MATCH: usize = 1;
/// Error code for "pushback value was too large".
const ZZ_PUSHBACK_2BIG: usize = 2;

/// Error messages for [`ZZ_UNKNOWN_ERROR`], [`ZZ_NO_MATCH`], and
/// [`ZZ_PUSHBACK_2BIG`] respectively.
const ZZ_ERROR_MSG: [&str; 3] = [
    "Unknown internal scanner error",
    "Error: could not match input",
    "Error: pushback value was too large",
];
// @generated end constants
/// Equivalent to `HTMLStripCharFilter.upperCaseVariantsAccepted`, a
/// `HashMap` filled by the static initializer.
static UPPER_CASE_VARIANTS_ACCEPTED: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| {
        let mut upper_case_variants_accepted = HashMap::new();
        upper_case_variants_accepted.insert("quot", "QUOT");
        upper_case_variants_accepted.insert("copy", "COPY");
        upper_case_variants_accepted.insert("gt", "GT");
        upper_case_variants_accepted.insert("lt", "LT");
        upper_case_variants_accepted.insert("reg", "REG");
        upper_case_variants_accepted.insert("amp", "AMP");
        upper_case_variants_accepted
    });

/// The `entities` array of the static initializer: each entity name and the
/// first UTF-16 code unit of its value, `entities[i + 1].charAt(0)`.
const ENTITIES: [(&str, u16); 253] = [
    ("AElig", 0x00C6),
    ("Aacute", 0x00C1),
    ("Acirc", 0x00C2),
    ("Agrave", 0x00C0),
    ("Alpha", 0x0391),
    ("Aring", 0x00C5),
    ("Atilde", 0x00C3),
    ("Auml", 0x00C4),
    ("Beta", 0x0392),
    ("Ccedil", 0x00C7),
    ("Chi", 0x03A7),
    ("Dagger", 0x2021),
    ("Delta", 0x0394),
    ("ETH", 0x00D0),
    ("Eacute", 0x00C9),
    ("Ecirc", 0x00CA),
    ("Egrave", 0x00C8),
    ("Epsilon", 0x0395),
    ("Eta", 0x0397),
    ("Euml", 0x00CB),
    ("Gamma", 0x0393),
    ("Iacute", 0x00CD),
    ("Icirc", 0x00CE),
    ("Igrave", 0x00CC),
    ("Iota", 0x0399),
    ("Iuml", 0x00CF),
    ("Kappa", 0x039A),
    ("Lambda", 0x039B),
    ("Mu", 0x039C),
    ("Ntilde", 0x00D1),
    ("Nu", 0x039D),
    ("OElig", 0x0152),
    ("Oacute", 0x00D3),
    ("Ocirc", 0x00D4),
    ("Ograve", 0x00D2),
    ("Omega", 0x03A9),
    ("Omicron", 0x039F),
    ("Oslash", 0x00D8),
    ("Otilde", 0x00D5),
    ("Ouml", 0x00D6),
    ("Phi", 0x03A6),
    ("Pi", 0x03A0),
    ("Prime", 0x2033),
    ("Psi", 0x03A8),
    ("Rho", 0x03A1),
    ("Scaron", 0x0160),
    ("Sigma", 0x03A3),
    ("THORN", 0x00DE),
    ("Tau", 0x03A4),
    ("Theta", 0x0398),
    ("Uacute", 0x00DA),
    ("Ucirc", 0x00DB),
    ("Ugrave", 0x00D9),
    ("Upsilon", 0x03A5),
    ("Uuml", 0x00DC),
    ("Xi", 0x039E),
    ("Yacute", 0x00DD),
    ("Yuml", 0x0178),
    ("Zeta", 0x0396),
    ("aacute", 0x00E1),
    ("acirc", 0x00E2),
    ("acute", 0x00B4),
    ("aelig", 0x00E6),
    ("agrave", 0x00E0),
    ("alefsym", 0x2135),
    ("alpha", 0x03B1),
    ("amp", 0x0026),
    ("and", 0x2227),
    ("ang", 0x2220),
    ("apos", 0x0027),
    ("aring", 0x00E5),
    ("asymp", 0x2248),
    ("atilde", 0x00E3),
    ("auml", 0x00E4),
    ("bdquo", 0x201E),
    ("beta", 0x03B2),
    ("brvbar", 0x00A6),
    ("bull", 0x2022),
    ("cap", 0x2229),
    ("ccedil", 0x00E7),
    ("cedil", 0x00B8),
    ("cent", 0x00A2),
    ("chi", 0x03C7),
    ("circ", 0x02C6),
    ("clubs", 0x2663),
    ("cong", 0x2245),
    ("copy", 0x00A9),
    ("crarr", 0x21B5),
    ("cup", 0x222A),
    ("curren", 0x00A4),
    ("dArr", 0x21D3),
    ("dagger", 0x2020),
    ("darr", 0x2193),
    ("deg", 0x00B0),
    ("delta", 0x03B4),
    ("diams", 0x2666),
    ("divide", 0x00F7),
    ("eacute", 0x00E9),
    ("ecirc", 0x00EA),
    ("egrave", 0x00E8),
    ("empty", 0x2205),
    ("emsp", 0x2003),
    ("ensp", 0x2002),
    ("epsilon", 0x03B5),
    ("equiv", 0x2261),
    ("eta", 0x03B7),
    ("eth", 0x00F0),
    ("euml", 0x00EB),
    ("euro", 0x20AC),
    ("exist", 0x2203),
    ("fnof", 0x0192),
    ("forall", 0x2200),
    ("frac12", 0x00BD),
    ("frac14", 0x00BC),
    ("frac34", 0x00BE),
    ("frasl", 0x2044),
    ("gamma", 0x03B3),
    ("ge", 0x2265),
    ("gt", 0x003E),
    ("hArr", 0x21D4),
    ("harr", 0x2194),
    ("hearts", 0x2665),
    ("hellip", 0x2026),
    ("iacute", 0x00ED),
    ("icirc", 0x00EE),
    ("iexcl", 0x00A1),
    ("igrave", 0x00EC),
    ("image", 0x2111),
    ("infin", 0x221E),
    ("int", 0x222B),
    ("iota", 0x03B9),
    ("iquest", 0x00BF),
    ("isin", 0x2208),
    ("iuml", 0x00EF),
    ("kappa", 0x03BA),
    ("lArr", 0x21D0),
    ("lambda", 0x03BB),
    ("lang", 0x2329),
    ("laquo", 0x00AB),
    ("larr", 0x2190),
    ("lceil", 0x2308),
    ("ldquo", 0x201C),
    ("le", 0x2264),
    ("lfloor", 0x230A),
    ("lowast", 0x2217),
    ("loz", 0x25CA),
    ("lrm", 0x200E),
    ("lsaquo", 0x2039),
    ("lsquo", 0x2018),
    ("lt", 0x003C),
    ("macr", 0x00AF),
    ("mdash", 0x2014),
    ("micro", 0x00B5),
    ("middot", 0x00B7),
    ("minus", 0x2212),
    ("mu", 0x03BC),
    ("nabla", 0x2207),
    ("nbsp", 0x0020),
    ("ndash", 0x2013),
    ("ne", 0x2260),
    ("ni", 0x220B),
    ("not", 0x00AC),
    ("notin", 0x2209),
    ("nsub", 0x2284),
    ("ntilde", 0x00F1),
    ("nu", 0x03BD),
    ("oacute", 0x00F3),
    ("ocirc", 0x00F4),
    ("oelig", 0x0153),
    ("ograve", 0x00F2),
    ("oline", 0x203E),
    ("omega", 0x03C9),
    ("omicron", 0x03BF),
    ("oplus", 0x2295),
    ("or", 0x2228),
    ("ordf", 0x00AA),
    ("ordm", 0x00BA),
    ("oslash", 0x00F8),
    ("otilde", 0x00F5),
    ("otimes", 0x2297),
    ("ouml", 0x00F6),
    ("para", 0x00B6),
    ("part", 0x2202),
    ("permil", 0x2030),
    ("perp", 0x22A5),
    ("phi", 0x03C6),
    ("pi", 0x03C0),
    ("piv", 0x03D6),
    ("plusmn", 0x00B1),
    ("pound", 0x00A3),
    ("prime", 0x2032),
    ("prod", 0x220F),
    ("prop", 0x221D),
    ("psi", 0x03C8),
    ("quot", 0x0022),
    ("rArr", 0x21D2),
    ("radic", 0x221A),
    ("rang", 0x232A),
    ("raquo", 0x00BB),
    ("rarr", 0x2192),
    ("rceil", 0x2309),
    ("rdquo", 0x201D),
    ("real", 0x211C),
    ("reg", 0x00AE),
    ("rfloor", 0x230B),
    ("rho", 0x03C1),
    ("rlm", 0x200F),
    ("rsaquo", 0x203A),
    ("rsquo", 0x2019),
    ("sbquo", 0x201A),
    ("scaron", 0x0161),
    ("sdot", 0x22C5),
    ("sect", 0x00A7),
    ("shy", 0x00AD),
    ("sigma", 0x03C3),
    ("sigmaf", 0x03C2),
    ("sim", 0x223C),
    ("spades", 0x2660),
    ("sub", 0x2282),
    ("sube", 0x2286),
    ("sum", 0x2211),
    ("sup", 0x2283),
    ("sup1", 0x00B9),
    ("sup2", 0x00B2),
    ("sup3", 0x00B3),
    ("supe", 0x2287),
    ("szlig", 0x00DF),
    ("tau", 0x03C4),
    ("there4", 0x2234),
    ("theta", 0x03B8),
    ("thetasym", 0x03D1),
    ("thinsp", 0x2009),
    ("thorn", 0x00FE),
    ("tilde", 0x02DC),
    ("times", 0x00D7),
    ("trade", 0x2122),
    ("uArr", 0x21D1),
    ("uacute", 0x00FA),
    ("uarr", 0x2191),
    ("ucirc", 0x00FB),
    ("ugrave", 0x00F9),
    ("uml", 0x00A8),
    ("upsih", 0x03D2),
    ("upsilon", 0x03C5),
    ("uuml", 0x00FC),
    ("weierp", 0x2118),
    ("xi", 0x03BE),
    ("yacute", 0x00FD),
    ("yen", 0x00A5),
    ("yuml", 0x00FF),
    ("zeta", 0x03B6),
    ("zwj", 0x200D),
    ("zwnj", 0x200C),
];

/// Equivalent to `HTMLStripCharFilter.entityValues`, `new
/// CharArrayMap<Character>(253, false)` filled by the static initializer with
/// every entity and the accepted upper case variants.
static ENTITY_VALUES: LazyLock<CharArrayMap<u16>> = LazyLock::new(|| {
    let mut entity_values = CharArrayMap::new(253, false);
    for (entity, value) in ENTITIES {
        entity_values.put(entity, value);
        if let Some(upper_case_variant) = UPPER_CASE_VARIANTS_ACCEPTED.get(entity) {
            entity_values.put(upper_case_variant, value);
        }
    }
    entity_values
});

/// Equivalent to `HTMLStripCharFilter.INITIAL_INPUT_SEGMENT_SIZE`.
const INITIAL_INPUT_SEGMENT_SIZE: usize = 1024;
/// Equivalent to `HTMLStripCharFilter.BLOCK_LEVEL_START_TAG_REPLACEMENT`.
const BLOCK_LEVEL_START_TAG_REPLACEMENT: u16 = b'\n' as u16;
/// Equivalent to `HTMLStripCharFilter.BLOCK_LEVEL_END_TAG_REPLACEMENT`.
const BLOCK_LEVEL_END_TAG_REPLACEMENT: u16 = b'\n' as u16;
/// Equivalent to `HTMLStripCharFilter.BR_START_TAG_REPLACEMENT`.
const BR_START_TAG_REPLACEMENT: u16 = b'\n' as u16;
/// Equivalent to `HTMLStripCharFilter.BR_END_TAG_REPLACEMENT`.
const BR_END_TAG_REPLACEMENT: u16 = b'\n' as u16;
/// Equivalent to `HTMLStripCharFilter.SCRIPT_REPLACEMENT`.
const SCRIPT_REPLACEMENT: u16 = b'\n' as u16;
/// Equivalent to `HTMLStripCharFilter.STYLE_REPLACEMENT`.
const STYLE_REPLACEMENT: u16 = b'\n' as u16;
/// Equivalent to `HTMLStripCharFilter.REPLACEMENT_CHARACTER`.
const REPLACEMENT_CHARACTER: u16 = 0xFFFD;

/// `Character.MIN_SURROGATE`.
const MIN_SURROGATE: i32 = 0xD800;
/// `Character.MAX_SURROGATE`.
const MAX_SURROGATE: i32 = 0xDFFF;

/// Which segment Java's `outputSegment` field refers to: it always holds
/// `inputSegment` or `entitySegment`, so the port names one of the two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Segment {
    /// `outputSegment == inputSegment`.
    Input,
    /// `outputSegment == entitySegment`.
    Entity,
}

/// A CharFilter that wraps another Reader and attempts to strip out HTML
/// constructs.
///
/// Equivalent to `org.apache.lucene.analysis.charfilter.HTMLStripCharFilter`,
/// a `BaseCharFilter`: the [`BaseCharFilter`] it holds carries the input (the
/// scanner's `zzReader` too) and the offset corrections.
#[derive(Debug)]
pub struct HTMLStripCharFilter {
    /// The `BaseCharFilter` (and `CharFilter`) state: the input and the offset
    /// corrections.
    base: BaseCharFilter,
    // @generated begin fields by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
    /// Current state of the DFA (`zzState`).
    zz_state: i32,
    /// Current lexical state (`zzLexicalState`).
    zz_lexical_state: i32,
    /// This buffer contains the current text to be matched and is the source of
    /// the `yytext()` string (`zzBuffer`).
    zz_buffer: Vec<u16>,
    /// Text position at the last accepting state (`zzMarkedPos`).
    zz_marked_pos: usize,
    /// Current text position in the buffer (`zzCurrentPos`).
    zz_current_pos: usize,
    /// Marks the beginning of the `yytext()` string in the buffer (`zzStartRead`).
    zz_start_read: usize,
    /// Marks the last character in the buffer, that has been read from input
    /// (`zzEndRead`).
    zz_end_read: usize,
    /// Whether the scanner is at the end of file (`zzAtEOF`).
    zz_at_eof: bool,
    /// The number of occupied positions in `zz_buffer` beyond `zz_end_read`: 1
    /// when a lead/high surrogate has been read from the input stream into the
    /// final `zz_buffer` position, 0 otherwise (`zzFinalHighSurrogate`).
    zz_final_high_surrogate: usize,
    /// Number of newlines encountered up to the start of the matched text
    /// (`yyline`).
    #[allow(dead_code)] // `@SuppressWarnings("unused")`, as in Java
    yyline: i32,
    /// Number of characters from the last newline up to the start of the matched
    /// text (`yycolumn`).
    #[allow(dead_code)] // `@SuppressWarnings("unused")`, as in Java
    yycolumn: i32,
    /// Number of characters (UTF-16 code units) up to the start of the matched
    /// text (`yychar`).
    yychar: i64,
    /// Whether the scanner is currently at the beginning of a line (`zzAtBOL`).
    #[allow(dead_code)] // `@SuppressWarnings("unused")`, as in Java
    zz_at_bol: bool,
    /// Whether the user-EOF-code has already been executed (`zzEOFDone`).
    zz_eof_done: bool,
    // @generated end fields
    escaped_tags: Option<CharArraySet>,
    input_start: i64,
    cumulative_diff: i32,
    escape_br: bool,
    escape_script: bool,
    escape_style: bool,
    restore_state: i32,
    previous_restore_state: i32,
    output_char_count: i32,
    eof_return_value: i32,
    input_segment: TextSegment,
    output_segment: Segment,
    entity_segment: TextSegment,
}

impl HTMLStripCharFilter {
    /// Creates a new scanner
    ///
    /// Equivalent to `HTMLStripCharFilter(java.io.Reader in)`, `super(in);
    /// this.zzReader = in;`, and the field initializers.
    pub fn new(input: Box<dyn CharReader>) -> Self {
        Self {
            base: BaseCharFilter::new(input),
            // @generated begin init by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
            zz_buffer: vec![0; ZZ_BUFFERSIZE],
            zz_state: 0,
            zz_lexical_state: YYINITIAL,
            zz_marked_pos: 0,
            zz_current_pos: 0,
            zz_start_read: 0,
            zz_end_read: 0,
            zz_at_eof: false,
            zz_final_high_surrogate: 0,
            yyline: 0,
            yycolumn: 0,
            yychar: 0,
            zz_at_bol: true,
            zz_eof_done: false,
            // @generated end init
            escaped_tags: None,
            input_start: 0,
            cumulative_diff: 0,
            escape_br: false,
            escape_script: false,
            escape_style: false,
            restore_state: 0,
            previous_restore_state: 0,
            output_char_count: 0,
            eof_return_value: 0,
            input_segment: TextSegment::with_size(INITIAL_INPUT_SEGMENT_SIZE),
            output_segment: Segment::Input,
            entity_segment: TextSegment::with_size(2),
        }
    }

    /// Creates a new HTMLStripCharFilter over the provided Reader with the
    /// specified start and end tags.
    ///
    /// * `input` Reader to strip html tags from.
    /// * `escaped_tags` Tags in this set (both start and end tags) will not be
    ///   filtered out.
    ///
    /// Equivalent to `HTMLStripCharFilter(Reader, Set<String>)`; Java's `null`
    /// set is `None`.
    pub fn with_escaped_tags(
        input: Box<dyn CharReader>,
        escaped_tags: Option<&HashSet<String>>,
    ) -> Self {
        let mut filter = Self::new(input);
        if let Some(escaped_tags) = escaped_tags {
            for tag in escaped_tags {
                if java_equals_ignore_case(tag, "BR") {
                    filter.escape_br = true;
                } else if java_equals_ignore_case(tag, "SCRIPT") {
                    filter.escape_script = true;
                } else if java_equals_ignore_case(tag, "STYLE") {
                    filter.escape_style = true;
                } else {
                    filter
                        .escaped_tags
                        .get_or_insert_with(|| CharArraySet::new(16, true))
                        .add(tag);
                }
            }
        }
        filter
    }

    /// Reads one UTF-16 code unit of the stripped text, or `-1` at the end of
    /// the stream.
    ///
    /// Equivalent to `HTMLStripCharFilter.read()`.
    ///
    /// # Errors
    ///
    /// What the input raises.
    pub fn read_char(&mut self) -> Result<i32> {
        if self.output_segment().is_read() {
            if self.zz_at_eof {
                return Ok(-1);
            }
            let ch = self.next_char()?;
            self.output_char_count += 1;
            return Ok(ch);
        }
        let ch = self.output_segment_mut().next_char();
        self.output_char_count += 1;
        Ok(ch)
    }

    /// Equivalent to the package-private `HTMLStripCharFilter.getInitialBufferSize()`,
    /// for testing purposes.
    pub(crate) fn get_initial_buffer_size() -> usize {
        ZZ_BUFFERSIZE
    }

    /// The segment Java's `outputSegment` refers to.
    fn output_segment(&self) -> &TextSegment {
        match self.output_segment {
            Segment::Input => &self.input_segment,
            Segment::Entity => &self.entity_segment,
        }
    }

    /// The segment Java's `outputSegment` refers to, for writing.
    fn output_segment_mut(&mut self) -> &mut TextSegment {
        match self.output_segment {
            Segment::Input => &mut self.input_segment,
            Segment::Entity => &mut self.entity_segment,
        }
    }

    /// `addOffCorrectMap(int, int)`, inherited from `BaseCharFilter`.
    fn add_off_correct_map(&mut self, off: i32, cumulative_diff: i32) {
        self.base.add_off_correct_map(off, cumulative_diff);
    }

    /// The text of the current match, Java's `yytext()`, as UTF-16 code units.
    fn yytext_units(&self) -> &[u16] {
        &self.zz_buffer[self.zz_start_read..self.zz_marked_pos]
    }

    /// `inputSegment.write(zzBuffer, zzStartRead, length)`.
    fn write_match_to_input_segment(&mut self, length: usize) {
        self.input_segment
            .write_slice(&self.zz_buffer, self.zz_start_read, length);
    }

    /// `escapedTags.contains(zzBuffer, zzStartRead, yylength())`, `false`
    /// when `escapedTags` is `null`.
    fn escaped_tags_contains_match(&self) -> bool {
        match &self.escaped_tags {
            None => false,
            Some(escaped_tags) => escaped_tags.contains_chars(
                Some(&self.zz_buffer),
                self.zz_start_read,
                self.yylength() as usize,
            ),
        }
    }

    /// The `%eof{ ... %}` code of the grammar, which `zzDoEOF()` runs once.
    ///
    /// Equivalent to the user EOF-code of `HTMLStripCharFilter.zzDoEOF()`.
    fn eof_code(&mut self) {
        match self.zz_lexical_state {
            SCRIPT
            | COMMENT
            | SCRIPT_COMMENT
            | STYLE
            | STYLE_COMMENT
            | SINGLE_QUOTED_STRING
            | DOUBLE_QUOTED_STRING
            | END_TAG_TAIL_EXCLUDE
            | END_TAG_TAIL_SUBSTITUTE
            | START_TAG_TAIL_EXCLUDE
            | SERVER_SIDE_INCLUDE
            | START_TAG_TAIL_SUBSTITUTE => {
                // Exclude
                // add (length of input that won't be output) [ - (substitution length) = 0 ]
                self.cumulative_diff += (self.yychar - self.input_start) as i32;
                // position the correction at (already output length) [ + (substitution length) = 0 ]
                self.add_off_correct_map(self.output_char_count, self.cumulative_diff);
                self.output_segment_mut().clear();
                self.eof_return_value = -1;
            }
            CHARACTER_REFERENCE_TAIL => {
                // Substitute
                // At end of file, allow char refs without semicolons
                // add (length of input that won't be output) - (substitution length)
                self.cumulative_diff +=
                    self.input_segment.length() as i32 - self.output_segment().length() as i32;
                // position the correction at (already output length) + (substitution length)
                self.add_off_correct_map(
                    self.output_char_count + self.output_segment().length() as i32,
                    self.cumulative_diff,
                );
                self.eof_return_value = if !self.output_segment().is_read() {
                    self.output_segment_mut().next_char()
                } else {
                    -1
                };
            }
            BANG
            | CDATA
            | AMPERSAND
            | NUMERIC_CHARACTER
            | END_TAG_TAIL_INCLUDE
            | START_TAG_TAIL_INCLUDE
            | LEFT_ANGLE_BRACKET
            | LEFT_ANGLE_BRACKET_SLASH
            | LEFT_ANGLE_BRACKET_SPACE => {
                // Include
                self.output_segment = Segment::Input;
                self.eof_return_value = if !self.output_segment().is_read() {
                    self.output_segment_mut().next_char()
                } else {
                    -1
                };
            }
            _ => {
                self.eof_return_value = -1;
            }
        }
    }

    /// `Integer.parseInt(s)` of the UTF-16 code units `s`, `None` where Java
    /// throws `NumberFormatException` (a lone surrogate, as Java's
    /// `Character.digit` of a surrogate, is no digit).
    fn parse_int(s: &[u16]) -> Option<i32> {
        rucene::util::extra::java_parse_int(&String::from_utf16_lossy(s))
    }

    /// `Integer.parseInt(s, 16)` of the UTF-16 code units `s`, `None` where
    /// Java throws `NumberFormatException`.
    fn parse_hex(s: &[u16]) -> Option<i32> {
        rucene::util::extra::java_parse_long_radix(&String::from_utf16_lossy(s), 16)
            .and_then(|v| i32::try_from(v).ok())
    }

    /// Java's `assert false : message` in a `catch` block: an `AssertionError`
    /// with assertions enabled (a debug build here), nothing otherwise.
    fn assert_false(message: impl FnOnce() -> String) {
        if cfg!(debug_assertions) {
            panic!("{}", message());
        }
    }

    /// The segment Java's `outputSegment` refers to after `outputSegment =
    /// entitySegment; outputSegment.clear();`, filled with the code point
    /// `code_point` as the decimal and hexadecimal character references fill
    /// it: U+FFFD for a surrogate, else `Character.toChars(codePoint,
    /// outputSegment.getArray(), 0)`.
    fn output_code_point(&mut self, code_point: i32) {
        self.output_segment = Segment::Entity;
        self.output_segment_mut().clear();
        if (MIN_SURROGATE..=MAX_SURROGATE).contains(&code_point) {
            self.output_segment_mut()
                .unsafe_write(REPLACEMENT_CHARACTER);
        } else {
            let segment = self.output_segment_mut();
            let length = match char::from_u32(code_point as u32) {
                Some(c) => c.encode_utf16(segment.get_array_mut()).len(),
                None => 0,
            };
            segment.set_length(length);
        }
    }

    /// `case 1:` of `HTMLStripCharFilter.nextChar()`.
    fn action_1(&mut self) -> Option<i32> {
        if self.yylength() == 1 {
            Some(i32::from(self.zz_buffer[self.zz_start_read]))
        } else {
            let segment = match self.output_segment {
                Segment::Input => &mut self.input_segment,
                Segment::Entity => &mut self.entity_segment,
            };
            segment.append(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos]);
            Some(segment.next_char())
        }
    }

    /// `case 2:` of `HTMLStripCharFilter.nextChar()`.
    fn action_2(&mut self) -> Option<i32> {
        self.input_start = self.yychar;
        self.input_segment.clear();
        self.input_segment.append_char(u16::from(b'&'));
        self.yybegin(AMPERSAND);
        None
    }

    /// `case 3:` of `HTMLStripCharFilter.nextChar()`.
    fn action_3(&mut self) -> Option<i32> {
        self.input_start = self.yychar;
        self.input_segment.clear();
        self.input_segment.append_char(u16::from(b'<'));
        self.yybegin(LEFT_ANGLE_BRACKET);
        None
    }

    /// `case 4:` of `HTMLStripCharFilter.nextChar()`.
    fn action_4(&mut self) -> Option<i32> {
        self.yypushback(self.yylength());
        self.output_segment = Segment::Input;
        self.output_segment_mut().restart();
        self.yybegin(YYINITIAL);
        Some(self.output_segment_mut().next_char())
    }

    /// `case 5:` of `HTMLStripCharFilter.nextChar()`.
    fn action_5(&mut self) -> Option<i32> {
        self.input_segment.append_char(u16::from(b'#'));
        self.yybegin(NUMERIC_CHARACTER);
        None
    }

    /// `case 6:` of `HTMLStripCharFilter.nextChar()`: a decimal character
    /// reference.
    fn action_6(&mut self) -> Option<i32> {
        let match_length = self.yylength();
        self.write_match_to_input_segment(match_length as usize);
        if match_length <= 7 {
            // 0x10FFFF = 1114111: max 7 decimal chars
            let decimal_char_ref = self.yytext_units();
            let code_point = match Self::parse_int(decimal_char_ref) {
                Some(code_point) => code_point,
                None => {
                    Self::assert_false(|| {
                        format!(
                            "Exception parsing code point '{}'",
                            String::from_utf16_lossy(decimal_char_ref)
                        )
                    });
                    0
                }
            };
            if code_point <= 0x10FFFF {
                self.output_code_point(code_point);
                self.yybegin(CHARACTER_REFERENCE_TAIL);
                None
            } else {
                self.output_segment = Segment::Input;
                self.yybegin(YYINITIAL);
                Some(self.output_segment_mut().next_char())
            }
        } else {
            self.output_segment = Segment::Input;
            self.yybegin(YYINITIAL);
            Some(self.output_segment_mut().next_char())
        }
    }

    /// `case 7:` of `HTMLStripCharFilter.nextChar()`.
    fn action_7(&mut self) -> Option<i32> {
        // add (previously matched input length) + (this match length) - (substitution length)
        self.cumulative_diff += self.input_segment.length() as i32 + self.yylength()
            - self.output_segment().length() as i32;
        // position the correction at (already output length) + (substitution length)
        self.add_off_correct_map(
            self.output_char_count + self.output_segment().length() as i32,
            self.cumulative_diff,
        );
        self.yybegin(YYINITIAL);
        Some(self.output_segment_mut().next_char())
    }

    /// `case 8:` of `HTMLStripCharFilter.nextChar()`.
    fn action_8(&mut self) -> Option<i32> {
        self.write_match_to_input_segment(self.yylength() as usize);
        self.yybegin(LEFT_ANGLE_BRACKET_SPACE);
        None
    }

    /// `case 9:` of `HTMLStripCharFilter.nextChar()`.
    fn action_9(&mut self) -> Option<i32> {
        self.input_segment.append_char(u16::from(b'!'));
        self.yybegin(BANG);
        None
    }

    /// `case 10:` of `HTMLStripCharFilter.nextChar()`.
    fn action_10(&mut self) -> Option<i32> {
        self.input_segment.append_char(u16::from(b'/'));
        self.yybegin(LEFT_ANGLE_BRACKET_SLASH);
        None
    }

    /// `case 11:` of `HTMLStripCharFilter.nextChar()`.
    fn action_11(&mut self) -> Option<i32> {
        self.write_match_to_input_segment(self.yylength() as usize);
        if self.escaped_tags_contains_match() {
            self.yybegin(START_TAG_TAIL_INCLUDE);
        } else {
            self.yybegin(START_TAG_TAIL_SUBSTITUTE);
        }
        None
    }

    /// `case 12:` of `HTMLStripCharFilter.nextChar()`.
    fn action_12(&mut self) -> Option<i32> {
        self.write_match_to_input_segment(self.yylength() as usize);
        if self.escaped_tags_contains_match() {
            self.yybegin(START_TAG_TAIL_INCLUDE);
        } else {
            self.yybegin(START_TAG_TAIL_EXCLUDE);
        }
        None
    }

    /// `case 13:` of `HTMLStripCharFilter.nextChar()`.
    fn action_13(&mut self) -> Option<i32> {
        self.input_segment
            .append(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos]);
        None
    }

    /// `case 14:` of `HTMLStripCharFilter.nextChar()`.
    fn action_14(&mut self) -> Option<i32> {
        // add (previously matched input length) + (this match length) [ - (substitution length) = 0 ]
        self.cumulative_diff += self.input_segment.length() as i32 + self.yylength();
        // position the correction at (already output length) [ + (substitution length) = 0 ]
        self.add_off_correct_map(self.output_char_count, self.cumulative_diff);
        self.input_segment.clear();
        self.yybegin(YYINITIAL);
        None
    }

    /// `case 18:` of `HTMLStripCharFilter.nextChar()`.
    fn action_18(&mut self) -> Option<i32> {
        self.write_match_to_input_segment(self.yylength() as usize);
        None
    }

    /// `case 19:` of `HTMLStripCharFilter.nextChar()`.
    fn action_19(&mut self) -> Option<i32> {
        self.write_match_to_input_segment(self.yylength() as usize);
        if self.escaped_tags_contains_match() {
            self.yybegin(END_TAG_TAIL_INCLUDE);
        } else {
            self.yybegin(END_TAG_TAIL_SUBSTITUTE);
        }
        None
    }

    /// `case 20:` of `HTMLStripCharFilter.nextChar()`.
    fn action_20(&mut self) -> Option<i32> {
        self.write_match_to_input_segment(self.yylength() as usize);
        if self.escaped_tags_contains_match() {
            self.yybegin(END_TAG_TAIL_INCLUDE);
        } else {
            self.yybegin(END_TAG_TAIL_EXCLUDE);
        }
        None
    }

    /// `case 21:` of `HTMLStripCharFilter.nextChar()`, the body of `case 1:`
    /// in another lexical state.
    fn action_21(&mut self) -> Option<i32> {
        if self.yylength() == 1 {
            Some(i32::from(self.zz_buffer[self.zz_start_read]))
        } else {
            let segment = match self.output_segment {
                Segment::Input => &mut self.input_segment,
                Segment::Entity => &mut self.entity_segment,
            };
            segment.append(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos]);
            Some(segment.next_char())
        }
    }

    /// `case 25:` of `HTMLStripCharFilter.nextChar()`.
    fn action_25(&mut self) -> Option<i32> {
        self.write_match_to_input_segment(self.yylength() as usize);
        self.output_segment = Segment::Input;
        self.yybegin(YYINITIAL);
        Some(self.output_segment_mut().next_char())
    }

    /// `case 26:` of `HTMLStripCharFilter.nextChar()`.
    fn action_26(&mut self) -> Option<i32> {
        // add (previously matched input length) + (this match length) - (substitution length)
        self.cumulative_diff += self.input_segment.length() as i32 + self.yylength() - 1;
        // position the correction at (already output length) + (substitution length)
        self.add_off_correct_map(self.output_char_count + 1, self.cumulative_diff);
        self.input_segment.clear();
        self.yybegin(YYINITIAL);
        Some(i32::from(BLOCK_LEVEL_END_TAG_REPLACEMENT))
    }

    /// `case 27:` of `HTMLStripCharFilter.nextChar()`.
    fn action_27(&mut self) -> Option<i32> {
        // add (previously matched input length) + (this match length) [ - (substitution length) = 0 ]
        self.cumulative_diff += self.input_segment.length() as i32 + self.yylength();
        // position the correction at (already output length) [ + (substitution length) = 0 ]
        self.add_off_correct_map(self.output_char_count, self.cumulative_diff);
        self.input_segment.clear();
        self.output_segment = Segment::Input;
        self.yybegin(YYINITIAL);
        None
    }

    /// `case 28:` of `HTMLStripCharFilter.nextChar()`.
    fn action_28(&mut self) -> Option<i32> {
        // add (previously matched input length) + (this match length) - (substitution length)
        self.cumulative_diff += self.input_segment.length() as i32 + self.yylength() - 1;
        // position the correction at (already output length) + (substitution length)
        self.add_off_correct_map(self.output_char_count + 1, self.cumulative_diff);
        self.input_segment.clear();
        self.yybegin(YYINITIAL);
        Some(i32::from(BLOCK_LEVEL_START_TAG_REPLACEMENT))
    }

    /// `case 31:` of `HTMLStripCharFilter.nextChar()`: a character entity.
    ///
    /// # Panics
    ///
    /// Where Java's `entityValues.get(...).charValue()` throws
    /// `NullPointerException`: a match that is no entity, which the grammar's
    /// `{CharacterEntities}` rule excludes.
    fn action_31(&mut self) -> Option<i32> {
        let length = self.yylength() as usize;
        self.write_match_to_input_segment(length);
        self.entity_segment.clear();
        let ch = *ENTITY_VALUES
            .get_chars(&self.zz_buffer, self.zz_start_read, length)
            .expect("INVARIANT: the grammar matches character entities only");
        self.entity_segment.append_char(ch);
        self.output_segment = Segment::Entity;
        self.yybegin(CHARACTER_REFERENCE_TAIL);
        None
    }

    /// `case 32:` of `HTMLStripCharFilter.nextChar()`: a hexadecimal
    /// character reference.
    fn action_32(&mut self) -> Option<i32> {
        let match_length = self.yylength();
        self.write_match_to_input_segment(match_length as usize);
        if match_length <= 6 {
            // 10FFFF: max 6 hex chars
            let start = self.zz_start_read + 1;
            let hex_char_ref = &self.zz_buffer[start..start + match_length as usize - 1];
            let code_point = match Self::parse_hex(hex_char_ref) {
                Some(code_point) => code_point,
                None => {
                    Self::assert_false(|| {
                        format!(
                            "Exception parsing hex code point '{}'",
                            String::from_utf16_lossy(hex_char_ref)
                        )
                    });
                    0
                }
            };
            if code_point <= 0x10FFFF {
                self.output_code_point(code_point);
                self.yybegin(CHARACTER_REFERENCE_TAIL);
                None
            } else {
                self.output_segment = Segment::Input;
                self.yybegin(YYINITIAL);
                Some(self.output_segment_mut().next_char())
            }
        } else {
            self.output_segment = Segment::Input;
            self.yybegin(YYINITIAL);
            Some(self.output_segment_mut().next_char())
        }
    }

    /// `case 33:` of `HTMLStripCharFilter.nextChar()`.
    fn action_33(&mut self) -> Option<i32> {
        if self.input_segment.length() > 2 {
            // Chars between "<!" and "--" - this is not a comment
            self.input_segment
                .append(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos]);
        } else {
            self.yybegin(COMMENT);
        }
        None
    }

    /// `case 34:` of `HTMLStripCharFilter.nextChar()`.
    fn action_34(&mut self) -> Option<i32> {
        self.yybegin(YYINITIAL);
        if self.escape_br {
            self.write_match_to_input_segment(self.yylength() as usize);
            self.output_segment = Segment::Input;
            Some(self.output_segment_mut().next_char())
        } else {
            // add (previously matched input length) + (this match length) - (substitution length)
            self.cumulative_diff += self.input_segment.length() as i32 + self.yylength() - 1;
            // position the correction at (already output length) + (substitution length)
            self.add_off_correct_map(self.output_char_count + 1, self.cumulative_diff);
            self.input_segment.reset();
            Some(i32::from(BR_START_TAG_REPLACEMENT))
        }
    }

    /// `case 35:` of `HTMLStripCharFilter.nextChar()`.
    fn action_35(&mut self) -> Option<i32> {
        // add (previously matched input length) + (this match length) [ - (substitution length) = 0]
        self.cumulative_diff +=
            (self.yychar - self.input_start + i64::from(self.yylength())) as i32;
        // position the correction at (already output length) [ + (substitution length) = 0]
        self.add_off_correct_map(self.output_char_count, self.cumulative_diff);
        self.input_segment.clear();
        self.yybegin(YYINITIAL);
        None
    }

    /// `case 37:` of `HTMLStripCharFilter.nextChar()`.
    fn action_37(&mut self) -> Option<i32> {
        self.yybegin(YYINITIAL);
        if self.escape_br {
            self.write_match_to_input_segment(self.yylength() as usize);
            self.output_segment = Segment::Input;
            Some(self.output_segment_mut().next_char())
        } else {
            // add (previously matched input length) + (this match length) - (substitution length)
            self.cumulative_diff += self.input_segment.length() as i32 + self.yylength() - 1;
            // position the correction at (already output length) + (substitution length)
            self.add_off_correct_map(self.output_char_count + 1, self.cumulative_diff);
            self.input_segment.reset();
            Some(i32::from(BR_END_TAG_REPLACEMENT))
        }
    }

    /// `case 38:` of `HTMLStripCharFilter.nextChar()`.
    fn action_38(&mut self) -> Option<i32> {
        // add (this match length) [ - (substitution length) = 0 ]
        self.cumulative_diff += self.yylength();
        // position the correction at (already output length) [ + (substitution length) = 0 ]
        self.add_off_correct_map(self.output_char_count, self.cumulative_diff);
        self.yybegin(YYINITIAL);
        None
    }

    /// `case 46:` of `HTMLStripCharFilter.nextChar()`.
    fn action_46(&mut self) -> Option<i32> {
        self.yybegin(STYLE);
        if self.escape_style {
            self.write_match_to_input_segment(self.yylength() as usize);
            self.output_segment = Segment::Input;
            self.input_start += 1 + i64::from(self.yylength());
            return Some(self.output_segment_mut().next_char());
        }
        None
    }

    /// `case 47:` of `HTMLStripCharFilter.nextChar()`.
    fn action_47(&mut self) -> Option<i32> {
        self.yybegin(SCRIPT);
        if self.escape_script {
            self.write_match_to_input_segment(self.yylength() as usize);
            self.output_segment = Segment::Input;
            self.input_start += 1 + i64::from(self.yylength());
            return Some(self.output_segment_mut().next_char());
        }
        None
    }

    /// `case 48:` of `HTMLStripCharFilter.nextChar()`.
    fn action_48(&mut self) -> Option<i32> {
        if self.input_segment.length() > 2 {
            // Chars between "<!" and "[CDATA[" - this is not a CDATA section
            self.input_segment
                .append(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos]);
        } else {
            // add (previously matched input length) + (this match length) [ - (substitution length) = 0 ]
            self.cumulative_diff += self.input_segment.length() as i32 + self.yylength();
            // position the correction at (already output length) [ + (substitution length) = 0 ]
            self.add_off_correct_map(self.output_char_count, self.cumulative_diff);
            self.input_segment.clear();
            self.yybegin(CDATA);
        }
        None
    }

    /// `case 49:` of `HTMLStripCharFilter.nextChar()`.
    fn action_49(&mut self) -> Option<i32> {
        self.input_segment.clear();
        self.yybegin(YYINITIAL);
        // add (previously matched input length) -- current match and substitution handled below
        self.cumulative_diff += (self.yychar - self.input_start) as i32;
        // position the offset correction at (already output length) -- substitution handled below
        let mut offset_correction_pos = self.output_char_count;
        let return_value = if self.escape_style {
            self.write_match_to_input_segment(self.yylength() as usize);
            self.output_segment = Segment::Input;
            self.output_segment_mut().next_char()
        } else {
            // add (this match length) - (substitution length)
            self.cumulative_diff += self.yylength() - 1;
            // add (substitution length)
            offset_correction_pos += 1;
            i32::from(STYLE_REPLACEMENT)
        };
        self.add_off_correct_map(offset_correction_pos, self.cumulative_diff);
        Some(return_value)
    }

    /// `case 50:` of `HTMLStripCharFilter.nextChar()`; its `cumulativeDiff +=
    /// yychar - inputStart` narrows the `long` sum to `int`.
    fn action_50(&mut self) -> Option<i32> {
        self.input_segment.clear();
        self.yybegin(YYINITIAL);
        // add (previously matched input length) -- current match and substitution handled below
        self.cumulative_diff =
            (i64::from(self.cumulative_diff) + (self.yychar - self.input_start)) as i32;
        // position at (already output length) -- substitution handled below
        let mut offset_correction_pos = self.output_char_count;
        let return_value = if self.escape_script {
            self.write_match_to_input_segment(self.yylength() as usize);
            self.output_segment = Segment::Input;
            self.output_segment_mut().next_char()
        } else {
            // add (this match length) - (substitution length)
            self.cumulative_diff += self.yylength() - 1;
            // add (substitution length)
            offset_correction_pos += 1;
            i32::from(SCRIPT_REPLACEMENT)
        };
        self.add_off_correct_map(offset_correction_pos, self.cumulative_diff);
        Some(return_value)
    }

    /// The UTF-16 code units `surrogatePair.substring(begin, end)` of the
    /// current match.
    fn match_substring(&self, begin: usize, end: usize) -> &[u16] {
        &self.yytext_units()[begin..end]
    }

    /// `(char) Integer.parseInt(surrogatePair.substring(begin, end)[, 16])` in
    /// a `try` block whose `catch` asserts `false` with `"Exception parsing
    /// <what> surrogate '<substring>'"`: `None` where Java's parse throws.
    fn parse_surrogate(&self, begin: usize, end: usize, hex: bool, what: &str) -> Option<u16> {
        let text = self.match_substring(begin, end);
        let parsed = if hex {
            Self::parse_hex(text)
        } else {
            Self::parse_int(text)
        };
        if parsed.is_none() {
            Self::assert_false(|| {
                format!(
                    "Exception parsing {what} surrogate '{}'",
                    String::from_utf16_lossy(text)
                )
            });
        }
        parsed.map(|v| v as u16)
    }

    /// The end of the surrogate-pair actions that match: the low surrogate in
    /// the entity segment, the correction for the 2 output units, and the
    /// high surrogate returned.
    fn output_surrogate_pair(&mut self, high_surrogate: u16) -> Option<i32> {
        // add (previously matched input length) + (this match length) - (substitution length)
        self.cumulative_diff += self.input_segment.length() as i32 + self.yylength() - 2;
        // position the correction at (already output length) + (substitution length)
        self.add_off_correct_map(self.output_char_count + 2, self.cumulative_diff);
        self.input_segment.clear();
        self.yybegin(YYINITIAL);
        Some(i32::from(high_surrogate))
    }

    /// The end of the surrogate-pair actions that do not match: consume only
    /// `'#'`.
    fn consume_only_hash(&mut self) -> Option<i32> {
        let surrogate_pair_length = self.yylength();
        self.yypushback(surrogate_pair_length - 1); // Consume only '#'
        self.input_segment.append_char(u16::from(b'#'));
        self.yybegin(NUMERIC_CHARACTER);
        None
    }

    /// `case 51:` of `HTMLStripCharFilter.nextChar()`: a decimal high
    /// surrogate followed by a decimal low surrogate.
    fn action_51(&mut self) -> Option<i32> {
        // Handle paired UTF-16 surrogates.
        // High surrogates are in decimal range [55296, 56319]
        let high_surrogate = self.parse_surrogate(1, 6, false, "high").unwrap_or(0);
        if crate::jdk::character::is_high_surrogate(high_surrogate) {
            // Low surrogates are in decimal range [56320, 57343]
            let low_surrogate = self.parse_surrogate(9, 14, false, "low").unwrap_or(0);
            if crate::jdk::character::is_low_surrogate(low_surrogate) {
                self.output_segment = Segment::Entity;
                self.output_segment_mut().clear();
                self.output_segment_mut().unsafe_write(low_surrogate);
                return self.output_surrogate_pair(high_surrogate);
            }
        }
        self.consume_only_hash()
    }

    /// `case 52:` of `HTMLStripCharFilter.nextChar()`: a decimal high
    /// surrogate followed by a hexadecimal low surrogate.
    fn action_52(&mut self) -> Option<i32> {
        // Handle paired UTF-16 surrogates.
        // High surrogates are in decimal range [55296, 56319]
        let high_surrogate = self.parse_surrogate(1, 6, false, "high").unwrap_or(0);
        if crate::jdk::character::is_high_surrogate(high_surrogate) {
            self.output_segment = Segment::Entity;
            self.output_segment_mut().clear();
            if let Some(low_surrogate) = self.parse_surrogate(10, 14, true, "low") {
                self.output_segment_mut().unsafe_write(low_surrogate);
            }
            return self.output_surrogate_pair(high_surrogate);
        }
        self.consume_only_hash()
    }

    /// `case 53:` of `HTMLStripCharFilter.nextChar()`: a hexadecimal high
    /// surrogate followed by a decimal low surrogate.
    fn action_53(&mut self) -> Option<i32> {
        // Handle paired UTF-16 surrogates.
        let high_surrogate = self.parse_surrogate(2, 6, true, "high").unwrap_or(0);
        // Low surrogates are in decimal range [56320, 57343]
        let low_surrogate = self.parse_surrogate(9, 14, false, "low").unwrap_or(0);
        if crate::jdk::character::is_low_surrogate(low_surrogate) {
            self.output_segment = Segment::Entity;
            self.output_segment_mut().clear();
            self.output_segment_mut().unsafe_write(low_surrogate);
            return self.output_surrogate_pair(high_surrogate);
        }
        self.consume_only_hash()
    }

    /// `case 54:` of `HTMLStripCharFilter.nextChar()`: a hexadecimal high
    /// surrogate followed by a hexadecimal low surrogate.
    fn action_54(&mut self) -> Option<i32> {
        // Handle paired UTF-16 surrogates.
        self.output_segment = Segment::Entity;
        self.output_segment_mut().clear();
        let high_surrogate = self.parse_surrogate(2, 6, true, "high").unwrap_or(0);
        if let Some(low_surrogate) = self.parse_surrogate(10, 14, true, "low") {
            self.output_segment_mut().unsafe_write(low_surrogate);
        }
        self.output_surrogate_pair(high_surrogate)
    }
}

impl CharReader for HTMLStripCharFilter {
    /// Equivalent to `HTMLStripCharFilter.read(char[], int, int)`: the units
    /// [`read_char`](HTMLStripCharFilter::read_char) returns, until `buf` is
    /// full or the stream ends (0 here where Java answers `-1`).
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        let mut i = 0;
        while i < buf.len() {
            let ch = self.read_char()?;
            if ch == -1 {
                break;
            }
            buf[i] = ch as u16;
            i += 1;
        }
        Ok(i)
    }

    /// Equivalent to `HTMLStripCharFilter.close()`, `yyclose()`.
    fn close(&mut self) -> Result<()> {
        self.yyclose()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for HTMLStripCharFilter {
    /// Equivalent to the inherited `BaseCharFilter.correct(int)`.
    fn correct(&self, current_off: i32) -> i32 {
        self.base.correct(current_off)
    }

    fn input(&self) -> &dyn CharReader {
        self.base.input()
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        self.base.input_mut()
    }
}

/// Equivalent to `org.apache.lucene.analysis.charfilter.HTMLStripCharFilter$TextSegment`,
/// an `OpenStringBuilder`
/// with a read position: the builder it extends is reached through
/// `Deref`/`DerefMut`.
#[derive(Clone, Debug)]
pub(crate) struct TextSegment {
    builder: OpenStringBuilder,
    /// The position from which the next char will be read.
    pos: usize,
}

impl TextSegment {
    /// Wraps the given buffer and sets this.len to the given length.
    ///
    /// Equivalent to `TextSegment(char[], int)`.
    #[allow(dead_code)] // unused, as in Java (`@SuppressWarnings("unused")`)
    pub(crate) fn from_array(buffer: Vec<u16>, length: usize) -> Self {
        Self {
            builder: OpenStringBuilder::from_array(buffer, length),
            pos: 0,
        }
    }

    /// Allocates an internal buffer of the given size.
    ///
    /// Equivalent to `TextSegment(int)`.
    pub(crate) fn with_size(size: usize) -> Self {
        Self {
            builder: OpenStringBuilder::with_size(size),
            pos: 0,
        }
    }

    /// Sets len = 0 and pos = 0.
    ///
    /// Equivalent to `TextSegment.clear()`.
    pub(crate) fn clear(&mut self) {
        self.builder.reset();
        self.restart();
    }

    /// Sets pos = 0
    ///
    /// Equivalent to `TextSegment.restart()`.
    pub(crate) fn restart(&mut self) {
        self.pos = 0;
    }

    /// Returns the next char in the segment.
    ///
    /// Equivalent to `TextSegment.nextChar()`; Java's `assert` is a debug
    /// assertion, and a read past the buffer panics where Java throws
    /// `ArrayIndexOutOfBoundsException`.
    pub(crate) fn next_char(&mut self) -> i32 {
        debug_assert!(
            !self.is_read(),
            "Attempting to read past the end of a segment."
        );
        let c = self.builder.get_array()[self.pos];
        self.pos += 1;
        i32::from(c)
    }

    /// Returns true when all characters in the text segment have been read
    ///
    /// Equivalent to `TextSegment.isRead()`.
    pub(crate) fn is_read(&self) -> bool {
        self.pos >= self.builder.length()
    }
}

impl Deref for TextSegment {
    type Target = OpenStringBuilder;

    fn deref(&self) -> &OpenStringBuilder {
        &self.builder
    }
}

impl DerefMut for TextSegment {
    fn deref_mut(&mut self) -> &mut OpenStringBuilder {
        &mut self.builder
    }
}

// @generated begin skeleton by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
impl HTMLStripCharFilter {
    /// Translates raw input code points to DFA table row.
    ///
    /// Equivalent to `HTMLStripCharFilter.zzCMap(int)`.
    fn zz_cmap(input: i32) -> i32 {
        let offset = input & 255;
        if offset == input {
            super::html_strip_char_filter_tables::ZZ_CMAP_BLOCKS[offset as usize]
        } else {
            super::html_strip_char_filter_tables::ZZ_CMAP_BLOCKS
                [(super::html_strip_char_filter_tables::ZZ_CMAP_TOP[(input >> 8) as usize] | offset)
                    as usize]
        }
    }

    /// Refills the input buffer.
    ///
    /// Equivalent to `HTMLStripCharFilter.zzRefill()`: returns `false` iff there was new
    /// input. The reader is the filter's input, Java's `zzReader`.
    ///
    /// # Errors
    ///
    /// What the reader raises, Java's `IOException` for a reader that returns no
    /// character, and the `ArrayIndexOutOfBoundsException` of `System.arraycopy`
    /// Java raises for a buffer shorter than the text it holds.
    fn zz_refill(&mut self) -> rucene::error::Result<bool> {
        /* first: make room (if you can) */
        if self.zz_start_read > 0 {
            self.zz_end_read += self.zz_final_high_surrogate;
            self.zz_final_high_surrogate = 0;
            crate::jdk::system::arraycopy_within(
                &mut self.zz_buffer,
                self.zz_start_read,
                0,
                self.zz_end_read as i64 - self.zz_start_read as i64,
            )?;

            /* translate stored positions */
            self.zz_end_read -= self.zz_start_read;
            self.zz_current_pos -= self.zz_start_read;
            self.zz_marked_pos -= self.zz_start_read;
            self.zz_start_read = 0;
        }

        /* is the buffer big enough? */
        if self.zz_current_pos >= self.zz_buffer.len() - self.zz_final_high_surrogate {
            /* if not: blow it up */
            let mut new_buffer = vec![0u16; self.zz_buffer.len() * 2];
            new_buffer[..self.zz_buffer.len()].copy_from_slice(&self.zz_buffer);
            self.zz_buffer = new_buffer;
            self.zz_end_read += self.zz_final_high_surrogate;
            self.zz_final_high_surrogate = 0;
        }

        /* fill the buffer with new input */
        let requested = self.zz_buffer.len() as i64 - self.zz_end_read as i64;
        let num_read = self.read_units(self.zz_end_read, requested)?;

        /* not supposed to occur according to specification of java.io.Reader */
        if num_read == 0 {
            return Err(rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Io(
            std::io::Error::other(
                "Reader returned 0 characters. See JFlex examples/zero-reader for a workaround.",
            ),
        )));
        }
        if num_read > 0 {
            self.zz_end_read += num_read as usize;
            if crate::jdk::character::is_high_surrogate(self.zz_buffer[self.zz_end_read - 1]) {
                if num_read == requested {
                    // We requested too few chars to encode a full Unicode character
                    self.zz_end_read -= 1;
                    self.zz_final_high_surrogate = 1;
                } else {
                    // There is room in the buffer for at least one more char
                    let c = self.read_unit()?; // Expecting to read a paired low surrogate char
                    if c == -1 {
                        return Ok(true);
                    } else {
                        self.zz_buffer[self.zz_end_read] = c as u16;
                        self.zz_end_read += 1;
                    }
                }
            }
            /* potentially more input available */
            return Ok(false);
        }

        /* numRead < 0 ==> end of stream */
        Ok(true)
    }

    /// `zzReader.read(zzBuffer, off, len)`: the number of code units stored at
    /// `off`, or `-1` at the end of the stream.
    ///
    /// # Errors
    ///
    /// What the reader raises, and, for a negative `len` (a buffer shrunk under
    /// its text), the `IndexOutOfBoundsException` of `java.io.Reader.read`.
    fn read_units(&mut self, off: usize, len: i64) -> rucene::error::Result<i64> {
        if len < 0 {
            return Err(rucene::error::LuceneError::from(
                rucene::error::LuceneErrorKind::Unchecked {
                    class: "java.lang.IndexOutOfBoundsException",
                    message: Some(format!(
                        "Range [{off}, {off} + {len}) out of bounds for length {}",
                        self.zz_buffer.len()
                    )),
                    cause: None,
                },
            ));
        }
        let len = len as usize;
        if len == 0 {
            // `Reader.read` answers 0 for an empty request.
            return Ok(0);
        }
        let got = self
            .base
            .input_mut()
            .read(&mut self.zz_buffer[off..off + len])?;
        Ok(if got == 0 { -1 } else { got as i64 })
    }

    /// `zzReader.read()`: the next code unit, or `-1` at the end of the stream.
    ///
    /// # Errors
    ///
    /// What the reader raises.
    fn read_unit(&mut self) -> rucene::error::Result<i32> {
        let mut one = [0u16; 1];
        if self.base.input_mut().read(&mut one)? == 0 {
            return Ok(-1);
        }
        Ok(i32::from(one[0]))
    }

    /// Closes the input reader.
    ///
    /// Equivalent to `HTMLStripCharFilter.yyclose()`. The reader is the filter's input, Java's `zzReader`.
    ///
    /// # Errors
    ///
    /// What closing the reader raises.
    #[allow(dead_code)] // unused, as in Java (`@SuppressWarnings("unused")`)
    fn yyclose(&mut self) -> rucene::error::Result<()> {
        self.zz_at_eof = true; // indicate end of file
        self.zz_end_read = self.zz_start_read; // invalidate buffer
        self.base.input_mut().close()
    }

    /// Resets the scanner to read from a new input stream.
    ///
    /// Does not close the old reader.
    ///
    /// All internal variables are reset, the old input stream **cannot** be
    /// reused (internal buffer is discarded and lost). Lexical state is set to
    /// `ZZ_INITIAL`.
    ///
    /// Internal scan buffer is resized down to its initial length, if it has
    /// grown.
    ///
    /// Equivalent to `HTMLStripCharFilter.yyreset(java.io.Reader)`; the scanner keeps reading the filter's input, which Java's (never called) `zzReader = reader` would replace.
    #[allow(dead_code)] // unused, as in Java (`@SuppressWarnings("unused")`)
    fn yyreset(&mut self) {
        self.zz_eof_done = false;
        self.yy_reset_position();
        self.zz_lexical_state = YYINITIAL;
        if self.zz_buffer.len() > ZZ_BUFFERSIZE {
            self.zz_buffer = vec![0; ZZ_BUFFERSIZE];
        }
    }

    /// Resets the input position.
    ///
    /// Equivalent to `HTMLStripCharFilter.yyResetPosition()`.
    fn yy_reset_position(&mut self) {
        self.zz_at_bol = true;
        self.zz_at_eof = false;
        self.zz_current_pos = 0;
        self.zz_marked_pos = 0;
        self.zz_start_read = 0;
        self.zz_end_read = 0;
        self.zz_final_high_surrogate = 0;
        self.yyline = 0;
        self.yycolumn = 0;
        self.yychar = 0;
    }

    /// Returns whether the scanner has reached the end of the reader it reads from.
    ///
    /// Equivalent to `HTMLStripCharFilter.yyatEOF()`.
    #[allow(dead_code)] // unused, as in Java (`@SuppressWarnings("unused")`)
    fn yyat_eof(&self) -> bool {
        self.zz_at_eof
    }

    /// Returns the current lexical state.
    ///
    /// Equivalent to `HTMLStripCharFilter.yystate()`.
    #[allow(dead_code)] // unused, as in Java (`@SuppressWarnings("unused")`)
    fn yystate(&self) -> i32 {
        self.zz_lexical_state
    }

    /// Enters a new lexical state.
    ///
    /// Equivalent to `HTMLStripCharFilter.yybegin(int)`.
    fn yybegin(&mut self, new_state: i32) {
        self.zz_lexical_state = new_state;
    }

    /// Returns the text matched by the current regular expression.
    ///
    /// Equivalent to `HTMLStripCharFilter.yytext()`; a lone surrogate, which a Java
    /// `String` holds, becomes U+FFFD in a Rust `String`.
    #[allow(dead_code)] // unused, as in Java (`@SuppressWarnings("unused")`)
    fn yytext(&self) -> String {
        String::from_utf16_lossy(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos])
    }

    /// Returns the UTF-16 code unit at the given position from the matched text, a
    /// value from 0 to `yylength()-1`.
    ///
    /// Equivalent to `HTMLStripCharFilter.yycharat(int)`.
    #[allow(dead_code)] // unused, as in Java (`@SuppressWarnings("unused")`)
    fn yycharat(&self, position: usize) -> u16 {
        self.zz_buffer[self.zz_start_read + position]
    }

    /// How many characters (UTF-16 code units) were matched.
    ///
    /// Equivalent to `HTMLStripCharFilter.yylength()`.
    fn yylength(&self) -> i32 {
        (self.zz_marked_pos - self.zz_start_read) as i32
    }

    /// Reports an error that occurred while scanning.
    ///
    /// In a well-formed scanner (no or only correct usage of `yypushback(int)` and
    /// a match-all fallback rule) this method will only be called with things that
    /// "Can't Possibly Happen".
    ///
    /// Equivalent to `HTMLStripCharFilter.zzScanError(int)`, which throws a
    /// `java.lang.Error`.
    ///
    /// # Panics
    ///
    /// Always, with the message of `error_code`, as Java throws an `Error`.
    fn zz_scan_error(error_code: usize) -> ! {
        let message = ZZ_ERROR_MSG
            .get(error_code)
            .copied()
            .unwrap_or(ZZ_ERROR_MSG[ZZ_UNKNOWN_ERROR]);
        panic!("{message}");
    }

    /// Pushes the specified amount of characters back into the input stream.
    ///
    /// They will be read again by then next call of the scanning method.
    ///
    /// Equivalent to `HTMLStripCharFilter.yypushback(int)`.
    ///
    /// # Panics
    ///
    /// When `number` is greater than [`yylength`](Self::yylength), as Java's
    /// `zzScanError(ZZ_PUSHBACK_2BIG)` throws an `Error`.
    fn yypushback(&mut self, number: i32) {
        if number > self.yylength() {
            Self::zz_scan_error(ZZ_PUSHBACK_2BIG);
        }
        self.zz_marked_pos = (self.zz_marked_pos as i64 - i64::from(number)) as usize;
    }

    /// Contains user EOF-code, which will be executed exactly once, when the end
    /// of file is reached.
    ///
    /// Equivalent to `HTMLStripCharFilter.zzDoEOF()`; the user EOF-code is
    /// [`eof_code`](Self::eof_code).
    fn zz_do_eof(&mut self) {
        if !self.zz_eof_done {
            self.zz_eof_done = true;
            self.eof_code();
        }
    }

    /// Resumes scanning until the next regular expression is matched, the end of
    /// input is encountered or an I/O-Error occurs.
    ///
    /// Equivalent to `HTMLStripCharFilter.nextChar()`. The reader is the filter's input, Java's `zzReader`.
    ///
    /// # Errors
    ///
    /// What the reader raises, and the `IndexOutOfBoundsException` of the JDK's
    /// `Character` methods for a buffer shorter than the text it holds.
    ///
    /// # Panics
    ///
    /// When no rule matches, as Java's `zzScanError(ZZ_NO_MATCH)` throws an
    /// `Error`; a grammar with a catch-all rule makes this unreachable.
    fn next_char(&mut self) -> rucene::error::Result<i32> {
        let mut zz_input: i32;
        let mut zz_action: i32;

        // cached fields:
        let mut zz_current_pos_l: usize;
        let mut zz_marked_pos_l: usize;
        let mut zz_end_read_l = self.zz_end_read;
        let zz_trans_l: &[i32] = &super::html_strip_char_filter_tables::ZZ_TRANS;
        let zz_row_map_l: &[i32] = &super::html_strip_char_filter_tables::ZZ_ROWMAP;
        let zz_attr_l: &[i32] = &super::html_strip_char_filter_tables::ZZ_ATTRIBUTE;

        loop {
            zz_marked_pos_l = self.zz_marked_pos;

            self.yychar += zz_marked_pos_l as i64 - self.zz_start_read as i64;

            zz_action = -1;

            zz_current_pos_l = zz_marked_pos_l;
            self.zz_current_pos = zz_marked_pos_l;
            self.zz_start_read = zz_marked_pos_l;

            self.zz_state = ZZ_LEXSTATE[self.zz_lexical_state as usize];

            // set up zzAction for empty match case:
            let mut zz_attributes = zz_attr_l[self.zz_state as usize];
            if (zz_attributes & 1) == 1 {
                zz_action = self.zz_state;
            }

            // zzForAction:
            loop {
                if zz_current_pos_l < zz_end_read_l {
                    zz_input = crate::jdk::character::checked_code_point_at(
                        &self.zz_buffer,
                        zz_current_pos_l,
                        zz_end_read_l,
                    )?;
                    zz_current_pos_l += crate::jdk::character::char_count(zz_input);
                } else if self.zz_at_eof {
                    zz_input = YYEOF;
                    break;
                } else {
                    // store back cached positions
                    self.zz_current_pos = zz_current_pos_l;
                    self.zz_marked_pos = zz_marked_pos_l;
                    let eof = self.zz_refill()?;
                    // get translated positions and possibly new buffer
                    zz_current_pos_l = self.zz_current_pos;
                    zz_marked_pos_l = self.zz_marked_pos;
                    zz_end_read_l = self.zz_end_read;
                    if eof {
                        zz_input = YYEOF;
                        break;
                    } else {
                        zz_input = crate::jdk::character::checked_code_point_at(
                            &self.zz_buffer,
                            zz_current_pos_l,
                            zz_end_read_l,
                        )?;
                        zz_current_pos_l += crate::jdk::character::char_count(zz_input);
                    }
                }
                let zz_next = zz_trans_l
                    [(zz_row_map_l[self.zz_state as usize] + Self::zz_cmap(zz_input)) as usize];
                if zz_next == -1 {
                    break;
                }
                self.zz_state = zz_next;

                zz_attributes = zz_attr_l[self.zz_state as usize];
                if (zz_attributes & 1) == 1 {
                    zz_action = self.zz_state;
                    zz_marked_pos_l = zz_current_pos_l;
                    if (zz_attributes & 8) == 8 {
                        break;
                    }
                }
            }

            // store back cached position
            self.zz_marked_pos = zz_marked_pos_l;

            if zz_input == YYEOF && self.zz_start_read == self.zz_current_pos {
                self.zz_at_eof = true;
                self.zz_do_eof();
                {
                    return Ok(self.eof_return_value);
                }
            } else {
                match if zz_action < 0 {
                    zz_action
                } else {
                    super::html_strip_char_filter_tables::ZZ_ACTION[zz_action as usize]
                } {
                    1 => {
                        if let Some(value) = self.action_1() {
                            return Ok(value);
                        }
                    }
                    2 => {
                        if let Some(value) = self.action_2() {
                            return Ok(value);
                        }
                    }
                    3 => {
                        if let Some(value) = self.action_3() {
                            return Ok(value);
                        }
                    }
                    4 => {
                        if let Some(value) = self.action_4() {
                            return Ok(value);
                        }
                    }
                    5 => {
                        if let Some(value) = self.action_5() {
                            return Ok(value);
                        }
                    }
                    6 => {
                        if let Some(value) = self.action_6() {
                            return Ok(value);
                        }
                    }
                    7 => {
                        if let Some(value) = self.action_7() {
                            return Ok(value);
                        }
                    }
                    8 => {
                        if let Some(value) = self.action_8() {
                            return Ok(value);
                        }
                    }
                    9 => {
                        if let Some(value) = self.action_9() {
                            return Ok(value);
                        }
                    }
                    10 => {
                        if let Some(value) = self.action_10() {
                            return Ok(value);
                        }
                    }
                    11 => {
                        if let Some(value) = self.action_11() {
                            return Ok(value);
                        }
                    }
                    12 => {
                        if let Some(value) = self.action_12() {
                            return Ok(value);
                        }
                    }
                    13 => {
                        if let Some(value) = self.action_13() {
                            return Ok(value);
                        }
                    }
                    14 => {
                        if let Some(value) = self.action_14() {
                            return Ok(value);
                        }
                    }
                    15 => {}
                    16 => {
                        self.restore_state = SCRIPT_COMMENT;
                        self.yybegin(DOUBLE_QUOTED_STRING);
                    }
                    17 => {
                        self.restore_state = SCRIPT_COMMENT;
                        self.yybegin(SINGLE_QUOTED_STRING);
                    }
                    18 => {
                        if let Some(value) = self.action_18() {
                            return Ok(value);
                        }
                    }
                    19 => {
                        if let Some(value) = self.action_19() {
                            return Ok(value);
                        }
                    }
                    20 => {
                        if let Some(value) = self.action_20() {
                            return Ok(value);
                        }
                    }
                    21 => {
                        if let Some(value) = self.action_21() {
                            return Ok(value);
                        }
                    }
                    22 => {
                        self.previous_restore_state = self.restore_state;
                        self.restore_state = SERVER_SIDE_INCLUDE;
                        self.yybegin(DOUBLE_QUOTED_STRING);
                    }
                    23 => {
                        self.previous_restore_state = self.restore_state;
                        self.restore_state = SERVER_SIDE_INCLUDE;
                        self.yybegin(SINGLE_QUOTED_STRING);
                    }
                    24 => {
                        self.yybegin(self.restore_state);
                        self.restore_state = self.previous_restore_state;
                    }
                    25 => {
                        if let Some(value) = self.action_25() {
                            return Ok(value);
                        }
                    }
                    26 => {
                        if let Some(value) = self.action_26() {
                            return Ok(value);
                        }
                    }
                    27 => {
                        if let Some(value) = self.action_27() {
                            return Ok(value);
                        }
                    }
                    28 => {
                        if let Some(value) = self.action_28() {
                            return Ok(value);
                        }
                    }
                    29 => {
                        self.restore_state = STYLE_COMMENT;
                        self.yybegin(DOUBLE_QUOTED_STRING);
                    }
                    30 => {
                        self.restore_state = STYLE_COMMENT;
                        self.yybegin(SINGLE_QUOTED_STRING);
                    }
                    31 => {
                        if let Some(value) = self.action_31() {
                            return Ok(value);
                        }
                    }
                    32 => {
                        if let Some(value) = self.action_32() {
                            return Ok(value);
                        }
                    }
                    33 => {
                        if let Some(value) = self.action_33() {
                            return Ok(value);
                        }
                    }
                    34 => {
                        if let Some(value) = self.action_34() {
                            return Ok(value);
                        }
                    }
                    35 => {
                        if let Some(value) = self.action_35() {
                            return Ok(value);
                        }
                    }
                    36 => {
                        self.yybegin(SCRIPT);
                    }
                    37 => {
                        if let Some(value) = self.action_37() {
                            return Ok(value);
                        }
                    }
                    38 => {
                        if let Some(value) = self.action_38() {
                            return Ok(value);
                        }
                    }
                    39 => {
                        self.yybegin(self.restore_state);
                    }
                    40 => {
                        self.yybegin(STYLE);
                    }
                    41 => {
                        self.yybegin(SCRIPT_COMMENT);
                    }
                    42 => {
                        self.yybegin(STYLE_COMMENT);
                    }
                    43 => {
                        self.restore_state = COMMENT;
                        self.yybegin(SERVER_SIDE_INCLUDE);
                    }
                    44 => {
                        self.restore_state = SCRIPT_COMMENT;
                        self.yybegin(SERVER_SIDE_INCLUDE);
                    }
                    45 => {
                        self.restore_state = STYLE_COMMENT;
                        self.yybegin(SERVER_SIDE_INCLUDE);
                    }
                    46 => {
                        if let Some(value) = self.action_46() {
                            return Ok(value);
                        }
                    }
                    47 => {
                        if let Some(value) = self.action_47() {
                            return Ok(value);
                        }
                    }
                    48 => {
                        if let Some(value) = self.action_48() {
                            return Ok(value);
                        }
                    }
                    49 => {
                        if let Some(value) = self.action_49() {
                            return Ok(value);
                        }
                    }
                    50 => {
                        if let Some(value) = self.action_50() {
                            return Ok(value);
                        }
                    }
                    51 => {
                        if let Some(value) = self.action_51() {
                            return Ok(value);
                        }
                    }
                    52 => {
                        if let Some(value) = self.action_52() {
                            return Ok(value);
                        }
                    }
                    53 => {
                        if let Some(value) = self.action_53() {
                            return Ok(value);
                        }
                    }
                    54 => {
                        if let Some(value) = self.action_54() {
                            return Ok(value);
                        }
                    }
                    55..=108 => {}
                    _ => Self::zz_scan_error(ZZ_NO_MATCH),
                }
            }
        }
    }
}
// @generated end skeleton
