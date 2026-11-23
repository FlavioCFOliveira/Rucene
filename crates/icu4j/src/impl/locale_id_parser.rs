// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.LocaleIDParser`.

use rucene::error::{LuceneError, Result};

use crate::r#impl::locale::ascii_util::AsciiUtil;
use crate::r#impl::locale_ids::LocaleIDs;

/// A `TreeMap<String, String>` in Java's `String` order (the comparator of
/// `getKeyComparator`, `lhs.compareTo(rhs)`): entries sorted by key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeywordMap(Vec<(String, String)>);

impl KeywordMap {
    fn find(&self, key: &str) -> std::result::Result<usize, usize> {
        self.0
            .binary_search_by(|(k, _)| rucene::util::java_collections::java_compare_to(k, key))
    }

    /// `get(key)`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.find(key).ok().map(|i| self.0[i].1.as_str())
    }

    /// `containsKey(key)`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.find(key).is_ok()
    }

    /// `put(key, value)`.
    pub fn insert(&mut self, key: String, value: String) {
        match self.find(&key) {
            Ok(i) => self.0[i].1 = value,
            Err(i) => self.0.insert(i, (key, value)),
        }
    }

    /// `remove(key)`.
    pub fn remove(&mut self, key: &str) {
        if let Ok(i) = self.find(key) {
            self.0.remove(i);
        }
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `keySet()`, in order.
    pub fn keys(&self) -> Vec<String> {
        self.0.iter().map(|(k, _)| k.clone()).collect()
    }

    /// `entrySet()`, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter().map(|(k, v)| (k, v))
    }
}

const KEYWORD_SEPARATOR: u16 = b'@' as u16;
const HYPHEN: u16 = b'-' as u16;
const KEYWORD_ASSIGN: u16 = b'=' as u16;
const COMMA: u16 = b',' as u16;
const ITEM_SEPARATOR: u16 = b';' as u16;
const DOT: u16 = b'.' as u16;
const UNDERSCORE: u16 = b'_' as u16;

/// `DONE`.
const DONE: u16 = 0xffff;

/// `MAX_VARIANTS_LENGTH`: there are no strict limitations of the syntax of
/// variant in the legacy locale format. If the locale is constructed from
/// `unicode_locale_id` as defined in UTS 35, then each
/// `unicode_variant_subtag` could have a maximum length of 8; 179 allows 20
/// subtags with separators.
const MAX_VARIANTS_LENGTH: usize = 179;

/// Utility class to parse and normalize locale ids (including POSIX style).
///
/// Equivalent to `com.ibm.icu.impl.LocaleIDParser`: the id is scanned as
/// Java's `char[]` (UTF-16 units); the output strings are made of the id's
/// units split at ASCII separators, so they convert back to Rust strings
/// unchanged.
#[derive(Debug, Clone)]
pub struct LocaleIDParser {
    id: Vec<u16>,
    index: usize,
    buffer: Vec<u16>,
    // um, don't handle POSIX ids unless we request it.  why not?  well... because.
    canonicalize: bool,
    had_country: bool,
    // used when canonicalizing
    keywords: Option<KeywordMap>,
    base_name: Option<String>,
}

fn trim16(s: &[u16]) -> &[u16] {
    let mut st = 0;
    let mut len = s.len();
    while st < len && s[st] <= b' ' as u16 {
        st += 1;
    }
    while st < len && s[len - 1] <= b' ' as u16 {
        len -= 1;
    }
    &s[st..len]
}

impl LocaleIDParser {
    /// `new LocaleIDParser(String localeID)`.
    pub fn new(locale_id: &str) -> Self {
        Self::with_canonicalize(locale_id, false)
    }

    /// `new LocaleIDParser(String localeID, boolean canonicalize)`.
    pub fn with_canonicalize(locale_id: &str, canonicalize: bool) -> Self {
        let id: Vec<u16> = locale_id.encode_utf16().collect();
        let cap = id.len() + 5;
        Self {
            id,
            index: 0,
            buffer: Vec::with_capacity(cap),
            canonicalize,
            had_country: false,
            keywords: None,
            base_name: None,
        }
    }

    fn reset(&mut self) {
        self.index = 0;
        self.buffer = Vec::with_capacity(self.id.len() + 5);
    }

    // utilities for working on text in the buffer

    fn append(&mut self, c: u16) {
        self.buffer.push(c);
    }

    fn add_separator(&mut self) {
        self.append(UNDERSCORE);
    }

    fn get_string(&self, start: usize) -> String {
        String::from_utf16_lossy(&self.buffer[start..])
    }

    fn set(&mut self, pos: usize, s: &str) {
        self.buffer.truncate(pos);
        self.buffer.extend(s.encode_utf16());
    }

    fn append_str(&mut self, s: &str) {
        self.buffer.extend(s.encode_utf16());
    }

    // utilities for parsing text out of the id

    fn next(&mut self) -> u16 {
        if self.index == self.id.len() {
            self.index += 1;
            return DONE;
        }
        let c = self.id[self.index];
        self.index += 1;
        c
    }

    fn skip_until_terminator_or_id_separator(&mut self) {
        loop {
            let c = self.next();
            if Self::is_terminator_or_id_separator(c) {
                break;
            }
        }
        self.index -= 1;
    }

    fn at_terminator(&self) -> bool {
        self.index >= self.id.len() || Self::is_terminator(self.id[self.index])
    }

    fn is_terminator(c: u16) -> bool {
        // always terminate at DOT, even if not handling POSIX.  It's an error...
        c == KEYWORD_SEPARATOR || c == DONE || c == DOT
    }

    fn is_terminator_or_id_separator(c: u16) -> bool {
        c == UNDERSCORE || c == HYPHEN || Self::is_terminator(c)
    }

    fn have_experimental_language_prefix(&self) -> bool {
        if self.id.len() > 2 {
            let c = self.id[1];
            if c == HYPHEN || c == UNDERSCORE {
                let c = self.id[0];
                return c == b'x' as u16 || c == b'X' as u16 || c == b'i' as u16 || c == b'I' as u16;
            }
        }
        false
    }

    fn have_keyword_assign(&self) -> bool {
        // assume it is safe to start from index
        self.id[self.index.min(self.id.len())..].contains(&KEYWORD_ASSIGN)
    }

    fn parse_language(&mut self) -> usize {
        let start_length = self.buffer.len();

        if self.have_experimental_language_prefix() {
            let c = AsciiUtil::to_lower16(self.id[0]);
            self.append(c);
            self.append(HYPHEN);
            self.index = 2;
        }

        loop {
            let c = self.next();
            if Self::is_terminator_or_id_separator(c) {
                break;
            }
            self.append(AsciiUtil::to_lower16(c));
        }
        self.index -= 1; // unget

        if self.buffer.len() - start_length == 3 {
            if let Some(lang) = LocaleIDs::three_to_two_letter_language(&self.get_string(0)) {
                self.set(0, &lang);
            }
        }

        0
    }

    fn skip_language(&mut self) {
        if self.have_experimental_language_prefix() {
            self.index = 2;
        }
        self.skip_until_terminator_or_id_separator();
    }

    fn parse_script(&mut self) -> usize {
        if !self.at_terminator() {
            let old_index = self.index; // save original index
            self.index += 1;

            let mut old_blen = self.buffer.len(); // get before append hyphen, if we truncate everything is undone
            let mut first_pass = true;
            loop {
                let c = self.next();
                if Self::is_terminator_or_id_separator(c) || !Self::is_alpha16(c) {
                    break;
                }
                if first_pass {
                    self.add_separator();
                    self.append(AsciiUtil::to_upper16(c));
                    first_pass = false;
                } else {
                    self.append(AsciiUtil::to_lower16(c));
                }
            }
            self.index -= 1; // unget

            /* If it's not exactly 4 characters long, then it's not a script. */
            if self.index - old_index != 5 {
                // +1 to account for separator
                self.index = old_index;
                self.buffer.truncate(old_blen);
            } else {
                old_blen += 1; // index past hyphen, for clients who want to extract just the script
            }

            return old_blen;
        }
        self.buffer.len()
    }

    fn is_alpha16(c: u16) -> bool {
        (b'A' as u16..=b'Z' as u16).contains(&c) || (b'a' as u16..=b'z' as u16).contains(&c)
    }

    fn skip_script(&mut self) {
        if !self.at_terminator() {
            let old_index = self.index;
            self.index += 1;

            loop {
                let c = self.next();
                if Self::is_terminator_or_id_separator(c) || !Self::is_alpha16(c) {
                    break;
                }
            }
            self.index -= 1;

            if self.index - old_index != 5 {
                // +1 to account for separator
                self.index = old_index;
            }
        }
    }

    fn parse_country(&mut self) -> usize {
        if !self.at_terminator() {
            let old_index = self.index;
            self.index += 1;

            let mut old_blen = self.buffer.len();
            let mut first_pass = true;
            loop {
                let c = self.next();
                if Self::is_terminator_or_id_separator(c) {
                    break;
                }
                if first_pass {
                    // first, add hyphen
                    self.had_country = true; // we have a country, let variant parsing know
                    self.add_separator();
                    old_blen += 1; // increment past hyphen
                    first_pass = false;
                }
                self.append(AsciiUtil::to_upper16(c));
            }
            self.index -= 1; // unget

            let chars_appended = self.buffer.len() - old_blen;

            if chars_appended == 0 {
                // Do nothing.
            } else if !(2..=3).contains(&chars_appended) {
                // It's not a country, so return index and blen to
                // their previous values.
                self.index = old_index;
                old_blen -= 1;
                self.buffer.truncate(old_blen);
                self.had_country = false;
            } else if chars_appended == 3 {
                if let Some(region) = LocaleIDs::three_to_two_letter_region(&self.get_string(old_blen)) {
                    self.set(old_blen, &region);
                }
            }

            return old_blen;
        }

        self.buffer.len()
    }

    fn skip_country(&mut self) {
        if !self.at_terminator() {
            if self.id[self.index] == UNDERSCORE || self.id[self.index] == HYPHEN {
                self.index += 1;
            }
            /*
             * Save the index point after the separator, since the format
             * requires two separators if the country is not present.
             */
            let old_index = self.index;

            self.skip_until_terminator_or_id_separator();
            let chars_skipped = self.index - old_index;
            if !(2..=3).contains(&chars_skipped) {
                self.index = old_index;
            }
        }
    }

    fn parse_variant(&mut self) -> Result<usize> {
        let mut old_blen = self.buffer.len();

        let mut start = true;
        let mut need_separator = true;
        let mut skipping = false;
        let mut first_pass = true;

        loop {
            let mut c = self.next();
            if c == DONE {
                break;
            }
            if c == DOT {
                start = false;
                skipping = true;
            } else if c == KEYWORD_SEPARATOR {
                if self.have_keyword_assign() {
                    break;
                }
                skipping = false;
                start = false;
                need_separator = true; // add another underscore if we have more text
            } else if start {
                start = false;
                if c != UNDERSCORE && c != HYPHEN {
                    self.index -= 1;
                }
            } else if !skipping {
                if need_separator {
                    need_separator = false;
                    if first_pass && !self.had_country {
                        // no country, we'll need two
                        self.add_separator();
                        old_blen += 1; // for sure
                    }
                    self.add_separator();
                    if first_pass {
                        // only for the first separator
                        old_blen += 1;
                        first_pass = false;
                    }
                }
                c = AsciiUtil::to_upper16(c);
                if c == HYPHEN || c == COMMA {
                    c = UNDERSCORE;
                }
                self.append(c);
                if self.buffer.len() - old_blen > MAX_VARIANTS_LENGTH {
                    return Err(LuceneError::illegal_argument("variants is too long"));
                }
            }
        }
        self.index -= 1; // unget

        Ok(old_blen)
    }

    // no need for skipvariant, to get the keywords we'll just scan directly for
    // the keyword separator

    /// `getLanguage()`: the normalized language.
    pub fn get_language(&mut self) -> String {
        self.reset();
        let s = self.parse_language();
        self.get_string(s)
    }

    /// `getScript()`.
    pub fn get_script(&mut self) -> String {
        self.reset();
        self.skip_language();
        let s = self.parse_script();
        self.get_string(s)
    }

    /// `getCountry()`.
    pub fn get_country(&mut self) -> String {
        self.reset();
        self.skip_language();
        self.skip_script();
        let s = self.parse_country();
        self.get_string(s)
    }

    /// `getVariant()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn get_variant(&mut self) -> Result<String> {
        self.reset();
        self.skip_language();
        self.skip_script();
        self.skip_country();
        let s = self.parse_variant()?;
        Ok(self.get_string(s))
    }

    /// `getLanguageScriptCountryVariant()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn get_language_script_country_variant(&mut self) -> Result<[String; 4]> {
        self.reset();
        let l = self.parse_language();
        let l = self.get_string(l);
        let s = self.parse_script();
        let s = self.get_string(s);
        let c = self.parse_country();
        let c = self.get_string(c);
        let v = self.parse_variant()?;
        let v = self.get_string(v);
        Ok([l, s, c, v])
    }

    /// `setBaseName(String baseName)`.
    pub fn set_base_name(&mut self, base_name: &str) {
        self.base_name = Some(base_name.to_string());
    }

    /// `parseBaseName()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn parse_base_name(&mut self) -> Result<()> {
        if let Some(base_name) = self.base_name.clone() {
            self.set(0, &base_name);
        } else {
            self.reset();
            self.parse_language();
            self.parse_script();
            self.parse_country();
            self.parse_variant()?;

            // catch unwanted trailing underscore after country if there was no variant
            let len = self.buffer.len();
            if len > 0 && self.buffer[len - 1] == UNDERSCORE {
                self.buffer.truncate(len - 1);
            }
        }
        Ok(())
    }

    /// `getBaseName()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn get_base_name(&mut self) -> Result<String> {
        if let Some(b) = &self.base_name {
            return Ok(b.clone());
        }
        self.parse_base_name()?;
        Ok(self.get_string(0))
    }

    /// `getName()`: the normalized full id.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn get_name(&mut self) -> Result<String> {
        self.parse_base_name()?;
        self.parse_keywords();
        Ok(self.get_string(0))
    }

    // keyword utilities

    fn set_to_keyword_start(&mut self) -> bool {
        let mut i = self.index;
        while i < self.id.len() {
            if self.id[i] == KEYWORD_SEPARATOR {
                if self.canonicalize {
                    i += 1; // increment i past separator for return
                    let mut j = i;
                    while j < self.id.len() {
                        if self.id[j] == KEYWORD_ASSIGN {
                            self.index = i;
                            return true;
                        }
                        j += 1;
                    }
                } else {
                    i += 1;
                    if i < self.id.len() {
                        self.index = i;
                        return true;
                    }
                }
                break;
            }
            i += 1;
        }
        false
    }

    fn is_done_or_keyword_assign(c: u16) -> bool {
        c == DONE || c == KEYWORD_ASSIGN
    }

    fn is_done_or_item_separator(c: u16) -> bool {
        c == DONE || c == ITEM_SEPARATOR
    }

    fn get_keyword(&mut self) -> String {
        let start = self.index;
        while !Self::is_done_or_keyword_assign(self.next()) {}
        self.index -= 1;
        AsciiUtil::to_lower_string(&String::from_utf16_lossy(trim16(&self.id[start..self.index])))
    }

    fn get_value(&mut self) -> String {
        let start = self.index;
        while !Self::is_done_or_item_separator(self.next()) {}
        self.index -= 1;
        String::from_utf16_lossy(trim16(&self.id[start..self.index])) // leave case alone
    }

    /// `getKeywordMap()`: the keywords, keys lower-cased and both trimmed.
    pub fn get_keyword_map(&mut self) -> &mut KeywordMap {
        if self.keywords.is_none() {
            let mut m: Option<KeywordMap> = None;
            if self.set_to_keyword_start() {
                // trim spaces and convert to lower case, both keywords and values.
                loop {
                    let key = self.get_keyword();
                    if key.is_empty() {
                        break;
                    }
                    let c = self.next();
                    let proceed = if c != KEYWORD_ASSIGN {
                        // throw new IllegalArgumentException("key '" + key + "' missing a value.");
                        if c == DONE {
                            break;
                        }
                        false
                    } else {
                        let value = self.get_value();
                        if value.is_empty() {
                            // throw new IllegalArgumentException("key '" + key + "' missing a value.");
                            false
                        } else {
                            match &mut m {
                                None => {
                                    let mut nm = KeywordMap::default();
                                    nm.insert(key, value);
                                    m = Some(nm);
                                }
                                Some(map) => {
                                    if !map.contains_key(&key) {
                                        map.insert(key, value);
                                    }
                                    // else: throw new IllegalArgumentException("key '" + key + "' already has a value.");
                                }
                            }
                            true
                        }
                    };
                    let _ = proceed;
                    if self.next() != ITEM_SEPARATOR {
                        break;
                    }
                }
            }
            self.keywords = Some(m.unwrap_or_default());
        }
        self.keywords.as_mut().expect("set above")
    }

    fn parse_keywords(&mut self) -> usize {
        let mut old_blen = self.buffer.len();
        let entries: Vec<(String, String)> = self
            .get_keyword_map()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !entries.is_empty() {
            let mut first = true;
            for (k, v) in entries {
                self.append(if first { KEYWORD_SEPARATOR } else { ITEM_SEPARATOR });
                first = false;
                self.append_str(&k);
                self.append(KEYWORD_ASSIGN);
                self.append_str(&v);
            }
            if !first {
                old_blen += 1;
            }
        }
        old_blen
    }

    /// `getKeywords()`: the keyword names in order, `None` when there are
    /// none.
    pub fn get_keywords(&mut self) -> Option<Vec<String>> {
        let m = self.get_keyword_map();
        if m.is_empty() {
            None
        } else {
            Some(m.keys())
        }
    }

    /// `getKeywordValue(String keywordName)`.
    pub fn get_keyword_value(&mut self, keyword_name: &str) -> Option<String> {
        let key = AsciiUtil::to_lower_string(java_trim(keyword_name));
        let m = self.get_keyword_map();
        if m.is_empty() {
            None
        } else {
            m.get(&key).map(str::to_string)
        }
    }

    /// `defaultKeywordValue(String keywordName, String value)`: sets the
    /// value only when the keyword has none.
    ///
    /// # Errors
    ///
    /// As [`set_keyword_value`](Self::set_keyword_value).
    pub fn default_keyword_value(&mut self, keyword_name: Option<&str>, value: Option<&str>) -> Result<()> {
        self.set_keyword_value_impl(keyword_name, value, false)
    }

    /// `setKeywordValue(String keywordName, String value)`: `value` `None`
    /// removes the keyword; `keywordName` `None` removes all keywords.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("keyword must not be empty")` or `("value
    /// must not be empty")`.
    pub fn set_keyword_value(&mut self, keyword_name: Option<&str>, value: Option<&str>) -> Result<()> {
        self.set_keyword_value_impl(keyword_name, value, true)
    }

    fn set_keyword_value_impl(&mut self, keyword_name: Option<&str>, value: Option<&str>, reset: bool) -> Result<()> {
        match keyword_name {
            None => {
                if reset {
                    // force new map, ignore value
                    self.keywords = Some(KeywordMap::default());
                }
            }
            Some(keyword_name) => {
                let keyword_name = AsciiUtil::to_lower_string(java_trim(keyword_name));
                if keyword_name.is_empty() {
                    return Err(LuceneError::illegal_argument("keyword must not be empty"));
                }
                let value = match value {
                    Some(v) => {
                        let v = java_trim(v).to_string();
                        if v.is_empty() {
                            return Err(LuceneError::illegal_argument("value must not be empty"));
                        }
                        Some(v)
                    }
                    None => None,
                };
                let m = self.get_keyword_map();
                if m.is_empty() {
                    // it is EMPTY_MAP
                    if let Some(value) = value {
                        // force new map
                        let mut nm = KeywordMap::default();
                        nm.insert(keyword_name, value);
                        self.keywords = Some(nm);
                    }
                } else if reset || !m.contains_key(&keyword_name) {
                    match value {
                        Some(value) => m.insert(keyword_name, value),
                        None => {
                            m.remove(&keyword_name);
                            if m.is_empty() {
                                // force new map
                                self.keywords = Some(KeywordMap::default());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// `String.trim()`.
pub(crate) fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| (c as u32) <= 0x20)
}
