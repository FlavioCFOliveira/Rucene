// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.LocaleExtensions`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use crate::r#impl::locale::ascii_util::{string_hash, AsciiUtil};
use crate::r#impl::locale::extension::Extension;
use crate::r#impl::locale::internal_locale_builder::{
    CaseInsensitiveChar, CaseInsensitiveString, InternalLocaleBuilder,
};
use crate::r#impl::locale::language_tag::LanguageTag;
use crate::r#impl::locale::unicode_locale_extension::{self, UnicodeLocaleExtension};

/// The extensions of a locale, by lower-case singleton, with their
/// canonical identifier.
///
/// Equivalent to `com.ibm.icu.impl.locale.LocaleExtensions`. The keys are
/// validated ASCII singletons, so the `char` order of the map is Java's
/// `Character` order.
#[derive(Debug, Clone)]
pub struct LocaleExtensions {
    /// `_map`.
    map: BTreeMap<char, Extension>,
    /// `_id`.
    id: String,
}

/// `EMPTY_EXTENSIONS`.
pub static EMPTY_EXTENSIONS: LazyLock<Arc<LocaleExtensions>> = LazyLock::new(|| {
    Arc::new(LocaleExtensions {
        map: BTreeMap::new(),
        id: String::new(),
    })
});

/// `CALENDAR_JAPANESE`: `u-ca-japanese`.
pub static CALENDAR_JAPANESE: LazyLock<Arc<LocaleExtensions>> = LazyLock::new(|| {
    let mut map = BTreeMap::new();
    map.insert(
        UnicodeLocaleExtension::SINGLETON,
        Extension::Unicode(unicode_locale_extension::CA_JAPANESE.clone()),
    );
    Arc::new(LocaleExtensions {
        map,
        id: "u-ca-japanese".to_string(),
    })
});

/// `NUMBER_THAI`: `u-nu-thai`.
pub static NUMBER_THAI: LazyLock<Arc<LocaleExtensions>> = LazyLock::new(|| {
    let mut map = BTreeMap::new();
    map.insert(
        UnicodeLocaleExtension::SINGLETON,
        Extension::Unicode(unicode_locale_extension::NU_THAI.clone()),
    );
    Arc::new(LocaleExtensions {
        map,
        id: "u-nu-thai".to_string(),
    })
});

impl LocaleExtensions {
    /// `LocaleExtensions(Map<CaseInsensitiveChar, String> extensions,
    /// Set<CaseInsensitiveString> uattributes, Map<CaseInsensitiveString,
    /// String> ukeywords)` (package-private, used by
    /// `InternalLocaleBuilder`); the collections in the builder's
    /// insertion order.
    pub(crate) fn from_builder(
        extensions: &[(CaseInsensitiveChar, String)],
        uattributes: &[CaseInsensitiveString],
        ukeywords: &[(CaseInsensitiveString, String)],
    ) -> Self {
        let has_extension = !extensions.is_empty();
        let has_u_attributes = !uattributes.is_empty();
        let has_u_keywords = !ukeywords.is_empty();

        if !has_extension && !has_u_attributes && !has_u_keywords {
            return Self {
                map: BTreeMap::new(),
                id: String::new(),
            };
        }

        // Build extension map
        let mut map = BTreeMap::new();
        if has_extension {
            for (k, v) in extensions {
                let key = AsciiUtil::to_lower(k.value());
                let mut value = v.clone();

                if LanguageTag::is_privateuse_prefix_char(key) {
                    // we need to exclude special variant in privuateuse, e.g. "x-abc-lvariant-DEF"
                    match InternalLocaleBuilder::remove_privateuse_variant(&value) {
                        Some(v) => value = v,
                        None => continue,
                    }
                }

                let e = Extension::new(key, AsciiUtil::to_lower_string(&value));
                map.insert(key, e);
            }
        }

        if has_u_attributes || has_u_keywords {
            let mut uaset: Option<BTreeSet<String>> = None;
            let mut ukmap: Option<BTreeMap<String, String>> = None;

            if has_u_attributes {
                let mut s = BTreeSet::new();
                for cis in uattributes {
                    s.insert(AsciiUtil::to_lower_string(cis.value()));
                }
                uaset = Some(s);
            }

            if has_u_keywords {
                let mut m = BTreeMap::new();
                for (k, v) in ukeywords {
                    let key = AsciiUtil::to_lower_string(k.value());
                    let ty = AsciiUtil::to_lower_string(v);
                    m.insert(key, ty);
                }
                ukmap = Some(m);
            }

            let ule = UnicodeLocaleExtension::new(uaset, ukmap);
            map.insert(
                UnicodeLocaleExtension::SINGLETON,
                Extension::Unicode(Arc::new(ule)),
            );
        }

        if map.is_empty() {
            // this could happen when only privuateuse with special variant
            Self {
                map,
                id: String::new(),
            }
        } else {
            let id = Self::to_id(&map);
            Self { map, id }
        }
    }

    /// `getKeys()`.
    pub fn get_keys(&self) -> impl Iterator<Item = char> + '_ {
        self.map.keys().copied()
    }

    /// `getExtension(Character key)`.
    pub fn get_extension(&self, key: char) -> Option<&Extension> {
        self.map.get(&AsciiUtil::to_lower(key))
    }

    /// `getExtensionValue(Character key)`.
    pub fn get_extension_value(&self, key: char) -> Option<&str> {
        self.map
            .get(&AsciiUtil::to_lower(key))
            .map(|e| e.get_value())
    }

    /// `getUnicodeLocaleAttributes()`.
    pub fn get_unicode_locale_attributes(&self) -> BTreeSet<String> {
        match self.map.get(&UnicodeLocaleExtension::SINGLETON) {
            None => BTreeSet::new(),
            Some(ext) => {
                let u = ext.as_unicode();
                debug_assert!(u.is_some());
                u.map(|u| u.get_unicode_locale_attributes().clone())
                    .unwrap_or_default()
            }
        }
    }

    /// `getUnicodeLocaleKeys()`.
    pub fn get_unicode_locale_keys(&self) -> Vec<String> {
        match self.map.get(&UnicodeLocaleExtension::SINGLETON) {
            None => Vec::new(),
            Some(ext) => {
                let u = ext.as_unicode();
                debug_assert!(u.is_some());
                u.map(|u| u.get_unicode_locale_keys().cloned().collect())
                    .unwrap_or_default()
            }
        }
    }

    /// `getUnicodeLocaleType(String unicodeLocaleKey)`.
    pub fn get_unicode_locale_type(&self, unicode_locale_key: &str) -> Option<String> {
        let ext = self.map.get(&UnicodeLocaleExtension::SINGLETON)?;
        let u = ext.as_unicode();
        debug_assert!(u.is_some());
        u.and_then(|u| {
            u.get_unicode_locale_type(&AsciiUtil::to_lower_string(unicode_locale_key))
                .map(str::to_string)
        })
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `isValidKey(char c)`.
    pub fn is_valid_key(c: char) -> bool {
        LanguageTag::is_extension_singleton_char(c) || LanguageTag::is_privateuse_prefix_char(c)
    }

    /// `isValidUnicodeLocaleKey(String ukey)`.
    pub fn is_valid_unicode_locale_key(ukey: &str) -> bool {
        UnicodeLocaleExtension::is_key(ukey)
    }

    fn to_id(map: &BTreeMap<char, Extension>) -> String {
        let mut buf = String::new();
        let mut privuse: Option<&Extension> = None;
        for (singleton, extension) in map {
            if LanguageTag::is_privateuse_prefix_char(*singleton) {
                privuse = Some(extension);
            } else {
                if !buf.is_empty() {
                    buf.push_str(LanguageTag::SEP);
                }
                buf.push_str(&extension.to_string());
            }
        }
        if let Some(privuse) = privuse {
            if !buf.is_empty() {
                buf.push_str(LanguageTag::SEP);
            }
            buf.push_str(&privuse.to_string());
        }
        buf
    }

    /// `getID()`.
    pub fn get_id(&self) -> &str {
        &self.id
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        string_hash(&self.id)
    }
}

/// `toString()`: the identifier.
impl fmt::Display for LocaleExtensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

/// `equals(Object)`: equal identifiers.
impl PartialEq for LocaleExtensions {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for LocaleExtensions {}

impl Hash for LocaleExtensions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
