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

//! Port of `XMLResourceIdentifierImpl.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use crate::java::string_hash_code;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;

/// The XMLResourceIdentifierImpl class is an implementation of the
/// XMLResourceIdentifier interface which defines the location identity of
/// a resource.
///
/// Equivalent to `org.apache.xerces.util.XMLResourceIdentifierImpl`.
#[derive(Debug, Clone, Default)]
pub struct XMLResourceIdentifierImpl {
    /// The public identifier.
    pub public_id: Option<String>,
    /// The literal system identifier.
    pub literal_system_id: Option<String>,
    /// The base system identifier.
    pub base_system_id: Option<String>,
    /// The expanded system identifier.
    pub expanded_system_id: Option<String>,
    /// The namespace of the resource.
    pub namespace: Option<String>,
}

impl XMLResourceIdentifierImpl {
    /// Equivalent to `new XMLResourceIdentifierImpl()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new XMLResourceIdentifierImpl(String, String, String,
    /// String, String)` (and the four-argument form with `null`).
    pub fn with_values(
        public_id: Option<&str>,
        literal_system_id: Option<&str>,
        base_system_id: Option<&str>,
        expanded_system_id: Option<&str>,
        namespace: Option<&str>,
    ) -> Self {
        let mut s = Self::new();
        s.set_values(public_id, literal_system_id, base_system_id, expanded_system_id, namespace);
        s
    }

    /// Equivalent to `setValues(String, String, String, String, String)`
    /// (and the four-argument form with `null`).
    pub fn set_values(
        &mut self,
        public_id: Option<&str>,
        literal_system_id: Option<&str>,
        base_system_id: Option<&str>,
        expanded_system_id: Option<&str>,
        namespace: Option<&str>,
    ) {
        self.public_id = public_id.map(str::to_string);
        self.literal_system_id = literal_system_id.map(str::to_string);
        self.base_system_id = base_system_id.map(str::to_string);
        self.expanded_system_id = expanded_system_id.map(str::to_string);
        self.namespace = namespace.map(str::to_string);
    }

    /// Clears the values.
    ///
    /// Equivalent to `clear()`.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut code = 0i32;
        for v in [
            &self.public_id,
            &self.literal_system_id,
            &self.base_system_id,
            &self.expanded_system_id,
            &self.namespace,
        ]
        .into_iter()
        .flatten()
        {
            code = code.wrapping_add(string_hash_code(v));
        }
        code
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        let mut str = String::new();
        for (i, v) in [
            &self.public_id,
            &self.literal_system_id,
            &self.base_system_id,
            &self.expanded_system_id,
            &self.namespace,
        ]
        .into_iter()
        .enumerate()
        {
            if i > 0 {
                str.push(':');
            }
            if let Some(v) = v {
                str.push_str(v);
            }
        }
        str
    }
}

impl XMLResourceIdentifier for XMLResourceIdentifierImpl {
    fn set_public_id(&mut self, public_id: Option<&str>) {
        self.public_id = public_id.map(str::to_string);
    }

    fn get_public_id(&self) -> Option<String> {
        self.public_id.clone()
    }

    fn set_expanded_system_id(&mut self, system_id: Option<&str>) {
        self.expanded_system_id = system_id.map(str::to_string);
    }

    fn get_expanded_system_id(&self) -> Option<String> {
        self.expanded_system_id.clone()
    }

    fn set_literal_system_id(&mut self, system_id: Option<&str>) {
        self.literal_system_id = system_id.map(str::to_string);
    }

    fn get_literal_system_id(&self) -> Option<String> {
        self.literal_system_id.clone()
    }

    fn set_base_system_id(&mut self, system_id: Option<&str>) {
        self.base_system_id = system_id.map(str::to_string);
    }

    fn get_base_system_id(&self) -> Option<String> {
        self.base_system_id.clone()
    }

    fn set_namespace(&mut self, namespace: Option<&str>) {
        self.namespace = namespace.map(str::to_string);
    }

    fn get_namespace(&self) -> Option<String> {
        self.namespace.clone()
    }
}
