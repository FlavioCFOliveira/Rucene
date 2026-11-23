// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.UResourceBundle`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::jdk::{ByteBuffer, ClassLoader};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{self, ICUResourceBundle};
use crate::r#impl::icu_resource_bundle_reader::ICUResourceBundleReader;
use crate::r#impl::resource_bundle_wrapper::ResourceBundleWrapper;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle_iterator::UResourceBundleIterator;
use crate::util::u_resource_type_mismatch_exception::{type_mismatch, UResourceTypeMismatchException};
use crate::util::version_info::VersionInfo;

/// What `ResourceBundle.getObject` returns for an ICU bundle: a string, a
/// string array or a bundle.
#[derive(Debug, Clone)]
pub enum ResourceObject {
    /// A `String`.
    String(Vec<u16>),
    /// A `String[]`.
    StringArray(Vec<Vec<u16>>),
    /// A `UResourceBundle`.
    Bundle(UResourceBundle),
}

/// A class representing a collection of resource information pertaining to
/// a given locale, and an item of such a collection.
///
/// Equivalent to the abstract class `com.ibm.icu.util.UResourceBundle`,
/// together with the `java.util.ResourceBundle` members it relies on (the
/// `parent` chain, `getObject`, `getString(String)`, `getStringArray(String)`).
/// Its concrete subclasses in ICU4J are `ICUResourceBundle` (through
/// `ICUResourceBundleImpl`) and `ResourceBundleWrapper`, which are the
/// variants; the methods dispatch to the overriding class's member, or run
/// the base class's body where the subclass does not override it.
#[derive(Debug, Clone)]
pub enum UResourceBundle {
    /// An `ICUResourceBundle`.
    Icu(Arc<ICUResourceBundle>),
    /// A `ResourceBundleWrapper`.
    Wrapper(Arc<ResourceBundleWrapper>),
}

/// `RootType` (private).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootType {
    Missing,
    Icu,
    Java,
}

/// `ROOT_CACHE`.
static ROOT_CACHE: LazyLock<Mutex<HashMap<String, RootType>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// `new MissingResourceException(s, className, key)` as the error.
pub(crate) fn missing_resource(s: impl Into<String>, class_name: impl Into<String>, key: impl Into<String>) -> LuceneError {
    MissingResourceException::new(s, class_name, key).error()
}

/// Whether `e` is a `MissingResourceException`.
pub(crate) fn is_missing_resource(e: &LuceneError) -> bool {
    MissingResourceException::of(e).is_some()
}

/// Whether `e` is a `UResourceTypeMismatchException`.
pub(crate) fn is_type_mismatch(e: &LuceneError) -> bool {
    UResourceTypeMismatchException::of(e).is_some()
}

/// Java's `String.valueOf` of a nullable key.
pub(crate) fn null_str(s: Option<&str>) -> &str {
    s.unwrap_or("null")
}

/// `new TreeSet<String>(...)` over the given strings: sorted in Java's
/// `String` order, duplicates removed.
pub(crate) fn tree_set(mut v: Vec<String>) -> Vec<String> {
    v.sort_by(|a, b| rucene::util::java_collections::java_compare_to(a, b));
    v.dedup();
    v
}

impl UResourceBundle {
    /// `NONE`: resource type constant for "no resource".
    pub const NONE: i32 = -1;
    /// `STRING`.
    pub const STRING: i32 = 0;
    /// `BINARY`.
    pub const BINARY: i32 = 1;
    /// `TABLE`.
    pub const TABLE: i32 = 2;
    /// `INT`.
    pub const INT: i32 = 7;
    /// `ARRAY`.
    pub const ARRAY: i32 = 8;
    /// `INT_VECTOR`.
    pub const INT_VECTOR: i32 = 14;

    /// `getBundleInstance(String baseName, String localeName)`: the bundle
    /// from ICU's class loader, with fallback.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance(base_name: Option<&str>, locale_name: &str) -> Result<UResourceBundle> {
        Self::get_bundle_instance_full(
            base_name,
            locale_name,
            icu_resource_bundle::icu_data_class_loader(),
            false,
        )
    }

    /// `getBundleInstance(String baseName, String localeName, ClassLoader
    /// root)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_loader(
        base_name: Option<&str>,
        locale_name: &str,
        root: ClassLoader,
    ) -> Result<UResourceBundle> {
        Self::get_bundle_instance_full(base_name, locale_name, root, false)
    }

    /// `getBundleInstance(String baseName, String localeName, ClassLoader
    /// root, boolean disableFallback)` (protected).
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_full(
        base_name: Option<&str>,
        locale_name: &str,
        root: ClassLoader,
        disable_fallback: bool,
    ) -> Result<UResourceBundle> {
        Self::instantiate_bundle(base_name, locale_name, root, disable_fallback)
    }

    /// `getBundleInstance(ULocale locale)`: the ICU data bundle of `locale`
    /// (`None`: the default locale).
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_ulocale(locale: Option<&ULocale>) -> Result<UResourceBundle> {
        let default;
        let locale = match locale {
            Some(l) => l,
            None => {
                default = ULocale::get_default();
                &default
            }
        };
        Self::get_bundle_instance_full(
            Some(ICUData::ICU_BASE_NAME),
            &locale.get_base_name(),
            icu_resource_bundle::icu_data_class_loader(),
            false,
        )
    }

    /// `getBundleInstance(String baseName)`: the bundle of the default
    /// locale (`baseName` `None`: the ICU data).
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_default(base_name: Option<&str>) -> Result<UResourceBundle> {
        let base_name = base_name.unwrap_or(ICUData::ICU_BASE_NAME);
        let uloc = ULocale::get_default();
        Self::get_bundle_instance_full(
            Some(base_name),
            &uloc.get_base_name(),
            icu_resource_bundle::icu_data_class_loader(),
            false,
        )
    }

    /// `getBundleInstance(String baseName, Locale locale)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_locale(base_name: Option<&str>, locale: Option<&Locale>) -> Result<UResourceBundle> {
        Self::get_bundle_instance_locale_loader(base_name, locale, icu_resource_bundle::icu_data_class_loader())
    }

    /// `getBundleInstance(String baseName, ULocale locale)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_for(base_name: Option<&str>, locale: Option<&ULocale>) -> Result<UResourceBundle> {
        Self::get_bundle_instance_for_loader(base_name, locale, icu_resource_bundle::icu_data_class_loader())
    }

    /// `getBundleInstance(String baseName, Locale locale, ClassLoader
    /// loader)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_locale_loader(
        base_name: Option<&str>,
        locale: Option<&Locale>,
        loader: ClassLoader,
    ) -> Result<UResourceBundle> {
        let base_name = base_name.unwrap_or(ICUData::ICU_BASE_NAME);
        let uloc = match locale {
            None => ULocale::get_default(),
            Some(l) => ULocale::for_locale(Some(l))?.ok_or_else(LuceneError::null_pointer)?,
        };
        Self::get_bundle_instance_full(Some(base_name), &uloc.get_base_name(), loader, false)
    }

    /// `getBundleInstance(String baseName, ULocale locale, ClassLoader
    /// loader)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found.
    pub fn get_bundle_instance_for_loader(
        base_name: Option<&str>,
        locale: Option<&ULocale>,
        loader: ClassLoader,
    ) -> Result<UResourceBundle> {
        let base_name = base_name.unwrap_or(ICUData::ICU_BASE_NAME);
        let default;
        let locale = match locale {
            Some(l) => l,
            None => {
                default = ULocale::get_default();
                &default
            }
        };
        Self::get_bundle_instance_full(Some(base_name), &locale.get_base_name(), loader, false)
    }

    /// `getULocale()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from a wrapper's `new ULocale(localeID)`.
    pub fn get_u_locale(&self) -> Result<ULocale> {
        match self {
            UResourceBundle::Icu(b) => Ok(b.get_u_locale()),
            UResourceBundle::Wrapper(w) => w.get_u_locale(),
        }
    }

    /// `getLocaleID()` (protected).
    pub fn get_locale_id(&self) -> String {
        match self {
            UResourceBundle::Icu(b) => b.get_locale_id().to_string(),
            UResourceBundle::Wrapper(w) => w.get_locale_id(),
        }
    }

    /// `getBaseName()` (protected).
    pub fn get_base_name(&self) -> String {
        match self {
            UResourceBundle::Icu(b) => b.get_base_name().to_string(),
            UResourceBundle::Wrapper(w) => w.get_base_name(),
        }
    }

    /// `getParent()` (protected).
    pub fn get_parent(&self) -> Option<UResourceBundle> {
        match self {
            UResourceBundle::Icu(b) => b.get_parent_bundle(),
            UResourceBundle::Wrapper(w) => w.get_parent(),
        }
    }

    /// `getLocale()`: `getULocale().toLocale()`.
    ///
    /// # Errors
    ///
    /// As [`get_u_locale`](Self::get_u_locale).
    pub fn get_locale(&self) -> Result<Locale> {
        Ok(self.get_u_locale()?.to_locale())
    }

    /// `getRootType(String baseName, ClassLoader root)` (private).
    fn get_root_type(base_name: &str, root: ClassLoader) -> Result<RootType> {
        let cached = ROOT_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(base_name)
            .copied();
        if let Some(r) = cached {
            return Ok(r);
        }
        let root_locale = if !base_name.contains('.') { "root" } else { "" };
        let root_type = match ICUResourceBundle::get_bundle_instance(Some(base_name), root_locale, root, true) {
            Ok(_) => RootType::Icu,
            Err(ex) if is_missing_resource(&ex) => {
                match ResourceBundleWrapper::get_bundle_instance(base_name, root_locale, Some(root), true) {
                    Ok(_) => RootType::Java,
                    //throw away the exception
                    Err(e) if is_missing_resource(&e) => RootType::Missing,
                    Err(e) => return Err(e),
                }
            }
            Err(ex) => return Err(ex),
        };
        Self::set_root_type(base_name, root_type);
        Ok(root_type)
    }

    fn set_root_type(base_name: &str, root_type: RootType) {
        ROOT_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(base_name.to_string(), root_type);
    }

    /// `instantiateBundle(String baseName, String localeName, ClassLoader
    /// root, boolean disableFallback)` (protected): loads a new resource
    /// bundle for the given base name, locale and class loader.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when no bundle is found;
    /// `NullPointerException` for a `None` base name, as Java's
    /// `ConcurrentHashMap.get(null)`.
    pub fn instantiate_bundle(
        base_name: Option<&str>,
        locale_name: &str,
        root: ClassLoader,
        disable_fallback: bool,
    ) -> Result<UResourceBundle> {
        let base_name = base_name.ok_or_else(LuceneError::null_pointer)?;
        let root_type = Self::get_root_type(base_name, root)?;
        match root_type {
            RootType::Icu => Ok(UResourceBundle::Icu(ICUResourceBundle::get_bundle_instance(
                Some(base_name),
                locale_name,
                root,
                disable_fallback,
            )?)),
            RootType::Java => Ok(UResourceBundle::Wrapper(ResourceBundleWrapper::get_bundle_instance(
                base_name,
                locale_name,
                Some(root),
                disable_fallback,
            )?)),
            RootType::Missing => {
                match ICUResourceBundle::get_bundle_instance(Some(base_name), locale_name, root, disable_fallback) {
                    Ok(b) => {
                        Self::set_root_type(base_name, RootType::Icu);
                        Ok(UResourceBundle::Icu(b))
                    }
                    Err(ex) if is_missing_resource(&ex) => {
                        let b = ResourceBundleWrapper::get_bundle_instance(
                            base_name,
                            locale_name,
                            Some(root),
                            disable_fallback,
                        )?;
                        Self::set_root_type(base_name, RootType::Java);
                        Ok(UResourceBundle::Wrapper(b))
                    }
                    Err(ex) => Err(ex),
                }
            }
        }
    }

    /// `getBinary()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` when the resource is not binary.
    pub fn get_binary(&self) -> Result<ByteBuffer> {
        match self {
            UResourceBundle::Icu(b) => b.get_binary(),
            UResourceBundle::Wrapper(_) => Err(type_mismatch("")),
        }
    }

    /// `getString()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` when the resource is not a string.
    pub fn get_string(&self) -> Result<Vec<u16>> {
        match self {
            UResourceBundle::Icu(b) => b.get_string(),
            UResourceBundle::Wrapper(_) => Err(type_mismatch("")),
        }
    }

    /// `getStringArray()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` when the resource is not a string
    /// array.
    pub fn get_string_array(&self) -> Result<Vec<Vec<u16>>> {
        match self {
            UResourceBundle::Icu(b) => b.get_string_array(),
            UResourceBundle::Wrapper(_) => Err(type_mismatch("")),
        }
    }

    /// `getBinary(byte[] ba)`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` when the resource is not binary.
    pub fn get_binary_bytes(&self, ba: Option<Vec<u8>>) -> Result<Vec<u8>> {
        match self {
            UResourceBundle::Icu(b) => b.get_binary_bytes(ba),
            UResourceBundle::Wrapper(_) => Err(type_mismatch("")),
        }
    }

    /// `getIntVector()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` when the resource is not an int
    /// vector.
    pub fn get_int_vector(&self) -> Result<Vec<i32>> {
        match self {
            UResourceBundle::Icu(b) => b.get_int_vector(),
            UResourceBundle::Wrapper(_) => Err(type_mismatch("")),
        }
    }

    /// `getInt()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` when the resource is not an int.
    pub fn get_int(&self) -> Result<i32> {
        match self {
            UResourceBundle::Icu(b) => b.get_int(),
            UResourceBundle::Wrapper(_) => Err(type_mismatch("")),
        }
    }

    /// `getUInt()`.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` when the resource is not an int.
    pub fn get_u_int(&self) -> Result<i32> {
        match self {
            UResourceBundle::Icu(b) => b.get_u_int(),
            UResourceBundle::Wrapper(_) => Err(type_mismatch("")),
        }
    }

    /// The Java class name, as `getClass().getName()` gives it.
    pub fn java_class_name(&self) -> &'static str {
        match self {
            UResourceBundle::Icu(b) => b.java_class_name(),
            UResourceBundle::Wrapper(_) => "com.ibm.icu.impl.ResourceBundleWrapper",
        }
    }

    /// `get(String aKey)`: the resource of a top-level key, with fallback.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the key is found nowhere.
    pub fn get(&self, a_key: &str) -> Result<UResourceBundle> {
        match self.find_top_level(a_key)? {
            Some(obj) => Ok(obj),
            None => {
                let full_name = ICUResourceBundleReader::get_full_name(Some(&self.get_base_name()), &self.get_locale_id());
                Err(missing_resource(
                    format!("Can't find resource for bundle {full_name}, key {a_key}"),
                    self.java_class_name(),
                    a_key,
                ))
            }
        }
    }

    /// `findTopLevel(String aKey)` (protected, deprecated): the resource of
    /// a top-level key in this bundle or its parents.
    ///
    /// # Errors
    ///
    /// The errors of `handleGet`.
    pub fn find_top_level(&self, a_key: &str) -> Result<Option<UResourceBundle>> {
        // NOTE: this only works for top-level resources.  For resources at lower
        // levels, it fails when you fall back to the parent, since you're now
        // looking at root resources, not at the corresponding nested resource.
        let mut res = Some(self.clone());
        while let Some(r) = res {
            if let Some(obj) = r.handle_get(a_key, None, self)? {
                return Ok(Some(obj));
            }
            res = r.get_parent();
        }
        Ok(None)
    }

    /// `getString(int index)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` for a missing item;
    /// `UResourceTypeMismatchException` when it is not a string.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` from a container for an index out of
    /// range.
    pub fn get_string_at(&self, index: i32) -> Result<Vec<u16>> {
        match self {
            UResourceBundle::Icu(b) if b.is_container() => b.container_get_string(index, self),
            _ => self.base_get_string_at(index),
        }
    }

    /// The `UResourceBundle.getString(int)` body.
    pub(crate) fn base_get_string_at(&self, index: i32) -> Result<Vec<u16>> {
        let temp = self.get_at(index)?;
        let temp = match &temp {
            UResourceBundle::Icu(t) => t,
            UResourceBundle::Wrapper(_) => {
                return Err(LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "class com.ibm.icu.impl.ResourceBundleWrapper cannot be cast to class com.ibm.icu.impl.ICUResourceBundle",
                ))
            }
        };
        if temp.get_type() == Self::STRING {
            return temp.get_string();
        }
        Err(type_mismatch(""))
    }

    /// `get(int index)`: the item at `index`, with fallback.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when it is found nowhere.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` from a container for an index out of
    /// range.
    pub fn get_at(&self, index: i32) -> Result<UResourceBundle> {
        let mut obj = self.handle_get_at(index, None, self)?;
        if obj.is_none() {
            obj = self.get_parent();
            if let Some(p) = obj {
                obj = Some(p.get_at(index)?);
            }
            if obj.is_none() {
                let key = self.get_key();
                return Err(missing_resource(
                    format!(
                        "Can't find resource for bundle {}, key {}",
                        self.java_class_name(),
                        null_str(key.as_deref())
                    ),
                    self.java_class_name(),
                    null_str(key.as_deref()),
                ));
            }
        }
        Ok(obj.expect("checked above"))
    }

    /// `findTopLevel(int index)` (protected, deprecated).
    ///
    /// # Errors
    ///
    /// The errors of `handleGet`.
    pub fn find_top_level_at(&self, index: i32) -> Result<Option<UResourceBundle>> {
        // NOTE: this _barely_ works for top-level resources.  For resources at lower
        // levels, it fails when you fall back to the parent, since you're now
        // looking at root resources, not at the corresponding nested resource.
        // Not only that, but unless the indices correspond 1-to-1, the index will
        // lose meaning.  Essentially this only works if the child resource arrays
        // are prefixes of their parent arrays.
        let mut res = Some(self.clone());
        while let Some(r) = res {
            if let Some(obj) = r.handle_get_at(index, None, self)? {
                return Ok(Some(obj));
            }
            res = r.get_parent();
        }
        Ok(None)
    }

    /// `getKeys()`: the keys of this bundle and its parents.
    ///
    /// # Errors
    ///
    /// The errors of [`key_set`](Self::key_set).
    pub fn get_keys(&self) -> Result<Vec<String>> {
        match self {
            UResourceBundle::Wrapper(w) => Ok(w.get_keys()),
            UResourceBundle::Icu(_) => self.key_set(),
        }
    }

    /// `keySet()` (deprecated): for a top-level resource, the sorted keys
    /// of this bundle and its parents (cached for an ICU bundle); else the
    /// keys of this resource.
    ///
    /// # Errors
    ///
    /// None in practice: the key enumeration of a table cannot fail.
    pub fn key_set(&self) -> Result<Vec<String>> {
        // TODO: Java 6 ResourceBundle has keySet() which calls handleKeySet()
        // and caches the results.
        // When we upgrade to Java 6, we still need to check for isTopLevelResource().
        // Keep the else branch as is. The if body should just return super.keySet().
        // Remove then-redundant caching of the keys.
        let mut keys: Option<Arc<Vec<String>>> = None;
        let mut icurb: Option<&Arc<ICUResourceBundle>> = None;
        if self.is_top_level_resource() {
            if let UResourceBundle::Icu(b) = self {
                // We do not cache the top-level keys in this base class so that
                // not every string/int/binary... resource has to have a keys cache field.
                icurb = Some(b);
                keys = b.get_top_level_key_set();
            }
        }
        if let Some(k) = keys {
            return Ok(k.to_vec());
        }
        if self.is_top_level_resource() {
            let mut new_key_set: Vec<String> = match self.get_parent() {
                None => Vec::new(),
                Some(parent) => parent.key_set()?,
            };
            new_key_set.extend(self.handle_key_set());
            let keys = Arc::new(tree_set(new_key_set));
            if let Some(b) = icurb {
                b.set_top_level_key_set(keys.clone());
            }
            Ok(keys.to_vec())
        } else {
            Ok(self.handle_key_set())
        }
    }

    /// `handleKeySet()` (protected, deprecated): the keys of this resource
    /// alone; empty except for a table.
    pub fn handle_key_set(&self) -> Vec<String> {
        match self {
            UResourceBundle::Icu(b) => b.handle_key_set(),
            UResourceBundle::Wrapper(_) => Vec::new(),
        }
    }

    /// `getSize()`: `1` for a scalar, the number of items of a container.
    pub fn get_size(&self) -> i32 {
        match self {
            UResourceBundle::Icu(b) => b.get_size(),
            UResourceBundle::Wrapper(_) => 1,
        }
    }

    /// `getType()`.
    pub fn get_type(&self) -> i32 {
        match self {
            UResourceBundle::Icu(b) => b.get_type(),
            UResourceBundle::Wrapper(_) => Self::NONE,
        }
    }

    /// `getVersion()`: `null` (no subclass overrides it).
    pub fn get_version(&self) -> Option<VersionInfo> {
        None
    }

    /// `getIterator()`.
    pub fn get_iterator(&self) -> UResourceBundleIterator {
        UResourceBundleIterator::new(self.clone())
    }

    /// `getKey()`: the key of this resource, `None` for a top-level bundle.
    pub fn get_key(&self) -> Option<String> {
        match self {
            UResourceBundle::Icu(b) => b.get_key().map(str::to_string),
            UResourceBundle::Wrapper(_) => None,
        }
    }

    /// `handleGet(String aKey, HashMap<String, String> aliasesVisited,
    /// UResourceBundle requested)` (protected).
    ///
    /// # Errors
    ///
    /// The errors of alias resolution and of an array's index parse.
    pub fn handle_get(
        &self,
        a_key: &str,
        aliases_visited: Option<&mut HashMap<String, String>>,
        requested: &UResourceBundle,
    ) -> Result<Option<UResourceBundle>> {
        match self {
            UResourceBundle::Icu(b) => b.handle_get(a_key, aliases_visited, requested),
            UResourceBundle::Wrapper(_) => Ok(None),
        }
    }

    /// `handleGet(int index, HashMap<String, String> aliasesVisited,
    /// UResourceBundle requested)` (protected).
    ///
    /// # Errors
    ///
    /// The errors of alias resolution.
    pub fn handle_get_at(
        &self,
        index: i32,
        aliases_visited: Option<&mut HashMap<String, String>>,
        requested: &UResourceBundle,
    ) -> Result<Option<UResourceBundle>> {
        match self {
            UResourceBundle::Icu(b) => b.handle_get_at(index, aliases_visited, requested),
            UResourceBundle::Wrapper(_) => Ok(None),
        }
    }

    /// `handleGetStringArray()` (protected): `None` except for an array.
    ///
    /// # Errors
    ///
    /// `UResourceTypeMismatchException` for an array of non-strings.
    pub fn handle_get_string_array(&self) -> Result<Option<Vec<Vec<u16>>>> {
        match self {
            UResourceBundle::Icu(b) => b.handle_get_string_array(),
            UResourceBundle::Wrapper(_) => Ok(None),
        }
    }

    /// `handleGetKeys()` (protected): `null` in the base class; the keys of
    /// this resource for an ICU bundle.
    pub fn handle_get_keys(&self) -> Option<Vec<String>> {
        match self {
            UResourceBundle::Icu(b) => Some(b.handle_key_set()),
            UResourceBundle::Wrapper(_) => None,
        }
    }

    /// `handleGetObject(String aKey)` (protected).
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the key is found nowhere.
    pub fn handle_get_object(&self, a_key: &str) -> Result<ResourceObject> {
        match self {
            UResourceBundle::Icu(b) if b.is_table() => b.table_handle_get_object(a_key, self),
            UResourceBundle::Wrapper(w) => w.handle_get_object(a_key),
            _ => self.handle_get_object_impl(a_key, self),
        }
    }

    /// The `UResourceBundle.handleGetObject` body.
    pub(crate) fn base_handle_get_object(&self, a_key: &str) -> Result<ResourceObject> {
        self.handle_get_object_impl(a_key, self)
    }

    /// `handleGetObjectImpl(String aKey, UResourceBundle requested)`
    /// (private): to facilitate XPath style aliases the requested bundle is
    /// passed down the lookup.
    fn handle_get_object_impl(&self, a_key: &str, requested: &UResourceBundle) -> Result<ResourceObject> {
        let mut obj = self.resolve_object(a_key, requested)?;
        if obj.is_none() {
            if let Some(parent_bundle) = self.get_parent() {
                obj = Some(parent_bundle.handle_get_object_impl(a_key, requested)?);
            }
            if obj.is_none() {
                return Err(missing_resource(
                    format!(
                        "Can't find resource for bundle {}, key {a_key}",
                        self.java_class_name()
                    ),
                    self.java_class_name(),
                    a_key,
                ));
            }
        }
        Ok(obj.expect("checked above"))
    }

    /// `resolveObject(String aKey, UResourceBundle requested)` (private):
    /// a string, a string array or the bundle, `None` when not found.
    fn resolve_object(&self, a_key: &str, requested: &UResourceBundle) -> Result<Option<ResourceObject>> {
        if self.get_type() == Self::STRING {
            return Ok(Some(ResourceObject::String(self.get_string()?)));
        }
        let obj = self.handle_get(a_key, None, requested)?;
        if let Some(obj) = obj {
            if obj.get_type() == Self::STRING {
                return Ok(Some(ResourceObject::String(obj.get_string()?)));
            }
            if obj.get_type() == Self::ARRAY {
                match obj.handle_get_string_array() {
                    Ok(Some(a)) => return Ok(Some(ResourceObject::StringArray(a))),
                    Ok(None) => return Ok(None),
                    Err(ex) if is_type_mismatch(&ex) => return Ok(Some(ResourceObject::Bundle(obj))),
                    Err(ex) => return Err(ex),
                }
            }
            return Ok(Some(ResourceObject::Bundle(obj)));
        }
        Ok(None)
    }

    /// `ResourceBundle.getObject(String key)` (final):
    /// `handleGetObject`, then the parent chain.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the key is found nowhere.
    pub fn get_object(&self, key: &str) -> Result<ResourceObject> {
        // handleGetObject of every UResourceBundle throws instead of
        // returning null, so ResourceBundle's parent loop never runs.
        self.handle_get_object(key)
    }

    /// `ResourceBundle.getString(String key)` (final): `(String)
    /// getObject(key)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException`; `ClassCastException` when the object is
    /// not a string.
    pub fn get_string_by_key(&self, key: &str) -> Result<Vec<u16>> {
        match self.get_object(key)? {
            ResourceObject::String(s) => Ok(s),
            other => Err(class_cast(&other, "java.lang.String")),
        }
    }

    /// `ResourceBundle.getStringArray(String key)` (final): `(String[])
    /// getObject(key)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException`; `ClassCastException` when the object is
    /// not a string array.
    pub fn get_string_array_by_key(&self, key: &str) -> Result<Vec<Vec<u16>>> {
        match self.get_object(key)? {
            ResourceObject::StringArray(s) => Ok(s),
            other => Err(class_cast(&other, "[Ljava.lang.String;")),
        }
    }

    /// `isTopLevelResource()` (protected, deprecated).
    pub fn is_top_level_resource(&self) -> bool {
        match self {
            UResourceBundle::Icu(b) => b.is_top_level_resource(),
            UResourceBundle::Wrapper(_) => true,
        }
    }

    /// The ICU bundle, `(ICUResourceBundle) this`.
    ///
    /// # Errors
    ///
    /// `ClassCastException` for a wrapper.
    pub fn as_icu(&self) -> Result<&Arc<ICUResourceBundle>> {
        match self {
            UResourceBundle::Icu(b) => Ok(b),
            UResourceBundle::Wrapper(_) => Err(LuceneError::unchecked(
                "java.lang.ClassCastException",
                "class com.ibm.icu.impl.ResourceBundleWrapper cannot be cast to class com.ibm.icu.impl.ICUResourceBundle",
            )),
        }
    }
}

/// The `ClassCastException` of a failed cast of a `getObject` result.
fn class_cast(obj: &ResourceObject, target: &str) -> LuceneError {
    let from = match obj {
        ResourceObject::String(_) => "java.lang.String".to_string(),
        ResourceObject::StringArray(_) => "[Ljava.lang.String;".to_string(),
        ResourceObject::Bundle(b) => b.java_class_name().to_string(),
    };
    LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!("class {from} cannot be cast to class {target}"),
    )
}
