// ContentHandler.java - handle main document content.
// http://www.saxproject.org
// Written by David Megginson
// NO WARRANTY!  This class is in the public domain.
// $Id: ContentHandler.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `ContentHandler.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::xml::sax::attributes::Attributes;
use crate::org::xml::sax::locator::Locator;

/// Receive notification of the logical content of a document.
///
/// Equivalent to `org.xml.sax.ContentHandler`; each method's `throws
/// SAXException` is its `Err`. Methods take `&self`: a handler keeps its
/// state in cells (see the crate documentation).
pub trait ContentHandler {
    /// Equivalent to `setDocumentLocator(Locator)`.
    fn set_document_locator(&self, locator: Rc<dyn Locator>);
    /// Equivalent to `startDocument()`.
    fn start_document(&self) -> JResult<()>;
    /// Equivalent to `endDocument()`.
    fn end_document(&self) -> JResult<()>;
    /// Equivalent to `startPrefixMapping(String, String)`.
    fn start_prefix_mapping(&self, prefix: &str, uri: &str) -> JResult<()>;
    /// Equivalent to `endPrefixMapping(String)`.
    fn end_prefix_mapping(&self, prefix: &str) -> JResult<()>;
    /// Equivalent to `startElement(String, String, String, Attributes)`.
    fn start_element(
        &self,
        uri: &str,
        local_name: &str,
        qname: &str,
        atts: &dyn Attributes,
    ) -> JResult<()>;
    /// Equivalent to `endElement(String, String, String)`.
    fn end_element(&self, uri: &str, local_name: &str, qname: &str) -> JResult<()>;
    /// Equivalent to `characters(char[], int, int)`.
    fn characters(&self, ch: &[u16], start: i32, length: i32) -> JResult<()>;
    /// Equivalent to `ignorableWhitespace(char[], int, int)`.
    fn ignorable_whitespace(&self, ch: &[u16], start: i32, length: i32) -> JResult<()>;
    /// Equivalent to `processingInstruction(String, String)`.
    fn processing_instruction(&self, target: &str, data: Option<&str>) -> JResult<()>;
    /// Equivalent to `skippedEntity(String)`.
    fn skipped_entity(&self, name: &str) -> JResult<()>;
}
