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

//! Port of `DirectoryTaxonomyReader.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.directory`.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::{LuceneError, Result};
use rucene::index::{
    directory_reader, sub_index_from_leaves, DirectoryReader, IndexReader, LeafReaderContext,
    MultiTerms,
};
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::{Accountable, Accountables, BytesRef, RamUsageEstimator};

use crate::facets_config::FacetsConfig;
use crate::taxonomy::directory::consts::Consts;
use crate::taxonomy::directory::directory_taxonomy_writer::{DirectoryTaxonomyWriter, INDEX_EPOCH};
use crate::taxonomy::directory::taxonomy_index_arrays::TaxonomyIndexArrays;
use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::lru_hash_map::LRUHashMap;
use crate::taxonomy::parallel_taxonomy_arrays::ParallelTaxonomyArrays;
use crate::taxonomy::taxonomy_reader::{
    TaxonomyReader, TaxonomyReaderBase, INVALID_ORDINAL, ROOT_ORDINAL,
};

const DEFAULT_CACHE_VALUE: usize = 4000;

// NOTE: very coarse estimate!
const BYTES_PER_CACHE_ENTRY: i64 = 4 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
    + 4 * RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
    + 8 * 2;

/// `LRUHashMap<FacetLabel, Integer>`, shared between a reader and the
/// readers `doOpenIfChanged` derives from it.
pub type OrdinalCache = Arc<Mutex<LRUHashMap<FacetLabel, i32>>>;
/// `LRUHashMap<Integer, FacetLabel>`, shared likewise.
pub type CategoryCache = Arc<Mutex<LRUHashMap<i32, FacetLabel>>>;

fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The protected method of `DirectoryTaxonomyReader` a subclass overrides:
/// `doOpenIfChanged`.
///
/// Rust has no implementation inheritance; a subclass's overrides are this
/// strategy object, and its default is the base class's body
/// ([`DirectoryTaxonomyReader::default_do_open_if_changed`]).
pub trait DirectoryTaxonomyReaderOverrides: Send + Sync {
    /// Implements the opening of a new `DirectoryTaxonomyReader` instance if
    /// the taxonomy has changed; `reader` is `this`.
    ///
    /// Equivalent to the protected `doOpenIfChanged()`, typed as Java's
    /// covariant override.
    fn do_open_if_changed(
        &self,
        reader: &DirectoryTaxonomyReader,
    ) -> Result<Option<DirectoryTaxonomyReader>> {
        reader.default_do_open_if_changed()
    }
}

/// The base class's own overrides.
struct DefaultOverrides;

impl DirectoryTaxonomyReaderOverrides for DefaultOverrides {}

fn default_overrides() -> Arc<dyn DirectoryTaxonomyReaderOverrides> {
    Arc::new(DefaultOverrides)
}

/// The fields `doClose()` nulls.
struct ReaderCaches {
    ordinal_cache: Option<OrdinalCache>,
    category_cache: Option<CategoryCache>,
    taxo_arrays: Option<Arc<TaxonomyIndexArrays>>,
}

/// A `TaxonomyReader` which retrieves stored taxonomy information from a
/// `Directory`.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.directory.DirectoryTaxonomyReader`.
pub struct DirectoryTaxonomyReader {
    base: TaxonomyReaderBase,
    taxo_writer: Option<Arc<DirectoryTaxonomyWriter>>,
    /// Used in `doOpenIfChanged`.
    taxo_epoch: i64,
    index_reader: Arc<dyn DirectoryReader>,
    caches: Mutex<ReaderCaches>,
    overrides: Arc<dyn DirectoryTaxonomyReaderOverrides>,
}

impl fmt::Debug for DirectoryTaxonomyReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectoryTaxonomyReader")
            .field("taxo_epoch", &self.taxo_epoch)
            .finish_non_exhaustive()
    }
}

impl DirectoryTaxonomyReader {
    /// Called only from `doOpenIfChanged`. If the taxonomy has been recreated,
    /// you should pass `None` as the caches and parent/children arrays.
    ///
    /// Equivalent to the package-private five-argument constructor
    /// `DirectoryTaxonomyReader(DirectoryReader, DirectoryTaxonomyWriter,
    /// LRUHashMap, LRUHashMap, TaxonomyIndexArrays)`; each `None` is Java's
    /// `null`.
    pub fn from_parts(
        index_reader: Arc<dyn DirectoryReader>,
        taxo_writer: Option<Arc<DirectoryTaxonomyWriter>>,
        ordinal_cache: Option<OrdinalCache>,
        category_cache: Option<CategoryCache>,
        taxo_arrays: Option<Arc<TaxonomyIndexArrays>>,
    ) -> Result<Self> {
        Self::from_parts_with_overrides(
            index_reader,
            taxo_writer,
            ordinal_cache,
            category_cache,
            taxo_arrays,
            default_overrides(),
        )
    }

    /// [`Self::from_parts`] as the `super(...)` call of a subclass that
    /// overrides the protected method listed by
    /// [`DirectoryTaxonomyReaderOverrides`].
    pub fn from_parts_with_overrides(
        index_reader: Arc<dyn DirectoryReader>,
        taxo_writer: Option<Arc<DirectoryTaxonomyWriter>>,
        ordinal_cache: Option<OrdinalCache>,
        category_cache: Option<CategoryCache>,
        taxo_arrays: Option<Arc<TaxonomyIndexArrays>>,
        overrides: Arc<dyn DirectoryTaxonomyReaderOverrides>,
    ) -> Result<Self> {
        let taxo_epoch = taxo_writer.as_ref().map_or(-1, |w| w.get_taxonomy_epoch());

        // use the same instance of the cache, note the protective code in
        // getOrdinal and getPath
        let ordinal_cache = ordinal_cache
            .unwrap_or_else(|| Arc::new(Mutex::new(LRUHashMap::new(DEFAULT_CACHE_VALUE))));
        let category_cache = category_cache
            .unwrap_or_else(|| Arc::new(Mutex::new(LRUHashMap::new(DEFAULT_CACHE_VALUE))));

        let taxo_arrays = match taxo_arrays {
            Some(copy_from) => Some(Arc::new(TaxonomyIndexArrays::with_copy_from(
                &(Arc::clone(&index_reader) as Arc<dyn IndexReader>),
                &copy_from,
            )?)),
            None => None,
        };
        Ok(Self {
            base: TaxonomyReaderBase::default(),
            taxo_writer,
            taxo_epoch,
            index_reader,
            caches: Mutex::new(ReaderCaches {
                ordinal_cache: Some(ordinal_cache),
                category_cache: Some(category_cache),
                taxo_arrays,
            }),
            overrides,
        })
    }

    /// Open for reading a taxonomy stored in a given `Directory`.
    ///
    /// Equivalent to `DirectoryTaxonomyReader(Directory)`.
    pub fn open(directory: Arc<dyn Directory>) -> Result<Self> {
        Self::open_with_overrides(directory, default_overrides())
    }

    /// [`Self::open`] as the `super(directory)` call of a subclass that
    /// overrides the protected method listed by
    /// [`DirectoryTaxonomyReaderOverrides`].
    pub fn open_with_overrides(
        directory: Arc<dyn Directory>,
        overrides: Arc<dyn DirectoryTaxonomyReaderOverrides>,
    ) -> Result<Self> {
        let index_reader = Self::open_index_reader(directory)?;
        Ok(Self {
            base: TaxonomyReaderBase::default(),
            taxo_writer: None,
            taxo_epoch: -1,
            index_reader,
            // These are the default cache sizes; they can be configured after
            // construction with the cache's setMaxSize() method
            caches: Mutex::new(ReaderCaches {
                ordinal_cache: Some(Arc::new(Mutex::new(LRUHashMap::new(DEFAULT_CACHE_VALUE)))),
                category_cache: Some(Arc::new(Mutex::new(LRUHashMap::new(DEFAULT_CACHE_VALUE)))),
                taxo_arrays: None,
            }),
            overrides,
        })
    }

    /// Opens a `DirectoryTaxonomyReader` over the given
    /// `DirectoryTaxonomyWriter` (for NRT).
    ///
    /// Equivalent to `DirectoryTaxonomyReader(DirectoryTaxonomyWriter)`.
    pub fn from_writer(taxo_writer: Arc<DirectoryTaxonomyWriter>) -> Result<Self> {
        let taxo_epoch = taxo_writer.get_taxonomy_epoch();
        let index_reader = Self::open_index_reader_from_writer(&taxo_writer)?;
        Ok(Self {
            base: TaxonomyReaderBase::default(),
            taxo_writer: Some(taxo_writer),
            taxo_epoch,
            index_reader,
            // These are the default cache sizes; they can be configured after
            // construction with the cache's setMaxSize() method
            caches: Mutex::new(ReaderCaches {
                ordinal_cache: Some(Arc::new(Mutex::new(LRUHashMap::new(DEFAULT_CACHE_VALUE)))),
                category_cache: Some(Arc::new(Mutex::new(LRUHashMap::new(DEFAULT_CACHE_VALUE)))),
                taxo_arrays: None,
            }),
            overrides: default_overrides(),
        })
    }

    /// Open the `DirectoryReader` from this `Directory`.
    ///
    /// Equivalent to `openIndexReader(Directory)`.
    pub fn open_index_reader(directory: Arc<dyn Directory>) -> Result<Arc<dyn DirectoryReader>> {
        directory_reader::open(directory)
    }

    /// Open the `DirectoryReader` from this `IndexWriter`.
    ///
    /// Equivalent to `openIndexReader(IndexWriter)`.
    pub fn open_index_reader_from_writer(
        writer: &DirectoryTaxonomyWriter,
    ) -> Result<Arc<dyn DirectoryReader>> {
        directory_reader::open_with_writer(&**writer.get_internal_index_writer())
    }

    /// Implements the opening of a new `DirectoryTaxonomyReader` instance if
    /// the taxonomy has changed.
    ///
    /// Equivalent to `doOpenIfChanged()`, typed as Java's covariant override:
    /// the virtual call, dispatched to [`DirectoryTaxonomyReaderOverrides`].
    pub fn open_if_changed(&self) -> Result<Option<DirectoryTaxonomyReader>> {
        Arc::clone(&self.overrides).do_open_if_changed(self)
    }

    /// The body of `DirectoryTaxonomyReader.doOpenIfChanged()`, which a
    /// subclass's override reaches as `super.doOpenIfChanged()`.
    pub fn default_do_open_if_changed(&self) -> Result<Option<DirectoryTaxonomyReader>> {
        self.ensure_open()?;

        // This works for both NRT and non-NRT readers (i.e. an NRT reader
        // remains NRT).
        let Some(r2) = directory_reader::open_if_changed(Arc::clone(&self.index_reader))? else {
            return Ok(None); // no changes, nothing to do
        };

        let result = (|| -> Result<DirectoryTaxonomyReader> {
            let mut recreated = false;
            if self.taxo_writer.is_none() {
                // not NRT, check epoch from commit data
                let t1 = self
                    .index_reader
                    .index_commit()?
                    .get_user_data()?
                    .get(INDEX_EPOCH)
                    .cloned();
                let t2 = r2
                    .index_commit()?
                    .get_user_data()?
                    .get(INDEX_EPOCH)
                    .cloned();
                match t1 {
                    None => {
                        if t2.is_some() {
                            recreated = true;
                        }
                    }
                    Some(t1) => {
                        if Some(&t1) != t2.as_ref() {
                            // t1 != null and t2 must not be null b/c
                            // DirTaxoWriter always puts the commit data.
                            // it's ok to use String.equals because we require
                            // the two epoch values to be the same.
                            recreated = true;
                        }
                    }
                }
            } else {
                // NRT, compare current taxoWriter.epoch() vs the one that was
                // given at construction
                let taxo_writer = self.taxo_writer.as_ref();
                if taxo_writer.is_some_and(|w| self.taxo_epoch != w.get_taxonomy_epoch()) {
                    recreated = true;
                }
            }

            if recreated {
                // if recreated, do not reuse anything from this instace. the
                // information will be lazily computed by the new instance
                // when needed.
                DirectoryTaxonomyReader::from_parts(
                    Arc::clone(&r2),
                    self.taxo_writer.clone(),
                    None,
                    None,
                    None,
                )
            } else {
                let (ordinal_cache, category_cache, taxo_arrays) = {
                    let caches = lock(&self.caches);
                    (
                        caches.ordinal_cache.clone(),
                        caches.category_cache.clone(),
                        caches.taxo_arrays.clone(),
                    )
                };
                DirectoryTaxonomyReader::from_parts(
                    Arc::clone(&r2),
                    self.taxo_writer.clone(),
                    ordinal_cache,
                    category_cache,
                    taxo_arrays,
                )
            }
        })();
        match result {
            Ok(reader) => Ok(Some(reader)),
            Err(e) => {
                // `IOUtils.closeWhileHandlingException(r2)`.
                let _ = r2.close();
                Err(e)
            }
        }
    }

    /// Expert: returns the underlying `DirectoryReader` instance that is used
    /// by this `TaxonomyReader`.
    ///
    /// Equivalent to `getInternalIndexReader()`.
    pub fn get_internal_index_reader(&self) -> Result<Arc<dyn DirectoryReader>> {
        self.ensure_open()?;
        Ok(Arc::clone(&self.index_reader))
    }

    fn index_reader(&self) -> Arc<dyn IndexReader> {
        Arc::clone(&self.index_reader) as Arc<dyn IndexReader>
    }

    fn ordinal_cache(&self) -> Result<OrdinalCache> {
        lock(&self.caches)
            .ordinal_cache
            .clone()
            .ok_or_else(LuceneError::null_pointer)
    }

    fn category_cache(&self) -> Result<CategoryCache> {
        lock(&self.caches)
            .category_cache
            .clone()
            .ok_or_else(LuceneError::null_pointer)
    }

    /// Equivalent to `getPathFromCache(int...)`.
    fn get_path_from_cache(&self, ordinals: &[i32]) -> Result<Vec<Option<FacetLabel>>> {
        let cache = self.category_cache()?;
        let mut cache = lock(&cache);
        Ok(ordinals.iter().map(|o| cache.get(o).cloned()).collect())
    }

    /// Checks if the ordinals in the array are >=0 and &lt; `indexReader.maxDoc()`.
    ///
    /// Equivalent to `checkOrdinalBounds(int...)`.
    fn check_ordinal_bounds(&self, ordinals: &[i32]) -> Result<()> {
        for &ordinal in ordinals {
            if ordinal < 0 || ordinal >= self.index_reader.max_doc() {
                return Err(LuceneError::illegal_argument(format!(
                    "ordinal {} is out of the range of the indexReader {}. The maximum possible ordinal number is {}",
                    ordinal,
                    self.index_reader,
                    self.index_reader.max_doc() - 1
                )));
            }
        }
        Ok(())
    }

    /// Setting the cache size affects both the categoryCache and the
    /// ordinalCache.
    ///
    /// Equivalent to `setCacheSize(int)`.
    pub fn set_cache_size(&self, size: usize) -> Result<()> {
        self.ensure_open()?;
        lock(&*self.category_cache()?).set_max_size(size);
        lock(&*self.ordinal_cache()?).set_max_size(size);
        Ok(())
    }

    /// Returns ordinal -&gt; label mapping, up to the provided max ordinal or
    /// number of ordinals, whichever is smaller.
    ///
    /// Equivalent to `toString(int)`.
    pub fn to_string_max(&self, max: i32) -> Result<String> {
        self.ensure_open()?;
        let mut sb = String::new();
        let limit = max.min(self.index_reader.max_doc());
        for i in 0..limit {
            match self.get_path(i) {
                Ok(None) => {
                    sb.push_str(&format!("{i}: NULL!! \n"));
                    continue;
                }
                Ok(Some(category)) => {
                    if category.length == 0 {
                        sb.push_str(&format!("{i}: EMPTY STRING!! \n"));
                        continue;
                    }
                    sb.push_str(&format!("{i}: {category}\n"));
                }
                Err(e) if e.is_io_exception() => {
                    sb.push_str(&format!("{i}: EXCEPTION!! \n"));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(sb)
    }

    fn leaves(&self) -> Result<Vec<Arc<LeafReaderContext>>> {
        self.index_reader().leaves()
    }
}

impl TaxonomyReader for DirectoryTaxonomyReader {
    fn taxonomy_reader_base(&self) -> &TaxonomyReaderBase {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn do_close(&self) -> Result<()> {
        self.index_reader.close()?;
        let mut caches = lock(&self.caches);
        caches.taxo_arrays = None;
        // do not clear() the caches, as they may be used by other DTR
        // instances.
        caches.ordinal_cache = None;
        caches.category_cache = None;
        Ok(())
    }

    fn do_open_if_changed(&self) -> Result<Option<Arc<dyn TaxonomyReader>>> {
        Ok(self
            .open_if_changed()?
            .map(|r| Arc::new(r) as Arc<dyn TaxonomyReader>))
    }

    fn get_parallel_taxonomy_arrays(&self) -> Result<Arc<dyn ParallelTaxonomyArrays>> {
        self.ensure_open()?;
        let mut caches = lock(&self.caches);
        if let Some(arrays) = &caches.taxo_arrays {
            return Ok(Arc::clone(arrays) as Arc<dyn ParallelTaxonomyArrays>);
        }
        // according to Java Concurrency in Practice, this might perform
        // better on some JVMs, because the array initialization doesn't
        // happen on the volatile member.
        let arrays = Arc::new(TaxonomyIndexArrays::new(&self.index_reader())?);
        caches.taxo_arrays = Some(Arc::clone(&arrays));
        Ok(arrays as Arc<dyn ParallelTaxonomyArrays>)
    }

    fn get_commit_user_data(&self) -> Result<HashMap<String, String>> {
        self.ensure_open()?;
        self.index_reader.index_commit()?.get_user_data()
    }

    fn get_ordinal(&self, cp: &FacetLabel) -> Result<i32> {
        self.ensure_open()?;
        if cp.length == 0 {
            return Ok(ROOT_ORDINAL);
        }

        // First try to find the answer in the LRU cache:
        let ordinal_cache = self.ordinal_cache()?;
        let res = lock(&ordinal_cache).get(cp).copied();
        if let Some(res) = res {
            if res < self.index_reader.max_doc() {
                // Since the cache is shared with DTR instances allocated from
                // doOpenIfChanged, we need to ensure that the ordinal is one
                // that this DTR instance recognizes.
                return Ok(res);
            } else {
                // if we get here, it means that the category was found in the
                // cache, but is not recognized by this TR instance. Therefore
                // there's no need to continue search for the path on disk,
                // because we won't find it there too.
                return Ok(INVALID_ORDINAL);
            }
        }

        // If we're still here, we have a cache miss. We need to fetch the
        // value from disk, and then also put it in the cache:
        let mut ret = INVALID_ORDINAL;
        let docs = MultiTerms::get_term_postings_enum(
            &self.index_reader(),
            Consts::FULL,
            &BytesRef::from_char_sequence(&FacetsConfig::path_to_string_len(
                &cp.components,
                cp.length,
            )?),
            0,
        )?;
        if let Some(mut docs) = docs {
            if docs.next_doc()? != NO_MORE_DOCS {
                ret = docs.doc_id();

                // we only store the fact that a category exists, not its
                // inexistence. This is required because the caches are shared
                // with new DTR instances that are allocated from
                // doOpenIfChanged. Therefore, if we only store information
                // about found categories, we cannot accidently tell a new
                // generation of DTR that a category does not exist.
                lock(&ordinal_cache).put(cp.clone(), ret);
            }
        }

        Ok(ret)
    }

    fn get_bulk_ordinals(&self, category_paths: &[FacetLabel]) -> Result<Vec<i32>> {
        self.ensure_open()?;
        if category_paths.is_empty() {
            return Ok(Vec::new());
        }
        if category_paths.len() == 1 {
            return Ok(vec![self.get_ordinal(&category_paths[0])?]);
        }
        // First try to find results in the cache:
        let mut result = vec![0; category_paths.len()];
        // this initial size is arbitrary.
        let mut indexes_missing_from_cache: Vec<usize> = Vec::new();
        let ordinal_cache = self.ordinal_cache()?;
        for (i, cp) in category_paths.iter().enumerate() {
            let res = lock(&ordinal_cache).get(cp).copied();
            if let Some(res) = res {
                if res < self.index_reader.max_doc() {
                    // Since the cache is shared with DTR instances allocated
                    // from doOpenIfChanged, we need to ensure that the ordinal
                    // is one that this DTR instance recognizes.
                    result[i] = res;
                } else {
                    // if we get here, it means that the category was found in
                    // the cache, but is not recognized by this TR instance.
                    // Therefore, there's no need to continue search for the
                    // path on disk, because we won't find it there too.
                    result[i] = INVALID_ORDINAL;
                }
            } else {
                indexes_missing_from_cache.push(i);
            }
        }
        // all ordinals found in cache: Java tests
        // `indexesMissingFromCache.length == 0`, the length of an array
        // allocated with `Math.min(10, categoryPaths.length)` (at least 2
        // here) slots, so the early return never happens and the terms are
        // always resolved.

        // sort the terms to get sequential access to the terms
        let mut missing: Vec<(usize, BytesRef)> =
            Vec::with_capacity(indexes_missing_from_cache.len());
        for &i in &indexes_missing_from_cache {
            let cp = &category_paths[i];
            missing.push((
                i,
                BytesRef::from_char_sequence(&FacetsConfig::path_to_string_len(
                    &cp.components,
                    cp.length,
                )?),
            ));
        }
        // `new StringSorter(BytesRefComparator.NATURAL).sort(...)`: the order
        // of equal terms only affects equal cache keys.
        missing.sort_by(|a, b| a.1.slice().cmp(b.1.slice()));

        let terms = MultiTerms::get_terms(&self.index_reader(), Consts::FULL)?
            .ok_or_else(LuceneError::null_pointer)?;
        let mut te = terms.iterator()?;
        for (res_index, term) in &missing {
            // If the term exists, we store the ordinal and seek to the next
            // term
            if te.seek_exact(term)? {
                let postings = te.postings(None, 0)?;
                result[*res_index] = INVALID_ORDINAL;
                if let Some(mut postings) = postings {
                    if postings.next_doc()? != NO_MORE_DOCS {
                        result[*res_index] = postings.doc_id();
                    }
                }
            } else {
                result[*res_index] = INVALID_ORDINAL;
            }
        }
        // populate cache
        {
            let mut cache = lock(&ordinal_cache);
            for (res_index, _) in &missing {
                let ord = result[*res_index];
                if ord != INVALID_ORDINAL {
                    cache.put(category_paths[*res_index].clone(), ord);
                }
            }
        }
        Ok(result)
    }

    fn get_path(&self, ordinal: i32) -> Result<Option<FacetLabel>> {
        self.ensure_open()?;

        // Since the cache is shared with DTR instances allocated from
        // doOpenIfChanged, we need to ensure that the ordinal is one that this
        // DTR instance recognizes. Therefore we do this check first, before we
        // check the cache.
        self.check_ordinal_bounds(&[ordinal])?;

        let ordinal_path = self.get_path_from_cache(&[ordinal])?;
        if let Some(label) = &ordinal_path[0] {
            return Ok(Some(label.clone()));
        }

        let leaves = self.leaves()?;
        let reader_index = sub_index_from_leaves(ordinal, &leaves) as usize;
        let leaf_reader = leaves[reader_index].leaf_reader();
        // TODO: Use LUCENE-9476 to get the bulk lookup API for extracting
        // BinaryDocValues
        let values = leaf_reader.get_binary_doc_values(Consts::FULL)?;
        let ret = match values {
            Some(mut values) => {
                if !values.advance_exact(ordinal - leaves[reader_index].doc_base())? {
                    return Err(LuceneError::illegal_state(""));
                }
                FacetLabel::new(FacetsConfig::string_to_path(
                    &values.binary_value()?.utf8_to_string()?,
                ))?
            }
            None => return Err(LuceneError::illegal_state("")),
        };

        lock(&*self.category_cache()?).put(ordinal, ret.clone());

        Ok(Some(ret))
    }

    /// Returns an array of FacetLabels for a given array of ordinals.
    ///
    /// This API is generally faster than iteratively calling `getPath` over
    /// an array of ordinals. It uses the `getPath` method iteratively when it
    /// detects that the index was created using `StoredFields` (with no
    /// performance gains) and uses DocValues based iteration when the index
    /// is based on BinaryDocValues. Lucene switched to BinaryDocValues in
    /// version 9.0.
    ///
    /// As in Java, `ordinals` is reordered.
    fn get_bulk_path(&self, ordinals: &mut [i32]) -> Result<Vec<Option<FacetLabel>>> {
        self.ensure_open()?;
        self.check_ordinal_bounds(ordinals)?;

        let ordinals_length = ordinals.len();
        let mut bulk_path = self.get_path_from_cache(ordinals)?;
        let mut original_position = vec![0usize; ordinals_length];

        let mut uncached_length = 0;
        for i in 0..ordinals_length {
            if bulk_path[i].is_none() {
                ordinals[uncached_length] = ordinals[i];
                original_position[uncached_length] = i;
                uncached_length += 1;
            }
        }

        if uncached_length == 0 {
            return Ok(bulk_path);
        }

        // sort the ordinals to make sure that they're within the same leaf
        // (`InPlaceMergeSorter`, a stable sort).
        let mut pairs: Vec<(i32, usize)> = (0..uncached_length)
            .map(|i| (ordinals[i], original_position[i]))
            .collect();
        pairs.sort_by_key(|p| p.0);
        for (i, (ord, pos)) in pairs.into_iter().enumerate() {
            ordinals[i] = ord;
            original_position[i] = pos;
        }

        let leaves = self.leaves()?;
        let mut leaf_reader_max_doc = 0;
        let mut leaf_reader_doc_base = 0;
        let mut values: Option<Box<dyn rucene::index::BinaryDocValues>> = None;

        for i in 0..uncached_length {
            let ord = ordinals[i];
            if values.is_none() || ord >= leaf_reader_doc_base + leaf_reader_max_doc {
                let reader_index = sub_index_from_leaves(ord, &leaves) as usize;
                let leaf_reader_context = &leaves[reader_index];
                let leaf_reader = leaf_reader_context.leaf_reader();
                leaf_reader_max_doc = leaf_reader.max_doc();
                leaf_reader_doc_base = leaf_reader_context.doc_base();
                values = leaf_reader.get_binary_doc_values(Consts::FULL)?;
            }
            // values is null if the leaf has no BinaryDocValues for Consts.FULL
            let values = values.as_mut().ok_or_else(LuceneError::null_pointer)?;
            let success = values.advance_exact(ord - leaf_reader_doc_base)?;
            debug_assert!(success);
            bulk_path[original_position[i]] = Some(FacetLabel::new(FacetsConfig::string_to_path(
                &values.binary_value()?.utf8_to_string()?,
            ))?);
        }

        let cache = self.category_cache()?;
        let mut cache = lock(&cache);
        for i in 0..uncached_length {
            cache.put(
                ordinals[i],
                bulk_path[original_position[i]]
                    .clone()
                    .expect("INVARIANT: filled above"),
            );
        }

        Ok(bulk_path)
    }

    fn get_size(&self) -> Result<i32> {
        self.ensure_open()?;
        Ok(self.index_reader.num_docs())
    }
}

impl Accountable for DirectoryTaxonomyReader {
    fn ram_bytes_used(&self) -> i64 {
        let caches = lock(&self.caches);
        let mut ram_bytes_used = 0;
        if let Some(taxo_arrays) = &caches.taxo_arrays {
            ram_bytes_used += taxo_arrays.ram_bytes_used();
        }
        if let Some(category_cache) = &caches.category_cache {
            ram_bytes_used += BYTES_PER_CACHE_ENTRY * lock(category_cache).size() as i64;
        }
        if let Some(ordinal_cache) = &caches.ordinal_cache {
            ram_bytes_used += BYTES_PER_CACHE_ENTRY * lock(ordinal_cache).size() as i64;
        }
        ram_bytes_used
    }

    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        let caches = lock(&self.caches);
        let mut resources: Vec<Box<dyn Accountable + '_>> = Vec::new();
        if let Some(taxo_arrays) = &caches.taxo_arrays {
            resources.push(Box::new(Accountables::named_accountable(
                "taxoArrays",
                taxo_arrays.ram_bytes_used(),
            )));
        }
        if let Some(category_cache) = &caches.category_cache {
            resources.push(Box::new(Accountables::named_accountable(
                "categoryCache",
                BYTES_PER_CACHE_ENTRY * lock(category_cache).size() as i64,
            )));
        }
        if let Some(ordinal_cache) = &caches.ordinal_cache {
            resources.push(Box::new(Accountables::named_accountable(
                "ordinalCache",
                BYTES_PER_CACHE_ENTRY * lock(ordinal_cache).size() as i64,
            )));
        }
        resources
    }
}
