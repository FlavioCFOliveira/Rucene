// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.StringTokenIterator`.

/// An iterator over the tokens of a text separated by any of a set of
/// delimiter characters; empty tokens are returned.
///
/// Equivalent to `com.ibm.icu.impl.locale.StringTokenIterator`. Positions
/// are UTF-16 indices, as Java's; the delimiters are single units, so a
/// token never splits a surrogate pair and converts back to a Rust string
/// unchanged.
#[derive(Debug, Clone)]
pub struct StringTokenIterator {
    text: Vec<u16>,
    dlms: Vec<u16>,
    token: Option<String>,
    start: i32,
    end: i32,
    done: bool,
}

impl StringTokenIterator {
    /// `new StringTokenIterator(String text, String dlms)`.
    pub fn new(text: &str, dlms: &str) -> Self {
        let mut it = Self {
            text: text.encode_utf16().collect(),
            dlms: dlms.encode_utf16().collect(),
            token: None,
            start: 0,
            end: 0,
            done: false,
        };
        it.set_start(0);
        it
    }

    fn token_of(&self, start: i32, end: i32) -> String {
        String::from_utf16_lossy(&self.text[start as usize..end as usize])
    }

    /// `first()`.
    pub fn first(&mut self) -> Option<String> {
        self.set_start(0);
        self.token.clone()
    }

    /// `current()`: `None` once done.
    pub fn current(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// `current()` while not done: the token as a string, empty after the
    /// end (where Java returns `null`, a state callers never read).
    pub(crate) fn cur(&self) -> &str {
        self.token.as_deref().unwrap_or("")
    }

    /// `currentStart()`.
    pub fn current_start(&self) -> i32 {
        self.start
    }

    /// `currentEnd()`.
    pub fn current_end(&self) -> i32 {
        self.end
    }

    /// `isDone()`.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// `next()`.
    #[allow(clippy::should_implement_trait)] // Java method name
    pub fn next(&mut self) -> Option<String> {
        if self.has_next() {
            self.start = self.end + 1;
            self.end = self.next_delimiter(self.start);
            self.token = Some(self.token_of(self.start, self.end));
        } else {
            self.start = self.end;
            self.token = None;
            self.done = true;
        }
        self.token.clone()
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        (self.end as usize) < self.text.len()
    }

    /// `setStart(int offset)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `offset` is past the text.
    pub fn set_start(&mut self, offset: i32) -> &mut Self {
        if offset as usize > self.text.len() {
            crate::jdk::index_out_of_bounds(None);
        }
        self.start = offset;
        self.end = self.next_delimiter(self.start);
        self.token = Some(self.token_of(self.start, self.end));
        self.done = false;
        self
    }

    /// `setText(String text)`.
    pub fn set_text(&mut self, text: &str) -> &mut Self {
        self.text = text.encode_utf16().collect();
        self.set_start(0);
        self
    }

    fn next_delimiter(&self, start: i32) -> i32 {
        let mut idx = start as usize;
        while idx < self.text.len() {
            let c = self.text[idx];
            if self.dlms.contains(&c) {
                break;
            }
            idx += 1;
        }
        idx as i32
    }
}
