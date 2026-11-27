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

//! The types declared in `ShapeFieldCacheProvider.java`, of the Java
//! package `org.apache.lucene.spatial.util`.

use std::sync::{Arc, Mutex, Weak};

use rucene::error::Result;
use rucene::index::postings_enum::POSTINGS_ENUM_NONE;
use rucene::index::terms::get_terms;
use rucene::index::LeafReader;
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::util::BytesRef;

use crate::util::shape_field_cache::ShapeFieldCache;

/// A cached reader (by identity, weakly held) and its cache.
type CacheEntry<T> = (Weak<dyn LeafReader>, Arc<ShapeFieldCache<T>>);

/// The fields of `ShapeFieldCacheProvider`.
///
/// Java keys the caches by reader in a `WeakHashMap`; here a cache is kept
/// with a weak reference to its reader, compared by identity, and dropped
/// once the reader is gone, which is what the `WeakHashMap` does when the
/// reader is collected.
#[derive(Debug)]
pub struct ShapeFieldCacheProviderBase<T: ?Sized> {
    // it may be a List<T> or T
    sidx: Mutex<Vec<CacheEntry<T>>>,
    /// `defaultSize`.
    pub default_size: i32,
    /// `shapeField`.
    pub shape_field: String,
}

impl<T: ?Sized> ShapeFieldCacheProviderBase<T> {
    /// `ShapeFieldCacheProvider(String shapeField, int defaultSize)`.
    pub fn new(shape_field: &str, default_size: i32) -> Self {
        Self {
            sidx: Mutex::new(Vec::new()),
            default_size,
            shape_field: shape_field.to_string(),
        }
    }
}

/// Equivalent to `org.apache.lucene.spatial.util.ShapeFieldCacheProvider`
/// (`public abstract class ShapeFieldCacheProvider<T extends Shape>`):
/// provides access to a [`ShapeFieldCache`] for a given `LeafReader`. If a
/// Cache does not exist for the Reader, then it is built by iterating over
/// the all terms for a given field, reconstructing the Shape from them, and
/// adding the Shape to the Cache.
pub trait ShapeFieldCacheProvider<T: ?Sized>: Send + Sync {
    /// The `ShapeFieldCacheProvider` fields.
    fn provider_base(&self) -> &ShapeFieldCacheProviderBase<T>;

    /// `readShape(BytesRef term)`: `None` (Java `null`) when the term holds
    /// no shape.
    fn read_shape(&self, term: &BytesRef) -> Option<Arc<T>>;

    /// `toString()`: `Object.toString()` (the class name and its identity
    /// hash, the core's stand-in).
    fn provider_to_string(&self) -> String;

    /// `getCache(LeafReader reader)` (`synchronized`).
    ///
    /// # Errors
    ///
    /// The I/O errors of reading the terms and postings.
    fn get_cache(&self, reader: &Arc<dyn LeafReader>) -> Result<Arc<ShapeFieldCache<T>>> {
        let base = self.provider_base();
        let mut sidx = base.sidx.lock().unwrap_or_else(|e| e.into_inner());
        sidx.retain(|(r, _)| r.strong_count() > 0);
        let key = Arc::as_ptr(reader) as *const ();
        if let Some((_, idx)) = sidx.iter().find(|(r, _)| r.as_ptr() as *const () == key) {
            return Ok(Arc::clone(idx));
        }
        let mut idx = ShapeFieldCache::new(reader.max_doc(), base.default_size);
        let mut docs = None;
        let terms = get_terms(&**reader, &base.shape_field)?;
        let mut te = terms.iterator()?;
        let mut term = te.next()?.cloned();
        while let Some(t) = term {
            if let Some(shape) = self.read_shape(&t) {
                let mut d = te
                    .postings(docs.take(), POSTINGS_ENUM_NONE)?
                    .expect("postings of a positioned TermsEnum");
                let mut docid = d.next_doc()?;
                while docid != NO_MORE_DOCS {
                    idx.add(docid, Arc::clone(&shape));
                    docid = d.next_doc()?;
                }
                docs = Some(d);
            }
            term = te.next()?.cloned();
        }
        let idx = Arc::new(idx);
        sidx.push((Arc::downgrade(reader), Arc::clone(&idx)));
        Ok(idx)
    }
}
