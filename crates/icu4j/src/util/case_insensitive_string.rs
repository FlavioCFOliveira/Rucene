// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.CaseInsensitiveString`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use crate::lang::u_character::UCharacter;
use crate::r#impl::utility::Utility;

/// A string used as a key in java.util.Hashtable and other collections. It
/// retains case information, but its equals() and hashCode() methods
/// ignore case.
///
/// Equivalent to `com.ibm.icu.util.CaseInsensitiveString`. Its lazily
/// computed `folded` string and `hash` are set once.
#[derive(Debug, Clone)]
pub struct CaseInsensitiveString {
    string: Vec<u16>,
    hash: OnceLock<i32>,
    folded: OnceLock<Vec<u16>>,
}

impl CaseInsensitiveString {
    fn fold_case(foldee: &[u16]) -> Vec<u16> {
        UCharacter::fold_case_str(foldee, true)
    }

    fn get_folded(&self) -> &[u16] {
        self.folded.get_or_init(|| Self::fold_case(&self.string))
    }

    /// `CaseInsensitiveString(String s)`.
    pub fn new(s: Vec<u16>) -> Self {
        Self {
            string: s,
            hash: OnceLock::new(),
            folded: OnceLock::new(),
        }
    }

    /// `getString()`.
    pub fn get_string(&self) -> &[u16] {
        &self.string
    }

    /// `hashCode()`: the hash code of the case-folded string.
    pub fn hash_code(&self) -> i32 {
        let folded = self.get_folded();
        *self
            .hash
            .get_or_init(|| Utility::char_sequence_hash_code(folded))
    }
}

impl PartialEq for CaseInsensitiveString {
    /// `equals(Object o)`: equal case-folded strings.
    fn eq(&self, o: &Self) -> bool {
        if std::ptr::eq(self, o) {
            return true;
        }
        self.get_folded() == o.get_folded()
    }
}

impl Eq for CaseInsensitiveString {}

impl Hash for CaseInsensitiveString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.hash_code());
    }
}

impl fmt::Display for CaseInsensitiveString {
    /// `toString()`: the original string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf16_lossy(&self.string))
    }
}
