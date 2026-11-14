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

//! The JFlex scanner behind [`WikipediaTokenizer`](super::wikipedia_tokenizer::WikipediaTokenizer),
//! ported from `org.apache.lucene.analysis.wikipedia.WikipediaTokenizerImpl`.
//!
//! Lucene 10.5.0 generates the Java class with JFlex 1.8.2 from
//! `WikipediaTokenizerImpl.jflex`. `tools/translate/jflex_java_to_rust.py`
//! translates the generated parts: the DFA tables (into
//! `wikipedia_tokenizer_impl_tables.rs`) and the JFlex skeleton with its
//! actions (into the `@generated` regions of this file). The user code of the
//! grammar is translated by hand, outside the regions.
//!
//! The JFlex buffer holds the UTF-16 code units a
//! [`CharReader`](rucene::analysis::CharReader) hands out, as Java's `char[]`
//! holds those of a `java.io.Reader`; positions count UTF-16 code units.

use rucene::analysis::CharTermAttribute;

use super::wikipedia_tokenizer;

// @generated begin constants by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
// From Lucene 10.5.0's lucene/analysis/common/src/java/org/apache/lucene/analysis/wikipedia/WikipediaTokenizerImpl.java
// (sha256 a4070e5da820c5c15e8062ae43cb41a3527aad04ee6740128d682775c984cb58).

/// This character denotes the end of file.
///
/// Equivalent to `WikipediaTokenizerImpl.YYEOF`.
pub const YYEOF: i32 = -1;

/// Initial size of the lookahead buffer.
///
/// Equivalent to `WikipediaTokenizerImpl.ZZ_BUFFERSIZE`.
const ZZ_BUFFERSIZE: usize = 4096;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.YYINITIAL`.
pub const YYINITIAL: i32 = 0;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.CATEGORY_STATE`.
pub const CATEGORY_STATE: i32 = 2;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.INTERNAL_LINK_STATE`.
pub const INTERNAL_LINK_STATE: i32 = 4;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.EXTERNAL_LINK_STATE`.
pub const EXTERNAL_LINK_STATE: i32 = 6;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.TWO_SINGLE_QUOTES_STATE`.
pub const TWO_SINGLE_QUOTES_STATE: i32 = 8;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.THREE_SINGLE_QUOTES_STATE`.
pub const THREE_SINGLE_QUOTES_STATE: i32 = 10;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.FIVE_SINGLE_QUOTES_STATE`.
pub const FIVE_SINGLE_QUOTES_STATE: i32 = 12;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.DOUBLE_EQUALS_STATE`.
pub const DOUBLE_EQUALS_STATE: i32 = 14;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.DOUBLE_BRACE_STATE`.
pub const DOUBLE_BRACE_STATE: i32 = 16;

/// Lexical state.
///
/// Equivalent to `WikipediaTokenizerImpl.STRING`.
pub const STRING: i32 = 18;

/// `ZZ_LEXSTATE[l]` is the state in the DFA for the lexical state `l`;
/// `ZZ_LEXSTATE[l+1]` is the state in the DFA for the lexical state `l` at the
/// beginning of a line; `l` is of the form `l = 2*k`, `k` a non negative integer.
const ZZ_LEXSTATE: [i32; 20] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9];

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
/// Equivalent to `WikipediaTokenizerImpl.ALPHANUM`, `WikipediaTokenizer.ALPHANUM_ID`.
pub const ALPHANUM: i32 = wikipedia_tokenizer::ALPHANUM_ID;
/// Equivalent to `WikipediaTokenizerImpl.APOSTROPHE`, `WikipediaTokenizer.APOSTROPHE_ID`.
pub const APOSTROPHE: i32 = wikipedia_tokenizer::APOSTROPHE_ID;
/// Equivalent to `WikipediaTokenizerImpl.ACRONYM`, `WikipediaTokenizer.ACRONYM_ID`.
pub const ACRONYM: i32 = wikipedia_tokenizer::ACRONYM_ID;
/// Equivalent to `WikipediaTokenizerImpl.COMPANY`, `WikipediaTokenizer.COMPANY_ID`.
pub const COMPANY: i32 = wikipedia_tokenizer::COMPANY_ID;
/// Equivalent to `WikipediaTokenizerImpl.EMAIL`, `WikipediaTokenizer.EMAIL_ID`.
pub const EMAIL: i32 = wikipedia_tokenizer::EMAIL_ID;
/// Equivalent to `WikipediaTokenizerImpl.HOST`, `WikipediaTokenizer.HOST_ID`.
pub const HOST: i32 = wikipedia_tokenizer::HOST_ID;
/// Equivalent to `WikipediaTokenizerImpl.NUM`, `WikipediaTokenizer.NUM_ID`.
pub const NUM: i32 = wikipedia_tokenizer::NUM_ID;
/// Equivalent to `WikipediaTokenizerImpl.CJ`, `WikipediaTokenizer.CJ_ID`.
pub const CJ: i32 = wikipedia_tokenizer::CJ_ID;
/// Equivalent to `WikipediaTokenizerImpl.INTERNAL_LINK`, `WikipediaTokenizer.INTERNAL_LINK_ID`.
pub const INTERNAL_LINK: i32 = wikipedia_tokenizer::INTERNAL_LINK_ID;
/// Equivalent to `WikipediaTokenizerImpl.EXTERNAL_LINK`, `WikipediaTokenizer.EXTERNAL_LINK_ID`.
pub const EXTERNAL_LINK: i32 = wikipedia_tokenizer::EXTERNAL_LINK_ID;
/// Equivalent to `WikipediaTokenizerImpl.CITATION`, `WikipediaTokenizer.CITATION_ID`.
pub const CITATION: i32 = wikipedia_tokenizer::CITATION_ID;
/// Equivalent to `WikipediaTokenizerImpl.CATEGORY`, `WikipediaTokenizer.CATEGORY_ID`.
pub const CATEGORY: i32 = wikipedia_tokenizer::CATEGORY_ID;
/// Equivalent to `WikipediaTokenizerImpl.BOLD`, `WikipediaTokenizer.BOLD_ID`.
pub const BOLD: i32 = wikipedia_tokenizer::BOLD_ID;
/// Equivalent to `WikipediaTokenizerImpl.ITALICS`, `WikipediaTokenizer.ITALICS_ID`.
pub const ITALICS: i32 = wikipedia_tokenizer::ITALICS_ID;
/// Equivalent to `WikipediaTokenizerImpl.BOLD_ITALICS`, `WikipediaTokenizer.BOLD_ITALICS_ID`.
pub const BOLD_ITALICS: i32 = wikipedia_tokenizer::BOLD_ITALICS_ID;
/// Equivalent to `WikipediaTokenizerImpl.HEADING`, `WikipediaTokenizer.HEADING_ID`.
pub const HEADING: i32 = wikipedia_tokenizer::HEADING_ID;
/// Equivalent to `WikipediaTokenizerImpl.SUB_HEADING`, `WikipediaTokenizer.SUB_HEADING_ID`.
pub const SUB_HEADING: i32 = wikipedia_tokenizer::SUB_HEADING_ID;
/// Equivalent to `WikipediaTokenizerImpl.EXTERNAL_LINK_URL`, `WikipediaTokenizer.EXTERNAL_LINK_URL_ID`.
pub const EXTERNAL_LINK_URL: i32 = wikipedia_tokenizer::EXTERNAL_LINK_URL_ID;

/// Equivalent to `WikipediaTokenizerImpl.TOKEN_TYPES`, the array
/// `WikipediaTokenizer.TOKEN_TYPES` itself.
pub static TOKEN_TYPES: &[&str; 18] = &wikipedia_tokenizer::TOKEN_TYPES;

/// JFlex-generated tokenizer that is aware of Wikipedia syntax.
///
/// Equivalent to `org.apache.lucene.analysis.wikipedia.WikipediaTokenizerImpl`.
/// Java's scanner holds its `java.io.Reader`; a reader is owned by the
/// [`Tokenizer`](rucene::analysis::Tokenizer), so the methods that read take
/// it as a parameter.
#[derive(Debug)]
pub(crate) struct WikipediaTokenizerImpl {
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
    #[allow(dead_code)] // `@SuppressWarnings("unused")`, as in Java
    zz_eof_done: bool,
    // @generated end fields
    current_tok_type: i32,
    num_balanced: i32,
    position_inc: i32,
    num_link_toks: i32,
    /// Anytime we start a new on a Wiki reserved token (category, link, etc.)
    /// this value will be 0, otherwise it will be the number of tokens seen
    /// this can be useful for detecting when a new reserved token is
    /// encountered see <https://issues.apache.org/jira/browse/LUCENE-1133>
    num_wiki_tokens_seen: i32,
    /// The current token decoded to `char`s for [`get_text`](Self::get_text),
    /// kept across tokens. Java has no such buffer: its term holds the UTF-16
    /// code units `zzBuffer` holds.
    text_scratch: Vec<char>,
}

impl WikipediaTokenizerImpl {
    /// Creates a new scanner.
    ///
    /// Equivalent to `WikipediaTokenizerImpl(java.io.Reader in)` and the
    /// field initializers; the reader is passed to each reading method
    /// instead.
    pub(crate) fn new() -> Self {
        Self {
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
            current_tok_type: 0,
            num_balanced: 0,
            position_inc: 1,
            num_link_toks: 0,
            num_wiki_tokens_seen: 0,
            text_scratch: Vec::new(),
        }
    }

    /// Returns the number of tokens seen inside a category or link, etc.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.getNumWikiTokensSeen()`.
    pub fn get_num_wiki_tokens_seen(&self) -> i32 {
        self.num_wiki_tokens_seen
    }

    /// Equivalent to `WikipediaTokenizerImpl.yychar()`: UTF-16 code units,
    /// truncated to `int` ("jflex supports > 2GB docs but not lucene").
    pub fn yychar(&self) -> i32 {
        self.yychar as i32
    }

    /// Equivalent to `WikipediaTokenizerImpl.getPositionIncrement()`.
    pub fn get_position_increment(&self) -> i32 {
        self.position_inc
    }

    /// Fills Lucene token with the current token text.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.getText(CharTermAttribute)`,
    /// `t.copyBuffer(zzBuffer, zzStartRead, zzMarkedPos - zzStartRead)`. The
    /// attribute holds `char`s, so the token's UTF-16 code units are decoded
    /// first (a lone surrogate would become U+FFFD).
    pub(crate) fn get_text(&mut self, t: &mut dyn CharTermAttribute) {
        let units = &self.zz_buffer[self.zz_start_read..self.zz_marked_pos];
        self.text_scratch.clear();
        self.text_scratch.extend(
            char::decode_utf16(units.iter().copied())
                .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)),
        );
        t.copy_buffer(&self.text_scratch, 0, self.text_scratch.len());
    }

    /// Equivalent to `WikipediaTokenizerImpl.setText(StringBuilder)`: appends
    /// the matched UTF-16 code units to `buffer` (Java's `StringBuilder`) and
    /// returns their number.
    pub(crate) fn set_text(&self, buffer: &mut Vec<u16>) -> i32 {
        let length = self.zz_marked_pos - self.zz_start_read;
        buffer.extend_from_slice(&self.zz_buffer[self.zz_start_read..self.zz_start_read + length]);
        length as i32
    }

    /// Equivalent to the package-private `WikipediaTokenizerImpl.reset()`.
    pub(crate) fn reset(&mut self) {
        self.current_tok_type = 0;
        self.num_balanced = 0;
        self.position_inc = 1;
        self.num_link_toks = 0;
        self.num_wiki_tokens_seen = 0;
    }
}

// @generated begin skeleton by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
#[allow(dead_code)] // the public JFlex API of a package-private class, used in part
impl WikipediaTokenizerImpl {
    /// Translates raw input code points to DFA table row.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.zzCMap(int)`.
    fn zz_cmap(input: i32) -> i32 {
        let offset = input & 255;
        if offset == input {
            super::wikipedia_tokenizer_impl_tables::ZZ_CMAP_BLOCKS[offset as usize]
        } else {
            super::wikipedia_tokenizer_impl_tables::ZZ_CMAP_BLOCKS
                [(super::wikipedia_tokenizer_impl_tables::ZZ_CMAP_TOP[(input >> 8) as usize]
                    | offset) as usize]
        }
    }

    /// Refills the input buffer.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.zzRefill()`: returns `false` iff there was new
    /// input. The reader is the one passed to the call: the Java scanner keeps its `zzReader`, a Rust `Tokenizer` lends its input for the length of each call.
    ///
    /// # Errors
    ///
    /// What the reader raises, Java's `IOException` for a reader that returns no
    /// character, and the `ArrayIndexOutOfBoundsException` of `System.arraycopy`
    /// Java raises for a buffer shorter than the text it holds.
    fn zz_refill(
        &mut self,
        reader: &mut dyn rucene::analysis::CharReader,
    ) -> rucene::error::Result<bool> {
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
        let num_read = self.read_units(reader, self.zz_end_read, requested)?;

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
                    let c = self.read_unit(reader)?; // Expecting to read a paired low surrogate char
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
    fn read_units(
        &mut self,
        reader: &mut dyn rucene::analysis::CharReader,
        off: usize,
        len: i64,
    ) -> rucene::error::Result<i64> {
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
        let got = reader.read(&mut self.zz_buffer[off..off + len])?;
        Ok(if got == 0 { -1 } else { got as i64 })
    }

    /// `zzReader.read()`: the next code unit, or `-1` at the end of the stream.
    ///
    /// # Errors
    ///
    /// What the reader raises.
    fn read_unit(
        &mut self,
        reader: &mut dyn rucene::analysis::CharReader,
    ) -> rucene::error::Result<i32> {
        let mut one = [0u16; 1];
        if reader.read(&mut one)? == 0 {
            return Ok(-1);
        }
        Ok(i32::from(one[0]))
    }

    /// Closes the input reader.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.yyclose()`. The reader is the one passed to the call: the Java scanner keeps its `zzReader`, a Rust `Tokenizer` lends its input for the length of each call.
    ///
    /// # Errors
    ///
    /// What closing the reader raises.
    pub fn yyclose(
        &mut self,
        reader: &mut dyn rucene::analysis::CharReader,
    ) -> rucene::error::Result<()> {
        self.zz_at_eof = true; // indicate end of file
        self.zz_end_read = self.zz_start_read; // invalidate buffer
        reader.close()
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
    /// Equivalent to `WikipediaTokenizerImpl.yyreset(java.io.Reader)`; the new reader is the one passed to the next reading call.
    pub fn yyreset(&mut self) {
        self.zz_eof_done = false;
        self.yy_reset_position();
        self.zz_lexical_state = YYINITIAL;
        if self.zz_buffer.len() > ZZ_BUFFERSIZE {
            self.zz_buffer = vec![0; ZZ_BUFFERSIZE];
        }
    }

    /// Resets the input position.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.yyResetPosition()`.
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
    /// Equivalent to `WikipediaTokenizerImpl.yyatEOF()`.
    pub fn yyat_eof(&self) -> bool {
        self.zz_at_eof
    }

    /// Returns the current lexical state.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.yystate()`.
    pub fn yystate(&self) -> i32 {
        self.zz_lexical_state
    }

    /// Enters a new lexical state.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.yybegin(int)`.
    pub fn yybegin(&mut self, new_state: i32) {
        self.zz_lexical_state = new_state;
    }

    /// Returns the text matched by the current regular expression.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.yytext()`; a lone surrogate, which a Java
    /// `String` holds, becomes U+FFFD in a Rust `String`.
    pub fn yytext(&self) -> String {
        String::from_utf16_lossy(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos])
    }

    /// Returns the UTF-16 code unit at the given position from the matched text, a
    /// value from 0 to `yylength()-1`.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.yycharat(int)`.
    pub fn yycharat(&self, position: usize) -> u16 {
        self.zz_buffer[self.zz_start_read + position]
    }

    /// How many characters (UTF-16 code units) were matched.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.yylength()`.
    pub fn yylength(&self) -> i32 {
        (self.zz_marked_pos - self.zz_start_read) as i32
    }

    /// Reports an error that occurred while scanning.
    ///
    /// In a well-formed scanner (no or only correct usage of `yypushback(int)` and
    /// a match-all fallback rule) this method will only be called with things that
    /// "Can't Possibly Happen".
    ///
    /// Equivalent to `WikipediaTokenizerImpl.zzScanError(int)`, which throws a
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
    /// Equivalent to `WikipediaTokenizerImpl.yypushback(int)`.
    ///
    /// # Panics
    ///
    /// When `number` is greater than [`yylength`](Self::yylength), as Java's
    /// `zzScanError(ZZ_PUSHBACK_2BIG)` throws an `Error`.
    pub fn yypushback(&mut self, number: i32) {
        if number > self.yylength() {
            Self::zz_scan_error(ZZ_PUSHBACK_2BIG);
        }
        self.zz_marked_pos = (self.zz_marked_pos as i64 - i64::from(number)) as usize;
    }

    /// Resumes scanning until the next regular expression is matched, the end of
    /// input is encountered or an I/O-Error occurs.
    ///
    /// Equivalent to `WikipediaTokenizerImpl.getNextToken()`. The reader is the one passed to the call: the Java scanner keeps its `zzReader`, a Rust `Tokenizer` lends its input for the length of each call.
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
    pub fn get_next_token(
        &mut self,
        reader: &mut dyn rucene::analysis::CharReader,
    ) -> rucene::error::Result<i32> {
        let mut zz_input: i32;
        let mut zz_action: i32;

        // cached fields:
        let mut zz_current_pos_l: usize;
        let mut zz_marked_pos_l: usize;
        let mut zz_end_read_l = self.zz_end_read;
        let zz_trans_l: &[i32] = &super::wikipedia_tokenizer_impl_tables::ZZ_TRANS;
        let zz_row_map_l: &[i32] = &super::wikipedia_tokenizer_impl_tables::ZZ_ROWMAP;
        let zz_attr_l: &[i32] = &super::wikipedia_tokenizer_impl_tables::ZZ_ATTRIBUTE;

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
                    let eof = self.zz_refill(reader)?;
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
                return Ok(YYEOF);
            } else {
                match if zz_action < 0 {
                    zz_action
                } else {
                    super::wikipedia_tokenizer_impl_tables::ZZ_ACTION[zz_action as usize]
                } {
                    1 => {
                        self.num_wiki_tokens_seen = 0;
                        self.position_inc = 1;
                        // Break so we don't hit fall-through warning:
                    }
                    2 => {
                        self.position_inc = 1;
                        return Ok(ALPHANUM);
                    }
                    3 => {
                        self.num_wiki_tokens_seen = 0;
                        self.position_inc = 1;
                        self.current_tok_type = EXTERNAL_LINK_URL;
                        self.yybegin(EXTERNAL_LINK_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    4 => {
                        self.position_inc = 1;
                        return Ok(CJ);
                    }
                    5 => {
                        self.position_inc = 1;
                        // Break so we don't hit fall-through warning:
                    }
                    6 => {
                        self.yybegin(CATEGORY_STATE);
                        self.num_wiki_tokens_seen += 1;
                        return Ok(self.current_tok_type);
                    }
                    7 => {
                        self.yybegin(INTERNAL_LINK_STATE);
                        self.num_wiki_tokens_seen += 1;
                        return Ok(self.current_tok_type);
                    }
                    8 => {
                        // Break so we don't hit fall-through warning:
                        // ignore
                    }
                    9 => {
                        if self.num_link_toks == 0 {
                            self.position_inc = 0;
                        } else {
                            self.position_inc = 1;
                        }
                        self.num_wiki_tokens_seen += 1;
                        self.current_tok_type = EXTERNAL_LINK;
                        self.yybegin(EXTERNAL_LINK_STATE);
                        self.num_link_toks += 1;
                        return Ok(self.current_tok_type);
                    }
                    10 => {
                        self.num_link_toks = 0;
                        self.position_inc = 0;
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                    }
                    11 => {
                        self.current_tok_type = BOLD;
                        self.yybegin(THREE_SINGLE_QUOTES_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    12 => {
                        self.current_tok_type = ITALICS;
                        self.num_wiki_tokens_seen += 1;
                        self.yybegin(STRING);
                        return Ok(self.current_tok_type);
                        // italics
                    }
                    13 => {
                        self.current_tok_type = EXTERNAL_LINK;
                        self.num_wiki_tokens_seen = 0;
                        self.yybegin(EXTERNAL_LINK_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    14 => {
                        self.yybegin(STRING);
                        self.num_wiki_tokens_seen += 1;
                        return Ok(self.current_tok_type);
                    }
                    15 => {
                        self.current_tok_type = HEADING;
                        self.yybegin(DOUBLE_EQUALS_STATE);
                        self.num_wiki_tokens_seen += 1;
                        return Ok(self.current_tok_type);
                    }
                    16 => {
                        self.current_tok_type = SUB_HEADING;
                        self.num_wiki_tokens_seen = 0;
                        self.yybegin(STRING);
                        // Break so we don't hit fall-through warning:
                    }
                    17 => {
                        self.yybegin(DOUBLE_BRACE_STATE);
                        self.num_wiki_tokens_seen = 0;
                        return Ok(self.current_tok_type);
                    }
                    18 => {
                        // Break so we don't hit fall-through warning:
                        // ignore STRING
                    }
                    19 => {
                        self.yybegin(STRING);
                        self.num_wiki_tokens_seen += 1;
                        return Ok(self.current_tok_type);
                        // STRING ALPHANUM
                    }
                    20 => {
                        self.num_balanced = 0;
                        self.num_wiki_tokens_seen = 0;
                        self.current_tok_type = EXTERNAL_LINK;
                        self.yybegin(EXTERNAL_LINK_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    21 => {
                        self.yybegin(STRING);
                        return Ok(self.current_tok_type);
                        // pipe
                    }
                    22 => {
                        self.num_wiki_tokens_seen = 0;
                        self.position_inc = 1;
                        if self.num_balanced == 0 {
                            self.num_balanced += 1;
                            self.yybegin(TWO_SINGLE_QUOTES_STATE);
                        } else {
                            self.num_balanced = 0;
                        }
                        // Break so we don't hit fall-through warning:
                    }
                    23 => {
                        self.num_wiki_tokens_seen = 0;
                        self.position_inc = 1;
                        self.yybegin(DOUBLE_EQUALS_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    24 => {
                        self.num_wiki_tokens_seen = 0;
                        self.position_inc = 1;
                        self.current_tok_type = INTERNAL_LINK;
                        self.yybegin(INTERNAL_LINK_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    25 => {
                        self.num_wiki_tokens_seen = 0;
                        self.position_inc = 1;
                        self.current_tok_type = CITATION;
                        self.yybegin(DOUBLE_BRACE_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    26 => {
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                    }
                    27 => {
                        self.num_link_toks = 0;
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                    }
                    28 => {
                        self.current_tok_type = INTERNAL_LINK;
                        self.num_wiki_tokens_seen = 0;
                        self.yybegin(INTERNAL_LINK_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    29 => {
                        self.current_tok_type = INTERNAL_LINK;
                        self.num_wiki_tokens_seen = 0;
                        self.yybegin(INTERNAL_LINK_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    30 => {
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                    }
                    31 => {
                        self.num_balanced = 0;
                        self.current_tok_type = ALPHANUM;
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                        // end italics
                    }
                    32 => {
                        self.num_balanced = 0;
                        self.num_wiki_tokens_seen = 0;
                        self.current_tok_type = INTERNAL_LINK;
                        self.yybegin(INTERNAL_LINK_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    33 => {
                        self.position_inc = 1;
                        return Ok(NUM);
                    }
                    34 => {
                        self.position_inc = 1;
                        return Ok(COMPANY);
                    }
                    35 => {
                        self.position_inc = 1;
                        return Ok(APOSTROPHE);
                    }
                    36 => {
                        self.position_inc = 1;
                        return Ok(HOST);
                    }
                    37 => {
                        self.current_tok_type = BOLD_ITALICS;
                        self.yybegin(FIVE_SINGLE_QUOTES_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    38 => {
                        self.num_balanced = 0;
                        self.current_tok_type = ALPHANUM;
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                        // end bold
                    }
                    39 => {
                        self.num_balanced = 0;
                        self.current_tok_type = ALPHANUM;
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                        // end sub header
                    }
                    40 => {
                        self.position_inc = 1;
                        return Ok(ACRONYM);
                    }
                    41 => {
                        self.position_inc = 1;
                        return Ok(EMAIL);
                    }
                    42 => {
                        self.num_balanced = 0;
                        self.current_tok_type = ALPHANUM;
                        self.yybegin(YYINITIAL);
                        // Break so we don't hit fall-through warning:
                        // end bold italics
                    }
                    43 => {
                        self.position_inc = 1;
                        self.num_wiki_tokens_seen += 1;
                        self.yybegin(EXTERNAL_LINK_STATE);
                        return Ok(self.current_tok_type);
                    }
                    44 => {
                        self.num_wiki_tokens_seen = 0;
                        self.position_inc = 1;
                        self.current_tok_type = CATEGORY;
                        self.yybegin(CATEGORY_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    45 => {
                        self.current_tok_type = CATEGORY;
                        self.num_wiki_tokens_seen = 0;
                        self.yybegin(CATEGORY_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    46 => {
                        self.num_balanced = 0;
                        self.num_wiki_tokens_seen = 0;
                        self.current_tok_type = CATEGORY;
                        self.yybegin(CATEGORY_STATE);
                        // Break so we don't hit fall-through warning:
                    }
                    47..=92 => {}
                    _ => Self::zz_scan_error(ZZ_NO_MATCH),
                }
            }
        }
    }
}
// @generated end skeleton
