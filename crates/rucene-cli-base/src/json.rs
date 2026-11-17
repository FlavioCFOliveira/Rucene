//! A small JSON (RFC 8259) reader and writer, standard library only.
//!
//! The bulk input of every index is JSON Lines (see [`crate::input`]); this
//! module parses one line into a [`JsonValue`] and writes the stored fields of
//! a document back as one JSON object (the output of `get`).
//!
//! Numbers are kept as their source text ([`JsonValue::Number`]) and parsed
//! later against the kind of the field they feed, so an integer field never
//! goes through `f64` and a double keeps its shortest round-trip form.

use crate::error::{CliError, Result};

/// The maximum nesting depth of arrays and objects the parser accepts.
///
/// The records are flat (one level of arrays at most), so a deeper value is
/// malformed input; the bound keeps the recursive parser off the stack limit.
pub const MAX_DEPTH: usize = 64;

/// A parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, as written in the source (validated against the RFC 8259
    /// grammar).
    Number(String),
    /// A string, unescaped.
    String(String),
    /// An array.
    Array(Vec<JsonValue>),
    /// An object, members in source order (duplicate names are rejected).
    Object(Vec<(String, JsonValue)>),
}

/// Parses `text` as exactly one JSON value, surrounded by optional
/// whitespace.
///
/// # Errors
///
/// Returns [`CliError::Input`] with the 1-based character column of the
/// first offending character when `text` is not one well-formed JSON value,
/// nests deeper than [`MAX_DEPTH`], or has an object with a repeated name.
pub fn parse(text: &str) -> Result<JsonValue> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.pos < parser.chars.len() {
        return Err(parser.error("unexpected characters after the JSON value"));
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn error(&self, message: &str) -> CliError {
        CliError::Input(format!("JSON column {}: {message}", self.pos + 1))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn expect_word(&mut self, word: &str) -> Result<()> {
        for expected in word.chars() {
            if self.peek() != Some(expected) {
                return Err(self.error(&format!("expected `{word}`")));
            }
            self.pos += 1;
        }
        Ok(())
    }

    fn value(&mut self, depth: usize) -> Result<JsonValue> {
        match self.peek() {
            None => Err(self.error("unexpected end of input")),
            Some('n') => self.expect_word("null").map(|()| JsonValue::Null),
            Some('t') => self.expect_word("true").map(|()| JsonValue::Bool(true)),
            Some('f') => self.expect_word("false").map(|()| JsonValue::Bool(false)),
            Some('"') => self.string().map(JsonValue::String),
            Some('[') => self.array(depth),
            Some('{') => self.object(depth),
            Some('-' | '0'..='9') => self.number(),
            Some(other) => Err(self.error(&format!("unexpected character {other:?}"))),
        }
    }

    fn enter(&self, depth: usize) -> Result<usize> {
        if depth >= MAX_DEPTH {
            return Err(self.error(&format!("nesting deeper than {MAX_DEPTH} levels")));
        }
        Ok(depth + 1)
    }

    fn array(&mut self, depth: usize) -> Result<JsonValue> {
        let depth = self.enter(depth)?;
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth)?);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]` in array")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<JsonValue> {
        let depth = self.enter(depth)?;
        self.pos += 1; // '{'
        let mut members: Vec<(String, JsonValue)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(JsonValue::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some('"') {
                return Err(self.error("expected a member name (a string)"));
            }
            let name_pos = self.pos;
            let name = self.string()?;
            if members.iter().any(|(existing, _)| *existing == name) {
                self.pos = name_pos;
                return Err(self.error(&format!("member {name:?} given twice")));
            }
            self.skip_whitespace();
            if self.peek() != Some(':') {
                return Err(self.error("expected `:` after the member name"));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.value(depth)?;
            members.push((name, value));
            self.skip_whitespace();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(members));
                }
                _ => return Err(self.error("expected `,` or `}` in object")),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some('0'..='9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<JsonValue> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        match self.peek() {
            Some('0') => self.pos += 1,
            Some('1'..='9') => {
                self.digits();
            }
            _ => return Err(self.error("expected a digit")),
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.error("expected a digit after `.`"));
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.error("expected a digit in the exponent"));
            }
        }
        Ok(JsonValue::Number(
            self.chars[start..self.pos].iter().collect(),
        ))
    }

    fn hex4(&mut self) -> Result<u32> {
        let mut value = 0u32;
        for _ in 0..4 {
            let digit = self
                .peek()
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| self.error("expected 4 hexadecimal digits after `\\u`"))?;
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error("unterminated string"));
            };
            self.pos += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(self.error("unterminated escape"));
                    };
                    self.pos += 1;
                    match escape {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000C}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => out.push(self.unicode_escape()?),
                        other => return Err(self.error(&format!("invalid escape `\\{other}`"))),
                    }
                }
                c if u32::from(c) < 0x20 => {
                    self.pos -= 1;
                    return Err(self.error("unescaped control character in string"));
                }
                c => out.push(c),
            }
        }
    }

    /// Decodes the code point of a `\uXXXX` escape (the `\u` already
    /// consumed), joining a UTF-16 surrogate pair written as two escapes.
    fn unicode_escape(&mut self) -> Result<char> {
        let first = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&first) {
            if self.peek() != Some('\\') || self.chars.get(self.pos + 1) != Some(&'u') {
                return Err(self.error("a high surrogate must be followed by `\\u` low surrogate"));
            }
            self.pos += 2;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.error("invalid low surrogate"));
            }
            0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else if (0xDC00..0xE000).contains(&first) {
            return Err(self.error("unpaired low surrogate"));
        } else {
            first
        };
        char::from_u32(code).ok_or_else(|| self.error("invalid code point"))
    }
}

/// Appends `text` to `out` as a JSON string literal: quotes, backslashes and
/// control characters escaped, everything else (accents included) written as
/// UTF-8.
pub fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars_arrays_and_objects() {
        let value =
            parse(r#" {"a": "x\u00e7\n", "b": [1, -2.5e3, true, null], "c": {}} "#).unwrap();
        assert_eq!(
            value,
            JsonValue::Object(vec![
                ("a".into(), JsonValue::String("xç\n".into())),
                (
                    "b".into(),
                    JsonValue::Array(vec![
                        JsonValue::Number("1".into()),
                        JsonValue::Number("-2.5e3".into()),
                        JsonValue::Bool(true),
                        JsonValue::Null,
                    ])
                ),
                ("c".into(), JsonValue::Object(vec![])),
            ])
        );
    }

    #[test]
    fn joins_surrogate_pairs() {
        assert_eq!(
            parse(r#""\ud83d\ude00""#).unwrap(),
            JsonValue::String("\u{1F600}".into())
        );
        assert!(parse(r#""\ud83d""#).is_err());
        assert!(parse(r#""\ude00""#).is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "01",
            "1.",
            "-",
            "1e",
            "tru",
            "\"a",
            "\"\u{1}\"",
            "{\"a\":1,\"a\":2}",
            "1 2",
            "\"\\x\"",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} must not parse");
        }
        let deep = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(parse(&deep).is_err());
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn writes_strings_that_parse_back() {
        let text = "Aquisição \"x\" \\ \t\n\u{1}";
        let mut out = String::new();
        write_string(&mut out, text);
        assert_eq!(parse(&out).unwrap(), JsonValue::String(text.into()));
    }
}
