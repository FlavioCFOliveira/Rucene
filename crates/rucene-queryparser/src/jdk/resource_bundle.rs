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

//! `java.util.ResourceBundle`, `java.util.PropertyResourceBundle` and
//! `java.util.MissingResourceException`: the part the flexible query
//! parser's `NLS` uses, `ResourceBundle.getBundle(String, Locale)` with the
//! default `Control` and `getObject(String)`, over `.properties` bundles.
//!
//! **Divergence.** An emulation of JDK 25 (`ResourceBundle`,
//! `ResourceBundle.Control`, `PropertyResourceBundle`, `Properties.load` and
//! `MissingResourceException` read as specification), not a port of a
//! Lucene file. What differs, and why:
//!
//! * **Where bundles come from.** Java finds a bundle on the class path, as
//!   a class (format `java.class`) and then as the resource
//!   `<base/name>_<locale>.properties` (format `java.properties`). A Rust
//!   crate has no class path: a bundle is the text of a `.properties`
//!   resource registered under its bundle name (`toBundleName`, the base name
//!   then `_` and the locale), by this crate for its own resources and by
//!   any caller with [`ResourceBundle::register_properties`]. Class bundles
//!   (`ListResourceBundle` subclasses) have no counterpart.
//! * **The cache.** Java caches loaded bundles, with soft references the
//!   collector may clear. The lookup the cache short-circuits gives the same
//!   bundle for the same base name, locale and default locale, so the port
//!   recomputes it on each call and only memoizes the parsed resources;
//!   bundle identity (`==` between two `getBundle` results) is not kept.
//! * **Encoding.** `PropertyResourceBundle(InputStream)` decodes UTF-8 and
//!   switches to ISO-8859-1 when it meets a malformed sequence, re-reading
//!   the current buffer of the reader (8192 bytes); the port re-reads the
//!   whole resource, which is the same for resources up to that size.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind, Result};

use super::locale::Locale;

/// The binary name of `java.util.MissingResourceException`.
const MISSING_RESOURCE_EXCEPTION: &str = "java.util.MissingResourceException";

/// A `java.util.MissingResourceException`, carried in
/// [`LuceneErrorKind::Throwable`].
///
/// Equivalent to the class: a `RuntimeException` with the class name and
/// key of the missing resource.
#[derive(Debug, Clone)]
pub struct MissingResourceException {
    message: String,
    class_name: String,
    key: String,
}

impl MissingResourceException {
    /// `new MissingResourceException(String s, String className, String key)`.
    pub fn new(
        message: impl Into<String>,
        class_name: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            message: message.into(),
            class_name: class_name.into(),
            key: key.into(),
        }
    }

    /// `getClassName()`.
    pub fn get_class_name(&self) -> &str {
        &self.class_name
    }

    /// `getKey()`.
    pub fn get_key(&self) -> &str {
        &self.key
    }

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }

    /// The `MissingResourceException` `error` stands for, when it is one.
    pub fn of(error: &LuceneError) -> Option<&MissingResourceException> {
        match error.kind() {
            LuceneErrorKind::Throwable(t) => t.as_any().downcast_ref(),
            _ => None,
        }
    }
}

impl JavaThrowable for MissingResourceException {
    fn java_class(&self) -> &'static str {
        MISSING_RESOURCE_EXCEPTION
    }

    fn get_message(&self) -> Option<&str> {
        Some(&self.message)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The registered `.properties` resources, by bundle name, with their parsed
/// key-value pairs once a lookup has read them.
struct Registry {
    resources: HashMap<String, &'static [u8]>,
    parsed: HashMap<String, Arc<HashMap<String, String>>>,
}

/// The resources of this crate: `QueryParserMessages.properties`.
static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| {
    let mut resources: HashMap<String, &'static [u8]> = HashMap::new();
    resources.insert(
        crate::flexible::core::messages::query_parser_messages::BUNDLE_NAME.to_string(),
        crate::flexible::core::messages::query_parser_messages::PROPERTIES,
    );
    Mutex::new(Registry {
        resources,
        parsed: HashMap::new(),
    })
});

/// A resource bundle loaded from a `.properties` resource, with its parent
/// chain.
///
/// Equivalent to the `PropertyResourceBundle` that `ResourceBundle.getBundle`
/// returns: its key-value pairs (`handleGetObject`), its parent and its
/// locale.
#[derive(Debug)]
pub struct ResourceBundle {
    lookup: Arc<HashMap<String, String>>,
    parent: Option<Arc<ResourceBundle>>,
    locale: Locale,
}

impl ResourceBundle {
    /// Registers the text of the `.properties` resource of the bundle
    /// `bundle_name` (`toBundleName(baseName, locale)`, for example
    /// `a.b.Messages_ja`), as putting `a/b/Messages_ja.properties` on the
    /// class path does; a later registration of a name replaces it.
    pub fn register_properties(bundle_name: &str, contents: &'static [u8]) {
        let mut registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
        registry.parsed.remove(bundle_name);
        registry.resources.insert(bundle_name.to_string(), contents);
    }

    /// Gets a resource bundle using the specified base name and locale, and
    /// the default `Control`.
    ///
    /// Equivalent to `ResourceBundle.getBundle(String baseName, Locale
    /// locale)`: the candidate locales of `locale` (`Control.
    /// getCandidateLocales`), then those of the default locale
    /// (`Control.getFallbackLocale`), each bundle found getting the next
    /// candidate's bundle as its parent; the base bundle (`Locale.ROOT`) is
    /// held back while a fallback locale may still give a more specific one.
    ///
    /// # Errors
    ///
    /// `MissingResourceException: Can't find bundle for base name <base>,
    /// locale <locale>` when no bundle is found; the
    /// `IllegalArgumentException("Malformed \\uxxxx encoding.")` of
    /// `Properties.load` for a malformed resource.
    pub fn get_bundle(base_name: &str, locale: &Locale) -> Result<Arc<ResourceBundle>> {
        let mut base_bundle: Option<Arc<ResourceBundle>> = None;
        let mut bundle: Option<Arc<ResourceBundle>> = None;
        let mut target_locale = Some(locale.clone());
        while let Some(target) = target_locale {
            let candidate_locales = candidate_locales(&target);
            bundle = find_bundle(base_name, &candidate_locales, 0, base_bundle.as_ref())?;
            // If the loaded bundle is the base bundle and exactly for the
            // requested locale or the only candidate locale, then take the
            // bundle as the resulting one. If the loaded bundle is the base
            // bundle, it's put on hold until we finish processing all
            // fallback locales.
            if let Some(found) = &bundle {
                let is_base_bundle = found.locale.is_root();
                if !is_base_bundle
                    || found.locale == *locale
                    || (candidate_locales.len() == 1 && found.locale == candidate_locales[0])
                {
                    break;
                }
                if is_base_bundle && base_bundle.is_none() {
                    base_bundle = Some(Arc::clone(found));
                }
            }
            target_locale = fallback_locale(&target);
        }
        match bundle {
            Some(bundle) => Ok(bundle),
            None => match base_bundle {
                Some(base_bundle) => Ok(base_bundle),
                None => Err(MissingResourceException::new(
                    format!("Can't find bundle for base name {base_name}, locale {locale}"),
                    format!("{base_name}_{locale}"),
                    "",
                )
                .error()),
            },
        }
    }

    /// Gets an object for the given key from this resource bundle or one of
    /// its parents.
    ///
    /// Equivalent to `ResourceBundle.getObject(String key)` of a
    /// `PropertyResourceBundle`, whose values are `String`s.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `null` key (`handleGetObject`), and
    /// `MissingResourceException: Can't find resource for bundle
    /// java.util.PropertyResourceBundle, key <key>` when neither this bundle
    /// nor a parent holds the key.
    pub fn get_object(&self, key: Option<&str>) -> Result<String> {
        let key = key.ok_or_else(LuceneError::null_pointer)?;
        let mut bundle = Some(self);
        while let Some(b) = bundle {
            if let Some(value) = b.lookup.get(key) {
                return Ok(value.clone());
            }
            bundle = b.parent.as_deref();
        }
        Err(MissingResourceException::new(
            format!("Can't find resource for bundle java.util.PropertyResourceBundle, key {key}"),
            "java.util.PropertyResourceBundle",
            key,
        )
        .error())
    }

    /// Equivalent to `ResourceBundle.getString(String key)`: `getObject`, a
    /// `String` for a `PropertyResourceBundle`.
    ///
    /// # Errors
    ///
    /// As [`get_object`](Self::get_object).
    pub fn get_string(&self, key: Option<&str>) -> Result<String> {
        self.get_object(key)
    }

    /// Equivalent to `ResourceBundle.getLocale()`: the candidate locale the
    /// bundle was loaded for.
    pub fn get_locale(&self) -> &Locale {
        &self.locale
    }
}

/// `findBundle`: the bundle of `candidates[index]` with its parent chain
/// (the bundles of the following candidates), or the parent when there is
/// no bundle for that locale.
fn find_bundle(
    base_name: &str,
    candidates: &[Locale],
    index: usize,
    base_bundle: Option<&Arc<ResourceBundle>>,
) -> Result<Option<Arc<ResourceBundle>>> {
    let target_locale = &candidates[index];
    let parent = if index != candidates.len() - 1 {
        find_bundle(base_name, candidates, index + 1, base_bundle)?
    } else if let (Some(base_bundle), true) = (base_bundle, target_locale.is_root()) {
        return Ok(Some(Arc::clone(base_bundle)));
    } else {
        None
    };
    match load_properties(&to_bundle_name(base_name, target_locale))? {
        Some(lookup) => Ok(Some(Arc::new(ResourceBundle {
            lookup,
            parent,
            locale: target_locale.clone(),
        }))),
        None => Ok(parent),
    }
}

/// The parsed resource of `bundle_name`, `None` when none is registered.
fn load_properties(bundle_name: &str) -> Result<Option<Arc<HashMap<String, String>>>> {
    let mut registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(parsed) = registry.parsed.get(bundle_name) {
        return Ok(Some(Arc::clone(parsed)));
    }
    let Some(contents) = registry.resources.get(bundle_name).copied() else {
        return Ok(None);
    };
    let parsed = Arc::new(load(&decode(contents))?);
    registry
        .parsed
        .insert(bundle_name.to_string(), Arc::clone(&parsed));
    Ok(Some(parsed))
}

/// `Control.toBundleName(String baseName, Locale locale)`.
pub fn to_bundle_name(base_name: &str, locale: &Locale) -> String {
    let language = locale.get_language();
    let script = locale.get_script();
    let country = locale.get_country();
    let variant = locale.get_variant();
    if language.is_empty() && country.is_empty() && variant.is_empty() {
        return base_name.to_string();
    }
    let mut sb = String::from(base_name);
    sb.push('_');
    if !script.is_empty() {
        if !variant.is_empty() {
            sb.push_str(&format!("{language}_{script}_{country}_{variant}"));
        } else if !country.is_empty() {
            sb.push_str(&format!("{language}_{script}_{country}"));
        } else {
            sb.push_str(&format!("{language}_{script}"));
        }
    } else if !variant.is_empty() {
        sb.push_str(&format!("{language}_{country}_{variant}"));
    } else if !country.is_empty() {
        sb.push_str(&format!("{language}_{country}"));
    } else {
        sb.push_str(language);
    }
    sb
}

/// `Control.getFallbackLocale(String baseName, Locale locale)`: the default
/// locale, unless `locale` is it.
fn fallback_locale(locale: &Locale) -> Option<Locale> {
    let default_locale = Locale::get_default();
    if *locale == default_locale {
        None
    } else {
        Some(default_locale)
    }
}

/// `Control.getCandidateLocales(String baseName, Locale locale)` over the
/// base locale (language, script, region, variant) of `locale`, with the
/// Norwegian and Chinese special cases of `createCandidateList`.
pub fn candidate_locales(locale: &Locale) -> Vec<Locale> {
    let language = locale.get_language().to_string();
    let mut script = locale.get_script().to_string();
    let region = locale.get_country().to_string();
    let mut variant = locale.get_variant().to_string();

    // Special handling for Norwegian
    let mut is_norwegian_bokmal = false;
    let mut is_norwegian_nynorsk = false;
    if language == "no" {
        if region == "NO" && variant == "NY" {
            variant = String::new();
            is_norwegian_nynorsk = true;
        } else {
            is_norwegian_bokmal = true;
        }
    }
    if language == "nb" || is_norwegian_bokmal {
        let tmp_list = default_list("nb", &script, &region, &variant);
        // Insert a locale replacing "nb" with "no" for every list entry
        // with precedence
        let mut bokmal_list = Vec::new();
        for l_nb in tmp_list {
            let is_root = l_nb.get_language().is_empty();
            let l_no = Locale::of_parts(
                if is_root { "" } else { "no" },
                l_nb.get_script(),
                l_nb.get_country(),
                l_nb.get_variant(),
            );
            if is_norwegian_bokmal {
                bokmal_list.push(l_no.clone());
            } else {
                bokmal_list.push(l_nb.clone());
            }
            if is_root {
                break;
            }
            if is_norwegian_bokmal {
                bokmal_list.push(l_nb);
            } else {
                bokmal_list.push(l_no);
            }
        }
        return bokmal_list;
    } else if language == "nn" || is_norwegian_nynorsk {
        // Insert no_NO_NY, no_NO, no after nn
        let mut nynorsk_list = default_list("nn", &script, &region, &variant);
        let mut idx = nynorsk_list.len() - 1;
        nynorsk_list.insert(idx, Locale::of_parts("no", "", "NO", "NY"));
        idx += 1;
        nynorsk_list.insert(idx, Locale::of_parts("no", "", "NO", ""));
        idx += 1;
        nynorsk_list.insert(idx, Locale::of_parts("no", "", "", ""));
        return nynorsk_list;
    } else if language == "zh" && script.is_empty() && !region.is_empty() {
        // Supply script for users who want to use zh_Hans/zh_Hant as bundle
        // names (recommended for Java7+)
        match region.as_str() {
            "TW" | "HK" | "MO" => script = "Hant".to_string(),
            "CN" | "SG" => script = "Hans".to_string(),
            _ => {}
        }
    }
    default_list(&language, &script, &region, &variant)
}

/// `Control.getDefaultList(String language, String script, String region,
/// String variant)`.
fn default_list(language: &str, script: &str, region: &str, variant: &str) -> Vec<Locale> {
    let mut region = region.to_string();
    let mut variants: Option<Vec<String>> = None;
    if !variant.is_empty() {
        let mut list = Vec::new();
        let units: Vec<u16> = variant.encode_utf16().collect();
        let mut idx = units.len() as i64;
        while idx != -1 {
            list.push(String::from_utf16_lossy(&units[..idx as usize]));
            idx -= 1;
            idx = last_index_of_underscore(&units, idx);
        }
        variants = Some(list);
    }
    let mut list = Vec::new();
    if let Some(variants) = &variants {
        for v in variants {
            list.push(Locale::of_parts(language, script, &region, v));
        }
    }
    if !region.is_empty() {
        list.push(Locale::of_parts(language, script, &region, ""));
    }
    if !script.is_empty() {
        list.push(Locale::of_parts(language, script, "", ""));
        // Special handling for Chinese
        if language == "zh" && region.is_empty() {
            // Supply region(country) for users who still package Chinese
            // bundles using old convention.
            match script {
                "Hans" => region = "CN".to_string(),
                "Hant" => region = "TW".to_string(),
                _ => {}
            }
        }
        // With script, after truncating variant, region and script, start
        // over without script.
        if let Some(variants) = &variants {
            for v in variants {
                list.push(Locale::of_parts(language, "", &region, v));
            }
        }
        if !region.is_empty() {
            list.push(Locale::of_parts(language, "", &region, ""));
        }
    }
    if !language.is_empty() {
        list.push(Locale::of_parts(language, "", "", ""));
    }
    // Add root locale at the end
    list.push(Locale::root());
    list
}

/// `String.lastIndexOf('_', fromIndex)` over UTF-16 units, `-1` when absent.
fn last_index_of_underscore(units: &[u16], from_index: i64) -> i64 {
    if from_index < 0 {
        return -1;
    }
    let start = (from_index as usize).min(units.len().saturating_sub(1));
    for i in (0..=start).rev() {
        if units[i] == u16::from(b'_') {
            return i as i64;
        }
    }
    -1
}

/// Decodes a `.properties` resource as `PropertyResourceBundle(InputStream)`
/// does: UTF-8, or ISO-8859-1 when the bytes are not well-formed UTF-8.
fn decode(bytes: &[u8]) -> Vec<u16> {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.encode_utf16().collect(),
        Err(_) => bytes.iter().map(|&b| u16::from(b)).collect(),
    }
}

/// `Properties.load(Reader)` (`load0` over `LineReader.readLine`): the
/// key-value pairs of a `.properties` text, a later key replacing an earlier
/// one.
///
/// # Errors
///
/// `IllegalArgumentException("Malformed \\uxxxx encoding.")`.
pub fn load(text: &[u16]) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let mut reader = LineReader { text, off: 0 };
    while let Some(line) = reader.read_line() {
        let limit = line.len();
        let mut key_len = 0usize;
        let mut value_start = limit;
        let mut has_sep = false;
        let mut preceding_backslash = false;
        while key_len < limit {
            let c = line[key_len];
            // need check if escaped.
            if (c == u16::from(b'=') || c == u16::from(b':')) && !preceding_backslash {
                value_start = key_len + 1;
                has_sep = true;
                break;
            } else if (c == u16::from(b' ') || c == u16::from(b'\t') || c == 0x0C)
                && !preceding_backslash
            {
                value_start = key_len + 1;
                break;
            }
            if c == u16::from(b'\\') {
                preceding_backslash = !preceding_backslash;
            } else {
                preceding_backslash = false;
            }
            key_len += 1;
        }
        while value_start < limit {
            let c = line[value_start];
            if c != u16::from(b' ') && c != u16::from(b'\t') && c != 0x0C {
                if !has_sep && (c == u16::from(b'=') || c == u16::from(b':')) {
                    has_sep = true;
                } else {
                    break;
                }
            }
            value_start += 1;
        }
        let key = load_convert(&line[..key_len])?;
        let value = load_convert(&line[value_start..limit])?;
        map.insert(key, value);
    }
    Ok(map)
}

/// `Properties.LineReader` over a decoded text.
struct LineReader<'a> {
    text: &'a [u16],
    off: usize,
}

impl LineReader<'_> {
    /// `readLine()`: the next logical line, with its leading whitespace,
    /// comment lines and line continuations removed; `None` at the end.
    fn read_line(&mut self) -> Option<Vec<u16>> {
        let mut line: Vec<u16> = Vec::new();
        let mut skip_white_space = true;
        let mut appended_line_begin = false;
        let mut preceding_backslash = false;
        loop {
            if self.off >= self.text.len() {
                if line.is_empty() {
                    return None;
                }
                if preceding_backslash {
                    line.pop();
                }
                return Some(line);
            }
            let c = self.text[self.off];
            self.off += 1;
            if skip_white_space {
                if c == u16::from(b' ') || c == u16::from(b'\t') || c == 0x0C {
                    continue;
                }
                if !appended_line_begin && (c == u16::from(b'\r') || c == u16::from(b'\n')) {
                    continue;
                }
                skip_white_space = false;
                appended_line_begin = false;
            }
            if line.is_empty() && (c == u16::from(b'#') || c == u16::from(b'!')) {
                // Comment, quickly consume the rest of the line, resume on
                // line-break and backslash.
                loop {
                    if self.off >= self.text.len() {
                        return None;
                    }
                    let c = self.text[self.off];
                    self.off += 1;
                    if c == u16::from(b'\r') || c == u16::from(b'\n') {
                        break;
                    }
                }
                skip_white_space = true;
                continue;
            }
            if c != u16::from(b'\n') && c != u16::from(b'\r') {
                line.push(c);
                preceding_backslash = if c == u16::from(b'\\') {
                    !preceding_backslash
                } else {
                    false
                };
            } else {
                // reached EOL
                if line.is_empty() {
                    skip_white_space = true;
                    continue;
                }
                if self.off >= self.text.len() {
                    if preceding_backslash {
                        line.pop();
                    }
                    return Some(line);
                }
                if preceding_backslash {
                    // backslash at EOL is not part of the line
                    line.pop();
                    // skip leading whitespace characters in the following
                    // line
                    skip_white_space = true;
                    appended_line_begin = true;
                    preceding_backslash = false;
                    // take care not to include any subsequent \n
                    if c == u16::from(b'\r') && self.text[self.off] == u16::from(b'\n') {
                        self.off += 1;
                    }
                } else {
                    return Some(line);
                }
            }
        }
    }
}

/// `Properties.loadConvert`: decodes `\uxxxx` escapes and the escaped
/// special characters.
fn load_convert(input: &[u16]) -> Result<String> {
    let malformed = || LuceneError::illegal_argument("Malformed \\uxxxx encoding.");
    let mut out: Vec<u16> = Vec::with_capacity(input.len());
    let end = input.len();
    let mut off = 0usize;
    while off < end {
        let mut a_char = input[off];
        off += 1;
        if a_char == u16::from(b'\\') {
            // `LineReader::readLine` excludes unescaped backslashes at the
            // end of the line.
            a_char = input[off];
            off += 1;
            if a_char == u16::from(b'u') {
                // Read the xxxx
                if off + 4 > end {
                    return Err(malformed());
                }
                let mut value: u32 = 0;
                for _ in 0..4 {
                    let c = input[off];
                    off += 1;
                    let digit = match c {
                        0x30..=0x39 => u32::from(c) - 0x30,
                        0x61..=0x66 => 10 + u32::from(c) - 0x61,
                        0x41..=0x46 => 10 + u32::from(c) - 0x41,
                        _ => return Err(malformed()),
                    };
                    value = (value << 4) + digit;
                }
                out.push(value as u16);
            } else {
                let converted = match a_char {
                    0x74 => u16::from(b'\t'),
                    0x72 => u16::from(b'\r'),
                    0x6E => u16::from(b'\n'),
                    0x66 => 0x0C,
                    other => other,
                };
                out.push(converted);
            }
        } else {
            out.push(a_char);
        }
    }
    Ok(String::from_utf16_lossy(&out))
}
