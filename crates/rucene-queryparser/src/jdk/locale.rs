/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! `java.util.Locale` as `Locale.Builder.setLanguageTag(tag).build()` and
//! `Locale.of(language, country, variant)` make it, with `toString()`,
//! `toLanguageTag()` and `Locale.getDefault()`.
//!
//! **Divergence.** This is an emulation of the JDK, copied from the one the
//! `rucene-analysis-common` crate holds (`crates/rucene-analysis-common/src/jdk/locale.rs`),
//! which this crate cannot depend on, with `Locale.getDefault()` /
//! `setDefault` from the `rucene-test-framework` one: reproduced from the `Locale`,
//! `Locale.Builder` and `IllformedLocaleException` javadoc and from RFC 5646,
//! and verified against OpenJDK 25. The tag is parsed as
//! `sun.util.locale.LanguageTag.parse` parses it (legacy tags are replaced by
//! their preferred form first), and the locale keeps the language, script,
//! region, variant and extensions the builder keeps.

use std::collections::BTreeMap;
use std::fmt;

use rucene::error::{LuceneError, Result};

/// `java.util.IllformedLocaleException`.
pub const ILLFORMED_LOCALE_EXCEPTION: &str = "java.util.IllformedLocaleException";

/// The legacy (grandfathered) tags `LanguageTag.parse` replaces before
/// parsing, with their preferred forms (RFC 5646 section 2.2.8, as the JDK
/// maps them).
const LEGACY: [(&str, &str); 26] = [
    ("art-lojban", "jbo"),
    ("cel-gaulish", "xtg-x-cel-gaulish"),
    ("en-gb-oed", "en-GB-x-oed"),
    ("i-ami", "ami"),
    ("i-bnn", "bnn"),
    ("i-default", "en-x-i-default"),
    ("i-enochian", "und-x-i-enochian"),
    ("i-hak", "hak"),
    ("i-klingon", "tlh"),
    ("i-lux", "lb"),
    ("i-mingo", "see-x-i-mingo"),
    ("i-navajo", "nv"),
    ("i-pwn", "pwn"),
    ("i-tao", "tao"),
    ("i-tay", "tay"),
    ("i-tsu", "tsu"),
    ("no-bok", "nb"),
    ("no-nyn", "nn"),
    ("sgn-be-fr", "sfb"),
    ("sgn-be-nl", "vgt"),
    ("sgn-ch-de", "sgg"),
    ("zh-guoyu", "cmn"),
    ("zh-hakka", "hak"),
    ("zh-min", "nan-x-zh-min"),
    ("zh-min-nan", "nan"),
    ("zh-xiang", "hsn"),
];

/// A `java.util.Locale`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Locale {
    language: String,
    script: String,
    region: String,
    variant: String,
    /// The extensions by key; `x` is the private use.
    extensions: BTreeMap<char, String>,
}

fn is_alpha(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_alnum(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn is_language(s: &str) -> bool {
    (2..=8).contains(&s.len()) && is_alpha(s)
}

fn is_extlang(s: &str) -> bool {
    s.len() == 3 && is_alpha(s)
}

fn is_script(s: &str) -> bool {
    s.len() == 4 && is_alpha(s)
}

fn is_region(s: &str) -> bool {
    (s.len() == 2 && is_alpha(s)) || (s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()))
}

fn is_variant(s: &str) -> bool {
    let len = s.len();
    ((5..=8).contains(&len) && is_alnum(s))
        || (len == 4 && s.as_bytes()[0].is_ascii_digit() && is_alnum(s))
}

fn is_extension_singleton(s: &str) -> bool {
    s.len() == 1 && is_alnum(s) && !s.eq_ignore_ascii_case("x")
}

fn is_extension_subtag(s: &str) -> bool {
    (2..=8).contains(&s.len()) && is_alnum(s)
}

fn is_privateuse_subtag(s: &str) -> bool {
    (1..=8).contains(&s.len()) && is_alnum(s)
}

/// `sun.util.locale.StringTokenIterator` over `-`: the tokens with their
/// start and end offsets.
fn tokens(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == '-' {
            out.push((start, i));
            start = i + 1;
        }
    }
    out.push((start, s.len()));
    out
}

#[derive(Default)]
struct LanguageTag {
    language: String,
    extlangs: Vec<String>,
    script: String,
    region: String,
    variants: Vec<String>,
    extensions: Vec<String>,
    privateuse: String,
}

/// `LanguageTag.parse(tag, sts)`: the parsed tag, or the error message and
/// index.
fn parse(language_tag: &str) -> std::result::Result<LanguageTag, (String, usize)> {
    let lower = language_tag.to_ascii_lowercase();
    let text: String = LEGACY
        .iter()
        .find(|(k, _)| *k == lower)
        .map_or_else(|| language_tag.to_string(), |(_, v)| (*v).to_string());
    let toks = tokens(&text);
    let mut i = 0;
    let cur = |i: usize| -> Option<&str> { toks.get(i).map(|&(s, e)| &text[s..e]) };
    let mut tag = LanguageTag::default();
    let mut parse_length = 0usize;
    let mut error: Option<(String, usize)> = None;

    if let Some(s) = cur(i) {
        if is_language(s) {
            tag.language = s.to_string();
            parse_length = toks[i].1;
            i += 1;
            for _ in 0..3 {
                match cur(i) {
                    Some(s) if is_extlang(s) => {
                        tag.extlangs.push(s.to_string());
                        parse_length = toks[i].1;
                        i += 1;
                    }
                    _ => break,
                }
            }
            if let Some(s) = cur(i) {
                if is_script(s) {
                    tag.script = s.to_string();
                    parse_length = toks[i].1;
                    i += 1;
                }
            }
            if let Some(s) = cur(i) {
                if is_region(s) {
                    tag.region = s.to_string();
                    parse_length = toks[i].1;
                    i += 1;
                }
            }
            while let Some(s) = cur(i) {
                if !is_variant(s) {
                    break;
                }
                tag.variants.push(s.to_string());
                parse_length = toks[i].1;
                i += 1;
            }
            while let Some(s) = cur(i) {
                if !is_extension_singleton(s) {
                    break;
                }
                let start = toks[i].0;
                let singleton = s.to_string();
                let mut sb = singleton.clone();
                i += 1;
                while let Some(s) = cur(i) {
                    if !is_extension_subtag(s) {
                        break;
                    }
                    sb.push('-');
                    sb.push_str(s);
                    parse_length = toks[i].1;
                    i += 1;
                }
                if parse_length <= start {
                    error = Some((format!("Incomplete extension '{singleton}'"), start));
                    break;
                }
                tag.extensions.push(sb);
            }
        }
    }
    if error.is_none() {
        if let Some(s) = cur(i) {
            if s.eq_ignore_ascii_case("x") {
                let start = toks[i].0;
                let mut sb = s.to_string();
                i += 1;
                while let Some(s) = cur(i) {
                    if !is_privateuse_subtag(s) {
                        break;
                    }
                    sb.push('-');
                    sb.push_str(s);
                    parse_length = toks[i].1;
                    i += 1;
                }
                if parse_length <= start {
                    error = Some(("Incomplete privateuse".to_string(), start));
                } else {
                    tag.privateuse = sb;
                }
            }
        }
    }
    if error.is_none() {
        if let Some(s) = cur(i) {
            let start = toks[i].0;
            error = Some(if s.is_empty() {
                ("Empty subtag".to_string(), start)
            } else {
                (format!("Invalid subtag: {s}"), start)
            });
        }
    }
    match error {
        Some(e) => Err(e),
        None => Ok(tag),
    }
}

/// The canonical ID of a `u` extension value: attributes sorted, then the
/// keywords sorted by key (`UnicodeLocaleExtension`).
fn canonical_unicode_extension(value: &str) -> String {
    let mut attributes: Vec<String> = Vec::new();
    let mut keywords: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for sub in value.split('-') {
        if sub.len() == 2 {
            current = Some(sub.to_string());
            keywords.entry(sub.to_string()).or_default();
        } else if let Some(key) = &current {
            keywords
                .entry(key.clone())
                .or_default()
                .push(sub.to_string());
        } else {
            attributes.push(sub.to_string());
        }
    }
    attributes.sort();
    attributes.dedup();
    let mut parts: Vec<String> = attributes;
    for (key, types) in keywords {
        parts.push(key);
        parts.extend(types);
    }
    parts.join("-")
}

fn title_case(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// `BaseLocale.convertOldISOCodes`: `iw` → `he`, `ji` → `yi`, `in` → `id`.
fn convert_old_iso_code(language: &str) -> String {
    match language {
        "iw" => "he".to_string(),
        "ji" => "yi".to_string(),
        "in" => "id".to_string(),
        other => other.to_string(),
    }
}

impl Locale {
    /// `new Locale.Builder().setLanguageTag(languageTag).build()`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException` (`message [at index i]`), an unchecked
    /// exception, when the tag is not well-formed.
    pub fn builder_from_language_tag(language_tag: &str) -> Result<Locale> {
        let tag = match parse(language_tag) {
            Ok(tag) => tag,
            Err((message, index)) => {
                return Err(LuceneError::unchecked(
                    ILLFORMED_LOCALE_EXCEPTION,
                    format!("{message} [at index {index}]"),
                ))
            }
        };
        // `InternalLocaleBuilder.setLanguageTag(LanguageTag)`.
        let language = if let Some(extlang) = tag.extlangs.first() {
            extlang.clone()
        } else if !tag.language.eq_ignore_ascii_case("und") {
            tag.language.clone()
        } else {
            String::new()
        };
        let mut variant = tag.variants.join("_");
        let mut extensions: BTreeMap<char, String> = BTreeMap::new();
        for ext in &tag.extensions {
            let key = ext.as_bytes()[0].to_ascii_lowercase() as char;
            // ignore duplicates
            if extensions.contains_key(&key) {
                continue;
            }
            let value = ext[2..].to_ascii_lowercase();
            let value = if key == 'u' {
                canonical_unicode_extension(&value)
            } else {
                value
            };
            extensions.insert(key, value);
        }
        if !tag.privateuse.is_empty() {
            extensions.insert('x', tag.privateuse[2..].to_string());
        }
        // `getBaseLocale()`: the private-use subtags after `lvariant` become
        // the variant; `getLocaleExtensions()` drops them from the private use.
        if let Some(privuse) = extensions.get(&'x').cloned() {
            let subtags: Vec<&str> = privuse.split('-').collect();
            if let Some(pos) = subtags
                .iter()
                .position(|s| s.eq_ignore_ascii_case("lvariant"))
            {
                if pos + 1 < subtags.len() {
                    if !variant.is_empty() {
                        variant.push('_');
                    }
                    variant.push_str(&subtags[pos + 1..].join("_"));
                }
                let kept = subtags[..pos].join("-");
                if kept.is_empty() {
                    extensions.remove(&'x');
                } else {
                    extensions.insert('x', kept.to_ascii_lowercase());
                }
            } else {
                extensions.insert('x', privuse.to_ascii_lowercase());
            }
        }
        let language = convert_old_iso_code(&language.to_ascii_lowercase());
        let script = title_case(&tag.script);
        let region = tag.region.to_ascii_uppercase();
        // `Locale.getInstance(baseloc, extensions)`: the compatibility
        // extensions of `ja_JP_JP` and `th_TH_TH`.
        if extensions.is_empty() {
            if language == "ja" && script.is_empty() && region == "JP" && variant == "JP" {
                extensions.insert('u', "ca-japanese".to_string());
            } else if language == "th" && script.is_empty() && region == "TH" && variant == "TH" {
                extensions.insert('u', "nu-thai".to_string());
            }
        }
        Ok(Locale {
            language,
            script,
            region,
            variant,
            extensions,
        })
    }

    /// `Locale.ENGLISH`: language `en`.
    pub fn english() -> Locale {
        Locale {
            language: "en".to_string(),
            ..Locale::default()
        }
    }

    /// `hasExtensions()`.
    pub fn has_extensions(&self) -> bool {
        !self.extensions.is_empty()
    }

    /// `Locale.US`: language `en`, country `US`.
    pub fn us() -> Locale {
        Locale {
            language: "en".to_string(),
            region: "US".to_string(),
            ..Locale::default()
        }
    }

    /// `Locale.getInstance(language, script, country, variant, null)`: a
    /// locale of these fields, as given, with the compatibility extension
    /// `Locale.getCompatibilityExtensions` gives `ja_JP_JP` (`u-ca-japanese`)
    /// and `th_TH_TH` (`u-nu-thai`).
    pub fn of_parts(language: &str, script: &str, region: &str, variant: &str) -> Locale {
        let mut extensions = BTreeMap::new();
        if script.is_empty() {
            if language == "ja" && region == "JP" && variant == "JP" {
                extensions.insert('u', "ca-japanese".to_string());
            } else if language == "th" && region == "TH" && variant == "TH" {
                extensions.insert('u', "nu-thai".to_string());
            }
        }
        Locale {
            language: language.to_string(),
            script: script.to_string(),
            region: region.to_string(),
            variant: variant.to_string(),
            extensions,
        }
    }

    /// `equals(Locale.ROOT)`: every field empty, and no extension.
    pub fn is_root(&self) -> bool {
        self.language.is_empty()
            && self.script.is_empty()
            && self.region.is_empty()
            && self.variant.is_empty()
            && self.extensions.is_empty()
    }

    /// `stripExtensions()`: this locale without its extensions.
    pub fn strip_extensions(&self) -> Locale {
        Locale {
            extensions: BTreeMap::new(),
            ..self.clone()
        }
    }

    /// `getUnicodeLocaleType(key)`: the type of the Unicode locale keyword
    /// `key` (`""` for a keyword without a type), `None` when the `u`
    /// extension has no such keyword.
    pub fn get_unicode_locale_type(&self, key: &str) -> Option<String> {
        let value = self.extensions.get(&'u')?;
        let subtags: Vec<&str> = value.split('-').collect();
        let mut i = 0;
        while i < subtags.len() {
            if subtags[i].len() == 2 {
                let k = subtags[i];
                let mut j = i + 1;
                while j < subtags.len() && subtags[j].len() != 2 {
                    j += 1;
                }
                if k == key {
                    return Some(subtags[i + 1..j].join("-"));
                }
                i = j;
            } else {
                i += 1;
            }
        }
        None
    }

    /// `new Locale.Builder().setLocale(this).setRegion(region).build()`, as
    /// `CalendarDataUtility.findRegionOverride` builds the region override.
    /// `InternalLocaleBuilder.setLocale` drops the compatibility variant of
    /// `ja_JP_JP` and `th_TH_TH`, maps `no_NO_NY` to `nn_NO`, and refuses
    /// any field that is not well-formed; the extensions are kept.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` (an unchecked exception) Java's
    /// `setLocale` throws: `Ill-formed language: <language> [at index 0]`,
    /// likewise for the script and the region, and `Ill-formed variant:
    /// <variant> [at index <i>]`, `i` the start of the first ill-formed
    /// subtag.
    pub fn with_region(&self, region: &str) -> Result<Locale> {
        let mut language = self.language.clone();
        let mut variant = self.variant.clone();
        if (language == "ja" && self.region == "JP" && variant == "JP")
            || (language == "th" && self.region == "TH" && variant == "TH")
        {
            variant.clear();
        } else if language == "no" && self.region == "NO" && variant == "NY" {
            language = "nn".to_string();
            variant.clear();
        }
        let ill_formed =
            |message: String| Err(LuceneError::unchecked(ILLFORMED_LOCALE_EXCEPTION, message));
        if !language.is_empty() && !is_language(&language) {
            return ill_formed(format!("Ill-formed language: {language} [at index 0]"));
        }
        if !self.script.is_empty() && !is_script(&self.script) {
            return ill_formed(format!("Ill-formed script: {} [at index 0]", self.script));
        }
        if !self.region.is_empty() && !is_region(&self.region) {
            return ill_formed(format!("Ill-formed region: {} [at index 0]", self.region));
        }
        if !variant.is_empty() {
            let mut index = 0;
            for subtag in variant.split('_') {
                if !is_variant(subtag) {
                    return ill_formed(format!("Ill-formed variant: {variant} [at index {index}]"));
                }
                index += subtag.len() + 1;
            }
        }
        Ok(Locale {
            language,
            region: region.to_string(),
            variant,
            ..self.clone()
        })
    }

    /// `getLanguage()`.
    pub fn get_language(&self) -> &str {
        &self.language
    }

    /// `getScript()`.
    pub fn get_script(&self) -> &str {
        &self.script
    }

    /// `getCountry()`.
    pub fn get_country(&self) -> &str {
        &self.region
    }

    /// `getVariant()`.
    pub fn get_variant(&self) -> &str {
        &self.variant
    }

    /// `LocaleExtensions.getID()`: the extensions by key, the private use
    /// last.
    fn extensions_id(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (key, value) in &self.extensions {
            if *key != 'x' {
                parts.push(format!("{key}-{value}"));
            }
        }
        if let Some(privuse) = self.extensions.get(&'x') {
            parts.push(format!("x-{privuse}"));
        }
        parts.join("-")
    }

    /// `toLanguageTag()`.
    pub fn to_language_tag(&self) -> String {
        let mut language = String::new();
        if is_language(&self.language) {
            language = convert_old_iso_code(&self.language);
        }
        let mut has_subtag = false;
        let script = if is_script(&self.script) {
            has_subtag = true;
            title_case(&self.script)
        } else {
            String::new()
        };
        let region = if is_region(&self.region) {
            has_subtag = true;
            self.region.to_ascii_uppercase()
        } else {
            String::new()
        };
        let mut variant = self.variant.clone();
        // Special handling for no_NO_NY - use nn_NO for language tag.
        if language == "no" && region == "NO" && variant == "NY" {
            language = "nn".to_string();
            variant.clear();
        }
        let mut variants: Vec<String> = Vec::new();
        let mut privuse_var: Option<String> = None;
        if !variant.is_empty() {
            let parts: Vec<&str> = variant.split('_').collect();
            let mut k = 0;
            while k < parts.len() && is_variant(parts[k]) {
                variants.push(parts[k].to_string());
                k += 1;
            }
            if !variants.is_empty() {
                has_subtag = true;
            }
            if k < parts.len() {
                let mut kept: Vec<&str> = Vec::new();
                while k < parts.len() && is_privateuse_subtag(parts[k]) {
                    kept.push(parts[k]);
                    k += 1;
                }
                if !kept.is_empty() {
                    privuse_var = Some(kept.join("-"));
                }
            }
        }
        let mut extensions: Vec<String> = Vec::new();
        let mut privateuse: Option<String> = None;
        for (key, value) in &self.extensions {
            if *key == 'x' {
                privateuse = Some(value.clone());
            } else {
                extensions.push(format!("{key}-{value}"));
            }
        }
        if !extensions.is_empty() {
            has_subtag = true;
        }
        if let Some(var) = privuse_var {
            privateuse = Some(match privateuse {
                None => format!("lvariant-{var}"),
                Some(p) => format!("{p}-lvariant-{var}"),
            });
        }
        if language.is_empty() && (has_subtag || privateuse.is_none()) {
            language = "und".to_string();
        }
        let mut buf: Vec<String> = Vec::new();
        if !language.is_empty() {
            buf.push(language.to_ascii_lowercase());
        }
        if !script.is_empty() {
            buf.push(script);
        }
        if !region.is_empty() {
            buf.push(region);
        }
        buf.extend(variants);
        buf.extend(extensions);
        if let Some(p) = privateuse {
            buf.push(format!("x-{p}"));
        }
        buf.join("-")
    }
}

impl fmt::Display for Locale {
    /// `toString()`: `language_COUNTRY_VARIANT_#Script-extensions`, with the
    /// separators Java writes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let l = !self.language.is_empty();
        let s = !self.script.is_empty();
        let r = !self.region.is_empty();
        let v = !self.variant.is_empty();
        let ext = self.extensions_id();
        let e = !ext.is_empty();
        let mut result = self.language.clone();
        if r || (l && (v || s || e)) {
            result.push('_');
            result.push_str(&self.region);
        }
        if v && (l || r) {
            result.push('_');
            result.push_str(&self.variant);
        }
        if s && (l || r) {
            result.push_str("_#");
            result.push_str(&self.script);
        }
        if e && (l || r) {
            result.push('_');
            if !s {
                result.push('#');
            }
            result.push_str(&ext);
        }
        f.write_str(&result)
    }
}

impl Locale {
    /// `Locale.ROOT`.
    pub fn root() -> Locale {
        Locale::default()
    }

    /// `Locale.of(String language, String country, String variant)`: the
    /// language lower-cased (with `convertOldISOCodes`), the country
    /// upper-cased, the variant as given, and the compatibility extension of
    /// `ja_JP_JP` and `th_TH_TH`.
    pub fn of(language: &str, country: &str, variant: &str) -> Locale {
        let language = convert_old_iso_code(&language.to_ascii_lowercase());
        let country = country.to_ascii_uppercase();
        Locale::of_parts(&language, "", &country, variant)
    }

    /// `Locale.getDefault()`: the locale [`set_default`](Self::set_default)
    /// set, else the JVM's startup default (see [`startup_default`]).
    pub fn get_default() -> Locale {
        if let Some(locale) = DEFAULT_LOCALE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return locale;
        }
        startup_default().clone()
    }

    /// `Locale.setDefault(Locale)`; `None` restores the startup default.
    pub fn set_default(locale: Option<Locale>) {
        *DEFAULT_LOCALE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = locale;
    }
}

/// `Locale.setDefault`'s value, `None` until set.
static DEFAULT_LOCALE: std::sync::RwLock<Option<Locale>> = std::sync::RwLock::new(None);

/// The default locale a JVM starts with on Linux: `user.language`,
/// `user.script`, `user.country` and `user.variant` as `java_props_md.c`
/// (`ParseLocale` of `LC_MESSAGES`, after `setlocale(LC_ALL, "")`) derives
/// them, computed once.
///
/// **Divergence.** The JVM system properties `user.language` and its kin
/// have no counterpart, and glibc's `setlocale` is not called: the category
/// is resolved from the environment as glibc resolves it (`LC_ALL`, then
/// `LC_MESSAGES`, then `LANG`, a missing or empty one meaning `C`), without
/// glibc's check that the named locale is installed (an uninstalled one makes
/// the JVM fall back to `C`, which is `en_US`).
pub fn startup_default() -> &'static Locale {
    static DEFAULT: std::sync::LazyLock<Locale> = std::sync::LazyLock::new(|| {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let lc = env("LC_ALL")
            .or_else(|| env("LC_MESSAGES"))
            .or_else(|| env("LANG"))
            .unwrap_or_else(|| "C".to_string());
        parse_posix_locale(&lc)
    });
    &DEFAULT
}

/// `ParseLocale` of `java_props_md.c` on Linux: `language_country.encoding@variant`
/// through the `locale_aliases`, `language_names`, `country_names`,
/// `script_names` and `variant_names` tables of `locale_str.h` (data, read as
/// specification), then `new Locale(...)` as `Locale.initDefault` builds it.
fn parse_posix_locale(lc: &str) -> Locale {
    const LOCALE_ALIASES: &[(&str, &str)] = &[
        ("ar", "ar_EG"),
        ("be", "be_BY"),
        ("bg", "bg_BG"),
        ("br", "br_FR"),
        ("ca", "ca_ES"),
        ("cs", "cs_CZ"),
        ("cz", "cs_CZ"),
        ("da", "da_DK"),
        ("de", "de_DE"),
        ("el", "el_GR"),
        ("en", "en_US"),
        ("eo", "eo"),
        ("es", "es_ES"),
        ("et", "et_EE"),
        ("eu", "eu_ES"),
        ("fi", "fi_FI"),
        ("fr", "fr_FR"),
        ("ga", "ga_IE"),
        ("gl", "gl_ES"),
        ("he", "iw_IL"),
        ("hr", "hr_HR"),
        ("hs", "en_US"),
        ("hu", "hu_HU"),
        ("id", "in_ID"),
        ("in", "in_ID"),
        ("is", "is_IS"),
        ("it", "it_IT"),
        ("iw", "iw_IL"),
        ("ja", "ja_JP"),
        ("kl", "kl_GL"),
        ("ko", "ko_KR"),
        ("lt", "lt_LT"),
        ("lv", "lv_LV"),
        ("mk", "mk_MK"),
        ("nl", "nl_NL"),
        ("no", "no_NO"),
        ("pl", "pl_PL"),
        ("pt", "pt_PT"),
        ("ro", "ro_RO"),
        ("ru", "ru_RU"),
        ("se", "se_NO"),
        ("sk", "sk_SK"),
        ("sl", "sl_SI"),
        ("sq", "sq_AL"),
        ("sr", "sr_CS"),
        ("su", "fi_FI"),
        ("sv", "sv_SE"),
        ("th", "th_TH"),
        ("tr", "tr_TR"),
        ("ua", "en_US"),
        ("uk", "uk_UA"),
        ("vi", "vi_VN"),
        ("wa", "wa_BE"),
        ("zh", "zh_CN"),
        ("bokmal", "nb_NO"),
        ("bokm\u{e5}l", "nb_NO"),
        ("catalan", "ca_ES"),
        ("croatian", "hr_HR"),
        ("czech", "cs_CZ"),
        ("danish", "da_DK"),
        ("dansk", "da_DK"),
        ("deutsch", "de_DE"),
        ("dutch", "nl_NL"),
        ("eesti", "et_EE"),
        ("estonian", "et_EE"),
        ("finnish", "fi_FI"),
        ("fran\u{e7}ais", "fr_FR"),
        ("french", "fr_FR"),
        ("galego", "gl_ES"),
        ("galician", "gl_ES"),
        ("german", "de_DE"),
        ("greek", "el_GR"),
        ("hebrew", "iw_IL"),
        ("hrvatski", "hr_HR"),
        ("hungarian", "hu_HU"),
        ("icelandic", "is_IS"),
        ("italian", "it_IT"),
        ("japanese", "ja_JP"),
        ("korean", "ko_KR"),
        ("lithuanian", "lt_LT"),
        ("norwegian", "no_NO"),
        ("nynorsk", "nn_NO"),
        ("polish", "pl_PL"),
        ("portuguese", "pt_PT"),
        ("romanian", "ro_RO"),
        ("russian", "ru_RU"),
        ("slovak", "sk_SK"),
        ("slovene", "sl_SI"),
        ("slovenian", "sl_SI"),
        ("spanish", "es_ES"),
        ("swedish", "sv_SE"),
        ("thai", "th_TH"),
        ("turkish", "tr_TR"),
    ];
    const LANGUAGE_NAMES: &[(&str, &str)] = &[
        ("C", "en"),
        ("POSIX", "en"),
        ("cz", "cs"),
        ("he", "iw"),
        ("hs", "en"),
        ("id", "in"),
        ("sh", "sr"),
        ("su", "fi"),
        ("ua", "en"),
        ("catalan", "ca"),
        ("croatian", "hr"),
        ("czech", "cs"),
        ("danish", "da"),
        ("dansk", "da"),
        ("deutsch", "de"),
        ("dutch", "nl"),
        ("finnish", "fi"),
        ("fran\u{e7}ais", "fr"),
        ("french", "fr"),
        ("german", "de"),
        ("greek", "el"),
        ("hebrew", "he"),
        ("hrvatski", "hr"),
        ("hungarian", "hu"),
        ("icelandic", "is"),
        ("italian", "it"),
        ("japanese", "ja"),
        ("norwegian", "no"),
        ("polish", "pl"),
        ("portuguese", "pt"),
        ("romanian", "ro"),
        ("russian", "ru"),
        ("slovak", "sk"),
        ("slovene", "sl"),
        ("slovenian", "sl"),
        ("spanish", "es"),
        ("swedish", "sv"),
        ("turkish", "tr"),
    ];
    const SCRIPT_NAMES: &[(&str, &str)] = &[
        ("cyrillic", "Cyrl"),
        ("devanagari", "Deva"),
        ("iqtelif", "Latn"),
        ("latin", "Latn"),
        ("Arab", "Arab"),
        ("Cyrl", "Cyrl"),
        ("Deva", "Deva"),
        ("Ethi", "Ethi"),
        ("Hans", "Hans"),
        ("Hant", "Hant"),
        ("Latn", "Latn"),
        ("Sund", "Sund"),
        ("Syrc", "Syrc"),
        ("Tfng", "Tfng"),
    ];
    const COUNTRY_NAMES: &[(&str, &str)] = &[("RN", "US"), ("YU", "CS")];
    const VARIANT_NAMES: &[(&str, &str)] = &[("nynorsk", "NY")];
    let lookup = |map: &[(&str, &str)], key: &str| -> Option<String> {
        map.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    };
    let lc = if lc == "C" || lc == "POSIX" {
        "en_US"
    } else {
        lc
    };
    let split_encoding_variant = |t: &str| -> (String, String) {
        if let Some(p) = t.find('.') {
            (t[..p].to_string(), t[p..].to_string())
        } else if let Some(p) = t.find('@') {
            (t[..p].to_string(), t[p..].to_string())
        } else {
            (t.to_string(), String::new())
        }
    };
    let (mut temp, mut encoding_variant) = split_encoding_variant(lc);
    if let Some(alias) = lookup(LOCALE_ALIASES, &temp) {
        let (t, ev) = split_encoding_variant(&alias);
        temp = t;
        if !ev.is_empty() {
            encoding_variant = ev;
        }
    }
    let (language, country) = match temp.find('_') {
        Some(p) => (temp[..p].to_string(), Some(temp[p + 1..].to_string())),
        None => (temp.clone(), None),
    };
    let mut p = encoding_variant.as_str();
    if let Some(e) = p.find('.') {
        p = &p[e + 1..];
    }
    let variant = p.find('@').map(|v| p[v + 1..].to_string());
    let std_language = lookup(LANGUAGE_NAMES, &language).unwrap_or(language);
    let std_country = country
        .map(|c| lookup(COUNTRY_NAMES, &c).unwrap_or(c))
        .unwrap_or_default();
    let (std_script, std_variant) = match &variant {
        Some(v) => (
            lookup(SCRIPT_NAMES, v).unwrap_or_default(),
            lookup(VARIANT_NAMES, v).unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    };
    // `Locale.initDefault()`: `getInstance(language, script, country,
    // variant, extensions)`, the language converted from its old ISO code
    let language = convert_old_iso_code(&std_language.to_ascii_lowercase());
    Locale::of_parts(
        &language,
        &std_script,
        &std_country.to_ascii_uppercase(),
        &std_variant,
    )
}
