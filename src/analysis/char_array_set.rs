//! A hash set keyed by character arrays, ported from `org.apache.lucene.analysis.CharArraySet`.
//!
//! This is a specialized set used by the analysis pipeline. It does not support
//! removing individual entries; keys are compared by their character content,
//! optionally ignoring case.

#![deny(unsafe_code)]

use crate::analysis::char_array_map::{CharArrayMap, Iter as MapIter};

/// A hash set of character-array keys, equivalent to Lucene's
/// `org.apache.lucene.analysis.CharArraySet`.
#[derive(Clone, Debug)]
pub struct CharArraySet {
    map: CharArrayMap<()>,
}

impl CharArraySet {
    /// Creates a set with enough capacity to hold at least `start_size` terms.
    pub fn new(start_size: usize, ignore_case: bool) -> Self {
        Self {
            map: CharArrayMap::new(start_size, ignore_case),
        }
    }

    /// Returns `true` if this set compares keys case-insensitively.
    pub fn ignore_case(&self) -> bool {
        self.map.ignore_case()
    }

    /// Creates a set from an iterator of string-like values.
    pub fn from_iter<I, S>(items: I, ignore_case: bool) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let collected: Vec<S> = items.into_iter().collect();
        let mut set = Self::new(collected.len(), ignore_case);
        for item in collected {
            set.add(item);
        }
        set
    }

    /// Clears all entries.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Returns the number of entries.
    pub fn size(&self) -> usize {
        self.map.size()
    }

    /// Returns `true` if the set contains no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Adds `key` to the set, returning `true` if it was not already present.
    pub fn add(&mut self, key: impl AsRef<str>) -> bool {
        let chars: Vec<char> = key.as_ref().chars().collect();
        self.add_chars(&chars, 0, chars.len())
    }

    /// Adds the `len` characters of `text` starting at `off` to the set.
    pub fn add_chars(&mut self, text: &[char], off: usize, len: usize) -> bool {
        self.map.put_chars(text, off, len, ()).is_none()
    }

    /// Returns `true` if `key` is in the set.
    pub fn contains(&self, key: impl AsRef<str>) -> bool {
        let chars: Vec<char> = key.as_ref().chars().collect();
        self.contains_chars(&chars, 0, chars.len())
    }

    /// Returns `true` if the `len` characters of `text` starting at `off` are in the set.
    pub fn contains_chars(&self, text: &[char], off: usize, len: usize) -> bool {
        self.map.contains_key_chars(text, off, len)
    }

    /// Returns an iterator over the keys of the set.
    ///
    /// The returned character slices are internal keys and must not be modified.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self.map.iter(),
        }
    }
}

impl<'a> IntoIterator for &'a CharArraySet {
    type Item = &'a [char];
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the keys of a [`CharArraySet`].
#[derive(Clone, Debug)]
pub struct Iter<'a> {
    inner: MapIter<'a, ()>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a [char];

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(key, _)| key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_contains() {
        let mut set = CharArraySet::new(4, false);
        assert!(set.add("hello"));
        assert!(!set.add("hello"));
        assert!(set.contains("hello"));
        assert!(!set.contains("world"));
        assert_eq!(set.size(), 1);
    }

    #[test]
    fn construction_from_iterator_and_slice() {
        let words = vec!["a", "an", "the", "is", "are"];
        let set = CharArraySet::from_iter(words, false);
        assert_eq!(set.size(), 5);
        for word in ["a", "an", "the", "is", "are"] {
            assert!(set.contains(word));
        }
    }

    #[test]
    fn case_insensitive_set() {
        let mut set = CharArraySet::new(4, true);
        assert!(set.add("Stop"));
        assert!(set.contains("stop"));
        assert!(set.contains("STOP"));
        assert!(set.contains("StOp"));
        assert_eq!(set.size(), 1);
    }

    #[test]
    fn clear_and_empty() {
        let mut set = CharArraySet::from_iter(["one", "two"], false);
        assert!(!set.is_empty());
        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.size(), 0);
        assert!(!set.contains("one"));
    }

    #[test]
    fn iteration() {
        let set = CharArraySet::from_iter(["beta", "alpha", "gamma"], false);
        let mut keys: Vec<String> = set.iter().map(|k| k.iter().collect()).collect();
        keys.sort();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);

        let also_keys: Vec<String> = (&set).into_iter().map(|k| k.iter().collect()).collect();
        assert_eq!(also_keys.len(), 3);
    }

    #[test]
    fn char_array_methods() {
        let mut set = CharArraySet::new(4, false);
        let chars: Vec<char> = "token".chars().collect();
        assert!(set.add_chars(&chars, 0, chars.len()));
        assert!(!set.add_chars(&chars, 0, chars.len()));
        assert!(set.contains_chars(&chars, 0, chars.len()));
    }

    #[test]
    fn empty_entry() {
        let mut set = CharArraySet::new(2, false);
        assert!(set.add(""));
        assert!(set.contains(""));
        assert_eq!(set.size(), 1);
    }
}
