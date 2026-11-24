// SAX error handler.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: ErrorHandler.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `ErrorHandler.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use crate::java::JResult;
use crate::org::xml::sax::sax_parse_exception::SAXParseException;

/// Basic interface for SAX error handlers.
///
/// Equivalent to `org.xml.sax.ErrorHandler`.
pub trait ErrorHandler {
    /// Equivalent to `warning(SAXParseException)`.
    fn warning(&self, exception: &SAXParseException) -> JResult<()>;
    /// Equivalent to `error(SAXParseException)`.
    fn error(&self, exception: &SAXParseException) -> JResult<()>;
    /// Equivalent to `fatalError(SAXParseException)`.
    fn fatal_error(&self, exception: &SAXParseException) -> JResult<()>;
}
