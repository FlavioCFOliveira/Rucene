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

//! Port of `XercesBridge.java`, of the Java package `org.cyberneko.html.xercesbridge` (NekoHTML 1.9.17).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_document_filter::XMLDocumentFilter;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge_2_3::XercesBridge23;

/// This class allows to transparently handle Xerces methods that have
/// changed among versions.
///
/// Equivalent to the abstract class
/// `org.cyberneko.html.xercesbridge.XercesBridge`: its concrete methods are
/// this trait's provided methods.
///
/// **Divergence.** Java chooses the bridge at class initialization by
/// instantiating, through `Class.forName`, the first of `XercesBridge_2_3`,
/// `_2_2`, `_2_1`, `_2_0` whose constructor accepts the Xerces on the class
/// path (`XercesBridge.java:40-73`). Rust has no class loading: the port is
/// built against Xerces 2.12.2, for which Java selects `XercesBridge_2_3`
/// (its constructor finds `NamespaceContext.declarePrefix(String, String)`),
/// so [`get_instance`] returns that bridge. The older bridges, which Java
/// never instantiates against Xerces 2.12.2, are not ported.
pub trait XercesBridge {
    /// Equivalent to `NamespaceContext_declarePrefix(NamespaceContext,
    /// String, String)`.
    fn namespace_context_declare_prefix(&self, _namespace_context: &dyn NamespaceContext, _ns: &str, _avalue: Option<&str>) {}

    /// Gets the Xerces version used.
    ///
    /// Equivalent to `getVersion()`.
    fn get_version(&self) -> String;

    /// Calls `startDocument` on the handler with the correct number of
    /// parameters.
    ///
    /// Equivalent to `XMLDocumentHandler_startDocument(XMLDocumentHandler,
    /// XMLLocator, String, NamespaceContext, Augmentations)`.
    ///
    /// # Errors
    ///
    /// What the handler throws.
    fn xml_document_handler_start_document(
        &self,
        document_handler: &dyn XMLDocumentHandler,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        nscontext: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()>;

    /// Calls `startPrefixMapping` on the handler (`XMLDocumentHandler` has
    /// no such method in recent Xerces versions).
    ///
    /// Equivalent to `XMLDocumentHandler_startPrefixMapping(
    /// XMLDocumentHandler, String, String, Augmentations)`.
    fn xml_document_handler_start_prefix_mapping(
        &self,
        _document_handler: &dyn XMLDocumentHandler,
        _prefix: &str,
        _uri: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) {
    }

    /// Calls `endPrefixMapping` on the handler.
    ///
    /// Equivalent to `XMLDocumentHandler_endPrefixMapping(
    /// XMLDocumentHandler, String, Augmentations)`.
    fn xml_document_handler_end_prefix_mapping(
        &self,
        _document_handler: &dyn XMLDocumentHandler,
        _prefix: &str,
        _augs: Option<&dyn Augmentations>,
    ) {
    }

    /// Calls `setDocumentSource` (if available in the Xerces version used)
    /// on the filter.
    ///
    /// Equivalent to `XMLDocumentFilter_setDocumentSource(
    /// XMLDocumentFilter, XMLDocumentSource)`.
    fn xml_document_filter_set_document_source(
        &self,
        _filter: &dyn XMLDocumentFilter,
        _last_source: Option<Rc<dyn XMLDocumentSource>>,
    ) {
    }
}

/// The unique instance of the current bridge class.
///
/// Equivalent to `XercesBridge.getInstance()` (see the trait's divergence).
pub fn get_instance() -> &'static dyn XercesBridge {
    static INSTANCE: XercesBridge23 = XercesBridge23::new();
    &INSTANCE
}
