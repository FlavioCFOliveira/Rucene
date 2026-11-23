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

use crate::java::JResult;
use crate::org::apache::xerces::parsers::xml_parser::XMLParser;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::parser::xml_dtd_content_model_source::XMLDTDContentModelSource;
use crate::org::apache::xerces::xni::parser::xml_dtd_source::XMLDTDSource;
use crate::org::apache::xerces::xni::parser::xml_parser_configuration::XMLParserConfiguration;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xml_string::XMLString;

/// This is the base class for all XML document parsers. XMLDocumentParser
/// provides a common implementation shared by the various document parsers
/// in the Xerces package.
///
/// Equivalent to the abstract class
/// `org.apache.xerces.parsers.AbstractXMLDocumentParser`: its state, held
/// by a subclass as its `base`; its handler bodies (empty, except
/// `emptyElement`, `startDTD` and `endDTD`) are the defaults the subclass
/// calls where Java inherits them. The subclass registers itself with the
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

    /// An empty element: `startElement(element, attributes, augs)` then
    /// `endElement(element, augs)`, dispatched to the subclass `this` (Java's
    /// virtual calls).
    ///
    /// Equivalent to `AbstractXMLDocumentParser.emptyElement(QName, XMLAttributes, Augmentations)`.
    ///
    /// # Errors
    ///
    /// The `XNIException` of the subclass's callbacks.
    pub fn empty_element<H: XMLDocumentHandler + ?Sized>(
        this: &H,
        element: &mut QName,
        attributes: &mut dyn XMLAttributes,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        this.start_element(element, attributes, augs)?;
        this.end_element(element, augs)
    }

    /// The start of the document. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startDocument(XMLLocator, String, NamespaceContext, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_document(
        &self,
        _locator: Option<Rc<dyn XMLLocator>>,
        _encoding: Option<&str>,
        _namespace_context: Option<Rc<dyn NamespaceContext>>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// Notifies of the presence of an XMLDecl line in the document. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.xmlDecl(String, String, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn xml_decl(
        &self,
        _version: Option<&str>,
        _encoding: Option<&str>,
        _standalone: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// Notifies of the presence of the DOCTYPE line in the document. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.doctypeDecl(String, String, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn doctype_decl(
        &self,
        _root_element: Option<&str>,
        _public_id: Option<&str>,
        _system_id: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The start of an element. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startElement(QName, XMLAttributes, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_element(
        &self,
        _element: &mut QName,
        _attributes: &mut dyn XMLAttributes,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// Character content. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.characters(XMLString, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn characters(&self, _text: &XMLString, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// Ignorable whitespace. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.ignorableWhitespace(XMLString, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn ignorable_whitespace(
        &self,
        _text: &XMLString,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The end of an element. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endElement(QName, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_element(
        &self,
        _element: &mut QName,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The start of a CDATA section. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startCDATA(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_cdata(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The end of a CDATA section. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endCDATA(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_cdata(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The end of the document. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endDocument(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_document(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// Notifies the start of an entity. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startGeneralEntity(String, XMLResourceIdentifier, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_general_entity(
        &self,
        _name: &str,
        _identifier: Option<&dyn XMLResourceIdentifier>,
        _encoding: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// Notifies of the presence of a TextDecl line in an entity (one method for the `XMLDocumentHandler` and `XMLDTDHandler` callbacks, as in Java). Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.textDecl(String, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn text_decl(
        &self,
        _version: Option<&str>,
        _encoding: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// Notifies the end of an entity. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endGeneralEntity(String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_general_entity(
        &self,
        _name: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// A comment (one method for both handler interfaces, as in Java). Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.comment(XMLString, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn comment(&self, _text: &XMLString, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// A processing instruction (one method for both handler interfaces, as in Java). Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.processingInstruction(String, XMLString, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn processing_instruction(
        &self,
        _target: &str,
        _data: Option<&XMLString>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The start of the DTD external subset. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startExternalSubset(XMLResourceIdentifier, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_external_subset(
        &self,
        _identifier: Option<&dyn XMLResourceIdentifier>,
        _augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The end of the DTD external subset. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endExternalSubset(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_external_subset(&self, _augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// Notifies the start of a parameter entity. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startParameterEntity(String, XMLResourceIdentifier, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_parameter_entity(
        &self,
        _name: &str,
        _identifier: Option<&dyn XMLResourceIdentifier>,
        _encoding: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// Notifies the end of a parameter entity. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endParameterEntity(String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_parameter_entity(
        &self,
        _name: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// Characters within an IGNORE conditional section. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.ignoredCharacters(XMLString, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn ignored_characters(
        &self,
        _text: &XMLString,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// An element declaration. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.elementDecl(String, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn element_decl(
        &self,
        _name: &str,
        _content_model: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The start of an attribute list. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startAttlist(String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_attlist(
        &self,
        _element_name: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// An attribute declaration. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.attributeDecl(String, String, String, String[], String, XMLString, XMLString, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    #[allow(clippy::too_many_arguments)]
    pub fn attribute_decl(
        &self,
        _element_name: &str,
        _attribute_name: &str,
        _type: &str,
        _enumeration: Option<&[String]>,
        _default_type: Option<&str>,
        _default_value: Option<&XMLString>,
        _non_normalized_default_value: Option<&XMLString>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The end of an attribute list. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endAttlist(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_attlist(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// An internal entity declaration. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.internalEntityDecl(String, XMLString, XMLString, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn internal_entity_decl(
        &self,
        _name: &str,
        _text: &XMLString,
        _non_normalized_text: &XMLString,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// An external entity declaration. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.externalEntityDecl(String, XMLResourceIdentifier, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn external_entity_decl(
        &self,
        _name: &str,
        _identifier: &dyn XMLResourceIdentifier,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// An unparsed entity declaration. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.unparsedEntityDecl(String, XMLResourceIdentifier, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn unparsed_entity_decl(
        &self,
        _name: &str,
        _identifier: &dyn XMLResourceIdentifier,
        _notation: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// A notation declaration. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.notationDecl(String, XMLResourceIdentifier, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn notation_decl(
        &self,
        _name: &str,
        _identifier: &dyn XMLResourceIdentifier,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// The start of a conditional section. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startConditional(short, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_conditional(&self, _type: i16, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The end of a conditional section. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endConditional(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_conditional(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The start of a content model. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startContentModel(String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_content_model(
        &self,
        _element_name: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// A content model of ANY. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.any(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn any(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// A content model of EMPTY. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.empty(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn empty(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// A start of either a mixed or children content model. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.startGroup(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn start_group(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The appearance of `#PCDATA` within a group. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.pcdata(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn pcdata(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// A referenced element in a mixed or children content model. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.element(String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn element(&self, _element_name: &str, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The separator between choices or sequences of a mixed or children content model. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.separator(short, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn separator(&self, _separator: i16, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The occurrence count for a child in a children content model or for the mixed content model group. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.occurrence(short, Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn occurrence(&self, _occurrence: i16, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The end of a group for mixed or children content models. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endGroup(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_group(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// The end of a content model. Java's body is empty.
    ///
    /// Equivalent to `AbstractXMLDocumentParser.endContentModel(Augmentations)`.
    ///
    /// # Errors
    ///
    /// None; the `XNIException` of the Java signature.
    pub fn end_content_model(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        Ok(())
    }

    /// Reset all components before parsing.
    ///
    /// Equivalent to the protected `reset()`.
    pub fn reset(&self) {
        self.base.reset();
        self.in_dtd.set(false);
    }
}
