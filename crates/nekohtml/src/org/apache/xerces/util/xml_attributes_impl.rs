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

//! Port of `XMLAttributesImpl.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::util::augmentations_impl::AugmentationsImpl;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;

/// The XMLAttributesImpl class is an implementation of the XMLAttributes
/// interface which defines a collection of attributes for an element.
///
/// Equivalent to `org.apache.xerces.util.XMLAttributesImpl`. Java keeps,
/// past `SIZE_LIMIT` attributes, a hash "table view" over the attributes to
/// find a duplicate name in constant time; the port finds the same index by
/// a linear search (the attribute chosen, and the duplicate
/// `checkDuplicatesNS` reports, are the same: the table view only changes
/// the cost). The `Attribute` objects are reused across
/// `removeAllAttributes`, as Java reuses them.
#[derive(Debug)]
pub struct XMLAttributesImpl {
    /// Namespaces.
    namespaces: bool,
    /// Attribute count.
    length: i32,
    /// Attribute information.
    attributes: Vec<Attribute>,
}

/// Default table size.
pub const TABLE_SIZE: i32 = 101;
/// Maximum hash collisions per bucket.
pub const MAX_HASH_COLLISIONS: i32 = 40;
/// `MULTIPLIERS_SIZE`.
pub const MULTIPLIERS_SIZE: i32 = 1 << 5;
/// `MULTIPLIERS_MASK`.
pub const MULTIPLIERS_MASK: i32 = MULTIPLIERS_SIZE - 1;
/// Threshold at which an instance is treated as a large attribute list.
pub const SIZE_LIMIT: i32 = 20;

/// Attribute information.
///
/// Equivalent to the nested class `XMLAttributesImpl.Attribute`.
struct Attribute {
    /// Name.
    name: QName,
    /// Type.
    type_: Option<String>,
    /// Value.
    value: Option<String>,
    /// Non-normalized value.
    non_normalized_value: Option<String>,
    /// Specified.
    specified: bool,
    /// Augmentations information for this attribute.
    augs: Rc<dyn Augmentations>,
}

impl std::fmt::Debug for Attribute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attribute")
            .field("name", &self.name)
            .field("type", &self.type_)
            .field("value", &self.value)
            .field("specified", &self.specified)
            .finish_non_exhaustive()
    }
}

impl Attribute {
    fn new() -> Self {
        Self {
            name: QName::new(),
            type_: None,
            value: None,
            non_normalized_value: None,
            specified: false,
            augs: Rc::new(AugmentationsImpl::new()),
        }
    }
}

impl Default for XMLAttributesImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl XMLAttributesImpl {
    /// Default constructor.
    ///
    /// Equivalent to `new XMLAttributesImpl()`.
    pub fn new() -> Self {
        Self::with_table_size(TABLE_SIZE)
    }

    /// Equivalent to `new XMLAttributesImpl(int)` (the table size only
    /// sizes Java's table view).
    pub fn with_table_size(_table_size: i32) -> Self {
        Self {
            namespaces: true,
            length: 0,
            attributes: (0..4).map(|_| Attribute::new()).collect(),
        }
    }

    /// Sets whether namespace processing is being performed.
    ///
    /// Equivalent to `setNamespaces(boolean)`.
    pub fn set_namespaces(&mut self, namespaces: bool) {
        self.namespaces = namespaces;
    }

    fn grow(&mut self, by_doubling: bool) {
        let len = self.attributes.len();
        let new_len = if by_doubling { len << 1 } else { len + 4 };
        self.attributes.extend((len..new_len).map(|_| Attribute::new()));
    }

    fn append_index(&mut self, by_doubling: bool) -> i32 {
        let index = self.length;
        let was = self.length as usize;
        self.length += 1;
        if was == self.attributes.len() {
            self.grow(by_doubling);
        }
        index
    }

    /// Returns the full name of the attribute at `index`.
    ///
    /// Equivalent to `getName(int)`.
    pub fn get_raw_name(&self, index: i32) -> Option<String> {
        if index < 0 || index >= self.length {
            return None;
        }
        self.attributes[index as usize].name.rawname.clone()
    }

    /// Sets the uri of the attribute at the specified index.
    ///
    /// Equivalent to `setURI(int, String)`.
    pub fn set_uri(&mut self, attr_index: i32, uri: Option<&str>) {
        self.attributes[attr_index as usize].name.uri = uri.map(str::to_string);
    }

    /// Look up the index of an attribute by XML 1.0 qualified name.
    ///
    /// Equivalent to `getIndexFast(String)`.
    pub fn get_index_fast(&self, qname: Option<&str>) -> i32 {
        for i in 0..self.length {
            if self.attributes[i as usize].name.rawname.as_deref() == qname {
                return i;
            }
        }
        -1
    }

    /// Look up the index of an attribute by Namespace name.
    ///
    /// Equivalent to `getIndexFast(String, String)`.
    pub fn get_index_fast_ns(&self, uri: Option<&str>, local_part: Option<&str>) -> i32 {
        for i in 0..self.length {
            let a = &self.attributes[i as usize];
            if a.name.localpart.as_deref() == local_part && a.name.uri.as_deref() == uri {
                return i;
            }
        }
        -1
    }

    /// Adds an attribute. The attribute's non-normalized value of the
    /// attribute will have the same value as the attribute value until set
    /// using the `setNonNormalizedValue` method. Also, the added attribute
    /// will be marked as specified in the XML instance document unless set
    /// otherwise using the `setSpecified` method.
    ///
    /// This method differs from `addAttribute` in that it does not check if
    /// an attribute of the same name already exists in the list before
    /// adding it.
    ///
    /// Equivalent to `addAttributeNS(QName, String, String)`.
    pub fn add_attribute_ns(&mut self, name: &QName, type_: &str, value: Option<&str>) {
        let index = self.length;
        let was = self.length as usize;
        self.length += 1;
        if was == self.attributes.len() {
            let by_doubling = self.length >= SIZE_LIMIT;
            self.grow(by_doubling);
        }
        self.reset_attribute(index, name, type_, value);
    }

    fn reset_attribute(&mut self, index: i32, name: &QName, type_: &str, value: Option<&str>) {
        let attribute = &mut self.attributes[index as usize];
        attribute.name.set_values_from(name);
        attribute.type_ = Some(type_.to_string());
        attribute.value = value.map(str::to_string);
        attribute.non_normalized_value = value.map(str::to_string);
        attribute.specified = false;
        attribute.augs.remove_all_items();
    }

    /// Checks for duplicate expanded names (local part and namespace name
    /// pairs) in the attribute specification. If a duplicate is found its
    /// name is returned.
    ///
    /// Equivalent to `checkDuplicatesNS()`.
    pub fn check_duplicates_ns(&self) -> Option<QName> {
        let length = self.length as usize;
        let same = |a: &Attribute, b: &Attribute| a.name.localpart == b.name.localpart && a.name.uri == b.name.uri;
        if self.length <= SIZE_LIMIT {
            for i in 0..length.saturating_sub(1) {
                for j in i + 1..length {
                    if same(&self.attributes[i], &self.attributes[j]) {
                        return Some(self.attributes[j].name.clone());
                    }
                }
            }
            None
        } else {
            for i in 0..length {
                for k in 0..i {
                    if same(&self.attributes[k], &self.attributes[i]) {
                        return Some(self.attributes[i].name.clone());
                    }
                }
            }
            None
        }
    }

    /// Equivalent to the private `getReportableType(String)`.
    fn get_reportable_type(type_: Option<&str>) -> Option<String> {
        let type_ = type_?;
        if type_.starts_with('(') {
            return Some("NMTOKEN".to_string());
        }
        Some(type_.to_string())
    }
}

impl XMLAttributes for XMLAttributesImpl {
    fn add_attribute(&mut self, name: &QName, type_: &str, value: Option<&str>) -> i32 {
        let has_uri = name.uri.as_deref().is_some_and(|u| !u.is_empty());
        let index;
        if self.length < SIZE_LIMIT {
            let found = if has_uri {
                self.get_index_fast_ns(name.uri.as_deref(), name.localpart.as_deref())
            } else {
                self.get_index_fast(name.rawname.as_deref())
            };
            index = if found == -1 { self.append_index(false) } else { found };
        } else {
            let found_ns = if has_uri {
                self.get_index_fast_ns(name.uri.as_deref(), name.localpart.as_deref())
            } else {
                -1
            };
            if found_ns != -1 {
                index = found_ns;
            } else {
                let found = self.get_index_fast(name.rawname.as_deref());
                index = if found == -1 { self.append_index(true) } else { found };
            }
        }
        self.reset_attribute(index, name, type_, value);
        index
    }

    fn remove_all_attributes(&mut self) {
        self.length = 0;
    }

    fn remove_attribute_at(&mut self, attr_index: i32) {
        if attr_index < self.length - 1 {
            let removed = self.attributes.remove(attr_index as usize);
            self.attributes.insert((self.length - 1) as usize, removed);
        }
        self.length -= 1;
    }

    fn get_length(&self) -> i32 {
        self.length
    }

    fn get_index(&self, qname: &str) -> i32 {
        for i in 0..self.length {
            if self.attributes[i as usize].name.rawname.as_deref() == Some(qname) {
                return i;
            }
        }
        -1
    }

    fn get_index_ns(&self, uri: Option<&str>, local_part: &str) -> i32 {
        for i in 0..self.length {
            let a = &self.attributes[i as usize];
            if a.name.localpart.as_deref() == Some(local_part) && a.name.uri.as_deref() == uri {
                return i;
            }
        }
        -1
    }

    fn set_name(&mut self, attr_index: i32, attr_name: &QName) {
        self.attributes[attr_index as usize].name.set_values_from(attr_name);
    }

    fn get_name(&self, attr_index: i32, attr_name: &mut QName) {
        attr_name.set_values_from(&self.attributes[attr_index as usize].name);
    }

    fn get_prefix(&self, index: i32) -> Option<String> {
        if index < 0 || index >= self.length {
            return None;
        }
        Some(self.attributes[index as usize].name.prefix.clone().unwrap_or_default())
    }

    fn get_uri(&self, index: i32) -> Option<String> {
        if index < 0 || index >= self.length {
            return None;
        }
        self.attributes[index as usize].name.uri.clone()
    }

    fn get_local_name(&self, index: i32) -> Option<String> {
        if !self.namespaces {
            return Some(String::new());
        }
        if index < 0 || index >= self.length {
            return None;
        }
        self.attributes[index as usize].name.localpart.clone()
    }

    fn get_qname(&self, index: i32) -> Option<String> {
        if index < 0 || index >= self.length {
            return None;
        }
        Some(self.attributes[index as usize].name.rawname.clone().unwrap_or_default())
    }

    fn set_type(&mut self, attr_index: i32, attr_type: &str) {
        self.attributes[attr_index as usize].type_ = Some(attr_type.to_string());
    }

    fn get_type(&self, index: i32) -> Option<String> {
        if index < 0 || index >= self.length {
            return None;
        }
        Self::get_reportable_type(self.attributes[index as usize].type_.as_deref())
    }

    fn get_type_by_qname(&self, qname: &str) -> Option<String> {
        let index = self.get_index(qname);
        if index != -1 {
            Self::get_reportable_type(self.attributes[index as usize].type_.as_deref())
        } else {
            None
        }
    }

    fn get_type_ns(&self, uri: Option<&str>, local_name: &str) -> Option<String> {
        if !self.namespaces {
            return None;
        }
        let index = self.get_index_ns(uri, local_name);
        if index != -1 {
            Self::get_reportable_type(self.attributes[index as usize].type_.as_deref())
        } else {
            None
        }
    }

    fn set_value(&mut self, attr_index: i32, attr_value: Option<&str>) {
        let attribute = &mut self.attributes[attr_index as usize];
        attribute.value = attr_value.map(str::to_string);
        attribute.non_normalized_value = attr_value.map(str::to_string);
    }

    fn get_value(&self, index: i32) -> Option<String> {
        if index < 0 || index >= self.length {
            return None;
        }
        self.attributes[index as usize].value.clone()
    }

    fn get_value_by_qname(&self, qname: &str) -> Option<String> {
        let index = self.get_index(qname);
        if index != -1 {
            self.attributes[index as usize].value.clone()
        } else {
            None
        }
    }

    fn get_value_ns(&self, uri: Option<&str>, local_name: &str) -> Option<String> {
        let index = self.get_index_ns(uri, local_name);
        if index != -1 {
            self.get_value(index)
        } else {
            None
        }
    }

    fn set_non_normalized_value(&mut self, attr_index: i32, attr_value: Option<&str>) {
        let attribute = &mut self.attributes[attr_index as usize];
        attribute.non_normalized_value = match attr_value {
            Some(v) => Some(v.to_string()),
            None => attribute.value.clone(),
        };
    }

    fn get_non_normalized_value(&self, attr_index: i32) -> Option<String> {
        self.attributes[attr_index as usize].non_normalized_value.clone()
    }

    fn set_specified(&mut self, attr_index: i32, specified: bool) {
        self.attributes[attr_index as usize].specified = specified;
    }

    fn is_specified(&self, attr_index: i32) -> bool {
        self.attributes[attr_index as usize].specified
    }

    fn get_augmentations(&self, attribute_index: i32) -> Option<Rc<dyn Augmentations>> {
        if attribute_index < 0 || attribute_index >= self.length {
            return None;
        }
        Some(Rc::clone(&self.attributes[attribute_index as usize].augs))
    }

    fn get_augmentations_ns(&self, uri: Option<&str>, local_part: &str) -> Option<Rc<dyn Augmentations>> {
        let index = self.get_index_ns(uri, local_part);
        if index != -1 {
            Some(Rc::clone(&self.attributes[index as usize].augs))
        } else {
            None
        }
    }

    fn get_augmentations_by_qname(&self, qname: &str) -> Option<Rc<dyn Augmentations>> {
        let index = self.get_index(qname);
        if index != -1 {
            Some(Rc::clone(&self.attributes[index as usize].augs))
        } else {
            None
        }
    }

    fn set_augmentations(&mut self, attr_index: i32, augs: Rc<dyn Augmentations>) {
        self.attributes[attr_index as usize].augs = augs;
    }
}
