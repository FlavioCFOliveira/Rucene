// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.KeyTypeData`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, Result};

use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{self, ICUResourceBundle, OpenType};
use crate::r#impl::locale::ascii_util::AsciiUtil;
use crate::util::output::Output;
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// `ValueType`: the type of value a key takes.
///
/// Equivalent to the enum `KeyTypeData.ValueType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// `single`: only one value.
    Single,
    /// `multiple`: one or more values.
    Multiple,
    /// `incremental`: increment over values.
    Incremental,
    /// `any`: any combination.
    Any,
}

impl ValueType {
    /// `ValueType.valueOf(String)`.
    fn value_of(s: &str) -> Result<ValueType> {
        match s {
            "single" => Ok(ValueType::Single),
            "multiple" => Ok(ValueType::Multiple),
            "incremental" => Ok(ValueType::Incremental),
            "any" => Ok(ValueType::Any),
            _ => Err(LuceneError::illegal_argument(format!(
                "No enum constant com.ibm.icu.impl.locale.KeyTypeData.ValueType.{s}"
            ))),
        }
    }
}

/// `SpecialType` and its `SpecialTypeHandler`s.
///
/// Each private `SpecialTypeHandler` subclass (`KeyTypeData.java:43-89`) is
/// the handler of one variant: its `pat` is the variant's pattern in
/// [`SpecialType::is_well_formed`], and none overrides the inherited
/// `canonicalize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SpecialType {
    Codepoints,
    ReorderCode,
    RgKeyValue,
    ScriptCode,
    SubdivisionCode,
    PrivateUse,
}

impl SpecialType {
    fn value_of(s: &str) -> Result<SpecialType> {
        match s {
            "CODEPOINTS" => Ok(SpecialType::Codepoints),
            "REORDER_CODE" => Ok(SpecialType::ReorderCode),
            "RG_KEY_VALUE" => Ok(SpecialType::RgKeyValue),
            "SCRIPT_CODE" => Ok(SpecialType::ScriptCode),
            "SUBDIVISION_CODE" => Ok(SpecialType::SubdivisionCode),
            "PRIVATE_USE" => Ok(SpecialType::PrivateUse),
            _ => Err(LuceneError::illegal_argument(format!(
                "No enum constant com.ibm.icu.impl.locale.KeyTypeData.SpecialType.{s}"
            ))),
        }
    }

    /// The handler's `isWellFormed(String value)`: doesn't test validity,
    /// just whether it is well formed.
    fn is_well_formed(self, value: &str) -> bool {
        static PATS: LazyLock<[JavaPattern; 6]> = LazyLock::new(|| {
            [
                // `KeyTypeData.CodepointsTypeHandler.pat`
                "[0-9a-fA-F]{4,6}(-[0-9a-fA-F]{4,6})*",
                // `KeyTypeData.ReorderCodeTypeHandler.pat`
                "[a-zA-Z]{3,8}(-[a-zA-Z]{3,8})*",
                // `KeyTypeData.RgKeyValueTypeHandler.pat`
                "([a-zA-Z]{2}|[0-9]{3})[zZ]{4}",
                // `KeyTypeData.ScriptCodeTypeHandler.pat`
                "[a-zA-Z]{4}(-[a-zA-Z]{4})*",
                // `KeyTypeData.SubdivisionKeyValueTypeHandler.pat`
                "([a-zA-Z]{2}|[0-9]{3})",
                // `KeyTypeData.PrivateUseKeyValueTypeHandler.pat`
                "[a-zA-Z0-9]{3,8}(-[a-zA-Z0-9]{3,8})*",
            ]
            .map(|p| JavaPattern::compile(p, 0).expect("valid pattern"))
        });
        PATS[self as usize].matches(value)
    }

    /// The handler's `canonicalize(String value)`.
    fn canonicalize(self, value: &str) -> String {
        AsciiUtil::to_lower_string(value)
    }
}

/// `KeyData` (private).
#[derive(Debug)]
struct KeyData {
    legacy_id: String,
    bcp_id: String,
    type_map: HashMap<String, Arc<TypeData>>,
    /// `specialTypes`, an `EnumSet` (declaration order).
    special_types: Option<Vec<SpecialType>>,
}

/// `Type` (private).
#[derive(Debug)]
struct TypeData {
    legacy_id: String,
    bcp_id: String,
}

/// The data loaded from `keyTypeData.res`.
#[derive(Debug)]
struct Tables {
    keymap: HashMap<String, Arc<KeyData>>,
    /// `BCP47_KEYS`, a `LinkedHashMap` (insertion order).
    bcp47_keys: Vec<(String, Vec<String>)>,
    deprecated_keys: Vec<String>,
    value_types: Vec<(String, ValueType)>,
    deprecated_key_types: Vec<(String, Vec<String>)>,
}

/// The static initializer's result (`initFromResourceBundle()`), or the
/// message of its exception.
static TABLES: LazyLock<std::result::Result<Arc<Tables>, String>> = LazyLock::new(|| {
    KeyTypeData::init_from_resource_bundle()
        .map(Arc::new)
        .map_err(|e| e.to_string())
});

fn tables() -> Result<Arc<Tables>> {
    match &*TABLES {
        Ok(t) => Ok(t.clone()),
        Err(m) => Err(crate::util::u_resource_bundle::missing_resource(
            m.clone(),
            "",
            "",
        )),
    }
}

/// `LinkedHashSet.add`.
fn linked_add(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
}

/// The mappings between legacy (ICU) and BCP 47 Unicode locale extension
/// keys and types, from the `keyTypeData` bundle.
///
/// Equivalent to `com.ibm.icu.impl.locale.KeyTypeData` (Java initializes
/// from the resource bundle; `initFromTables` and its empty `KEY_DATA` are
/// unused proof-of-concept code, kept as the no-op they are).
pub struct KeyTypeData;

impl KeyTypeData {
    /// `toBcpKey(String key)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn to_bcp_key(key: &str) -> Result<Option<String>> {
        let key = AsciiUtil::to_lower_string(key);
        Ok(tables()?.keymap.get(&key).map(|k| k.bcp_id.clone()))
    }

    /// `toLegacyKey(String key)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn to_legacy_key(key: &str) -> Result<Option<String>> {
        let key = AsciiUtil::to_lower_string(key);
        Ok(tables()?.keymap.get(&key).map(|k| k.legacy_id.clone()))
    }

    fn to_type(
        key: &str,
        ty: &str,
        is_known_key: Option<&mut Output<bool>>,
        is_special_type: Option<&mut Output<bool>>,
        bcp: bool,
    ) -> Result<Option<String>> {
        let mut is_known_key = is_known_key;
        let mut is_special_type = is_special_type;
        if let Some(k) = is_known_key.as_deref_mut() {
            k.value = Some(false);
        }
        if let Some(s) = is_special_type.as_deref_mut() {
            s.value = Some(false);
        }
        let key = AsciiUtil::to_lower_string(key);
        let ty = AsciiUtil::to_lower_string(ty);
        let tables = tables()?;
        if let Some(key_data) = tables.keymap.get(&key) {
            if let Some(k) = is_known_key {
                k.value = Some(true);
            }
            if let Some(t) = key_data.type_map.get(&ty) {
                return Ok(Some(if bcp {
                    t.bcp_id.clone()
                } else {
                    t.legacy_id.clone()
                }));
            }
            if let Some(special_types) = &key_data.special_types {
                for st in special_types {
                    if st.is_well_formed(&ty) {
                        if let Some(s) = is_special_type.as_deref_mut() {
                            s.value = Some(true);
                        }
                        return Ok(Some(st.canonicalize(&ty)));
                    }
                }
            }
        }
        Ok(None)
    }

    /// `toBcpType(String key, String type, Output<Boolean> isKnownKey,
    /// Output<Boolean> isSpecialType)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn to_bcp_type(
        key: &str,
        ty: &str,
        is_known_key: Option<&mut Output<bool>>,
        is_special_type: Option<&mut Output<bool>>,
    ) -> Result<Option<String>> {
        Self::to_type(key, ty, is_known_key, is_special_type, true)
    }

    /// `toLegacyType(String key, String type, Output<Boolean> isKnownKey,
    /// Output<Boolean> isSpecialType)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn to_legacy_type(
        key: &str,
        ty: &str,
        is_known_key: Option<&mut Output<bool>>,
        is_special_type: Option<&mut Output<bool>>,
    ) -> Result<Option<String>> {
        Self::to_type(key, ty, is_known_key, is_special_type, false)
    }

    /// `try { b.get(key) } catch (MissingResourceException e) { null }`.
    fn get_optional(b: &UResourceBundle, key: &str) -> Result<Option<UResourceBundle>> {
        match b.get(key) {
            Ok(r) => Ok(Some(r)),
            Err(e) if is_missing_resource(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn init_from_resource_bundle() -> Result<Tables> {
        let key_type_data_res = UResourceBundle::Icu(ICUResourceBundle::get_bundle_instance_open(
            Some(ICUData::ICU_BASE_NAME),
            "keyTypeData",
            icu_resource_bundle::icu_data_class_loader(),
            OpenType::Direct,
        )?);

        let (deprecated_keys, value_types) =
            Self::get_key_info(&key_type_data_res.get("keyInfo")?)?;
        let deprecated_key_types = Self::get_type_info(&key_type_data_res.get("typeInfo")?)?;

        let key_map_res = key_type_data_res.get("keyMap")?;
        let type_map_res = key_type_data_res.get("typeMap")?;

        // alias data is optional
        let type_alias_res = Self::get_optional(&key_type_data_res, "typeAlias")?;
        let bcp_type_alias_res = Self::get_optional(&key_type_data_res, "bcpTypeAlias")?;

        let s = |v: Vec<u16>| String::from_utf16_lossy(&v);
        let mut keymap: HashMap<String, Arc<KeyData>> = HashMap::new();

        // iterate through keyMap resource
        let mut key_map_itr = key_map_res.get_iterator();
        let mut bcp47_keys: Vec<(String, Vec<String>)> = Vec::new();

        while key_map_itr.has_next() {
            let key_map_entry = key_map_itr.next()?;
            let legacy_key_id = key_map_entry.get_key().unwrap_or_else(|| "null".into());
            let mut bcp_key_id = s(key_map_entry.get_string()?);

            let mut has_same_key = false;
            if bcp_key_id.is_empty() {
                // Empty value indicates that BCP key is same with the legacy key.
                bcp_key_id = legacy_key_id.clone();
                has_same_key = true;
            }
            let mut bcp47_types: Vec<String> = Vec::new();

            let is_tz = legacy_key_id == "timezone";

            // reverse type alias map
            let mut type_alias_map: Option<HashMap<String, HashSet<String>>> = None;
            if let Some(type_alias_res) = &type_alias_res {
                if let Some(type_alias_res_by_key) =
                    Self::get_optional(type_alias_res, &legacy_key_id)?
                {
                    let mut m: HashMap<String, HashSet<String>> = HashMap::new();
                    let mut it = type_alias_res_by_key.get_iterator();
                    while it.has_next() {
                        let e = it.next()?;
                        let mut from = e.get_key().unwrap_or_else(|| "null".into());
                        let to = s(e.get_string()?);
                        if is_tz {
                            from = from.replace(':', "/");
                        }
                        m.entry(to).or_default().insert(from);
                    }
                    type_alias_map = Some(m);
                }
            }

            // reverse bcp type alias map
            let mut bcp_type_alias_map: Option<HashMap<String, HashSet<String>>> = None;
            if let Some(bcp_type_alias_res) = &bcp_type_alias_res {
                if let Some(by_key) = Self::get_optional(bcp_type_alias_res, &bcp_key_id)? {
                    let mut m: HashMap<String, HashSet<String>> = HashMap::new();
                    let mut it = by_key.get_iterator();
                    while it.has_next() {
                        let e = it.next()?;
                        let from = e.get_key().unwrap_or_else(|| "null".into());
                        let to = s(e.get_string()?);
                        m.entry(to).or_default().insert(from);
                    }
                    bcp_type_alias_map = Some(m);
                }
            }

            let mut type_data_map: HashMap<String, Arc<TypeData>> = HashMap::new();
            let mut special_type_set: Option<Vec<SpecialType>> = None;

            // look up type map for the key, and walk through the mapping data
            let type_map_res_by_key = match type_map_res.get(&legacy_key_id) {
                Ok(r) => Some(r),
                Err(e) if is_missing_resource(&e) => {
                    // type map for each key must exist
                    debug_assert!(false, "type map for each key must exist");
                    None
                }
                Err(e) => return Err(e),
            };
            if let Some(type_map_res_by_key) = type_map_res_by_key {
                let mut it = type_map_res_by_key.get_iterator();
                while it.has_next() {
                    let type_map_entry = it.next()?;
                    let mut legacy_type_id =
                        type_map_entry.get_key().unwrap_or_else(|| "null".into());
                    let mut bcp_type_id = s(type_map_entry.get_string()?);

                    // special types
                    let first = crate::r#impl::locale::ascii_util::char_at16(&legacy_type_id, 0);
                    let is_special_type =
                        (b'9' as u16) < first && first < (b'a' as u16) && bcp_type_id.is_empty();
                    if is_special_type {
                        let st = SpecialType::value_of(&legacy_type_id)?;
                        let set = special_type_set.get_or_insert_with(Vec::new);
                        if !set.contains(&st) {
                            set.push(st);
                            set.sort();
                        }
                        linked_add(&mut bcp47_types, legacy_type_id);
                        continue;
                    }

                    if is_tz {
                        // a timezone key uses a colon instead of a slash in the resource.
                        // e.g. America:Los_Angeles
                        legacy_type_id = legacy_type_id.replace(':', "/");
                    }

                    let mut has_same_type = false;
                    if bcp_type_id.is_empty() {
                        // Empty value indicates that BCP type is same with the legacy type.
                        bcp_type_id = legacy_type_id.clone();
                        has_same_type = true;
                    }
                    linked_add(&mut bcp47_types, bcp_type_id.clone());

                    // Note: legacy type value should never be
                    // equivalent to bcp type value of a different
                    // type under the same key. So we use a single
                    // map for lookup.
                    let t = Arc::new(TypeData {
                        legacy_id: legacy_type_id.clone(),
                        bcp_id: bcp_type_id.clone(),
                    });
                    type_data_map.insert(AsciiUtil::to_lower_string(&legacy_type_id), t.clone());
                    if !has_same_type {
                        type_data_map.insert(AsciiUtil::to_lower_string(&bcp_type_id), t.clone());
                    }

                    // Also put aliases in the map
                    if let Some(m) = &type_alias_map {
                        if let Some(set) = m.get(&legacy_type_id) {
                            for alias in set {
                                type_data_map.insert(AsciiUtil::to_lower_string(alias), t.clone());
                            }
                        }
                    }
                    if let Some(m) = &bcp_type_alias_map {
                        if let Some(set) = m.get(&bcp_type_id) {
                            for alias in set {
                                type_data_map.insert(AsciiUtil::to_lower_string(alias), t.clone());
                            }
                        }
                    }
                }
            }

            // LinkedHashMap.put: a repeated key keeps its first position.
            match bcp47_keys.iter_mut().find(|(k, _)| *k == bcp_key_id) {
                Some(e) => e.1 = bcp47_types,
                None => bcp47_keys.push((bcp_key_id.clone(), bcp47_types)),
            }

            let key_data = Arc::new(KeyData {
                legacy_id: legacy_key_id.clone(),
                bcp_id: bcp_key_id.clone(),
                type_map: type_data_map,
                special_types: special_type_set,
            });

            keymap.insert(AsciiUtil::to_lower_string(&legacy_key_id), key_data.clone());
            if !has_same_key {
                keymap.insert(AsciiUtil::to_lower_string(&bcp_key_id), key_data);
            }
        }

        Ok(Tables {
            keymap,
            bcp47_keys,
            deprecated_keys,
            value_types,
            deprecated_key_types,
        })
    }

    /// `getKeyInfo(UResourceBundle keyInfoRes)` (private): the deprecated
    /// keys and the value types.
    #[allow(clippy::type_complexity)] // type mirrors the Java signature
    fn get_key_info(
        key_info_res: &UResourceBundle,
    ) -> Result<(Vec<String>, Vec<(String, ValueType)>)> {
        let mut deprecated_keys: Vec<String> = Vec::new();
        let mut value_types: Vec<(String, ValueType)> = Vec::new();
        let mut it = key_info_res.get_iterator();
        while it.has_next() {
            let key_info_entry = it.next()?;
            let key = key_info_entry.get_key().unwrap_or_else(|| "null".into());
            // `KeyTypeData.KeyInfoType.valueOf(key)`: the private enum
            // `KeyInfoType {deprecated, valueType}` is the two keys matched here.
            let is_deprecated = match key.as_str() {
                "deprecated" => true,
                "valueType" => false,
                _ => {
                    return Err(LuceneError::illegal_argument(format!(
                        "No enum constant com.ibm.icu.impl.locale.KeyTypeData.KeyInfoType.{key}"
                    )))
                }
            };
            let mut it2 = key_info_entry.get_iterator();
            while it2.has_next() {
                let key_info_entry2 = it2.next()?;
                let key2 = key_info_entry2.get_key().unwrap_or_else(|| "null".into());
                let value2 = String::from_utf16_lossy(&key_info_entry2.get_string()?);
                if is_deprecated {
                    linked_add(&mut deprecated_keys, key2);
                } else {
                    let vt = ValueType::value_of(&value2)?;
                    match value_types.iter_mut().find(|(k, _)| *k == key2) {
                        Some(e) => e.1 = vt,
                        None => value_types.push((key2, vt)),
                    }
                }
            }
        }
        Ok((deprecated_keys, value_types))
    }

    /// `getTypeInfo(UResourceBundle typeInfoRes)` (private): the deprecated
    /// types by key.
    fn get_type_info(type_info_res: &UResourceBundle) -> Result<Vec<(String, Vec<String>)>> {
        let mut deprecated_key_types: Vec<(String, Vec<String>)> = Vec::new();
        let mut it = type_info_res.get_iterator();
        while it.has_next() {
            let key_info_entry = it.next()?;
            let key = key_info_entry.get_key().unwrap_or_else(|| "null".into());
            // `KeyTypeData.TypeInfoType.valueOf(key)`: the private enum
            // `TypeInfoType {deprecated}` is the one key matched here.
            if key != "deprecated" {
                return Err(LuceneError::illegal_argument(format!(
                    "No enum constant com.ibm.icu.impl.locale.KeyTypeData.TypeInfoType.{key}"
                )));
            }
            let mut it2 = key_info_entry.get_iterator();
            while it2.has_next() {
                let key_info_entry2 = it2.next()?;
                let key2 = key_info_entry2.get_key().unwrap_or_else(|| "null".into());
                let mut deprecated_types: Vec<String> = Vec::new();
                let mut it3 = key_info_entry2.get_iterator();
                while it3.has_next() {
                    let key_info_entry3 = it3.next()?;
                    let key3 = key_info_entry3.get_key().unwrap_or_else(|| "null".into());
                    // allow for expansion
                    linked_add(&mut deprecated_types, key3);
                }
                match deprecated_key_types.iter_mut().find(|(k, _)| *k == key2) {
                    Some(e) => e.1 = deprecated_types,
                    None => deprecated_key_types.push((key2, deprecated_types)),
                }
            }
        }
        Ok(deprecated_key_types)
    }

    /// `getBcp47Keys()`: in the data's order.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn get_bcp47_keys() -> Result<Vec<String>> {
        Ok(tables()?
            .bcp47_keys
            .iter()
            .map(|(k, _)| k.clone())
            .collect())
    }

    /// `getBcp47KeyTypes(String key)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn get_bcp47_key_types(key: &str) -> Result<Option<Vec<String>>> {
        Ok(tables()?
            .bcp47_keys
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone()))
    }

    /// `isDeprecated(String key)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn is_deprecated(key: &str) -> Result<bool> {
        Ok(tables()?.deprecated_keys.iter().any(|k| k == key))
    }

    /// `isDeprecated(String key, String type)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn is_deprecated_type(key: &str, ty: &str) -> Result<bool> {
        Ok(tables()?
            .deprecated_key_types
            .iter()
            .find(|(k, _)| k == key)
            .is_some_and(|(_, types)| types.iter().any(|t| t == ty)))
    }

    /// `getValueType(String key)`: `single` when unknown.
    ///
    /// # Errors
    ///
    /// The errors of loading the data.
    pub fn get_value_type(key: &str) -> Result<ValueType> {
        Ok(tables()?
            .value_types
            .iter()
            .find(|(k, _)| k == key)
            .map_or(ValueType::Single, |(_, v)| *v))
    }
}
