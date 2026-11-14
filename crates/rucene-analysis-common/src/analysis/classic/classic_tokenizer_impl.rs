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

//! The JFlex scanner behind [`ClassicTokenizer`](super::classic_tokenizer::ClassicTokenizer),
//! ported from `org.apache.lucene.analysis.classic.ClassicTokenizerImpl`.
//!
//! Lucene 10.5.0 generates the Java class with JFlex 1.8.2 from
//! `ClassicTokenizerImpl.jflex`. `tools/translate/jflex_java_to_rust.py`
//! translates the generated parts: the DFA tables (into
//! `classic_tokenizer_impl_tables.rs`) and the JFlex skeleton with its actions
//! (into the `@generated` regions of this file). The user code of the grammar
//! is translated by hand, outside the regions.
//!
//! # The reader is read as UTF-16
//!
//! The JFlex buffer is a `char[]` of UTF-16 code units, filled by
//! `java.io.Reader.read(char[], int, int)`; a [`CharReader`](rucene::analysis::CharReader)
//! hands out the same code units, so the buffer holds exactly the code units
//! Java's holds and `zzRefill`'s surrogate handling runs unchanged. Positions
//! (`yychar`, `yylength`, the offsets) count UTF-16 code units, as Java's do.

use rucene::analysis::CharTermAttribute;
use rucene::error::{LuceneError, Result};

use super::classic_tokenizer;

// @generated begin constants by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
// From Lucene 10.5.0's lucene/analysis/common/src/java/org/apache/lucene/analysis/classic/ClassicTokenizerImpl.java
// (sha256 3d6d533b184aa7555ce4f9946f0e17e1560f3de05288afa6fa15d6efa1fda0c0).

/// This character denotes the end of file.
///
/// Equivalent to `ClassicTokenizerImpl.YYEOF`.
pub const YYEOF: i32 = -1;

/// Initial size of the lookahead buffer.
///
/// Equivalent to `ClassicTokenizerImpl.ZZ_BUFFERSIZE`.
const ZZ_BUFFERSIZE: usize = 4096;

/// Lexical state.
///
/// Equivalent to `ClassicTokenizerImpl.YYINITIAL`.
pub const YYINITIAL: i32 = 0;

/// `ZZ_LEXSTATE[l]` is the state in the DFA for the lexical state `l`;
/// `ZZ_LEXSTATE[l+1]` is the state in the DFA for the lexical state `l` at the
/// beginning of a line; `l` is of the form `l = 2*k`, `k` a non negative integer.
const ZZ_LEXSTATE: [i32; 2] = [0, 0];

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
/// Equivalent to `ClassicTokenizerImpl.ALPHANUM`, `ClassicTokenizer.ALPHANUM`.
pub const ALPHANUM: i32 = classic_tokenizer::ALPHANUM;
/// Equivalent to `ClassicTokenizerImpl.APOSTROPHE`, `ClassicTokenizer.APOSTROPHE`.
pub const APOSTROPHE: i32 = classic_tokenizer::APOSTROPHE;
/// Equivalent to `ClassicTokenizerImpl.ACRONYM`, `ClassicTokenizer.ACRONYM`.
pub const ACRONYM: i32 = classic_tokenizer::ACRONYM;
/// Equivalent to `ClassicTokenizerImpl.COMPANY`, `ClassicTokenizer.COMPANY`.
pub const COMPANY: i32 = classic_tokenizer::COMPANY;
/// Equivalent to `ClassicTokenizerImpl.EMAIL`, `ClassicTokenizer.EMAIL`.
pub const EMAIL: i32 = classic_tokenizer::EMAIL;
/// Equivalent to `ClassicTokenizerImpl.HOST`, `ClassicTokenizer.HOST`.
pub const HOST: i32 = classic_tokenizer::HOST;
/// Equivalent to `ClassicTokenizerImpl.NUM`, `ClassicTokenizer.NUM`.
pub const NUM: i32 = classic_tokenizer::NUM;
/// Equivalent to `ClassicTokenizerImpl.CJ`, `ClassicTokenizer.CJ`.
pub const CJ: i32 = classic_tokenizer::CJ;
/// Equivalent to `ClassicTokenizerImpl.ACRONYM_DEP`, `ClassicTokenizer.ACRONYM_DEP`.
pub const ACRONYM_DEP: i32 = classic_tokenizer::ACRONYM_DEP;

/// Equivalent to `ClassicTokenizerImpl.TOKEN_TYPES`, the array
/// `ClassicTokenizer.TOKEN_TYPES` itself.
pub static TOKEN_TYPES: &[&str; 9] = &classic_tokenizer::TOKEN_TYPES;

/// This class implements the classic lucene StandardTokenizer up until 3.0
///
/// Equivalent to `org.apache.lucene.analysis.classic.ClassicTokenizerImpl`.
/// Java's scanner holds its `java.io.Reader`; a reader is owned by the
/// [`Tokenizer`](rucene::analysis::Tokenizer), so the methods that read take
/// it as a parameter.
#[derive(Debug)]
pub(crate) struct ClassicTokenizerImpl {
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
    /// The current token decoded to `char`s for [`get_text`](Self::get_text),
    /// kept across tokens. Java has no such buffer: its term holds the UTF-16
    /// code units `zzBuffer` holds.
    text_scratch: Vec<char>,
}

impl ClassicTokenizerImpl {
    /// Creates a new scanner.
    ///
    /// Equivalent to `ClassicTokenizerImpl(java.io.Reader in)`; the reader is
    /// passed to each reading method instead.
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
            text_scratch: Vec::new(),
        }
    }

    /// Character count processed so far.
    ///
    /// Equivalent to `ClassicTokenizerImpl.yychar()`: UTF-16 code units,
    /// truncated to `int` ("jflex supports > 2GB docs but not lucene").
    pub fn yychar(&self) -> i32 {
        self.yychar as i32
    }

    /// Fills CharTermAttribute with the current token text.
    ///
    /// Equivalent to `ClassicTokenizerImpl.getText(CharTermAttribute)`,
    /// `t.copyBuffer(zzBuffer, zzStartRead, zzMarkedPos - zzStartRead)`. The
    /// attribute holds `char`s, so the token's UTF-16 code units are decoded
    /// first (a lone surrogate, which no rule of the grammar matches alone,
    /// would become U+FFFD).
    pub fn get_text(&mut self, t: &mut dyn CharTermAttribute) {
        let units = &self.zz_buffer[self.zz_start_read..self.zz_marked_pos];
        self.text_scratch.clear();
        self.text_scratch.extend(
            char::decode_utf16(units.iter().copied())
                .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)),
        );
        t.copy_buffer(&self.text_scratch, 0, self.text_scratch.len());
    }

    /// Equivalent to `ClassicTokenizerImpl.setBufferSize(int)`, which throws
    /// `UnsupportedOperationException`.
    ///
    /// # Errors
    ///
    /// Always, Java's `java.lang.UnsupportedOperationException`.
    pub fn set_buffer_size(&mut self, _num_chars: i32) -> Result<()> {
        Err(LuceneError::from(
            rucene::error::LuceneErrorKind::Unchecked {
                class: "java.lang.UnsupportedOperationException",
                message: None,
                cause: None,
            },
        ))
    }
}

// @generated begin skeleton by tools/translate/jflex_java_to_rust.py; DO NOT EDIT
impl ClassicTokenizerImpl {
    /// Translates raw input code points to DFA table row.
    ///
    /// Equivalent to `ClassicTokenizerImpl.zzCMap(int)`.
    fn zz_cmap(input: i32) -> i32 {
        let offset = input & 255;
        if offset == input {
            super::classic_tokenizer_impl_tables::ZZ_CMAP_BLOCKS[offset as usize]
        } else {
            super::classic_tokenizer_impl_tables::ZZ_CMAP_BLOCKS
                [(super::classic_tokenizer_impl_tables::ZZ_CMAP_TOP[(input >> 8) as usize] | offset)
                    as usize]
        }
    }

    /// Refills the input buffer.
    ///
    /// Equivalent to `ClassicTokenizerImpl.zzRefill()`: returns `false` iff there was new
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
    /// Equivalent to `ClassicTokenizerImpl.yyclose()`. The reader is the one passed to the call: the Java scanner keeps its `zzReader`, a Rust `Tokenizer` lends its input for the length of each call.
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
    /// Equivalent to `ClassicTokenizerImpl.yyreset(java.io.Reader)`; the new reader is the one passed to the next reading call.
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
    /// Equivalent to `ClassicTokenizerImpl.yyResetPosition()`.
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
    /// Equivalent to `ClassicTokenizerImpl.yyatEOF()`.
    pub fn yyat_eof(&self) -> bool {
        self.zz_at_eof
    }

    /// Returns the current lexical state.
    ///
    /// Equivalent to `ClassicTokenizerImpl.yystate()`.
    pub fn yystate(&self) -> i32 {
        self.zz_lexical_state
    }

    /// Enters a new lexical state.
    ///
    /// Equivalent to `ClassicTokenizerImpl.yybegin(int)`.
    pub fn yybegin(&mut self, new_state: i32) {
        self.zz_lexical_state = new_state;
    }

    /// Returns the text matched by the current regular expression.
    ///
    /// Equivalent to `ClassicTokenizerImpl.yytext()`; a lone surrogate, which a Java
    /// `String` holds, becomes U+FFFD in a Rust `String`.
    pub fn yytext(&self) -> String {
        String::from_utf16_lossy(&self.zz_buffer[self.zz_start_read..self.zz_marked_pos])
    }

    /// Returns the UTF-16 code unit at the given position from the matched text, a
    /// value from 0 to `yylength()-1`.
    ///
    /// Equivalent to `ClassicTokenizerImpl.yycharat(int)`.
    pub fn yycharat(&self, position: usize) -> u16 {
        self.zz_buffer[self.zz_start_read + position]
    }

    /// How many characters (UTF-16 code units) were matched.
    ///
    /// Equivalent to `ClassicTokenizerImpl.yylength()`.
    pub fn yylength(&self) -> i32 {
        (self.zz_marked_pos - self.zz_start_read) as i32
    }

    /// Reports an error that occurred while scanning.
    ///
    /// In a well-formed scanner (no or only correct usage of `yypushback(int)` and
    /// a match-all fallback rule) this method will only be called with things that
    /// "Can't Possibly Happen".
    ///
    /// Equivalent to `ClassicTokenizerImpl.zzScanError(int)`, which throws a
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
    /// Equivalent to `ClassicTokenizerImpl.yypushback(int)`.
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
    /// Equivalent to `ClassicTokenizerImpl.getNextToken()`. The reader is the one passed to the call: the Java scanner keeps its `zzReader`, a Rust `Tokenizer` lends its input for the length of each call.
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
        let zz_trans_l: &[i32] = &super::classic_tokenizer_impl_tables::ZZ_TRANS;
        let zz_row_map_l: &[i32] = &super::classic_tokenizer_impl_tables::ZZ_ROWMAP;
        let zz_attr_l: &[i32] = &super::classic_tokenizer_impl_tables::ZZ_ATTRIBUTE;

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
                    super::classic_tokenizer_impl_tables::ZZ_ACTION[zz_action as usize]
                } {
                    1 => {
                        // Break so we don't hit fall-through warning:
                        // ignore
                    }
                    2 => {
                        return Ok(ALPHANUM);
                    }
                    3 => {
                        return Ok(CJ);
                    }
                    4 => {
                        return Ok(NUM);
                    }
                    5 => {
                        return Ok(HOST);
                    }
                    6 => {
                        return Ok(COMPANY);
                    }
                    7 => {
                        return Ok(APOSTROPHE);
                    }
                    8 => {
                        return Ok(ACRONYM_DEP);
                    }
                    9 => {
                        return Ok(ACRONYM);
                    }
                    10 => {
                        return Ok(EMAIL);
                    }
                    11..=20 => {}
                    _ => Self::zz_scan_error(ZZ_NO_MATCH),
                }
            }
        }
    }
}
// @generated end skeleton
