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

//! Port of `XMLAttributes.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::qname::QName;

/// The XMLAttributes interface defines a collection of attributes for an
/// element.
///
/// Equivalent to `org.apache.xerces.xni.XMLAttributes`.
pub trait XMLAttributes {
    /// Equivalent to `addAttribute(QName, String, String)`.
    fn add_attribute(
        &mut self,
        attr_name: &QName,
        attr_type: &str,
        attr_value: Option<&str>,
    ) -> i32;
    /// Equivalent to `removeAllAttributes()`.
    fn remove_all_attributes(&mut self);
    /// Equivalent to `removeAttributeAt(int)`.
    fn remove_attribute_at(&mut self, attr_index: i32);
    /// Equivalent to `getLength()`.
    fn get_length(&self) -> i32;
    /// Equivalent to `getIndex(String)`.
    fn get_index(&self, qname: &str) -> i32;
    /// Equivalent to `getIndex(String, String)`.
    fn get_index_ns(&self, uri: Option<&str>, local_part: &str) -> i32;
    /// Equivalent to `setName(int, QName)`.
    fn set_name(&mut self, attr_index: i32, attr_name: &QName);
    /// Equivalent to `getName(int, QName)`.
    fn get_name(&self, attr_index: i32, attr_name: &mut QName);
    /// Equivalent to `getPrefix(int)`.
    fn get_prefix(&self, index: i32) -> Option<String>;
    /// Equivalent to `getURI(int)`.
    fn get_uri(&self, index: i32) -> Option<String>;
    /// Equivalent to `getLocalName(int)`.
    fn get_local_name(&self, index: i32) -> Option<String>;
    /// Equivalent to `getQName(int)`.
    fn get_qname(&self, index: i32) -> Option<String>;
    /// Equivalent to `setType(int, String)`.
    fn set_type(&mut self, attr_index: i32, attr_type: &str);
    /// Equivalent to `getType(int)`.
    fn get_type(&self, index: i32) -> Option<String>;
    /// Equivalent to `getType(String)`.
    fn get_type_by_qname(&self, qname: &str) -> Option<String>;
    /// Equivalent to `getType(String, String)`.
    fn get_type_ns(&self, uri: Option<&str>, local_name: &str) -> Option<String>;
    /// Equivalent to `setValue(int, String)`.
    fn set_value(&mut self, attr_index: i32, attr_value: Option<&str>);
    /// Equivalent to `getValue(int)`.
    fn get_value(&self, index: i32) -> Option<String>;
    /// Equivalent to `getValue(String)`.
    fn get_value_by_qname(&self, qname: &str) -> Option<String>;
    /// Equivalent to `getValue(String, String)`.
    fn get_value_ns(&self, uri: Option<&str>, local_name: &str) -> Option<String>;
    /// Equivalent to `setNonNormalizedValue(int, String)`.
    fn set_non_normalized_value(&mut self, attr_index: i32, attr_value: Option<&str>);
    /// Equivalent to `getNonNormalizedValue(int)`.
    fn get_non_normalized_value(&self, attr_index: i32) -> Option<String>;
    /// Equivalent to `setSpecified(int, boolean)`.
    fn set_specified(&mut self, attr_index: i32, specified: bool);
    /// Equivalent to `isSpecified(int)`.
    fn is_specified(&self, attr_index: i32) -> bool;
    /// Equivalent to `getAugmentations(int)`.
    fn get_augmentations(&self, attribute_index: i32) -> Option<Rc<dyn Augmentations>>;
    /// Equivalent to `getAugmentations(String, String)`.
    fn get_augmentations_ns(
        &self,
        uri: Option<&str>,
        local_part: &str,
    ) -> Option<Rc<dyn Augmentations>>;
    /// Equivalent to `getAugmentations(String)`.
    fn get_augmentations_by_qname(&self, qname: &str) -> Option<Rc<dyn Augmentations>>;
    /// Equivalent to `setAugmentations(int, Augmentations)`.
    fn set_augmentations(&mut self, attr_index: i32, augs: Rc<dyn Augmentations>);
}
