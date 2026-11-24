// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.SimpleCache`.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use crate::r#impl::icu_cache::{ICUCache, SOFT, WEAK};

const DEFAULT_CAPACITY: i32 = 16;

/// A map held through one soft (or weak) reference, created on the first
/// `put`.
///
/// Equivalent to `com.ibm.icu.impl.SimpleCache<K, V>`; the synchronized
/// `HashMap` is a `Mutex<HashMap>` and the `volatile` reference a lock.
///
/// **Divergence.** Java's `SoftReference`/`WeakReference` may be cleared
/// by the garbage collector, after which the cache starts empty again; the
/// port never clears it (the crate's precedent for soft caches, allowed by
/// the `SoftReference` contract; `clear()` still drops it).
#[derive(Debug)]
#[allow(clippy::type_complexity)] // type mirrors the Java signature
pub struct SimpleCache<K, V> {
    cache_ref: Mutex<Option<Arc<Mutex<HashMap<K, V>>>>>,
    cache_type: i32,
    capacity: i32,
}

impl<K, V> Default for SimpleCache<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> SimpleCache<K, V> {
    /// `new SimpleCache()`.
    pub fn new() -> Self {
        Self {
            cache_ref: Mutex::new(None),
            cache_type: SOFT,
            capacity: DEFAULT_CAPACITY,
        }
    }

    /// `new SimpleCache(int cacheType)`.
    pub fn with_type(cache_type: i32) -> Self {
        Self::with_type_and_capacity(cache_type, DEFAULT_CAPACITY)
    }

    /// `new SimpleCache(int cacheType, int initialCapacity)`.
    pub fn with_type_and_capacity(cache_type: i32, initial_capacity: i32) -> Self {
        let mut c = Self::new();
        if cache_type == WEAK {
            c.cache_type = cache_type;
        }
        if initial_capacity > 0 {
            c.capacity = initial_capacity;
        }
        c
    }

    /// The reference kind (`SOFT` or `WEAK`).
    pub fn cache_type(&self) -> i32 {
        self.cache_type
    }
}

impl<K: Eq + Hash + Send, V: Clone + Send> ICUCache<K, V> for SimpleCache<K, V> {
    fn get(&self, key: &K) -> Option<V> {
        let r = self
            .cache_ref
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let map = r?;
        let guard = map.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key).cloned()
    }

    fn put(&self, key: K, value: V) {
        let map = {
            let mut r = self.cache_ref.lock().unwrap_or_else(|e| e.into_inner());
            match r.as_ref() {
                Some(m) => m.clone(),
                None => {
                    let m = Arc::new(Mutex::new(HashMap::with_capacity(self.capacity as usize)));
                    *r = Some(m.clone());
                    m
                }
            }
        };
        map.lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, value);
    }

    fn clear(&self) {
        *self.cache_ref.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}
