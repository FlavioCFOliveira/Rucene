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

//! Port of `XMLResourceIdentifier.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

/// This represents the basic physical description of the location of any
/// XML resource (a Schema grammar, a DTD, a general entity etc.)
///
/// Equivalent to `org.apache.xerces.xni.XMLResourceIdentifier`.
pub trait XMLResourceIdentifier {
    /// `this instanceof XMLDTDDescription`.
    fn is_dtd_description(&self) -> bool {
        false
    }

    /// `this instanceof XMLEntityDescription ? ((XMLEntityDescription)
    /// this).getEntityName() : null`, and whether it is one.
    fn as_entity_description(
        &self,
    ) -> Option<&dyn crate::org::apache::xerces::r#impl::xml_entity_description::XMLEntityDescription>
    {
        None
    }

    /// Equivalent to `setPublicId(String)`.
    fn set_public_id(&mut self, public_id: Option<&str>);
    /// Equivalent to `getPublicId()`.
    fn get_public_id(&self) -> Option<String>;
    /// Equivalent to `setExpandedSystemId(String)`.
    fn set_expanded_system_id(&mut self, system_id: Option<&str>);
    /// Equivalent to `getExpandedSystemId()`.
    fn get_expanded_system_id(&self) -> Option<String>;
    /// Equivalent to `setLiteralSystemId(String)`.
    fn set_literal_system_id(&mut self, system_id: Option<&str>);
    /// Equivalent to `getLiteralSystemId()`.
    fn get_literal_system_id(&self) -> Option<String>;
    /// Equivalent to `setBaseSystemId(String)`.
    fn set_base_system_id(&mut self, system_id: Option<&str>);
    /// Equivalent to `getBaseSystemId()`.
    fn get_base_system_id(&self) -> Option<String>;
    /// Equivalent to `setNamespace(String)`.
    fn set_namespace(&mut self, namespace: Option<&str>);
    /// Equivalent to `getNamespace()`.
    fn get_namespace(&self) -> Option<String>;
}
