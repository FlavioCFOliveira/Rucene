// SAX document handler.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: DocumentHandler.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `DocumentHandler.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::xml::sax::attribute_list::AttributeList;
use crate::org::xml::sax::locator::Locator;

/// Receive notification of general document events (SAX 1).
///
/// Equivalent to `org.xml.sax.DocumentHandler`.
pub trait DocumentHandler {
    /// Equivalent to `setDocumentLocator(Locator)`.
    fn set_document_locator(&self, locator: Rc<dyn Locator>);
    /// Equivalent to `startDocument()`.
    fn start_document(&self) -> JResult<()>;
    /// Equivalent to `endDocument()`.
    fn end_document(&self) -> JResult<()>;
    /// Equivalent to `startElement(String, AttributeList)`.
    fn start_element(&self, name: &str, atts: &dyn AttributeList) -> JResult<()>;
    /// Equivalent to `endElement(String)`.
    fn end_element(&self, name: &str) -> JResult<()>;
    /// Equivalent to `characters(char[], int, int)`.
    fn characters(&self, ch: &[u16], start: i32, length: i32) -> JResult<()>;
    /// Equivalent to `ignorableWhitespace(char[], int, int)`.
    fn ignorable_whitespace(&self, ch: &[u16], start: i32, length: i32) -> JResult<()>;
    /// Equivalent to `processingInstruction(String, String)`.
    fn processing_instruction(&self, target: &str, data: Option<&str>) -> JResult<()>;
}
