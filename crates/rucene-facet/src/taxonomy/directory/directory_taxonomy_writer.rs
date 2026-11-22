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

//! Port of `DirectoryTaxonomyWriter.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.directory`.

use std::fmt;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::document::{BinaryDocValuesField, Document, NumericDocValuesField, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::reader_manager::ReaderManager;
use rucene::index::{
    directory_reader, IndexReader, SegmentInfos, TwoPhaseCommit,
    POSTINGS_ENUM_NONE,
};
use rucene::store::Directory;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::BytesRef;

use crate::facets_config::FacetsConfig;
use crate::taxonomy::directory::consts::Consts;
use crate::taxonomy::directory::taxonomy_index_arrays::TaxonomyIndexArrays;
use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::parallel_taxonomy_arrays::ParallelTaxonomyArrays;
use crate::taxonomy::taxonomy_reader::{INVALID_ORDINAL, ROOT_ORDINAL};
use crate::taxonomy::taxonomy_writer::TaxonomyWriter;
use crate::taxonomy::writercache::lru_taxonomy_writer_cache::LruTaxonomyWriterCache;
use crate::taxonomy::writercache::taxonomy_writer_cache::TaxonomyWriterCache;

/// Property name of user commit data that contains the index epoch.
///
/// Equivalent to `DirectoryTaxonomyWriter.INDEX_EPOCH`.
pub const INDEX_EPOCH: &str = "index.epoch";

const DEFAULT_CACHE_SIZE: i32 = 4000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `Long.parseLong(s, 16)`.
pub(crate) fn parse_long_hex(s: &str) -> Result<i64> {
    i64::from_str_radix(s, 16).map_err(|_| {
        LuceneError::unchecked(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\" under radix 16"),
        )
    })
}

/// `Long.toString(v, 16)`.
pub(crate) fn long_to_hex(v: i64) -> String {
    if v < 0 {
        format!("-{:x}", v.unsigned_abs())
    } else {
        format!("{v:x}")
    }
}

/// The protected methods of `DirectoryTaxonomyWriter` a subclass overrides:
/// `openIndexWriter`, `createIndexWriterConfig` and `enrichOrdinalDocument`.
///
/// Rust has no implementation inheritance; a subclass's overrides are this
/// strategy object, and its defaults are the base class's bodies.
pub trait DirectoryTaxonomyWriterOverrides: Send + Sync {
    /// Open internal index writer, which contains the taxonomy data.
    ///
    /// Equivalent to `openIndexWriter(Directory, IndexWriterConfig)`.
    fn open_index_writer(
        &self,
        directory: Arc<dyn Directory>,
        config: Arc<IndexWriterConfig>,
    ) -> Result<Arc<IndexWriter>> {
        IndexWriter::new(directory, config)
    }

    /// Create the `IndexWriterConfig` that would be used for opening the
    /// internal index writer.
    ///
    /// Equivalent to `createIndexWriterConfig(OpenMode)`.
    fn create_index_writer_config(&self, open_mode: OpenMode) -> Result<IndexWriterConfig> {
        // TODO: should we use a more optimized Codec?
        // The taxonomy has a unique structure, where each term is associated
        // with one document

        // Make sure we use a MergePolicy which always merges adjacent segments
        // and thus keeps the doc IDs ordered as well (this is crucial for the
        // taxonomy index).
        let config = IndexWriterConfig::with_analyzer(None);
        config.set_open_mode(open_mode)?;
        config.set_merge_policy(Some(Arc::new(LogMergePolicy::by_byte_size())))?;
        Ok(config)
    }

    /// Child classes can implement this method to modify the document
    /// corresponding to a category path before indexing it.
    ///
    /// Equivalent to `enrichOrdinalDocument(Document, FacetLabel)`.
    fn enrich_ordinal_document(&self, _d: &mut Document, _category_path: &FacetLabel) {}
}

/// The base class's own overrides.
struct DefaultOverrides;

impl DirectoryTaxonomyWriterOverrides for DefaultOverrides {}

/// The fields Java guards with the writer's monitor.
struct WriterState {
    cache_misses_until_fill: i32,
    should_fill_cache: bool,
    reader_manager: Option<ReaderManager>,
    should_refresh_reader_manager: bool,
    cache_is_complete: bool,
    taxo_arrays: Option<Arc<TaxonomyIndexArrays>>,
}

/// `TaxonomyWriter` which uses a `Directory` to store the taxonomy
/// information on disk, and keeps an additional in-memory cache of some or
/// all categories.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.directory.DirectoryTaxonomyWriter`.
/// Its `synchronized` methods hold [`state`](Self::state); the `volatile`
/// and atomic fields read outside it are atomics.
pub struct DirectoryTaxonomyWriter {
    dir: Arc<dyn Directory>,
    index_writer: Arc<IndexWriter>,
    cache: Arc<dyn TaxonomyWriterCache>,
    cache_misses: AtomicI32,
    /// Records the taxonomy index epoch, updated on `replaceTaxonomy` as
    /// well.
    index_epoch: AtomicI64,
    next_id: AtomicI32,
    /// `fullPathField`, reused across category documents as Java reuses it.
    full_path_field: Arc<StringField>,
    is_closed: AtomicBool,
    state: Mutex<WriterState>,
    overrides: Box<dyn DirectoryTaxonomyWriterOverrides>,
}

impl fmt::Debug for DirectoryTaxonomyWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectoryTaxonomyWriter")
            .field("next_id", &self.next_id)
            .field("index_epoch", &self.index_epoch)
            .finish_non_exhaustive()
    }
}

impl DirectoryTaxonomyWriter {
    /// Construct a Taxonomy writer.
    ///
    /// Equivalent to `DirectoryTaxonomyWriter(Directory, OpenMode,
    /// TaxonomyWriterCache)`; `None` is Java's `null` cache.
    pub fn with_cache(
        directory: Arc<dyn Directory>,
        open_mode: OpenMode,
        cache: Option<Arc<dyn TaxonomyWriterCache>>,
    ) -> Result<Self> {
        Self::with_overrides(directory, open_mode, cache, Box::new(DefaultOverrides))
    }

    /// The constructor of a subclass that overrides the protected methods
    /// listed by [`DirectoryTaxonomyWriterOverrides`].
    pub fn with_overrides(
        directory: Arc<dyn Directory>,
        open_mode: OpenMode,
        cache: Option<Arc<dyn TaxonomyWriterCache>>,
        overrides: Box<dyn DirectoryTaxonomyWriterOverrides>,
    ) -> Result<Self> {
        let dir = directory;
        let config = Arc::new(overrides.create_index_writer_config(open_mode)?);
        let index_writer = overrides.open_index_writer(Arc::clone(&dir), Arc::clone(&config))?;

        // verify (to some extent) that merge policy in effect would preserve
        // category docids
        debug_assert!(
            !index_writer
                .get_config()
                .merge_policy()
                .as_any()
                .is_some_and(|a| a.is::<rucene::index::merge_policy::TieredMergePolicy>()),
            "for preserving category docids, merging none-adjacent segments is not allowed"
        );

        // after we opened the writer, and the index is locked, it's safe to
        // check the commit data and read the index epoch
        let open_mode = config.open_mode();
        let mut index_epoch;
        if !directory_reader::index_exists(&*dir)? {
            index_epoch = 1;
        } else {
            let mut epoch_str = None;
            let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir))?;
            let commit_data = infos.user_data();
            if let Some(s) = commit_data.get(INDEX_EPOCH) {
                epoch_str = Some(s.clone());
            }
            // no commit data, or no epoch in it means an old taxonomy, so set
            // its epoch to 1, for lack of a better value.
            index_epoch = match epoch_str {
                None => 1,
                Some(s) => parse_long_hex(&s)?,
            };
        }

        if open_mode == OpenMode::CREATE {
            index_epoch += 1;
        }

        let full_path_field = Arc::new(StringField::new(Consts::FULL, String::new(), Store::NO)?);

        let next_id = index_writer.get_doc_stats()?.max_doc;

        let cache = cache.unwrap_or_else(Self::default_taxonomy_writer_cache);
        let writer = Self {
            dir,
            index_writer,
            cache,
            cache_misses: AtomicI32::new(0),
            index_epoch: AtomicI64::new(index_epoch),
            next_id: AtomicI32::new(next_id),
            full_path_field,
            is_closed: AtomicBool::new(false),
            state: Mutex::new(WriterState {
                cache_misses_until_fill: 11,
                should_fill_cache: true,
                reader_manager: None,
                should_refresh_reader_manager: false,
                cache_is_complete: false,
                taxo_arrays: None,
            }),
            overrides,
        };

        if writer.next_id.load(Ordering::SeqCst) == 0 {
            lock(&writer.state).cache_is_complete = true;
            // Make sure that the taxonomy always contain the root category
            // with category id 0.
            writer.add_category(&FacetLabel::empty())?;
        } else {
            // There are some categories on the disk, which we have not yet
            // read into the cache, and therefore the cache is incomplete.
            lock(&writer.state).cache_is_complete = false;
        }
        Ok(writer)
    }

    /// Returns the [`TaxonomyWriterCache`] in use by this writer.
    ///
    /// Equivalent to `getCache()`.
    pub fn get_cache(&self) -> &Arc<dyn TaxonomyWriterCache> {
        &self.cache
    }

    /// Creates a new instance with a default cache as defined by
    /// [`default_taxonomy_writer_cache`](Self::default_taxonomy_writer_cache).
    ///
    /// Equivalent to `DirectoryTaxonomyWriter(Directory, OpenMode)`.
    pub fn with_open_mode(directory: Arc<dyn Directory>, open_mode: OpenMode) -> Result<Self> {
        Self::with_cache(
            directory,
            open_mode,
            Some(Self::default_taxonomy_writer_cache()),
        )
    }

    /// Defines the default [`TaxonomyWriterCache`] to use in constructors
    /// which do not specify one.
    ///
    /// Equivalent to `defaultTaxonomyWriterCache()`.
    pub fn default_taxonomy_writer_cache() -> Arc<dyn TaxonomyWriterCache> {
        Arc::new(LruTaxonomyWriterCache::new(DEFAULT_CACHE_SIZE))
    }

    /// Create this with `OpenMode.CREATE_OR_APPEND`.
    ///
    /// Equivalent to `DirectoryTaxonomyWriter(Directory)`.
    pub fn new(d: Arc<dyn Directory>) -> Result<Self> {
        Self::with_open_mode(d, OpenMode::CREATE_OR_APPEND)
    }

    fn state(&self) -> MutexGuard<'_, WriterState> {
        lock(&self.state)
    }

    /// Equivalent to `initReaderManager()`.
    fn init_reader_manager(&self, state: &mut WriterState) -> Result<()> {
        if state.reader_manager.is_none() {
            self.ensure_open()?;
            // verify that the taxo-writer hasn't been closed on us.
            state.reader_manager = Some(ReaderManager::from_writer_with(
                &*self.index_writer,
                false,
                false,
            )?);
            state.should_refresh_reader_manager = false;
        }
        Ok(())
    }

    /// A hook for extending classes to close additional resources that were
    /// used.
    ///
    /// Equivalent to `closeResources()`.
    fn close_resources(&self, state: &mut WriterState) -> Result<()> {
        if let Some(reader_manager) = state.reader_manager.take() {
            reader_manager.close()?;
        }
        self.cache.close()
    }

    /// Equivalent to `doClose()`.
    fn do_close(&self, state: &mut WriterState) -> Result<()> {
        self.is_closed.store(true, Ordering::SeqCst);
        self.close_resources(state)
    }

    /// Look up the given category in the cache and/or the on-disk storage,
    /// returning the category's ordinal, or a negative number in case the
    /// category does not yet exist in the taxonomy.
    ///
    /// Equivalent to `findCategory(FacetLabel)`.
    fn find_category(&self, state: &mut WriterState, category_path: &FacetLabel) -> Result<i32> {
        // If we can find the category in the cache, or we know the cache is
        // complete, we can return the response directly from it
        let mut res = self.cache.get(category_path)?;
        if res >= 0 || state.cache_is_complete {
            return Ok(res);
        }

        self.cache_misses.fetch_add(1, Ordering::SeqCst);
        // After a few cache misses, it makes sense to read all the categories
        // from disk and into the cache.
        self.perhaps_fill_cache(state)?;
        res = self.cache.get(category_path)?;
        if res >= 0 || state.cache_is_complete {
            // if after filling the cache from the info on disk, the category
            // is in it or the cache is complete, return whatever cache.get
            // returned.
            return Ok(res);
        }

        // if we get here, it means the category is not in the cache, and it
        // is not complete, and therefore we must look for the category on
        // disk.

        // We need to get an answer from the on-disk index.
        self.init_reader_manager(state)?;

        let mut doc = -1;
        let reader_manager = state
            .reader_manager
            .as_ref()
            .expect("INVARIANT: initialized above");
        let reader = reader_manager.acquire()?;
        let result = (|| -> Result<()> {
            let cat_term = BytesRef::from_char_sequence(&FacetsConfig::path_to_string_len(
                &category_path.components,
                category_path.length,
            )?);
            let reader: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
            for ctx in reader.leaves()? {
                let Some(terms) = ctx.leaf_reader().terms(Consts::FULL)? else {
                    continue;
                };
                let mut terms_enum = terms.iterator()?;
                if terms_enum.seek_exact(&cat_term)? {
                    // TODO: we could use this to obtain the docID
                    let mut docs = terms_enum
                        .postings(None, 0 /* freqs not required */)?
                        .ok_or_else(LuceneError::null_pointer)?;
                    // if the term was found, we know it has exactly one
                    // document.
                    doc = docs.next_doc()? + ctx.doc_base();
                    break;
                }
            }
            Ok(())
        })();
        let released = reader_manager.release(reader);
        result?;
        released?;
        if doc > 0 {
            self.add_to_cache(state, category_path, doc)?;
        }
        Ok(doc)
    }

    /// Add a new category into the index (and the cache), and return its new
    /// ordinal.
    ///
    /// Equivalent to `internalAddCategory(FacetLabel)`.
    fn internal_add_category(&self, state: &mut WriterState, cp: &FacetLabel) -> Result<i32> {
        // Find our parent's ordinal (recursively adding the parent category to
        // the taxonomy if it's not already there). Then add the parent ordinal
        // as payloads (rather than a stored field; payloads can be more
        // efficiently read into memory in bulk by LuceneTaxonomyReader)
        let parent;
        if cp.length > 1 {
            let parent_path = cp.subpath(cp.length - 1);
            let mut found = self.find_category(state, &parent_path)?;
            if found < 0 {
                found = self.internal_add_category(state, &parent_path)?;
            }
            parent = found;
        } else if cp.length == 1 {
            parent = ROOT_ORDINAL;
        } else {
            parent = INVALID_ORDINAL;
        }
        self.add_category_document(state, cp, parent)
    }

    /// Verifies that this instance wasn't closed, or throws
    /// `AlreadyClosedException` if it is.
    ///
    /// Equivalent to `ensureOpen()`.
    pub fn ensure_open(&self) -> Result<()> {
        if self.is_closed.load(Ordering::SeqCst) {
            return Err(LuceneError::already_closed(
                "The taxonomy writer has already been closed",
            ));
        }
        Ok(())
    }

    /// Note that the methods calling `addCategoryDocument()` are synchronized,
    /// so this method is effectively synchronized as well.
    ///
    /// Equivalent to `addCategoryDocument(FacetLabel, int)`.
    fn add_category_document(
        &self,
        state: &mut WriterState,
        category_path: &FacetLabel,
        parent: i32,
    ) -> Result<i32> {
        let mut d = Document::new();
        // Before Lucene 2.9, position increments >=0 were supported, so we
        // added 1 to parent to allow the parent -1 (the parent of the root).
        // Unfortunately, starting with Lucene 2.9, after LUCENE-1542, this is
        // no longer enough, since 0 is not encoded consistently either (see
        // comment in SinglePositionTokenStream). But because we must be
        // backward-compatible with existing indexes, we can't just fix what
        // we write here (e.g., to write parent+2), and need to do a
        // workaround in the reader (which knows that anyway only category 0
        // has a parent -1).
        d.add(NumericDocValuesField::new(
            Consts::FIELD_PARENT_ORDINAL_NDV,
            parent as i64,
        ));

        let field_path =
            FacetsConfig::path_to_string_len(&category_path.components, category_path.length)?;
        self.full_path_field.set_string_value(field_path.clone())?;

        d.add(BinaryDocValuesField::new(
            Consts::FULL,
            BytesRef::from_char_sequence(&field_path),
        ));
        d.add_shared(Arc::clone(&self.full_path_field) as Arc<dyn rucene::index::IndexableField>);

        // add arbitrary ordinal data to the doc
        self.overrides.enrich_ordinal_document(&mut d, category_path);

        self.index_writer.add_document(&d)?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        // added a category document, mark that ReaderManager is not up-to-date
        state.should_refresh_reader_manager = true;

        // also add to the parent array
        let arrays = self.get_taxo_arrays(state)?;
        state.taxo_arrays = Some(arrays.add(id, parent));

        // NOTE: this line must be executed last, or else the cache gets
        // updated before the parents array (LUCENE-4596)
        self.add_to_cache(state, category_path, id)?;

        Ok(id)
    }

    /// Equivalent to `addToCache(FacetLabel, int)`.
    fn add_to_cache(
        &self,
        state: &mut WriterState,
        category_path: &FacetLabel,
        id: i32,
    ) -> Result<()> {
        if self.cache.put(category_path, id)? {
            // If cache.put() returned true, it means the cache was limited in
            // size, became full, and parts of it had to be evicted. It is
            // possible that a relatively-new category that isn't yet visible
            // to our 'reader' was evicted, and therefore we must now refresh
            // the reader.
            self.refresh_reader_manager(state)?;
            state.cache_is_complete = false;
        }
        Ok(())
    }

    /// Equivalent to `refreshReaderManager()`.
    fn refresh_reader_manager(&self, state: &mut WriterState) -> Result<()> {
        // this method is synchronized since it cannot happen concurrently with
        // addCategoryDocument -- when this method returns, we must know that
        // the reader manager's state is current. also, it sets
        // shouldRefresh to false, and this cannot overlap with addCatDoc too.
        // NOTE: since this method is sync'ed, it can call maybeRefresh, instead
        // of maybeRefreshBlocking. If ever this is changed, make sure to change
        // the call too.
        if state.should_refresh_reader_manager {
            if let Some(reader_manager) = state.reader_manager.as_ref() {
                reader_manager.maybe_refresh()?;
                state.should_refresh_reader_manager = false;
            }
        }
        Ok(())
    }

    /// Equivalent to the monitor-held body of `commit()`.
    fn commit_locked(&self) -> Result<i64> {
        self.ensure_open()?;
        // LUCENE-4972: if we always call setCommitData, we create empty
        // commits
        let mut data: JavaHashMap<String> = JavaHashMap::new();
        for (k, v) in self.index_writer.get_live_commit_data()? {
            data.insert(k, v);
        }
        let epoch_str = data.get(INDEX_EPOCH).cloned();
        if epoch_str.is_none()
            || parse_long_hex(epoch_str.as_deref().expect("INVARIANT: checked"))?
                != self.index_epoch.load(Ordering::SeqCst)
        {
            self.index_writer.set_live_commit_data(
                self.combined_commit_data(Some(self.index_writer.get_live_commit_data()?)),
            )?;
        }
        self.index_writer.commit()
    }

    /// Combine original user data with the taxonomy epoch.
    ///
    /// Equivalent to `combinedCommitData(Iterable)`; the result iterates in
    /// the order of Java's `HashMap.entrySet()`.
    fn combined_commit_data(&self, commit_data: Option<Vec<(String, String)>>) -> Vec<(String, String)> {
        let mut m: JavaHashMap<String> = JavaHashMap::new();
        if let Some(commit_data) = commit_data {
            for (k, v) in commit_data {
                m.insert(k, v);
            }
        }
        m.insert(
            INDEX_EPOCH.to_string(),
            long_to_hex(self.index_epoch.load(Ordering::SeqCst)),
        );
        m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Set the number of cache misses before an attempt is made to read the
    /// entire taxonomy into the in-memory cache.
    ///
    /// Equivalent to `setCacheMissesUntilFill(int)`.
    pub fn set_cache_misses_until_fill(&self, i: i32) -> Result<()> {
        self.ensure_open()?;
        self.state().cache_misses_until_fill = i;
        Ok(())
    }

    /// Equivalent to `perhapsFillCache()`.
    fn perhaps_fill_cache(&self, state: &mut WriterState) -> Result<()> {
        if self.cache_misses.load(Ordering::SeqCst) < state.cache_misses_until_fill {
            return Ok(());
        }

        if !state.should_fill_cache {
            // we already filled the cache once, there's no need to re-fill it
            return Ok(());
        }
        state.should_fill_cache = false;

        self.init_reader_manager(state)?;

        let mut aborted = false;
        let reader_manager = state
            .reader_manager
            .as_ref()
            .expect("INVARIANT: initialized above");
        let reader = reader_manager.acquire()?;
        let result = (|| -> Result<()> {
            let reader: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
            for ctx in reader.leaves()? {
                let Some(terms) = ctx.leaf_reader().terms(Consts::FULL)? else {
                    continue;
                };
                let mut terms_enum = terms.iterator()?;
                // TODO: share per-segment TermsEnum here!
                loop {
                    let Some(t) = terms_enum.next()?.cloned() else {
                        break;
                    };
                    if !self.cache.is_full()? {
                        let cp = FacetLabel::new(FacetsConfig::string_to_path(&t.utf8_to_string()?))?;
                        let mut postings_enum = terms_enum
                            .postings(None, POSTINGS_ENUM_NONE)?
                            .ok_or_else(LuceneError::null_pointer)?;
                        let res = self
                            .cache
                            .put(&cp, postings_enum.next_doc()? + ctx.doc_base())?;
                        debug_assert!(!res, "entries should not have been evicted from the cache");
                    } else {
                        // the cache is full and the next put() will evict
                        // entries from it, therefore abort the iteration.
                        aborted = true;
                        break;
                    }
                }
                if aborted {
                    break;
                }
            }
            Ok(())
        })();
        let released = reader_manager.release(reader);
        result?;
        released?;

        state.cache_is_complete = !aborted;
        if state.cache_is_complete {
            // everything is in the cache, so no need to keep the reader
            // manager open. this block is executed in a sync block so that it
            // works well with initReaderManager called in parallel.
            if let Some(reader_manager) = state.reader_manager.take() {
                reader_manager.close()?;
            }
        }
        Ok(())
    }

    /// Equivalent to `getTaxoArrays()`.
    fn get_taxo_arrays(&self, state: &mut WriterState) -> Result<Arc<TaxonomyIndexArrays>> {
        if let Some(arrays) = &state.taxo_arrays {
            return Ok(Arc::clone(arrays));
        }
        self.init_reader_manager(state)?;
        let reader_manager = state
            .reader_manager
            .as_ref()
            .expect("INVARIANT: initialized above");
        let reader = reader_manager.acquire()?;
        // according to Java Concurrency, this might perform better on some
        // JVMs, since the object initialization doesn't happen on the
        // volatile member.
        let arrays = TaxonomyIndexArrays::new(&(Arc::clone(&reader) as Arc<dyn IndexReader>));
        let released = reader_manager.release(reader);
        let arrays = Arc::new(arrays?);
        released?;
        state.taxo_arrays = Some(Arc::clone(&arrays));
        Ok(arrays)
    }

    /// Takes the categories from the given taxonomy directory, and adds the
    /// missing ones to this taxonomy.
    ///
    /// Equivalent to `addTaxonomy(Directory, OrdinalMap)`.
    pub fn add_taxonomy(&self, taxo_dir: Arc<dyn Directory>, map: &mut dyn OrdinalMap) -> Result<()> {
        self.ensure_open()?;
        let r = directory_reader::open(taxo_dir)?;
        let result = (|| -> Result<()> {
            let size = r.num_docs();
            map.set_size(size)?;
            let mut base = 0;
            let reader: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
            for ctx in reader.leaves()? {
                let ar = ctx.leaf_reader();
                let terms = ar.terms(Consts::FULL)?.ok_or_else(LuceneError::null_pointer)?;
                // TODO: share per-segment TermsEnum here!
                let mut te = terms.iterator()?;
                loop {
                    let Some(term) = te.next()?.cloned() else {
                        break;
                    };
                    let cp = FacetLabel::new(FacetsConfig::string_to_path(&term.utf8_to_string()?))?;
                    let ordinal = self.add_category(&cp)?;
                    let mut docs = te
                        .postings(None, POSTINGS_ENUM_NONE)?
                        .ok_or_else(LuceneError::null_pointer)?;
                    map.add_mapping(docs.next_doc()? + base, ordinal)?;
                }
                base += ar.max_doc(); // no deletions, so we're ok
            }
            map.add_done()
        })();
        let closed = r.close();
        result?;
        closed
    }

    /// Replaces the current taxonomy with the given one.
    ///
    /// Equivalent to `replaceTaxonomy(Directory)`.
    pub fn replace_taxonomy(&self, taxo_dir: Arc<dyn Directory>) -> Result<()> {
        let mut state = self.state();
        // replace the taxonomy by doing IW optimized operations
        self.index_writer.delete_all()?;
        self.index_writer.add_indexes_dirs(&[taxo_dir])?;
        state.should_refresh_reader_manager = true;
        self.init_reader_manager(&mut state)?; // ensure that it's initialized
        self.refresh_reader_manager(&mut state)?;
        self.next_id
            .store(self.index_writer.get_doc_stats()?.max_doc, Ordering::SeqCst);
        state.taxo_arrays = None; // must nullify so that it's re-computed next time it's needed

        // need to clear the cache, so that addCategory won't accidentally
        // return old categories that are in the cache.
        self.cache.clear()?;
        state.cache_is_complete = false;
        state.should_fill_cache = true;
        self.cache_misses.store(0, Ordering::SeqCst);

        // update indexEpoch as a taxonomy replace is just like it has be
        // recreated
        self.index_epoch.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Delete all taxonomy data and start over.
    ///
    /// Equivalent to the package-private `deleteAll()`.
    pub(crate) fn delete_all(&self) -> Result<()> {
        let mut state = self.state();
        self.index_writer.delete_all()?;
        state.should_refresh_reader_manager = true;
        self.init_reader_manager(&mut state)?; // ensure that it's initialized
        self.refresh_reader_manager(&mut state)?;
        self.next_id.store(0, Ordering::SeqCst);
        state.taxo_arrays = None; // must nullify so that it's re-computed next time it's needed

        // need to clear the cache, so that addCategory won't accidentally
        // return old categories that are in the cache.
        self.cache.clear()?;
        state.cache_is_complete = false;
        state.should_fill_cache = true;
        self.cache_misses.store(0, Ordering::SeqCst);
        self.index_epoch.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Returns the `Directory` of this taxonomy writer.
    ///
    /// Equivalent to `getDirectory()`.
    pub fn get_directory(&self) -> &Arc<dyn Directory> {
        &self.dir
    }

    /// Used by `DirectoryTaxonomyReader` to support NRT.
    ///
    /// Equivalent to `getInternalIndexWriter()`.
    pub fn get_internal_index_writer(&self) -> &Arc<IndexWriter> {
        &self.index_writer
    }

    /// Expert: returns current index epoch, if this is a near-real-time
    /// reader.
    ///
    /// Equivalent to `getTaxonomyEpoch()`.
    pub fn get_taxonomy_epoch(&self) -> i64 {
        self.index_epoch.load(Ordering::SeqCst)
    }
}

impl TwoPhaseCommit for DirectoryTaxonomyWriter {
    fn prepare_commit(&self) -> Result<i64> {
        let _state = self.state();
        self.ensure_open()?;
        // LUCENE-4972: if we always call setCommitData, we create empty
        // commits
        let mut data: JavaHashMap<String> = JavaHashMap::new();
        for (k, v) in self.index_writer.get_live_commit_data()? {
            data.insert(k, v);
        }
        let epoch_str = data.get(INDEX_EPOCH).cloned();
        if epoch_str.is_none()
            || parse_long_hex(epoch_str.as_deref().expect("INVARIANT: checked"))?
                != self.index_epoch.load(Ordering::SeqCst)
        {
            self.index_writer.set_live_commit_data(
                self.combined_commit_data(Some(self.index_writer.get_live_commit_data()?)),
            )?;
        }
        self.index_writer.prepare_commit()
    }

    fn commit(&self) -> Result<i64> {
        let _state = self.state();
        self.commit_locked()
    }

    fn rollback(&self) -> Result<()> {
        let mut state = self.state();
        self.ensure_open()?;
        self.index_writer.rollback()?;
        self.do_close(&mut state)
    }
}

impl TaxonomyWriter for DirectoryTaxonomyWriter {
    fn add_category(&self, category_path: &FacetLabel) -> Result<i32> {
        self.ensure_open()?;

        // check the cache outside the synchronized block. this results in
        // better concurrency when categories are there.
        let mut res = self.cache.get(category_path)?;
        if res < 0 {
            // the category is not in the cache - following code cannot be
            // executed in parallel.
            let mut state = self.state();
            res = self.find_category(&mut state, category_path)?;
            if res < 0 {
                // This is a new category, and we need to insert it into the
                // index (and the cache). Actually, we might also need to add
                // some of the category's ancestors before we can add the
                // category itself (while keeping the invariant that a parent
                // is always added to the taxonomy before its child).
                // internalAddCategory() does all this recursively
                res = self.internal_add_category(&mut state, category_path)?;
            }
        }
        Ok(res)
    }

    fn get_parent(&self, ordinal: i32) -> Result<i32> {
        self.ensure_open()?;
        // Note: the following if() just enforces that a user can never ask for
        // the parent of a nonexistent category - even if the parent array was
        // allocated bigger than it really needs to be.
        let next_id = self.next_id.load(Ordering::SeqCst);
        if ordinal < 0 || ordinal >= next_id {
            return Err(LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {ordinal} out of bounds for length {next_id}"),
            ));
        }
        let arrays = {
            let mut state = self.state();
            self.get_taxo_arrays(&mut state)?
        };
        let parents = arrays.parents();
        debug_assert!(
            ordinal < parents.length(),
            "requested ordinal ({}); parents.length ({}) !",
            ordinal,
            parents.length()
        );
        Ok(parents.get(ordinal))
    }

    fn get_size(&self) -> Result<i32> {
        self.ensure_open()?;
        Ok(self.next_id.load(Ordering::SeqCst))
    }

    fn set_live_commit_data(&self, commit_user_data: Option<Vec<(String, String)>>) -> Result<()> {
        self.index_writer
            .set_live_commit_data(self.combined_commit_data(commit_user_data))
    }

    fn get_live_commit_data(&self) -> Result<Vec<(String, String)>> {
        Ok(self.combined_commit_data(Some(self.index_writer.get_live_commit_data()?)))
    }

    /// Frees used resources as well as closes the underlying `IndexWriter`,
    /// which commits whatever changes made to it to the underlying
    /// `Directory`.
    fn close(&self) -> Result<()> {
        let mut state = self.state();
        if !self.is_closed.load(Ordering::SeqCst) {
            self.commit_locked()?;
            self.index_writer.close()?;
            self.do_close(&mut state)?;
        }
        Ok(())
    }
}

/// Mapping from old ordinal to new ordinals, used when merging indexes with
/// separate taxonomies.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.directory.DirectoryTaxonomyWriter.OrdinalMap`.
pub trait OrdinalMap {
    /// Set the size of the map.
    ///
    /// Equivalent to `setSize(int)`.
    fn set_size(&mut self, size: i32) -> Result<()>;

    /// Record a mapping.
    ///
    /// Equivalent to `addMapping(int, int)`.
    fn add_mapping(&mut self, orig_ordinal: i32, new_ordinal: i32) -> Result<()>;

    /// Call `add_done()` to say that all `add_mapping()` have been done.
    ///
    /// Equivalent to `addDone()`.
    fn add_done(&mut self) -> Result<()>;

    /// Return the map from the taxonomy's original (consecutive) ordinals to
    /// the new taxonomy's ordinals.
    ///
    /// Equivalent to `getMap()`.
    fn get_map(&mut self) -> Result<Vec<i32>>;
}

/// `OrdinalMap` maintained in memory.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.directory.DirectoryTaxonomyWriter.MemoryOrdinalMap`.
#[derive(Debug, Default)]
pub struct MemoryOrdinalMap {
    map: Option<Vec<i32>>,
}

impl MemoryOrdinalMap {
    /// Sole constructor.
    ///
    /// Equivalent to `MemoryOrdinalMap()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl OrdinalMap for MemoryOrdinalMap {
    fn set_size(&mut self, taxonomy_size: i32) -> Result<()> {
        self.map = Some(vec![0; taxonomy_size.max(0) as usize]);
        Ok(())
    }

    fn add_mapping(&mut self, orig_ordinal: i32, new_ordinal: i32) -> Result<()> {
        let map = self.map.as_mut().ok_or_else(LuceneError::null_pointer)?;
        map[orig_ordinal as usize] = new_ordinal;
        Ok(())
    }

    fn add_done(&mut self) -> Result<()> {
        // nothing to do
        Ok(())
    }

    fn get_map(&mut self) -> Result<Vec<i32>> {
        self.map.clone().ok_or_else(LuceneError::null_pointer)
    }
}

/// `OrdinalMap` maintained on file system.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.directory.DirectoryTaxonomyWriter.DiskOrdinalMap`.
/// Java's `DataOutputStream` writes big-endian `int`s.
#[derive(Debug)]
pub struct DiskOrdinalMap {
    map_file: PathBuf,
    map: Option<Vec<i32>>,
    out: Option<BufWriter<std::fs::File>>,
}

impl DiskOrdinalMap {
    /// Sole constructor.
    ///
    /// Equivalent to `DiskOrdinalMap(Path)`.
    pub fn new(map_file: &Path) -> Result<Self> {
        let out = BufWriter::new(std::fs::File::create(map_file)?);
        Ok(Self {
            map_file: map_file.to_path_buf(),
            map: None,
            out: Some(out),
        })
    }

    fn write_int(&mut self, v: i32) -> Result<()> {
        let out = self.out.as_mut().ok_or_else(LuceneError::null_pointer)?;
        out.write_all(&v.to_be_bytes())?;
        Ok(())
    }
}

impl OrdinalMap for DiskOrdinalMap {
    fn add_mapping(&mut self, orig_ordinal: i32, new_ordinal: i32) -> Result<()> {
        self.write_int(orig_ordinal)?;
        self.write_int(new_ordinal)
    }

    fn set_size(&mut self, taxonomy_size: i32) -> Result<()> {
        self.write_int(taxonomy_size)
    }

    fn add_done(&mut self) -> Result<()> {
        if let Some(mut out) = self.out.take() {
            out.flush()?;
        }
        Ok(())
    }

    fn get_map(&mut self) -> Result<Vec<i32>> {
        if let Some(map) = &self.map {
            return Ok(map.clone());
        }
        self.add_done()?; // in case this wasn't previously called
        let mut input = BufReader::new(std::fs::File::open(&self.map_file)?);
        let mut read_int = || -> Result<i32> {
            let mut b = [0u8; 4];
            input.read_exact(&mut b)?;
            Ok(i32::from_be_bytes(b))
        };
        let size = read_int()?;
        let mut map = vec![0; size.max(0) as usize];
        for _ in 0..map.len() {
            let orig_ordinal = read_int()?;
            let new_ordinal = read_int()?;
            map[orig_ordinal as usize] = new_ordinal;
        }
        // Delete the temporary file, which is no longer needed.
        std::fs::remove_file(&self.map_file)?;
        self.map = Some(map.clone());
        Ok(map)
    }
}