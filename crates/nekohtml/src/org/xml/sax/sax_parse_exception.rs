// SAX exception class.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: SAXParseException.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `SAXParseException.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use std::ops::Deref;

use crate::java::Exception;
use crate::org::xml::sax::locator::Locator;
use crate::org::xml::sax::sax_exception::{SAXException, SAXExceptionClass};

/// Encapsulate an XML parse error or warning.
///
/// Equivalent to `org.xml.sax.SAXParseException`: a [`SAXException`] of that
/// class, which it dereferences to.
#[derive(Debug, Clone)]
pub struct SAXParseException(SAXException);

impl SAXParseException {
    fn init(
        message: Option<&str>,
        e: Option<Exception>,
        public_id: Option<String>,
        system_id: Option<String>,
        line_number: i32,
        column_number: i32,
    ) -> Self {
        Self(SAXException::of(
            SAXExceptionClass::SAXParseException {
                public_id,
                system_id,
                line_number,
                column_number,
            },
            message.map(str::to_string),
            e,
        ))
    }

    /// Equivalent to `new SAXParseException(String, Locator)`.
    pub fn with_locator(message: Option<&str>, locator: Option<&dyn Locator>) -> Self {
        Self::with_locator_and_exception(message, locator, None)
    }

    /// Equivalent to `new SAXParseException(String, Locator, Exception)`.
    pub fn with_locator_and_exception(
        message: Option<&str>,
        locator: Option<&dyn Locator>,
        e: Option<Exception>,
    ) -> Self {
        match locator {
            Some(l) => Self::init(
                message,
                e,
                l.get_public_id(),
                l.get_system_id(),
                l.get_line_number(),
                l.get_column_number(),
            ),
            None => Self::init(message, e, None, None, -1, -1),
        }
    }

    /// Equivalent to `new SAXParseException(String, String, String, int,
    /// int)`.
    pub fn new(
        message: Option<&str>,
        public_id: Option<&str>,
        system_id: Option<&str>,
        line_number: i32,
        column_number: i32,
    ) -> Self {
        Self::with_exception(message, public_id, system_id, line_number, column_number, None)
    }

    /// Equivalent to `new SAXParseException(String, String, String, int,
    /// int, Exception)`.
    pub fn with_exception(
        message: Option<&str>,
        public_id: Option<&str>,
        system_id: Option<&str>,
        line_number: i32,
        column_number: i32,
        e: Option<Exception>,
    ) -> Self {
        Self::init(
            message,
            e,
            public_id.map(str::to_string),
            system_id.map(str::to_string),
            line_number,
            column_number,
        )
    }

    /// `(SAXParseException) e` of a [`SAXException`] of that class.
    pub fn from_sax_exception(e: &SAXException) -> Option<Self> {
        match e.class() {
            SAXExceptionClass::SAXParseException { .. } => Some(Self(e.clone())),
            _ => None,
        }
    }

    fn location(&self) -> (&Option<String>, &Option<String>, i32, i32) {
        match self.0.class() {
            SAXExceptionClass::SAXParseException {
                public_id,
                system_id,
                line_number,
                column_number,
            } => (public_id, system_id, *line_number, *column_number),
            _ => unreachable!("INVARIANT: a SAXParseException has a location"),
        }
    }

    /// Equivalent to `getPublicId()`.
    pub fn get_public_id(&self) -> Option<&str> {
        self.location().0.as_deref()
    }

    /// Equivalent to `getSystemId()`.
    pub fn get_system_id(&self) -> Option<&str> {
        self.location().1.as_deref()
    }

    /// Equivalent to `getLineNumber()`.
    pub fn get_line_number(&self) -> i32 {
        self.location().2
    }

    /// Equivalent to `getColumnNumber()`.
    pub fn get_column_number(&self) -> i32 {
        self.location().3
    }

    /// The exception as its base class.
    pub fn into_sax_exception(self) -> SAXException {
        self.0
    }
}

impl Deref for SAXParseException {
    type Target = SAXException;

    fn deref(&self) -> &SAXException {
        &self.0
    }
}

impl From<SAXParseException> for Exception {
    fn from(e: SAXParseException) -> Self {
        Exception::Sax(e.0)
    }
}
