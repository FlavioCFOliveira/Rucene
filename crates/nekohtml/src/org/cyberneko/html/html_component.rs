/* 
 * Copyright 2004-2008 Andy Clark, Marc Guillemot
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

//! Port of `HTMLComponent.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use crate::org::apache::xerces::xni::parser::xml_component::XMLComponent;

/// This interface extends the XNI `XMLComponent` interface to add methods
/// that allow the preferred default values for features and properties to
/// be queried.
///
/// Equivalent to `org.cyberneko.html.HTMLComponent` (its two methods are
/// those of [`XMLComponent`] in this port, which declares them as Xerces
/// 2.12.2's `XMLComponent` does).
pub trait HTMLComponent: XMLComponent {}
