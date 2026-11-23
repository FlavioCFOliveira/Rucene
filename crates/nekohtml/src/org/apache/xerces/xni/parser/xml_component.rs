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

//! Port of `XMLComponent.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use crate::java::{JResult, Object};
use crate::org::apache::xerces::xni::parser::xml_component_manager::XMLComponentManager;

/// The component interface defines methods that must be implemented by
/// components in a parser configuration.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLComponent`; `throws
/// XMLConfigurationException` is the method's `Err`.
pub trait XMLComponent {
    /// Equivalent to `reset(XMLComponentManager)`.
    fn reset(&self, component_manager: &dyn XMLComponentManager) -> JResult<()>;
    /// Equivalent to `getRecognizedFeatures()`.
    fn get_recognized_features(&self) -> Option<Vec<String>>;
    /// Equivalent to `setFeature(String, boolean)`.
    fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()>;
    /// Equivalent to `getRecognizedProperties()`.
    fn get_recognized_properties(&self) -> Option<Vec<String>>;
    /// Equivalent to `setProperty(String, Object)`.
    fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()>;
    /// Equivalent to `getFeatureDefault(String)`.
    fn get_feature_default(&self, feature_id: &str) -> Option<bool>;
    /// Equivalent to `getPropertyDefault(String)`.
    fn get_property_default(&self, property_id: &str) -> Option<Object>;
}
