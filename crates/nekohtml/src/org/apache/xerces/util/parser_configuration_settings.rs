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

//! Port of `ParserConfigurationSettings.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::java::{JResult, Object};
use crate::org::apache::xerces::r#impl::constants;
use crate::org::apache::xerces::xni::parser::xml_component_manager::XMLComponentManager;
use crate::org::apache::xerces::xni::parser::xml_configuration_exception::{
    XMLConfigurationException, NOT_RECOGNIZED,
};

/// This class implements the basic operations for managing parser
/// configuration features and properties.
///
/// Equivalent to `org.apache.xerces.util.ParserConfigurationSettings`. A
/// Java subclass holds one as its `base`.
#[derive(Default)]
pub struct ParserConfigurationSettings {
    /// Recognized properties.
    pub recognized_properties: RefCell<Vec<String>>,
    /// Properties.
    pub properties: RefCell<HashMap<String, Option<Object>>>,
    /// Recognized features.
    pub recognized_features: RefCell<Vec<String>>,
    /// Features.
    pub features: RefCell<HashMap<String, bool>>,
    /// Parent parser configuration settings.
    pub parent_settings: Option<Rc<dyn XMLComponentManager>>,
}

/// Equivalent to `ParserConfigurationSettings.PARSER_SETTINGS`.
pub fn parser_settings() -> String {
    format!("{}{}", constants::XERCES_FEATURE_PREFIX, constants::PARSER_SETTINGS)
}

impl std::fmt::Debug for ParserConfigurationSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserConfigurationSettings")
            .field("recognized_features", &self.recognized_features.borrow())
            .field("features", &self.features.borrow())
            .field("recognized_properties", &self.recognized_properties.borrow())
            .finish_non_exhaustive()
    }
}

impl ParserConfigurationSettings {
    /// Default Constructor.
    ///
    /// Equivalent to `new ParserConfigurationSettings()`.
    pub fn new() -> Self {
        Self::with_parent(None)
    }

    /// Constructs a parser configuration settings object with a parent
    /// settings object.
    ///
    /// Equivalent to `new ParserConfigurationSettings(XMLComponentManager)`.
    pub fn with_parent(parent: Option<Rc<dyn XMLComponentManager>>) -> Self {
        Self {
            recognized_properties: RefCell::new(Vec::new()),
            properties: RefCell::new(HashMap::new()),
            recognized_features: RefCell::new(Vec::new()),
            features: RefCell::new(HashMap::new()),
            parent_settings: parent,
        }
    }

    /// Allows a parser to add parser specific features to be recognized and
    /// managed by the parser configuration.
    ///
    /// Equivalent to `addRecognizedFeatures(String[])`.
    pub fn add_recognized_features(&self, feature_ids: Option<&[String]>) {
        let mut recognized = self.recognized_features.borrow_mut();
        for feature_id in feature_ids.unwrap_or(&[]) {
            if !recognized.contains(feature_id) {
                recognized.push(feature_id.clone());
            }
        }
    }

    /// Set the state of a feature.
    ///
    /// Equivalent to `setFeature(String, boolean)`.
    ///
    /// # Errors
    ///
    /// `XMLConfigurationException` for an unrecognized feature.
    pub fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()> {
        self.check_feature(feature_id)?;
        self.features.borrow_mut().insert(feature_id.to_string(), state);
        Ok(())
    }

    /// Allows a parser to add parser specific properties to be recognized
    /// and managed by the parser configuration.
    ///
    /// Equivalent to `addRecognizedProperties(String[])`.
    pub fn add_recognized_properties(&self, property_ids: Option<&[String]>) {
        let mut recognized = self.recognized_properties.borrow_mut();
        for property_id in property_ids.unwrap_or(&[]) {
            if !recognized.contains(property_id) {
                recognized.push(property_id.clone());
            }
        }
    }

    /// Equivalent to `setProperty(String, Object)`.
    ///
    /// # Errors
    ///
    /// `XMLConfigurationException` for an unrecognized property.
    pub fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()> {
        self.check_property(property_id)?;
        self.properties.borrow_mut().insert(property_id.to_string(), value);
        Ok(())
    }

    /// Returns the state of a feature.
    ///
    /// Equivalent to `getFeature(String)`.
    ///
    /// # Errors
    ///
    /// `XMLConfigurationException` for an unrecognized feature.
    pub fn get_feature(&self, feature_id: &str) -> JResult<bool> {
        let state = self.features.borrow().get(feature_id).copied();
        match state {
            None => {
                self.check_feature(feature_id)?;
                Ok(false)
            }
            Some(state) => Ok(state),
        }
    }

    /// Returns the value of a property.
    ///
    /// Equivalent to `getProperty(String)`.
    ///
    /// # Errors
    ///
    /// `XMLConfigurationException` for an unrecognized property.
    pub fn get_property(&self, property_id: &str) -> JResult<Option<Object>> {
        let property_value = self.properties.borrow().get(property_id).cloned().flatten();
        if property_value.is_none() {
            self.check_property(property_id)?;
        }
        Ok(property_value)
    }

    /// Check a feature.
    ///
    /// Equivalent to the protected `checkFeature(String)`.
    ///
    /// # Errors
    ///
    /// `XMLConfigurationException(NOT_RECOGNIZED, featureId)`.
    pub fn check_feature(&self, feature_id: &str) -> JResult<()> {
        if !self.recognized_features.borrow().iter().any(|f| f == feature_id) {
            if let Some(parent) = &self.parent_settings {
                parent.get_feature(feature_id)?;
            } else {
                return Err(XMLConfigurationException::new(NOT_RECOGNIZED, feature_id).into());
            }
        }
        Ok(())
    }

    /// Check a property.
    ///
    /// Equivalent to the protected `checkProperty(String)`.
    ///
    /// # Errors
    ///
    /// `XMLConfigurationException(NOT_RECOGNIZED, propertyId)`.
    pub fn check_property(&self, property_id: &str) -> JResult<()> {
        if !self.recognized_properties.borrow().iter().any(|p| p == property_id) {
            if let Some(parent) = &self.parent_settings {
                parent.get_property(property_id)?;
            } else {
                return Err(XMLConfigurationException::new(NOT_RECOGNIZED, property_id).into());
            }
        }
        Ok(())
    }
}

impl XMLComponentManager for ParserConfigurationSettings {
    fn get_feature(&self, feature_id: &str) -> JResult<bool> {
        ParserConfigurationSettings::get_feature(self, feature_id)
    }

    fn get_property(&self, property_id: &str) -> JResult<Option<Object>> {
        ParserConfigurationSettings::get_property(self, property_id)
    }
}
