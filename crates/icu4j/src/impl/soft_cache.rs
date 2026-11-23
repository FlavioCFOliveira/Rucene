// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.SoftCache`.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::cache_value::CacheValue;

/// The function a subclass of `SoftCache` overrides `createInstance` with.
type Creator<K, V, D> = Box<dyn Fn(&K, D) -> Result<Option<V>> + Send + Sync>;

/// Generic, thread-safe cache implementation, usually storing cached
/// instances behind soft references.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.SoftCache<K, V, D>`.
/// Java's anonymous subclasses override `createInstance`; here the
/// subclass's method is the closure given to [`new`](Self::new). Java's
/// `ConcurrentHashMap` is a map under a mutex, held only for each `get` and
/// `putIfAbsent`, so `createInstance` runs outside it, as in Java.
pub struct SoftCache<K, V, D> {
    map: Mutex<HashMap<K, MapValue<V>>>,
    create: Creator<K, V, D>,
}

/// A map value: the object itself when values are strong, else a shared
/// [`CacheValue`] wrapping it (Java stores either in one
/// `ConcurrentHashMap<K, Object>` and tests `instanceof CacheValue`).
#[derive(Clone)]
enum MapValue<V> {
    Plain(V),
    Wrapped(Arc<CacheValue<V>>),
}

impl<K, V, D> SoftCache<K, V, D>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// A cache whose `createInstance` is `create`.
    pub fn new(create: impl Fn(&K, D) -> Result<Option<V>> + Send + Sync + 'static) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            create: Box::new(create),
        }
    }

    fn map_get(&self, key: &K) -> Option<MapValue<V>> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// `ConcurrentHashMap.putIfAbsent`: the previous value, or `None` after
    /// inserting `value`.
    fn map_put_if_absent(&self, key: K, value: MapValue<V>) -> Option<MapValue<V>> {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(&key) {
            return Some(existing.clone());
        }
        map.insert(key, value);
        None
    }
}

impl<K, V, D> CacheBase<K, V, D> for SoftCache<K, V, D>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// `getInstance(K key, D data)` (final).
    fn get_instance(&self, key: K, data: D) -> Result<Option<V>> {
        // We do not cache all values with soft references, see the comments
        // of the Java method: a strong value is stored as is.
        let map_value = self.map_get(&key);
        if let Some(map_value) = map_value {
            let cv = match map_value {
                // The value was stored directly.
                MapValue::Plain(v) => return Ok(Some(v)),
                MapValue::Wrapped(cv) => cv,
            };
            if cv.is_null() {
                return Ok(None);
            }
            if let Some(value) = cv.get() {
                return Ok(Some(value));
            }
            // The instance has been evicted, its Reference cleared.
            // Create and set a new instance.
            let value = self.create_instance(&key, data)?;
            cv.reset_if_cleared(value)
        } else {
            // We had never cached an instance for this key.
            let value = self.create_instance(&key, data)?;
            let map_value: MapValue<V> = match &value {
                Some(v) if CacheValue::<V>::future_instances_will_be_strong() => {
                    MapValue::Plain(v.clone())
                }
                _ => MapValue::Wrapped(Arc::new(CacheValue::get_instance(value.clone()))),
            };
            let previous = self.map_put_if_absent(key, map_value);
            match previous {
                // Normal "put": Our new value is now cached.
                None => Ok(value),
                // Race condition: Another thread beat us to putting a CacheValue
                // into the map. Return its value, but just in case the garbage collector
                // was aggressive, we also offer our new instance for caching.
                Some(MapValue::Plain(v)) => Ok(Some(v)),
                Some(MapValue::Wrapped(cv)) => cv.reset_if_cleared(value),
            }
        }
    }

    fn create_instance(&self, key: &K, data: D) -> Result<Option<V>> {
        (self.create)(key, data)
    }
}
