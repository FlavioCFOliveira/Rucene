//! A hash map keyed by character arrays, ported from `org.apache.lucene.analysis.CharArrayMap`.
//!
//! This is a specialized, insert-only map used by the analysis pipeline. It
//! does not support removing individual entries, and keys are compared by their
//! character content, optionally ignoring case.

#![deny(unsafe_code)]

use crate::util::chars_ref::CharsRef;

const INIT_SIZE: usize = 8;

/// A hash map from character-array keys to values, equivalent to Lucene's
/// `org.apache.lucene.analysis.CharArrayMap`.
#[derive(Clone, Debug)]
pub struct CharArrayMap<V> {
    ignore_case: bool,
    count: usize,
    keys: Vec<Option<Vec<char>>>,
    values: Vec<Option<V>>,
}

impl<V> CharArrayMap<V> {
    /// Creates a map with enough capacity to hold at least `start_size` terms.
    pub fn new(start_size: usize, ignore_case: bool) -> Self {
        let mut size = INIT_SIZE;
        while start_size + (start_size >> 2) > size {
            size <<= 1;
        }
        Self {
            ignore_case,
            count: 0,
            keys: (0..size).map(|_| None).collect(),
            values: (0..size).map(|_| None).collect(),
        }
    }

    /// Returns `true` if this map compares keys case-insensitively.
    pub fn ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// Clears all entries.
    pub fn clear(&mut self) {
        self.count = 0;
        for key in &mut self.keys {
            *key = None;
        }
        for value in &mut self.values {
            *value = None;
        }
    }

    /// Returns the number of entries.
    pub fn size(&self) -> usize {
        self.count
    }

    /// Returns `true` if the map contains no entries.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the value associated with `key`, or `None` if absent.
    pub fn get(&self, key: impl AsRef<str>) -> Option<&V> {
        let chars: Vec<char> = key.as_ref().chars().collect();
        self.get_chars(&chars, 0, chars.len())
    }

    /// Returns the value associated with the `len` characters of `text` starting
    /// at `off`, or `None` if absent.
    pub fn get_chars(&self, text: &[char], off: usize, len: usize) -> Option<&V> {
        let normalized = self.normalize(&text[off..off + len]);
        let slot = self.get_slot(&normalized);
        self.values[slot].as_ref()
    }

    /// Returns `true` if `key` is present.
    pub fn contains_key(&self, key: impl AsRef<str>) -> bool {
        let chars: Vec<char> = key.as_ref().chars().collect();
        self.contains_key_chars(&chars, 0, chars.len())
    }

    /// Returns `true` if the `len` characters of `text` starting at `off` are present.
    pub fn contains_key_chars(&self, text: &[char], off: usize, len: usize) -> bool {
        self.get_chars(text, off, len).is_some()
    }

    /// Inserts `value` under `key`, returning the previous value if any.
    pub fn put(&mut self, key: impl AsRef<str>, value: V) -> Option<V> {
        let chars: Vec<char> = key.as_ref().chars().collect();
        self.put_chars(&chars, 0, chars.len(), value)
    }

    /// Inserts `value` under the `len` characters of `text` starting at `off`,
    /// returning the previous value if any.
    pub fn put_chars(&mut self, text: &[char], off: usize, len: usize, value: V) -> Option<V> {
        let normalized = self.normalize(&text[off..off + len]);
        let slot = self.get_slot(&normalized);
        if self.keys[slot].is_some() {
            let old = self.values[slot].take();
            self.values[slot] = Some(value);
            return old;
        }
        self.keys[slot] = Some(normalized);
        self.values[slot] = Some(value);
        self.count += 1;
        if self.count + (self.count >> 2) > self.keys.len() {
            self.rehash();
        }
        None
    }

    /// Returns an iterator over the key/value pairs in the map.
    pub fn iter(&self) -> Iter<'_, V> {
        Iter { map: self, pos: 0 }
    }

    /// Normalizes a key according to the map's case policy.
    fn normalize(&self, text: &[char]) -> Vec<char> {
        if self.ignore_case {
            text.iter().flat_map(|c| c.to_lowercase()).collect()
        } else {
            text.to_vec()
        }
    }

    /// Returns the table slot for `text`, whether occupied or not.
    fn get_slot(&self, text: &[char]) -> usize {
        let mut code = CharsRef::string_hash_code(text, 0, text.len());
        let mask = self.keys.len() - 1;
        let mut pos = (code as u32 as usize) & mask;
        let inc = ((code >> 8).wrapping_add(code)) | 1;
        while self.keys[pos].is_some() && self.keys[pos].as_deref().unwrap() != text {
            code = code.wrapping_add(inc);
            pos = (code as u32 as usize) & mask;
        }
        pos
    }

    /// Doubles the table size and rehashes existing entries.
    fn rehash(&mut self) {
        let new_size = self.keys.len() * 2;
        let old_keys = std::mem::take(&mut self.keys);
        let mut old_values = std::mem::take(&mut self.values);
        self.keys = (0..new_size).map(|_| None).collect();
        self.values = (0..new_size).map(|_| None).collect();
        for i in 0..old_keys.len() {
            if let Some(key) = &old_keys[i] {
                let slot = self.get_slot(key);
                self.keys[slot] = old_keys[i].clone();
                self.values[slot] = old_values[i].take();
            }
        }
    }
}

impl<'a, V> IntoIterator for &'a CharArrayMap<V> {
    type Item = (&'a [char], &'a V);
    type IntoIter = Iter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the entries of a [`CharArrayMap`].
#[derive(Clone, Debug)]
pub struct Iter<'a, V> {
    map: &'a CharArrayMap<V>,
    pos: usize,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a [char], &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.map.keys.len() && self.map.keys[self.pos].is_none() {
            self.pos += 1;
        }
        if self.pos >= self.map.keys.len() {
            return None;
        }
        let key = self.map.keys[self.pos].as_deref().unwrap();
        let value = self.map.values[self.pos].as_ref().unwrap();
        self.pos += 1;
        Some((key, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_put_get_and_overwrite() {
        let mut map = CharArrayMap::new(4, false);
        assert!(map.is_empty());
        assert_eq!(map.put("one", 1), None);
        assert_eq!(map.put("two", 2), None);
        assert_eq!(map.size(), 2);
        assert_eq!(map.get("one"), Some(&1));
        assert_eq!(map.put("one", 10), Some(1));
        assert_eq!(map.get("one"), Some(&10));
    }

    #[test]
    fn contains_key_and_clear() {
        let mut map = CharArrayMap::new(4, false);
        map.put("alpha", 'a');
        map.put("beta", 'b');
        assert!(map.contains_key("alpha"));
        assert!(!map.contains_key("gamma"));
        map.clear();
        assert!(map.is_empty());
        assert!(!map.contains_key("alpha"));
    }

    #[test]
    fn case_insensitive_lookup() {
        let mut map = CharArrayMap::new(4, true);
        map.put("Hello", 1);
        assert_eq!(map.get("hello"), Some(&1));
        assert_eq!(map.get("HELLO"), Some(&1));
        assert!(map.contains_key("HeLLo"));
        assert_eq!(map.size(), 1);
    }

    #[test]
    fn char_array_methods() {
        let mut map = CharArrayMap::new(4, false);
        let key: Vec<char> = "token".chars().collect();
        assert_eq!(map.put_chars(&key, 0, key.len(), "value"), None);
        assert_eq!(map.get_chars(&key, 0, key.len()), Some(&"value"));
        assert!(map.contains_key_chars(&key, 0, key.len()));
    }

    #[test]
    fn iteration() {
        let mut map = CharArrayMap::new(4, false);
        map.put("first", 1);
        map.put("second", 2);
        map.put("third", 3);

        let mut keys: Vec<String> = map.iter().map(|(k, _)| k.iter().collect()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );

        let sum: i32 = map.iter().map(|(_, v)| *v).sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn rehash_preserves_entries() {
        let mut map = CharArrayMap::new(2, false);
        for i in 0..50 {
            map.put(format!("key-{i}"), i);
        }
        assert_eq!(map.size(), 50);
        for i in 0..50 {
            assert_eq!(map.get(format!("key-{i}")), Some(&i));
        }
    }

    #[test]
    fn empty_key() {
        let mut map = CharArrayMap::new(2, false);
        assert_eq!(map.put("", "empty"), None);
        assert_eq!(map.get(""), Some(&"empty"));
        assert!(map.contains_key(""));
        assert_eq!(map.size(), 1);
    }
}
