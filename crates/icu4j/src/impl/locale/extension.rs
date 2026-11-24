// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.Extension`.

use std::fmt;
use std::sync::Arc;

use crate::r#impl::locale::language_tag::LanguageTag;
use crate::r#impl::locale::unicode_locale_extension::UnicodeLocaleExtension;

/// A locale extension: a singleton key and its value.
///
/// Equivalent to `com.ibm.icu.impl.locale.Extension` and its subclass
/// `UnicodeLocaleExtension`: Java's `instanceof UnicodeLocaleExtension`
/// is the [`Unicode`](Extension::Unicode) variant
/// ([`as_unicode`](Self::as_unicode)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extension {
    /// A plain `Extension(char key, String value)`.
    Plain {
        /// `_key`.
        key: char,
        /// `_value`.
        value: String,
    },
    /// A `UnicodeLocaleExtension`.
    Unicode(Arc<UnicodeLocaleExtension>),
}

impl Extension {
    /// `new Extension(char key, String value)` (package-private).
    pub(crate) fn new(key: char, value: String) -> Self {
        Extension::Plain { key, value }
    }

    /// `getKey()`.
    pub fn get_key(&self) -> char {
        match self {
            Extension::Plain { key, .. } => *key,
            Extension::Unicode(_) => UnicodeLocaleExtension::SINGLETON,
        }
    }

    /// `getValue()`.
    pub fn get_value(&self) -> &str {
        match self {
            Extension::Plain { value, .. } => value,
            Extension::Unicode(u) => u.get_value(),
        }
    }

    /// `getID()`: the key, `-` and the value.
    pub fn get_id(&self) -> String {
        format!("{}{}{}", self.get_key(), LanguageTag::SEP, self.get_value())
    }

    /// `this instanceof UnicodeLocaleExtension`, the subclass view.
    pub fn as_unicode(&self) -> Option<&Arc<UnicodeLocaleExtension>> {
        match self {
            Extension::Unicode(u) => Some(u),
            Extension::Plain { .. } => None,
        }
    }
}

/// `toString()`: [`get_id`](Extension::get_id).
impl fmt::Display for Extension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_id())
    }
}
