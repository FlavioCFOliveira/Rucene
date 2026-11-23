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

//! Port of `AbstractXMLDocumentParser.java`, of the Java package `org.apache.xerces.parsers` (Xerces-J 2.12.2).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use crate::org::apache::xerces::parsers::xml_parser::XMLParser;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::parser::xml_dtd_content_model_source::XMLDTDContentModelSource;
use crate::org::apache::xerces::xni::parser::xml_dtd_source::XMLDTDSource;
use crate::org::apache::xerces::xni::parser::xml_parser_configuration::XMLParserConfiguration;

/// This is the base class for all XML document parsers. XMLDocumentParser
/// provides a common implementation shared by the various document parsers
/// in the Xerces package.
///
/// Equivalent to the abstract class
/// `org.apache.xerces.parsers.AbstractXMLDocumentParser`: its state, held
/// by a subclass as its `base`; its empty handler bodies are the defaults
/// the subclass falls back to. The subclass registers itself with the
/// configuration as the document, DTD and DTD content model handler, as
/// the Java constructor does. The sources are held weakly (see the crate
/// documentation on reference cycles).
pub struct AbstractXMLDocumentParser {
    /// The `XMLParser` this class extends.
    pub base: XMLParser,
    /// `fInDTD`.
    pub in_dtd: Cell<bool>,
    /// Document source.
    pub document_source: RefCell<Option<Weak<dyn XMLDocumentSource>>>,
    /// DTD source.
    pub dtd_source: RefCell<Option<Weak<dyn XMLDTDSource>>>,
    /// DTD content model source.
    pub dtd_content_model_source: RefCell<Option<Weak<dyn XMLDTDContentModelSource>>>,
}

impl std::fmt::Debug for AbstractXMLDocumentParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbstractXMLDocumentParser")
            .field("in_dtd", &self.in_dtd.get())
            .finish_non_exhaustive()
    }
}

impl AbstractXMLDocumentParser {
    /// Constructs a document parser using the default symbol table and
    /// grammar pool.
    ///
    /// Equivalent to `AbstractXMLDocumentParser(XMLParserConfiguration)`,
    /// less the handler registration (see the type's documentation).
    pub fn new(config: Rc<dyn XMLParserConfiguration>) -> Self {
        Self {
            base: XMLParser::new(config),
            in_dtd: Cell::new(false),
            document_source: RefCell::new(None),
            dtd_source: RefCell::new(None),
            dtd_content_model_source: RefCell::new(None),
        }
    }

    /// Sets the document source.
    ///
    /// Equivalent to `setDocumentSource(XMLDocumentSource)`.
    pub fn set_document_source(&self, source: Option<Rc<dyn XMLDocumentSource>>) {
        *self.document_source.borrow_mut() = source.map(|s| Rc::downgrade(&s));
    }

    /// Returns the document source.
    ///
    /// Equivalent to `getDocumentSource()`.
    pub fn get_document_source(&self) -> Option<Rc<dyn XMLDocumentSource>> {
        self.document_source
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade)
    }

    /// Equivalent to `setDTDSource(XMLDTDSource)`.
    pub fn set_dtd_source(&self, source: Option<Rc<dyn XMLDTDSource>>) {
        *self.dtd_source.borrow_mut() = source.map(|s| Rc::downgrade(&s));
    }

    /// Equivalent to `getDTDSource()`.
    pub fn get_dtd_source(&self) -> Option<Rc<dyn XMLDTDSource>> {
        self.dtd_source.borrow().as_ref().and_then(Weak::upgrade)
    }

    /// Equivalent to `setDTDContentModelSource(XMLDTDContentModelSource)`.
    pub fn set_dtd_content_model_source(&self, source: Option<Rc<dyn XMLDTDContentModelSource>>) {
        *self.dtd_content_model_source.borrow_mut() = source.map(|s| Rc::downgrade(&s));
    }

    /// Equivalent to `getDTDContentModelSource()`.
    pub fn get_dtd_content_model_source(&self) -> Option<Rc<dyn XMLDTDContentModelSource>> {
        self.dtd_content_model_source
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade)
    }

    /// Equivalent to the base `startDTD(XMLLocator, Augmentations)`.
    pub fn start_dtd(&self) {
        self.in_dtd.set(true);
    }

    /// Equivalent to the base `endDTD(Augmentations)`.
    pub fn end_dtd(&self) {
        self.in_dtd.set(false);
    }

    /// Reset all components before parsing.
    ///
    /// Equivalent to the protected `reset()`.
    pub fn reset(&self) {
        self.base.reset();
        self.in_dtd.set(false);
    }
}
