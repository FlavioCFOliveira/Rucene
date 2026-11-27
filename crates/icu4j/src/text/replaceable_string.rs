// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.ReplaceableString`.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::text::replaceable::Replaceable;
use crate::text::utf16::UTF16;

/// A [`Replaceable`] over a `StringBuffer`, without metadata.
///
/// Equivalent to `com.ibm.icu.text.ReplaceableString`. Java's
/// `StringBuffer` is a synchronized, shareable buffer: here it is an
/// `Arc<Mutex<Vec<u16>>>`, so that
/// [`from_string_buffer`](Self::from_string_buffer) shares the caller's
/// buffer as `ReplaceableString(StringBuffer)` does.
#[derive(Debug, Clone)]
pub struct ReplaceableString {
    buf: Arc<Mutex<Vec<u16>>>,
}

impl Default for ReplaceableString {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplaceableString {
    /// `ReplaceableString(String str)`: a new buffer holding `str`.
    pub fn from_str_units(s: &[u16]) -> Self {
        Self {
            buf: Arc::new(Mutex::new(s.to_vec())),
        }
    }

    /// `ReplaceableString(StringBuffer buf)`: the given buffer, shared.
    pub fn from_string_buffer(buf: Arc<Mutex<Vec<u16>>>) -> Self {
        Self { buf }
    }

    /// `ReplaceableString()`: an empty buffer.
    pub fn new() -> Self {
        Self {
            buf: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn buf(&self) -> MutexGuard<'_, Vec<u16>> {
        self.buf.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `toString()`: the text.
    pub fn to_units(&self) -> Vec<u16> {
        self.buf().clone()
    }

    /// `substring(int start, int limit)`.
    ///
    /// # Panics
    ///
    /// Where Java throws `StringIndexOutOfBoundsException`: an invalid range.
    pub fn substring(&self, start: i32, limit: i32) -> Vec<u16> {
        self.buf()[start as usize..limit as usize].to_vec()
    }
}

impl Replaceable for ReplaceableString {
    fn length(&self) -> i32 {
        self.buf().len() as i32
    }

    fn char_at(&self, offset: i32) -> u16 {
        self.buf()[offset as usize]
    }

    fn char32_at(&self, offset: i32) -> i32 {
        let buf = self.buf();
        UTF16::char_at_buffer(&buf, offset)
    }

    fn get_chars(&self, src_start: i32, src_limit: i32, dst: &mut [u16], dst_start: i32) {
        if src_start != src_limit {
            let buf = self.buf();
            let n = (src_limit - src_start) as usize;
            let d = dst_start as usize;
            dst[d..d + n].copy_from_slice(&buf[src_start as usize..src_limit as usize]);
        }
    }

    fn replace(&self, start: i32, limit: i32, text: &[u16]) {
        let mut buf = self.buf();
        string_buffer_replace(&mut buf, start, limit, text);
    }

    fn replace_chars(
        &self,
        start: i32,
        limit: i32,
        chars: &[u16],
        chars_start: i32,
        chars_len: i32,
    ) {
        let mut buf = self.buf();
        // buf.delete(start, limit)
        let len = buf.len() as i32;
        let end = limit.min(len);
        assert!(
            start >= 0 && start <= end,
            "java.lang.StringIndexOutOfBoundsException: start {start}, end {limit}, length {len}"
        );
        buf.drain(start as usize..end as usize);
        // buf.insert(start, chars, charsStart, charsLen)
        let cs = chars_start as usize;
        let ins = &chars[cs..cs + chars_len as usize];
        let at = start as usize;
        buf.splice(at..at, ins.iter().copied());
    }

    fn copy(&self, start: i32, limit: i32, dest: i32) {
        if start == limit && start >= 0 && start <= self.length() {
            return;
        }
        let mut text = vec![0u16; (limit - start) as usize];
        self.get_chars(start, limit, &mut text, 0);
        self.replace_chars(dest, dest, &text, 0, limit - start);
    }

    fn has_meta_data(&self) -> bool {
        false
    }
}

/// `StringBuffer.replace(int start, int end, String str)`: `end` is
/// clamped to the length.
///
/// # Panics
///
/// Where Java throws `StringIndexOutOfBoundsException`: `start` negative,
/// above the length, or above `end`.
pub(crate) fn string_buffer_replace(buf: &mut Vec<u16>, start: i32, end: i32, s: &[u16]) {
    let count = buf.len() as i32;
    let end = end.min(count);
    assert!(
        start >= 0 && start <= count && start <= end,
        "java.lang.StringIndexOutOfBoundsException: start {start}, end {end}, length {count}"
    );
    buf.splice(start as usize..end as usize, s.iter().copied());
}
