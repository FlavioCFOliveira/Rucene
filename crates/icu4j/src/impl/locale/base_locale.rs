// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.BaseLocale`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use crate::r#impl::locale::ascii_util::AsciiUtil;
use crate::r#impl::locale::locale_object_cache::LocaleObjectCache;

/// The language, script, region and variant of a locale, canonicalized
/// (lower-case language, title-case script, upper-case region and
/// variant) and cached.
///
/// Equivalent to `com.ibm.icu.impl.locale.BaseLocale` (`JDKIMPL` is
/// `false`). Instances are shared (`Arc`); Java's interned field strings
/// compare by value here.
#[derive(Debug, Clone)]
pub struct BaseLocale {
    language: String,
    script: String,
    region: String,
    variant: String,
    hash: i32,
}

/// `CACHE`: an instance of the private class `BaseLocale.Cache`
/// (`BaseLocale.java:251-266`), a `LocaleObjectCache` whose two overrides
/// are the closures given to [`LocaleObjectCache::new`]:
/// `createObject(Key key)` (`new BaseLocale(key._lang, key._scrt,
/// key._regn, key._vart)`) and `normalizeKey(Key key)` (`Key.normalize(key)`).
static CACHE: LazyLock<LocaleObjectCache<Key, Arc<BaseLocale>>> = LazyLock::new(|| {
    LocaleObjectCache::new(
        |key: &Key| {
            Some(Arc::new(BaseLocale::new(
                &key.lang, &key.scrt, &key.regn, &key.vart,
            )))
        },
        |key: Key| Some(Key::normalize(&key)),
    )
});

/// `ROOT`.
pub static ROOT: LazyLock<Arc<BaseLocale>> =
    LazyLock::new(|| BaseLocale::get_instance("", "", "", ""));

impl BaseLocale {
    /// `SEP`.
    pub const SEP: &'static str = "_";

    fn new(language: &str, script: &str, region: &str, variant: &str) -> Self {
        let language = AsciiUtil::to_lower_string(language);
        let script = AsciiUtil::to_title_string(script);
        let region = AsciiUtil::to_upper_string(region);
        let variant = AsciiUtil::to_upper_string(variant);
        let mut h: i32 = 0;
        // Generating a hash value from language, script, region and variant
        for s in [&language, &script, &region, &variant] {
            for c in s.encode_utf16() {
                h = h.wrapping_mul(31).wrapping_add(c as i32);
            }
        }
        Self {
            language,
            script,
            region,
            variant,
            hash: h,
        }
    }

    /// `getInstance(String language, String script, String region, String
    /// variant)`; Java's `null` arguments are empty strings.
    pub fn get_instance(language: &str, script: &str, region: &str, variant: &str) -> Arc<Self> {
        let key = Key::new(language, script, region, variant);
        CACHE
            .get(key)
            .expect("the BaseLocale cache creates an object for every key")
    }

    /// `getLanguage()`.
    pub fn get_language(&self) -> &str {
        &self.language
    }

    /// `getScript()`.
    pub fn get_script(&self) -> &str {
        &self.script
    }

    /// `getRegion()`.
    pub fn get_region(&self) -> &str {
        &self.region
    }

    /// `getVariant()`.
    pub fn get_variant(&self) -> &str {
        &self.variant
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        self.hash
    }
}

/// `equals(Object)`.
impl PartialEq for BaseLocale {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.language == other.language
            && self.script == other.script
            && self.region == other.region
            && self.variant == other.variant
    }
}

impl Eq for BaseLocale {}

impl Hash for BaseLocale {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

/// `toString()`: `language=..., script=..., region=..., variant=...` with
/// the empty fields left out.
impl fmt::Display for BaseLocale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = String::new();
        for (name, value) in [
            ("language=", &self.language),
            ("script=", &self.script),
            ("region=", &self.region),
            ("variant=", &self.variant),
        ] {
            if !value.is_empty() {
                if !buf.is_empty() {
                    buf.push_str(", ");
                }
                buf.push_str(name);
                buf.push_str(value);
            }
        }
        f.write_str(&buf)
    }
}

/// `BaseLocale.Key` (private): the four fields compared ignoring ASCII
/// case.
#[derive(Debug, Clone)]
struct Key {
    lang: String,
    scrt: String,
    regn: String,
    vart: String,
    hash: i32,
}

impl Key {
    fn new(language: &str, script: &str, region: &str, variant: &str) -> Self {
        let mut h: i32 = 0;
        // Generating a hash value from language, script, region and variant
        for s in [language, script, region, variant] {
            for c in s.encode_utf16() {
                h = h
                    .wrapping_mul(31)
                    .wrapping_add(AsciiUtil::to_lower16(c) as i32);
            }
        }
        Self {
            lang: language.to_string(),
            scrt: script.to_string(),
            regn: region.to_string(),
            vart: variant.to_string(),
            hash: h,
        }
    }

    /// `compareTo(Key other)`.
    #[allow(dead_code)]
    fn compare_to(&self, other: &Key) -> i32 {
        let mut res = AsciiUtil::case_ignore_compare(&self.lang, &other.lang);
        if res == 0 {
            res = AsciiUtil::case_ignore_compare(&self.scrt, &other.scrt);
            if res == 0 {
                res = AsciiUtil::case_ignore_compare(&self.regn, &other.regn);
                if res == 0 {
                    res = AsciiUtil::case_ignore_compare(&self.vart, &other.vart);
                }
            }
        }
        res
    }

    /// `normalize(Key key)`.
    fn normalize(key: &Key) -> Key {
        Key::new(
            &AsciiUtil::to_lower_string(&key.lang),
            &AsciiUtil::to_title_string(&key.scrt),
            &AsciiUtil::to_upper_string(&key.regn),
            &AsciiUtil::to_upper_string(&key.vart),
        )
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        AsciiUtil::case_ignore_match(&other.lang, &self.lang)
            && AsciiUtil::case_ignore_match(&other.scrt, &self.scrt)
            && AsciiUtil::case_ignore_match(&other.regn, &self.regn)
            && AsciiUtil::case_ignore_match(&other.vart, &self.vart)
    }
}

impl Eq for Key {}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}
