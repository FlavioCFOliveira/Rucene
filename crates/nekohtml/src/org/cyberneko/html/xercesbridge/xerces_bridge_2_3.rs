/*
 * Copyright Marc Guillemot
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

//! Port of `XercesBridge_2_3.java`, of the Java package `org.cyberneko.html.xercesbridge` (NekoHTML 1.9.17).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_document_filter::XMLDocumentFilter;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge::XercesBridge;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge_2_2::XercesBridge22Methods;

/// Xerces bridge for use with Xerces 2.3 and higher.
///
/// Equivalent to `org.cyberneko.html.xercesbridge.XercesBridge_2_3` (a
/// `XercesBridge_2_2`, whose bodies it inherits through
/// [`XercesBridge22Methods`]).
#[derive(Debug, Default)]
pub struct XercesBridge23;

impl XercesBridge23 {
    /// Should fail for Xerces version less than 2.3: Java looks up
    /// `NamespaceContext.declarePrefix(String, String)` reflectively; Xerces
    /// 2.12.2 has it, so construction succeeds.
    ///
    /// Equivalent to `new XercesBridge_2_3()`.
    pub const fn new() -> Self {
        Self
    }
}

impl XercesBridge22Methods for XercesBridge23 {}

impl XercesBridge for XercesBridge23 {
    fn namespace_context_declare_prefix(
        &self,
        namespace_context: &dyn NamespaceContext,
        ns: &str,
        avalue: Option<&str>,
    ) {
        namespace_context.declare_prefix(ns, avalue);
    }

    fn get_version(&self) -> String {
        self.version_2_2()
    }

    fn xml_document_handler_start_document(
        &self,
        document_handler: &dyn XMLDocumentHandler,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        nscontext: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.start_document_2_2(document_handler, locator, encoding, nscontext, augs)
    }

    /// `XercesBridge_2_2`'s body: does nothing, not needed.
    fn xml_document_handler_start_prefix_mapping(
        &self,
        _document_handler: &dyn XMLDocumentHandler,
        _prefix: &str,
        _uri: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) {
    }

    fn xml_document_filter_set_document_source(
        &self,
        filter: &dyn XMLDocumentFilter,
        last_source: Option<Rc<dyn XMLDocumentSource>>,
    ) {
        self.set_document_source_2_2(filter, last_source);
    }
}
