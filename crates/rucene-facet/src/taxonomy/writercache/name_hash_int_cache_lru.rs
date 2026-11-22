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

//! Port of `NameHashIntCacheLRU.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.writercache`.

use std::ops::{Deref, DerefMut};

use crate::taxonomy::writercache::name_int_cache_lru::NameIntCacheLRU;

/// An LRU cache of mapping from name to int. Used to cache Ordinals of
/// category paths. It uses as key, hash of the path instead of the path. This
/// way the cache takes less RAM, but correctness depends on assuming no
/// collisions.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.NameHashIntCacheLRU`,
/// a [`NameIntCacheLRU`] whose `key(...)` overrides return
/// `name.longHashCode()`.
#[derive(Debug)]
pub struct NameHashIntCacheLRU {
    inner: NameIntCacheLRU,
}

impl NameHashIntCacheLRU {
    /// Equivalent to `NameHashIntCacheLRU(int)`.
    pub(crate) fn new(max_cache_size: i32) -> Self {
        Self {
            inner: NameIntCacheLRU::with_keys(max_cache_size, true),
        }
    }

    pub(crate) fn into_inner(self) -> NameIntCacheLRU {
        self.inner
    }
}

impl Deref for NameHashIntCacheLRU {
    type Target = NameIntCacheLRU;

    fn deref(&self) -> &NameIntCacheLRU {
        &self.inner
    }
}

impl DerefMut for NameHashIntCacheLRU {
    fn deref_mut(&mut self) -> &mut NameIntCacheLRU {
        &mut self.inner
    }
}