// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.LocaleSyntaxException`.

use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

/// A checked exception for an ill-formed locale element, with the index of
/// the error.
///
/// Equivalent to `com.ibm.icu.impl.locale.LocaleSyntaxException`, which
/// extends `Exception`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleSyntaxException {
    message: String,
    /// `_index`.
    index: i32,
}

impl LocaleSyntaxException {
    /// The Java binary class name.
    pub const CLASS: &'static str = "com.ibm.icu.impl.locale.LocaleSyntaxException";

    /// `new LocaleSyntaxException(String msg)`: error index `0`.
    pub fn new(msg: impl Into<String>) -> Self {
        Self::with_index(msg, 0)
    }

    /// `new LocaleSyntaxException(String msg, int errorIndex)`.
    pub fn with_index(msg: impl Into<String>, error_index: i32) -> Self {
        Self {
            message: msg.into(),
            index: error_index,
        }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// `getErrorIndex()`.
    pub fn get_error_index(&self) -> i32 {
        self.index
    }

    /// This exception as the error it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }
}

impl JavaThrowable for LocaleSyntaxException {
    fn java_class(&self) -> &'static str {
        Self::CLASS
    }

    fn get_message(&self) -> Option<&str> {
        Some(&self.message)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
