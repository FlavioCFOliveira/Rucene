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

    /// Returns a copy of this set, preserving its case policy.
    ///
    /// The copy is always modifiable, even when this set is not: Java's
    /// `CharArrayMap.copy` builds a plain `CharArrayMap`.
    ///
    /// Equivalent to `org.apache.lucene.analysis.CharArraySet#copy`. Java also
    /// accepts a plain `Set`, which it copies entry by entry; that path is
    /// [`from_iter`](Self::from_iter) here, because the Rust signature is
    /// already typed to a `CharArraySet`.
    pub fn copy(&self) -> Self {
        Self {
            map: self.map.copy(),
        }
    }

    /// Returns an unmodifiable view of `set`.
    ///
    /// A set that already refuses writes is returned unchanged; any other set
    /// is returned with its writes disabled, so [`add`](Self::add) and
    /// [`clear`](Self::clear) then panic.
    ///
    /// Equivalent to `org.apache.lucene.analysis.CharArraySet#unmodifiableSet`.
    /// Java's `NullPointerException` for a null argument has no counterpart:
    /// the port takes the set by value, so there is nothing to be null.
    pub fn unmodifiable_set(set: Self) -> Self {
        if set.map.is_unmodifiable() {
            return set;
        }
        Self {
            map: CharArrayMap::unmodifiable_map(set.map),
        }
    }

    /// Clears all entries.
    ///
    /// # Panics
    ///
    /// Panics when the set is unmodifiable, where Java's underlying
    /// `CharArrayMap.UnmodifiableCharArrayMap.clear()` throws
    /// `UnsupportedOperationException`.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Returns the number of entries.
    ///
    /// Equivalent to `CharArraySet.size()`.
    pub fn size(&self) -> usize {
        self.map.size()
    }

    /// Returns `true` if the set contains no entries.
    ///
    /// Equivalent to `CharArraySet.isEmpty()`, inherited from
    /// `AbstractCollection` as `size() == 0`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Adds `key` to the set, returning `true` if it was not already present.
    ///
    /// # Panics
    ///
    /// Panics when the set is unmodifiable, where Java's `CharArraySet.add`
    /// on an unmodifiable set throws `UnsupportedOperationException`.
    pub fn add(&mut self, key: impl AsRef<str>) -> bool {
        let mut chars: Vec<char> = key.as_ref().chars().collect();
        let len = chars.len();
        self.add_chars(&mut chars, 0, len)
    }

    /// Adds the `len` characters of `text` starting at `off` to the set.
    ///
    /// Equivalent to `CharArraySet.add(char[])`, `map.put(text, PLACEHOLDER)`:
    /// when the set ignores case, `text` is lower-cased in place, as
    /// [`CharArrayMap::put_chars`] documents.
    ///
    /// # Panics
    ///
    /// Panics when the set is unmodifiable, for the reason
    /// [`add`](Self::add) does.
    pub fn add_chars(&mut self, text: &mut [char], off: usize, len: usize) -> bool {
        self.map.put_chars(text, off, len, ()).is_none()
    }

    /// Returns `true` if `key` is in the set.
    ///
    /// Equivalent to `CharArraySet.contains(CharSequence)`,
    /// `map.containsKey(cs)`.
    pub fn contains(&self, key: impl AsRef<str>) -> bool {
        self.map.contains_key(key)
    }

    /// Returns `true` if the `len` characters of `text` starting at `off` are in the set.
    ///
    /// Equivalent to `CharArraySet.contains(char[], int, int)`,
    /// `map.containsKey(text, off, len)`.
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

    /// `CharArraySet.add(char[])` lower-cases the caller's array when the set
    /// ignores case. Java oracle (`scratchpad/auditH/oracle/AuditHExtra.java`,
    /// `extra_out.txt`): `set word after add=hello` for `HeLLo`.
    #[test]
    fn add_chars_lower_cases_the_callers_array_when_ignoring_case() {
        let mut set = CharArraySet::new(4, true);
        let mut word: Vec<char> = "HeLLo".chars().collect();
        assert!(set.add_chars(&mut word, 0, 5));
        assert_eq!(word.iter().collect::<String>(), "hello");
    }

    #[test]
    fn char_array_methods() {
        let mut set = CharArraySet::new(4, false);
        let mut chars: Vec<char> = "token".chars().collect();
        let len = chars.len();
        assert!(set.add_chars(&mut chars, 0, len));
        assert!(!set.add_chars(&mut chars, 0, len));
        assert!(set.contains_chars(&chars, 0, chars.len()));
    }

    /// `MiscOracle` O3 (`oracle/h2/MiscOracle.java`): the iteration order is
    /// Java's slot order, which the hash decides — `CharsRef.stringHashCode`
    /// over UTF-16 units, or `Character.toLowerCase` of each code point when
    /// the set ignores case — and lookups lower-case each code point with
    /// `Character.toLowerCase`.
    #[test]
    fn iteration_follows_javas_slot_order() {
        let mut plain = CharArraySet::new(8, false);
        for word in ["beta", "alpha", "gamma", "\u{1F600}x", "delta"] {
            plain.add(word);
        }
        assert_eq!(plain.to_string(), "[beta, \u{1F600}x, gamma, delta, alpha]");

        let mut folded = CharArraySet::new(8, true);
        for word in [
            "\u{130}stanbul",
            "\u{3A3}\u{391}\u{3A3}",
            "Stra\u{DF}e",
            "\u{10400}B",
            "Hello",
        ] {
            folded.add(word);
        }
        assert_eq!(
            folded.to_string(),
            "[istanbul, \u{3C3}\u{3B1}\u{3C3}, stra\u{DF}e, \u{10428}b, hello]"
        );

        let mut big = CharArraySet::new(2, true);
        for i in 0..30 {
            big.add(format!(
                "Key{i}{}",
                if i % 3 == 0 { "\u{10400}" } else { "" }
            ));
        }
        assert_eq!(
            big.to_string(),
            "[key13, key14, key16, key15\u{10428}, key19, key21\u{10428}, key6\u{10428}, key17, \
             key0\u{10428}, key18\u{10428}, key20, key22, key23, key25, key26, key28, key29, \
             key24\u{10428}, key12\u{10428}, key9\u{10428}, key1, key2, key3\u{10428}, key4, \
             key5, key7, key8, key27\u{10428}, key10, key11]"
        );
    }

    /// `MiscOracle` O3: `contains(CharSequence)` lower-cases each code point
    /// of the query with `Character.toLowerCase` when the set ignores case.
    #[test]
    fn lookups_lower_case_each_code_point() {
        let mut folded = CharArraySet::new(8, true);
        for word in [
            "\u{130}stanbul",
            "\u{3A3}\u{391}\u{3A3}",
            "Stra\u{DF}e",
            "\u{10400}B",
            "Hello",
        ] {
            folded.add(word);
        }
        for (query, expected) in [
            ("istanbul", true),
            ("\u{130}STANBUL", true),
            ("i\u{307}stanbul", false),
            ("\u{3C3}\u{3B1}\u{3C3}", true),
            ("\u{3C3}\u{3B1}\u{3C2}", false),
            ("\u{10428}b", true),
            ("STRASSE", false),
            ("hELLO", true),
        ] {
            assert_eq!(folded.contains(query), expected, "{query:?}");
        }
    }

    #[test]
    fn empty_entry() {
        let mut set = CharArraySet::new(2, false);
        assert!(set.add(""));
        assert!(set.contains(""));
        assert_eq!(set.size(), 1);
    }
}

impl std::fmt::Display for CharArraySet {
    /// Port of `CharArraySet.toString()`: the keys, each rendered as its own
    /// characters, separated by `", "` and wrapped in square brackets.
    ///
    /// The order is the set's iteration order, which is the hash-table slot
    /// order on both sides and is not the insertion order.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        for (i, key) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            for c in key {
                f.write_str(c.encode_utf8(&mut [0u8; 4]))?;
            }
        }
        f.write_str("]")
    }
}
