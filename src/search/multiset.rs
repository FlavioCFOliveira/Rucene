//! Multisets, ported from `org.apache.lucene.search.Multiset`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::hash::{Hash, Hasher};

use crate::util::java_collections::{
    java_identity_hash_code_stand_in, JavaHashCode, JavaObjectHashMap,
};

/// A set that allows for duplicate elements.
///
/// Equivalent to the `final org.apache.lucene.search.Multiset<T>`, which
/// extends `AbstractCollection<T>`. Two multisets are equal if they contain the
/// same unique elements and if each unique element has as many occurrences in
/// both multisets.
///
/// Java backs it with a `java.util.HashMap<T, Integer>` and iterates that
/// map's entries, each element repeated as many times as it occurs
/// (`Multiset.java:33-62`); this port backs it with a
/// [`JavaObjectHashMap`], so it iterates in exactly Java's order.
///
/// **Divergence from Lucene 10.5.0.** Java's element type is unconstrained
/// because every object has `hashCode`/`equals`; Rust requires the bounds to be
/// declared, so the element type is `T: JavaHashCode + Eq`. Wrap trait objects
/// that carry their own equality — [`Query`](crate::search::Query), above all —
/// in [`QueryKey`](crate::search::QueryKey).
#[derive(Debug, Clone)]
pub struct Multiset<T> {
    map: JavaObjectHashMap<T, i32>,
    size: usize,
}

impl<T> Default for Multiset<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Multiset<T> {
    /// Creates an empty multiset.
    ///
    /// Equivalent to `new Multiset<>()`.
    pub fn new() -> Self {
        Self {
            map: JavaObjectHashMap::new(),
            size: 0,
        }
    }

    /// Returns the number of elements, counting duplicates.
    ///
    /// Equivalent to `Multiset.size()`.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns whether this multiset holds no element at all.
    ///
    /// Equivalent to `AbstractCollection.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Removes every element.
    ///
    /// Equivalent to `Multiset.clear()`.
    pub fn clear(&mut self) {
        self.map.clear();
        self.size = 0;
    }

    /// Returns an iterator over the elements, yielding each element as many
    /// times as it occurs.
    ///
    /// Equivalent to `Multiset.iterator()`: the backing `HashMap`'s entries in
    /// Java's order, each key repeated `count` times.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.map
            .iter()
            .flat_map(|(element, count)| std::iter::repeat_n(element, *count as usize))
    }
}

impl<T: JavaHashCode + Eq> Multiset<T> {
    /// Adds one occurrence of `element`.
    ///
    /// Equivalent to `Multiset.add(T)`, which always returns `true`:
    /// `map.put(e, map.getOrDefault(e, 0) + 1)`, which keeps the key object
    /// already in the map.
    pub fn add(&mut self, element: T) {
        match self.map.get_mut(&element) {
            Some(count) => *count += 1,
            None => {
                self.map.insert(element, 1);
            }
        }
        self.size += 1;
    }

    /// Adds one occurrence of every element of `elements`.
    ///
    /// Equivalent to `AbstractCollection.addAll(Collection)`.
    pub fn add_all(&mut self, elements: impl IntoIterator<Item = T>) {
        for element in elements {
            self.add(element);
        }
    }

    /// Removes one occurrence of `element`, returning whether it was present.
    ///
    /// Equivalent to `Multiset.remove(Object)`.
    pub fn remove(&mut self, element: &T) -> bool {
        match self.map.get_mut(element) {
            None => false,
            Some(count) => {
                if *count == 1 {
                    self.map.remove(element);
                } else {
                    *count -= 1;
                }
                self.size -= 1;
                true
            }
        }
    }

    /// Returns whether `element` occurs at least once.
    ///
    /// Equivalent to `Multiset.contains(Object)`.
    pub fn contains(&self, element: &T) -> bool {
        self.map.contains_key(element)
    }

    /// Java's `Multiset.hashCode()`: `31 * getClass().hashCode() +
    /// map.hashCode()` (`Multiset.java:116-119`), where `map.hashCode()` sums
    /// `key.hashCode() ^ count` over the entries.
    ///
    /// `getClass().hashCode()` is a JVM identity hash, unreproducible in Java
    /// itself; it is replaced by
    /// [`java_identity_hash_code_stand_in`], which declares that divergence.
    pub fn java_hash_code(&self) -> i32 {
        31i32
            .wrapping_mul(java_identity_hash_code_stand_in(
                "org.apache.lucene.search.Multiset",
            ))
            .wrapping_add(self.map.java_hash_code_with(|count| *count))
    }
}

impl<T: JavaHashCode + Eq> PartialEq for Multiset<T> {
    /// `Multiset.equals(Object)`: the same size and `map.equals(that.map)`.
    fn eq(&self, other: &Self) -> bool {
        // `size == that.size` is not necessary but helps escaping early.
        self.size == other.size
            && self.map.len() == other.map.len()
            && self
                .map
                .iter()
                .all(|(element, count)| other.map.get(element) == Some(count))
    }
}

impl<T: JavaHashCode + Eq> Eq for Multiset<T> {}

impl<T: JavaHashCode + Eq> Hash for Multiset<T> {
    /// Feeds [`Multiset::java_hash_code`], consistent with [`PartialEq`].
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.java_hash_code());
    }
}

impl<T: JavaHashCode + Eq> FromIterator<T> for Multiset<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut multiset = Self::new();
        multiset.add_all(iter);
        multiset
    }
}

impl<T: JavaHashCode + Eq + Debug> JavaHashCode for Multiset<T> {
    fn java_hash_code(&self) -> i32 {
        Multiset::java_hash_code(self)
    }
}
