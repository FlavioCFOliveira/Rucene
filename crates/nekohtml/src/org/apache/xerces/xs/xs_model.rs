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

//! Port of `XSModel.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::string_list::StringList;
use crate::org::apache::xerces::xs::xs_attribute_declaration::XSAttributeDeclaration;
use crate::org::apache::xerces::xs::xs_attribute_group_definition::XSAttributeGroupDefinition;
use crate::org::apache::xerces::xs::xs_element_declaration::XSElementDeclaration;
use crate::org::apache::xerces::xs::xs_idc_definition::XSIDCDefinition;
use crate::org::apache::xerces::xs::xs_model_group_definition::XSModelGroupDefinition;
use crate::org::apache::xerces::xs::xs_named_map::XSNamedMap;
use crate::org::apache::xerces::xs::xs_namespace_item_list::XSNamespaceItemList;
use crate::org::apache::xerces::xs::xs_notation_declaration::XSNotationDeclaration;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_type_definition::XSTypeDefinition;

/// Equivalent to `org.apache.xerces.xs.XSModel`.
pub trait XSModel {
    /// Equivalent to `getNamespaces()`.
    fn get_namespaces(&self) -> Option<Rc<dyn StringList>>;

    /// Equivalent to `getNamespaceItems()`.
    fn get_namespace_items(&self) -> Option<Rc<dyn XSNamespaceItemList>>;

    /// Equivalent to `getComponents(short)`.
    fn get_components(&self, object_type: i16) -> Option<Rc<dyn XSNamedMap>>;

    /// Equivalent to `getComponentsByNamespace(short, String)`.
    fn get_components_by_namespace(
        &self,
        object_type: i16,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSNamedMap>>;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;

    /// Equivalent to `getElementDeclaration(String, String)`.
    fn get_element_declaration(
        &self,
        name: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSElementDeclaration>>;

    /// Equivalent to `getAttributeDeclaration(String, String)`.
    fn get_attribute_declaration(
        &self,
        name: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSAttributeDeclaration>>;

    /// Equivalent to `getTypeDefinition(String, String)`.
    fn get_type_definition(
        &self,
        name: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSTypeDefinition>>;

    /// Equivalent to `getAttributeGroup(String, String)`.
    fn get_attribute_group(
        &self,
        name: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSAttributeGroupDefinition>>;

    /// Equivalent to `getModelGroupDefinition(String, String)`.
    fn get_model_group_definition(
        &self,
        name: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSModelGroupDefinition>>;

    /// Equivalent to `getNotationDeclaration(String, String)`.
    fn get_notation_declaration(
        &self,
        name: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSNotationDeclaration>>;

    /// Equivalent to `getIDCDefinition(String, String)`.
    fn get_idc_definition(
        &self,
        name: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<Rc<dyn XSIDCDefinition>>;

    /// Equivalent to `getSubstitutionGroup(XSElementDeclaration)`.
    fn get_substitution_group(
        &self,
        head: Option<&dyn XSElementDeclaration>,
    ) -> Option<Rc<dyn XSObjectList>>;
}
