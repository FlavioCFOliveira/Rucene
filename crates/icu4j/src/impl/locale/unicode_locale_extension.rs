// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.UnicodeLocaleExtension`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock};

use crate::r#impl::locale::ascii_util::{len16, AsciiUtil};
use crate::r#impl::locale::language_tag::LanguageTag;

/// The Unicode locale extension (`u`): attributes and keywords.
///
/// Equivalent to `com.ibm.icu.impl.locale.UnicodeLocaleExtension`, a
/// subclass of `Extension` (see [`Extension::Unicode`]
/// (crate::r#impl::locale::extension::Extension::Unicode)). The attribute
/// and keyword sets hold validated ASCII strings, so the byte order of the
/// sorted collections is Java's `String` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodeLocaleExtension {
    /// `_value` of `Extension`.
    value: String,
    /// `_attributes`.
    attributes: BTreeSet<String>,
    /// `_keywords`.
    keywords: BTreeMap<String, String>,
}

/// `CA_JAPANESE`: `ca-japanese`.
pub static CA_JAPANESE: LazyLock<Arc<UnicodeLocaleExtension>> = LazyLock::new(|| {
    let mut keywords = BTreeMap::new();
    keywords.insert("ca".to_string(), "japanese".to_string());
    Arc::new(UnicodeLocaleExtension {
        value: "ca-japanese".to_string(),
        attributes: BTreeSet::new(),
        keywords,
    })
});

/// `NU_THAI`: `nu-thai`.
pub static NU_THAI: LazyLock<Arc<UnicodeLocaleExtension>> = LazyLock::new(|| {
    let mut keywords = BTreeMap::new();
    keywords.insert("nu".to_string(), "thai".to_string());
    Arc::new(UnicodeLocaleExtension {
        value: "nu-thai".to_string(),
        attributes: BTreeSet::new(),
        keywords,
    })
});

impl UnicodeLocaleExtension {
    /// `SINGLETON`.
    pub const SINGLETON: char = 'u';

    /// `new UnicodeLocaleExtension(SortedSet<String> attributes,
    /// SortedMap<String, String> keywords)` (package-private). Java's
    /// `_value` stays `null` when both are empty; the port keeps it empty.
    pub(crate) fn new(
        attributes: Option<BTreeSet<String>>,
        keywords: Option<BTreeMap<String, String>>,
    ) -> Self {
        let attributes = attributes.unwrap_or_default();
        let keywords = keywords.unwrap_or_default();
        let mut value = String::new();
        if !attributes.is_empty() || !keywords.is_empty() {
            let mut sb = String::new();
            for attribute in &attributes {
                sb.push_str(LanguageTag::SEP);
                sb.push_str(attribute);
            }
            for (key, value) in &keywords {
                sb.push_str(LanguageTag::SEP);
                sb.push_str(key);
                if !value.is_empty() {
                    sb.push_str(LanguageTag::SEP);
                    sb.push_str(value);
                }
            }
            value = sb[1..].to_string(); // skip leading '-'
        }
        Self {
            value,
            attributes,
            keywords,
        }
    }

    /// `getValue()`.
    pub fn get_value(&self) -> &str {
        &self.value
    }

    /// `getUnicodeLocaleAttributes()`.
    pub fn get_unicode_locale_attributes(&self) -> &BTreeSet<String> {
        &self.attributes
    }

    /// `getUnicodeLocaleKeys()`.
    pub fn get_unicode_locale_keys(&self) -> impl Iterator<Item = &String> {
        self.keywords.keys()
    }

    /// `getUnicodeLocaleType(String unicodeLocaleKey)`.
    pub fn get_unicode_locale_type(&self, unicode_locale_key: &str) -> Option<&str> {
        self.keywords.get(unicode_locale_key).map(String::as_str)
    }

    /// `isSingletonChar(char c)`.
    pub fn is_singleton_char(c: char) -> bool {
        Self::SINGLETON == AsciiUtil::to_lower(c)
    }

    /// `isAttribute(String s)`: `3*8alphanum`.
    pub fn is_attribute(s: &str) -> bool {
        let n = len16(s);
        (3..=8).contains(&n) && AsciiUtil::is_alpha_numeric_string(s)
    }

    /// `isKey(String s)`: `alphanum alpha`.
    pub fn is_key(s: &str) -> bool {
        let b = s.as_bytes();
        len16(s) == 2 && b[0].is_ascii_alphanumeric() && b[1].is_ascii_alphabetic()
    }

    /// `isTypeSubtag(String s)`: `3*8alphanum`.
    pub fn is_type_subtag(s: &str) -> bool {
        let n = len16(s);
        (3..=8).contains(&n) && AsciiUtil::is_alpha_numeric_string(s)
    }

    /// `isType(String s)`: a sequence of type subtags delimited by `-`.
    pub fn is_type(s: &str) -> bool {
        // sequence of type subtags delimited by '-'
        let mut start_idx = 0usize;
        let mut saw_subtag;
        loop {
            let idx = s[start_idx..].find(LanguageTag::SEP).map(|i| i + start_idx);
            let subtag = match idx {
                Some(i) => &s[start_idx..i],
                None => &s[start_idx..],
            };
            if !Self::is_type_subtag(subtag) {
                return false;
            }
            saw_subtag = true;
            match idx {
                Some(i) => start_idx = i + 1,
                None => break,
            }
        }
        saw_subtag && start_idx < s.len()
    }
}
