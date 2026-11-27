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

//! Port of `XMLDTDHandler.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::parser::xml_dtd_source::XMLDTDSource;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xml_string::XMLString;

/// The DTD handler interface defines callback methods to report information
/// items in the document type declaration.
///
/// Equivalent to `org.apache.xerces.xni.XMLDTDHandler`.
pub trait XMLDTDHandler {
    /// Equivalent to `startDTD(XMLLocator, Augmentations)`.
    fn start_dtd(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `startParameterEntity(String, XMLResourceIdentifier,
    /// String, Augmentations)`.
    fn start_parameter_entity(
        &self,
        name: &str,
        identifier: Option<&dyn XMLResourceIdentifier>,
        encoding: Option<&str>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `textDecl(String, String, Augmentations)`.
    fn text_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `endParameterEntity(String, Augmentations)`.
    fn end_parameter_entity(
        &self,
        name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `startExternalSubset(XMLResourceIdentifier,
    /// Augmentations)`.
    fn start_external_subset(
        &self,
        identifier: Option<&dyn XMLResourceIdentifier>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `endExternalSubset(Augmentations)`.
    fn end_external_subset(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `comment(XMLString, Augmentations)`.
    fn comment(&self, text: &XMLString, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `processingInstruction(String, XMLString,
    /// Augmentations)`.
    fn processing_instruction(
        &self,
        target: &str,
        data: Option<&XMLString>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `elementDecl(String, String, Augmentations)`.
    fn element_decl(
        &self,
        name: &str,
        content_model: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `startAttlist(String, Augmentations)`.
    fn start_attlist(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `attributeDecl(String, String, String, String[],
    /// String, XMLString, XMLString, Augmentations)`.
    #[allow(clippy::too_many_arguments)]
    fn attribute_decl(
        &self,
        element_name: &str,
        attribute_name: &str,
        type_: &str,
        enumeration: Option<&[String]>,
        default_type: Option<&str>,
        default_value: Option<&XMLString>,
        non_normalized_default_value: Option<&XMLString>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `endAttlist(Augmentations)`.
    fn end_attlist(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `internalEntityDecl(String, XMLString, XMLString,
    /// Augmentations)`.
    fn internal_entity_decl(
        &self,
        name: &str,
        text: &XMLString,
        non_normalized_text: &XMLString,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `externalEntityDecl(String, XMLResourceIdentifier,
    /// Augmentations)`.
    fn external_entity_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `unparsedEntityDecl(String, XMLResourceIdentifier,
    /// String, Augmentations)`.
    fn unparsed_entity_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        notation: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `notationDecl(String, XMLResourceIdentifier,
    /// Augmentations)`.
    fn notation_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `startConditional(short, Augmentations)`.
    fn start_conditional(
        &self,
        type_: i16,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `ignoredCharacters(XMLString, Augmentations)`.
    fn ignored_characters(
        &self,
        text: &XMLString,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()>;
    /// Equivalent to `endConditional(Augmentations)`.
    fn end_conditional(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `endDTD(Augmentations)`.
    fn end_dtd(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()>;
    /// Equivalent to `setDTDSource(XMLDTDSource)`.
    fn set_dtd_source(&self, source: Option<Rc<dyn XMLDTDSource>>);
    /// Equivalent to `getDTDSource()`.
    fn get_dtd_source(&self) -> Option<Rc<dyn XMLDTDSource>>;
}

/// Conditional section: INCLUDE.
pub const CONDITIONAL_INCLUDE: i16 = 0;
/// Conditional section: IGNORE.
pub const CONDITIONAL_IGNORE: i16 = 1;
