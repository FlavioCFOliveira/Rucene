/*
Copyright (c) 2001, Dr Martin Porter
Copyright (c) 2004,2005, Richard Boulton
Copyright (c) 2013, Yoshiki Shibukawa
Copyright (c) 2006,2007,2009,2010,2011,2014-2019, Olly Betts
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

  1. Redistributions of source code must retain the above copyright notice,
     this list of conditions and the following disclaimer.
  2. Redistributions in binary form must reproduce the above copyright notice,
     this list of conditions and the following disclaimer in the documentation
     and/or other materials provided with the distribution.
  3. Neither the name of the Snowball project nor the names of its contributors
     may be used to endorse or promote products derived from this software
     without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/

//! Base class for a snowball stemmer, ported from
//! `org.tartarus.snowball.SnowballProgram`.

use super::among::Among;

/// The number of UTF-16 units of `s`: the `length()` of the Java `String`
/// literal `s` stands for. Support of [`utf16!`].
pub(crate) const fn utf16_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut units = 0;
    while i < bytes.len() {
        let lead = bytes[i];
        if lead < 0x80 {
            i += 1;
            units += 1;
        } else if lead < 0xE0 {
            i += 2;
            units += 1;
        } else if lead < 0xF0 {
            i += 3;
            units += 1;
        } else {
            i += 4;
            units += 2;
        }
    }
    units
}

/// The UTF-16 units of `s`, `N` of them: what `toCharArray()` returns for
/// the Java `String` literal `s` stands for. Support of [`utf16!`].
pub(crate) const fn encode_utf16<const N: usize>(s: &str) -> [u16; N] {
    let bytes = s.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    let mut n = 0;
    while i < bytes.len() {
        let lead = bytes[i] as u32;
        let code_point = if lead < 0x80 {
            i += 1;
            lead
        } else if lead < 0xE0 {
            let cp = ((lead & 0x1F) << 6) | (bytes[i + 1] as u32 & 0x3F);
            i += 2;
            cp
        } else if lead < 0xF0 {
            let cp = ((lead & 0x0F) << 12)
                | ((bytes[i + 1] as u32 & 0x3F) << 6)
                | (bytes[i + 2] as u32 & 0x3F);
            i += 3;
            cp
        } else {
            let cp = ((lead & 0x07) << 18)
                | ((bytes[i + 1] as u32 & 0x3F) << 12)
                | ((bytes[i + 2] as u32 & 0x3F) << 6)
                | (bytes[i + 3] as u32 & 0x3F);
            i += 4;
            cp
        };
        if code_point >= 0x10000 {
            let v = code_point - 0x10000;
            out[n] = (0xD800 + (v >> 10)) as u16;
            out[n + 1] = (0xDC00 + (v & 0x3FF)) as u16;
            n += 2;
        } else {
            out[n] = code_point as u16;
            n += 1;
        }
    }
    out
}

/// The UTF-16 units of the string literal `$s`, as a `&'static [u16]`
/// computed at compile time: the Java `String` literal of the generated
/// Snowball code (`eq_s("st")`, `slice_from("e")`, `new Among("hed", -1,
/// 1)`), whose `char`s the program compares unit by unit.
macro_rules! utf16 {
    ($s:literal) => {{
        const UNITS: [u16; $crate::tartarus::snowball::snowball_program::utf16_len($s)] =
            $crate::tartarus::snowball::snowball_program::encode_utf16($s);
        &UNITS
    }};
}
pub(crate) use utf16;

/// Base class for a snowball stemmer.
///
/// Equivalent to `org.tartarus.snowball.SnowballProgram`. A generated
/// stemmer holds one as its superclass state (`base`) and implements
/// [`AsRef`]/[`AsMut`] of it, the Rust form of `extends SnowballProgram`
/// that [`SnowballProgram::find_among`] needs to call the stemmer's routines.
///
/// The current string is a buffer of UTF-16 units (`u16`), as Java's
/// `char[]` is, and the cursors are `i32`, as Java's `int` fields are, so
/// that the cursor arithmetic of the generated code (which computes, for
/// example, `cursor - 3` before comparing it with `limit_backward`) is
/// Java's.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * Java's `setCurrent(char[], int)` keeps a reference to the caller's
///   array, so the program edits the caller's buffer in place until a
///   replacement outgrows it (`replace_s` then switches to a copy); and the
///   copy constructor and `copy_from` share the other program's array.
///   Rust cannot alias a buffer that way: the program owns its buffer
///   ([`set_current_buffer`](Self::set_current_buffer) takes it by value,
///   the copy constructor and `copy_from` clone it), and the caller reads
///   the result back through
///   [`get_current_buffer`](Self::get_current_buffer), as Java's
///   `SnowballFilter` does when the buffer was replaced.
/// * `SnowballProgram implements Serializable` and its `serialVersionUID`
///   (`2016072500L`) serve Java object serialization, which has no
///   counterpart here; neither is ported.
#[derive(Debug)]
pub struct SnowballProgram {
    /// The current string (`private char[] current`).
    current: Vec<u16>,
    /// Java: `protected int cursor`.
    pub cursor: i32,
    /// Java: `protected int length`.
    pub length: i32,
    /// Java: `protected int limit`.
    pub limit: i32,
    /// Java: `protected int limit_backward`.
    pub limit_backward: i32,
    /// Java: `protected int bra`.
    pub bra: i32,
    /// Java: `protected int ket`.
    pub ket: i32,
}

impl Default for SnowballProgram {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<SnowballProgram> for SnowballProgram {
    fn as_ref(&self) -> &SnowballProgram {
        self
    }
}

impl AsMut<SnowballProgram> for SnowballProgram {
    fn as_mut(&mut self) -> &mut SnowballProgram {
        self
    }
}

impl SnowballProgram {
    /// Equivalent to the `protected SnowballProgram()` constructor:
    /// `setCurrent("")`.
    pub fn new() -> Self {
        let mut program = Self {
            current: Vec::new(),
            cursor: 0,
            length: 0,
            limit: 0,
            limit_backward: 0,
            bra: 0,
            ket: 0,
        };
        program.set_current("");
        program
    }

    /// Set the current string.
    ///
    /// Equivalent to `setCurrent(String)`: `setCurrent(value.toCharArray(),
    /// value.length())`.
    pub fn set_current(&mut self, value: &str) {
        let text: Vec<u16> = value.encode_utf16().collect();
        let length = text.len() as i32;
        self.set_current_buffer(text, length);
    }

    /// Get the current string.
    ///
    /// Equivalent to `getCurrent()`, `new String(current, 0, length)`.
    ///
    /// **Divergence from Lucene 10.5.0.** A Java `String` may hold a lone
    /// surrogate, which a stemmer leaves when it deletes one unit of a
    /// surrogate pair; a Rust `String` cannot, so a lone surrogate becomes
    /// U+FFFD. [`get_current_buffer`](Self::get_current_buffer) returns the
    /// units unchanged.
    pub fn get_current(&self) -> String {
        String::from_utf16_lossy(&self.current[..self.length as usize])
    }

    /// Set the current string.
    ///
    /// Equivalent to `setCurrent(char[] text, int length)`: `text` is the
    /// character array containing input, `length` the valid length of
    /// `text`. The program takes the array over (see the type's
    /// divergences).
    pub fn set_current_buffer(&mut self, text: Vec<u16>, length: i32) {
        self.current = text;
        self.cursor = 0;
        self.limit = length;
        self.length = length;
        self.limit_backward = 0;
        self.bra = self.cursor;
        self.ket = self.limit;
    }

    /// Get the current buffer containing the stem.
    ///
    /// NOTE: this may be a different character array than the one
    /// originally provided with `set_current_buffer`, in the exceptional
    /// case that stemming produced a longer intermediate or result string.
    ///
    /// It is necessary to use [`get_current_buffer_length`] to determine the
    /// valid length of the returned buffer. For example, many words are
    /// stemmed simply by subtracting from the length to remove suffixes.
    ///
    /// Equivalent to `getCurrentBuffer()`.
    ///
    /// [`get_current_buffer_length`]: Self::get_current_buffer_length
    pub fn get_current_buffer(&self) -> &[u16] {
        &self.current
    }

    /// Get the valid length of the character array in
    /// [`get_current_buffer`](Self::get_current_buffer).
    ///
    /// Equivalent to `getCurrentBufferLength()`.
    pub fn get_current_buffer_length(&self) -> i32 {
        self.length
    }

    /// Equivalent to the copy constructor `SnowballProgram(SnowballProgram
    /// other)`, which shares `other`'s array (here, a clone of it; see the
    /// type's divergences).
    pub fn from_program(other: &SnowballProgram) -> Self {
        Self {
            current: other.current.clone(),
            cursor: other.cursor,
            length: other.length,
            limit: other.limit,
            limit_backward: other.limit_backward,
            bra: other.bra,
            ket: other.ket,
        }
    }

    /// Equivalent to `copy_from(SnowballProgram other)`, which shares
    /// `other`'s array (here, a clone of it; see the type's divergences).
    pub fn copy_from(&mut self, other: &SnowballProgram) {
        self.current.clone_from(&other.current);
        self.cursor = other.cursor;
        self.length = other.length;
        self.limit = other.limit;
        self.limit_backward = other.limit_backward;
        self.bra = other.bra;
        self.ket = other.ket;
    }

    /// Equivalent to `in_grouping(char[] s, int min, int max)`.
    pub fn in_grouping(&mut self, s: &[u16], min: i32, max: i32) -> bool {
        if self.cursor >= self.limit {
            return false;
        }
        let mut ch = i32::from(self.current[self.cursor as usize]);
        if ch > max || ch < min {
            return false;
        }
        ch -= min;
        if (i32::from(s[(ch >> 3) as usize]) & (0x1 << (ch & 0x7))) == 0 {
            return false;
        }
        self.cursor += 1;
        true
    }

    /// Equivalent to `in_grouping_b(char[] s, int min, int max)`.
    pub fn in_grouping_b(&mut self, s: &[u16], min: i32, max: i32) -> bool {
        if self.cursor <= self.limit_backward {
            return false;
        }
        let mut ch = i32::from(self.current[(self.cursor - 1) as usize]);
        if ch > max || ch < min {
            return false;
        }
        ch -= min;
        if (i32::from(s[(ch >> 3) as usize]) & (0x1 << (ch & 0x7))) == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// Equivalent to `out_grouping(char[] s, int min, int max)`.
    pub fn out_grouping(&mut self, s: &[u16], min: i32, max: i32) -> bool {
        if self.cursor >= self.limit {
            return false;
        }
        let mut ch = i32::from(self.current[self.cursor as usize]);
        if ch > max || ch < min {
            self.cursor += 1;
            return true;
        }
        ch -= min;
        if (i32::from(s[(ch >> 3) as usize]) & (0x1 << (ch & 0x7))) == 0 {
            self.cursor += 1;
            return true;
        }
        false
    }

    /// Equivalent to `out_grouping_b(char[] s, int min, int max)`.
    pub fn out_grouping_b(&mut self, s: &[u16], min: i32, max: i32) -> bool {
        if self.cursor <= self.limit_backward {
            return false;
        }
        let mut ch = i32::from(self.current[(self.cursor - 1) as usize]);
        if ch > max || ch < min {
            self.cursor -= 1;
            return true;
        }
        ch -= min;
        if (i32::from(s[(ch >> 3) as usize]) & (0x1 << (ch & 0x7))) == 0 {
            self.cursor -= 1;
            return true;
        }
        false
    }

    /// Equivalent to `eq_s(CharSequence s)`; `s` is the sequence's UTF-16
    /// units.
    pub fn eq_s(&mut self, s: &[u16]) -> bool {
        let s_length = s.len() as i32;
        if self.limit - self.cursor < s_length {
            return false;
        }
        let mut i = 0;
        while i != s_length {
            if self.current[(self.cursor + i) as usize] != s[i as usize] {
                return false;
            }
            i += 1;
        }
        self.cursor += s_length;
        true
    }

    /// Equivalent to `eq_s_b(CharSequence s)`; `s` is the sequence's UTF-16
    /// units.
    pub fn eq_s_b(&mut self, s: &[u16]) -> bool {
        let s_length = s.len() as i32;
        if self.cursor - self.limit_backward < s_length {
            return false;
        }
        let mut i = 0;
        while i != s_length {
            if self.current[(self.cursor - s_length + i) as usize] != s[i as usize] {
                return false;
            }
            i += 1;
        }
        self.cursor -= s_length;
        true
    }

    /// Equivalent to `find_among(Among v[])`, a binary search of the
    /// forward-sorted table `v` for the longest entry matching at the
    /// cursor of `this`.
    ///
    /// `this` is the program whose routines the entries name (Java's
    /// `this`, the stemmer); an entry's routine is called directly (see
    /// [`Among`]'s divergence), with what Java's `invokeExact` call does: an
    /// error the routine raises (here, a panic) propagates.
    pub fn find_among<P: AsMut<SnowballProgram>>(this: &mut P, v: &[Among<P>]) -> i32 {
        let mut i: i32 = 0;
        let mut j: i32 = v.len() as i32;

        let c = this.as_mut().cursor;
        let l = this.as_mut().limit;

        let mut common_i: i32 = 0;
        let mut common_j: i32 = 0;

        let mut first_key_inspected = false;

        loop {
            let k = i + ((j - i) >> 1);
            let mut diff: i32 = 0;
            let mut common = if common_i < common_j {
                common_i
            } else {
                common_j
            }; // smaller
            let w = &v[k as usize];
            let program = this.as_mut();
            let mut i2 = common;
            while i2 < w.s.len() as i32 {
                if c + common == l {
                    diff = -1;
                    break;
                }
                diff =
                    i32::from(program.current[(c + common) as usize]) - i32::from(w.s[i2 as usize]);
                if diff != 0 {
                    break;
                }
                common += 1;
                i2 += 1;
            }
            if diff < 0 {
                j = k;
                common_j = common;
            } else {
                i = k;
                common_i = common;
            }
            if j - i <= 1 {
                if i > 0 {
                    break; // v->s has been inspected
                }
                if j == i {
                    break; // only one item in v
                }

                // - but now we need to go round once more to get
                // v->s inspected. This looks messy, but is actually
                // the optimal approach.

                if first_key_inspected {
                    break;
                }
                first_key_inspected = true;
            }
        }
        loop {
            let w = &v[i as usize];
            let w_length = w.s.len() as i32;
            if common_i >= w_length {
                this.as_mut().cursor = c + w_length;
                let Some(method) = w.method else {
                    return w.result;
                };
                let res = method(this);
                this.as_mut().cursor = c + w_length;
                if res {
                    return w.result;
                }
            }
            i = w.substring_i;
            if i < 0 {
                return 0;
            }
        }
    }

    /// Equivalent to `find_among_b(Among v[])`, `find_among` for backwards
    /// processing: the table `v` is sorted on the reversed strings, which
    /// are matched backwards from the cursor of `this`. Same comments apply.
    pub fn find_among_b<P: AsMut<SnowballProgram>>(this: &mut P, v: &[Among<P>]) -> i32 {
        let mut i: i32 = 0;
        let mut j: i32 = v.len() as i32;

        let c = this.as_mut().cursor;
        let lb = this.as_mut().limit_backward;

        let mut common_i: i32 = 0;
        let mut common_j: i32 = 0;

        let mut first_key_inspected = false;

        loop {
            let k = i + ((j - i) >> 1);
            let mut diff: i32 = 0;
            let mut common = if common_i < common_j {
                common_i
            } else {
                common_j
            };
            let w = &v[k as usize];
            let program = this.as_mut();
            let mut i2 = w.s.len() as i32 - 1 - common;
            while i2 >= 0 {
                if c - common == lb {
                    diff = -1;
                    break;
                }
                diff = i32::from(program.current[(c - 1 - common) as usize])
                    - i32::from(w.s[i2 as usize]);
                if diff != 0 {
                    break;
                }
                common += 1;
                i2 -= 1;
            }
            if diff < 0 {
                j = k;
                common_j = common;
            } else {
                i = k;
                common_i = common;
            }
            if j - i <= 1 {
                if i > 0 {
                    break;
                }
                if j == i {
                    break;
                }
                if first_key_inspected {
                    break;
                }
                first_key_inspected = true;
            }
        }
        loop {
            let w = &v[i as usize];
            let w_length = w.s.len() as i32;
            if common_i >= w_length {
                this.as_mut().cursor = c - w_length;
                let Some(method) = w.method else {
                    return w.result;
                };
                let res = method(this);
                this.as_mut().cursor = c - w_length;
                if res {
                    return w.result;
                }
            }
            i = w.substring_i;
            if i < 0 {
                return 0;
            }
        }
    }

    /// To replace chars between `c_bra` and `c_ket` in current by the
    /// chars in `s`.
    ///
    /// Equivalent to `replace_s(int c_bra, int c_ket, CharSequence s)`;
    /// returns the adjustment of the length. Java's `Arrays.copyOf` growth
    /// to exactly the new length is a resize of the owned buffer, its new
    /// units zero as `copyOf` pads them.
    pub fn replace_s(&mut self, c_bra: i32, c_ket: i32, s: &[u16]) -> i32 {
        let s_length = s.len() as i32;
        let adjustment = s_length - (c_ket - c_bra);
        let new_length = self.length + adjustment;
        // resize if necessary
        if new_length > self.current.len() as i32 {
            self.current.resize(new_length as usize, 0);
        }
        // if the substring being replaced is longer or shorter than the
        // replacement, need to shift things around
        if adjustment != 0 && c_ket < self.length {
            self.current.copy_within(
                c_ket as usize..self.length as usize,
                (c_bra + s_length) as usize,
            );
        }
        // insert the replacement text
        self.current[c_bra as usize..(c_bra + s_length) as usize].copy_from_slice(s);

        self.length += adjustment;
        self.limit += adjustment;
        if self.cursor >= c_ket {
            self.cursor += adjustment;
        } else if self.cursor > c_bra {
            self.cursor = c_bra;
        }
        adjustment
    }

    /// Equivalent to `slice_check()`: Java's `assert` statements, checked
    /// here by `debug_assert!` with Java's messages.
    pub fn slice_check(&self) {
        debug_assert!(self.bra >= 0, "bra={}", self.bra);
        debug_assert!(self.bra <= self.ket, "bra={},ket={}", self.bra, self.ket);
        debug_assert!(
            self.limit <= self.length,
            "limit={},length={}",
            self.limit,
            self.length
        );
        debug_assert!(
            self.ket <= self.limit,
            "ket={},limit={}",
            self.ket,
            self.limit
        );
    }

    /// Equivalent to `slice_from(CharSequence s)`.
    pub fn slice_from(&mut self, s: &[u16]) {
        self.slice_check();
        self.replace_s(self.bra, self.ket, s);
    }

    /// Equivalent to `slice_del()`: `slice_from("")`.
    pub fn slice_del(&mut self) {
        self.slice_from(&[]);
    }

    /// Equivalent to `insert(int c_bra, int c_ket, CharSequence s)`.
    pub fn insert(&mut self, c_bra: i32, c_ket: i32, s: &[u16]) {
        let adjustment = self.replace_s(c_bra, c_ket, s);
        if c_bra <= self.bra {
            self.bra += adjustment;
        }
        if c_bra <= self.ket {
            self.ket += adjustment;
        }
    }

    /// Copy the slice into the supplied `StringBuilder`.
    ///
    /// Equivalent to `slice_to(StringBuilder s)`; the builder is a buffer of
    /// UTF-16 units.
    pub fn slice_to(&self, s: &mut Vec<u16>) {
        self.slice_check();
        let len = self.ket - self.bra;
        s.clear();
        s.extend_from_slice(&self.current[self.bra as usize..(self.bra + len) as usize]);
    }

    /// Equivalent to `assign_to(StringBuilder s)`; the builder is a buffer
    /// of UTF-16 units.
    pub fn assign_to(&self, s: &mut Vec<u16>) {
        s.clear();
        s.extend_from_slice(&self.current[..self.limit as usize]);
    }
}
