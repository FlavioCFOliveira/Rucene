// XMLReader.java - read an XML document.
// http://www.saxproject.org
// Written by David Megginson
// NO WARRANTY!  This class is in the Public Domain.
// $Id: XMLReader.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `XMLReader.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use std::rc::Rc;

use crate::java::{JResult, Object};
use crate::org::xml::sax::content_handler::ContentHandler;
use crate::org::xml::sax::dtd_handler::DTDHandler;
use crate::org::xml::sax::entity_resolver::EntityResolver;
use crate::org::xml::sax::error_handler::ErrorHandler;
use crate::org::xml::sax::input_source::InputSource;

/// Interface for reading an XML document using callbacks.
///
/// Equivalent to `org.xml.sax.XMLReader`.
pub trait XMLReader {
    /// Equivalent to `getFeature(String)`.
    fn get_feature(&self, name: &str) -> JResult<bool>;
    /// Equivalent to `setFeature(String, boolean)`.
    fn set_feature(&self, name: &str, value: bool) -> JResult<()>;
    /// Equivalent to `getProperty(String)`.
    fn get_property(&self, name: &str) -> JResult<Option<Object>>;
    /// Equivalent to `setProperty(String, Object)`.
    fn set_property(&self, name: &str, value: Option<Object>) -> JResult<()>;
    /// Equivalent to `setEntityResolver(EntityResolver)`.
    fn set_entity_resolver(&self, resolver: Option<Rc<dyn EntityResolver>>);
    /// Equivalent to `getEntityResolver()`.
    fn get_entity_resolver(&self) -> Option<Rc<dyn EntityResolver>>;
    /// Equivalent to `setDTDHandler(DTDHandler)`.
    fn set_dtd_handler(&self, handler: Option<Rc<dyn DTDHandler>>);
    /// Equivalent to `getDTDHandler()`.
    fn get_dtd_handler(&self) -> Option<Rc<dyn DTDHandler>>;
    /// Equivalent to `setContentHandler(ContentHandler)`.
    fn set_content_handler(&self, handler: Option<Rc<dyn ContentHandler>>);
    /// Equivalent to `getContentHandler()`.
    fn get_content_handler(&self) -> Option<Rc<dyn ContentHandler>>;
    /// Equivalent to `setErrorHandler(ErrorHandler)`.
    fn set_error_handler(&self, handler: Option<Rc<dyn ErrorHandler>>);
    /// Equivalent to `getErrorHandler()`.
    fn get_error_handler(&self) -> Option<Rc<dyn ErrorHandler>>;
    /// Equivalent to `parse(InputSource)`.
    fn parse(&self, input: &InputSource) -> JResult<()>;
    /// Equivalent to `parse(String)`.
    fn parse_system_id(&self, system_id: Option<&str>) -> JResult<()>;
}
