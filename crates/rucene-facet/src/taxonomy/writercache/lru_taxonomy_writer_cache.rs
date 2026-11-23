/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `LruTaxonomyWriterCache.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.writercache`.

use std::sync::{Mutex, MutexGuard, PoisonError};

use rucene::error::{LuceneError, Result};

use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::writercache::name_hash_int_cache_lru::NameHashIntCacheLRU;
use crate::taxonomy::writercache::name_int_cache_lru::NameIntCacheLRU;
use crate::taxonomy::writercache::taxonomy_writer_cache::TaxonomyWriterCache;

/// Determines cache type.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.LruTaxonomyWriterCache.LRUType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum LRUType {
    /// Use only the label's 64 bit longHashCode as the hash key.
    LRU_HASHED,
    /// Use the label as the hash key; this is always correct but will usually
    /// use more RAM.
    LRU_STRING,
}

/// `TaxonomyWriterCache` using `LRUType.LRU_STRING` as the default.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.LruTaxonomyWriterCache`.
/// Java's `synchronized` methods lock the cache's monitor; the port locks
/// [`cache`](Self::cache), whose `None` is the cache `close()` nulls.
#[derive(Debug)]
pub struct LruTaxonomyWriterCache {
    cache: Mutex<Option<NameIntCacheLRU>>,
}

impl LruTaxonomyWriterCache {
    /// Creates this with `LRUType.LRU_STRING` method.
    ///
    /// Equivalent to `LruTaxonomyWriterCache(int)`.
    pub fn new(cache_size: i32) -> Self {
        // TODO (Facet): choose between NameHashIntCacheLRU and NameIntCacheLRU.
        // For guaranteed correctness - not relying on no-collisions in the hash
        // function, NameIntCacheLRU should be used:
        // On the other hand, NameHashIntCacheLRU takes less RAM but if there
        // are collisions two different paths would be mapped to the same
        // ordinal...
        Self::with_type(cache_size, LRUType::LRU_STRING)
    }

    /// Creates this with the specified method.
    ///
    /// Equivalent to `LruTaxonomyWriterCache(int, LRUType)`.
    pub fn with_type(cache_size: i32, lru_type: LRUType) -> Self {
        let cache = if lru_type == LRUType::LRU_HASHED {
            NameHashIntCacheLRU::new(cache_size).into_inner()
        } else {
            NameIntCacheLRU::new(cache_size)
        };
        Self {
            cache: Mutex::new(Some(cache)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<NameIntCacheLRU>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn closed() -> LuceneError {
    LuceneError::null_pointer()
}

impl TaxonomyWriterCache for LruTaxonomyWriterCache {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn is_full(&self) -> Result<bool> {
        let guard = self.lock();
        let cache = guard.as_ref().ok_or_else(closed)?;
        Ok(cache.get_size() == cache.get_max_size())
    }

    fn clear(&self) -> Result<()> {
        self.lock().as_mut().ok_or_else(closed)?.clear();
        Ok(())
    }

    fn close(&self) -> Result<()> {
        let mut guard = self.lock();
        guard.as_mut().ok_or_else(closed)?.clear();
        *guard = None;
        Ok(())
    }

    fn size(&self) -> Result<i32> {
        Ok(self.lock().as_ref().ok_or_else(closed)?.get_size())
    }

    fn get(&self, category_path: &FacetLabel) -> Result<i32> {
        let mut guard = self.lock();
        let res = guard.as_mut().ok_or_else(closed)?.get(category_path);
        Ok(res.unwrap_or(-1))
    }

    fn put(&self, category_path: &FacetLabel, ordinal: i32) -> Result<bool> {
        let mut guard = self.lock();
        let cache = guard.as_mut().ok_or_else(closed)?;
        let ret = cache.put(category_path, ordinal);
        // If the cache is full, we need to clear one or more old entries from
        // the cache. However, if we delete from the cache a recent addition
        // that isn't yet in our reader, for this entry to be visible to us we
        // need to make sure that the changes have been committed and we reopen
        // the reader. Because this is a slow operation, we don't delete entries
        // one-by-one but rather in bulk (put() removes the 2/3rd oldest
        // entries).
        if ret {
            cache.make_room_lru();
        }
        Ok(ret)
    }
}
