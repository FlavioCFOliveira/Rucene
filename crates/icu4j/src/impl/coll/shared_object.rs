// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.SharedObject`.

use std::sync::Arc;

/// Base of objects shared by reference with copy-on-write.
///
/// Equivalent to `com.ibm.icu.impl.coll.SharedObject`: a type is a shared
/// object when it is [`Clone`] (Java's `clone()`, which gives the copy a
/// fresh reference count) and is held through [`Reference`].
///
/// **Divergence.** Java keeps an explicit `AtomicInteger refCount` that
/// only `Reference` instances increment and decrement
/// (`SharedObject.java:91-163`); `copyOnWrite()` mutates the object in
/// place when that count is at most 1. Here the count is the [`Arc`]
/// strong count, and [`Reference::copy_on_write`] is [`Arc::make_mut`]:
/// Rust forbids mutating an object other owners can read, so any other
/// owner (not only another `Reference`) makes the write go to a copy. The
/// values seen through every reference are the same as in Java.
pub trait SharedObject: Clone {}

/// Similar to a smart pointer, basically a port of the static methods of
/// C++ `SharedObject`.
///
/// Equivalent to `SharedObject.Reference<T>`.
#[derive(Debug)]
pub struct Reference<T> {
    ref_: Option<Arc<T>>,
}

impl<T> Clone for Reference<T> {
    /// `clone()`: one more reference to the same object (`addRef()`).
    fn clone(&self) -> Self {
        Self {
            ref_: self.ref_.clone(),
        }
    }
}

impl<T: Clone> Reference<T> {
    /// `Reference(T r)`.
    pub fn new(r: Option<T>) -> Self {
        Self {
            ref_: r.map(Arc::new),
        }
    }

    /// A reference to an object already shared as an [`Arc`].
    pub fn from_arc(r: Option<Arc<T>>) -> Self {
        Self { ref_: r }
    }

    /// `readOnly()`; `None` for Java's `null`.
    pub fn read_only(&self) -> Option<&T> {
        self.ref_.as_deref()
    }

    /// The shared object itself, as Java hands out the same instance.
    pub fn read_only_arc(&self) -> Option<&Arc<T>> {
        self.ref_.as_ref()
    }

    /// Returns a writable version of the object, cloning it first when it
    /// is shared.
    ///
    /// Equivalent to `copyOnWrite()`.
    ///
    /// # Panics
    ///
    /// With Java's `NullPointerException` text when the reference is
    /// `null` (Java dereferences `ref` unconditionally).
    pub fn copy_on_write(&mut self) -> &mut T {
        match self.ref_.as_mut() {
            Some(r) => Arc::make_mut(r),
            None => panic!("java.lang.NullPointerException"),
        }
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.ref_ = None;
    }

    /// `ref.getRefCount()`: the number of owners of the object (0 for `null`).
    pub fn get_ref_count(&self) -> i32 {
        self.ref_
            .as_ref()
            .map_or(0, |r| Arc::strong_count(r) as i32)
    }
}
