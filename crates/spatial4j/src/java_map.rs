//! The `java.util.Map<String, String>` the spatial factories read their
//! arguments from.
//!
//! spatial4j's `SpatialContextFactory.makeSpatialContext(Map<String,String>
//! args, ...)` and Lucene's `SpatialPrefixTreeFactory.makeSPT(Map<String,
//! String> args, ...)` take any `Map`, keep it (`this.args = args`) and read
//! it with `args.get(key)` only. Lucene's `SpatialDocMaker` passes a lazy
//! `AbstractMap` view whose `get` reads the benchmark `Config` at call time
//! and whose `entrySet()` throws (`SpatialDocMaker.java:72-85`). [`StringMap`]
//! is that `Map` interface, read side: a plain [`HashMap`] implements it, and
//! so can such a view.

use std::collections::HashMap;
use std::fmt::Debug;

use rucene::error::Result;

/// Equivalent to the read methods of `java.util.Map<String, String>` that
/// the spatial factories may call: `get`, `containsKey` and `entrySet`.
///
/// A map is shared by reference (`Arc<dyn StringMap>`), as Java's factories
/// keep the caller's map object.
pub trait StringMap: Debug + Send + Sync {
    /// `get(Object key)`: the value mapped to `key`, `None` for `null`.
    ///
    /// # Errors
    ///
    /// What the map's `get` throws (a view may read a source that fails).
    fn get(&self, key: &str) -> Result<Option<String>>;

    /// `entrySet()`, iterated: the mappings, in the map's iteration order.
    ///
    /// # Errors
    ///
    /// What the map's `entrySet` throws (for example a view's
    /// `UnsupportedOperationException`).
    fn entry_set(&self) -> Result<Vec<(String, String)>>;

    /// `containsKey(Object key)`. The default is `AbstractMap.containsKey`:
    /// a scan of `entrySet()`.
    ///
    /// # Errors
    ///
    /// What `entry_set` throws.
    fn contains_key(&self, key: &str) -> Result<bool> {
        Ok(self.entry_set()?.iter().any(|(k, _)| k == key))
    }
}

/// Equivalent to `java.util.HashMap<String, String>` as a `Map`.
impl StringMap for HashMap<String, String> {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(HashMap::get(self, key).cloned())
    }

    fn entry_set(&self) -> Result<Vec<(String, String)>> {
        Ok(self.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    fn contains_key(&self, key: &str) -> Result<bool> {
        Ok(HashMap::contains_key(self, key))
    }
}
