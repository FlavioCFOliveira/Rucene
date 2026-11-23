// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CacheBase`.

use rucene::error::Result;

/// Base class for cache implementations: a value per key, created on
/// demand from the key and extra data.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.CacheBase<K, V, D>`.
/// A Java value may be `null`, so the values are `Option<V>`; `create_instance`
/// may throw the exceptions of the subclass that creates them, so both
/// methods return a `Result`.
pub trait CacheBase<K, V, D> {
    /// `getInstance(K key, D data)`: the cached value for `key`, creating it
    /// from `key` and `data` when it is not cached.
    ///
    /// # Errors
    ///
    /// The exception [`create_instance`](Self::create_instance) throws.
    fn get_instance(&self, key: K, data: D) -> Result<Option<V>>;

    /// `createInstance(K key, D data)` (protected): a new value for `key`.
    ///
    /// # Errors
    ///
    /// The exception the subclass throws.
    fn create_instance(&self, key: &K, data: D) -> Result<Option<V>>;
}
