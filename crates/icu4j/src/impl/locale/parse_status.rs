// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.ParseStatus`.

/// The status of a language tag parse: the parsed length, the error index
/// and the error message.
///
/// Equivalent to `com.ibm.icu.impl.locale.ParseStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseStatus {
    /// `_parseLength` (package-private), in UTF-16 units.
    pub(crate) parse_length: i32,
    /// `_errorIndex` (package-private).
    pub(crate) error_index: i32,
    /// `_errorMsg` (package-private).
    pub(crate) error_msg: Option<String>,
}

impl Default for ParseStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl ParseStatus {
    /// `new ParseStatus()`.
    pub fn new() -> Self {
        Self {
            parse_length: 0,
            error_index: -1,
            error_msg: None,
        }
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.parse_length = 0;
        self.error_index = -1;
        self.error_msg = None;
    }

    /// `isError()`.
    pub fn is_error(&self) -> bool {
        self.error_index >= 0
    }

    /// `getErrorIndex()`.
    pub fn get_error_index(&self) -> i32 {
        self.error_index
    }

    /// `getParseLength()`.
    pub fn get_parse_length(&self) -> i32 {
        self.parse_length
    }

    /// `getErrorMessage()`.
    pub fn get_error_message(&self) -> Option<&str> {
        self.error_msg.as_deref()
    }
}
