/* 
 * Copyright 2002-2009 Andy Clark, Marc Guillemot
 * 
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `DefaultFilter.java`, of the Java package `org.cyberneko.html.filters` (NekoHTML 1.9.17).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::java::{JResult, Object};
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_component::XMLComponent;
use crate::org::apache::xerces::xni::parser::xml_component_manager::XMLComponentManager;
use crate::org::apache::xerces::xni::parser::xml_document_filter::XMLDocumentFilter;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xml_string::XMLString;
use crate::org::cyberneko::html::html_component::HTMLComponent;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge;

/// This class implements a filter that simply passes document events to the
/// next handler. It can be used as a base class to simplify the development
/// of new document filters.
///
/// Equivalent to `org.cyberneko.html.filters.DefaultFilter`; a subclass
/// holds one as its `base` and calls its methods for `super`. The document
/// source is held weakly (see the crate documentation).
#[derive(Default)]
pub struct DefaultFilter {
    /// Next document handler (`fDocumentHandler`).
    pub document_handler: RefCell<Option<Rc<dyn XMLDocumentHandler>>>,
    /// Previous document source (`fDocumentSource`).
    pub document_source: RefCell<Option<Weak<dyn XMLDocumentSource>>>,
}

impl std::fmt::Debug for DefaultFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DefaultFilter")
    }
}

macro_rules! next {
    ($self:ident, $h:ident => $body:expr) => {{
        let handler = $self.document_handler.borrow().clone();
        if let Some($h) = handler {
            $body
        } else {
            Ok(())
        }
    }};
}

/// Utility method for merging string arrays for recognized features and
/// recognized properties.
///
/// Equivalent to `DefaultFilter.merge(String[], String[])`.
pub fn merge(array1: Option<Vec<String>>, array2: Option<Vec<String>>) -> Option<Vec<String>> {
    match (array1, array2) {
        (None, a2) => a2,
        (a1, None) => a1,
        (Some(mut a1), Some(a2)) => {
            a1.extend(a2);
            Some(a1)
        }
    }
}

impl DefaultFilter {
    /// Equivalent to `new DefaultFilter()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The next handler (`fDocumentHandler`).
    pub fn next_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.document_handler.borrow().clone()
    }

    /// Equivalent to the three-argument `startDocument(XMLLocator,
    /// String, Augmentations)` (the XNI 2.0 form), which passes a `null`
    /// namespace context.
    ///
    /// # Errors
    ///
    /// What the next handler throws.
    pub fn start_document_no_context(&self, locator: Option<Rc<dyn XMLLocator>>, encoding: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        XMLDocumentHandler::start_document(self, locator, encoding, None, augs)
    }

    /// Start prefix mapping (XNI 2.0).
    ///
    /// Equivalent to `startPrefixMapping(String, String, Augmentations)`.
    pub fn start_prefix_mapping(&self, prefix: &str, uri: Option<&str>, augs: Option<&dyn Augmentations>) {
        if let Some(h) = self.next_handler() {
            xerces_bridge::get_instance().xml_document_handler_start_prefix_mapping(&*h, prefix, uri, augs);
        }
    }

    /// End prefix mapping (XNI 2.0).
    ///
    /// Equivalent to `endPrefixMapping(String, Augmentations)`.
    pub fn end_prefix_mapping(&self, prefix: &str, augs: Option<&dyn Augmentations>) {
        if let Some(h) = self.next_handler() {
            xerces_bridge::get_instance().xml_document_handler_end_prefix_mapping(&*h, prefix, augs);
        }
    }
}

impl XMLDocumentSource for DefaultFilter {
    fn set_document_handler(&self, handler: Option<Rc<dyn XMLDocumentHandler>>) {
        *self.document_handler.borrow_mut() = handler;
    }

    fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.document_handler.borrow().clone()
    }
}

impl XMLDocumentHandler for DefaultFilter {
    fn start_document(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        nscontext: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        next!(self, h => xerces_bridge::get_instance().xml_document_handler_start_document(&*h, locator, encoding, nscontext, augs))
    }

    fn xml_decl(&self, version: Option<&str>, encoding: Option<&str>, standalone: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.xml_decl(version, encoding, standalone, augs))
    }

    fn doctype_decl(&self, root: Option<&str>, public_id: Option<&str>, system_id: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.doctype_decl(root, public_id, system_id, augs))
    }

    fn comment(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.comment(text, augs))
    }

    fn processing_instruction(&self, target: &str, data: Option<&XMLString>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.processing_instruction(target, data, augs))
    }

    fn start_element(&self, element: &mut QName, attributes: &mut dyn XMLAttributes, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.start_element(element, attributes, augs))
    }

    fn empty_element(&self, element: &mut QName, attributes: &mut dyn XMLAttributes, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.empty_element(element, attributes, augs))
    }

    fn start_general_entity(
        &self,
        name: &str,
        id: Option<&dyn XMLResourceIdentifier>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        next!(self, h => h.start_general_entity(name, id, encoding, augs))
    }

    fn text_decl(&self, version: Option<&str>, encoding: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.text_decl(version, encoding, augs))
    }

    fn end_general_entity(&self, name: &str, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.end_general_entity(name, augs))
    }

    fn characters(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.characters(text, augs))
    }

    fn ignorable_whitespace(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.ignorable_whitespace(text, augs))
    }

    fn end_element(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.end_element(element, augs))
    }

    fn start_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.start_cdata(augs))
    }

    fn end_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.end_cdata(augs))
    }

    fn end_document(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        next!(self, h => h.end_document(augs))
    }

    fn set_document_source(&self, source: Option<Rc<dyn XMLDocumentSource>>) {
        *self.document_source.borrow_mut() = source.map(|s| Rc::downgrade(&s));
    }

    fn get_document_source(&self) -> Option<Rc<dyn XMLDocumentSource>> {
        self.document_source.borrow().as_ref().and_then(Weak::upgrade)
    }
}

impl XMLComponent for DefaultFilter {
    /// Returns a list of feature identifiers that are recognized by this
    /// component: none.
    fn get_recognized_features(&self) -> Option<Vec<String>> {
        None
    }

    fn get_feature_default(&self, _feature_id: &str) -> Option<bool> {
        None
    }

    fn get_recognized_properties(&self) -> Option<Vec<String>> {
        None
    }

    fn get_property_default(&self, _property_id: &str) -> Option<Object> {
        None
    }

    fn reset(&self, _component_manager: &dyn XMLComponentManager) -> JResult<()> {
        Ok(())
    }

    fn set_feature(&self, _feature_id: &str, _state: bool) -> JResult<()> {
        Ok(())
    }

    fn set_property(&self, _property_id: &str, _value: Option<Object>) -> JResult<()> {
        Ok(())
    }
}

impl HTMLComponent for DefaultFilter {}

impl XMLDocumentFilter for DefaultFilter {
    fn as_document_handler(self: Rc<Self>) -> Rc<dyn XMLDocumentHandler> {
        self
    }

    fn as_document_source(self: Rc<Self>) -> Rc<dyn XMLDocumentSource> {
        self
    }

    fn as_html_component(self: Rc<Self>) -> Option<Rc<dyn HTMLComponent>> {
        Some(self)
    }
}
