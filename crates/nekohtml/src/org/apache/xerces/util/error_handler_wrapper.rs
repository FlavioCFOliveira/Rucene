/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `ErrorHandlerWrapper.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use crate::java::{Exception, JResult};
use crate::org::apache::xerces::xni::parser::xml_error_handler::XMLErrorHandler;
use crate::org::apache::xerces::xni::parser::xml_parse_exception::XMLParseException;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xni_exception::XNIException;
use crate::org::xml::sax::error_handler::ErrorHandler;
use crate::org::xml::sax::sax_exception::SAXException;
use crate::org::xml::sax::sax_parse_exception::SAXParseException;

/// This class wraps a SAX error handler in an XNI error handler.
///
/// Equivalent to `org.apache.xerces.util.ErrorHandlerWrapper`.
#[derive(Default)]
pub struct ErrorHandlerWrapper {
    error_handler: RefCell<Option<Rc<dyn ErrorHandler>>>,
}

impl std::fmt::Debug for ErrorHandlerWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ErrorHandlerWrapper")
    }
}

/// The anonymous `XMLLocator` of `createXMLParseException`.
struct SAXLocation {
    public_id: Option<String>,
    expanded_system_id: Option<String>,
    line_number: i32,
    column_number: i32,
}

impl XMLLocator for SAXLocation {
    fn get_public_id(&self) -> Option<String> {
        self.public_id.clone()
    }

    fn get_expanded_system_id(&self) -> Option<String> {
        self.expanded_system_id.clone()
    }

    fn get_base_system_id(&self) -> Option<String> {
        None
    }

    fn get_literal_system_id(&self) -> Option<String> {
        None
    }

    fn get_column_number(&self) -> i32 {
        self.column_number
    }

    fn get_line_number(&self) -> i32 {
        self.line_number
    }

    fn get_character_offset(&self) -> i32 {
        -1
    }

    fn get_encoding(&self) -> Option<String> {
        None
    }

    fn get_xml_version(&self) -> Option<String> {
        None
    }
}

impl ErrorHandlerWrapper {
    /// Default constructor.
    ///
    /// Equivalent to `new ErrorHandlerWrapper()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps the specified SAX error handler.
    ///
    /// Equivalent to `new ErrorHandlerWrapper(ErrorHandler)`.
    pub fn with_handler(error_handler: Option<Rc<dyn ErrorHandler>>) -> Self {
        let s = Self::new();
        s.set_error_handler(error_handler);
        s
    }

    /// Sets the SAX error handler.
    ///
    /// Equivalent to `setErrorHandler(ErrorHandler)`.
    pub fn set_error_handler(&self, error_handler: Option<Rc<dyn ErrorHandler>>) {
        *self.error_handler.borrow_mut() = error_handler;
    }

    /// Returns the SAX error handler.
    ///
    /// Equivalent to `getErrorHandler()`.
    pub fn get_error_handler(&self) -> Option<Rc<dyn ErrorHandler>> {
        self.error_handler.borrow().clone()
    }

    /// Creates a SAXParseException from an XMLParseException.
    ///
    /// Equivalent to `createSAXParseException(XMLParseException)`.
    pub fn create_sax_parse_exception(exception: &XMLParseException) -> SAXParseException {
        SAXParseException::with_exception(
            exception.get_message(),
            exception.get_public_id(),
            exception.get_expanded_system_id(),
            exception.get_line_number(),
            exception.get_column_number(),
            exception.get_exception().cloned(),
        )
    }

    /// Creates an XMLParseException from a SAXParseException.
    ///
    /// Equivalent to `createXMLParseException(SAXParseException)`.
    pub fn create_xml_parse_exception(exception: &SAXParseException) -> XMLParseException {
        let location = SAXLocation {
            public_id: exception.get_public_id().map(str::to_string),
            expanded_system_id: exception.get_system_id().map(str::to_string),
            line_number: exception.get_line_number(),
            column_number: exception.get_column_number(),
        };
        XMLParseException::with_exception(
            Some(&location),
            exception.get_message().as_deref(),
            Some(Exception::Sax((**exception).clone())),
        )
    }

    /// Creates an XNIException from a SAXException.
    ///
    /// Equivalent to `createXNIException(SAXException)`.
    pub fn create_xni_exception(exception: &SAXException) -> XNIException {
        XNIException::with_message_and_exception(
            exception.get_message().as_deref(),
            Some(Exception::Sax(exception.clone())),
        )
    }

    fn convert(e: Exception) -> Exception {
        match e {
            Exception::Sax(e) => match SAXParseException::from_sax_exception(&e) {
                Some(pe) => Self::create_xml_parse_exception(&pe).into(),
                None => Self::create_xni_exception(&e).into(),
            },
            other => other,
        }
    }

    fn call(
        &self,
        exception: &XMLParseException,
        f: impl FnOnce(&dyn ErrorHandler, &SAXParseException) -> JResult<()>,
    ) -> JResult<()> {
        if let Some(handler) = self.get_error_handler() {
            let sax_exception = Self::create_sax_parse_exception(exception);
            f(&*handler, &sax_exception).map_err(Self::convert)?;
        }
        Ok(())
    }
}

impl XMLErrorHandler for ErrorHandlerWrapper {
    fn warning(&self, _domain: &str, _key: &str, exception: &XMLParseException) -> JResult<()> {
        self.call(exception, |h, e| h.warning(e))
    }

    fn error(&self, _domain: &str, _key: &str, exception: &XMLParseException) -> JResult<()> {
        self.call(exception, |h, e| h.error(e))
    }

    fn fatal_error(&self, _domain: &str, _key: &str, exception: &XMLParseException) -> JResult<()> {
        self.call(exception, |h, e| h.fatal_error(e))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
