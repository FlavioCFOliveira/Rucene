// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.ULocale`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::{Category as JdkCategory, Locale};

use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{self, ICUResourceBundle};
use crate::r#impl::icu_resource_table_access::ICUResourceTableAccess;
use crate::r#impl::locale::ascii_util::{
    len16, string_compare, string_hash, substring16_from, to_lower_case_default_locale,
    to_upper_case_default_locale, AsciiUtil,
};
use crate::r#impl::locale::base_locale::{self, BaseLocale};
use crate::r#impl::locale::internal_locale_builder::InternalLocaleBuilder;
use crate::r#impl::locale::key_type_data::KeyTypeData;
use crate::r#impl::locale::language_tag::LanguageTag;
use crate::r#impl::locale::likely_subtags::LikelySubtags;
use crate::r#impl::locale::locale_extensions::{self, LocaleExtensions};
use crate::r#impl::locale::locale_syntax_exception::LocaleSyntaxException;
use crate::r#impl::locale::parse_status::ParseStatus;
use crate::r#impl::locale::unicode_locale_extension::UnicodeLocaleExtension;
use crate::r#impl::locale_id_parser::{KeywordMap, LocaleIDParser};
use crate::r#impl::locale_ids::LocaleIDs;
use crate::r#impl::soft_cache::SoftCache;
use crate::text::locale_display_names::{self, DialectHandling};
use crate::util::illformed_locale_exception::IllformedLocaleException;
use crate::util::locale_matcher::LocaleMatcher;
use crate::util::locale_priority_list::LocalePriorityList;
use crate::util::u_resource_bundle::UResourceBundle;

/// `nameCache`: normalized names by locale ID.
static NAME_CACHE: LazyLock<SoftCache<String, String, ()>> = LazyLock::new(|| {
    SoftCache::new(|tmp_locale_id: &String, _unused: ()| Ok(Some(LocaleIDParser::new(tmp_locale_id).get_name()?)))
});

/// Types for `ULocale.getAvailableLocalesByType`.
///
/// Equivalent to the enum `ULocale.AvailableType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvailableType {
    /// `DEFAULT`: locales that return data when passed to ICU APIs, but
    /// not including legacy or alias locales.
    Default,
    /// `ONLY_LEGACY_ALIASES`: legacy or alias locales that return data when
    /// passed to ICU APIs.
    OnlyLegacyAliases,
    /// `WITH_LEGACY_ALIASES`: the union of the two.
    WithLegacyAliases,
}

/// Enum for locale categories, used to set default locales.
///
/// Equivalent to the enum `ULocale.Category`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// `DISPLAY`: category used to represent the default locale for
    /// displaying user interfaces.
    Display,
    /// `FORMAT`: category used to represent the default locale for
    /// formatting dates, numbers and/or currencies.
    Format,
}

impl Category {
    fn ordinal(self) -> usize {
        match self {
            Category::Display => 0,
            Category::Format => 1,
        }
    }

    const VALUES: [Category; 2] = [Category::Display, Category::Format];
}

/// Options for `minimizeSubtags`.
///
/// Equivalent to the enum `ULocale.Minimize` (deprecated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Minimize {
    /// `FAVOR_SCRIPT`.
    FavorScript,
    /// `FAVOR_REGION`.
    FavorRegion,
}

/// Opaque selector enum for `getLocale()`.
///
/// Equivalent to the class `ULocale.Type`; `ACTUAL_LOCALE` and
/// `VALID_LOCALE` are its two instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    /// `ACTUAL_LOCALE`.
    ActualLocale,
    /// `VALID_LOCALE`.
    ValidLocale,
}

/// `ACTUAL_LOCALE`.
pub const ACTUAL_LOCALE: Type = Type::ActualLocale;
/// `VALID_LOCALE`.
pub const VALID_LOCALE: Type = Type::ValidLocale;

/// `EMPTY_STRING`.
const EMPTY_STRING: &str = "";
/// `UNDERSCORE`.
const UNDERSCORE: char = '_';
/// `LOCALE_ATTRIBUTE_KEY`: special keyword key for Unicode locale
/// attributes.
const LOCALE_ATTRIBUTE_KEY: &str = "attribute";
/// `UNDEFINED_LANGUAGE`.
const UNDEFINED_LANGUAGE: &str = "und";
/// `UNDEFINED_SCRIPT`.
const UNDEFINED_SCRIPT: &str = "Zzzz";
/// `UNDEFINED_REGION`.
const UNDEFINED_REGION: &str = "ZZ";

/// `CANONICALIZE_MAP`.
const CANONICALIZE_MAP: &[(&str, &str)] = &[
    ("art__LOJBAN", "jbo"),          /* registered name */
    ("cel__GAULISH", "cel__GAULISH"), /* registered name */
    ("de__1901", "de__1901"),        /* registered name */
    ("de__1906", "de__1906"),        /* registered name */
    ("en__BOONT", "en__BOONT"),      /* registered name */
    ("en__SCOUSE", "en__SCOUSE"),    /* registered name */
    ("hy__AREVELA", "hy"),           /* Registered IANA variant */
    ("hy__AREVMDA", "hyw"),          /* Registered IANA variant */
    ("sl__ROZAJ", "sl__ROZAJ"),      /* registered name */
    ("zh__GUOYU", "zh"),             /* registered name */
    ("zh__HAKKA", "hak"),            /* registered name */
    ("zh__XIANG", "hsn"),            /* registered name */
    // Three letter subtags won't be treated as variants.
    ("zh_GAN", "gan"),        /* registered name */
    ("zh_MIN", "zh__MIN"),    /* registered name */
    ("zh_MIN_NAN", "nan"),    /* registered name */
    ("zh_WUU", "wuu"),        /* registered name */
    ("zh_YUE", "yue"),        /* registered name */
];

/// `LANG_DIR_STRING`.
const LANG_DIR_STRING: &str = "root-en-es-pt-zh-ja-ko-de-fr-it-ar+he+fa+ru-nl-pl-th-tr-";

/// The shared state of a locale: its normalized ID and the lazily computed
/// JDK locale, base locale and extensions.
#[derive(Debug)]
struct Inner {
    locale_id: String,
    locale: OnceLock<Locale>,
    base_locale: OnceLock<Arc<BaseLocale>>,
    extensions: OnceLock<Arc<LocaleExtensions>>,
}

/// A class analogous to `java.util.Locale` that provides additional support
/// for ICU protocol: an immutable, normalized locale ID
/// (`language_Script_COUNTRY_VARIANT@keywords`).
///
/// Equivalent to `com.ibm.icu.util.ULocale`. Instances share their state
/// (`Arc`); Java's `transient volatile` caches of the JDK locale, the base
/// locale and the extensions are set once.
///
/// Display names (`getDisplay*`) are Rust strings, `None` for Java's
/// `null`, as the [`LocaleDisplayNames`](crate::text::locale_display_names::LocaleDisplayNames)
/// results they return.
#[derive(Clone)]
pub struct ULocale(Arc<Inner>);

impl fmt::Debug for ULocale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ULocale({:?})", self.0.locale_id)
    }
}

macro_rules! ulocale_constant {
    ($(#[$doc:meta])* $name:ident, $id:literal, $jdk:expr) => {
        $(#[$doc])*
        pub static $name: LazyLock<ULocale> = LazyLock::new(|| ULocale::from_parts($id, $jdk));
    };
}

ulocale_constant!(
    /// `ENGLISH`.
    ENGLISH, "en", Some(Locale::of("en", "", ""))
);
ulocale_constant!(
    /// `FRENCH`.
    FRENCH, "fr", Some(Locale::of("fr", "", ""))
);
ulocale_constant!(
    /// `GERMAN`.
    GERMAN, "de", Some(Locale::of("de", "", ""))
);
ulocale_constant!(
    /// `ITALIAN`.
    ITALIAN, "it", Some(Locale::of("it", "", ""))
);
ulocale_constant!(
    /// `JAPANESE`.
    JAPANESE, "ja", Some(Locale::of("ja", "", ""))
);
ulocale_constant!(
    /// `KOREAN`.
    KOREAN, "ko", Some(Locale::of("ko", "", ""))
);
ulocale_constant!(
    /// `CHINESE`.
    CHINESE, "zh", Some(Locale::of("zh", "", ""))
);
ulocale_constant!(
    /// `FRANCE`.
    FRANCE, "fr_FR", Some(Locale::of("fr", "FR", ""))
);
ulocale_constant!(
    /// `GERMANY`.
    GERMANY, "de_DE", Some(Locale::of("de", "DE", ""))
);
ulocale_constant!(
    /// `ITALY`.
    ITALY, "it_IT", Some(Locale::of("it", "IT", ""))
);
ulocale_constant!(
    /// `JAPAN`.
    JAPAN, "ja_JP", Some(Locale::of("ja", "JP", ""))
);
ulocale_constant!(
    /// `KOREA`.
    KOREA, "ko_KR", Some(Locale::of("ko", "KR", ""))
);
ulocale_constant!(
    /// `UK`.
    UK, "en_GB", Some(Locale::of("en", "GB", ""))
);
ulocale_constant!(
    /// `US`.
    US, "en_US", Some(Locale::of("en", "US", ""))
);
ulocale_constant!(
    /// `CANADA`.
    CANADA, "en_CA", Some(Locale::of("en", "CA", ""))
);
ulocale_constant!(
    /// `CANADA_FRENCH`.
    CANADA_FRENCH, "fr_CA", Some(Locale::of("fr", "CA", ""))
);
ulocale_constant!(
    /// `ROOT`: the root locale (`EMPTY_LOCALE`, `new Locale("", "")`).
    ROOT, "", Some(Locale::of("", "", ""))
);

// Special note about static initializer for
//   - SIMPLIFIED_CHINESE
//   - TRADTIONAL_CHINESE
//   - CHINA
//   - TAIWAN
//
// Equivalent JDK Locale for ULocale.SIMPLIFIED_CHINESE is different
// by JRE version. JRE 7 or later supports a script tag "Hans", while
// JRE 6 or older does not. JDK's Locale.SIMPLIFIED_CHINESE is actually
// zh_CN, not zh_Hans. This is same in Java 7 or later versions.
//
// ULocale#toLocale() implementation create a Locale with a script tag.
// When a new ULocale is constructed with the single arg
// constructor, the volatile field 'Locale locale' is initialized by
// #toLocale() method.
//
// Because we cannot hardcode corresponding JDK Locale representation below,
// SIMPLIFIED_CHINESE is constructed without JDK Locale argument, and
// #toLocale() is used for resolving the best matching JDK Locale at runtime.
//
// The same thing applies to TRADITIONAL_CHINESE.

/// `SIMPLIFIED_CHINESE`: `new ULocale("zh_Hans")`.
pub static SIMPLIFIED_CHINESE: LazyLock<ULocale> =
    LazyLock::new(|| ULocale::new("zh_Hans").expect("a constant locale ID is valid"));
/// `TRADITIONAL_CHINESE`.
pub static TRADITIONAL_CHINESE: LazyLock<ULocale> =
    LazyLock::new(|| ULocale::new("zh_Hant").expect("a constant locale ID is valid"));
/// `CHINA` (also `PRC`).
pub static CHINA: LazyLock<ULocale> =
    LazyLock::new(|| ULocale::new("zh_Hans_CN").expect("a constant locale ID is valid"));
/// `TAIWAN`.
pub static TAIWAN: LazyLock<ULocale> =
    LazyLock::new(|| ULocale::new("zh_Hant_TW").expect("a constant locale ID is valid"));

/// `CACHE`: `forLocale` results by JDK locale.
static CACHE: LazyLock<SoftCache<Locale, ULocale, ()>> =
    LazyLock::new(|| SoftCache::new(|key: &Locale, _unused: ()| Ok(Some(JDKLocaleHelper::to_u_locale(key)?))));

/// The default locales: `defaultULocale`, `defaultCategoryLocales` and
/// `defaultCategoryULocales`.
struct Defaults {
    default_u_locale: Option<ULocale>,
    category_locales: [Option<Locale>; 2],
    category_u_locales: [Option<ULocale>; 2],
}

static DEFAULTS: Mutex<Defaults> = Mutex::new(Defaults {
    default_u_locale: None,
    category_locales: [None, None],
    category_u_locales: [None, None],
});

/// The class initialization of the defaults: `0` not started, `1` running
/// (Java's `defaultULocale` is still `null`), `2` done.
static DEFAULTS_INIT: AtomicU8 = AtomicU8::new(0);

fn lock_defaults() -> std::sync::MutexGuard<'static, Defaults> {
    DEFAULTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The static initializer of the default locales.
fn ensure_defaults() {
    if DEFAULTS_INIT
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let default_locale = Locale::get_default();
    let default_u_locale = ULocale::for_locale(Some(&default_locale)).ok().flatten();
    let mut category_locales: [Option<Locale>; 2] = [None, None];
    let mut category_u_locales: [Option<ULocale>; 2] = [None, None];
    if JDKLocaleHelper::has_locale_categories() {
        for cat in Category::VALUES {
            let idx = cat.ordinal();
            let l = JDKLocaleHelper::get_default(cat);
            category_u_locales[idx] = ULocale::for_locale(Some(&l)).ok().flatten();
            category_locales[idx] = Some(l);
        }
    }
    {
        let mut d = lock_defaults();
        d.default_u_locale = default_u_locale;
        d.category_locales = category_locales;
        d.category_u_locales = category_u_locales;
    }
    DEFAULTS_INIT.store(2, Ordering::SeqCst);
}

/// `RegionValidateMap.gValidRegionMap`.
const G_VALID_REGION_MAP: [i32; 22] = [
    0xeedf597cu32 as i32, 0xdeddbdefu32 as i32, 0x15943f3f, 0x0e00d580,
    0xb0095c00u32 as i32, 0x0015fb9f, 0x781c068d, 0x0340400f,
    0xf42b1d00u32 as i32, 0xfd4f8141u32 as i32, 0x25d7fffc, 0x0100084b,
    0x538f3c40, 0x40000001, 0xfdf15100u32 as i32, 0x9fbb7ae7u32 as i32,
    0x0410419a, 0x00408557, 0x00004002, 0x00100001,
    0x00400408, 0x00000001,
];

/// The valid region codes of CLDR, as a bit map.
///
/// Equivalent to `ULocale.RegionValidateMap` (deprecated, internal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionValidateMap {
    /// `map` (protected).
    pub map: Vec<i32>,
}

/// `RegionValidateMap.BUILTIN`.
pub static BUILTIN: LazyLock<RegionValidateMap> = LazyLock::new(RegionValidateMap::new);

impl Default for RegionValidateMap {
    fn default() -> Self {
        Self::new()
    }
}

impl RegionValidateMap {
    /// `new RegionValidateMap()`.
    pub fn new() -> Self {
        Self {
            map: G_VALID_REGION_MAP.to_vec(),
        }
    }

    /// `isSet(String region)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` where Java's index computation
    /// runs past the map (a two-letter region whose default-locale
    /// lower-casing leaves ASCII).
    pub fn is_set(&self, region: &str) -> bool {
        let index = self.value(region);
        if index < 0 {
            return false;
        }
        0 != (self.map[(index / 32) as usize] & (1i32 << (index % 32)))
    }

    /// `equals(RegionValidateMap that)`.
    pub fn java_equals(&self, that: &RegionValidateMap) -> bool {
        self.map == that.map
    }

    /// `value(String region)` (protected).
    pub fn value(&self, region: &str) -> i32 {
        static PAT: LazyLock<JavaPattern> =
            LazyLock::new(|| JavaPattern::compile("[a-zA-Z][a-zA-Z]", 0).expect("valid pattern"));
        if PAT.matches(region) {
            let region = to_lower_case_default_locale(region);
            let a_value = 'a' as i32;
            let mut cps = region.chars();
            let c0 = cps.next().map_or(0, |c| c as i32);
            // codePointAt(1) indexes the second UTF-16 unit.
            let units: Vec<u16> = region.encode_utf16().collect();
            let c1 = crate::jdk::code_point_at(&units, 1);
            return (c0 - a_value) * 26 + c1 - a_value;
        }
        -1
    }
}

/// `split(String regex)` with a literal one-character separator: Java
/// removes the trailing empty strings.
fn java_split(s: &str, sep: char) -> Vec<String> {
    let mut parts: Vec<String> = s.split(sep).map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() && !s.is_empty() {
        parts.clear();
    }
    parts
}

impl ULocale {
    /// `ULocale(String localeID, Locale locale)` (private).
    fn from_parts(locale_id: &str, locale: Option<Locale>) -> Self {
        let lock = OnceLock::new();
        if let Some(l) = locale {
            let _ = lock.set(l);
        }
        ULocale(Arc::new(Inner {
            locale_id: locale_id.to_string(),
            locale: lock,
            base_locale: OnceLock::new(),
            extensions: OnceLock::new(),
        }))
    }

    /// `ROOT`.
    pub fn root() -> ULocale {
        ROOT.clone()
    }

    /// `forLocale(Locale loc)`: the `ULocale` of a JDK locale (`None` for
    /// `null`).
    ///
    /// # Errors
    ///
    /// The errors of the key-type data lookups.
    pub fn for_locale(loc: Option<&Locale>) -> Result<Option<ULocale>> {
        match loc {
            None => Ok(None),
            Some(l) => CACHE.get_instance(l.clone(), ()),
        }
    }

    /// `new ULocale(String localeID)`: the locale of a normalized form of
    /// the ID (BCP 47 tags are converted).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn new(locale_id: &str) -> Result<ULocale> {
        Ok(Self::from_parts(&Self::get_name_of(Some(locale_id))?, None))
    }

    /// `new ULocale(String a, String b)`: language and country.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new2(a: Option<&str>, b: Option<&str>) -> Result<ULocale> {
        Self::new3(a, b, None)
    }

    /// `new ULocale(String a, String b, String c)`: language, country and
    /// variant.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new3(a: Option<&str>, b: Option<&str>, c: Option<&str>) -> Result<ULocale> {
        let id = Self::lscv_to_id(a, b, c, Some(EMPTY_STRING));
        Ok(Self::from_parts(&Self::get_name_of(Some(&id))?, None))
    }

    /// `createCanonical(String nonCanonicalID)`.
    ///
    /// # Errors
    ///
    /// The errors of [`canonicalize`](Self::canonicalize).
    pub fn create_canonical(non_canonical_id: &str) -> Result<ULocale> {
        Ok(Self::from_parts(&Self::canonicalize(non_canonical_id)?, None))
    }

    /// `createCanonical(ULocale locale)`.
    ///
    /// # Errors
    ///
    /// The errors of [`canonicalize`](Self::canonicalize).
    pub fn create_canonical_of(locale: &ULocale) -> Result<ULocale> {
        Self::create_canonical(locale.get_name())
    }

    fn lscv_to_id(lang: Option<&str>, script: Option<&str>, country: Option<&str>, variant: Option<&str>) -> String {
        let mut buf = String::new();

        if let Some(l) = lang.filter(|l| !l.is_empty()) {
            buf.push_str(l);
        }
        if let Some(s) = script.filter(|s| !s.is_empty()) {
            buf.push(UNDERSCORE);
            buf.push_str(s);
        }
        if let Some(c) = country.filter(|c| !c.is_empty()) {
            buf.push(UNDERSCORE);
            buf.push_str(c);
        }
        if let Some(v) = variant.filter(|v| !v.is_empty()) {
            if country.is_none_or(str::is_empty) {
                buf.push(UNDERSCORE);
            }
            buf.push(UNDERSCORE);
            buf.push_str(v);
        }
        buf
    }

    /// `toLocale()`: the corresponding JDK locale.
    pub fn to_locale(&self) -> Locale {
        self.0.locale.get_or_init(|| JDKLocaleHelper::to_locale(self)).clone()
    }

    /// `getDefault()`: the current default `ULocale`, following changes of
    /// the JDK default locale.
    ///
    /// # Errors
    ///
    /// The errors of [`for_locale`](Self::for_locale).
    pub fn get_default() -> ULocale {
        ensure_defaults();
        // Only synchronize if we must update the default locale.
        let current = lock_defaults().default_u_locale.clone();
        let current = match current {
            // When Java's default locale has extensions (such as ja-JP-u-ca-japanese),
            // Locale -> ULocale mapping requires BCP47 keyword mapping data that is currently
            // stored in a resource bundle.
            // If this happens during the class initialization's call to .forLocale(defaultLocale),
            // then defaultULocale is still null until forLocale() returns.
            // However, UResourceBundle currently requires non-null default ULocale.
            // For now, this implementation returns ULocale.ROOT to avoid the problem.
            // TODO: Consider moving BCP47 mapping data out of resource bundle later.
            None => return ROOT.clone(),
            Some(c) => c,
        };
        let current_default = Locale::get_default();
        if current.to_locale() == current_default {
            return current;
        }
        // Java computes the next default inside `synchronized (ULocale.class)`,
        // which is re-entrant; forLocale may call getDefault, so the port
        // computes it outside the lock and stores it under the lock.
        let next = match ULocale::for_locale(Some(&current_default)) {
            Ok(Some(n)) => n,
            _ => return current,
        };
        let mut d = lock_defaults();
        if let Some(c) = &d.default_u_locale {
            if c.to_locale() == current_default {
                return c.clone();
            }
        }
        d.default_u_locale = Some(next.clone());
        next
    }

    /// `setDefault(ULocale newLocale)`: sets the JDK default locale and all
    /// category defaults.
    pub fn set_default(new_locale: &ULocale) {
        ensure_defaults();
        Locale::set_default(Some(new_locale.to_locale()));
        lock_defaults().default_u_locale = Some(new_locale.clone());
        // This method also updates all category default locales
        for cat in Category::VALUES {
            Self::set_default_category(cat, new_locale);
        }
    }

    /// `getDefault(Category category)`.
    pub fn get_default_category(category: Category) -> ULocale {
        ensure_defaults();
        let idx = category.ordinal();
        let (stored_locale, stored) = {
            let d = lock_defaults();
            (d.category_locales[idx].clone(), d.category_u_locales[idx].clone())
        };
        let Some(stored) = stored else {
            // Just in case this method is called during ULocale class
            // initialization. Unlike getDefault(), we do not have
            // cyclic dependency for category default.
            return ROOT.clone();
        };
        let current_category_default = JDKLocaleHelper::get_default(category);
        if stored_locale.as_ref() != Some(&current_category_default) {
            let next = match ULocale::for_locale(Some(&current_category_default)) {
                Ok(Some(n)) => n,
                _ => return stored,
            };
            let mut d = lock_defaults();
            d.category_locales[idx] = Some(current_category_default);
            d.category_u_locales[idx] = Some(next.clone());
            return next;
        }
        stored
    }

    /// `setDefault(Category category, ULocale newLocale)`.
    pub fn set_default_category(category: Category, new_locale: &ULocale) {
        let new_java_default = new_locale.to_locale();
        let idx = category.ordinal();
        {
            let mut d = lock_defaults();
            d.category_u_locales[idx] = Some(new_locale.clone());
            d.category_locales[idx] = Some(new_java_default.clone());
        }
        JDKLocaleHelper::set_default(category, new_java_default);
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        string_hash(&self.0.locale_id)
    }

    /// `compareTo(ULocale other)`: language, script, region, variant, then
    /// the keywords; `-1`, `0` or `1`.
    pub fn compare_to(&self, other: &ULocale) -> i32 {
        if Arc::ptr_eq(&self.0, &other.0) {
            return 0;
        }

        let mut cmp;

        // Language
        cmp = string_compare(&self.get_language(), &other.get_language());
        if cmp == 0 {
            // Script
            cmp = string_compare(&self.get_script(), &other.get_script());
            if cmp == 0 {
                // Region
                cmp = string_compare(&self.get_country(), &other.get_country());
                if cmp == 0 {
                    // Variant
                    cmp = string_compare(&self.get_variant(), &other.get_variant());
                    if cmp == 0 {
                        // Keywords
                        let this_kwd = self.get_keywords();
                        let other_kwd = other.get_keywords();
                        match (this_kwd, other_kwd) {
                            (None, o) => cmp = if o.is_none() { 0 } else { -1 },
                            (Some(_), None) => cmp = 1,
                            (Some(t), Some(o)) => {
                                // Both have keywords
                                let mut ti = t.iter();
                                let mut oi = o.iter().peekable();
                                while cmp == 0 {
                                    let Some(this_key) = ti.next() else { break };
                                    let Some(other_key) = oi.next() else {
                                        cmp = 1;
                                        break;
                                    };
                                    // Compare keyword keys
                                    cmp = string_compare(this_key, other_key);
                                    if cmp == 0 {
                                        // Compare keyword values
                                        let this_val = self.get_keyword_value(this_key);
                                        let other_val = other.get_keyword_value(other_key);
                                        cmp = match (this_val, other_val) {
                                            (None, o) => {
                                                if o.is_none() {
                                                    0
                                                } else {
                                                    -1
                                                }
                                            }
                                            (Some(_), None) => 1,
                                            (Some(t), Some(o)) => string_compare(&t, &o),
                                        };
                                    }
                                }
                                if cmp == 0 && oi.peek().is_some() {
                                    cmp = -1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Normalize the result value:
        // Note: String.compareTo() may return value other than -1, 0, 1.
        // A value other than those are OK by the definition, but we don't want
        // associate any semantics other than negative/zero/positive.
        cmp.signum()
    }

    /// `getAvailableLocales()`: the installed locales of the ICU data.
    ///
    /// # Errors
    ///
    /// The errors of reading the index bundle; `NullPointerException` when
    /// it has no list.
    pub fn get_available_locales() -> Result<Vec<ULocale>> {
        ICUResourceBundle::get_available_u_locales()?.ok_or_else(LuceneError::null_pointer)
    }

    /// `getAvailableLocalesByType(AvailableType type)`.
    ///
    /// # Errors
    ///
    /// The errors of reading the index bundle; `NullPointerException` when
    /// it has no list of the type.
    pub fn get_available_locales_by_type(ty: AvailableType) -> Result<Vec<ULocale>> {
        if ty == AvailableType::WithLegacyAliases {
            let mut result = ICUResourceBundle::get_available_u_locales_type(AvailableType::Default)?
                .ok_or_else(LuceneError::null_pointer)?;
            result.extend(
                ICUResourceBundle::get_available_u_locales_type(AvailableType::OnlyLegacyAliases)?
                    .ok_or_else(LuceneError::null_pointer)?,
            );
            Ok(result)
        } else {
            ICUResourceBundle::get_available_u_locales_type(ty)?.ok_or_else(LuceneError::null_pointer)
        }
    }

    /// `getISOCountries()`.
    pub fn get_iso_countries() -> Vec<String> {
        LocaleIDs::get_iso_countries()
    }

    /// `getISOLanguages()`.
    pub fn get_iso_languages() -> Vec<String> {
        LocaleIDs::get_iso_languages()
    }

    /// `getLanguage()`.
    pub fn get_language(&self) -> String {
        self.base().get_language().to_string()
    }

    /// `getLanguage(String localeID)`.
    pub fn get_language_of(locale_id: &str) -> String {
        LocaleIDParser::new(locale_id).get_language()
    }

    /// `getScript()`.
    pub fn get_script(&self) -> String {
        self.base().get_script().to_string()
    }

    /// `getScript(String localeID)`.
    pub fn get_script_of(locale_id: &str) -> String {
        LocaleIDParser::new(locale_id).get_script()
    }

    /// `getCountry()`.
    pub fn get_country(&self) -> String {
        self.base().get_region().to_string()
    }

    /// `getCountry(String localeID)`.
    pub fn get_country_of(locale_id: &str) -> String {
        LocaleIDParser::new(locale_id).get_country()
    }

    fn get_region_from_key(locale: &ULocale, key: &str) -> Option<String> {
        let subdivision = locale.get_keyword_value(key)?;
        // In UTS35
        //   type = alphanum{3,8} (sep alphanum{3,8})* ;
        // so we know the subdivision must fit the type already.
        //
        //   unicode_subdivision_id = unicode_region_subtag unicode_subdivision_suffix ;
        //   unicode_region_subtag = (alpha{2} | digit{3}) ;
        //   unicode_subdivision_suffix = alphanum{1,4} ;
        // But we also know there are no id in start with digit{3} in
        // https://github.com/unicode-org/cldr/blob/main/common/validity/subdivision.xml
        // Therefore we can simplify as
        // unicode_subdivision_id = alpha{2} alphanum{1,4}
        //
        // and only need to accept/reject the code based on the alpha{2} and the length.
        let n = len16(&subdivision);
        if !(3..=6).contains(&n) {
            return None;
        }
        let region = to_upper_case_default_locale(&crate::r#impl::locale::ascii_util::substring16(&subdivision, 0, 2));
        if BUILTIN.is_set(&region) {
            return Some(region);
        }
        None
    }

    /// `getRegionForSupplementalData(ULocale locale, boolean inferRegion)`
    /// (deprecated): the region of the `rg` keyword, else of the locale,
    /// else (when inferring) of the `sd` keyword or of the likely subtags.
    pub fn get_region_for_supplemental_data(locale: &ULocale, infer_region: bool) -> Result<String> {
        if let Some(region) = Self::get_region_from_key(locale, "rg") {
            return Ok(region);
        }
        let mut region = locale.get_country();
        if region.is_empty() && infer_region {
            if let Some(r) = Self::get_region_from_key(locale, "sd") {
                return Ok(r);
            }
            let maximized = Self::add_likely_subtags(locale)?;
            region = maximized.get_country();
        }
        Ok(region)
    }

    /// `getVariant()`.
    pub fn get_variant(&self) -> String {
        self.base().get_variant().to_string()
    }

    /// `getVariant(String localeID)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn get_variant_of(locale_id: &str) -> Result<String> {
        LocaleIDParser::new(locale_id).get_variant()
    }

    /// `getFallback(String localeID)`: the ID with its last segment
    /// removed.
    ///
    /// # Errors
    ///
    /// As [`get_name_of`](Self::get_name_of).
    pub fn get_fallback_of(locale_id: &str) -> Result<String> {
        Ok(Self::get_fallback_string(&Self::get_name_of(Some(locale_id))?))
    }

    /// `getFallback()`: `None` for the root locale.
    pub fn get_fallback(&self) -> Option<ULocale> {
        if self.0.locale_id.is_empty() || self.0.locale_id.starts_with('@') {
            return None;
        }
        Some(Self::from_parts(&Self::get_fallback_string(&self.0.locale_id), None))
    }

    fn get_fallback_string(fallback: &str) -> String {
        let ext_start = fallback.find('@').unwrap_or(fallback.len());
        let mut last = match fallback[..ext_start].rfind('_') {
            None => 0,
            Some(l) => l,
        };
        if last > 0 || fallback[..ext_start].rfind('_').is_some() {
            // truncate empty segment
            while last > 0 {
                if fallback.as_bytes()[last - 1] != b'_' {
                    break;
                }
                last -= 1;
            }
        }
        format!("{}{}", &fallback[..last], &fallback[ext_start..])
    }

    /// `getBaseName()`: the ID without keywords.
    pub fn get_base_name(&self) -> String {
        Self::get_base_name_of(&self.0.locale_id).expect("a normalized locale ID parses")
    }

    /// `getBaseName(String localeID)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn get_base_name_of(locale_id: &str) -> Result<String> {
        if !locale_id.contains('@') {
            return Ok(locale_id.to_string());
        }
        LocaleIDParser::new(locale_id).get_base_name()
    }

    /// `getName()`: the normalized ID.
    pub fn get_name(&self) -> &str {
        &self.0.locale_id // always normalized
    }

    fn get_shortest_subtag_length(locale_id: &str) -> i32 {
        let units: Vec<u16> = locale_id.encode_utf16().collect();
        let mut length = units.len() as i32;
        let mut reset = true;
        let mut tmp_length = 0;

        for &c in &units {
            if c != b'_' as u16 && c != b'-' as u16 {
                if reset {
                    reset = false;
                    tmp_length = 0;
                }
                tmp_length += 1;
            } else {
                if tmp_length != 0 && tmp_length < length {
                    length = tmp_length;
                }
                reset = true;
            }
        }

        length
    }

    /// `getName(String localeID)`: the normalized form of an ID (BCP 47
    /// tags are converted, `root` is the empty ID).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`;
    /// `NullPointerException` for a `None` ID, as Java's cache lookup.
    pub fn get_name_of(locale_id: Option<&str>) -> Result<String> {
        let mut tmp_locale_id;
        // Convert BCP47 id if necessary
        match locale_id {
            Some(id) if !id.contains('@') && Self::get_shortest_subtag_length(id) == 1 => {
                tmp_locale_id = id.to_string();
                let units: Vec<u16> = id.encode_utf16().collect();
                if id.contains('_') && units[1] != b'_' as u16 && units[1] != b'-' as u16 {
                    tmp_locale_id = id.replace('_', "-");
                }
                tmp_locale_id = Self::for_language_tag(&tmp_locale_id)?.get_name().to_string();
                if tmp_locale_id.is_empty() {
                    tmp_locale_id = id.to_string();
                }
            }
            Some(id) if id.eq_ignore_ascii_case("root") && len16(id) == 4 => {
                tmp_locale_id = EMPTY_STRING.to_string();
            }
            Some(id) => tmp_locale_id = Self::strip_leading_und(id),
            None => return Err(LuceneError::null_pointer()),
        }
        Ok(NAME_CACHE
            .get_instance(tmp_locale_id, ())?
            .expect("the name cache creates a name for every ID"))
    }

    fn strip_leading_und(locale_id: &str) -> String {
        let length = len16(locale_id);
        if length < 3 {
            return locale_id.to_string();
        }

        // If not starts with "und", return.
        if !locale_id.as_bytes()[..3].eq_ignore_ascii_case(b"und") {
            return locale_id.to_string();
        }

        // The string is equals to "und" case-insensitively.
        if length == 3 {
            return EMPTY_STRING.to_string();
        }

        // localeID must have a length >= 4
        let separator = locale_id.as_bytes()[3];
        if separator == b'-' || separator == b'_' {
            // "und-*" or "und_*"
            return locale_id[3..].to_string();
        }

        locale_id.to_string()
    }

    /// `getKeywords()`: the keyword names in order, `None` when there are
    /// none.
    pub fn get_keywords(&self) -> Option<Vec<String>> {
        Self::get_keywords_of(&self.0.locale_id)
    }

    /// `getKeywords(String localeID)`.
    pub fn get_keywords_of(locale_id: &str) -> Option<Vec<String>> {
        LocaleIDParser::new(locale_id).get_keywords()
    }

    /// `getKeywordValue(String keywordName)`.
    pub fn get_keyword_value(&self, keyword_name: &str) -> Option<String> {
        Self::get_keyword_value_of(&self.0.locale_id, keyword_name)
    }

    /// `getKeywordValue(String localeID, String keywordName)`.
    pub fn get_keyword_value_of(locale_id: &str, keyword_name: &str) -> Option<String> {
        LocaleIDParser::new(locale_id).get_keyword_value(keyword_name)
    }

    /// `canonicalize(String localeID)`: the canonical form of an ID, with
    /// the CLDR alias data applied.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the parser or the alias data; the
    /// errors of loading the alias data.
    pub fn canonicalize(locale_id: &str) -> Result<String> {
        let mut parser = LocaleIDParser::with_canonicalize(locale_id, true);
        let base_name = parser.get_base_name()?;
        let mut found_variant = false;

        if locale_id.is_empty() {
            return Ok(String::new());
        }

        // we have an ID in the form xx_Yyyy_ZZ_KKKKK

        /* See if this is an already known locale */
        for (from, to) in CANONICALIZE_MAP {
            if *from == base_name {
                found_variant = true;
                parser.set_base_name(to);
                break;
            }
        }

        /* total mondo hack for Norwegian, fortunately the main NY case is handled earlier */
        if !found_variant && parser.get_language() == "nb" && parser.get_variant()? == "NY" {
            let id = Self::lscv_to_id(Some("nn"), Some(&parser.get_script()), Some(&parser.get_country()), None);
            parser.set_base_name(&id);
        }

        let name = parser.get_name()?;
        if !Self::is_known_canonicalized_locale(&name) {
            let full = parser.get_name()?;
            let base = parser.get_base_name()?;
            let mut replacer = AliasReplacer::new(
                parser.get_language(),
                parser.get_script(),
                parser.get_country(),
                AsciiUtil::to_lower_string(&parser.get_variant()?),
                Some(substring16_from(&full, len16(&base))),
            );
            if let Some(replaced) = replacer.replace()? {
                parser = LocaleIDParser::new(&replaced);
            }
        }

        parser.get_name()
    }

    fn is_known_canonicalized_locale(name: &str) -> bool {
        if name == "c" || name == "en" || name == "en_US" {
            return true;
        }
        static G_KNOWN_CANONICALIZED_CASES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
            [
                "af", "af_ZA", "am", "am_ET", "ar", "ar_001", "as", "as_IN", "az", "az_AZ",
                "be", "be_BY", "bg", "bg_BG", "bn", "bn_IN", "bs", "bs_BA", "ca", "ca_ES",
                "cs", "cs_CZ", "cy", "cy_GB", "da", "da_DK", "de", "de_DE", "el", "el_GR",
                "en", "en_GB", "en_US", "es", "es_419", "es_ES", "et", "et_EE", "eu",
                "eu_ES", "fa", "fa_IR", "fi", "fi_FI", "fil", "fil_PH", "fr", "fr_FR",
                "ga", "ga_IE", "gl", "gl_ES", "gu", "gu_IN", "he", "he_IL", "hi", "hi_IN",
                "hr", "hr_HR", "hu", "hu_HU", "hy", "hy_AM", "id", "id_ID", "is", "is_IS",
                "it", "it_IT", "ja", "ja_JP", "jv", "jv_ID", "ka", "ka_GE", "kk", "kk_KZ",
                "km", "km_KH", "kn", "kn_IN", "ko", "ko_KR", "ky", "ky_KG", "lo", "lo_LA",
                "lt", "lt_LT", "lv", "lv_LV", "mk", "mk_MK", "ml", "ml_IN", "mn", "mn_MN",
                "mr", "mr_IN", "ms", "ms_MY", "my", "my_MM", "nb", "nb_NO", "ne", "ne_NP",
                "nl", "nl_NL", "no", "or", "or_IN", "pa", "pa_IN", "pl", "pl_PL", "ps", "ps_AF",
                "pt", "pt_BR", "pt_PT", "ro", "ro_RO", "ru", "ru_RU", "sd", "sd_IN", "si",
                "si_LK", "sk", "sk_SK", "sl", "sl_SI", "so", "so_SO", "sq", "sq_AL", "sr",
                "sr_Cyrl_RS", "sr_Latn", "sr_RS", "sv", "sv_SE", "sw", "sw_TZ", "ta",
                "ta_IN", "te", "te_IN", "th", "th_TH", "tk", "tk_TM", "tr", "tr_TR", "uk",
                "uk_UA", "ur", "ur_PK", "uz", "uz_UZ", "vi", "vi_VN", "yue", "yue_Hant",
                "yue_Hant_HK", "yue_HK", "zh", "zh_CN", "zh_Hans", "zh_Hans_CN", "zh_Hant",
                "zh_Hant_TW", "zh_TW", "zu", "zu_ZA",
            ]
            .into_iter()
            .collect()
        });
        G_KNOWN_CANONICALIZED_CASES.contains(name)
    }

    /// `setKeywordValue(String keyword, String value)`: a new locale with
    /// the keyword set (`value` `None` removes it; `keyword` `None` removes
    /// all keywords).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty keyword or value.
    pub fn set_keyword_value(&self, keyword: Option<&str>, value: Option<&str>) -> Result<ULocale> {
        Ok(Self::from_parts(
            &Self::set_keyword_value_of(&self.0.locale_id, keyword, value)?,
            None,
        ))
    }

    /// `setKeywordValue(String localeID, String keyword, String value)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty keyword or value.
    pub fn set_keyword_value_of(locale_id: &str, keyword: Option<&str>, value: Option<&str>) -> Result<String> {
        let mut parser = LocaleIDParser::new(locale_id);
        parser.set_keyword_value(keyword, value)?;
        parser.get_name()
    }

    /// `getISO3Language()`.
    pub fn get_iso3_language(&self) -> String {
        Self::get_iso3_language_of(&self.0.locale_id)
    }

    /// `getISO3Language(String localeID)`.
    pub fn get_iso3_language_of(locale_id: &str) -> String {
        LocaleIDs::get_iso3_language(&Self::get_language_of(locale_id))
    }

    /// `getISO3Country()`.
    pub fn get_iso3_country(&self) -> String {
        Self::get_iso3_country_of(&self.0.locale_id)
    }

    /// `getISO3Country(String localeID)`.
    pub fn get_iso3_country_of(locale_id: &str) -> String {
        LocaleIDs::get_iso3_country(&Self::get_country_of(locale_id))
    }

    /// `getCharacterOrientation()`: `ltr`, `rtl`, `ttb` or `btt`, from the
    /// locale data's `layout` table.
    ///
    /// # Errors
    ///
    /// The errors of opening the locale bundle.
    pub fn get_character_orientation(&self) -> Result<Option<String>> {
        ICUResourceTableAccess::get_table_string(ICUData::ICU_BASE_NAME, self, "layout", "characters", Some("characters"))
    }

    /// `getLineOrientation()`.
    ///
    /// # Errors
    ///
    /// The errors of opening the locale bundle.
    pub fn get_line_orientation(&self) -> Result<Option<String>> {
        ICUResourceTableAccess::get_table_string(ICUData::ICU_BASE_NAME, self, "layout", "lines", Some("lines"))
    }

    /// `isRightToLeft()`: whether this locale's script is written
    /// right-to-left (the likely script when the locale has none).
    ///
    /// # Errors
    ///
    /// The errors of the likely-subtags and property names data.
    pub fn is_right_to_left(&self) -> Result<bool> {
        let mut script = self.get_script();
        if script.is_empty() {
            // Fastpath: We know the likely scripts and their writing direction
            // for some common languages.
            let lang = self.get_language();
            if !lang.is_empty() {
                if let Some(lang_index) = LANG_DIR_STRING.find(lang.as_str()) {
                    match LANG_DIR_STRING.as_bytes()[lang_index + lang.len()] {
                        b'-' => return Ok(false),
                        b'+' => return Ok(true),
                        _ => {} // partial match of a longer code
                    }
                }
            }
            // Otherwise, find the likely script.
            let likely = ULocale::add_likely_subtags(self)?;
            script = likely.get_script();
            if script.is_empty() {
                return Ok(false);
            }
        }
        let script_code = crate::lang::u_script::UScript::get_code_from_name(&script)?;
        Ok(crate::lang::u_script::UScript::is_right_to_left(script_code))
    }

    /// `addLikelySubtags(ULocale loc)`: the maximized locale.
    ///
    /// # Errors
    ///
    /// The errors of loading the likely-subtags data.
    pub fn add_likely_subtags(loc: &ULocale) -> Result<ULocale> {
        let mut tags: [String; 3] = Default::default();
        let mut trailing: Option<String> = None;

        let trailing_index = Self::parse_tag_string(&loc.0.locale_id, &mut tags)?;

        if trailing_index < len16(&loc.0.locale_id) {
            trailing = Some(substring16_from(&loc.0.locale_id, trailing_index));
        }

        let max = LikelySubtags::instance()?.make_maximized_lsr_from(
            &ULocale::new3(Some(&loc.get_language()), Some(&loc.get_script()), Some(&loc.get_country()))?,
            true,
        )?;
        let new_locale_id = Self::create_tag_string(&max.language, &max.script, &max.region, trailing.as_deref());

        Ok(match new_locale_id {
            None => loc.clone(),
            Some(id) => ULocale::new(&id)?,
        })
    }

    /// `minimizeSubtags(ULocale loc)`: favoring the region.
    ///
    /// # Errors
    ///
    /// The errors of loading the likely-subtags data.
    pub fn minimize_subtags(loc: &ULocale) -> Result<ULocale> {
        Self::minimize_subtags_favor(loc, Minimize::FavorRegion)
    }

    /// `minimizeSubtags(ULocale loc, Minimize fieldToFavor)` (deprecated).
    ///
    /// # Errors
    ///
    /// The errors of loading the likely-subtags data.
    pub fn minimize_subtags_favor(loc: &ULocale, field_to_favor: Minimize) -> Result<ULocale> {
        let mut tags: [String; 3] = Default::default();
        let mut trailing: Option<String> = None;

        let trailing_index = Self::parse_tag_string(&loc.0.locale_id, &mut tags)?;

        if trailing_index < len16(&loc.0.locale_id) {
            trailing = Some(substring16_from(&loc.0.locale_id, trailing_index));
        }
        let lsr = LikelySubtags::instance()?.minimize_subtags(
            &loc.get_language(),
            &loc.get_script(),
            &loc.get_country(),
            field_to_favor,
        )?;
        let new_locale_id = Self::create_tag_string(&lsr.language, &lsr.script, &lsr.region, trailing.as_deref());
        Ok(match new_locale_id {
            None => loc.clone(),
            Some(id) => ULocale::new(&id)?,
        })
    }

    fn append_tag(tag: &str, buffer: &mut String) {
        if !buffer.is_empty() {
            buffer.push(UNDERSCORE);
        }
        buffer.push_str(tag);
    }

    /// `createTagString(String lang, String script, String region, String
    /// trailing)` (private); never `null` in Java, `Some` here.
    fn create_tag_string(lang: &str, script: &str, region: &str, trailing: Option<&str>) -> Option<String> {
        let mut tag = String::new();

        if !lang.is_empty() {
            Self::append_tag(lang, &mut tag);
        } else {
            /*
             * Append the value for an unknown language, if
             * we found no language.
             */
            Self::append_tag(UNDEFINED_LANGUAGE, &mut tag);
        }

        if !script.is_empty() {
            Self::append_tag(script, &mut tag);
        }

        if !region.is_empty() {
            Self::append_tag(region, &mut tag);
        }

        if let Some(trailing) = trailing.filter(|t| len16(t) > 1) {
            /*
             * The current ICU format expects two underscores
             * will separate the variant from the preceding
             * parts of the tag, if there is no region.
             */
            let mut separators = 0;
            let tb = trailing.as_bytes();
            if tb[0] == b'_' {
                if tb[1] == b'_' {
                    separators = 2;
                }
            } else {
                separators = 1;
            }

            if !region.is_empty() {
                /*
                 * If we appended a region, we may need to strip
                 * the extra separator from the variant portion.
                 */
                if separators == 2 {
                    tag.push_str(&trailing[1..]);
                } else {
                    tag.push_str(trailing);
                }
            } else {
                /*
                 * If we did not append a region, we may need to add
                 * an extra separator to the variant portion.
                 */
                if separators == 1 {
                    tag.push(UNDERSCORE);
                }
                tag.push_str(trailing);
            }
        }

        Some(tag)
    }

    /// `parseTagString(String localeID, String tags[])` (private): the
    /// language, script and region into `tags`, and the index of the
    /// variant's separator or of the keywords.
    fn parse_tag_string(locale_id: &str, tags: &mut [String; 3]) -> Result<i32> {
        let mut parser = LocaleIDParser::new(locale_id);

        let lang = parser.get_language();
        let script = parser.get_script();
        let region = parser.get_country();

        tags[0] = if lang.is_empty() {
            UNDEFINED_LANGUAGE.to_string()
        } else {
            lang
        };

        tags[1] = if script == UNDEFINED_SCRIPT {
            String::new()
        } else {
            script
        };

        tags[2] = if region == UNDEFINED_REGION {
            String::new()
        } else {
            region
        };

        /*
         * Search for the variant.  If there is one, then return the index of
         * the preceding separator.
         * If there's no variant, search for the keyword delimiter,
         * and return its index.  Otherwise, return the length of the
         * string.
         *
         * $TOTO(dbertoni) we need to take into account that we might
         * find a part of the language as the variant, since it can
         * can have a variant portion that is long enough to contain
         * the same characters as the variant.
         */
        let variant = parser.get_variant()?;

        if !variant.is_empty() {
            let index = crate::r#impl::locale::ascii_util::index_of16(locale_id, &variant, 0);
            Ok(if index > 0 { index - 1 } else { index })
        } else {
            let units: Vec<u16> = locale_id.encode_utf16().collect();
            let index = crate::jdk::index_of_unit(&units, b'@' as u16, 0);
            Ok(if index == -1 { units.len() as i32 } else { index })
        }
    }

    // --------------------------------
    //      BCP47/OpenJDK APIs
    // --------------------------------

    /// `PRIVATE_USE_EXTENSION`.
    pub const PRIVATE_USE_EXTENSION: char = 'x';

    /// `UNICODE_LOCALE_EXTENSION`.
    pub const UNICODE_LOCALE_EXTENSION: char = 'u';

    /// `getExtension(char key)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid extension key: ...")`.
    pub fn get_extension(&self, key: char) -> Result<Option<String>> {
        if !LocaleExtensions::is_valid_key(key) {
            return Err(LuceneError::illegal_argument(format!("Invalid extension key: {key}")));
        }
        Ok(self.extensions().get_extension_value(key).map(str::to_string))
    }

    /// `getExtensionKeys()`.
    pub fn get_extension_keys(&self) -> Vec<char> {
        self.extensions().get_keys().collect()
    }

    /// `getUnicodeLocaleAttributes()`.
    pub fn get_unicode_locale_attributes(&self) -> Vec<String> {
        self.extensions().get_unicode_locale_attributes().into_iter().collect()
    }

    /// `getUnicodeLocaleType(String key)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid Unicode locale key: ...")`.
    pub fn get_unicode_locale_type(&self, key: &str) -> Result<Option<String>> {
        if !LocaleExtensions::is_valid_unicode_locale_key(key) {
            return Err(LuceneError::illegal_argument(format!("Invalid Unicode locale key: {key}")));
        }
        Ok(self.extensions().get_unicode_locale_type(key))
    }

    /// `getUnicodeLocaleKeys()`.
    pub fn get_unicode_locale_keys(&self) -> Vec<String> {
        self.extensions().get_unicode_locale_keys()
    }

    /// `toLanguageTag()`: the well-formed BCP 47 tag of this locale.
    pub fn to_language_tag(&self) -> String {
        let mut base = self.base();
        let mut exts = self.extensions();

        if base.get_variant().eq_ignore_ascii_case("POSIX") {
            // special handling for variant POSIX
            base = BaseLocale::get_instance(base.get_language(), base.get_script(), base.get_region(), "");
            if exts.get_unicode_locale_type("va").is_none() {
                // add va-posix
                let mut ilocbld = InternalLocaleBuilder::new();
                ilocbld
                    .set_locale(&base_locale::ROOT, Some(&exts))
                    .and_then(|b| b.set_unicode_locale_keyword("va", Some("posix")).map(|_| ()))
                    // this should not happen
                    .expect("va-posix is a valid keyword for valid extensions");
                exts = ilocbld.get_locale_extensions();
            }
        }

        let tag = LanguageTag::parse_locale(&base, &exts);

        let mut buf = String::new();
        let subtag = tag.get_language();
        if !subtag.is_empty() {
            buf.push_str(&LanguageTag::canonicalize_language(subtag));
        }

        let subtag = tag.get_script();
        if !subtag.is_empty() {
            buf.push_str(LanguageTag::SEP);
            buf.push_str(&LanguageTag::canonicalize_script(subtag));
        }

        let subtag = tag.get_region();
        if !subtag.is_empty() {
            buf.push_str(LanguageTag::SEP);
            buf.push_str(&LanguageTag::canonicalize_region(subtag));
        }

        // ICU-20478: Sort variants per UTS35.
        let mut variants: Vec<String> = tag.get_variants().to_vec();
        variants.sort_by(|a, b| rucene::util::java_collections::java_compare_to(a, b));
        for s in &variants {
            buf.push_str(LanguageTag::SEP);
            buf.push_str(&LanguageTag::canonicalize_variant(s));
        }

        for s in tag.get_extensions() {
            buf.push_str(LanguageTag::SEP);
            buf.push_str(&LanguageTag::canonicalize_extension(s));
        }

        let subtag = tag.get_privateuse();
        if !subtag.is_empty() {
            if buf.is_empty() {
                buf.push_str(UNDEFINED_LANGUAGE);
            }
            buf.push_str(LanguageTag::SEP);
            buf.push_str(LanguageTag::PRIVATEUSE);
            buf.push_str(LanguageTag::SEP);
            buf.push_str(&LanguageTag::canonicalize_privateuse(subtag));
        }

        buf
    }

    /// `forLanguageTag(String languageTag)`: the locale of a BCP 47 tag,
    /// lenient (parsing stops at the first ill-formed subtag).
    ///
    /// # Errors
    ///
    /// The errors of the key-type data lookups.
    pub fn for_language_tag(language_tag: &str) -> Result<ULocale> {
        let tag = LanguageTag::parse(language_tag, None);
        let mut bldr = InternalLocaleBuilder::new();
        bldr.set_language_tag(&tag);
        Self::get_instance(&bldr.get_base_locale(), &bldr.get_locale_extensions())
    }

    /// `toUnicodeLocaleKey(String keyword)`: the BCP 47 key of a legacy
    /// keyword, `None` when it is not well-formed.
    ///
    /// # Errors
    ///
    /// The errors of loading the key-type data.
    pub fn to_unicode_locale_key(keyword: &str) -> Result<Option<String>> {
        let mut bcp_key = KeyTypeData::to_bcp_key(keyword)?;
        if bcp_key.is_none() && UnicodeLocaleExtension::is_key(keyword) {
            // unknown keyword, but syntax is fine..
            bcp_key = Some(AsciiUtil::to_lower_string(keyword));
        }
        Ok(bcp_key)
    }

    /// `toUnicodeLocaleType(String keyword, String value)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the key-type data.
    pub fn to_unicode_locale_type(keyword: &str, value: &str) -> Result<Option<String>> {
        let mut bcp_type = KeyTypeData::to_bcp_type(keyword, value, None, None)?;
        if bcp_type.is_none() && UnicodeLocaleExtension::is_type(value) {
            // unknown keyword, but syntax is fine..
            bcp_type = Some(AsciiUtil::to_lower_string(value));
        }
        Ok(bcp_type)
    }

    /// `toLegacyKey(String keyword)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the key-type data.
    pub fn to_legacy_key(keyword: &str) -> Result<Option<String>> {
        let mut legacy_key = KeyTypeData::to_legacy_key(keyword)?;
        if legacy_key.is_none() {
            // Checks if the specified locale key is well-formed with the legacy locale syntax.
            //
            // Note:
            //  Neither ICU nor LDML/CLDR provides the definition of keyword syntax.
            //  However, a key should not contain '=' obviously. For now, all existing
            //  keys are using ASCII alphabetic letters only. We won't add any new key
            //  that is not compatible with the BCP 47 syntax. Therefore, we assume
            //  a valid key consist from [0-9a-zA-Z], no symbols.
            static PAT: LazyLock<JavaPattern> =
                LazyLock::new(|| JavaPattern::compile("[0-9a-zA-Z]+", 0).expect("valid pattern"));
            if PAT.matches(keyword) {
                legacy_key = Some(AsciiUtil::to_lower_string(keyword));
            }
        }
        Ok(legacy_key)
    }

    /// `toLegacyType(String keyword, String value)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the key-type data.
    pub fn to_legacy_type(keyword: &str, value: &str) -> Result<Option<String>> {
        let mut legacy_type = KeyTypeData::to_legacy_type(keyword, value, None, None)?;
        if legacy_type.is_none() {
            // Checks if the specified locale type is well-formed with the legacy locale syntax.
            //
            // Note:
            //  Neither ICU nor LDML/CLDR provides the definition of keyword syntax.
            //  However, a type should not contain '=' obviously. For now, all existing
            //  types are using ASCII alphabetic letters with a few symbol letters. We won't
            //  add any new type that is not compatible with the BCP 47 syntax except timezone
            //  IDs. For now, we assume a valid type start with [0-9a-zA-Z], but may contain
            //  '-' '_' '/' in the middle.
            static PAT: LazyLock<JavaPattern> = LazyLock::new(|| {
                JavaPattern::compile("[0-9a-zA-Z]+([_/\\-][0-9a-zA-Z]+)*", 0).expect("valid pattern")
            });
            if PAT.matches(value) {
                legacy_type = Some(AsciiUtil::to_lower_string(value));
            }
        }
        Ok(legacy_type)
    }

    /// `getInstance(BaseLocale base, LocaleExtensions exts)` (private).
    fn get_instance(base: &BaseLocale, exts: &LocaleExtensions) -> Result<ULocale> {
        let mut id = Self::lscv_to_id(
            Some(base.get_language()),
            Some(base.get_script()),
            Some(base.get_region()),
            Some(base.get_variant()),
        );

        let ext_keys: Vec<char> = exts.get_keys().collect();
        if !ext_keys.is_empty() {
            // legacy locale ID assume Unicode locale keywords and
            // other extensions are at the same level.
            // e.g. @a=ext-for-aa;calendar=japanese;m=ext-for-mm;x=priv-use

            let mut kwds = KeywordMap::default();
            for key in ext_keys {
                let ext = exts.get_extension(key).expect("a key has its extension");
                if let Some(uext) = ext.as_unicode() {
                    let ukeys: Vec<String> = uext.get_unicode_locale_keys().cloned().collect();
                    for bcp_key in ukeys {
                        let bcp_type = uext.get_unicode_locale_type(&bcp_key).unwrap_or("");
                        // convert to legacy key/type
                        let lkey = Self::to_legacy_key(&bcp_key)?.ok_or_else(LuceneError::null_pointer)?;
                        let ltype = Self::to_legacy_type(&bcp_key, if bcp_type.is_empty() { "yes" } else { bcp_type })?; // use "yes" as the value of typeless keywords
                        let ltype = ltype.unwrap_or_else(|| "null".to_string());
                        // special handling for u-va-posix, since this is a variant, not a keyword
                        if lkey == "va" && ltype == "posix" && base.get_variant().is_empty() {
                            id.push_str("_POSIX");
                        } else {
                            kwds.insert(lkey, ltype);
                        }
                    }
                    // Mapping Unicode locale attribute to the special keyword, attribute=xxx-yyy
                    let uattributes = uext.get_unicode_locale_attributes();
                    if !uattributes.is_empty() {
                        let mut attrbuf = String::new();
                        for attr in uattributes {
                            if !attrbuf.is_empty() {
                                attrbuf.push('-');
                            }
                            attrbuf.push_str(attr);
                        }
                        kwds.insert(LOCALE_ATTRIBUTE_KEY.to_string(), attrbuf);
                    }
                } else {
                    kwds.insert(key.to_string(), ext.get_value().to_string());
                }
            }

            if !kwds.is_empty() {
                let mut buf = id.clone();
                buf.push('@');
                let mut insert_sep = false;
                for (k, v) in kwds.iter() {
                    if insert_sep {
                        buf.push(';');
                    } else {
                        insert_sep = true;
                    }
                    buf.push_str(k);
                    buf.push('=');
                    buf.push_str(v);
                }
                id = buf;
            }
        }
        ULocale::new(&id)
    }

    fn base(&self) -> Arc<BaseLocale> {
        self.0
            .base_locale
            .get_or_init(|| {
                let (mut language, mut script, mut region, mut variant) =
                    (String::new(), String::new(), String::new(), String::new());
                if self.0.locale_id != ROOT.0.locale_id {
                    let mut lp = LocaleIDParser::new(&self.0.locale_id);
                    language = lp.get_language();
                    script = lp.get_script();
                    region = lp.get_country();
                    variant = lp
                        .get_variant()
                        .expect("a normalized locale ID has a variant the parser accepts");
                }
                BaseLocale::get_instance(&language, &script, &region, &variant)
            })
            .clone()
    }

    fn extensions(&self) -> Arc<LocaleExtensions> {
        if let Some(e) = self.0.extensions.get() {
            return e.clone();
        }
        let computed = match self.get_keywords() {
            None => locale_extensions::EMPTY_EXTENSIONS.clone(),
            Some(kwitr) => {
                let mut intbld = InternalLocaleBuilder::new();
                for key in kwitr {
                    let value = self.get_keyword_value(&key).unwrap_or_default();
                    if key == LOCALE_ATTRIBUTE_KEY {
                        // special keyword used for representing Unicode locale attributes
                        let uattributes: Vec<&str> = value.split(['-', '_']).collect();
                        for uattr in uattributes {
                            // ignore and fall through
                            let _ = intbld.add_unicode_locale_attribute(Some(uattr));
                        }
                    } else if len16(&key) >= 2 {
                        let bcp_key = Self::to_unicode_locale_key(&key).ok().flatten();
                        let bcp_type = Self::to_unicode_locale_type(&key, &value).ok().flatten();
                        if let (Some(bcp_key), Some(bcp_type)) = (bcp_key, bcp_type) {
                            // ignore and fall through
                            let _ = intbld.set_unicode_locale_keyword(&bcp_key, Some(&bcp_type));
                        }
                    } else if len16(&key) == 1 && !key.starts_with(Self::UNICODE_LOCALE_EXTENSION) {
                        // ignore and fall through
                        let c = key.chars().next().expect("a one-unit key has a char");
                        let _ = intbld.set_extension(c, Some(&value.replace('_', LanguageTag::SEP)));
                    }
                }
                intbld.get_locale_extensions()
            }
        };
        self.0.extensions.get_or_init(|| computed).clone()
    }
}

// display names

/// The display locale of the no-argument `getDisplay*` methods:
/// `getDefault(Category.DISPLAY)`.
fn default_display_locale() -> ULocale {
    ULocale::get_default_category(Category::Display)
}

/// `String.trim()` over a Rust string: without the leading and trailing
/// characters `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

impl ULocale {
    /// `getDisplayLanguage()`: this locale's language localized for display
    /// in the default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_language(&self) -> Result<Option<String>> {
        Self::get_display_language_internal(self, &default_display_locale(), false)
    }

    /// `getDisplayLanguage(ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_language_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_language_internal(self, display_locale, false)
    }

    /// `getDisplayLanguage(String localeID, String displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_language_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_language_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?, false)
    }

    /// `getDisplayLanguage(String localeID, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_language_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_language_internal(&ULocale::new(locale_id)?, display_locale, false)
    }

    /// `getDisplayLanguageWithDialect()`: the language with dialect (base
    /// name) localized for the default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_language_with_dialect(&self) -> Result<Option<String>> {
        Self::get_display_language_internal(self, &default_display_locale(), true)
    }

    /// `getDisplayLanguageWithDialect(ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_language_with_dialect_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_language_internal(self, display_locale, true)
    }

    /// `getDisplayLanguageWithDialect(String localeID, String
    /// displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_language_with_dialect_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_language_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?, true)
    }

    /// `getDisplayLanguageWithDialect(String localeID, ULocale
    /// displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_language_with_dialect_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_language_internal(&ULocale::new(locale_id)?, display_locale, true)
    }

    fn get_display_language_internal(locale: &ULocale, display_locale: &ULocale, use_dialect: bool) -> Result<Option<String>> {
        let lang = if use_dialect { locale.get_base_name() } else { locale.get_language() };
        locale_display_names::get_instance(display_locale).language_display_name(&lang)
    }

    /// `getDisplayScript()`: this locale's script localized for display in
    /// the default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_script(&self) -> Result<Option<String>> {
        Self::get_display_script_internal(self, &default_display_locale())
    }

    /// `getDisplayScriptInContext()` (internal, deprecated).
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_script_in_context(&self) -> Result<Option<String>> {
        Self::get_display_script_in_context_internal(self, &default_display_locale())
    }

    /// `getDisplayScript(ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_script_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_script_internal(self, display_locale)
    }

    /// `getDisplayScriptInContext(ULocale displayLocale)` (internal,
    /// deprecated).
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_script_in_context_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_script_in_context_internal(self, display_locale)
    }

    /// `getDisplayScript(String localeID, String displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_script_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_script_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayScriptInContext(String localeID, String displayLocaleID)`
    /// (internal, deprecated).
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_script_in_context_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_script_in_context_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayScript(String localeID, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_script_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_script_internal(&ULocale::new(locale_id)?, display_locale)
    }

    /// `getDisplayScriptInContext(String localeID, ULocale displayLocale)`
    /// (internal, deprecated).
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_script_in_context_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_script_in_context_internal(&ULocale::new(locale_id)?, display_locale)
    }

    // displayLocaleID is canonical, localeID need not be since parsing will fix this.
    fn get_display_script_internal(locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
        locale_display_names::get_instance(display_locale).script_display_name(&locale.get_script())
    }

    fn get_display_script_in_context_internal(locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
        locale_display_names::get_instance(display_locale).script_display_name_in_context(&locale.get_script())
    }

    /// `getDisplayCountry()`: this locale's country localized for display
    /// in the default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_country(&self) -> Result<Option<String>> {
        Self::get_display_country_internal(self, &default_display_locale())
    }

    /// `getDisplayCountry(ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_country_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_country_internal(self, display_locale)
    }

    /// `getDisplayCountry(String localeID, String displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_country_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_country_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayCountry(String localeID, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_country_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_country_internal(&ULocale::new(locale_id)?, display_locale)
    }

    // displayLocaleID is canonical, localeID need not be since parsing will fix this.
    fn get_display_country_internal(locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
        locale_display_names::get_instance(display_locale).region_display_name(&locale.get_country())
    }

    /// `getDisplayVariant()`: this locale's variant localized for display
    /// in the default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_variant(&self) -> Result<Option<String>> {
        Self::get_display_variant_internal(self, &default_display_locale())
    }

    /// `getDisplayVariant(ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_variant_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_variant_internal(self, display_locale)
    }

    /// `getDisplayVariant(String localeID, String displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_variant_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_variant_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayVariant(String localeID, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_variant_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_variant_internal(&ULocale::new(locale_id)?, display_locale)
    }

    fn get_display_variant_internal(locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
        locale_display_names::get_instance(display_locale).variant_display_name(&locale.get_variant())
    }

    /// `getDisplayKeyword(String keyword)`: a keyword localized for display
    /// in the default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_keyword(keyword: &str) -> Result<Option<String>> {
        Self::get_display_keyword_internal(keyword, &default_display_locale())
    }

    /// `getDisplayKeyword(String keyword, String displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_keyword_of(keyword: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_keyword_internal(keyword, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayKeyword(String keyword, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_keyword_in(keyword: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_keyword_internal(keyword, display_locale)
    }

    fn get_display_keyword_internal(keyword: &str, display_locale: &ULocale) -> Result<Option<String>> {
        locale_display_names::get_instance(display_locale).key_display_name(keyword)
    }

    /// `getDisplayKeywordValue(String keyword)`: the value of a keyword of
    /// this locale localized for display in the default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_keyword_value(&self, keyword: &str) -> Result<Option<String>> {
        Self::get_display_keyword_value_internal(self, keyword, &default_display_locale())
    }

    /// `getDisplayKeywordValue(String keyword, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_keyword_value_in(&self, keyword: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_keyword_value_internal(self, keyword, display_locale)
    }

    /// `getDisplayKeywordValue(String localeID, String keyword, String
    /// displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_keyword_value_of(locale_id: &str, keyword: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_keyword_value_internal(&ULocale::new(locale_id)?, keyword, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayKeywordValue(String localeID, String keyword, ULocale
    /// displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_keyword_value_of_in(locale_id: &str, keyword: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_keyword_value_internal(&ULocale::new(locale_id)?, keyword, display_locale)
    }

    // displayLocaleID is canonical, localeID need not be since parsing will fix this.
    fn get_display_keyword_value_internal(locale: &ULocale, keyword: &str, display_locale: &ULocale) -> Result<Option<String>> {
        let keyword = AsciiUtil::to_lower_string(java_trim(keyword));
        let value = locale.get_keyword_value(&keyword);
        locale_display_names::get_instance(display_locale).key_value_display_name(&keyword, value.as_deref())
    }

    /// `getDisplayName()`: this locale localized for display in the default
    /// `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_name(&self) -> Result<Option<String>> {
        Self::get_display_name_internal(self, &default_display_locale())
    }

    /// `getDisplayName(ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_name_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_name_internal(self, display_locale)
    }

    /// `getDisplayName(String localeID, String displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_name_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_name_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayName(String localeID, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_name_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_name_internal(&ULocale::new(locale_id)?, display_locale)
    }

    fn get_display_name_internal(locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
        locale_display_names::get_instance(display_locale).locale_display_name(locale)
    }

    /// `getDisplayNameWithDialect()`: this locale localized for display in
    /// the default `DISPLAY` locale, using dialect names.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_name_with_dialect(&self) -> Result<Option<String>> {
        Self::get_display_name_with_dialect_internal(self, &default_display_locale())
    }

    /// `getDisplayNameWithDialect(ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name lookup.
    pub fn get_display_name_with_dialect_in(&self, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_name_with_dialect_internal(self, display_locale)
    }

    /// `getDisplayNameWithDialect(String localeID, String displayLocaleID)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_name_with_dialect_of(locale_id: &str, display_locale_id: &str) -> Result<Option<String>> {
        Self::get_display_name_with_dialect_internal(&ULocale::new(locale_id)?, &ULocale::new(display_locale_id)?)
    }

    /// `getDisplayNameWithDialect(String localeID, ULocale displayLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale` and of the display-name lookup.
    pub fn get_display_name_with_dialect_of_in(locale_id: &str, display_locale: &ULocale) -> Result<Option<String>> {
        Self::get_display_name_with_dialect_internal(&ULocale::new(locale_id)?, display_locale)
    }

    fn get_display_name_with_dialect_internal(locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
        locale_display_names::get_instance_dialect(display_locale, DialectHandling::DialectNames).locale_display_name(locale)
    }

    /// `acceptLanguage(String acceptLanguageList, ULocale[]
    /// availableLocales, boolean[] fallback)`: the best match of an HTTP
    /// `Accept-Language` list among the available locales, `None` if none
    /// (or if the list cannot be parsed, Java's `catch
    /// (IllegalArgumentException e)`). `fallback[0]`, when given, is set to
    /// whether the match is a fallback (not the desired locale itself).
    ///
    /// # Errors
    ///
    /// The errors of the matcher (data loading, likely subtags).
    pub fn accept_language(
        accept_language_list: &str,
        available_locales: &[ULocale],
        fallback: Option<&mut [bool]>,
    ) -> Result<Option<ULocale>> {
        let mut fallback = fallback;
        if let Some(f) = fallback.as_deref_mut() {
            f[0] = true;
        }
        let desired = match LocalePriorityList::add_accept_language(accept_language_list) {
            Ok(mut b) => b.build(),
            Err(e) if e.is_illegal_argument_exception() => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut builder = LocaleMatcher::builder();
        for locale in available_locales {
            builder.add_supported_u_locale(locale.clone());
        }
        let matcher = builder.build()?;
        let result = matcher.get_best_match_result_iter(desired.get_u_locales())?;
        if result.get_desired_index() >= 0 {
            if let Some(f) = fallback {
                if result.get_desired_u_locale()? == result.get_supported_u_locale() {
                    f[0] = false;
                }
            }
            return Ok(result.get_supported_u_locale());
        }
        Ok(None)
    }

    /// `acceptLanguage(ULocale[] acceptLanguageList, ULocale[]
    /// availableLocales, boolean[] fallback)`: the best match of a list of
    /// desired locales in preference order.
    ///
    /// # Errors
    ///
    /// The errors of the matcher (data loading, likely subtags).
    pub fn accept_language_locales(
        accept_language_list: &[ULocale],
        available_locales: &[ULocale],
        fallback: Option<&mut [bool]>,
    ) -> Result<Option<ULocale>> {
        let mut fallback = fallback;
        if let Some(f) = fallback.as_deref_mut() {
            f[0] = true;
        }
        let mut builder = LocaleMatcher::builder();
        for locale in available_locales {
            builder.add_supported_u_locale(locale.clone());
        }
        let matcher = builder.build()?;
        let result = if accept_language_list.len() == 1 {
            matcher.get_best_match_result(&accept_language_list[0])?
        } else {
            matcher.get_best_match_result_iter(accept_language_list.iter().cloned())?
        };
        if result.get_desired_index() >= 0 {
            if let Some(f) = fallback {
                if result.get_desired_u_locale()? == result.get_supported_u_locale() {
                    f[0] = false;
                }
            }
            return Ok(result.get_supported_u_locale());
        }
        Ok(None)
    }

    /// `acceptLanguage(String acceptLanguageList, boolean[] fallback)`:
    /// against `getAvailableLocales()`.
    ///
    /// # Errors
    ///
    /// The errors of `getAvailableLocales` and of the matcher.
    pub fn accept_language_available(accept_language_list: &str, fallback: Option<&mut [bool]>) -> Result<Option<ULocale>> {
        Self::accept_language(accept_language_list, &ULocale::get_available_locales()?, fallback)
    }

    /// `acceptLanguage(ULocale[] acceptLanguageList, boolean[] fallback)`:
    /// against `getAvailableLocales()`.
    ///
    /// # Errors
    ///
    /// The errors of `getAvailableLocales` and of the matcher.
    pub fn accept_language_locales_available(
        accept_language_list: &[ULocale],
        fallback: Option<&mut [bool]>,
    ) -> Result<Option<ULocale>> {
        Self::accept_language_locales(accept_language_list, &ULocale::get_available_locales()?, fallback)
    }
}

/// `equals(Object obj)`: equal IDs.
impl PartialEq for ULocale {
    fn eq(&self, other: &Self) -> bool {
        self.0.locale_id == other.0.locale_id
    }
}

impl Eq for ULocale {}

impl Hash for ULocale {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.locale_id.hash(state);
    }
}

/// `toString()`: the ID.
impl fmt::Display for ULocale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.locale_id)
    }
}

/// The CLDR alias data of `canonicalize`, loaded once from the `metadata`
/// bundle.
struct AliasData {
    language_alias_map: HashMap<String, String>,
    script_alias_map: HashMap<String, String>,
    territory_alias_map: HashMap<String, Vec<String>>,
    variant_alias_map: HashMap<String, String>,
    subdivision_alias_map: HashMap<String, String>,
}

/// `aliasDataIsLoaded` and the maps.
static ALIAS_DATA: Mutex<Option<Arc<AliasData>>> = Mutex::new(None);

/// The alias replacement of `canonicalize`.
///
/// Equivalent to `ULocale.AliasReplacer` (private static class).
struct AliasReplacer {
    language: String,
    script: String,
    region: String,
    variants: Option<Vec<String>>,
    extensions: Option<String>,
    data: Option<Arc<AliasData>>,
}

impl AliasReplacer {
    fn new(language: String, script: String, region: String, variants: String, extensions: Option<String>) -> Self {
        let variants = if !variants.is_empty() {
            Some(java_split(&variants, '_'))
        } else {
            None
        };
        Self {
            language,
            script,
            region,
            variants,
            extensions,
            data: None,
        }
    }

    fn data(&self) -> &AliasData {
        self.data.as_deref().expect("loadAliasData ran")
    }

    fn joined_variants(&self) -> String {
        self.variants.as_ref().map(|v| v.join("_")).unwrap_or_default()
    }

    fn replace(&mut self) -> Result<Option<String>> {
        let mut changed = false;
        self.data = Some(Self::load_alias_data()?);
        let mut count = 0;
        loop {
            if count > 10 {
                // Throw exception when we loop through too many time
                // stop to avoid infinity loop cauesd by incorrect data
                // in resource.
                return Err(LuceneError::illegal_argument(format!(
                    "Have problem to resolve locale alias of {}{}",
                    ULocale::lscv_to_id(
                        Some(&self.language),
                        Some(&self.script),
                        Some(&self.region),
                        Some(&self.joined_variants())
                    ),
                    self.extensions.as_deref().unwrap_or("null")
                )));
            }
            count += 1;
            // Anytime we replace something, we need to start over again.
            //                      lang  REGION  variant
            if self.replace_language(true, true, true)
                || self.replace_language(true, true, false)
                || self.replace_language(true, false, true)
                || self.replace_language(true, false, false)
                || self.replace_language(false, false, true)
                || self.replace_region()?
                || self.replace_script()
                || self.replace_variant()
            {
                // Some values in data is changed, try to match from the
                // beginning again.
                changed = true;
                continue;
            }
            // Nothing changed in this iteration, break out the loop
            break;
        } // while(1)
        if self.extensions.is_none() && !changed {
            return Ok(None);
        }
        let mut result = ULocale::lscv_to_id(
            Some(&self.language),
            Some(&self.script),
            Some(&self.region),
            Some(&self.joined_variants()),
        );
        if let Some(extensions) = self.extensions.clone() {
            let mut keyword_changed = false;
            let mut temp = ULocale::new(&format!("{result}{extensions}"))?;
            if let Some(keywords) = temp.get_keywords() {
                for key in keywords {
                    if key == "rg" || key == "sd" || key == "t" {
                        let value = temp.get_keyword_value(&key).ok_or_else(LuceneError::null_pointer)?;
                        let replacement = if key == "t" {
                            Some(self.replace_transformed_extensions(&value)?)
                        } else {
                            self.replace_subdivision(&value)
                        };
                        if let Some(replacement) = replacement {
                            temp = temp.set_keyword_value(Some(&key), Some(&replacement))?;
                            keyword_changed = true;
                        }
                    }
                }
            }
            let mut extensions = extensions;
            if keyword_changed {
                extensions = substring16_from(temp.get_name(), len16(&temp.get_base_name()));
                changed = true;
            }
            result.push_str(&extensions);
            self.extensions = Some(extensions);
        }
        if changed {
            return Ok(Some(result));
        }
        // Nothing changed in any iteration of the loop.
        Ok(None)
    }

    /// `loadAliasData()`: initializes the alias data from the ICU resource
    /// bundles, once.
    fn load_alias_data() -> Result<Arc<AliasData>> {
        let mut guard = ALIAS_DATA.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = guard.as_ref() {
            return Ok(d.clone());
        }
        let mut language_alias_map = HashMap::new();
        let mut script_alias_map = HashMap::new();
        let mut territory_alias_map = HashMap::new();
        let mut variant_alias_map = HashMap::new();
        let mut subdivision_alias_map = HashMap::new();

        let metadata = UResourceBundle::get_bundle_instance_loader(
            Some(ICUData::ICU_BASE_NAME),
            "metadata",
            icu_resource_bundle::icu_data_class_loader(),
        )?;
        let metadata_alias = metadata.get("alias")?;
        let language_alias = metadata_alias.get("language")?;
        let script_alias = metadata_alias.get("script")?;
        let territory_alias = metadata_alias.get("territory")?;
        let variant_alias = metadata_alias.get("variant")?;
        let subdivision_alias = metadata_alias.get("subdivision")?;

        let entry = |b: &UResourceBundle, i: i32| -> Result<(String, String)> {
            let res = b.get_at(i)?;
            let alias_from = res.get_key().unwrap_or_else(|| "null".to_string());
            let alias_to = String::from_utf16_lossy(&res.get("replacement")?.get_string()?);
            Ok((alias_from, alias_to))
        };

        for i in 0..language_alias.get_size() {
            let (alias_from, alias_to) = entry(&language_alias, i)?;
            let test_locale = Locale::of(&alias_from, "", "");
            // if there are script in the aliasFrom
            // or we have both a und as language and a region code.
            if !test_locale.get_script().is_empty()
                || (alias_from.starts_with("und") && !test_locale.get_country().is_empty())
            {
                return Err(LuceneError::illegal_argument(format!(
                    "key [{alias_from}] in alias:language contains unsupported fields combination."
                )));
            }
            language_alias_map.insert(alias_from, alias_to);
        }
        for i in 0..script_alias.get_size() {
            let (alias_from, alias_to) = entry(&script_alias, i)?;
            if len16(&alias_from) != 4 {
                return Err(LuceneError::illegal_argument(format!(
                    "Incorrect key [{alias_from}] in alias:script."
                )));
            }
            script_alias_map.insert(alias_from, alias_to);
        }
        for i in 0..territory_alias.get_size() {
            let (alias_from, alias_to) = entry(&territory_alias, i)?;
            let n = len16(&alias_from);
            if !(2..=3).contains(&n) {
                return Err(LuceneError::illegal_argument(format!(
                    "Incorrect key [{alias_from}] in alias:territory."
                )));
            }
            territory_alias_map.insert(alias_from, java_split(&alias_to, ' '));
        }
        let bad_variant = |s: &str| {
            let n = len16(s);
            !(4..=8).contains(&n) || (n == 4 && !s.as_bytes()[0].is_ascii_digit())
        };
        for i in 0..variant_alias.get_size() {
            let (alias_from, alias_to) = entry(&variant_alias, i)?;
            if bad_variant(&alias_from) {
                return Err(LuceneError::illegal_argument(format!(
                    "Incorrect key [{alias_from}] in alias:variant."
                )));
            }
            if bad_variant(&alias_to) {
                return Err(LuceneError::illegal_argument(format!(
                    "Incorrect variant [{alias_to}] for the key [{alias_from}] in alias:variant."
                )));
            }
            variant_alias_map.insert(alias_from, alias_to);
        }
        for i in 0..subdivision_alias.get_size() {
            let (alias_from, alias_to) = entry(&subdivision_alias, i)?;
            let mut alias_to = match java_split(&alias_to, ' ').into_iter().next() {
                Some(a) => a,
                None => crate::jdk::array_index_out_of_bounds(0),
            };
            let n = len16(&alias_from);
            if !(3..=8).contains(&n) {
                return Err(LuceneError::illegal_argument(format!(
                    "Incorrect key [{alias_from}] in alias:territory."
                )));
            }
            let m = len16(&alias_to);
            if m == 2 {
                // Add 'zzzz' based on changes to UTS #35 for CLDR-14312.
                alias_to.push_str("zzzz");
            } else if !(2..=8).contains(&m) {
                return Err(LuceneError::illegal_argument(format!(
                    "Incorrect value [{alias_to}] in alias:territory."
                )));
            }
            subdivision_alias_map.insert(alias_from, alias_to);
        }

        let data = Arc::new(AliasData {
            language_alias_map,
            script_alias_map,
            territory_alias_map,
            variant_alias_map,
            subdivision_alias_map,
        });
        *guard = Some(data.clone());
        Ok(data)
    }

    fn generate_key(language: &str, region: Option<&str>, variant: Option<&str>) -> String {
        debug_assert!(variant.is_none_or(|v| len16(v) >= 4));
        let mut buf = String::new();
        buf.push_str(language);
        if let Some(r) = region.filter(|r| !r.is_empty()) {
            buf.push(UNDERSCORE);
            buf.push_str(r);
        }
        if let Some(v) = variant.filter(|v| !v.is_empty()) {
            buf.push(UNDERSCORE);
            buf.push_str(v);
        }
        buf
    }

    fn delete_or_replace(input: Option<&str>, ty: Option<&str>, replacement: Option<&str>) -> Option<String> {
        if replacement.is_some_and(|r| !r.is_empty()) {
            if input.is_none_or(str::is_empty) {
                replacement.map(str::to_string)
            } else {
                input.map(str::to_string)
            }
        } else if ty.is_none_or(str::is_empty) {
            input.map(str::to_string)
        } else {
            None
        }
    }

    fn replace_language(&mut self, check_language: bool, check_region: bool, check_variants: bool) -> bool {
        if (check_region && self.region.is_empty()) || (check_variants && self.variants.is_none()) {
            // Nothing to search
            return false;
        }
        let variant_size = if check_variants {
            self.variants.as_ref().map_or(0, Vec::len)
        } else {
            1
        };

        // Since we may have more than one variant, we need to loop through
        // them.
        let search_language = if check_language {
            self.language.clone()
        } else {
            UNDEFINED_LANGUAGE.to_string()
        };
        let search_region = if check_region { Some(self.region.clone()) } else { None };
        let mut search_variant: Option<String> = None;
        for variant_index in 0..variant_size {
            if check_variants {
                search_variant = Some(self.variants.as_ref().expect("checked above")[variant_index].clone());
            }
            if search_variant.as_deref().is_some_and(|v| len16(v) < 4) {
                // Do not consider  ill-formed variant subtag.
                search_variant = None;
            }
            let type_key = Self::generate_key(&search_language, search_region.as_deref(), search_variant.as_deref());
            let replacement = match self.data().language_alias_map.get(&type_key) {
                None => continue, // Found no replacement data.
                Some(r) => r.clone(),
            };

            let mut replaced_script: Option<String> = None;
            let mut replaced_region: Option<String> = None;
            let mut replaced_variant: Option<String> = None;
            let mut replaced_extensions: Option<String> = None;
            let replaced_language: String;

            if !replacement.contains('_') {
                replaced_language = if replacement == UNDEFINED_LANGUAGE {
                    self.language.clone()
                } else {
                    replacement.clone()
                };
            } else {
                let replacement_fields = java_split(&replacement, '_');
                let mut rl = replacement_fields[0].clone();
                let mut index = 1;

                if rl == UNDEFINED_LANGUAGE {
                    rl = self.language.clone();
                }
                replaced_language = rl;
                let mut consumed = len16(&replacement_fields[0]) + 1;
                while replacement_fields.len() > index {
                    let field = &replacement_fields[index];
                    let len = len16(field);
                    if 1 == len {
                        replaced_extensions = Some(substring16_from(&replacement, consumed));
                        break;
                    } else if (2..=3).contains(&len) {
                        debug_assert!(replaced_region.is_none());
                        replaced_region = Some(field.clone());
                    } else if (5..=8).contains(&len) {
                        debug_assert!(replaced_variant.is_none());
                        replaced_variant = Some(field.clone());
                    } else if len == 4 {
                        if field.as_bytes()[0].is_ascii_digit() {
                            debug_assert!(replaced_variant.is_none());
                            replaced_variant = Some(field.clone());
                        } else {
                            debug_assert!(replaced_script.is_none());
                            replaced_script = Some(field.clone());
                        }
                    }
                    index += 1;
                    consumed += len + 1;
                }
            }

            let replaced_script = Self::delete_or_replace(Some(&self.script), None, replaced_script.as_deref());
            let replaced_region =
                Self::delete_or_replace(Some(&self.region), search_region.as_deref(), replaced_region.as_deref());
            let replaced_variant = Self::delete_or_replace(
                search_variant.as_deref(),
                search_variant.as_deref(),
                replaced_variant.as_deref(),
            );

            if self.language == replaced_language
                && Some(&self.script) == replaced_script.as_ref()
                && Some(&self.region) == replaced_region.as_ref()
                && search_variant == replaced_variant
                && replaced_extensions.is_none()
            {
                // Replacement produce no changes on search.
                // For example, apply pa_IN=> pa_Guru_IN on pa_Guru_IN.
                continue;
            }
            self.language = replaced_language;
            // Java stores the possibly null replacements; a null script or
            // region reads as empty in every later use (lscvToID and the
            // isEmpty checks).
            self.script = replaced_script.unwrap_or_default();
            self.region = replaced_region.unwrap_or_default();
            if search_variant.as_deref().is_some_and(|v| !v.is_empty()) {
                let variants = self.variants.as_mut().expect("a searched variant exists");
                if let Some(rv) = replaced_variant.filter(|v| !v.is_empty()) {
                    variants[variant_index] = rv;
                } else {
                    variants.remove(variant_index);
                    if variants.is_empty() {
                        self.variants = None;
                    }
                }
            }
            if replaced_extensions.as_deref().is_some_and(|e| !e.is_empty()) {
                // DO NOTHING
                // UTS35 does not specifiy what should we do if we have extensions in the
                // replacement. Currently we know only the following 4 "BCP47 LegacyRules" have
                // extensions in them languageAlias:
                //  i_default => en_x_i_default
                //  i_enochian => und_x_i_enochian
                //  i_mingo => see_x_i_mingo
                //  zh_min => nan_x_zh_min
                // But all of them are already changed by code inside LanguageTag before
                // hitting this code.
            }

            // Something in search changed by language alias data.
            return true;
        }
        // Nothing changed in search by language alias data.
        false
    }

    fn replace_region(&mut self) -> Result<bool> {
        if self.region.is_empty() {
            return Ok(false);
        }
        let replacement = match self.data().territory_alias_map.get(&self.region) {
            // Found no replacement data for this region.
            None => return Ok(false),
            Some(r) => r.clone(),
        };
        let replaced_region = if replacement.len() > 1 {
            let region_of_language_and_script = ULocale::add_likely_subtags(&ULocale::new3(
                Some(&self.language),
                Some(&self.script),
                None,
            )?)?
            .get_country();
            if replacement.contains(&region_of_language_and_script) {
                region_of_language_and_script
            } else {
                replacement[0].clone()
            }
        } else {
            replacement[0].clone()
        };
        debug_assert!(self.region != replaced_region);
        self.region = replaced_region;
        // The region is changed by data in territory alias.
        Ok(true)
    }

    fn replace_script(&mut self) -> bool {
        if self.script.is_empty() {
            return false;
        }
        let replacement = match self.data().script_alias_map.get(&self.script) {
            // Found no replacement data for this script.
            None => return false,
            Some(r) => r.clone(),
        };
        debug_assert!(self.script != replacement);
        self.script = replacement;
        // The script is changed by data in script alias.
        true
    }

    fn replace_variant(&mut self) -> bool {
        let Some(variants) = self.variants.clone() else {
            return false;
        };
        for (i, variant) in variants.iter().enumerate() {
            let replacement = match self.data().variant_alias_map.get(variant) {
                // Found no replacement data for this variant.
                None => continue,
                Some(r) => r.clone(),
            };
            debug_assert!(len16(&replacement) >= 4);
            debug_assert!(len16(&replacement) <= 8);
            if *variant != replacement {
                let vs = self.variants.as_mut().expect("present above");
                vs[i] = replacement;
                // Special hack to handle hepburn-heploc => alalc97
                if variant == "heploc" {
                    if let Some(p) = vs.iter().position(|v| v == "hepburn") {
                        vs.remove(p);
                    }
                    if vs.is_empty() {
                        self.variants = None;
                    }
                }
                return true;
            }
        }
        false
    }

    fn replace_subdivision(&self, subdivision: &str) -> Option<String> {
        self.data().subdivision_alias_map.get(subdivision).cloned()
    }

    fn replace_transformed_extensions(&self, extensions: &str) -> Result<String> {
        let mut builder = String::new();
        let subtags = java_split(extensions, '-');
        let mut tfields: Vec<String> = Vec::new();
        let mut processed_length = 0;
        let mut tlang_length = 0;
        let mut tkey = String::new();
        for subtag in &subtags {
            if LanguageTag::is_t_key(subtag) {
                if tlang_length == 0 {
                    // Found the first tkey. Record the total length of the preceding
                    // tlang subtags. -1 if there is no tlang before the first tkey.
                    tlang_length = processed_length - 1;
                }
                if !builder.is_empty() {
                    // Finish & store the previous tkey with its tvalue subtags.
                    tfields.push(std::mem::take(&mut builder));
                }
                // Start collecting subtags for this new tkey.
                tkey = subtag.clone();
                builder.push_str(subtag);
            } else if tlang_length != 0 {
                builder.push_str(LanguageTag::SEP);
                builder.push_str(&ULocale::to_unicode_locale_type(&tkey, subtag)?.unwrap_or_else(|| "null".into()));
            }
            processed_length += len16(subtag) + 1;
        }
        if !builder.is_empty() {
            // Finish & store the previous=last tkey with its tvalue subtags.
            tfields.push(std::mem::take(&mut builder));
        }
        let tlang = if tlang_length > 0 {
            crate::r#impl::locale::ascii_util::substring16(extensions, 0, tlang_length)
        } else if tfields.is_empty() {
            extensions.to_string()
        } else {
            String::new()
        };
        if !tlang.is_empty() {
            let canonicalized = ULocale::create_canonical_of(&ULocale::for_language_tag(extensions)?)?.to_language_tag();
            builder.push_str(&AsciiUtil::to_lower_string(&canonicalized));
        }

        if !tfields.is_empty() {
            if !builder.is_empty() {
                builder.push_str(LanguageTag::SEP);
            }
            // tfields are sorted by alphabetical order of their keys
            tfields.sort_by(|a, b| rucene::util::java_collections::java_compare_to(a, b));
            builder.push_str(&tfields.join(LanguageTag::SEP));
        }
        Ok(builder)
    }
}

/// `ULocale.Builder`: builds a `ULocale` from values configured by the
/// setters, validated as BCP 47.
///
/// Equivalent to the class `ULocale.Builder`.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    locbld: InternalLocaleBuilder,
}

/// `new IllformedLocaleException(e.getMessage(), e.getErrorIndex())`.
fn illformed(e: LocaleSyntaxException) -> LuceneError {
    IllformedLocaleException::with_index(e.get_message(), e.get_error_index()).error()
}

impl Builder {
    /// `new Builder()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `setLocale(ULocale locale)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException` when a field of the locale is ill-formed.
    pub fn set_locale(&mut self, locale: &ULocale) -> Result<&mut Self> {
        self.locbld
            .set_locale(&locale.base(), Some(&locale.extensions()))
            .map_err(illformed)?;
        Ok(self)
    }

    /// `setLanguageTag(String languageTag)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException` for an ill-formed tag.
    pub fn set_language_tag(&mut self, language_tag: &str) -> Result<&mut Self> {
        let mut sts = ParseStatus::new();
        let tag = LanguageTag::parse(language_tag, Some(&mut sts));
        if sts.is_error() {
            return Err(IllformedLocaleException::with_index(
                sts.get_error_message().unwrap_or("null"),
                sts.get_error_index(),
            )
            .error());
        }
        self.locbld.set_language_tag(&tag);
        Ok(self)
    }

    /// `setLanguage(String language)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn set_language(&mut self, language: Option<&str>) -> Result<&mut Self> {
        self.locbld.set_language(language).map_err(illformed)?;
        Ok(self)
    }

    /// `setScript(String script)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn set_script(&mut self, script: Option<&str>) -> Result<&mut Self> {
        self.locbld.set_script(script).map_err(illformed)?;
        Ok(self)
    }

    /// `setRegion(String region)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn set_region(&mut self, region: Option<&str>) -> Result<&mut Self> {
        self.locbld.set_region(region).map_err(illformed)?;
        Ok(self)
    }

    /// `setVariant(String variant)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn set_variant(&mut self, variant: Option<&str>) -> Result<&mut Self> {
        self.locbld.set_variant(variant).map_err(illformed)?;
        Ok(self)
    }

    /// `setExtension(char key, String value)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn set_extension(&mut self, key: char, value: Option<&str>) -> Result<&mut Self> {
        self.locbld.set_extension(key, value).map_err(illformed)?;
        Ok(self)
    }

    /// `setUnicodeLocaleKeyword(String key, String type)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn set_unicode_locale_keyword(&mut self, key: &str, ty: Option<&str>) -> Result<&mut Self> {
        self.locbld.set_unicode_locale_keyword(key, ty).map_err(illformed)?;
        Ok(self)
    }

    /// `addUnicodeLocaleAttribute(String attribute)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn add_unicode_locale_attribute(&mut self, attribute: Option<&str>) -> Result<&mut Self> {
        self.locbld.add_unicode_locale_attribute(attribute).map_err(illformed)?;
        Ok(self)
    }

    /// `removeUnicodeLocaleAttribute(String attribute)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException`.
    pub fn remove_unicode_locale_attribute(&mut self, attribute: Option<&str>) -> Result<&mut Self> {
        self.locbld.remove_unicode_locale_attribute(attribute).map_err(illformed)?;
        Ok(self)
    }

    /// `clear()`.
    pub fn clear(&mut self) -> &mut Self {
        self.locbld.clear();
        self
    }

    /// `clearExtensions()`.
    pub fn clear_extensions(&mut self) -> &mut Self {
        self.locbld.clear_extensions();
        self
    }

    /// `build()`.
    ///
    /// # Errors
    ///
    /// The errors of the key-type data lookups.
    pub fn build(&self) -> Result<ULocale> {
        ULocale::get_instance(&self.locbld.get_base_locale(), &self.locbld.get_locale_extensions())
    }
}

/// JDK locale helper: the mapping between `ULocale` and `java.util.Locale`.
///
/// Equivalent to `ULocale.JDKLocaleHelper` (private). Java finds
/// `Locale.Category` by reflection; JDK 25 has it, so
/// `hasLocaleCategories` is `true`, and the category defaults are the JDK
/// `Locale` emulation's (`Locale.getDefault(Category)` /
/// `setDefault(Category, Locale)`).
///
/// **Divergence.** The JDK extensions of a `Locale` are read through its
/// language tag (`toLanguageTag()` parsed by ICU's `LanguageTag`), because
/// the emulation exposes no extension accessors; for a well-formed locale
/// the tag carries the same extension keys and values.
struct JDKLocaleHelper;

impl JDKLocaleHelper {
    fn has_locale_categories() -> bool {
        true
    }

    fn to_u_locale(loc: &Locale) -> Result<ULocale> {
        let mut language = loc.get_language().to_string();
        let script = loc.get_script().to_string();
        let country = loc.get_country().to_string();
        let mut variant = loc.get_variant().to_string();

        let mut attributes: Option<Vec<String>> = None;
        let mut keywords: Option<KeywordMap> = None;

        if loc.has_extensions() {
            let tag = LanguageTag::parse(&loc.to_language_tag(), None);
            let mut bldr = InternalLocaleBuilder::new();
            bldr.set_language_tag(&tag);
            let exts = bldr.get_locale_extensions();
            let ext_keys: Vec<char> = exts.get_keys().collect();
            for ext_key in ext_keys {
                if ext_key == 'u' {
                    // Found Unicode locale extension

                    // attributes
                    let u_attributes = exts.get_unicode_locale_attributes();
                    if !u_attributes.is_empty() {
                        attributes = Some(u_attributes.into_iter().collect());
                    }

                    // keywords
                    for kw_key in exts.get_unicode_locale_keys() {
                        if let Some(kw_val) = exts.get_unicode_locale_type(&kw_key) {
                            if kw_key == "va" {
                                // va-* is interpreted as a variant
                                variant = if variant.is_empty() {
                                    kw_val
                                } else {
                                    format!("{kw_val}_{variant}")
                                };
                            } else {
                                keywords.get_or_insert_with(KeywordMap::default).insert(kw_key, kw_val);
                            }
                        }
                    }
                } else if let Some(ext_val) = exts.get_extension_value(ext_key) {
                    keywords
                        .get_or_insert_with(KeywordMap::default)
                        .insert(ext_key.to_string(), ext_val.to_string());
                }
            }
        }

        // JDK locale no_NO_NY is not interpreted as Nynorsk by ICU,
        // and it should be transformed to nn_NO.

        // Note: JDK7+ unerstand both no_NO_NY and nn_NO. When convert
        // ICU locale to JDK, we do not need to map nn_NO back to no_NO_NY.

        if language == "no" && country == "NO" && variant == "NY" {
            language = "nn".to_string();
            variant = String::new();
        }

        // Constructing ID
        let mut buf = language.clone();

        if !script.is_empty() {
            buf.push('_');
            buf.push_str(&script);
        }

        if !country.is_empty() {
            buf.push('_');
            buf.push_str(&country);
        }

        if !variant.is_empty() {
            if country.is_empty() {
                buf.push('_');
            }
            buf.push('_');
            buf.push_str(&variant);
        }

        if let Some(attributes) = attributes {
            // transform Unicode attributes into a keyword
            let mut attr_buf = String::new();
            for attr in attributes {
                if !attr_buf.is_empty() {
                    attr_buf.push('-');
                }
                attr_buf.push_str(&attr);
            }
            keywords
                .get_or_insert_with(KeywordMap::default)
                .insert(LOCALE_ATTRIBUTE_KEY.to_string(), attr_buf);
        }

        if let Some(keywords) = keywords {
            buf.push('@');
            let mut add_sep = false;
            for (kw_key, kw_val) in keywords.iter() {
                let mut kw_key = kw_key.clone();
                let mut kw_val = kw_val.clone();

                if len16(&kw_key) != 1 {
                    // Unicode locale key
                    kw_key = ULocale::to_legacy_key(&kw_key)?.unwrap_or_else(|| "null".to_string());
                    // use "yes" as the value of typeless keywords
                    kw_val = ULocale::to_legacy_type(&kw_key, if kw_val.is_empty() { "yes" } else { &kw_val })?
                        .unwrap_or_else(|| "null".to_string());
                }

                if add_sep {
                    buf.push(';');
                } else {
                    add_sep = true;
                }
                buf.push_str(&kw_key);
                buf.push('=');
                buf.push_str(&kw_val);
            }
        }

        Ok(ULocale::from_parts(&ULocale::get_name_of(Some(&buf))?, Some(loc.clone())))
    }

    fn to_locale(uloc: &ULocale) -> Locale {
        let mut loc: Option<Locale> = None;
        let uloc_str = uloc.get_name();
        if !uloc.get_script().is_empty() || uloc_str.contains('@') {
            // With script or keywords available, the best way
            // to get a mapped Locale is to go through a language tag.
            // A Locale with script or keywords can only have variants
            // that is 1 to 8 alphanum. If this ULocale has a variant
            // subtag not satisfying the criteria, the variant subtag
            // will be lost.
            let tag = uloc.to_language_tag();

            // Workaround for variant casing problem:
            //
            // The variant field in ICU is case insensitive and normalized
            // to upper case letters by getVariant(), while
            // the variant field in JDK Locale is case sensitive.
            // ULocale#toLanguageTag use lower case characters for
            // BCP 47 variant and private use x-lvariant.
            //
            // Locale#forLanguageTag in JDK preserves character casing
            // for variant. Because ICU always normalizes variant to
            // upper case, we convert language tag to upper case here.
            let tag = AsciiUtil::to_upper_string(&tag);
            // Locale.forLanguageTag: the tag ICU generates is well-formed.
            loc = Locale::builder_from_language_tag(&tag).ok();
        }
        match loc {
            Some(l) => l,
            None => {
                // Without script or keywords, use a Locale constructor,
                // so we can preserve any ill-formed variants.
                Locale::of(&uloc.get_language(), &uloc.get_country(), &uloc.get_variant())
            }
        }
    }

    fn get_default(category: Category) -> Locale {
        Locale::get_default_category(Self::jdk_category(category))
    }

    fn set_default(category: Category, new_locale: Locale) {
        Locale::set_default_category(Self::jdk_category(category), new_locale);
    }

    /// The `Locale.Category` of the same name (`getCategory(Category)`
    /// finds it by name through reflection).
    fn jdk_category(category: Category) -> JdkCategory {
        match category {
            Category::Display => JdkCategory::Display,
            Category::Format => JdkCategory::Format,
        }
    }
}
