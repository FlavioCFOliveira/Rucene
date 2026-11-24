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

//! Port of `XMLComponentManager.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use crate::java::{JResult, Object};

/// The component manager manages a parser configuration and the components
/// that make up that configuration.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLComponentManager`.
pub trait XMLComponentManager {
    /// Equivalent to `getFeature(String)`.
    fn get_feature(&self, feature_id: &str) -> JResult<bool>;
    /// Equivalent to `getProperty(String)`.
    fn get_property(&self, property_id: &str) -> JResult<Option<Object>>;
}
