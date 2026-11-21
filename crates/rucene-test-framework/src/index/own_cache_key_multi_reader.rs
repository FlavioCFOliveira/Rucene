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

//! Port of `org.apache.lucene.tests.index.OwnCacheKeyMultiReader`: a
//! `MultiReader` that has its own cache key, occasionally useful for testing
//! purposes.

use std::fmt;
use std::sync::{Arc, Mutex, Weak};

use rucene::error::Result;
use rucene::index::index_reader::{
    add_closed_listener_to_set, build_composite_context, composite_reader_to_string, CacheHelper,
    CacheKey, ClosedListener, CompositeReader, IndexReader, IndexReaderCore, StoredFields,
};
use rucene::index::leaf_reader::TermVectors;
use rucene::index::reader_context::IndexReaderContext;
use rucene::index::{MultiReader, Term};
use rucene::internal::tests_hooks::TestSecrets;
use rucene::util::IOUtils;

/// The state Java's `readerClosedListeners` set and anonymous `cacheHelper`
/// hold: the reader's own key and its closed listeners.
struct OwnCacheKeyState {
    cache_key: CacheKey,
    /// Java's `CopyOnWriteArraySet<ClosedListener>`: insertion order, one
    /// entry per listener object.
    reader_closed_listeners: Mutex<Vec<Arc<dyn ClosedListener>>>,
}

/// Port of `org.apache.lucene.tests.index.OwnCacheKeyMultiReader` (`final`,
/// `extends MultiReader`): the [`MultiReader`] over the given sub-readers
/// (closing them when it closes), with a reader cache helper of its own,
/// whose key differs from any sub-reader's and whose closed listeners run
/// when this reader closes.
pub struct OwnCacheKeyMultiReader {
    r#in: Arc<MultiReader>,
    state: Arc<OwnCacheKeyState>,
}

impl OwnCacheKeyMultiReader {
    /// `new OwnCacheKeyMultiReader(subReaders...)`: `super(subReaders)`, a
    /// `MultiReader` that closes its sub-readers.
    ///
    /// # Errors
    ///
    /// What `new MultiReader(subReaders)` raises, and the error of reaching
    /// `TestSecrets.getIndexPackageAccess()` for the key.
    pub fn new(sub_readers: Vec<Arc<dyn IndexReader>>) -> Result<Self> {
        let cache_key = TestSecrets::get_index_package_access()?.new_cache_key();
        Ok(OwnCacheKeyMultiReader {
            r#in: Arc::new(MultiReader::new(sub_readers, true)?),
            state: Arc::new(OwnCacheKeyState {
                cache_key,
                reader_closed_listeners: Mutex::new(Vec::new()),
            }),
        })
    }
}

/// The anonymous `CacheHelper` of `OwnCacheKeyMultiReader`.
struct OwnCacheHelper {
    state: Arc<OwnCacheKeyState>,
    reader: Arc<MultiReader>,
}

impl fmt::Debug for OwnCacheHelper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnCacheHelper").finish_non_exhaustive()
    }
}

impl CacheHelper for OwnCacheHelper {
    fn get_key(&self) -> &CacheKey {
        &self.state.cache_key
    }

    /// `ensureOpen()`, then the listener is added to the
    /// `readerClosedListeners` set. The reader's
    /// `AlreadyClosedException` is unchecked in Java; this signature cannot
    /// fail, so it is thrown (a panic).
    fn add_closed_listener(&self, listener: Arc<dyn ClosedListener>) {
        super::ensure_open_unchecked(self.reader.core());
        add_closed_listener_to_set(
            &mut self
                .state
                .reader_closed_listeners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            listener,
        );
    }
}

impl fmt::Debug for OwnCacheKeyMultiReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnCacheKeyMultiReader")
            .field("in", &self.r#in)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for OwnCacheKeyMultiReader {
    /// `CompositeReader.toString()` under this class's simple name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&composite_reader_to_string(
            "OwnCacheKeyMultiReader",
            &self.r#in.get_sequential_sub_readers(),
        ))
    }
}

impl IndexReader for OwnCacheKeyMultiReader {
    fn core(&self) -> &IndexReaderCore {
        self.r#in.core()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.r#in.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.r#in.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.r#in.max_doc()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.r#in.stored_fields()
    }

    fn do_close(&self) -> Result<()> {
        self.r#in.do_close()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        Some(Box::new(OwnCacheHelper {
            state: Arc::clone(&self.state),
            reader: Arc::clone(&self.r#in),
        }))
    }

    /// Every listener is called with this reader's key, even after one fails
    /// (`IOUtils.applyToAll`): the first error is returned.
    fn notify_reader_closed_listeners(&self) -> Result<()> {
        let listeners = self
            .state
            .reader_closed_listeners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        IOUtils::apply_to_all(listeners.iter().map(Some).collect(), |l| {
            l.on_close(self.state.cache_key.clone())
        })
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.r#in.doc_freq(term)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.r#in.total_term_freq(term)
    }

    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        self.r#in.get_sum_doc_freq(field)
    }

    fn get_doc_count(&self, field: &str) -> Result<i32> {
        self.r#in.get_doc_count(field)
    }

    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        self.r#in.get_sum_total_term_freq(field)
    }

    fn build_context(
        self: Arc<Self>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext> {
        build_composite_context(
            self,
            parent,
            ord_in_parent,
            doc_base_in_parent,
            leaf_ord,
            leaf_doc_base,
        )
    }
}

impl CompositeReader for OwnCacheKeyMultiReader {
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.r#in.get_sequential_sub_readers()
    }
}
