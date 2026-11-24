// SAX parser interface.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: Parser.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `Parser.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use std::rc::Rc;

use rucene_queryparser::jdk::locale::Locale;

use crate::java::JResult;
use crate::org::xml::sax::document_handler::DocumentHandler;
use crate::org::xml::sax::dtd_handler::DTDHandler;
use crate::org::xml::sax::entity_resolver::EntityResolver;
use crate::org::xml::sax::error_handler::ErrorHandler;
use crate::org::xml::sax::input_source::InputSource;

/// Basic interface for SAX (Simple API for XML) parsers (SAX 1).
///
/// Equivalent to `org.xml.sax.Parser`.
pub trait Parser {
    /// Equivalent to `setLocale(Locale)`.
    fn set_locale(&self, locale: Option<Locale>) -> JResult<()>;
    /// Equivalent to `setEntityResolver(EntityResolver)`.
    fn set_entity_resolver(&self, resolver: Option<Rc<dyn EntityResolver>>);
    /// Equivalent to `setDTDHandler(DTDHandler)`.
    fn set_dtd_handler(&self, handler: Option<Rc<dyn DTDHandler>>);
    /// Equivalent to `setDocumentHandler(DocumentHandler)`.
    fn set_document_handler(&self, handler: Option<Rc<dyn DocumentHandler>>);
    /// Equivalent to `setErrorHandler(ErrorHandler)`.
    fn set_error_handler(&self, handler: Option<Rc<dyn ErrorHandler>>);
    /// Equivalent to `parse(InputSource)`.
    fn parse(&self, source: &InputSource) -> JResult<()>;
    /// Equivalent to `parse(String)`.
    fn parse_system_id(&self, system_id: Option<&str>) -> JResult<()>;
}
