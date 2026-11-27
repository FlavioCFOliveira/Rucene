// SAX exception class.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: SAXException.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `SAXException.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use crate::java::{throwable_to_string, Exception};

/// The class of a [`SAXException`]: the base class or one of the
/// subclasses Xerces throws and catches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SAXExceptionClass {
    /// `org.xml.sax.SAXException`.
    SAXException,
    /// `org.xml.sax.SAXNotRecognizedException`.
    SAXNotRecognizedException,
    /// `org.xml.sax.SAXNotSupportedException`.
    SAXNotSupportedException,
    /// `org.xml.sax.SAXParseException`, with its location.
    SAXParseException {
        /// `publicId`.
        public_id: Option<String>,
        /// `systemId`.
        system_id: Option<String>,
        /// `lineNumber`.
        line_number: i32,
        /// `columnNumber`.
        column_number: i32,
    },
}

/// Encapsulate a general SAX error or warning.
///
/// Equivalent to `org.xml.sax.SAXException`; [`class`](Self::class) tells
/// which subclass an instance is (`instanceof`).
#[derive(Debug, Clone)]
pub struct SAXException {
    class: SAXExceptionClass,
    message: Option<String>,
    exception: Option<Box<Exception>>,
}

impl SAXException {
    /// Equivalent to `new SAXException()`.
    pub fn new() -> Self {
        Self::of(SAXExceptionClass::SAXException, None, None)
    }

    /// Equivalent to `new SAXException(String)`.
    pub fn with_message(message: Option<&str>) -> Self {
        Self::of(
            SAXExceptionClass::SAXException,
            message.map(str::to_string),
            None,
        )
    }

    /// Equivalent to `new SAXException(Exception)`.
    pub fn with_exception(e: Exception) -> Self {
        Self::of(SAXExceptionClass::SAXException, None, Some(e))
    }

    /// Equivalent to `new SAXException(String, Exception)`.
    pub fn with_message_and_exception(message: Option<&str>, e: Option<Exception>) -> Self {
        Self::of(
            SAXExceptionClass::SAXException,
            message.map(str::to_string),
            e,
        )
    }

    /// A SAX exception of class `class`.
    pub(crate) fn of(
        class: SAXExceptionClass,
        message: Option<String>,
        exception: Option<Exception>,
    ) -> Self {
        Self {
            class,
            message,
            exception: exception.map(Box::new),
        }
    }

    /// The class of this exception (`instanceof`).
    pub fn class(&self) -> &SAXExceptionClass {
        &self.class
    }

    /// `getClass().getName()`.
    pub fn class_name(&self) -> &'static str {
        match self.class {
            SAXExceptionClass::SAXException => "org.xml.sax.SAXException",
            SAXExceptionClass::SAXNotRecognizedException => "org.xml.sax.SAXNotRecognizedException",
            SAXExceptionClass::SAXNotSupportedException => "org.xml.sax.SAXNotSupportedException",
            SAXExceptionClass::SAXParseException { .. } => "org.xml.sax.SAXParseException",
        }
    }

    /// Return a detail message for this exception.
    ///
    /// Equivalent to `getMessage()`.
    pub fn get_message(&self) -> Option<String> {
        match (&self.message, &self.exception) {
            (None, Some(e)) => e.get_message(),
            (m, _) => m.clone(),
        }
    }

    /// Return the embedded exception, if any.
    ///
    /// Equivalent to `getException()`.
    pub fn get_exception(&self) -> Option<&Exception> {
        self.exception.as_deref()
    }

    /// Override toString to pick up any embedded exception.
    ///
    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        match &self.exception {
            Some(e) => e.to_java_string(),
            None => throwable_to_string(self.class_name(), self.get_message().as_deref()),
        }
    }
}

impl Default for SAXException {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SAXException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_java_string())
    }
}
