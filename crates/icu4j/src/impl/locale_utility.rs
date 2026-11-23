// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.LocaleUtility`.

use rucene_queryparser::jdk::locale::Locale;

/// A class to hold utility functions missing from `java.util.Locale`.
///
/// Equivalent to `com.ibm.icu.impl.LocaleUtility`.
pub struct LocaleUtility;

impl LocaleUtility {
    /// `getLocaleFromName(String name)`: a locale from a
    /// `language_country_variant` name.
    pub fn get_locale_from_name(name: &str) -> Locale {
        let mut country = "";
        let mut variant = "";
        let language;
        match name.find('_') {
            None => language = name,
            Some(i1) => {
                language = &name[..i1];
                let rest = &name[i1 + 1..];
                match rest.find('_') {
                    None => country = rest,
                    Some(i2) => {
                        country = &rest[..i2];
                        variant = &rest[i2 + 1..];
                    }
                }
            }
        }
        Locale::of(language, country, variant)
    }

    /// `isFallbackOf(String parent, String child)`: whether `child` is
    /// `parent` or starts with `parent` followed by `_`.
    pub fn is_fallback_of(parent: &str, child: &str) -> bool {
        if !child.starts_with(parent) {
            return false;
        }
        let i = parent.len();
        i == child.len() || child.as_bytes()[i] == b'_'
    }

    /// `isFallbackOf(Locale parent, Locale child)`.
    pub fn is_fallback_of_locale(parent: &Locale, child: &Locale) -> bool {
        Self::is_fallback_of(&parent.to_string(), &child.to_string())
    }

    /// `fallback(Locale loc)`: the locale without its rightmost non-empty
    /// part, `None` when all parts are empty.
    pub fn fallback(loc: &Locale) -> Option<Locale> {
        // Split the locale into parts and remove the rightmost part
        let mut parts = [
            loc.get_language().to_string(),
            loc.get_country().to_string(),
            loc.get_variant().to_string(),
        ];
        let mut i = 2i32;
        while i >= 0 {
            if !parts[i as usize].is_empty() {
                parts[i as usize].clear();
                break;
            }
            i -= 1;
        }
        if i < 0 {
            return None; // All parts were empty
        }
        Some(Locale::of(&parts[0], &parts[1], &parts[2]))
    }
}
