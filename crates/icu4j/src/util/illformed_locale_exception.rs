// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.IllformedLocaleException`.

use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

/// Thrown by methods in `ULocale` and `ULocale.Builder` to indicate that an
/// argument is not a well-formed BCP 47 tag.
///
/// Equivalent to `com.ibm.icu.util.IllformedLocaleException`, which extends
/// `RuntimeException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IllformedLocaleException {
    message: Option<String>,
    err_idx: i32,
}

impl IllformedLocaleException {
    /// The Java binary class name.
    pub const CLASS: &'static str = "com.ibm.icu.util.IllformedLocaleException";

    /// `new IllformedLocaleException()`.
    pub fn new() -> Self {
        Self {
            message: None,
            err_idx: -1,
        }
    }

    /// `new IllformedLocaleException(String message)`.
    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            err_idx: -1,
        }
    }

    /// `new IllformedLocaleException(String message, int errorIndex)`: the
    /// message gains ` [at index <i>]` for a non-negative index.
    pub fn with_index(message: &str, error_index: i32) -> Self {
        let suffix = if error_index < 0 {
            String::new()
        } else {
            format!(" [at index {error_index}]")
        };
        Self {
            message: Some(format!("{message}{suffix}")),
            err_idx: error_index,
        }
    }

    /// `getErrorIndex()`.
    pub fn get_error_index(&self) -> i32 {
        self.err_idx
    }

    /// This exception as the error it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }
}

impl Default for IllformedLocaleException {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaThrowable for IllformedLocaleException {
    fn java_class(&self) -> &'static str {
        Self::CLASS
    }

    fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
