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

//! Port of `TaxonomyWriterCache.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.writercache`.

use rucene::error::Result;

use crate::taxonomy::facet_label::FacetLabel;

/// `TaxonomyWriterCache` is a relatively simple interface for a cache of
/// category-&gt;ordinal mappings, used in `TaxonomyWriter` implementations.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.TaxonomyWriterCache`.
/// Methods take `&self`: Java's implementations synchronize internally. The
/// results are `Result`s because Java's implementation dereferences a cache
/// that `close()` has nulled.
pub trait TaxonomyWriterCache: Send + Sync {
    /// The cache as `Any`, for Java's `instanceof` checks on a cache
    /// (`getCache() instanceof LruTaxonomyWriterCache`); `None` for a cache
    /// that does not expose its concrete type.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Let go of whatever resources the cache is holding.
    ///
    /// Equivalent to `close()`.
    fn close(&self) -> Result<()>;

    /// Lookup a category in the cache, returning its ordinal, or a negative
    /// number if the category is not in the cache.
    ///
    /// Equivalent to `get(FacetLabel)`.
    fn get(&self, category_path: &FacetLabel) -> Result<i32>;

    /// Add a category to the cache, with the given ordinal as the value.
    /// Returns `true` if the cache needed to evict entries.
    ///
    /// Equivalent to `put(FacetLabel, int)`.
    fn put(&self, category_path: &FacetLabel, ordinal: i32) -> Result<bool>;

    /// Returns true if the cache is full.
    ///
    /// Equivalent to `isFull()`.
    fn is_full(&self) -> Result<bool>;

    /// Clears the content of the cache.
    ///
    /// Equivalent to `clear()`.
    fn clear(&self) -> Result<()>;

    /// How many labels are currently stored in the cache.
    ///
    /// Equivalent to `size()`.
    fn size(&self) -> Result<i32>;
}
