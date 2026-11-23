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

//! Port of `XMLDocumentHandler.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xml_string::XMLString;
use crate::org::cyberneko::html::html_tag_balancing_listener::HTMLTagBalancingListener;

/// The document handler interface defines callback methods to report
/// information items in XML documents.
///
/// Equivalent to `org.apache.xerces.xni.XMLDocumentHandler`; each `throws
/// XNIException` is the method's `Err`. The element name and attributes are
/// passed mutably, as the Java objects a filter may modify before passing
/// them on.
pub trait XMLDocumentHandler {
    /// Equivalent to `startDocument(XMLLocator, String, NamespaceContext,
    /// Augmentations)`.
    fn start_document(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        namespace_context: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `xmlDecl(String, String, String, Augmentations)`.
    fn xml_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        standalone: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `doctypeDecl(String, String, String, Augmentations)`.
    fn doctype_decl(
        &self,
        root_element: Option<&str>,
        public_id: Option<&str>,
        system_id: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `comment(XMLString, Augmentations)`.
    fn comment(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `processingInstruction(String, XMLString,
    /// Augmentations)`.
    fn processing_instruction(&self, target: &str, data: Option<&XMLString>, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `startElement(QName, XMLAttributes, Augmentations)`.
    fn start_element(&self, element: &mut QName, attributes: &mut dyn XMLAttributes, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `emptyElement(QName, XMLAttributes, Augmentations)`.
    fn empty_element(&self, element: &mut QName, attributes: &mut dyn XMLAttributes, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `startGeneralEntity(String, XMLResourceIdentifier,
    /// String, Augmentations)`.
    fn start_general_entity(
        &self,
        name: &str,
        identifier: Option<&dyn XMLResourceIdentifier>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `textDecl(String, String, Augmentations)`.
    fn text_decl(&self, version: Option<&str>, encoding: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `endGeneralEntity(String, Augmentations)`.
    fn end_general_entity(&self, name: &str, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `characters(XMLString, Augmentations)`.
    fn characters(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `ignorableWhitespace(XMLString, Augmentations)`.
    fn ignorable_whitespace(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `endElement(QName, Augmentations)`.
    fn end_element(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `startCDATA(Augmentations)`.
    fn start_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `endCDATA(Augmentations)`.
    fn end_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `endDocument(Augmentations)`.
    fn end_document(&self, augs: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `setDocumentSource(XMLDocumentSource)`.
    fn set_document_source(&self, source: Option<Rc<dyn XMLDocumentSource>>);
    /// Equivalent to `getDocumentSource()`.
    fn get_document_source(&self) -> Option<Rc<dyn XMLDocumentSource>>;

    /// `this instanceof HTMLTagBalancingListener ?
    /// (HTMLTagBalancingListener) this : null`.
    fn as_tag_balancing_listener(self: Rc<Self>) -> Option<Rc<dyn HTMLTagBalancingListener>> {
        None
    }
}
