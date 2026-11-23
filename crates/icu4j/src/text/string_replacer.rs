// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.StringReplacer`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use rucene::error::Result;

use crate::r#impl::utility::Utility;
use crate::text::replaceable::Replaceable;
use crate::text::rule_based_transliterator::Data;
use crate::text::unicode_replacer::UnicodeReplacer;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// A replacer that produces static text as its output. The text may
/// contain transliterator stand-in characters that represent nested
/// `UnicodeReplacer` objects, making it possible to encode a tree of
/// replacers in a `StringReplacer`. A `StringReplacer` that contains such
/// stand-ins is called a <em>complex</em> `StringReplacer`. A complex
/// `StringReplacer` has a slower processing loop than a non-complex one.
///
/// Equivalent to the package-private class `com.ibm.icu.text.StringReplacer`.
///
/// **Divergence.** As [`StringMatcher`](crate::text::string_matcher::StringMatcher),
/// the owning `Data` is held as a [`Weak`] (Java's strong reference closes
/// a cycle), and `isComplex`, which `replace` rewrites, is an atomic.
pub struct StringReplacer {
    /// Output text, possibly containing stand-in characters that represent
    /// nested `UnicodeReplacer`s.
    output: Vec<u16>,
    /// Cursor position. Value is ignored if `has_cursor` is false.
    cursor_pos: i32,
    /// True if this object outputs a cursor position.
    has_cursor: bool,
    /// A complex object contains nested replacers and requires more
    /// complex processing. `StringReplacer`s are initially assumed to be
    /// complex. If no nested replacers are seen during processing, then
    /// `is_complex` is set to false, and future replacements are short
    /// circuited for better performance.
    is_complex: AtomicBool,
    /// Object that translates stand-in characters in `output` to
    /// `UnicodeReplacer` objects.
    data: Weak<Data>,
}

impl std::fmt::Debug for StringReplacer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StringReplacer")
            .field("output", &String::from_utf16_lossy(&self.output))
            .field("cursor_pos", &self.cursor_pos)
            .field("has_cursor", &self.has_cursor)
            .finish()
    }
}

impl StringReplacer {
    /// `StringReplacer(String theOutput, int theCursorPos, Data theData)`:
    /// a replacer with a cursor position.
    pub(crate) fn new_with_cursor(the_output: Vec<u16>, the_cursor_pos: i32, the_data: &Arc<Data>) -> Self {
        Self {
            output: the_output,
            cursor_pos: the_cursor_pos,
            has_cursor: true,
            data: Arc::downgrade(the_data),
            is_complex: AtomicBool::new(true),
        }
    }

    /// `StringReplacer(String theOutput, Data theData)`: a replacer without
    /// a cursor position.
    pub(crate) fn new(the_output: Vec<u16>, the_data: &Arc<Data>) -> Self {
        Self {
            output: the_output,
            cursor_pos: 0,
            has_cursor: false,
            data: Arc::downgrade(the_data),
            is_complex: AtomicBool::new(true),
        }
    }

    fn data(&self) -> Arc<Data> {
        self.data.upgrade().expect("RuleBasedTransliterator.Data released while its StringReplacer is in use")
    }
}

impl UnicodeReplacer for StringReplacer {
    /// `replace(Replaceable text, int start, int limit, int[] cursor)`.
    fn replace(&self, text: &dyn Replaceable, start: i32, limit: i32, cursor: &mut [i32]) -> Result<i32> {
        let out_len;
        let mut new_start = 0;

        // NOTE: It should be possible to _always_ run the complex
        // processing code; just slower.  If not, then there is a bug
        // in the complex processing code.

        // Simple (no nested replacers) Processing Code :
        if !self.is_complex.load(Ordering::Relaxed) {
            text.replace(start, limit, &self.output);
            out_len = self.output.len() as i32;

            // Setup default cursor position (for cursorPos within output)
            new_start = self.cursor_pos;
        }
        // Complex (nested replacers) Processing Code :
        else {
            // Accumulate replacement text in buf as we go
            let mut buf: Vec<u16> = Vec::new();
            let mut o_output; // offset into 'output'
            self.is_complex.store(false, Ordering::Relaxed);

            // The temporary buffer starts at tempStart, and extends
            // to destLimit + tempExtra.  The start of the buffer has a single
            // character from before the key.  This provides style
            // data when addition characters are filled into the
            // temporary buffer.  If there is nothing to the left, use
            // the non-character U+FFFF, which Replaceable subclasses
            // should treat specially as a "no-style character."
            // destStart points to the point after the style context
            // character, so it is tempStart+1 or tempStart+2.
            let temp_start = text.length(); // start of temp buffer
            let mut dest_start = temp_start; // copy new text to here
            if start > 0 {
                let len = UTF16::get_char_count(text.char32_at(start - 1));
                text.copy(start - len, start, temp_start);
                dest_start += len;
            } else {
                text.replace(temp_start, temp_start, &[0xFFFF]);
                dest_start += 1;
            }
            let mut dest_limit = dest_start;
            let mut temp_extra = 0; // temp chars after destLimit

            let data = self.data();
            o_output = 0;
            while o_output < self.output.len() as i32 {
                if o_output == self.cursor_pos {
                    // Record the position of the cursor
                    new_start = buf.len() as i32 + dest_limit - dest_start; // relative to start
                    // the buf.length() was inserted for bug 5789
                    // the problem is that if we are accumulating into a buffer (when r == null below)
                    // then the actual length of the text at that point needs to add the buf length.
                    // there was an alternative suggested in #5789, but that looks like it won't work
                    // if we have accumulated some stuff in the dest part AND have a non-zero buffer.
                }
                let c = UTF16::char_at(&self.output, o_output);

                // When we are at the last position copy the right style
                // context character into the temporary buffer.  We don't
                // do this before because it will provide an incorrect
                // right context for previous replace() operations.
                let next_index = o_output + UTF16::get_char_count(c);
                if next_index == self.output.len() as i32 {
                    temp_extra = UTF16::get_char_count(text.char32_at(limit));
                    text.copy(limit, limit + temp_extra, dest_limit);
                }

                match data.lookup_replacer(c) {
                    None => {
                        // Accumulate straight (non-segment) text.
                        UTF16::append(&mut buf, c)?;
                    }
                    Some(r) => {
                        self.is_complex.store(true, Ordering::Relaxed);

                        // Insert any accumulated straight text.
                        if !buf.is_empty() {
                            text.replace(dest_limit, dest_limit, &buf);
                            dest_limit += buf.len() as i32;
                            buf.clear();
                        }

                        // Delegate output generation to replacer object
                        let len = r.replace(text, dest_limit, dest_limit, cursor)?;
                        dest_limit += len;
                    }
                }
                o_output = next_index;
            }
            // Insert any accumulated straight text.
            if !buf.is_empty() {
                text.replace(dest_limit, dest_limit, &buf);
                dest_limit += buf.len() as i32;
            }
            if o_output == self.cursor_pos {
                // Record the position of the cursor
                new_start = dest_limit - dest_start; // relative to start
            }

            out_len = dest_limit - dest_start;

            // Copy new text to start, and delete it
            text.copy(dest_start, dest_limit, start);
            text.replace(temp_start + out_len, dest_limit + temp_extra + out_len, &[]);

            // Delete the old text (the key)
            text.replace(start + out_len, limit + out_len, &[]);
        }

        if self.has_cursor {
            // Adjust the cursor for positions outside the key.  These
            // refer to code points rather than code units.  If cursorPos
            // is within the output string, then use newStart, which has
            // already been set above.
            if self.cursor_pos < 0 {
                new_start = start;
                let mut n = self.cursor_pos;
                // Outside the output string, cursorPos counts code points
                while n < 0 && new_start > 0 {
                    new_start -= UTF16::get_char_count(text.char32_at(new_start - 1));
                    n += 1;
                }
                new_start += n;
            } else if self.cursor_pos > self.output.len() as i32 {
                new_start = start + out_len;
                let mut n = self.cursor_pos - self.output.len() as i32;
                // Outside the output string, cursorPos counts code points
                while n > 0 && new_start < text.length() {
                    new_start += UTF16::get_char_count(text.char32_at(new_start));
                    n -= 1;
                }
                new_start += n;
            } else {
                // Cursor is within output string.  It has been set up above
                // to be relative to start.
                new_start += start;
            }

            cursor[0] = new_start;
        }

        Ok(out_len)
    }

    /// `toReplacerPattern(boolean escapeUnprintable)`.
    fn to_replacer_pattern(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        let mut rule = Vec::new();
        let mut quote_buf = Vec::new();

        let mut cursor = self.cursor_pos;

        // Handle a cursor preceding the output
        if self.has_cursor && cursor < 0 {
            while {
                let c = cursor;
                cursor += 1;
                c < 0
            } {
                Utility::append_to_rule(&mut rule, b'@' as i32, true, escape_unprintable, &mut quote_buf)?;
            }
            // Fall through and append '|' below
        }

        let data = self.data();
        for i in 0..self.output.len() as i32 {
            if self.has_cursor && i == cursor {
                Utility::append_to_rule(&mut rule, b'|' as i32, true, escape_unprintable, &mut quote_buf)?;
            }
            let c = self.output[i as usize]; // Ok to use 16-bits here

            match data.lookup_replacer(c as i32) {
                None => {
                    Utility::append_to_rule(&mut rule, c as i32, false, escape_unprintable, &mut quote_buf)?;
                }
                Some(r) => {
                    let mut buf = vec![b' ' as u16];
                    buf.extend_from_slice(&r.to_replacer_pattern(escape_unprintable)?);
                    buf.push(b' ' as u16);
                    Utility::append_to_rule_str(&mut rule, &buf, true, escape_unprintable, &mut quote_buf)?;
                }
            }
        }

        // Handle a cursor after the output.  Use > rather than >= because
        // if cursor == output.length() it is at the end of the output,
        // which is the default position, so we need not emit it.
        if self.has_cursor && cursor > self.output.len() as i32 {
            cursor -= self.output.len() as i32;
            while {
                let c = cursor;
                cursor -= 1;
                c > 0
            } {
                Utility::append_to_rule(&mut rule, b'@' as i32, true, escape_unprintable, &mut quote_buf)?;
            }
            Utility::append_to_rule(&mut rule, b'|' as i32, true, escape_unprintable, &mut quote_buf)?;
        }
        // Flush quoteBuf out to result
        Utility::append_to_rule(&mut rule, -1, true, escape_unprintable, &mut quote_buf)?;

        Ok(rule)
    }

    /// `addReplacementSetTo(UnicodeSet toUnionTo)`.
    fn add_replacement_set_to(&self, to_union_to: &mut UnicodeSet) -> Result<()> {
        let data = self.data();
        let mut i = 0;
        while i < self.output.len() as i32 {
            let ch = UTF16::char_at(&self.output, i);
            match data.lookup_replacer(ch) {
                None => {
                    to_union_to.add(ch)?;
                }
                Some(r) => r.add_replacement_set_to(to_union_to)?,
            }
            i += UTF16::get_char_count(ch);
        }
        Ok(())
    }
}
