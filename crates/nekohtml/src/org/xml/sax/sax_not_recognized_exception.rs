// SAXNotRecognizedException.java - unrecognized feature or value.
// http://www.saxproject.org
// Written by David Megginson
// NO WARRANTY!  This class is in the Public Domain.
// $Id: SAXNotRecognizedException.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `SAXNotRecognizedException.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use crate::org::xml::sax::sax_exception::{SAXException, SAXExceptionClass};

/// Exception class for an unrecognized identifier.
///
/// Equivalent to `org.xml.sax.SAXNotRecognizedException`: its constructors
/// build a [`SAXException`] of that class.
#[derive(Debug)]
pub struct SAXNotRecognizedException;

impl SAXNotRecognizedException {
    /// Equivalent to `new SAXNotRecognizedException()`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> SAXException {
        SAXException::of(SAXExceptionClass::SAXNotRecognizedException, None, None)
    }

    /// Equivalent to `new SAXNotRecognizedException(String)`.
    pub fn with_message(message: Option<&str>) -> SAXException {
        SAXException::of(
            SAXExceptionClass::SAXNotRecognizedException,
            message.map(str::to_string),
            None,
        )
    }
}
