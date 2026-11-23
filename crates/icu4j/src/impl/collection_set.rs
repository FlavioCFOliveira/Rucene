// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CollectionSet`.

/// A `Set` view over a collection whose elements are already distinct,
/// delegating every operation to it (cheaper than copying into a
/// `HashSet`).
///
/// Equivalent to `com.ibm.icu.impl.CollectionSet<E>`, which implements
/// `java.util.Set<E>` over a `java.util.Collection<E>`.
///
/// **Divergence.** Java wraps any `Collection` (ICU passes a map's
/// `values()` view, `MeasureUnit.java:830-832`); the port owns the
/// elements in a `Vec` (Rust cannot share a live view of another map's
/// values), in the collection's iteration order. Equality of elements is
/// `E`'s `PartialEq` (Java's `equals`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CollectionSet<E> {
    data: Vec<E>,
}

impl<E: PartialEq + Clone> CollectionSet<E> {
    /// `new CollectionSet(Collection<E> data)`.
    pub fn new(data: Vec<E>) -> Self {
        Self { data }
    }

    /// `size()`.
    pub fn size(&self) -> i32 {
        self.data.len() as i32
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `contains(Object o)`.
    pub fn contains(&self, o: &E) -> bool {
        self.data.contains(o)
    }

    /// `iterator()`.
    pub fn iter(&self) -> std::slice::Iter<'_, E> {
        self.data.iter()
    }

    /// `toArray()` (and `toArray(T[])`).
    pub fn to_array(&self) -> Vec<E> {
        self.data.clone()
    }

    /// `add(E e)`: appends to the underlying collection (a list's
    /// `add`, which always answers `true`).
    pub fn add(&mut self, e: E) -> bool {
        self.data.push(e);
        true
    }

    /// `remove(Object o)`: removes the first equal element.
    pub fn remove(&mut self, o: &E) -> bool {
        match self.data.iter().position(|x| x == o) {
            Some(i) => {
                self.data.remove(i);
                true
            }
            None => false,
        }
    }

    /// `containsAll(Collection<?> c)`.
    pub fn contains_all(&self, c: &[E]) -> bool {
        c.iter().all(|x| self.data.contains(x))
    }

    /// `addAll(Collection<? extends E> c)`.
    pub fn add_all(&mut self, c: &[E]) -> bool {
        self.data.extend_from_slice(c);
        !c.is_empty()
    }

    /// `retainAll(Collection<?> c)`.
    pub fn retain_all(&mut self, c: &[E]) -> bool {
        let before = self.data.len();
        self.data.retain(|x| c.contains(x));
        before != self.data.len()
    }

    /// `removeAll(Collection<?> c)`.
    pub fn remove_all(&mut self, c: &[E]) -> bool {
        let before = self.data.len();
        self.data.retain(|x| !c.contains(x));
        before != self.data.len()
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.data.clear();
    }
}
