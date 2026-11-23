// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.RuleCharacterIterator`.

use rucene::error::{LuceneError, Result};

use crate::jdk::{illegal_argument_no_message, ParsePosition};
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::utility::Utility;
use crate::text::symbol_table::{SymbolTable, SYMBOL_REF};
use crate::text::utf16::UTF16;

/// `DONE`: the value returned at the end of the text.
pub const DONE: i32 = -1;
/// `PARSE_VARIABLES`: expand `$name` references through the symbol table.
pub const PARSE_VARIABLES: i32 = 1;
/// `PARSE_ESCAPES`: parse backslash escapes.
pub const PARSE_ESCAPES: i32 = 2;
/// `SKIP_WHITESPACE`: skip pattern white space.
pub const SKIP_WHITESPACE: i32 = 4;

/// A position of a [`RuleCharacterIterator`], saved and restored by
/// `getPos`/`setPos`.
///
/// Equivalent to `RuleCharacterIterator.Position`.
#[derive(Debug, Clone, Default)]
pub struct Position {
    buf: Option<Vec<u16>>,
    buf_pos: i32,
    pos_index: i32,
}

/// An iterator that returns 32-bit code points from a rule text, expanding
/// variable references and escapes and skipping white space as asked.
///
/// Equivalent to `com.ibm.icu.impl.RuleCharacterIterator`. Java keeps a
/// reference to the caller's `ParsePosition`; here the iterator borrows it
/// mutably for its lifetime.
pub struct RuleCharacterIterator<'a> {
    text: &'a [u16],
    pos: &'a mut ParsePosition,
    sym: Option<&'a dyn SymbolTable>,
    buf: Option<Vec<u16>>,
    buf_pos: i32,
    is_escaped: bool,
}

impl<'a> RuleCharacterIterator<'a> {
    /// `RuleCharacterIterator(String text, SymbolTable sym, ParsePosition pos)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `pos` is past the end of `text`.
    pub fn new(text: &'a [u16], sym: Option<&'a dyn SymbolTable>, pos: &'a mut ParsePosition) -> Result<Self> {
        if pos.get_index() > text.len() as i32 {
            return Err(illegal_argument_no_message());
        }
        Ok(Self { text, pos, sym, buf: None, buf_pos: 0, is_escaped: false })
    }

    /// `atEnd()`.
    pub fn at_end(&self) -> bool {
        self.buf.is_none() && self.pos.get_index() == self.text.len() as i32
    }

    /// `next(int options)`: the next code point, `DONE` at the end.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an undefined variable or an invalid
    /// escape; the errors of the symbol table.
    pub fn next(&mut self, options: i32) -> Result<i32> {
        let mut c;
        self.is_escaped = false;

        loop {
            c = self.current();
            self.advance(UTF16::get_char_count(c));

            if c == SYMBOL_REF as i32 && self.buf.is_none() && (options & PARSE_VARIABLES) != 0 {
                if let Some(sym) = self.sym {
                    let name = sym.parse_reference(self.text, self.pos, self.text.len() as i32)?;
                    // If name is null there was an isolated SYMBOL_REF;
                    // return it.  Caller must be prepared for this.
                    let Some(name) = name else { break };
                    self.buf_pos = 0;
                    let chars = sym.lookup(&name);
                    let Some(chars) = chars else {
                        self.buf = None;
                        return Err(LuceneError::illegal_argument(format!(
                            "Undefined variable: {}",
                            String::from_utf16_lossy(&name)
                        )));
                    };
                    // Handle empty variable value
                    if chars.is_empty() {
                        self.buf = None;
                    }
                    self.buf = Some(chars);
                    continue;
                }
            }

            if (options & SKIP_WHITESPACE) != 0 && PatternProps::is_white_space(c) {
                continue;
            }

            if c == b'\\' as i32 && (options & PARSE_ESCAPES) != 0 {
                let cp_and_length = Utility::unescape_and_length_at(self.get_current_buffer(), self.get_current_buffer_pos());
                if cp_and_length < 0 {
                    return Err(LuceneError::illegal_argument("Invalid escape"));
                }
                c = Utility::cp_from_code_point_and_length(cp_and_length);
                self.jumpahead(Utility::length_from_code_point_and_length(cp_and_length))?;
                self.is_escaped = true;
            }

            break;
        }

        Ok(c)
    }

    /// `isEscaped()`.
    pub fn is_escaped(&self) -> bool {
        self.is_escaped
    }

    /// `inVariable()`.
    pub fn in_variable(&self) -> bool {
        self.buf.is_some()
    }

    /// `getPos(Position p)`: the current position, stored into `p` when
    /// given.
    pub fn get_pos(&self, p: Option<Position>) -> Position {
        let mut p = p.unwrap_or_default();
        p.buf = self.buf.clone();
        p.buf_pos = self.buf_pos;
        p.pos_index = self.pos.get_index();
        p
    }

    /// `setPos(Position p)`.
    pub fn set_pos(&mut self, p: &Position) {
        self.buf = p.buf.clone();
        self.pos.set_index(p.pos_index);
        self.buf_pos = p.buf_pos;
    }

    /// `skipIgnored(int options)`.
    pub fn skip_ignored(&mut self, options: i32) {
        if (options & SKIP_WHITESPACE) != 0 {
            loop {
                let a = self.current();
                if !PatternProps::is_white_space(a) {
                    break;
                }
                self.advance(UTF16::get_char_count(a));
            }
        }
    }

    /// `getCurrentBuffer()`.
    pub fn get_current_buffer(&self) -> &[u16] {
        match &self.buf {
            Some(b) => b,
            None => self.text,
        }
    }

    /// `getCurrentBufferPos()`.
    pub fn get_current_buffer_pos(&self) -> i32 {
        if self.buf.is_some() {
            self.buf_pos
        } else {
            self.pos.get_index()
        }
    }

    /// `jumpahead(int count)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a negative count or one past the end.
    pub fn jumpahead(&mut self, count: i32) -> Result<()> {
        if count < 0 {
            return Err(illegal_argument_no_message());
        }
        if let Some(buf) = &self.buf {
            self.buf_pos += count;
            if self.buf_pos > buf.len() as i32 {
                return Err(illegal_argument_no_message());
            }
            if self.buf_pos == buf.len() as i32 {
                self.buf = None;
            }
        } else {
            let i = self.pos.get_index() + count;
            self.pos.set_index(i);
            if i > self.text.len() as i32 {
                return Err(illegal_argument_no_message());
            }
        }
        Ok(())
    }

    /// `toString()`: the text with `|` at the position.
    pub fn to_units(&self) -> Vec<u16> {
        let b = self.pos.get_index() as usize;
        let mut s = self.text[..b].to_vec();
        s.push(b'|' as u16);
        s.extend_from_slice(&self.text[b..]);
        s
    }

    fn current(&self) -> i32 {
        if let Some(buf) = &self.buf {
            UTF16::char_at(buf, self.buf_pos)
        } else {
            let i = self.pos.get_index();
            if i < self.text.len() as i32 {
                UTF16::char_at(self.text, i)
            } else {
                DONE
            }
        }
    }

    fn advance(&mut self, count: i32) {
        if let Some(buf) = &self.buf {
            self.buf_pos += count;
            if self.buf_pos == buf.len() as i32 {
                self.buf = None;
            }
        } else {
            self.pos.set_index(self.pos.get_index() + count);
            if self.pos.get_index() > self.text.len() as i32 {
                self.pos.set_index(self.text.len() as i32);
            }
        }
    }
}
