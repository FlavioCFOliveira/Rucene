// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUCache`.

/// `ICUCache.SOFT`: values held through soft references.
pub const SOFT: i32 = 0;

/// `ICUCache.WEAK`: values held through weak references.
pub const WEAK: i32 = 1;

/// The type of [`NULL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Null;

/// `ICUCache.NULL`: a unique object callers store to cache "no value".
pub static NULL: Null = Null;

/// A cache.
///
/// Equivalent to the interface `com.ibm.icu.impl.ICUCache<K, V>`; Java's
/// `null` result of `get` is `None`.
pub trait ICUCache<K, V>: Send + Sync {
    /// `clear()`.
    fn clear(&self);

    /// `put(K key, V value)`.
    fn put(&self, key: K, value: V);

    /// `get(Object key)`.
    fn get(&self, key: &K) -> Option<V>;
}
