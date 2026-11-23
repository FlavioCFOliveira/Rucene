// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.LocaleObjectCache`.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

/// The functions a subclass of `LocaleObjectCache` overrides.
type CreateFn<K, V> = Box<dyn Fn(&K) -> Option<V> + Send + Sync>;
/// `normalizeKey`.
type NormalizeFn<K> = Box<dyn Fn(K) -> Option<K> + Send + Sync>;

/// A concurrent cache of objects created from keys, which may be
/// normalized before the object is created.
///
/// Equivalent to the abstract class
/// `com.ibm.icu.impl.locale.LocaleObjectCache<K, V>`: Java's subclasses
/// override `createObject` and `normalizeKey`, which are the closures given
/// to [`new`](Self::new). Java holds the values through `SoftReference`s
/// the collector may clear (removing the entries through a
/// `ReferenceQueue`); a Rust process has no collector, so the values stay
/// cached, one of the behaviours the `SoftReference` contract allows (as
/// [`CacheValue`](crate::r#impl::cache_value::CacheValue) does).
pub struct LocaleObjectCache<K, V> {
    map: Mutex<HashMap<K, V>>,
    create: CreateFn<K, V>,
    normalize: NormalizeFn<K>,
}

impl<K, V> LocaleObjectCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// A cache whose `createObject` is `create` and whose `normalizeKey`
    /// is `normalize` (Java's default returns the key unchanged).
    pub fn new(
        create: impl Fn(&K) -> Option<V> + Send + Sync + 'static,
        normalize: impl Fn(K) -> Option<K> + Send + Sync + 'static,
    ) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            create: Box::new(create),
            normalize: Box::new(normalize),
        }
    }

    fn map_get(&self, key: &K) -> Option<V> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// `get(K key)`.
    pub fn get(&self, key: K) -> Option<V> {
        let value = self.map_get(&key);
        if value.is_none() {
            let key = (self.normalize)(key)?;
            if let Some(value) = self.map_get(&key) {
                return Some(value);
            }
            let new_val = (self.create)(&key)?;
            self.map
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key, new_val.clone());
            return Some(new_val);
        }
        value
    }
}
