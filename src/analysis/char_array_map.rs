//! A hash map keyed by character arrays, ported from `org.apache.lucene.analysis.CharArrayMap`.
//!
//! This is a specialized, insert-only map used by the analysis pipeline. It
//! does not support removing individual entries, and keys are compared by their
//! character content, optionally ignoring case.
//!
//! Keys are UTF-16 code units (`u16`), as Java's `char[]` keys are, so a key
//! may hold a lone surrogate. The string-taking methods (`put`, `get`,
//! `contains_key`) take a Rust string, which cannot hold one, and encode it
//! to UTF-16 first, as Java's `String.toCharArray()` does.

#![deny(unsafe_code)]

const INIT_SIZE: usize = 8;

/// A hash map from character-array keys to values, equivalent to Lucene's
/// `org.apache.lucene.analysis.CharArrayMap`.
#[derive(Clone, Debug)]
pub struct CharArrayMap<V> {
    ignore_case: bool,
    count: usize,
    keys: Vec<Option<Vec<u16>>>,
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
    /// Whether this map is `CharArrayMap.EMPTY_MAP`, the `EmptyCharArrayMap`
    /// singleton [`empty_map`](Self::empty_map) stands for: Java's `copy`
    /// answers that same instance for it (`CharArrayMap.java:538`).
    empty_map: bool,
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
            empty_map: false,
        }
    }

    /// Creates a map from the entries of `c`.
    ///
    /// Equivalent to `CharArrayMap(Map, boolean)` (`CharArrayMap.java:80-83`):
    /// `this(c.size(), ignoreCase)`, then `putAll(c)`. Java's map keys are
    /// stored by their `toString()`; here they are strings.
    pub fn from_map<K, I>(c: I, ignore_case: bool) -> Self
    where
        K: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        let entries: Vec<(K, V)> = c.into_iter().collect();
        let mut map = Self::new(entries.len(), ignore_case);
        map.put_all(entries);
        map
    }

    /// Copies every mapping of `m` into this map.
    ///
    /// Equivalent to `AbstractMap.putAll(Map)`, which `CharArrayMap` inherits:
    /// `put(e.getKey(), e.getValue())` for each entry, in `m`'s iteration
    /// order. Java's `put(Object, V)` stores a non-`char[]` key by its
    /// `toString()`; here the keys are strings.
    ///
    /// # Panics
    ///
    /// Panics on the first entry when the map is unmodifiable, as
    /// [`put`](Self::put) does.
    pub fn put_all<K, I>(&mut self, m: I)
    where
        K: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        for (key, value) in m {
            self.put(key, value);
        }
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
        // `if (map == EMPTY_MAP) return emptyMap();` (`CharArrayMap.java:538`).
        if self.empty_map {
            return Self::empty_map();
        }
        Self {
            ignore_case: self.ignore_case,
            count: self.count,
            keys: self.keys.clone(),
            values: self.values.clone(),
            // Java's copy builds a plain `CharArrayMap`, so a copy of an
            // unmodifiable map can be written to again.
            unmodifiable: false,
            empty_map: false,
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
        map.empty_map = true;
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
    /// Equivalent to `CharArrayMap.getHashCode(char[], int, int)`
    /// (`CharArrayMap.java:266-279`), the hash [`get_slot`](Self::get_slot)
    /// probes from: when the map ignores case, a `31 * h + c` walk over the
    /// code points (`Character.codePointAt(text, i, stop)`, so a lone
    /// surrogate is a code point of its own), each replaced by
    /// `Character.toLowerCase(int)`; otherwise `CharsRef.stringHashCode`, the
    /// same walk over the UTF-16 code units.
    pub fn get_hash_code(&self, text: &[u16]) -> i32 {
        let mut code: i32 = 0;
        if self.ignore_case {
            let stop = text.len();
            let mut i = 0;
            while i < stop {
                let code_point_at = crate::analysis::character_utils::code_point_at(text, i, stop);
                code = code.wrapping_mul(31).wrapping_add(
                    crate::util::automaton::case_folding::to_lower_case(code_point_at),
                );
                i += crate::analysis::character_utils::char_count(code_point_at);
            }
        } else {
            for &unit in text {
                code = code.wrapping_mul(31).wrapping_add(i32::from(unit));
            }
        }
        code
    }

    /// Removing an entry is not supported.
    ///
    /// Equivalent to `CharArrayMap.remove(Object)`, which throws
    /// `UnsupportedOperationException`: the open-addressed table has no tombstone,
    /// so an entry cannot be taken out without rebuilding it.
    pub fn remove(&mut self, _key: &[u16]) -> crate::error::Result<Option<V>> {
        Err(crate::error::LuceneError::unsupported_operation(
            "CharArrayMap does not support removal".to_string(),
        ))
    }

    /// Returns a live view of the map's keys.
    ///
    /// Equivalent to `CharArrayMap.keySet()` (`CharArrayMap.java:339-371`):
    /// a `CharArraySet` over this very map, whose `add` overloads throw
    /// `UnsupportedOperationException` so that no key can be added without a
    /// value, and whose other operations (`size`, `contains`, iteration,
    /// `clear`, `toString`) read or write the map.
    ///
    /// **Divergence from Lucene 10.5.0.** Java caches the view in a field and
    /// its callers keep the reference alive beside the map. A Rust view that
    /// can clear the map must borrow it mutably, so each call returns a new
    /// [`KeySet`] borrowing the map, and a caller re-acquires the view where
    /// Java keeps its reference; every operation behaves as Java's.
    ///
    /// Java's package-private `originalKeySet()` — `return super.keySet()`,
    /// "helper for CharArraySet to not produce endless recursion"
    /// (`CharArrayMap.java:334-337`) — has no counterpart: this port's
    /// [`CharArraySet::iter`](crate::analysis::CharArraySet::iter) walks the
    /// map's slots through [`CharArrayMap::iter`] and never consults
    /// `key_set`, so the cycle the Java helper breaks cannot form.
    pub fn key_set(&mut self) -> KeySet<'_, V> {
        KeySet { map: self }
    }

    /// Returns a view of the map's values.
    ///
    /// Equivalent to `AbstractMap.values()`, which `CharArrayMap` inherits: a
    /// collection backed by the map, iterated in slot order.
    pub fn values(&self) -> Values<'_, V> {
        Values { map: self }
    }

    /// Returns a live view of the map's entries.
    ///
    /// Equivalent to `CharArrayMap.entrySet()` (`CharArrayMap.java:325-331`),
    /// which builds the view once with [`create_entry_set`](Self::create_entry_set).
    /// The view borrows the map for the reason [`key_set`](Self::key_set)
    /// does; its [`EntrySet::iterator`] does not, so the map stays usable
    /// while the iterator walks it, as in Java.
    pub fn entry_set(&mut self) -> EntrySet<'_, V> {
        self.create_entry_set()
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
    /// `setValue`.
    pub fn create_entry_set(&mut self) -> EntrySet<'_, V> {
        let allow_modify = !self.unmodifiable;
        EntrySet {
            map: self,
            allow_modify,
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
    ///
    /// Equivalent to `CharArrayMap.get(CharSequence)`.
    pub fn get(&self, key: impl AsRef<str>) -> Option<&V> {
        let chars: Vec<u16> = key.as_ref().encode_utf16().collect();
        self.get_chars(&chars, 0, chars.len())
    }

    /// Returns the value associated with the `len` characters of `text` starting
    /// at `off`, or `None` if absent.
    ///
    /// Equivalent to `CharArrayMap.get(char[], int, int)`.
    pub fn get_chars(&self, text: &[u16], off: usize, len: usize) -> Option<&V> {
        let slot = self.get_slot(&text[off..off + len]);
        self.values[slot].as_ref()
    }

    /// Returns `true` if `key` is present.
    ///
    /// Equivalent to `CharArrayMap.containsKey(CharSequence)`.
    pub fn contains_key(&self, key: impl AsRef<str>) -> bool {
        let chars: Vec<u16> = key.as_ref().encode_utf16().collect();
        self.contains_key_chars(&chars, 0, chars.len())
    }

    /// Returns `true` if the `len` characters of `text` starting at `off` are present.
    ///
    /// Equivalent to `CharArrayMap.containsKey(char[], int, int)`.
    pub fn contains_key_chars(&self, text: &[u16], off: usize, len: usize) -> bool {
        self.get_chars(text, off, len).is_some()
    }

    /// Inserts `value` under `key`, returning the previous value if any.
    ///
    /// Equivalent to `CharArrayMap.put(String, V)`, `put(text.toCharArray(),
    /// value)`: the key is lower-cased on a copy.
    pub fn put(&mut self, key: impl AsRef<str>, value: V) -> Option<V> {
        let mut chars: Vec<u16> = key.as_ref().encode_utf16().collect();
        let len = chars.len();
        self.put_chars(&mut chars, 0, len, value)
    }

    /// Inserts `value` under the `len` characters of `text` starting at `off`,
    /// returning the previous value if any.
    ///
    /// # Panics
    ///
    /// Panics when the map is unmodifiable, where Java's
    /// `UnmodifiableCharArrayMap.put(...)` throws `UnsupportedOperationException`.
    ///
    /// Equivalent to `CharArrayMap.put(char[], V)` (`CharArrayMap.java:185-208`),
    /// which lower-cases the key with `CharacterUtils.toLowerCase` first when
    /// the map ignores case, on the caller's array: "If ignoreCase is true for
    /// this Set, the text array will be directly modified." Java then keeps
    /// that array as the key, and its contract forbids the caller to modify it
    /// afterwards; this map keeps a copy of it.
    pub fn put_chars(&mut self, text: &mut [u16], off: usize, len: usize, value: V) -> Option<V> {
        self.assert_modifiable();
        if self.ignore_case {
            crate::analysis::character_utils::to_lower_case_utf16(text, off, off + len);
        }
        let key = text[off..off + len].to_vec();
        let slot = self.get_slot(&key);
        if self.keys[slot].is_some() {
            let old = self.values[slot].take();
            self.values[slot] = Some(value);
            return old;
        }
        self.keys[slot] = Some(key);
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
    /// Java's `UnmodifiableCharArrayMap` overrides throw
    /// `new UnsupportedOperationException()` (`CharArrayMap.java:562-601`).
    /// The port's write methods do not return `Result`, so that unchecked
    /// exception is thrown as a panic carrying the Java class.
    fn assert_modifiable(&self) {
        if self.unmodifiable {
            crate::store::java_nio_file::throw_unchecked(
                "java.lang.UnsupportedOperationException",
                None,
            );
        }
    }

    /// Returns the table slot for `text`, whether occupied or not.
    ///
    /// Equivalent to the private `CharArrayMap.getSlot(char[], int, int)`
    /// (`CharArrayMap.java:136-149`): probe from
    /// [`get_hash_code`](Self::get_hash_code) of the text as given, stepping by
    /// `((code >> 8) + code) | 1`, until an empty slot or a key
    /// [`equals`](Self::equals) accepts.
    fn get_slot(&self, text: &[u16]) -> usize {
        let mut code = self.get_hash_code(text);
        let mask = self.keys.len() - 1;
        let mut pos = (code as u32 as usize) & mask;
        let inc = ((code >> 8).wrapping_add(code)) | 1;
        while let Some(key) = self.keys[pos].as_deref() {
            if self.equals(text, key) {
                break;
            }
            code = code.wrapping_add(inc);
            pos = (code as u32 as usize) & mask;
        }
        pos
    }

    /// Returns whether the lookup `text` matches the stored `key`.
    ///
    /// Equivalent to the private `CharArrayMap.equals(char[], int, int, char[])`
    /// (`CharArrayMap.java:230-246`): the UTF-16 lengths must agree, and every
    /// code point of `text`, lower-cased with `Character.toLowerCase(int)` when
    /// the map ignores case, must equal the stored key's code point at the
    /// same index (stored keys are already lower-cased); otherwise every unit
    /// must agree.
    fn equals(&self, text: &[u16], key: &[u16]) -> bool {
        let len = text.len();
        if len != key.len() {
            return false;
        }
        if self.ignore_case {
            let mut i = 0;
            while i < len {
                let code_point_at = crate::analysis::character_utils::code_point_at(text, i, len);
                if crate::util::automaton::case_folding::to_lower_case(code_point_at)
                    != crate::analysis::character_utils::code_point_at(key, i, key.len())
                {
                    return false;
                }
                i += crate::analysis::character_utils::char_count(code_point_at);
            }
            true
        } else {
            text == key
        }
    }

    /// Doubles the table size and rehashes existing entries.
    fn rehash(&mut self) {
        debug_assert!(self.keys.len() == self.values.len());
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
    type Item = (&'a [u16], &'a V);
    type IntoIter = Iter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// A live view of the keys of a [`CharArrayMap`].
///
/// Equivalent to the anonymous `CharArraySet` subclass `CharArrayMap.keySet()`
/// returns (`CharArrayMap.java:339-371`): a set over the map itself whose `add`
/// overloads throw `UnsupportedOperationException`.
#[derive(Debug)]
pub struct KeySet<'a, V> {
    map: &'a mut CharArrayMap<V>,
}

impl<V> KeySet<'_, V> {
    /// Returns the number of keys.
    ///
    /// Equivalent to `CharArraySet.size()`, `map.size()`.
    pub fn size(&self) -> usize {
        self.map.size()
    }

    /// Returns `true` if the map has no keys.
    ///
    /// Equivalent to `isEmpty()`, inherited from `AbstractCollection` as
    /// `size() == 0`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns `true` if `key` is a key of the map.
    ///
    /// Equivalent to `CharArraySet.contains(CharSequence)`,
    /// `map.containsKey(cs)`.
    pub fn contains(&self, key: impl AsRef<str>) -> bool {
        self.map.contains_key(key)
    }

    /// Returns `true` if the `len` characters of `text` starting at `off` are
    /// a key of the map.
    ///
    /// Equivalent to `CharArraySet.contains(char[], int, int)`,
    /// `map.containsKey(text, off, len)`.
    pub fn contains_chars(&self, text: &[u16], off: usize, len: usize) -> bool {
        self.map.contains_key_chars(text, off, len)
    }

    /// Iterates over the keys, in slot order.
    ///
    /// Equivalent to `CharArraySet.iterator()`, `map.originalKeySet().iterator()`.
    pub fn iter(&self) -> impl Iterator<Item = &[u16]> {
        self.map.iter().map(|(key, _)| key)
    }

    /// Removes every entry of the map.
    ///
    /// Equivalent to `CharArraySet.clear()`, `map.clear()`.
    ///
    /// # Panics
    ///
    /// Panics when the map is unmodifiable, as [`CharArrayMap::clear`] does.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Refuses to add `text`: a key cannot be added without a value.
    ///
    /// Equivalent to the `add(String)`, `add(CharSequence)` and `add(Object)`
    /// overrides of the view (`CharArrayMap.java:345-358`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneErrorKind::UnsupportedOperation`](crate::error::LuceneErrorKind::UnsupportedOperation),
    /// Java's `UnsupportedOperationException`.
    pub fn add(&mut self, text: impl AsRef<str>) -> crate::error::Result<bool> {
        let _ = text;
        Err(unsupported_key_set_add())
    }

    /// Refuses to add the `len` characters of `text` starting at `off`.
    ///
    /// Equivalent to the `add(char[])` override of the view
    /// (`CharArrayMap.java:360-363`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneErrorKind::UnsupportedOperation`](crate::error::LuceneErrorKind::UnsupportedOperation),
    /// Java's `UnsupportedOperationException`.
    pub fn add_chars(
        &mut self,
        text: &mut [u16],
        off: usize,
        len: usize,
    ) -> crate::error::Result<bool> {
        let _ = (text, off, len);
        Err(unsupported_key_set_add())
    }
}

impl<V> std::fmt::Display for KeySet<'_, V> {
    /// Port of `CharArraySet.toString()`: the keys, each rendered as its own
    /// characters, separated by `", "` and wrapped in square brackets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        for (i, key) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(&crate::analysis::character_utils::utf16_to_string(key))?;
        }
        f.write_str("]")
    }
}

/// The `UnsupportedOperationException` the key-set view's `add` throws.
fn unsupported_key_set_add() -> crate::error::LuceneError {
    crate::error::LuceneError::unsupported_operation(
        "the key set of a CharArrayMap cannot add keys".to_string(),
    )
}

/// A view of the values of a [`CharArrayMap`].
///
/// Equivalent to the collection `AbstractMap.values()` returns.
#[derive(Debug)]
pub struct Values<'a, V> {
    map: &'a CharArrayMap<V>,
}

impl<'a, V> Values<'a, V> {
    /// Returns the number of values.
    ///
    /// Equivalent to `size()`, the map's size.
    pub fn size(&self) -> usize {
        self.map.size()
    }

    /// Iterates over the values, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = &'a V> {
        self.map.iter().map(|(_, value)| value)
    }
}

impl<V: std::fmt::Display> std::fmt::Display for Values<'_, V> {
    /// Port of `AbstractCollection.toString()`: the values, each rendered by
    /// `String.valueOf`, separated by `", "` and wrapped in square brackets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        for (i, value) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{value}")?;
        }
        f.write_str("]")
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
    /// Returns an iterator over the entries.
    ///
    /// Equivalent to `EntrySet.iterator()`, `new EntryIterator(allowModify)`.
    /// The iterator does not borrow the map: like Java's inner class, which
    /// reads the arrays of its enclosing map, each of its methods takes the
    /// map it walks.
    pub fn iterator(&self) -> EntryIterator {
        EntryIterator::new(self.map, self.allow_modify)
    }

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
    pub fn contains(&self, key: &[u16], value: &V) -> bool
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
    /// Returns [`LuceneErrorKind::UnsupportedOperation`](crate::error::LuceneErrorKind::UnsupportedOperation)
    /// for the entries of an unmodifiable map, where Java throws
    /// `UnsupportedOperationException`.
    pub fn clear(&mut self) -> crate::error::Result<()> {
        if !self.allow_modify {
            return Err(unmodifiable_entry());
        }
        self.map.clear();
        Ok(())
    }
}

impl<V: std::fmt::Display> std::fmt::Display for EntrySet<'_, V> {
    /// Port of `AbstractCollection.toString()` over the entries: each
    /// rendered by `MapEntry.toString()`, separated by `", "` and wrapped in
    /// square brackets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        let mut iterator = self.iterator();
        let mut first = true;
        while iterator.has_next(self.map) {
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            let entry = iterator.next(self.map);
            f.write_str(&entry.to_string(self.map))?;
        }
        f.write_str("]")
    }
}

/// An iterator over the entries of a [`CharArrayMap`], exposing the keys as
/// character arrays.
///
/// Equivalent to the public `CharArrayMap.EntryIterator`
/// (`CharArrayMap.java:374-431`). Java's inner class reads the `keys` and
/// `values` arrays of its enclosing map; here each method takes that map,
/// so the iterator holds only its position and the map can be read and
/// written between two calls, as in Java.
#[derive(Clone, Debug)]
pub struct EntryIterator {
    pos: isize,
    last_pos: isize,
    allow_modify: bool,
}

impl EntryIterator {
    /// `EntryIterator(boolean allowModify)`: positions on the first entry.
    fn new<V>(map: &CharArrayMap<V>, allow_modify: bool) -> Self {
        let mut iterator = Self {
            pos: -1,
            last_pos: -1,
            allow_modify,
        };
        iterator.go_next(map);
        iterator
    }

    /// `goNext()`.
    fn go_next<V>(&mut self, map: &CharArrayMap<V>) {
        self.last_pos = self.pos;
        self.pos += 1;
        while (self.pos as usize) < map.keys.len() && map.keys[self.pos as usize].is_none() {
            self.pos += 1;
        }
    }

    /// Returns `true` if another entry follows.
    ///
    /// Equivalent to `EntryIterator.hasNext()`.
    pub fn has_next<V>(&self, map: &CharArrayMap<V>) -> bool {
        (self.pos as usize) < map.keys.len()
    }

    /// Returns the next key; the returned characters must not be modified.
    ///
    /// Equivalent to `EntryIterator.nextKey()`.
    ///
    /// # Panics
    ///
    /// Panics past the last entry, where Java's `keys[lastPos]` throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn next_key<'m, V>(&mut self, map: &'m CharArrayMap<V>) -> &'m [u16] {
        self.go_next(map);
        map.keys[self.last_pos as usize]
            .as_deref()
            .expect("INVARIANT: go_next stops only on an occupied slot")
    }

    /// Returns the next key as a new string.
    ///
    /// Equivalent to `EntryIterator.nextKeyString()`.
    ///
    /// # Panics
    ///
    /// As [`next_key`](Self::next_key).
    pub fn next_key_string<V>(&mut self, map: &CharArrayMap<V>) -> String {
        crate::analysis::character_utils::utf16_to_string(self.next_key(map))
    }

    /// Returns the value of the last key returned.
    ///
    /// Equivalent to `EntryIterator.currentValue()`; `None` stands for Java's
    /// `null`.
    ///
    /// # Panics
    ///
    /// Panics before the first key, where Java's `values[-1]` throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn current_value<'m, V>(&self, map: &'m CharArrayMap<V>) -> Option<&'m V> {
        map.values[self.last_pos as usize].as_ref()
    }

    /// Replaces the value of the last key returned, returning the previous
    /// one.
    ///
    /// Equivalent to `EntryIterator.setValue(V)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::UnsupportedOperation`](crate::error::LuceneErrorKind::UnsupportedOperation)
    /// when the map is unmodifiable, where Java throws
    /// `UnsupportedOperationException`.
    pub fn set_value<V>(
        &self,
        map: &mut CharArrayMap<V>,
        value: V,
    ) -> crate::error::Result<Option<V>> {
        if !self.allow_modify {
            return Err(unmodifiable_entry());
        }
        Ok(map.values[self.last_pos as usize].replace(value))
    }

    /// Returns the next entry.
    ///
    /// Equivalent to `EntryIterator.next()`, `new MapEntry(lastPos,
    /// allowModify)`.
    pub fn next<V>(&mut self, map: &CharArrayMap<V>) -> MapEntry {
        self.go_next(map);
        MapEntry {
            pos: self.last_pos as usize,
            allow_modify: self.allow_modify,
        }
    }

    /// Removing an entry is not supported.
    ///
    /// Equivalent to `EntryIterator.remove()`.
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneErrorKind::UnsupportedOperation`](crate::error::LuceneErrorKind::UnsupportedOperation).
    pub fn remove(&mut self) -> crate::error::Result<()> {
        Err(crate::error::LuceneError::unsupported_operation(
            "CharArrayMap entries cannot be removed".to_string(),
        ))
    }
}

/// One entry of a [`CharArrayMap`], designated by its slot.
///
/// Equivalent to the private `CharArrayMap.MapEntry` (`CharArrayMap.java:433-469`),
/// an inner class holding the slot `pos`; each method takes the enclosing
/// map Java's inner class reads.
#[derive(Clone, Copy, Debug)]
pub struct MapEntry {
    pos: usize,
    allow_modify: bool,
}

impl MapEntry {
    /// Returns a copy of the key.
    ///
    /// Equivalent to `MapEntry.getKey()`, which clones the key so that a
    /// caller cannot corrupt the map.
    pub fn get_key<V>(&self, map: &CharArrayMap<V>) -> Vec<u16> {
        map.keys[self.pos].clone().unwrap_or_default()
    }

    /// Returns the value; `None` stands for Java's `null`.
    ///
    /// Equivalent to `MapEntry.getValue()`.
    pub fn get_value<'m, V>(&self, map: &'m CharArrayMap<V>) -> Option<&'m V> {
        map.values[self.pos].as_ref()
    }

    /// Replaces the value, returning the previous one.
    ///
    /// Equivalent to `MapEntry.setValue(V)` (`CharArrayMap.java:455-460`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::UnsupportedOperation`](crate::error::LuceneErrorKind::UnsupportedOperation)
    /// when the map is unmodifiable, where Java throws
    /// `UnsupportedOperationException`.
    pub fn set_value<V>(
        &self,
        map: &mut CharArrayMap<V>,
        value: V,
    ) -> crate::error::Result<Option<V>> {
        if !self.allow_modify {
            return Err(unmodifiable_entry());
        }
        Ok(map.values[self.pos].replace(value))
    }

    /// Renders `key=value`.
    ///
    /// Equivalent to `MapEntry.toString()` for a value that is not the map
    /// itself: `String.valueOf(keys[pos]) + '=' + values[pos]`, `null` for an
    /// absent value.
    pub fn to_string<V: std::fmt::Display>(&self, map: &CharArrayMap<V>) -> String {
        let key = crate::analysis::character_utils::utf16_to_string(
            map.keys[self.pos].as_deref().unwrap_or_default(),
        );
        match &map.values[self.pos] {
            Some(value) => format!("{key}={value}"),
            None => format!("{key}=null"),
        }
    }
}

/// The `UnsupportedOperationException` an unmodifiable map's entries throw.
fn unmodifiable_entry() -> crate::error::LuceneError {
    crate::error::LuceneError::unsupported_operation(
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
    type Item = (&'a [u16], &'a V);

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

    /// `CharArrayMap.put(char[], V)` lower-cases the caller's array when the
    /// map ignores case, and leaves it alone otherwise. Java oracle
    /// (`scratchpad/auditH/oracle/AuditHExtra.java`, `extra_out.txt`): `map
    /// key after put=abç𐐨` for `ABÇ𐐀`, and `case-sensitive key after
    /// put=ABC`.
    #[test]
    fn put_chars_lower_cases_the_callers_array_when_ignoring_case() {
        let mut map = CharArrayMap::new(4, true);
        let mut key: Vec<u16> = "ABÇ\u{10400}".encode_utf16().collect();
        let len = key.len();
        assert_eq!(map.put_chars(&mut key, 0, len, "v"), None);
        assert_eq!(String::from_utf16_lossy(&key), "abç\u{10428}");
        assert_eq!(map.get("abç\u{10428}"), Some(&"v"));

        let mut map = CharArrayMap::new(4, false);
        let mut key: Vec<u16> = "ABC".encode_utf16().collect();
        assert_eq!(map.put_chars(&mut key, 0, 3, "v"), None);
        assert_eq!(String::from_utf16_lossy(&key), "ABC");
    }

    #[test]
    fn char_array_methods() {
        let mut map = CharArrayMap::new(4, false);
        let mut key: Vec<u16> = "token".encode_utf16().collect();
        let len = key.len();
        assert_eq!(map.put_chars(&mut key, 0, len, "value"), None);
        assert_eq!(map.get_chars(&key, 0, key.len()), Some(&"value"));
        assert!(map.contains_key_chars(&key, 0, key.len()));
    }

    #[test]
    fn iteration() {
        let mut map = CharArrayMap::new(4, false);
        map.put("first", 1);
        map.put("second", 2);
        map.put("third", 3);

        let mut keys: Vec<String> = map
            .iter()
            .map(|(k, _)| String::from_utf16_lossy(k))
            .collect();
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
        for (key, value) in self.iter() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            write!(
                f,
                "{}={value:?}",
                crate::analysis::character_utils::utf16_to_string(key)
            )?;
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
