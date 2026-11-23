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

use crate::java::string_hash_code;
use crate::org::apache::xerces::util::augmentations_impl::AugmentationsImpl;
use crate::org::apache::xerces::util::prime_number_sequence_generator::PrimeNumberSequenceGenerator;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;

/// The XMLAttributesImpl class is an implementation of the XMLAttributes
/// interface which defines a collection of attributes for an element.
///
/// Equivalent to `org.apache.xerces.util.XMLAttributesImpl`. Past
/// `SIZE_LIMIT` attributes it keeps Java's hash "table view" over the
/// attributes. An `Attribute` is referred to by its index in `attributes`
/// (the table view's entries and each attribute's `next`): Java's
/// references, since an attribute object only changes position in
/// `removeAttributeAt`, which marks the table view inconsistent as Java
/// does. The `Attribute` objects are reused across `removeAllAttributes`,
/// as Java reuses them.
#[derive(Debug)]
pub struct XMLAttributesImpl {
    /// Namespaces.
    namespaces: bool,
    /// Usage count for the attribute table view. Incremented each time all
    /// attributes are removed.
    large_count: i32,
    /// Attribute count.
    length: i32,
    /// Attribute information.
    attributes: Vec<Attribute>,
    /// Provides an alternate view of the attribute specification: the head
    /// of each bucket's chain.
    attribute_table_view: Option<Vec<Option<usize>>>,
    /// Tracks whether each chain in the hash table is stale with respect to
    /// the current state of this object. A chain is stale if its state is
    /// not the same as the number of times the attribute table view has
    /// been used.
    attribute_table_view_chain_state: Option<Vec<i32>>,
    /// Actual number of buckets in the table view.
    table_view_buckets: i32,
    /// Indicates whether the table view contains consistent data.
    is_table_view_consistent: bool,
    /// Array of randomly selected hash function multipliers or `None` if
    /// the default `String.hashCode()` function should be used.
    hash_multipliers: Option<Vec<i32>>,
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
    /// Pointer to the next attribute in the chain (an index into
    /// `XMLAttributesImpl::attributes`).
    next: Option<usize>,
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
            next: None,
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

    /// Equivalent to `new XMLAttributesImpl(int)`.
    pub fn with_table_size(table_size: i32) -> Self {
        Self {
            namespaces: true,
            large_count: 1,
            length: 0,
            attributes: (0..4).map(|_| Attribute::new()).collect(),
            attribute_table_view: None,
            attribute_table_view_chain_state: None,
            table_view_buckets: table_size,
            is_table_view_consistent: false,
            hash_multipliers: None,
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
        self.attributes
            .extend((len..new_len).map(|_| Attribute::new()));
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
    pub fn check_duplicates_ns(&mut self) -> Option<QName> {
        let length = self.length as usize;
        let same = |a: &Attribute, b: &Attribute| {
            a.name.localpart == b.name.localpart && a.name.uri == b.name.uri
        };
        // If the list is small check for duplicates using pairwise comparison.
        if self.length <= SIZE_LIMIT {
            for i in 0..length.saturating_sub(1) {
                for j in i + 1..length {
                    if same(&self.attributes[i], &self.attributes[j]) {
                        return Some(self.attributes[j].name.clone());
                    }
                }
            }
            None
        }
        // If the list is large check duplicates using a hash table.
        else {
            self.check_many_duplicates_ns()
        }
    }

    /// Equivalent to the private `checkManyDuplicatesNS()`.
    fn check_many_duplicates_ns(&mut self) -> Option<QName> {
        // We don't want this table view to be read if someone calls
        // addAttribute so we invalidate it up front.
        self.is_table_view_consistent = false;

        self.prepare_table_view();

        let length = self.length as usize;
        for i in 0..length {
            let bucket = {
                let name = &self.attributes[i].name;
                self.get_table_view_bucket_ns(name.localpart.as_deref(), name.uri.as_deref())
            };

            // The chain is stale.
            // This must be a unique attribute.
            if self.chain_state()[bucket] != self.large_count {
                let large_count = self.large_count;
                self.chain_state_mut()[bucket] = large_count;
                self.attributes[i].next = None;
                self.table_view_mut()[bucket] = Some(i);
            }
            // This chain is active.
            // We need to check if any of the attributes has the same name.
            else {
                // Search the table.
                let mut collision_count = 0;
                let mut found = self.table_view()[bucket];
                while let Some(f) = found {
                    let (a, b) = (&self.attributes[f].name, &self.attributes[i].name);
                    if a.localpart == b.localpart && a.uri == b.uri {
                        return Some(self.attributes[i].name.clone());
                    }
                    found = self.attributes[f].next;
                    collision_count += 1;
                }

                // Select a new hash function and rehash the table view
                // if the collision threshold is exceeded.
                if collision_count >= MAX_HASH_COLLISIONS {
                    // The current attribute will be processed in the rehash.
                    self.rebalance_table_view_ns(i as i32 + 1);
                } else {
                    // Update table view
                    self.attributes[i].next = self.table_view()[bucket];
                    self.table_view_mut()[bucket] = Some(i);
                }
            }
        }
        None
    }

    fn table_view(&self) -> &[Option<usize>] {
        self.attribute_table_view
            .as_deref()
            .expect("prepareTableView allocated the table view")
    }

    fn table_view_mut(&mut self) -> &mut [Option<usize>] {
        self.attribute_table_view
            .as_deref_mut()
            .expect("prepareTableView allocated the table view")
    }

    fn chain_state(&self) -> &[i32] {
        self.attribute_table_view_chain_state
            .as_deref()
            .expect("prepareTableView allocated the chain state")
    }

    fn chain_state_mut(&mut self) -> &mut [i32] {
        self.attribute_table_view_chain_state
            .as_deref_mut()
            .expect("prepareTableView allocated the chain state")
    }

    /// Returns the position in the table view where the given attribute
    /// name would be hashed.
    ///
    /// Equivalent to the protected `getTableViewBucket(String)`.
    fn get_table_view_bucket(&self, qname: Option<&str>) -> usize {
        ((self.hash(qname) & 0x7FFF_FFFF) % self.table_view_buckets) as usize
    }

    /// Returns the position in the table view where the given attribute
    /// name would be hashed.
    ///
    /// Equivalent to the protected `getTableViewBucket(String, String)`.
    fn get_table_view_bucket_ns(&self, localpart: Option<&str>, uri: Option<&str>) -> usize {
        match uri {
            None => ((self.hash(localpart) & 0x7FFF_FFFF) % self.table_view_buckets) as usize,
            Some(uri) => {
                ((self.hash_ns(localpart, uri) & 0x7FFF_FFFF) % self.table_view_buckets) as usize
            }
        }
    }

    /// Equivalent to the private `hash(String)`.
    fn hash(&self, localpart: Option<&str>) -> i32 {
        let localpart = localpart.expect("java.lang.NullPointerException");
        if self.hash_multipliers.is_none() {
            return string_hash_code(localpart);
        }
        self.hash0(localpart)
    }

    /// Equivalent to the private `hash(String, String)`.
    fn hash_ns(&self, localpart: Option<&str>, uri: &str) -> i32 {
        let localpart = localpart.expect("java.lang.NullPointerException");
        match &self.hash_multipliers {
            None => {
                string_hash_code(localpart).wrapping_add(string_hash_code(uri).wrapping_mul(31))
            }
            Some(multipliers) => self.hash0(localpart).wrapping_add(
                self.hash0(uri)
                    .wrapping_mul(multipliers[MULTIPLIERS_SIZE as usize]),
            ),
        }
    }

    /// Equivalent to the private `hash0(String)`.
    fn hash0(&self, symbol: &str) -> i32 {
        let mut code = 0i32;
        let multipliers = self
            .hash_multipliers
            .as_deref()
            .expect("hash0 runs once a rebalance set the multipliers");
        for (i, c) in symbol.encode_utf16().enumerate() {
            code = code
                .wrapping_mul(multipliers[i & MULTIPLIERS_MASK as usize])
                .wrapping_add(i32::from(c));
        }
        code
    }

    /// Purges all elements from the table view.
    ///
    /// Equivalent to the protected `cleanTableView()`.
    fn clean_table_view(&mut self) {
        self.large_count = self.large_count.wrapping_add(1);
        if self.large_count < 0 {
            // Overflow. We actually need to visit the chain state array.
            let buckets = self.table_view_buckets;
            if let Some(chain_state) = self.attribute_table_view_chain_state.as_deref_mut() {
                for i in (0..buckets).rev() {
                    chain_state[i as usize] = 0;
                }
            }
            self.large_count = 1;
        }
    }

    /// Increases the capacity of the table view.
    ///
    /// Equivalent to the private `growTableView()`.
    fn grow_table_view(&mut self) {
        let length = self.length;
        let mut table_view_buckets = self.table_view_buckets;
        loop {
            table_view_buckets = (table_view_buckets << 1).wrapping_add(1);
            if table_view_buckets < 0 {
                table_view_buckets = i32::MAX;
                break;
            }
            if length <= table_view_buckets {
                break;
            }
        }
        self.table_view_buckets = table_view_buckets;
        self.attribute_table_view = None;
        self.large_count = 1;
    }

    /// Prepares the table view for use.
    ///
    /// Equivalent to the protected `prepareTableView()`.
    fn prepare_table_view(&mut self) {
        if self.length > self.table_view_buckets {
            self.grow_table_view();
        }
        if self.attribute_table_view.is_none() {
            self.attribute_table_view = Some(vec![None; self.table_view_buckets as usize]);
            self.attribute_table_view_chain_state = Some(vec![0; self.table_view_buckets as usize]);
        } else {
            self.clean_table_view();
        }
    }

    /// Prepares the table view for use and populates it with the
    /// attributes which have been previously read.
    ///
    /// Equivalent to the protected `prepareAndPopulateTableView()`.
    fn prepare_and_populate_table_view(&mut self) {
        self.prepare_and_populate_table_view_count(self.length);
    }

    /// Equivalent to the private `prepareAndPopulateTableView(int)`.
    fn prepare_and_populate_table_view_count(&mut self, count: i32) {
        self.prepare_table_view();
        // Need to populate the hash table with the attributes we've processed so far.
        for i in 0..count as usize {
            let bucket = self.get_table_view_bucket(self.attributes[i].name.rawname.as_deref());
            self.link_into_table_view(i, bucket);
        }
    }

    /// Equivalent to the private `prepareAndPopulateTableViewNS(int)`.
    fn prepare_and_populate_table_view_ns(&mut self, count: i32) {
        self.prepare_table_view();
        // Need to populate the hash table with the attributes we've processed so far.
        for i in 0..count as usize {
            let bucket = {
                let name = &self.attributes[i].name;
                self.get_table_view_bucket_ns(name.localpart.as_deref(), name.uri.as_deref())
            };
            self.link_into_table_view(i, bucket);
        }
    }

    /// The loop body the two populating methods share: a stale chain
    /// restarts at attribute `i`, an active one gets it at its head.
    fn link_into_table_view(&mut self, i: usize, bucket: usize) {
        if self.chain_state()[bucket] != self.large_count {
            let large_count = self.large_count;
            self.chain_state_mut()[bucket] = large_count;
            self.attributes[i].next = None;
            self.table_view_mut()[bucket] = Some(i);
        } else {
            // Update table view
            self.attributes[i].next = self.table_view()[bucket];
            self.table_view_mut()[bucket] = Some(i);
        }
    }

    /// Randomly selects a new hash function and reorganizes the table view
    /// in order to more evenly distribute its entries. This method is
    /// called automatically when the number of attributes in one bucket
    /// exceeds `MAX_HASH_COLLISIONS`.
    ///
    /// Equivalent to the private `rebalanceTableView(int)`.
    fn rebalance_table_view(&mut self, count: i32) {
        let multipliers = self
            .hash_multipliers
            .get_or_insert_with(|| vec![0; MULTIPLIERS_SIZE as usize + 1]);
        PrimeNumberSequenceGenerator::generate_sequence(multipliers);
        self.prepare_and_populate_table_view_count(count);
    }

    /// Randomly selects a new hash function and reorganizes the table view
    /// in order to more evenly distribute its entries. This method is
    /// called automatically when the number of attributes in one bucket
    /// exceeds `MAX_HASH_COLLISIONS`.
    ///
    /// Equivalent to the private `rebalanceTableViewNS(int)`.
    fn rebalance_table_view_ns(&mut self, count: i32) {
        let multipliers = self
            .hash_multipliers
            .get_or_insert_with(|| vec![0; MULTIPLIERS_SIZE as usize + 1]);
        PrimeNumberSequenceGenerator::generate_sequence(multipliers);
        self.prepare_and_populate_table_view_ns(count);
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
        let mut index;
        if self.length < SIZE_LIMIT {
            index = if has_uri {
                self.get_index_fast_ns(name.uri.as_deref(), name.localpart.as_deref())
            } else {
                self.get_index_fast(name.rawname.as_deref())
            };

            if index == -1 {
                index = self.append_index(false);
            }
        } else {
            // `name.uri == null || name.uri.length() == 0 ||
            // (index = getIndexFast(name.uri, name.localpart)) == -1`
            index = if has_uri {
                self.get_index_fast_ns(name.uri.as_deref(), name.localpart.as_deref())
            } else {
                -1
            };
            if index == -1 {
                // If attributes were removed from the list after the table view
                // was last used, we have to rebuild the table view. If the list
                // grew past SIZE_LIMIT, the table view is populated for the
                // first time. If the list outgrew the table view, it is
                // repopulated in a larger table.
                if !self.is_table_view_consistent
                    || self.length == SIZE_LIMIT
                    || (self.length > SIZE_LIMIT && self.length > self.table_view_buckets)
                {
                    self.prepare_and_populate_table_view();
                    self.is_table_view_consistent = true;
                }

                let bucket = self.get_table_view_bucket(name.rawname.as_deref());

                // The chain is stale.
                // This must be a unique attribute.
                if self.chain_state()[bucket] != self.large_count {
                    index = self.append_index(true);

                    // Update table view.
                    let large_count = self.large_count;
                    self.chain_state_mut()[bucket] = large_count;
                    self.attributes[index as usize].next = None;
                    self.table_view_mut()[bucket] = Some(index as usize);
                }
                // This chain is active.
                // We need to check if any of the attributes has the same rawname.
                else {
                    // Search the table.
                    let mut collision_count = 0;
                    let mut found = self.table_view()[bucket];
                    while let Some(f) = found {
                        if self.attributes[f].name.rawname == name.rawname {
                            break;
                        }
                        found = self.attributes[f].next;
                        collision_count += 1;
                    }
                    // This attribute is unique.
                    if found.is_none() {
                        index = self.append_index(true);
                        // Select a new hash function and rehash the table view
                        // if the collision threshold is exceeded.
                        if collision_count >= MAX_HASH_COLLISIONS {
                            // The current attribute will be processed in the rehash.
                            // Need to set its name first.
                            self.attributes[index as usize].name.set_values_from(name);
                            self.rebalance_table_view(self.length);
                        } else {
                            // Update table view
                            self.attributes[index as usize].next = self.table_view()[bucket];
                            self.table_view_mut()[bucket] = Some(index as usize);
                        }
                    }
                    // Duplicate. We still need to find the index.
                    else {
                        index = self.get_index_fast(name.rawname.as_deref());
                    }
                }
            }
        }

        // set values
        self.reset_attribute(index, name, type_, value);
        index
    }

    fn remove_all_attributes(&mut self) {
        self.length = 0;
    }

    fn remove_attribute_at(&mut self, attr_index: i32) {
        self.is_table_view_consistent = false;
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
        self.attributes[attr_index as usize]
            .name
            .set_values_from(attr_name);
    }

    fn get_name(&self, attr_index: i32, attr_name: &mut QName) {
        attr_name.set_values_from(&self.attributes[attr_index as usize].name);
    }

    fn get_prefix(&self, index: i32) -> Option<String> {
        if index < 0 || index >= self.length {
            return None;
        }
        Some(
            self.attributes[index as usize]
                .name
                .prefix
                .clone()
                .unwrap_or_default(),
        )
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
        Some(
            self.attributes[index as usize]
                .name
                .rawname
                .clone()
                .unwrap_or_default(),
        )
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
        self.attributes[attr_index as usize]
            .non_normalized_value
            .clone()
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

    fn get_augmentations_ns(
        &self,
        uri: Option<&str>,
        local_part: &str,
    ) -> Option<Rc<dyn Augmentations>> {
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
