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
    /// Whether this map refuses every write.
    ///
    /// Java expresses the same thing with a subclass,
    /// `CharArrayMap.UnmodifiableCharArrayMap`, whose `put`, `clear` and
    /// `remove` throw `UnsupportedOperationException`
    /// (`CharArrayMap.java` lines 562-601). Rust has no implementation
    /// inheritance, so the port carries the distinction in a field, and
    /// [`is_unmodifiable`](Self::is_unmodifiable) stands in for Java's
    /// `instanceof UnmodifiableCharArrayMap` test.
    unmodifiable: bool,
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
            unmodifiable: false,
        }
    }

    /// Returns `true` if this map compares keys case-insensitively.
    pub fn ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// Clears all entries.
    ///
    /// # Panics
    ///
    /// Panics when the map is unmodifiable, where Java's
    /// `UnmodifiableCharArrayMap.clear()` throws `UnsupportedOperationException`.
    pub fn clear(&mut self) {
        self.assert_modifiable();
        self.count = 0;
        for key in &mut self.keys {
            *key = None;
        }
        for value in &mut self.values {
            *value = None;
        }
    }

    /// Returns a copy of this map.
    ///
    /// Equivalent to `CharArrayMap.copy(Map)`, whose fast path copies the two
    /// backing arrays rather than re-hashing every entry — even on a very small
    /// map that is about ten times quicker.
    pub fn copy(&self) -> Self
    where
        V: Clone,
    {
        Self {
            ignore_case: self.ignore_case,
            count: self.count,
            keys: self.keys.clone(),
            values: self.values.clone(),
            // Java's copy builds a plain `CharArrayMap`, so a copy of an
            // unmodifiable map can be written to again.
            unmodifiable: false,
        }
    }

    /// Returns an empty map that refuses every write.
    ///
    /// Equivalent to `CharArrayMap.emptyMap()`, whose `EMPTY_MAP` is an
    /// `EmptyCharArrayMap`, itself an `UnmodifiableCharArrayMap`
    /// (`CharArrayMap.java` lines 608-611).
    pub fn empty_map() -> Self {
        let mut map = Self::new(0, false);
        map.unmodifiable = true;
        map
    }

    /// Reports whether this map refuses every write.
    ///
    /// Equivalent to Java's `map instanceof CharArrayMap.UnmodifiableCharArrayMap`
    /// test, which `CharArrayMap.unmodifiableMap` and `CharArraySet.unmodifiableSet`
    /// both perform to avoid wrapping a map twice.
    pub fn is_unmodifiable(&self) -> bool {
        self.unmodifiable
    }

    /// Returns an unmodifiable view of `map`.
    ///
    /// An empty map becomes [`empty_map`](Self::empty_map), a map that already
    /// refuses writes is returned unchanged, and any other map is returned with
    /// its writes disabled.
    ///
    /// Equivalent to `CharArrayMap.unmodifiableMap(CharArrayMap)`. Java's
    /// `NullPointerException` for a null argument has no counterpart: the port
    /// takes the map by value, so there is nothing to be null.
    pub fn unmodifiable_map(mut map: Self) -> Self {
        if map.is_empty() {
            return Self::empty_map();
        }
        if map.unmodifiable {
            return map;
        }
        map.unmodifiable = true;
        map
    }

    /// Returns the hash Lucene computes for a key.
    ///
    /// Equivalent to `CharArrayMap.getHashCode(char[], int, int)`: a
    /// `31 * h + c` walk over the code points, lower-cased first when the map
    /// ignores case.
    pub fn get_hash_code(&self, text: &[char]) -> i32 {
        let mut code: i32 = 0;
        if self.ignore_case {
            for c in text {
                for lower in c.to_lowercase() {
                    code = code.wrapping_mul(31).wrapping_add(lower as i32);
                }
            }
        } else {
            for c in text {
                code = code.wrapping_mul(31).wrapping_add(*c as i32);
            }
        }
        code
    }

    /// Removing an entry is not supported.
    ///
    /// Equivalent to `CharArrayMap.remove(Object)`, which throws
    /// `UnsupportedOperationException`: the open-addressed table has no tombstone,
    /// so an entry cannot be taken out without rebuilding it.
    pub fn remove(&mut self, _key: &[char]) -> crate::error::Result<Option<V>> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "CharArrayMap does not support removal".to_string(),
        ))
    }

    /// Returns every key in the map.
    ///
    /// Equivalent to `CharArrayMap.keySet()`, which in Java returns a
    /// `CharArraySet` *view* over this map. This port returns owned keys.
    ///
    /// Java's package-private `originalKeySet()` — `return super.keySet()`,
    /// commented "helper for CharArraySet to not produce endless recursion"
    /// (`CharArrayMap.java:334-337`) — has no counterpart here, and not for the
    /// reason of write-protection. It exists because `CharArraySet.iterator()`
    /// is implemented as `map.originalKeySet().iterator()`
    /// (`CharArraySet.java:164-167`): calling the overridden `keySet()` there
    /// would hand back a `CharArraySet` whose own `iterator()` would call
    /// `keySet()` again, forever. This port's [`CharArraySet::iter`] walks the
    /// map's slots directly through [`CharArrayMap::iter`] and never consults
    /// `key_set`, so the cycle the Java helper breaks cannot form.
    ///
    /// [`CharArraySet::iter`]: crate::analysis::CharArraySet::iter
    pub fn key_set(&self) -> Vec<Vec<char>> {
        self.keys.iter().flatten().cloned().collect()
    }

    /// Returns every key with its value.
    ///
    /// Equivalent to `CharArrayMap.entrySet()`.
    pub fn entry_set(&self) -> Vec<(Vec<char>, &V)> {
        self.keys
            .iter()
            .zip(self.values.iter())
            .filter_map(|(key, value)| match (key, value) {
                (Some(key), Some(value)) => Some((key.clone(), value)),
                _ => None,
            })
            .collect()
    }

    /// Returns a view of the entries through which their values can be
    /// replaced.
    ///
    /// Equivalent to `CharArrayMap#createEntrySet` (`CharArrayMap.java:322-324`),
    /// which builds the `EntrySet` that `entrySet()` returns, together with
    /// its override in `UnmodifiableCharArrayMap` (`CharArrayMap.java:598-601`):
    /// Java picks `EntrySet(true)` or `EntrySet(false)` by subclass, this port
    /// by [`is_unmodifiable`](Self::is_unmodifiable), so the entries of an
    /// unmodifiable map refuse [`MapEntry::set_value`] exactly as Java's refuse
    /// `setValue`. [`entry_set`](Self::entry_set) stays the read-only snapshot.
    pub fn create_entry_set(&mut self) -> EntrySet<'_, V> {
        let allow_modify = !self.unmodifiable;
        EntrySet {
            map: self,
            allow_modify,
        }
    }

    /// Returns the number of entries.    /// Returns the number of entries.
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
    ///
    /// # Panics
    ///
    /// Panics when the map is unmodifiable, where Java's
    /// `UnmodifiableCharArrayMap.put(...)` throws `UnsupportedOperationException`.
    pub fn put_chars(&mut self, text: &[char], off: usize, len: usize, value: V) -> Option<V> {
        self.assert_modifiable();
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

    /// Refuses a write to an unmodifiable map.
    ///
    /// Divergence: Java raises `UnsupportedOperationException`, an unchecked
    /// exception, from the overrides in `UnmodifiableCharArrayMap`. The port's
    /// write methods do not return `Result`, so the unchecked exception becomes
    /// a panic, which is how this crate carries Java's unchecked exceptions
    /// through a method whose signature cannot report an error.
    fn assert_modifiable(&self) {
        assert!(
            !self.unmodifiable,
            "CharArrayMap is unmodifiable and cannot be written to"
        );
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

/// The entries of a [`CharArrayMap`], with the right to replace their values
/// when the map is modifiable.
///
/// Equivalent to `CharArrayMap.EntrySet` (`CharArrayMap.java:471-512`).
/// `remove(Object)`, which Java refuses unconditionally, has no counterpart.
#[derive(Debug)]
pub struct EntrySet<'a, V> {
    map: &'a mut CharArrayMap<V>,
    allow_modify: bool,
}

impl<V> EntrySet<'_, V> {
    /// Returns the number of entries.
    ///
    /// Equivalent to `EntrySet.size()`.
    pub fn size(&self) -> usize {
        self.map.count
    }

    /// Returns whether the map holds `key` mapped to `value`.
    ///
    /// Equivalent to `EntrySet.contains(Object)` for a `Map.Entry`, which
    /// compares the value mapped to the entry's key with `equals`.
    pub fn contains(&self, key: &[char], value: &V) -> bool
    where
        V: PartialEq,
    {
        self.map.get_chars(key, 0, key.len()) == Some(value)
    }

    /// Removes every entry.
    ///
    /// Equivalent to `EntrySet.clear()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation)
    /// for the entries of an unmodifiable map, where Java throws
    /// `UnsupportedOperationException`.
    pub fn clear(&mut self) -> crate::error::Result<()> {
        if !self.allow_modify {
            return Err(unmodifiable_entry());
        }
        self.map.clear();
        Ok(())
    }

    /// Iterates over the entries, in slot order.
    ///
    /// Equivalent to `EntrySet.iterator()`.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = MapEntry<'_, V>> {
        let allow_modify = self.allow_modify;
        self.map
            .keys
            .iter()
            .zip(self.map.values.iter_mut())
            .filter_map(move |(key, value)| match (key, value) {
                (Some(key), Some(value)) => Some(MapEntry {
                    key,
                    value,
                    allow_modify,
                }),
                _ => None,
            })
    }
}

/// One entry of an [`EntrySet`].
///
/// Equivalent to the private `CharArrayMap.MapEntry` (`CharArrayMap.java:433-469`).
#[derive(Debug)]
pub struct MapEntry<'a, V> {
    key: &'a [char],
    value: &'a mut V,
    allow_modify: bool,
}

impl<V> MapEntry<'_, V> {
    /// Returns a copy of the key.
    ///
    /// Equivalent to `MapEntry.getKey()`, which clones the key so that a
    /// caller cannot corrupt the map.
    pub fn get_key(&self) -> Vec<char> {
        self.key.to_vec()
    }

    /// Returns the value.
    ///
    /// Equivalent to `MapEntry.getValue()`.
    pub fn get_value(&self) -> &V {
        self.value
    }

    /// Replaces the value, returning the previous one.
    ///
    /// Equivalent to `MapEntry.setValue(V)` (`CharArrayMap.java:455-460`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation)
    /// when the map is unmodifiable, where Java throws
    /// `UnsupportedOperationException`.
    pub fn set_value(&mut self, value: V) -> crate::error::Result<V> {
        if !self.allow_modify {
            return Err(unmodifiable_entry());
        }
        Ok(std::mem::replace(self.value, value))
    }
}

impl<V: std::fmt::Display> std::fmt::Display for MapEntry<'_, V> {
    /// Renders `key=value`, as `MapEntry.toString()` does for a value that is
    /// not the map itself.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={}", self.key.iter().collect::<String>(), self.value)
    }
}

/// The `UnsupportedOperationException` an unmodifiable map's entries throw.
fn unmodifiable_entry() -> crate::error::LuceneError {
    crate::error::LuceneError::UnsupportedOperation(
        "CharArrayMap is unmodifiable and its entries cannot be written to".to_string(),
    )
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

impl<V: std::fmt::Debug> std::fmt::Display for CharArrayMap<V> {
    /// Renders the map as `{key=value, …}`.
    ///
    /// Equivalent to `CharArrayMap.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        let mut first = true;
        for (key, value) in self.entry_set() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            write!(f, "{}={value:?}", key.iter().collect::<String>())?;
        }
        write!(f, "}}")
    }
}

impl<V: PartialEq> PartialEq for CharArrayMap<V> {
    /// Two maps are equal when they hold the same keys with the same values.
    ///
    /// Equivalent to `AbstractMap.equals(Object)`, which `CharArrayMap`
    /// inherits.
    fn eq(&self, other: &Self) -> bool {
        if self.size() != other.size() {
            return false;
        }
        self.keys
            .iter()
            .zip(self.values.iter())
            .filter_map(|(k, v)| k.as_ref().zip(v.as_ref()))
            .all(|(key, value)| {
                other
                    .get_chars(key, 0, key.len())
                    .is_some_and(|found| found == value)
            })
    }
}
