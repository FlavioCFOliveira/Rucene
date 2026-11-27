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

//! Port of `XMLDocumentFilter.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::cyberneko::html::html_component::HTMLComponent;

/// Defines a document filter that acts as both a receiver and an emitter of
/// document events.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLDocumentFilter`.
pub trait XMLDocumentFilter: XMLDocumentHandler + XMLDocumentSource {
    /// This filter as the [`XMLDocumentHandler`] it is.
    fn as_document_handler(self: Rc<Self>) -> Rc<dyn XMLDocumentHandler>;

    /// This filter as the [`XMLDocumentSource`] it is.
    fn as_document_source(self: Rc<Self>) -> Rc<dyn XMLDocumentSource>;

    /// `this instanceof HTMLComponent ? (HTMLComponent) this : null`.
    fn as_html_component(self: Rc<Self>) -> Option<Rc<dyn HTMLComponent>> {
        None
    }
}
