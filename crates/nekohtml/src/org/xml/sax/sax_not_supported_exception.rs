// SAXNotSupportedException.java - unsupported feature or value.
// http://www.saxproject.org
// Written by David Megginson
// NO WARRANTY!  This class is in the Public Domain.
// $Id: SAXNotSupportedException.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `SAXNotSupportedException.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use crate::org::xml::sax::sax_exception::{SAXException, SAXExceptionClass};

/// Exception class for an unsupported operation.
///
/// Equivalent to `org.xml.sax.SAXNotSupportedException`: its constructors
/// build a [`SAXException`] of that class.
#[derive(Debug)]
pub struct SAXNotSupportedException;

impl SAXNotSupportedException {
    /// Equivalent to `new SAXNotSupportedException()`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> SAXException {
        SAXException::of(SAXExceptionClass::SAXNotSupportedException, None, None)
    }

    /// Equivalent to `new SAXNotSupportedException(String)`.
    pub fn with_message(message: Option<&str>) -> SAXException {
        SAXException::of(
            SAXExceptionClass::SAXNotSupportedException,
            message.map(str::to_string),
            None,
        )
    }
}
