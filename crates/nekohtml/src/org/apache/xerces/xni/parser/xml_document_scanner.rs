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

//! Port of `XMLDocumentScanner.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use crate::java::JResult;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;

/// This interface defines a generic document scanner.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLDocumentScanner`.
pub trait XMLDocumentScanner: XMLDocumentSource {
    /// Equivalent to `setInputSource(XMLInputSource)`.
    fn set_input_source(&self, input_source: &XMLInputSource) -> JResult<()>;
    /// Equivalent to `scanDocument(boolean)`.
    fn scan_document(&self, complete: bool) -> JResult<bool>;
}
