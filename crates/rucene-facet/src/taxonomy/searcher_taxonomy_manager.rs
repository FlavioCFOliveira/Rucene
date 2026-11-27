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

//! Port of `SearcherTaxonomyManager.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::{directory_reader, DirectoryReader, IndexReader};
use rucene::search::reference_manager::{ManagedReference, ReferenceManager, RefreshSource};
use rucene::search::refresh_commit_supplier::{LatestCommitSupplier, RefreshCommitSupplier};
use rucene::search::searcher_factory::{DefaultSearcherFactory, SearcherFactory};
use rucene::search::searcher_manager::{get_searcher, ManagedSearcher};
use rucene::search::IndexSearcher;
use rucene::store::Directory;

use crate::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use crate::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use crate::taxonomy::taxonomy_reader::TaxonomyReader;

/// Holds a matched pair of `IndexSearcher` and `TaxonomyReader`.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.taxonomy.SearcherTaxonomyManager.SearcherAndTaxonomy`;
/// the searcher is held with the `DirectoryReader` it searches.
pub struct SearcherAndTaxonomy {
    /// Point-in-time `IndexSearcher`.
    pub searcher: ManagedSearcher,
    /// Matching point-in-time `DirectoryTaxonomyReader`.
    pub taxonomy_reader: Arc<DirectoryTaxonomyReader>,
}

impl SearcherAndTaxonomy {
    /// Equivalent to `searcher()`.
    pub fn searcher(&self) -> &IndexSearcher {
        self.searcher.searcher()
    }

    /// Equivalent to `taxonomyReader()`.
    pub fn taxonomy_reader(&self) -> &Arc<DirectoryTaxonomyReader> {
        &self.taxonomy_reader
    }
}

impl fmt::Debug for SearcherAndTaxonomy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearcherAndTaxonomy")
            .finish_non_exhaustive()
    }
}

impl ManagedReference for SearcherAndTaxonomy {
    /// Equivalent to `SearcherTaxonomyManager.decRef(SearcherAndTaxonomy)`.
    fn release_ref(&self) -> Result<()> {
        self.searcher.reader().dec_ref()?;
        self.taxonomy_reader.dec_ref()
    }

    /// Equivalent to `SearcherTaxonomyManager.tryIncRef(SearcherAndTaxonomy)`.
    fn try_acquire_ref(&self) -> bool {
        if self.searcher.reader().try_inc_ref() {
            if self.taxonomy_reader.try_inc_ref() {
                return true;
            } else {
                let _ = self.searcher.reader().dec_ref();
            }
        }
        false
    }

    /// Equivalent to `SearcherTaxonomyManager.getRefCount(SearcherAndTaxonomy)`.
    fn ref_count(&self) -> i32 {
        // This is used only by assert:
        self.searcher.reader().get_ref_count()
    }
}

/// The `refreshIfNeeded` half of `SearcherTaxonomyManager`.
#[derive(Debug)]
struct TaxonomyRefreshSource {
    searcher_factory: Arc<dyn SearcherFactory>,
    taxo_epoch: i64,
    taxo_writer: Option<Arc<DirectoryTaxonomyWriter>>,
    refresh_commit_supplier: Arc<dyn RefreshCommitSupplier>,
}

impl RefreshSource<SearcherAndTaxonomy> for TaxonomyRefreshSource {
    /// Equivalent to `refreshIfNeeded(SearcherAndTaxonomy)`.
    fn refresh_if_needed(
        &self,
        reference: &Arc<SearcherAndTaxonomy>,
    ) -> Result<Option<Arc<SearcherAndTaxonomy>>> {
        // Must re-open searcher first, otherwise we may get a new reader that
        // references ords not yet known to the taxonomy reader:
        let dr = Arc::clone(reference.searcher.reader());
        let refresh_commit = self
            .refresh_commit_supplier
            .get_searcher_refresh_commit(&dr)?;
        let new_reader = match refresh_commit {
            Some(commit) => {
                directory_reader::open_if_changed_with_commit(Arc::clone(&dr), &*commit)?
            }
            None => directory_reader::open_if_changed(Arc::clone(&dr))?,
        };
        let Some(new_reader) = new_reader else {
            return Ok(None);
        };
        let tr = match reference.taxonomy_reader.open_if_changed() {
            Ok(tr) => tr,
            Err(t1) => {
                let _ = new_reader.close();
                return Err(t1);
            }
        };
        let tr: Arc<DirectoryTaxonomyReader> = match tr {
            None => {
                reference.taxonomy_reader.inc_ref()?;
                Arc::clone(&reference.taxonomy_reader)
            }
            Some(tr) => {
                if let Some(taxo_writer) = &self.taxo_writer {
                    if taxo_writer.get_taxonomy_epoch() != self.taxo_epoch {
                        let closed_reader = new_reader.close();
                        let closed_taxo = tr.close();
                        closed_reader?;
                        closed_taxo?;
                        return Err(LuceneError::illegal_state(
                            "DirectoryTaxonomyWriter.replaceTaxonomy was called, which is not allowed when using SearcherTaxonomyManager",
                        ));
                    }
                }
                Arc::new(tr)
            }
        };
        Ok(Some(Arc::new(SearcherAndTaxonomy {
            searcher: get_searcher(&*self.searcher_factory, new_reader, Some(dr))?,
            taxonomy_reader: tr,
        })))
    }
}

/// Manages near-real-time reopen of both an `IndexSearcher` and a
/// `TaxonomyReader`.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.SearcherTaxonomyManager`,
/// a `ReferenceManager<SearcherAndTaxonomy>`, which it dereferences to.
pub struct SearcherTaxonomyManager {
    manager: ReferenceManager<SearcherAndTaxonomy>,
}

impl SearcherTaxonomyManager {
    /// Creates near-real-time searcher and taxonomy reader from the
    /// corresponding writers.
    ///
    /// Equivalent to `SearcherTaxonomyManager(IndexWriter, SearcherFactory,
    /// DirectoryTaxonomyWriter)`.
    pub fn from_writers(
        writer: &Arc<IndexWriter>,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
        taxo_writer: Arc<DirectoryTaxonomyWriter>,
    ) -> Result<Self> {
        Self::from_writers_with_deletes(writer, true, searcher_factory, taxo_writer)
    }

    /// Expert: creates near-real-time searcher and taxonomy reader from the
    /// corresponding writers, controlling whether deletes should be applied.
    ///
    /// Equivalent to `SearcherTaxonomyManager(IndexWriter, boolean,
    /// SearcherFactory, DirectoryTaxonomyWriter)`.
    pub fn from_writers_with_deletes(
        writer: &Arc<IndexWriter>,
        apply_all_deletes: bool,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
        taxo_writer: Arc<DirectoryTaxonomyWriter>,
    ) -> Result<Self> {
        Self::from_writers_with_commit_supplier(
            writer,
            apply_all_deletes,
            searcher_factory,
            taxo_writer,
            None,
        )
    }

    /// Expert: creates near-real-time searcher and taxonomy reader from the
    /// corresponding writers, controlling whether deletes should be applied
    /// and which commit a refresh opens.
    ///
    /// Equivalent to `SearcherTaxonomyManager(IndexWriter, boolean,
    /// SearcherFactory, DirectoryTaxonomyWriter, RefreshCommitSupplier)`.
    pub fn from_writers_with_commit_supplier(
        writer: &Arc<IndexWriter>,
        apply_all_deletes: bool,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
        taxo_writer: Arc<DirectoryTaxonomyWriter>,
        refresh_commit_supplier: Option<Arc<dyn RefreshCommitSupplier>>,
    ) -> Result<Self> {
        let searcher_factory: Arc<dyn SearcherFactory> =
            searcher_factory.unwrap_or_else(|| Arc::new(DefaultSearcherFactory));
        let taxo_reader = DirectoryTaxonomyReader::from_writer(Arc::clone(&taxo_writer))?;
        let current = SearcherAndTaxonomy {
            searcher: get_searcher(
                &*searcher_factory,
                directory_reader::open_with_writer_flags(&**writer, apply_all_deletes, false)?,
                None,
            )?,
            taxonomy_reader: Arc::new(taxo_reader),
        };
        let taxo_epoch = taxo_writer.get_taxonomy_epoch();
        let source = TaxonomyRefreshSource {
            searcher_factory,
            taxo_epoch,
            taxo_writer: Some(taxo_writer),
            refresh_commit_supplier: refresh_commit_supplier
                .unwrap_or_else(|| Arc::new(LatestCommitSupplier)),
        };
        Ok(Self {
            manager: ReferenceManager::new(Arc::new(current), Arc::new(source)),
        })
    }

    /// Creates search and taxonomy readers over the corresponding
    /// directories.
    ///
    /// Equivalent to `SearcherTaxonomyManager(Directory, Directory,
    /// SearcherFactory)`.
    pub fn from_directories(
        index_dir: Arc<dyn Directory>,
        taxo_dir: Arc<dyn Directory>,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
    ) -> Result<Self> {
        let searcher_factory: Arc<dyn SearcherFactory> =
            searcher_factory.unwrap_or_else(|| Arc::new(DefaultSearcherFactory));
        let taxo_reader = DirectoryTaxonomyReader::open(taxo_dir)?;
        let current = SearcherAndTaxonomy {
            searcher: get_searcher(&*searcher_factory, directory_reader::open(index_dir)?, None)?,
            taxonomy_reader: Arc::new(taxo_reader),
        };
        let source = TaxonomyRefreshSource {
            searcher_factory,
            taxo_epoch: -1,
            taxo_writer: None,
            refresh_commit_supplier: Arc::new(LatestCommitSupplier),
        };
        Ok(Self {
            manager: ReferenceManager::new(Arc::new(current), Arc::new(source)),
        })
    }

    /// Creates this from already opened `IndexReader` and
    /// `DirectoryTaxonomyReader` instances.
    ///
    /// Equivalent to `SearcherTaxonomyManager(IndexReader,
    /// DirectoryTaxonomyReader, SearcherFactory)`. The reader is a
    /// `DirectoryReader`, which `refreshIfNeeded` casts it to.
    pub fn from_readers(
        reader: Arc<dyn DirectoryReader>,
        taxo_reader: Arc<DirectoryTaxonomyReader>,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
    ) -> Result<Self> {
        Self::from_readers_with_commit_supplier(reader, taxo_reader, searcher_factory, None)
    }

    /// Equivalent to `SearcherTaxonomyManager(IndexReader,
    /// DirectoryTaxonomyReader, SearcherFactory, RefreshCommitSupplier)`.
    pub fn from_readers_with_commit_supplier(
        reader: Arc<dyn DirectoryReader>,
        taxo_reader: Arc<DirectoryTaxonomyReader>,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
        refresh_commit_supplier: Option<Arc<dyn RefreshCommitSupplier>>,
    ) -> Result<Self> {
        let searcher_factory: Arc<dyn SearcherFactory> =
            searcher_factory.unwrap_or_else(|| Arc::new(DefaultSearcherFactory));
        let current = SearcherAndTaxonomy {
            searcher: get_searcher(&*searcher_factory, reader, None)?,
            taxonomy_reader: taxo_reader,
        };
        let source = TaxonomyRefreshSource {
            searcher_factory,
            taxo_epoch: -1,
            taxo_writer: None,
            refresh_commit_supplier: refresh_commit_supplier
                .unwrap_or_else(|| Arc::new(LatestCommitSupplier)),
        };
        Ok(Self {
            manager: ReferenceManager::new(Arc::new(current), Arc::new(source)),
        })
    }

    /// Returns the generation of the commit the current searcher reads.
    ///
    /// Equivalent to the package-private `getSearcherCommitGeneration()`.
    // Public for the port of `TestSearcherTaxonomyManager` (Java package-private).
    pub fn get_searcher_commit_generation(&self) -> Result<i64> {
        let sat = self.manager.acquire()?;
        let gen = sat
            .searcher
            .reader()
            .index_commit()
            .map(|c| c.get_generation());
        self.manager.release(sat)?;
        gen
    }

    /// Equivalent to the package-private `isSearcherCurrent()`.
    // Public for the port of `TestSearcherTaxonomyManager` (Java package-private).
    pub fn is_searcher_current(&self) -> Result<bool> {
        let sat = self.manager.acquire()?;
        let current = sat.searcher.reader().is_current();
        self.manager.release(sat)?;
        current
    }

    /// Equivalent to the package-private `isTaxonomyCurrent()`.
    // Public for the port of `TestSearcherTaxonomyManager` (Java package-private).
    pub fn is_taxonomy_current(&self) -> Result<bool> {
        let sat = self.manager.acquire()?;
        let current = sat
            .taxonomy_reader
            .get_internal_index_reader()
            .and_then(|r| r.is_current());
        self.manager.release(sat)?;
        current
    }
}

impl std::ops::Deref for SearcherTaxonomyManager {
    type Target = ReferenceManager<SearcherAndTaxonomy>;

    fn deref(&self) -> &ReferenceManager<SearcherAndTaxonomy> {
        &self.manager
    }
}

#[allow(dead_code)]
fn _reader_type(_: &dyn IndexReader) {}
