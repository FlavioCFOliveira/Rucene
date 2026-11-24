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

//! Port of `XMLGrammarDescription.java`, of the Java package `org.apache.xerces.xni.grammars` (Xerces-J 2.12.2).

use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;

/// All information specific to a particular grammar type.
///
/// Equivalent to `org.apache.xerces.xni.grammars.XMLGrammarDescription`.
pub trait XMLGrammarDescription: XMLResourceIdentifier {
    /// Equivalent to `getGrammarType()`.
    fn get_grammar_type(&self) -> String;
}

/// The grammar type constant for XML Schema grammars.
pub const XML_SCHEMA: &str = "http://www.w3.org/2001/XMLSchema";
/// The grammar type constant for DTD grammars.
pub const XML_DTD: &str = "http://www.w3.org/TR/REC-xml";
