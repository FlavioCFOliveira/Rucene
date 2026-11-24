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

//! Port of `XMLDocumentSource.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;

/// Defines a document source. In other words, any object that implements
/// this interface is able to emit document "events" to the registered
/// document handler.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLDocumentSource`.
pub trait XMLDocumentSource {
    /// Equivalent to `setDocumentHandler(XMLDocumentHandler)`.
    fn set_document_handler(&self, handler: Option<Rc<dyn XMLDocumentHandler>>);
    /// Equivalent to `getDocumentHandler()`.
    fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>>;
}
