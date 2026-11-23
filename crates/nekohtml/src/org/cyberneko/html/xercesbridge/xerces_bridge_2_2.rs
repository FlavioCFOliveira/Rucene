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

//! Port of `XercesBridge_2_2.java`, of the Java package `org.cyberneko.html.xercesbridge` (NekoHTML 1.9.17).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::apache::xerces::r#impl::version::Version;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_document_filter::XMLDocumentFilter;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge::XercesBridge;

/// Xerces bridge for use with Xerces 2.2 and higher.
///
/// Equivalent to `org.cyberneko.html.xercesbridge.XercesBridge_2_2`; its
/// methods are the provided methods of [`XercesBridge22Methods`], which
/// `XercesBridge_2_3` inherits.
#[derive(Debug, Default)]
pub struct XercesBridge22;

impl XercesBridge22 {
    /// Should fail for Xerces version less than 2.2 (it cannot fail against
    /// Xerces 2.12.2).
    ///
    /// Equivalent to `new XercesBridge_2_2()`.
    pub const fn new() -> Self {
        Self
    }
}

/// The bodies of `XercesBridge_2_2`, for it and its subclass.
pub trait XercesBridge22Methods {
    /// Equivalent to `getVersion()`: the Xerces version.
    fn version_2_2(&self) -> String {
        Version::get_version().to_string()
    }

    /// Equivalent to `XMLDocumentHandler_startDocument(...)`: the
    /// four-argument `startDocument`.
    ///
    /// # Errors
    ///
    /// What the handler throws.
    fn start_document_2_2(
        &self,
        document_handler: &dyn XMLDocumentHandler,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        nscontext: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        document_handler.start_document(locator, encoding, nscontext, augs)
    }

    /// Equivalent to `XMLDocumentFilter_setDocumentSource(...)`.
    fn set_document_source_2_2(&self, filter: &dyn XMLDocumentFilter, last_source: Option<Rc<dyn XMLDocumentSource>>) {
        filter.set_document_source(last_source);
    }
}

impl XercesBridge22Methods for XercesBridge22 {}

impl XercesBridge for XercesBridge22 {
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

    /// Does nothing, not needed.
    fn xml_document_handler_start_prefix_mapping(
        &self,
        _document_handler: &dyn XMLDocumentHandler,
        _prefix: &str,
        _uri: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) {
    }

    fn xml_document_filter_set_document_source(&self, filter: &dyn XMLDocumentFilter, last_source: Option<Rc<dyn XMLDocumentSource>>) {
        self.set_document_source_2_2(filter, last_source);
    }
}
