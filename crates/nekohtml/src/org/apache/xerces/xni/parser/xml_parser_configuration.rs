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

//! Port of `XMLParserConfiguration.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use std::rc::Rc;

use rucene_queryparser::jdk::locale::Locale;

use crate::java::{JResult, Object};
use crate::org::apache::xerces::xni::parser::xml_component_manager::XMLComponentManager;
use crate::org::apache::xerces::xni::parser::xml_entity_resolver::XMLEntityResolver;
use crate::org::apache::xerces::xni::parser::xml_error_handler::XMLErrorHandler;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;
use crate::org::apache::xerces::xni::parser::xml_pull_parser_configuration::XMLPullParserConfiguration;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_dtd_content_model_handler::XMLDTDContentModelHandler;
use crate::org::apache::xerces::xni::xml_dtd_handler::XMLDTDHandler;

/// Represents a parser configuration.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLParserConfiguration`.
pub trait XMLParserConfiguration: XMLComponentManager {
    /// Equivalent to `parse(XMLInputSource)`.
    fn parse(&self, input_source: &XMLInputSource) -> JResult<()>;
    /// Equivalent to `addRecognizedFeatures(String[])`.
    fn add_recognized_features(&self, feature_ids: Option<&[String]>);
    /// Equivalent to `setFeature(String, boolean)`.
    fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()>;
    /// Equivalent to `addRecognizedProperties(String[])`.
    fn add_recognized_properties(&self, property_ids: Option<&[String]>);
    /// Equivalent to `setProperty(String, Object)`.
    fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()>;
    /// Equivalent to `setErrorHandler(XMLErrorHandler)`.
    fn set_error_handler(&self, error_handler: Option<Rc<dyn XMLErrorHandler>>);
    /// Equivalent to `getErrorHandler()`.
    fn get_error_handler(&self) -> Option<Rc<dyn XMLErrorHandler>>;
    /// Equivalent to `setDocumentHandler(XMLDocumentHandler)`.
    fn set_document_handler(&self, document_handler: Option<Rc<dyn XMLDocumentHandler>>);
    /// Equivalent to `getDocumentHandler()`.
    fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>>;
    /// Equivalent to `setDTDHandler(XMLDTDHandler)`.
    fn set_dtd_handler(&self, dtd_handler: Option<Rc<dyn XMLDTDHandler>>);
    /// Equivalent to `getDTDHandler()`.
    fn get_dtd_handler(&self) -> Option<Rc<dyn XMLDTDHandler>>;
    /// Equivalent to `setDTDContentModelHandler(XMLDTDContentModelHandler)`.
    fn set_dtd_content_model_handler(&self, dtd_content_model_handler: Option<Rc<dyn XMLDTDContentModelHandler>>);
    /// Equivalent to `getDTDContentModelHandler()`.
    fn get_dtd_content_model_handler(&self) -> Option<Rc<dyn XMLDTDContentModelHandler>>;
    /// Equivalent to `setEntityResolver(XMLEntityResolver)`.
    fn set_entity_resolver(&self, entity_resolver: Option<Rc<dyn XMLEntityResolver>>);
    /// Equivalent to `getEntityResolver()`.
    fn get_entity_resolver(&self) -> Option<Rc<dyn XMLEntityResolver>>;
    /// Equivalent to `setLocale(Locale)`.
    fn set_locale(&self, locale: Option<Locale>) -> JResult<()>;
    /// Equivalent to `getLocale()`.
    fn get_locale(&self) -> Locale;

    /// `this instanceof XMLPullParserConfiguration ?
    /// (XMLPullParserConfiguration) this : null`.
    fn as_pull_parser_configuration(&self) -> Option<&dyn XMLPullParserConfiguration> {
        None
    }

    /// `this instanceof XML11Configurable`.
    fn is_xml11_configurable(&self) -> bool {
        false
    }
}
