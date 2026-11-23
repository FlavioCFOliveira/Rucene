// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.InternalLocaleBuilder`.

use std::sync::Arc;

use crate::r#impl::locale::ascii_util::{substring16, substring16_from, AsciiUtil};
use crate::r#impl::locale::base_locale::BaseLocale;
use crate::r#impl::locale::language_tag::LanguageTag;
use crate::r#impl::locale::locale_extensions::{self, LocaleExtensions};
use crate::r#impl::locale::locale_syntax_exception::LocaleSyntaxException;
use crate::r#impl::locale::string_token_iterator::StringTokenIterator;
use crate::r#impl::locale::unicode_locale_extension::UnicodeLocaleExtension;

/// The result of a builder setter: the `LocaleSyntaxException` it throws.
pub type BuilderResult<T> = std::result::Result<T, LocaleSyntaxException>;

/// The mutable state behind `ULocale.Builder`: the base locale fields and
/// the extensions, validated as they are set.
///
/// Equivalent to `com.ibm.icu.impl.locale.InternalLocaleBuilder` (`JDKIMPL`
/// is `false`). Java's `HashMap<CaseInsensitiveChar, String>`,
/// `HashSet<CaseInsensitiveString>` and `HashMap<CaseInsensitiveString,
/// String>` are vectors in insertion order looked up with Java's
/// `probe.equals(stored)` (see [`CaseInsensitiveChar`]): their content is
/// read only through `LocaleExtensions`, which sorts it, and entries whose
/// keys fold together share a hash bucket, which Java iterates in insertion
/// order.
#[derive(Debug, Clone, Default)]
pub struct InternalLocaleBuilder {
    language: String,
    script: String,
    region: String,
    variant: String,
    extensions: Vec<(CaseInsensitiveChar, String)>,
    uattributes: Vec<CaseInsensitiveString>,
    ukeywords: Vec<(CaseInsensitiveString, String)>,
}

/// `PRIVUSE_KEY`.
const PRIVUSE_KEY: CaseInsensitiveChar = CaseInsensitiveChar('x');

fn ext_find(map: &[(CaseInsensitiveChar, String)], key: CaseInsensitiveChar) -> Option<usize> {
    map.iter().position(|(k, _)| key.java_equals(*k))
}

fn ext_put(map: &mut Vec<(CaseInsensitiveChar, String)>, key: CaseInsensitiveChar, value: String) {
    match ext_find(map, key) {
        Some(i) => map[i].1 = value,
        None => map.push((key, value)),
    }
}

fn ukw_find(map: &[(CaseInsensitiveString, String)], key: &CaseInsensitiveString) -> Option<usize> {
    map.iter().position(|(k, _)| k == key)
}

fn ukw_put(map: &mut Vec<(CaseInsensitiveString, String)>, key: CaseInsensitiveString, value: String) {
    match ukw_find(map, &key) {
        Some(i) => map[i].1 = value,
        None => map.push((key, value)),
    }
}

fn uattr_add(set: &mut Vec<CaseInsensitiveString>, value: CaseInsensitiveString) {
    if !set.contains(&value) {
        set.push(value);
    }
}

impl InternalLocaleBuilder {
    /// `new InternalLocaleBuilder()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `setLanguage(String language)`: `None` or empty clears it.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException("Ill-formed language: ...")`.
    pub fn set_language(&mut self, language: Option<&str>) -> BuilderResult<&mut Self> {
        match language {
            None | Some("") => self.language = String::new(),
            Some(language) => {
                if !LanguageTag::is_language(language) {
                    return Err(LocaleSyntaxException::with_index(
                        format!("Ill-formed language: {language}"),
                        0,
                    ));
                }
                self.language = language.to_string();
            }
        }
        Ok(self)
    }

    /// `setScript(String script)`.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException("Ill-formed script: ...")`.
    pub fn set_script(&mut self, script: Option<&str>) -> BuilderResult<&mut Self> {
        match script {
            None | Some("") => self.script = String::new(),
            Some(script) => {
                if !LanguageTag::is_script(script) {
                    return Err(LocaleSyntaxException::with_index(
                        format!("Ill-formed script: {script}"),
                        0,
                    ));
                }
                self.script = script.to_string();
            }
        }
        Ok(self)
    }

    /// `setRegion(String region)`.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException("Ill-formed region: ...")`.
    pub fn set_region(&mut self, region: Option<&str>) -> BuilderResult<&mut Self> {
        match region {
            None | Some("") => self.region = String::new(),
            Some(region) => {
                if !LanguageTag::is_region(region) {
                    return Err(LocaleSyntaxException::with_index(
                        format!("Ill-formed region: {region}"),
                        0,
                    ));
                }
                self.region = region.to_string();
            }
        }
        Ok(self)
    }

    /// `setVariant(String variant)`.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException("Ill-formed variant: ...")` at the first bad
    /// subtag.
    pub fn set_variant(&mut self, variant: Option<&str>) -> BuilderResult<&mut Self> {
        match variant {
            None | Some("") => self.variant = String::new(),
            Some(variant) => {
                // normalize separators to "_"
                let var = variant.replace(LanguageTag::SEP, BaseLocale::SEP);
                let err_idx = Self::check_variants(&var, BaseLocale::SEP);
                if err_idx != -1 {
                    return Err(LocaleSyntaxException::with_index(
                        format!("Ill-formed variant: {variant}"),
                        err_idx,
                    ));
                }
                self.variant = var;
            }
        }
        Ok(self)
    }

    /// `addUnicodeLocaleAttribute(String attribute)`.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException("Ill-formed Unicode locale attribute: ...")`.
    pub fn add_unicode_locale_attribute(&mut self, attribute: Option<&str>) -> BuilderResult<&mut Self> {
        let attribute = match attribute {
            Some(a) if UnicodeLocaleExtension::is_attribute(a) => a,
            other => {
                return Err(LocaleSyntaxException::new(format!(
                    "Ill-formed Unicode locale attribute: {}",
                    other.unwrap_or("null")
                )))
            }
        };
        // Use case insensitive string to prevent duplication
        uattr_add(&mut self.uattributes, CaseInsensitiveString::new(attribute));
        Ok(self)
    }

    /// `removeUnicodeLocaleAttribute(String attribute)`.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException("Ill-formed Unicode locale attribute: ...")`.
    pub fn remove_unicode_locale_attribute(&mut self, attribute: Option<&str>) -> BuilderResult<&mut Self> {
        let attribute = match attribute {
            Some(a) if UnicodeLocaleExtension::is_attribute(a) => a,
            other => {
                return Err(LocaleSyntaxException::new(format!(
                    "Ill-formed Unicode locale attribute: {}",
                    other.unwrap_or("null")
                )))
            }
        };
        let probe = CaseInsensitiveString::new(attribute);
        self.uattributes.retain(|a| *a != probe);
        Ok(self)
    }

    /// `setUnicodeLocaleKeyword(String key, String type)`: `type` `None`
    /// removes the key.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException` for an ill-formed key or type.
    pub fn set_unicode_locale_keyword(&mut self, key: &str, ty: Option<&str>) -> BuilderResult<&mut Self> {
        if !UnicodeLocaleExtension::is_key(key) {
            return Err(LocaleSyntaxException::new(format!(
                "Ill-formed Unicode locale keyword key: {key}"
            )));
        }

        let cikey = CaseInsensitiveString::new(key);
        match ty {
            None => {
                // null type is used for remove the key
                if let Some(i) = ukw_find(&self.ukeywords, &cikey) {
                    self.ukeywords.remove(i);
                }
            }
            Some(ty) => {
                if !ty.is_empty() {
                    // normalize separator to "-"
                    let tp = ty.replace(BaseLocale::SEP, LanguageTag::SEP);
                    // validate
                    let mut itr = StringTokenIterator::new(&tp, LanguageTag::SEP);
                    while !itr.is_done() {
                        let s = itr.cur();
                        if !UnicodeLocaleExtension::is_type_subtag(s) {
                            return Err(LocaleSyntaxException::with_index(
                                format!("Ill-formed Unicode locale keyword type: {ty}"),
                                itr.current_start(),
                            ));
                        }
                        itr.next();
                    }
                }
                ukw_put(&mut self.ukeywords, cikey, ty.to_string());
            }
        }
        Ok(self)
    }

    /// `setExtension(char singleton, String value)`: `value` `None` or
    /// empty removes the extension.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException` for an ill-formed key or value.
    pub fn set_extension(&mut self, singleton: char, value: Option<&str>) -> BuilderResult<&mut Self> {
        // validate key
        let is_bcp_privateuse = LanguageTag::is_privateuse_prefix_char(singleton);
        if !is_bcp_privateuse && !LanguageTag::is_extension_singleton_char(singleton) {
            return Err(LocaleSyntaxException::new(format!(
                "Ill-formed extension key: {singleton}"
            )));
        }

        let remove = value.is_none_or(str::is_empty);
        let key = CaseInsensitiveChar(singleton);

        if remove {
            if UnicodeLocaleExtension::is_singleton_char(key.value()) {
                // clear entire Unicode locale extension
                self.uattributes.clear();
                self.ukeywords.clear();
            } else if let Some(i) = ext_find(&self.extensions, key) {
                self.extensions.remove(i);
            }
        } else {
            let value = value.unwrap_or("");
            // validate value
            let val = value.replace(BaseLocale::SEP, LanguageTag::SEP);
            let mut itr = StringTokenIterator::new(&val, LanguageTag::SEP);
            while !itr.is_done() {
                let s = itr.cur();
                let valid_subtag = if is_bcp_privateuse {
                    LanguageTag::is_privateuse_subtag(s)
                } else {
                    LanguageTag::is_extension_subtag(s)
                };
                if !valid_subtag {
                    return Err(LocaleSyntaxException::with_index(
                        format!("Ill-formed extension value: {s}"),
                        itr.current_start(),
                    ));
                }
                itr.next();
            }

            if UnicodeLocaleExtension::is_singleton_char(key.value()) {
                self.set_unicode_locale_extension(&val);
            } else {
                ext_put(&mut self.extensions, key, val);
            }
        }
        Ok(self)
    }

    /// `setExtensions(String subtags)`: extension and private use subtags
    /// in a single string representation.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException` for incomplete or ill-formed subtags.
    pub fn set_extensions_str(&mut self, subtags: Option<&str>) -> BuilderResult<&mut Self> {
        let subtags = match subtags {
            None | Some("") => {
                self.clear_extensions();
                return Ok(self);
            }
            Some(s) => s.replace(BaseLocale::SEP, LanguageTag::SEP),
        };
        let mut itr = StringTokenIterator::new(&subtags, LanguageTag::SEP);

        let mut extensions: Option<Vec<String>> = None;
        let mut privateuse: Option<String> = None;

        let mut parsed = 0;
        let mut start;

        // Make a list of extension subtags
        while !itr.is_done() {
            let s = itr.cur().to_string();
            if LanguageTag::is_extension_singleton(&s) {
                start = itr.current_start();
                let singleton = s;
                let mut sb = singleton.clone();

                itr.next();
                while !itr.is_done() {
                    let s = itr.cur().to_string();
                    if LanguageTag::is_extension_subtag(&s) {
                        sb.push_str(LanguageTag::SEP);
                        sb.push_str(&s);
                        parsed = itr.current_end();
                    } else {
                        break;
                    }
                    itr.next();
                }

                if parsed < start {
                    return Err(LocaleSyntaxException::with_index(
                        format!("Incomplete extension '{singleton}'"),
                        start,
                    ));
                }

                extensions.get_or_insert_with(Vec::new).push(sb);
            } else {
                break;
            }
        }
        if !itr.is_done() {
            let s = itr.cur().to_string();
            if LanguageTag::is_privateuse_prefix(&s) {
                start = itr.current_start();
                let mut sb = s;

                itr.next();
                while !itr.is_done() {
                    let s = itr.cur().to_string();
                    if !LanguageTag::is_privateuse_subtag(&s) {
                        break;
                    }
                    sb.push_str(LanguageTag::SEP);
                    sb.push_str(&s);
                    parsed = itr.current_end();

                    itr.next();
                }
                if parsed <= start {
                    return Err(LocaleSyntaxException::with_index(
                        format!("Incomplete privateuse:{}", substring16_from(&subtags, start)),
                        start,
                    ));
                } else {
                    privateuse = Some(sb);
                }
            }
        }

        if !itr.is_done() {
            return Err(LocaleSyntaxException::with_index(
                format!(
                    "Ill-formed extension subtags:{}",
                    substring16_from(&subtags, itr.current_start())
                ),
                itr.current_start(),
            ));
        }

        Ok(self.set_extensions_list(extensions.as_deref(), privateuse.as_deref()))
    }

    /// `setExtensions(List<String> bcpExtensions, String privateuse)`
    /// (private): BCP 47 extensions, already well-formed, but possibly with
    /// duplicates.
    fn set_extensions_list(&mut self, bcp_extensions: Option<&[String]>, privateuse: Option<&str>) -> &mut Self {
        self.clear_extensions();

        if let Some(bcp_extensions) = bcp_extensions.filter(|b| !b.is_empty()) {
            // Java never adds to its set of processed extensions, so
            // duplicates are not skipped: a later one replaces an earlier.
            for bcp_ext in bcp_extensions {
                let first = bcp_ext.chars().next().expect("an extension has a singleton");
                let key = CaseInsensitiveChar(first);
                // each extension string contains singleton, e.g. "a-abc-def"
                if UnicodeLocaleExtension::is_singleton_char(key.value()) {
                    self.set_unicode_locale_extension(&substring16_from(bcp_ext, 2));
                } else {
                    ext_put(&mut self.extensions, key, substring16_from(bcp_ext, 2));
                }
            }
        }
        if let Some(privateuse) = privateuse.filter(|p| !p.is_empty()) {
            // privateuse string contains prefix, e.g. "x-abc-def"
            let first = privateuse.chars().next().expect("a private use has a prefix");
            ext_put(
                &mut self.extensions,
                CaseInsensitiveChar(first),
                substring16_from(privateuse, 2),
            );
        }

        self
    }

    /// `setLanguageTag(LanguageTag langtag)`: resets the builder's state
    /// with the given language tag.
    pub fn set_language_tag(&mut self, langtag: &LanguageTag) -> &mut Self {
        self.clear();
        if !langtag.get_extlangs().is_empty() {
            self.language = langtag.get_extlangs()[0].clone();
        } else {
            let language = langtag.get_language();
            if language != LanguageTag::UNDETERMINED {
                self.language = language.to_string();
            }
        }
        self.script = langtag.get_script().to_string();
        self.region = langtag.get_region().to_string();

        let mut bcp_variants: Vec<String> = langtag.get_variants().to_vec();
        bcp_variants.sort_by(|a, b| rucene::util::java_collections::java_compare_to(a, b));
        if !bcp_variants.is_empty() {
            let mut var = bcp_variants[0].clone();
            for v in &bcp_variants[1..] {
                var.push_str(BaseLocale::SEP);
                var.push_str(v);
            }
            self.variant = var;
        }

        let extensions = langtag.get_extensions().to_vec();
        let privateuse = langtag.get_privateuse().to_string();
        self.set_extensions_list(Some(&extensions), Some(&privateuse));

        self
    }

    /// `setLocale(BaseLocale base, LocaleExtensions extensions)`.
    ///
    /// # Errors
    ///
    /// `LocaleSyntaxException` when a base locale field is ill-formed.
    pub fn set_locale(
        &mut self,
        base: &BaseLocale,
        extensions: Option<&LocaleExtensions>,
    ) -> BuilderResult<&mut Self> {
        let language = base.get_language();
        let script = base.get_script();
        let region = base.get_region();
        let variant = base.get_variant();

        // Validate base locale fields before updating internal state.
        // LocaleExtensions always store validated/canonicalized values,
        // so no checks are necessary.
        if !language.is_empty() && !LanguageTag::is_language(language) {
            return Err(LocaleSyntaxException::new(format!("Ill-formed language: {language}")));
        }

        if !script.is_empty() && !LanguageTag::is_script(script) {
            return Err(LocaleSyntaxException::new(format!("Ill-formed script: {script}")));
        }

        if !region.is_empty() && !LanguageTag::is_region(region) {
            return Err(LocaleSyntaxException::new(format!("Ill-formed region: {region}")));
        }

        if !variant.is_empty() {
            let err_idx = Self::check_variants(variant, BaseLocale::SEP);
            if err_idx != -1 {
                return Err(LocaleSyntaxException::with_index(
                    format!("Ill-formed variant: {variant}"),
                    err_idx,
                ));
            }
        }

        // The input locale is validated at this point.
        // Now, updating builder's internal fields.
        self.language = language.to_string();
        self.script = script.to_string();
        self.region = region.to_string();
        self.variant = variant.to_string();
        self.clear_extensions();

        if let Some(extensions) = extensions {
            // map extensions back to builder's internal format
            for key in extensions.get_keys() {
                let e = extensions
                    .get_extension(key)
                    .expect("a key of the extensions has its extension");
                if let Some(ue) = e.as_unicode() {
                    for uatr in ue.get_unicode_locale_attributes() {
                        uattr_add(&mut self.uattributes, CaseInsensitiveString::new(uatr));
                    }
                    for ukey in ue.get_unicode_locale_keys() {
                        ukw_put(
                            &mut self.ukeywords,
                            CaseInsensitiveString::new(ukey),
                            ue.get_unicode_locale_type(ukey).unwrap_or("").to_string(),
                        );
                    }
                } else {
                    ext_put(
                        &mut self.extensions,
                        CaseInsensitiveChar(key),
                        e.get_value().to_string(),
                    );
                }
            }
        }
        Ok(self)
    }

    /// `clear()`.
    pub fn clear(&mut self) -> &mut Self {
        self.language.clear();
        self.script.clear();
        self.region.clear();
        self.variant.clear();
        self.clear_extensions();
        self
    }

    /// `clearExtensions()`.
    pub fn clear_extensions(&mut self) -> &mut Self {
        self.extensions.clear();
        self.uattributes.clear();
        self.ukeywords.clear();
        self
    }

    /// `getBaseLocale()`: the special private use subtag sequence
    /// identified by `lvariant` is interpreted as the Java variant.
    pub fn get_base_locale(&self) -> Arc<BaseLocale> {
        let language = &self.language;
        let script = &self.script;
        let region = &self.region;
        let mut variant = self.variant.clone();

        // Special private use subtag sequence identified by "lvariant" will be
        // interpreted as Java variant.
        if let Some(i) = ext_find(&self.extensions, PRIVUSE_KEY) {
            let privuse = &self.extensions[i].1;
            let mut itr = StringTokenIterator::new(privuse, LanguageTag::SEP);
            let mut saw_prefix = false;
            let mut priv_var_start = -1;
            while !itr.is_done() {
                if saw_prefix {
                    priv_var_start = itr.current_start();
                    break;
                }
                if AsciiUtil::case_ignore_match(itr.cur(), LanguageTag::PRIVUSE_VARIANT_PREFIX) {
                    saw_prefix = true;
                }
                itr.next();
            }
            if priv_var_start != -1 {
                let mut sb = variant.clone();
                if !sb.is_empty() {
                    sb.push_str(BaseLocale::SEP);
                }
                sb.push_str(
                    &substring16_from(privuse, priv_var_start).replace(LanguageTag::SEP, BaseLocale::SEP),
                );
                variant = sb;
            }
        }

        BaseLocale::get_instance(language, script, region, &variant)
    }

    /// `getLocaleExtensions()`.
    pub fn get_locale_extensions(&self) -> Arc<LocaleExtensions> {
        if self.extensions.is_empty() && self.uattributes.is_empty() && self.ukeywords.is_empty() {
            return locale_extensions::EMPTY_EXTENSIONS.clone();
        }

        Arc::new(LocaleExtensions::from_builder(
            &self.extensions,
            &self.uattributes,
            &self.ukeywords,
        ))
    }

    /// `removePrivateuseVariant(String privuseVal)` (package-private): the
    /// private use value without the special `lvariant` sequence, `None`
    /// when nothing remains.
    pub(crate) fn remove_privateuse_variant(privuse_val: &str) -> Option<String> {
        let mut itr = StringTokenIterator::new(privuse_val, LanguageTag::SEP);

        // Note: privateuse value "abc-lvariant" is unchanged
        // because no subtags after "lvariant".

        let mut prefix_start = -1;
        let mut saw_privuse_var = false;
        while !itr.is_done() {
            if prefix_start != -1 {
                // Note: privateuse value "abc-lvariant" is unchanged
                // because no subtags after "lvariant".
                saw_privuse_var = true;
                break;
            }
            if AsciiUtil::case_ignore_match(itr.cur(), LanguageTag::PRIVUSE_VARIANT_PREFIX) {
                prefix_start = itr.current_start();
            }
            itr.next();
        }
        if !saw_privuse_var {
            return Some(privuse_val.to_string());
        }

        debug_assert!(prefix_start == 0 || prefix_start > 1);
        if prefix_start == 0 {
            None
        } else {
            Some(substring16(privuse_val, 0, prefix_start - 1))
        }
    }

    /// `checkVariants(String variants, String sep)` (private): the start of
    /// the first ill-formed variant subtag, `-1` when all are valid.
    fn check_variants(variants: &str, sep: &str) -> i32 {
        let mut itr = StringTokenIterator::new(variants, sep);
        while !itr.is_done() {
            let s = itr.cur();
            if !LanguageTag::is_variant(s) {
                return itr.current_start();
            }
            itr.next();
        }
        -1
    }

    /// `setUnicodeLocaleExtension(String subtags)` (private): parses valid
    /// Unicode locale extension subtags (without the singleton); duplicated
    /// attributes and keywords are ignored.
    fn set_unicode_locale_extension(&mut self, subtags: &str) {
        // wipe out existing attributes/keywords
        self.uattributes.clear();
        self.ukeywords.clear();

        let mut itr = StringTokenIterator::new(subtags, LanguageTag::SEP);

        // parse attributes
        while !itr.is_done() {
            if !UnicodeLocaleExtension::is_attribute(itr.cur()) {
                break;
            }
            uattr_add(&mut self.uattributes, CaseInsensitiveString::new(itr.cur()));
            itr.next();
        }

        // parse keywords
        let mut key: Option<CaseInsensitiveString> = None;
        let mut type_start = -1;
        let mut type_end = -1;
        while !itr.is_done() {
            if let Some(k) = key.clone() {
                if UnicodeLocaleExtension::is_key(itr.cur()) {
                    // next keyword - emit previous one
                    debug_assert!(type_start == -1 || type_end != -1);
                    let ty = if type_start == -1 {
                        String::new()
                    } else {
                        substring16(subtags, type_start, type_end)
                    };
                    ukw_put(&mut self.ukeywords, k, ty);

                    // reset keyword info
                    let tmp_key = CaseInsensitiveString::new(itr.cur());
                    key = if ukw_find(&self.ukeywords, &tmp_key).is_some() {
                        None
                    } else {
                        Some(tmp_key)
                    };
                    type_start = -1;
                    type_end = -1;
                } else {
                    if type_start == -1 {
                        type_start = itr.current_start();
                    }
                    type_end = itr.current_end();
                }
            } else if UnicodeLocaleExtension::is_key(itr.cur()) {
                // 1. first keyword or
                // 2. next keyword, but previous one was duplicate
                let k = CaseInsensitiveString::new(itr.cur());
                key = if ukw_find(&self.ukeywords, &k).is_some() {
                    // duplicate
                    None
                } else {
                    Some(k)
                };
            }

            if !itr.has_next() {
                if let Some(k) = key.take() {
                    // last keyword
                    debug_assert!(type_start == -1 || type_end != -1);
                    let ty = if type_start == -1 {
                        String::new()
                    } else {
                        substring16(subtags, type_start, type_end)
                    };
                    ukw_put(&mut self.ukeywords, k, ty);
                }
                break;
            }

            itr.next();
        }
    }
}

/// A string compared ignoring ASCII case.
///
/// Equivalent to `InternalLocaleBuilder.CaseInsensitiveString`
/// (package-private).
#[derive(Debug, Clone)]
pub struct CaseInsensitiveString(String);

impl CaseInsensitiveString {
    /// `new CaseInsensitiveString(String s)`.
    pub(crate) fn new(s: &str) -> Self {
        Self(s.to_string())
    }

    /// `value()`.
    pub fn value(&self) -> &str {
        &self.0
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        crate::r#impl::locale::ascii_util::string_hash(&AsciiUtil::to_lower_string(&self.0))
    }
}

/// `equals(Object)`: `caseIgnoreMatch`.
impl PartialEq for CaseInsensitiveString {
    fn eq(&self, other: &Self) -> bool {
        AsciiUtil::case_ignore_match(&self.0, &other.0)
    }
}

impl Eq for CaseInsensitiveString {}

/// A character key.
///
/// Equivalent to `InternalLocaleBuilder.CaseInsensitiveChar`
/// (package-private). Java's `equals` compares the receiver's character
/// with the *lower-cased* character of the argument, so it is not
/// symmetric: an upper-case probe never equals a stored key, and a
/// lower-case probe equals a stored key of either case.
/// [`java_equals`](Self::java_equals) is that `equals`, which the builder's
/// lookups call as Java's `HashMap` does (`probe.equals(stored)`, after
/// comparing the hashes, the lower-cased characters).
#[derive(Debug, Clone, Copy)]
pub struct CaseInsensitiveChar(char);

impl CaseInsensitiveChar {
    /// `value()`.
    pub fn value(&self) -> char {
        self.0
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        AsciiUtil::to_lower(self.0) as i32
    }

    /// `equals(Object obj)` with the hash test of `HashMap` before it.
    pub fn java_equals(&self, stored: CaseInsensitiveChar) -> bool {
        self.hash_code() == stored.hash_code() && self.0 == AsciiUtil::to_lower(stored.0)
    }
}
