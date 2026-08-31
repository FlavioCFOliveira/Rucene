//! Multisets, ported from `org.apache.lucene.search.Multiset`.

#![deny(unsafe_code)]

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

/// A set that allows for duplicate elements.
///
/// Equivalent to the `final org.apache.lucene.search.Multiset<T>`, which
/// extends `AbstractCollection<T>`. Two multisets are equal if they contain the
/// same unique elements and if each unique element has as many occurrences in
/// both multisets. Iteration order is not specified.
///
/// **Divergence from Lucene 10.5.0.** Java's element type is unconstrained
/// because every object has `hashCode`/`equals`; Rust requires the bounds to be
/// declared, so the element type is `T: Hash + Eq`. Wrap trait objects that
/// carry their own equality — [`Query`](crate::search::Query), above all — in
/// [`QueryKey`](crate::search::QueryKey).
#[derive(Debug, Clone)]
pub struct Multiset<T: Hash + Eq> {
    map: HashMap<T, i32>,
    size: usize,
}

impl<T: Hash + Eq> Default for Multiset<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Hash + Eq> Multiset<T> {
    /// Creates an empty multiset.
    ///
    /// Equivalent to `new Multiset<>()`.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
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

    /// Adds one occurrence of `element`.
    ///
    /// Equivalent to `Multiset.add(T)`, which always returns `true`.
    pub fn add(&mut self, element: T) {
        *self.map.entry(element).or_insert(0) += 1;
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

    /// Returns the number of occurrences of `element`.
    ///
    /// Equivalent to reading the private `Map<T, Integer>` that backs the
    /// multiset; Java has no accessor because the count is only used through
    /// `equals`.
    pub fn count(&self, element: &T) -> i32 {
        self.map.get(element).copied().unwrap_or(0)
    }

    /// Returns an iterator over the elements, yielding each element as many
    /// times as it occurs.
    ///
    /// Equivalent to `Multiset.iterator()`. As in Java, the order is
    /// unspecified.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.map
            .iter()
            .flat_map(|(element, count)| std::iter::repeat(element).take(*count as usize))
    }

    /// Returns an iterator over the distinct elements and their counts.
    ///
    /// Equivalent to iterating the private backing map, which Java's `equals`
    /// and `hashCode` do.
    pub fn entries(&self) -> impl Iterator<Item = (&T, i32)> {
        self.map.iter().map(|(element, count)| (element, *count))
    }
}

impl<T: Hash + Eq> PartialEq for Multiset<T> {
    fn eq(&self, other: &Self) -> bool {
        // `size == that.size` is not necessary but helps escaping early.
        self.size == other.size && self.map == other.map
    }
}

impl<T: Hash + Eq> Eq for Multiset<T> {}

impl<T: Hash + Eq> Hash for Multiset<T> {
    /// Reproduces `Multiset.hashCode()`, which is `31 * getClass().hashCode() +
    /// map.hashCode()`.
    ///
    /// **Divergence from Lucene 10.5.0.** The numeric value cannot match Java's,
    /// because the two languages hash differently. What is reproduced is the
    /// property the method exists for: the hash is independent of iteration
    /// order and is a function of the (element, count) pairs only.
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut aggregate: u64 = 0;
        for (element, count) in &self.map {
            let mut hasher = DefaultHasher::new();
            element.hash(&mut hasher);
            count.hash(&mut hasher);
            aggregate = aggregate.wrapping_add(hasher.finish());
        }
        state.write_u64(aggregate);
    }
}

impl<T: Hash + Eq> FromIterator<T> for Multiset<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut multiset = Self::new();
        multiset.add_all(iter);
        multiset
    }
}
