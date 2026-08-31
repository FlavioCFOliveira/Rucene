//! Searcher sharing, ported from
//! `org.apache.lucene.search.SearcherManager`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::directory_reader::{open, open_if_changed, open_if_changed_with_commit};
use crate::index::{DirectoryReader, IndexReader};
use crate::search::index_searcher::IndexSearcher;
use crate::search::reference_manager::{ManagedReference, ReferenceManager, RefreshSource};
use crate::search::refresh_commit_supplier::{LatestCommitSupplier, RefreshCommitSupplier};
use crate::search::searcher_factory::{DefaultSearcherFactory, SearcherFactory};
use crate::store::Directory;

/// The reference a [`SearcherManager`] manages: an [`IndexSearcher`] together
/// with the [`DirectoryReader`] it searches.
///
/// **Divergence from Lucene 10.5.0.** Java manages the `IndexSearcher` itself
/// and recovers the reader with `(DirectoryReader) searcher.getIndexReader()`,
/// a downcast Rust cannot perform on a trait object. The typed handle is
/// therefore kept beside the searcher; it is the very same reader the searcher
/// holds, so the ref counting, the refresh and the results are unchanged.
#[derive(Debug)]
pub struct ManagedSearcher {
    searcher: IndexSearcher,
    reader: Arc<dyn DirectoryReader>,
}

impl ManagedSearcher {
    /// Returns the searcher.
    ///
    /// Equivalent to the `IndexSearcher` a Java `SearcherManager` hands out.
    pub fn searcher(&self) -> &IndexSearcher {
        &self.searcher
    }

    /// Returns the reader the searcher searches.
    ///
    /// Equivalent to `(DirectoryReader) searcher.getIndexReader()`.
    pub fn reader(&self) -> &Arc<dyn DirectoryReader> {
        &self.reader
    }
}

impl ManagedReference for ManagedSearcher {
    fn release_ref(&self) -> Result<()> {
        self.reader.dec_ref()
    }

    fn try_acquire_ref(&self) -> bool {
        self.reader.try_inc_ref()
    }

    fn ref_count(&self) -> i32 {
        self.reader.get_ref_count()
    }
}

/// The refresh strategy of a [`SearcherManager`].
///
/// Equivalent to `SearcherManager.refreshIfNeeded(IndexSearcher)`, which asks
/// the [`RefreshCommitSupplier`] which commit to open and then calls
/// `DirectoryReader.openIfChanged`.
#[derive(Debug)]
struct SearcherRefreshSource {
    searcher_factory: Arc<dyn SearcherFactory>,
    refresh_commit_supplier: Arc<dyn RefreshCommitSupplier>,
}

impl RefreshSource<ManagedSearcher> for SearcherRefreshSource {
    fn refresh_if_needed(
        &self,
        current: &Arc<ManagedSearcher>,
    ) -> Result<Option<Arc<ManagedSearcher>>> {
        let dr = Arc::clone(&current.reader);
        let refresh_commit = self
            .refresh_commit_supplier
            .get_searcher_refresh_commit(&dr)?;
        let new_reader = match refresh_commit {
            None => open_if_changed(dr)?,
            Some(commit) => open_if_changed_with_commit(dr, commit.as_ref())?,
        };
        match new_reader {
            None => Ok(None),
            Some(new_reader) => Ok(Some(Arc::new(get_searcher(
                self.searcher_factory.as_ref(),
                new_reader,
                Some(Arc::clone(&current.reader)),
            )?))),
        }
    }
}

/// Safely shares [`IndexSearcher`] instances across threads while periodically
/// reopening them, closing each searcher only once every thread has finished
/// using it.
///
/// Equivalent to the `final` class
/// `org.apache.lucene.search.SearcherManager`, which extends
/// `ReferenceManager<IndexSearcher>`. As with
/// [`ReaderManager`](crate::index::ReaderManager), the port expresses the
/// subclass as a type alias over the generic manager plus an inherent
/// implementation carrying the constructors; the managed reference is a
/// [`ManagedSearcher`].
///
/// Use [`ReferenceManager::acquire`] to obtain the current searcher and
/// [`ReferenceManager::release`] to release it. In addition, periodically call
/// [`ReferenceManager::maybe_refresh`] — ideally from a background thread
/// rather than just before each query, so that unlucky queries are not
/// penalised — and [`ReferenceManager::close`] once you are done.
pub type SearcherManager = ReferenceManager<ManagedSearcher>;

impl SearcherManager {
    /// Creates a manager from a [`Directory`].
    ///
    /// Equivalent to `SearcherManager(Directory, SearcherFactory)`; pass `None`
    /// for the factory to get the default one, as Java's `null` does.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the initial reader.
    pub fn from_directory(
        directory: Arc<dyn Directory>,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
    ) -> Result<Self> {
        let reader = open(directory)?;
        Self::build(reader, searcher_factory, None)
    }

    /// Creates a manager from an already-opened [`DirectoryReader`], stealing
    /// the incoming reference.
    ///
    /// Equivalent to
    /// `SearcherManager(DirectoryReader, SearcherFactory, RefreshCommitSupplier)`,
    /// which also covers the two-argument constructor when the supplier is
    /// `None`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the initial searcher.
    pub fn from_reader(
        reader: Arc<dyn DirectoryReader>,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
        refresh_commit_supplier: Option<Arc<dyn RefreshCommitSupplier>>,
    ) -> Result<Self> {
        Self::build(reader, searcher_factory, refresh_commit_supplier)
    }

    /// Creates a manager from an [`IndexWriter`](crate::index::DirectoryReaderIndexWriter),
    /// controlling whether past deletions are applied.
    ///
    /// Equivalent to
    /// `SearcherManager(IndexWriter, boolean, boolean, SearcherFactory, RefreshCommitSupplier)`,
    /// which also covers the shorter writer constructors: the two-argument one
    /// passes `apply_all_deletes = true` and `write_all_deletes = false`.
    ///
    /// * `apply_all_deletes` — if true, all buffered deletes are made visible
    ///   in the searcher; if false they may remain buffered in the writer, so
    ///   deleted documents may still be returned, which can be faster;
    /// * `write_all_deletes` — if true, new deletes are forcefully written to
    ///   the index files.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the near-real-time reader.
    pub fn from_writer(
        writer: &dyn crate::index::DirectoryReaderIndexWriter,
        apply_all_deletes: bool,
        write_all_deletes: bool,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
        refresh_commit_supplier: Option<Arc<dyn RefreshCommitSupplier>>,
    ) -> Result<Self> {
        let reader = writer.get_reader(apply_all_deletes, write_all_deletes)?;
        Self::build(reader, searcher_factory, refresh_commit_supplier)
    }

    fn build(
        reader: Arc<dyn DirectoryReader>,
        searcher_factory: Option<Arc<dyn SearcherFactory>>,
        refresh_commit_supplier: Option<Arc<dyn RefreshCommitSupplier>>,
    ) -> Result<Self> {
        let searcher_factory: Arc<dyn SearcherFactory> =
            searcher_factory.unwrap_or_else(|| Arc::new(DefaultSearcherFactory));
        let refresh_commit_supplier: Arc<dyn RefreshCommitSupplier> =
            refresh_commit_supplier.unwrap_or_else(|| Arc::new(LatestCommitSupplier));
        let current = Arc::new(get_searcher(searcher_factory.as_ref(), reader, None)?);
        Ok(ReferenceManager::new(
            current,
            Arc::new(SearcherRefreshSource {
                searcher_factory,
                refresh_commit_supplier,
            }),
        ))
    }

    /// Returns the index commit generation of the current searcher.
    ///
    /// Equivalent to the package-private
    /// `SearcherManager.getSearcherCommitGeneration()`, which exists for
    /// testing; it is `pub` here because the port has no package visibility.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the commit.
    pub fn get_searcher_commit_generation(&self) -> Result<i64> {
        let managed = self.acquire()?;
        let generation = managed.reader.index_commit()?.get_generation();
        let result = self.release(managed);
        result?;
        Ok(generation)
    }

    /// Returns `true` when no change has occurred since the current searcher's
    /// reader was opened.
    ///
    /// Equivalent to the package-private
    /// `SearcherManager.isSearcherCurrent()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while checking the reader.
    pub fn is_searcher_current(&self) -> Result<bool> {
        let managed = self.acquire()?;
        let current = managed.reader.is_current();
        self.release(managed)?;
        current
    }
}

/// Expert: creates a searcher from the provided reader using the provided
/// factory.
///
/// Equivalent to the static
/// `SearcherManager.getSearcher(SearcherFactory, IndexReader, IndexReader)`,
/// which decrements the incoming reader's reference count when it throws.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] — with Java's message — when the
/// factory does not wrap exactly the reader it was given, and propagates any
/// I/O error the factory raises.
pub fn get_searcher(
    searcher_factory: &dyn SearcherFactory,
    reader: Arc<dyn DirectoryReader>,
    previous_reader: Option<Arc<dyn DirectoryReader>>,
) -> Result<ManagedSearcher> {
    let index_reader: Arc<dyn IndexReader> = Arc::clone(&reader).get_context().reader();
    let previous_index_reader =
        previous_reader.map(|previous| Arc::clone(&previous).get_context().reader());
    let outcome = searcher_factory.new_searcher(Arc::clone(&index_reader), previous_index_reader);
    let searcher = match outcome {
        Err(error) => {
            reader.dec_ref()?;
            return Err(error);
        }
        Ok(searcher) => searcher,
    };
    if !Arc::ptr_eq(searcher.get_index_reader(), &index_reader) {
        reader.dec_ref()?;
        return Err(LuceneError::IllegalState(format!(
            "SearcherFactory must wrap exactly the provided reader (got {:?} but expected {:?})",
            searcher.get_index_reader(),
            index_reader
        )));
    }
    Ok(ManagedSearcher { searcher, reader })
}
