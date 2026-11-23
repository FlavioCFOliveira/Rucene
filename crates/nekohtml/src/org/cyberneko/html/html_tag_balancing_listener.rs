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

//! Port of `HTMLTagBalancingListener.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;

/// Extension of the XMLDocumentHandler to receive information on the
/// elements ignored by the tag balancer.
///
/// Equivalent to `org.cyberneko.html.HTMLTagBalancingListener`; a document
/// handler that is one answers `Some(self)` from
/// `XMLDocumentHandler::as_tag_balancing_listener`.
pub trait HTMLTagBalancingListener {
    /// Notifies that the start element has been ignored.
    fn ignored_start_element(&self, elem: &QName, attrs: &dyn XMLAttributes, augs: Option<&dyn Augmentations>);
    /// Notifies that the end element has been ignored.
    fn ignored_end_element(&self, element: &QName, augs: Option<&dyn Augmentations>);
}
