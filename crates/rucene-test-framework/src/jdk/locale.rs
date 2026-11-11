//! `java.util.Locale` as `Locale.Builder.setLanguageTag(tag).build()` makes
//! it, with `toString()` and `toLanguageTag()`.
//!
//! **Divergence.** This is an emulation of the JDK, reproduced from the
//! `Locale`, `Locale.Builder` and `IllformedLocaleException` javadoc and from
//! RFC 5646, and verified against OpenJDK 25 (`ltc_locales.tsv`): the tag is
//! parsed as `sun.util.locale.LanguageTag.parse` parses it (legacy tags are
//! replaced by their preferred form first), and the locale keeps the
//! language, script, region, variant and extensions the builder keeps. The
//! locale data behind a locale (display names, the CLDR) is not available.

use std::collections::BTreeMap;
use std::fmt;

use randomizedtesting::jdk::exception::{register_class, Throwable, RUNTIME_EXCEPTION};

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
fn parse(language_tag: &str) -> Result<LanguageTag, (String, usize)> {
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
    /// # Panics
    ///
    /// `IllformedLocaleException` (`message [at index i]`) when the tag is not
    /// well-formed.
    pub fn builder_from_language_tag(language_tag: &str) -> Locale {
        let tag = match parse(language_tag) {
            Ok(tag) => tag,
            Err((message, index)) => {
                register_class(ILLFORMED_LOCALE_EXCEPTION, RUNTIME_EXCEPTION);
                Throwable::new(
                    ILLFORMED_LOCALE_EXCEPTION,
                    format!("{message} [at index {index}]"),
                )
                .throw()
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
        Locale {
            language,
            script,
            region,
            variant,
            extensions,
        }
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

/// `Locale.getDefault()`, as `Locale.setDefault` set it.
static DEFAULT_LOCALE: std::sync::RwLock<Option<Locale>> = std::sync::RwLock::new(None);

/// `Locale.getDefault()`: the default `TestRuleSetupAndRestoreClassEnv` set.
///
/// **Divergence.** A Rust process has no JVM default locale: before one is
/// set, this is `None` where Java returns the platform's.
pub fn get_default() -> Option<Locale> {
    DEFAULT_LOCALE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// `Locale.setDefault(locale)`; `None` restores the unset state.
pub fn set_default(locale: Option<Locale>) {
    *DEFAULT_LOCALE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = locale;
}
