// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! The concrete resource classes of ICU bundles, ported from
//! `com.ibm.icu.impl.ICUResourceBundleImpl`: its members are methods of
//! [`ICUResourceBundle`] declared in this module, and its subclasses are
//! the [`ImplKind`] of an [`ICUResourceBundle`].

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use rucene::error::Result;

use crate::jdk::ByteBuffer;
use crate::r#impl::cache_value::CacheValue;
use crate::r#impl::icu_resource_bundle::{ICUResourceBundle, WholeBundle};
use crate::r#impl::icu_resource_bundle_reader::{self, Container, ICUResourceBundleReader};
use crate::util::u_resource_bundle::{ResourceObject, UResourceBundle};
use crate::util::u_resource_type_mismatch_exception::type_mismatch;

/// The subclass of `ICUResourceBundleImpl` an ICU resource is.
///
/// The private and package-private nested classes of
/// `ICUResourceBundleImpl` are its variants: `ICUResourceBundleImpl.ResourceString`,
/// `ICUResourceBundleImpl.ResourceBinary`, `ICUResourceBundleImpl.ResourceInt`,
/// `ICUResourceBundleImpl.ResourceIntVector` and the abstract
/// `ICUResourceBundleImpl.ResourceContainer`'s subclasses
/// `ICUResourceBundleImpl.ResourceArray` and `ICUResourceBundleImpl.ResourceTable`
/// (`ICUResourceBundleImpl.java:63-308`). The members of
/// `ResourceContainer` are methods of [`ICUResourceBundle`] that act on the
/// `Array` and `Table` variants.
#[derive(Debug)]
pub enum ImplKind {
    /// `ResourceString`: `value` holds the string unless it is large (Java
    /// lets the reader cache's soft reference do its job).
    String {
        /// `value`.
        value: Option<Vec<u16>>,
    },
    /// `ResourceBinary`.
    Binary,
    /// `ResourceInt`.
    Int,
    /// `ResourceIntVector`.
    IntVector,
    /// `ResourceArray`, with its reader container.
    Array {
        /// `value`.
        value: Arc<Container>,
    },
    /// `ResourceTable`, with its reader container.
    Table {
        /// `value`.
        value: Arc<Container>,
    },
}

impl ICUResourceBundle {
    /// `ICUResourceBundleImpl(WholeBundle)` with
    /// `ResourceTable(WholeBundle wholeBundle, int rootRes)`.
    pub(crate) fn new_root_table(whole_bundle: Arc<WholeBundle>, root_res: i32) -> Self {
        let value = whole_bundle
            .reader
            .get_table(root_res)
            .expect("the root resource is a table");
        let resource = whole_bundle.reader.get_root_resource();
        Self {
            whole_bundle,
            container: None,
            key: None,
            resource,
            parent: RwLock::new(None),
            kind: ImplKind::Table { value },
        }
    }

    /// `ICUResourceBundleImpl(ICUResourceBundleImpl container, String key,
    /// int resource)`: an item, sharing its container's bundle and parent.
    fn new_item(
        container: &Arc<ICUResourceBundle>,
        key: &str,
        resource: i32,
        kind: ImplKind,
    ) -> Self {
        Self {
            whole_bundle: container.whole_bundle.clone(),
            container: Some(container.clone()),
            key: Some(key.to_string()),
            resource,
            parent: RwLock::new(container.get_parent()),
            kind,
        }
    }

    /// `getResource()`.
    pub fn get_resource(&self) -> i32 {
        self.resource
    }

    /// The Java class name of the resource.
    pub fn java_class_name(&self) -> &'static str {
        match self.kind {
            ImplKind::String { .. } => "com.ibm.icu.impl.ICUResourceBundleImpl$ResourceString",
            ImplKind::Binary => "com.ibm.icu.impl.ICUResourceBundleImpl$ResourceBinary",
            ImplKind::Int => "com.ibm.icu.impl.ICUResourceBundleImpl$ResourceInt",
            ImplKind::IntVector => "com.ibm.icu.impl.ICUResourceBundleImpl$ResourceIntVector",
            ImplKind::Array { .. } => "com.ibm.icu.impl.ICUResourceBundleImpl$ResourceArray",
            ImplKind::Table { .. } => "com.ibm.icu.impl.ICUResourceBundleImpl$ResourceTable",
        }
    }

    /// `this instanceof ResourceContainer`.
    pub fn is_container(&self) -> bool {
        matches!(self.kind, ImplKind::Array { .. } | ImplKind::Table { .. })
    }

    /// `this instanceof ResourceTable`.
    pub fn is_table(&self) -> bool {
        matches!(self.kind, ImplKind::Table { .. })
    }

    /// `ResourceContainer.value`.
    pub(crate) fn container_value(&self) -> Option<&Arc<Container>> {
        match &self.kind {
            ImplKind::Array { value } | ImplKind::Table { value } => Some(value),
            _ => None,
        }
    }

    fn reader(&self) -> &ICUResourceBundleReader {
        &self.whole_bundle.reader
    }

    /// `createBundleObject(String _key, int _resource, HashMap<String,
    /// String> aliasesVisited, UResourceBundle requested)` (protected
    /// final).
    ///
    /// # Errors
    ///
    /// `IllegalStateException("The resource type is unknown")`; the errors
    /// of alias resolution.
    pub(crate) fn create_bundle_object(
        self: &Arc<Self>,
        key: &str,
        resource: i32,
        aliases_visited: Option<&mut HashMap<String, String>>,
        requested: &UResourceBundle,
    ) -> Result<Arc<ICUResourceBundle>> {
        let kind = match ICUResourceBundleReader::res_get_type(resource) {
            UResourceBundle::STRING | ICUResourceBundle::STRING_V2 => {
                let s = self
                    .reader()
                    .get_string(resource)
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                // Allow the reader cache's SoftReference to do its job.
                let value = if (s.len() as i32) < icu_resource_bundle_reader::LARGE_SIZE / 2
                    || CacheValue::<()>::future_instances_will_be_strong()
                {
                    Some(s)
                } else {
                    None
                };
                ImplKind::String { value }
            }
            UResourceBundle::BINARY => ImplKind::Binary,
            ICUResourceBundle::ALIAS => {
                return ICUResourceBundle::get_aliased_resource_of(
                    self,
                    None,
                    0,
                    key,
                    resource,
                    aliases_visited,
                    requested,
                )
            }
            UResourceBundle::INT => ImplKind::Int,
            UResourceBundle::INT_VECTOR => ImplKind::IntVector,
            UResourceBundle::ARRAY | ICUResourceBundle::ARRAY16 => ImplKind::Array {
                value: self
                    .reader()
                    .get_array(resource)
                    .expect("an array resource has an array"),
            },
            UResourceBundle::TABLE | ICUResourceBundle::TABLE16 | ICUResourceBundle::TABLE32 => {
                ImplKind::Table {
                    value: self
                        .reader()
                        .get_table(resource)
                        .expect("a table resource has a table"),
                }
            }
            _ => {
                return Err(rucene::error::LuceneError::illegal_state(
                    "The resource type is unknown",
                ))
            }
        };
        Ok(Arc::new(Self::new_item(self, key, resource, kind)))
    }

    // Scalar values ------------------------------------------------------- ***

    /// `getType()`.
    pub fn get_type(&self) -> i32 {
        match self.kind {
            ImplKind::String { .. } => UResourceBundle::STRING,
            ImplKind::Binary => UResourceBundle::BINARY,
            ImplKind::Int => UResourceBundle::INT,
            ImplKind::IntVector => UResourceBundle::INT_VECTOR,
            ImplKind::Array { .. } => UResourceBundle::ARRAY,
            ImplKind::Table { .. } => UResourceBundle::TABLE,
        }
    }

    /// `getBinary()`: `ResourceBinary`'s buffer.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for another type.
    pub fn get_binary(&self) -> Result<ByteBuffer> {
        match self.kind {
            ImplKind::Binary => self
                .reader()
                .get_binary(self.resource)
                .ok_or_else(|| type_mismatch("")),
            _ => Err(type_mismatch("")),
        }
    }

    /// `getBinary(byte[] ba)`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for another type.
    pub fn get_binary_bytes(&self, ba: Option<Vec<u8>>) -> Result<Vec<u8>> {
        match self.kind {
            ImplKind::Binary => self
                .reader()
                .get_binary_bytes(self.resource, ba)
                .ok_or_else(|| type_mismatch("")),
            _ => Err(type_mismatch("")),
        }
    }

    /// `getInt()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for another type.
    pub fn get_int(&self) -> Result<i32> {
        match self.kind {
            ImplKind::Int => Ok(ICUResourceBundleReader::res_get_int(self.resource)),
            _ => Err(type_mismatch("")),
        }
    }

    /// `getUInt()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for another type.
    pub fn get_u_int(&self) -> Result<i32> {
        match self.kind {
            ImplKind::Int => Ok(ICUResourceBundleReader::res_get_uint(self.resource)),
            _ => Err(type_mismatch("")),
        }
    }

    /// `getString()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for another type.
    pub fn get_string(&self) -> Result<Vec<u16>> {
        match &self.kind {
            ImplKind::String { value } => {
                if let Some(v) = value {
                    return Ok(v.clone());
                }
                self.reader()
                    .get_string(self.resource)
                    .ok_or_else(rucene::error::LuceneError::null_pointer)
            }
            _ => Err(type_mismatch("")),
        }
    }

    /// `getIntVector()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for another type.
    pub fn get_int_vector(&self) -> Result<Vec<i32>> {
        match self.kind {
            ImplKind::IntVector => self
                .reader()
                .get_int_vector(self.resource)
                .ok_or_else(|| type_mismatch("")),
            _ => Err(type_mismatch("")),
        }
    }

    // Container values ---------------------------------------------------- ***

    /// `getSize()`: the container's size, `1` for a scalar.
    pub fn get_size(&self) -> i32 {
        match self.container_value() {
            Some(v) => v.get_size(),
            None => 1,
        }
    }

    /// `ResourceContainer.getString(int index)`.
    ///
    /// # Errors
    ///
    /// The errors of `UResourceBundle.getString(int)` for a non-string item.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an index out of range.
    pub(crate) fn container_get_string(
        &self,
        index: i32,
        this: &UResourceBundle,
    ) -> Result<Vec<u16>> {
        let value = self.container_value().expect("a container has a value");
        let res = value.get_container_resource(self.reader(), index);
        if res == ICUResourceBundle::RES_BOGUS {
            crate::jdk::index_out_of_bounds(None);
        }
        if let Some(s) = self.reader().get_string(res) {
            return Ok(s);
        }
        this.base_get_string_at(index)
    }

    /// `ResourceContainer.getContainerResource(int index)` (protected).
    fn get_container_resource(&self, index: i32) -> i32 {
        self.container_value()
            .expect("a container has a value")
            .get_container_resource(self.reader(), index)
    }

    /// `ResourceContainer.createBundleObject(int index, String resKey,
    /// HashMap<String, String> aliasesVisited, UResourceBundle requested)`
    /// (protected).
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an index out of range.
    fn create_bundle_object_at(
        self: &Arc<Self>,
        index: i32,
        res_key: &str,
        aliases_visited: Option<&mut HashMap<String, String>>,
        requested: &UResourceBundle,
    ) -> Result<Arc<ICUResourceBundle>> {
        let item = self.get_container_resource(index);
        if item == ICUResourceBundle::RES_BOGUS {
            crate::jdk::index_out_of_bounds(None);
        }
        self.create_bundle_object(res_key, item, aliases_visited, requested)
    }

    /// `ResourceArray.handleGetStringArray()`, `null` for the other
    /// classes.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for an item that is not a string.
    pub fn handle_get_string_array(&self) -> Result<Option<Vec<Vec<u16>>>> {
        match &self.kind {
            ImplKind::Array { value } => {
                let reader = self.reader();
                let length = value.get_size();
                let mut strings = Vec::with_capacity(length as usize);
                for i in 0..length {
                    match reader.get_string(value.get_container_resource(reader, i)) {
                        Some(s) => strings.push(s),
                        None => return Err(type_mismatch("")),
                    }
                }
                Ok(Some(strings))
            }
            _ => Ok(None),
        }
    }

    /// `getStringArray()`: `ResourceArray`'s strings.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for another type or a non-string
    /// item.
    pub fn get_string_array(&self) -> Result<Vec<Vec<u16>>> {
        match self.kind {
            ImplKind::Array { .. } => Ok(self
                .handle_get_string_array()?
                .expect("an array has strings")),
            _ => Err(type_mismatch("")),
        }
    }

    /// `handleGet(String resKey, HashMap<String, String> aliasesVisited,
    /// UResourceBundle requested)`: an array reads the key as an index
    /// (`Integer.parseInt`), a table looks it up; scalars have no items.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for an array key that is not a number; the
    /// errors of alias resolution.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an array index out of range.
    pub fn handle_get(
        self: &Arc<Self>,
        res_key: &str,
        aliases_visited: Option<&mut HashMap<String, String>>,
        requested: &UResourceBundle,
    ) -> Result<Option<UResourceBundle>> {
        match &self.kind {
            ImplKind::Array { .. } => {
                let units: Vec<u16> = res_key.encode_utf16().collect();
                let i = crate::jdk::parse_int(&units, 10)?;
                Ok(Some(UResourceBundle::Icu(self.create_bundle_object_at(
                    i,
                    res_key,
                    aliases_visited,
                    requested,
                )?)))
            }
            ImplKind::Table { value } => {
                let key: Vec<u16> = res_key.encode_utf16().collect();
                let i = value.find_table_item(self.reader(), &key);
                if i < 0 {
                    return Ok(None);
                }
                let res = self.get_container_resource(i);
                Ok(Some(UResourceBundle::Icu(self.create_bundle_object(
                    res_key,
                    res,
                    aliases_visited,
                    requested,
                )?)))
            }
            _ => Ok(None),
        }
    }

    /// `handleGet(int index, HashMap<String, String> aliasesVisited,
    /// UResourceBundle requested)`.
    ///
    /// # Errors
    ///
    /// The errors of alias resolution.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for an index out of range.
    pub fn handle_get_at(
        self: &Arc<Self>,
        index: i32,
        aliases_visited: Option<&mut HashMap<String, String>>,
        requested: &UResourceBundle,
    ) -> Result<Option<UResourceBundle>> {
        match &self.kind {
            ImplKind::Array { .. } => {
                Ok(Some(UResourceBundle::Icu(self.create_bundle_object_at(
                    index,
                    &index.to_string(),
                    aliases_visited,
                    requested,
                )?)))
            }
            ImplKind::Table { value } => {
                let item_key = match value.get_key(self.reader(), index) {
                    Some(k) => k,
                    None => crate::jdk::index_out_of_bounds(None),
                };
                let res = self.get_container_resource(index);
                Ok(Some(UResourceBundle::Icu(self.create_bundle_object(
                    &item_key,
                    res,
                    aliases_visited,
                    requested,
                )?)))
            }
            _ => Ok(None),
        }
    }

    /// `ResourceTable.getKey(int index)` (protected).
    pub fn table_get_key(&self, index: i32) -> Option<String> {
        match &self.kind {
            ImplKind::Table { value } => value.get_key(self.reader(), index),
            _ => None,
        }
    }

    /// `handleKeySet()`: the sorted keys of a table, empty otherwise.
    pub fn handle_key_set(&self) -> Vec<String> {
        match &self.kind {
            ImplKind::Table { value } => {
                let reader = self.reader();
                let mut key_set = Vec::with_capacity(value.get_size() as usize);
                for i in 0..value.get_size() {
                    key_set.push(
                        value
                            .get_key(reader, i)
                            .expect("an index in range has a key"),
                    );
                }
                crate::util::u_resource_bundle::tree_set(key_set)
            }
            _ => Vec::new(),
        }
    }

    /// `ResourceTable.handleGetObject(String key)`: the fast path for
    /// strings and string arrays, then `UResourceBundle.handleGetObject`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the key is found nowhere.
    pub(crate) fn table_handle_get_object(
        &self,
        key: &str,
        this: &UResourceBundle,
    ) -> Result<ResourceObject> {
        // Fast path for common cases: Avoid creating UResourceBundles if possible.
        // It would be even better if we could override getString(key)/getStringArray(key),
        // so that we know the expected object type,
        // but those are final in java.util.ResourceBundle.
        let reader = self.reader();
        let value = self.container_value().expect("a table has a value");
        let units: Vec<u16> = key.encode_utf16().collect();
        let index = value.find_table_item(reader, &units);
        if index >= 0 {
            let res = value.get_container_resource(reader, index);
            // getString(key)
            if let Some(s) = reader.get_string(res) {
                return Ok(ResourceObject::String(s));
            }
            // getStringArray(key)
            if let Some(array) = reader.get_array(res) {
                let length = array.get_size();
                let mut strings = Vec::with_capacity(length as usize);
                let mut j = 0;
                loop {
                    if j == length {
                        return Ok(ResourceObject::StringArray(strings));
                    }
                    match reader.get_string(array.get_container_resource(reader, j)) {
                        Some(s) => strings.push(s),
                        // Equivalent to resolveObject(key, requested):
                        // If this is not a string array,
                        // then build and return a UResourceBundle.
                        None => break,
                    }
                    j += 1;
                }
            }
        }
        this.base_handle_get_object(key)
    }

    /// `ResourceTable.findString(String key)` (package-private): the string
    /// of `key` in this table, `None` when absent or not a string.
    pub(crate) fn find_string(&self, key: &str) -> Option<Vec<u16>> {
        let reader = self.reader();
        let value = self.container_value()?;
        let units: Vec<u16> = key.encode_utf16().collect();
        let index = value.find_table_item(reader, &units);
        if index < 0 {
            return None;
        }
        reader.get_string(value.get_container_resource(reader, index))
    }
}
