// DefaultHandler.java - default implementation of the core handlers.
// http://www.saxproject.org
// Written by David Megginson
// NO WARRANTY!  This class is in the public domain.
// $Id: DefaultHandler.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `DefaultHandler.java`, of the Java package `org.xml.sax.helpers` (SAX 2.0.2, xml-apis 1.4.01).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::xml::sax::attributes::Attributes;
use crate::org::xml::sax::content_handler::ContentHandler;
use crate::org::xml::sax::dtd_handler::DTDHandler;
use crate::org::xml::sax::entity_resolver::EntityResolver;
use crate::org::xml::sax::error_handler::ErrorHandler;
use crate::org::xml::sax::input_source::InputSource;
use crate::org::xml::sax::locator::Locator;
use crate::org::xml::sax::sax_parse_exception::SAXParseException;

/// Default base class for SAX2 event handlers: every callback does nothing,
/// except `fatalError`, which throws the exception it receives.
///
/// Equivalent to `org.xml.sax.helpers.DefaultHandler`. A Java subclass is a
/// type that implements the four interfaces and calls these bodies for the
/// methods it does not override.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultHandler;

impl DefaultHandler {
    /// Equivalent to `new DefaultHandler()`.
    pub fn new() -> Self {
        Self
    }
}

impl EntityResolver for DefaultHandler {
    fn resolve_entity(
        &self,
        _public_id: Option<&str>,
        _system_id: Option<&str>,
    ) -> JResult<Option<InputSource>> {
        Ok(None)
    }
}

impl DTDHandler for DefaultHandler {
    fn notation_decl(
        &self,
        _name: &str,
        _public_id: Option<&str>,
        _system_id: Option<&str>,
    ) -> JResult<()> {
        Ok(())
    }

    fn unparsed_entity_decl(
        &self,
        _name: &str,
        _public_id: Option<&str>,
        _system_id: Option<&str>,
        _notation_name: &str,
    ) -> JResult<()> {
        Ok(())
    }
}

impl ContentHandler for DefaultHandler {
    fn set_document_locator(&self, _locator: Rc<dyn Locator>) {}

    fn start_document(&self) -> JResult<()> {
        Ok(())
    }

    fn end_document(&self) -> JResult<()> {
        Ok(())
    }

    fn start_prefix_mapping(&self, _prefix: &str, _uri: &str) -> JResult<()> {
        Ok(())
    }

    fn end_prefix_mapping(&self, _prefix: &str) -> JResult<()> {
        Ok(())
    }

    fn start_element(
        &self,
        _uri: &str,
        _local_name: &str,
        _qname: &str,
        _attributes: &dyn Attributes,
    ) -> JResult<()> {
        Ok(())
    }

    fn end_element(&self, _uri: &str, _local_name: &str, _qname: &str) -> JResult<()> {
        Ok(())
    }

    fn characters(&self, _ch: &[u16], _start: i32, _length: i32) -> JResult<()> {
        Ok(())
    }

    fn ignorable_whitespace(&self, _ch: &[u16], _start: i32, _length: i32) -> JResult<()> {
        Ok(())
    }

    fn processing_instruction(&self, _target: &str, _data: Option<&str>) -> JResult<()> {
        Ok(())
    }

    fn skipped_entity(&self, _name: &str) -> JResult<()> {
        Ok(())
    }
}

impl ErrorHandler for DefaultHandler {
    fn warning(&self, _e: &SAXParseException) -> JResult<()> {
        Ok(())
    }

    fn error(&self, _e: &SAXParseException) -> JResult<()> {
        Ok(())
    }

    fn fatal_error(&self, e: &SAXParseException) -> JResult<()> {
        Err(e.clone().into())
    }
}
