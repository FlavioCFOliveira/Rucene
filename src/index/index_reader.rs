//! Top-level reader abstractions ported from `org.apache.lucene.index`.
//!
//! This module provides [`IndexReader`], the abstract point-in-time view of an
//! index, together with its lifecycle helpers ([`IndexReaderCore`]), the
//! [`StoredFields`] API, cache helpers, and the [`CompositeReader`] contract
//! for readers composed of sub-readers.

#![deny(unsafe_code)]

use std::{
    fmt::{Debug, Formatter},
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc, Mutex, Weak,
    },
};

use crate::error::{LuceneError, Result};
use crate::index::reader_context::{CompositeReaderContext, IndexReaderContext, LeafReaderContext};
use crate::index::TermVectors;

// ---------------------------------------------------------------------------
// Cache helpers
// ---------------------------------------------------------------------------

/// Opaque cache key identifying a resource that can be cached on.
///
/// Equivalent to `org.apache.lucene.index.IndexReader.CacheKey`. Identity
/// (`==`) is the only meaningful comparison.
#[derive(Debug, Default, Clone)]
pub struct CacheKey;

/// Listener invoked when a resource guarded by a [`CacheKey`] is closed.
///
/// Equivalent to `org.apache.lucene.index.IndexReader.ClosedListener`.
pub trait ClosedListener: Send + Sync {
    /// Called when the guarded resource is closed.
    fn on_close(&self, key: CacheKey) -> Result<()>;
}

/// Helper used to build caches keyed on the data contained in a reader.
///
/// Equivalent to `org.apache.lucene.index.IndexReader.CacheHelper`.
pub trait CacheHelper: Send + Sync + Debug {
    /// Returns the cache key for this resource.
    fn get_key(&self) -> &CacheKey;

    /// Registers a listener to be called when the resource is closed.
    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>);
}

// ---------------------------------------------------------------------------
// StoredFields
// ---------------------------------------------------------------------------

/// API for reading stored fields.
///
/// Equivalent to `org.apache.lucene.index.StoredFields`. Instances are not
/// thread-safe and should be used by a single thread.
pub trait StoredFields: Send + Sync + Debug {
    /// Optional prefetch hint for the given document.
    ///
    /// Default implementation is a no-op.
    fn prefetch(&mut self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    /// Visits the stored fields of `doc_id` using the supplied visitor.
    ///
    /// Implementations read the stored values and call the matching visitor
    /// callbacks.
    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn crate::codecs::stub::StoredFieldVisitor,
    ) -> Result<()>;

    /// Returns the stored fields of `doc_id` as a [`Document`](crate::document::Document).
    fn document(&self, doc_id: i32) -> Result<crate::document::Document>;

    /// Like [`Self::document`], but only loads the named fields.
    fn document_fields(
        &self,
        doc_id: i32,
        fields_to_load: &std::collections::HashSet<String>,
    ) -> Result<crate::document::Document>;
}

// ---------------------------------------------------------------------------
// IndexReaderCore
// ---------------------------------------------------------------------------

/// Shared state for all [`IndexReader`] implementations.
///
/// Equivalent to the private fields of `org.apache.lucene.index.IndexReader`
/// (`refCount`, `closed`, `parentReaders`, etc.).
pub struct IndexReaderCore {
    closed: AtomicBool,
    closed_by_child: AtomicBool,
    ref_count: AtomicI32,
    parent_readers: Mutex<Vec<Weak<dyn IndexReader>>>,
}

impl Default for IndexReaderCore {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for IndexReaderCore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexReaderCore")
            .field("ref_count", &self.ref_count.load(Ordering::SeqCst))
            .field("closed", &self.closed.load(Ordering::SeqCst))
            .finish()
    }
}

impl IndexReaderCore {
    /// Creates a fresh core with a reference count of one.
    pub fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            closed_by_child: AtomicBool::new(false),
            ref_count: AtomicI32::new(1),
            parent_readers: Mutex::new(Vec::new()),
        }
    }

    /// Returns the current reference count.
    pub fn ref_count(&self) -> i32 {
        self.ref_count.load(Ordering::SeqCst)
    }

    /// Increments the reference count, returning `false` if the reader is closed.
    pub fn try_inc_ref(&self) -> bool {
        let mut count = self.ref_count.load(Ordering::SeqCst);
        while count > 0 {
            match self.ref_count.compare_exchange_weak(
                count,
                count + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => count = actual,
            }
        }
        false
    }

    /// Increments the reference count, or returns an error if the reader is closed.
    pub fn inc_ref(&self) -> Result<()> {
        if !self.try_inc_ref() {
            return Err(LuceneError::AlreadyClosed(
                "this IndexReader is closed".to_string(),
            ));
        }
        Ok(())
    }

    /// Decrements the reference count, invoking `on_zero` exactly once when the
    /// count reaches zero (before reporting close to parent readers).
    ///
    /// This is the core of `IndexReader::dec_ref`, factored out so that readers
    /// held as `dyn LeafReader` (which cannot be upcast to `dyn IndexReader`) can
    /// still be reference-counted through their `IndexReaderCore`. `on_zero` is
    /// expected to perform the reader-specific `do_close` work and return its
    /// result, which is propagated after parent readers are notified.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the count is already zero, and
    /// [`LuceneError::IllegalState`] if it would drop below zero.
    pub fn dec_ref_with_close<F>(&self, on_zero: F) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        if self.ref_count.load(Ordering::SeqCst) <= 0 {
            return Err(LuceneError::AlreadyClosed(
                "this IndexReader is closed".to_string(),
            ));
        }
        let rc = self.ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc == 0 {
            self.set_closed();
            let close_result = on_zero();
            self.report_close_to_parent_readers();
            close_result
        } else if rc < 0 {
            Err(LuceneError::IllegalState(format!(
                "too many decRef calls: refCount is {rc} after decrement"
            )))
        } else {
            Ok(())
        }
    }

    /// Throws an error if this reader or any of its child readers is closed.
    pub fn ensure_open(&self) -> Result<()> {
        if self.ref_count.load(Ordering::SeqCst) <= 0 {
            return Err(LuceneError::AlreadyClosed(
                "this IndexReader is closed".to_string(),
            ));
        }
        if self.closed_by_child.load(Ordering::SeqCst) {
            return Err(LuceneError::AlreadyClosed(
                "this IndexReader cannot be used anymore as one of its child readers was closed"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Registers a parent reader that should be marked closed when this reader
    /// closes.
    pub fn register_parent_reader(&self, parent: Arc<dyn IndexReader>) {
        if let Ok(mut parents) = self.parent_readers.lock() {
            parents.push(Arc::downgrade(&parent));
        }
    }

    /// Marks registered parents as closed by child and recurses.
    pub fn report_close_to_parent_readers(&self) {
        if let Ok(parents) = self.parent_readers.lock() {
            for parent in parents.iter() {
                if let Some(parent) = parent.upgrade() {
                    parent.core().closed_by_child.store(true, Ordering::SeqCst);
                    parent.core().report_close_to_parent_readers();
                }
            }
        }
    }

    /// Sets the closed flag.
    pub fn set_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Returns `true` after this reader has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// IndexReader
// ---------------------------------------------------------------------------

/// Abstract point-in-time view of an index.
///
/// Equivalent to `org.apache.lucene.index.IndexReader`. Search and all
/// high-level access to the index go through this trait.
///
/// Concrete reader types do not implement this trait directly. Leaf readers
/// implement [`LeafReader`](crate::index::LeafReader) and get an automatic
/// `IndexReader` implementation through a blanket impl. Composite readers
/// implement [`CompositeReader`] (which extends `IndexReader`) and must provide
/// an explicit `IndexReader` implementation, typically overriding
/// [`IndexReader::build_context`] to build a [`CompositeReaderContext`].
pub trait IndexReader: Send + Sync + Debug + 'static {
    /// Returns the shared core state for this reader.
    fn core(&self) -> &IndexReaderCore;

    /// Returns a [`TermVectors`] reader for this index.
    fn term_vectors(&self) -> Result<Box<dyn TermVectors>>;

    /// Returns the number of live documents.
    fn num_docs(&self) -> i32;

    /// Returns one greater than the largest possible document number.
    fn max_doc(&self) -> i32;

    /// Returns a [`StoredFields`] reader for this index.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>>;

    /// Implements close logic (called once the reference count reaches zero).
    fn do_close(&self) -> Result<()>;

    /// Returns an optional cache helper for this reader.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>>;

    /// Returns the number of documents containing `term`.
    fn doc_freq(&self, term: &crate::index::Term) -> Result<i32>;

    /// Returns the total occurrence count of `term`.
    fn total_term_freq(&self, term: &crate::index::Term) -> Result<i64>;

    /// Returns the sum of [`TermsEnum::doc_freq`](crate::index::TermsEnum::doc_freq)
    /// for all terms in `field`.
    fn get_sum_doc_freq(&self, field: &str) -> Result<i64>;

    /// Returns the number of documents that have at least one term in `field`.
    fn get_doc_count(&self, field: &str) -> Result<i32>;

    /// Returns the sum of total term frequencies for all terms in `field`.
    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64>;

    /// Hook invoked after this reader is closed. Default is a no-op.
    fn notify_reader_closed_listeners(&self) -> Result<()> {
        Ok(())
    }

    /// Returns the current reference count.
    fn get_ref_count(&self) -> i32 {
        self.core().ref_count()
    }

    /// Increments the reference count if the reader is still open.
    fn try_inc_ref(&self) -> bool {
        self.core().try_inc_ref()
    }

    /// Increments the reference count, failing if the reader is closed.
    fn inc_ref(&self) -> Result<()> {
        self.core().inc_ref()
    }

    /// Decrements the reference count, closing the reader when it reaches zero.
    fn dec_ref(&self) -> Result<()> {
        if self.core().ref_count() <= 0 {
            return Err(LuceneError::AlreadyClosed(
                "this IndexReader is closed".to_string(),
            ));
        }
        let rc = self.core().ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc == 0 {
            self.core().set_closed();
            let close_result = self.do_close();
            self.core().report_close_to_parent_readers();
            let _ = self.notify_reader_closed_listeners();
            close_result
        } else if rc < 0 {
            Err(LuceneError::IllegalState(format!(
                "too many decRef calls: refCount is {rc} after decrement"
            )))
        } else {
            Ok(())
        }
    }

    /// Ensures this reader is still open.
    fn ensure_open(&self) -> Result<()> {
        self.core().ensure_open()
    }

    /// Registers a parent reader to be closed when this reader closes.
    fn register_parent_reader(&self, parent: Arc<dyn IndexReader>) {
        self.core().register_parent_reader(parent);
    }

    /// Returns the number of deleted documents.
    fn num_deleted_docs(&self) -> i32 {
        self.max_doc() - self.num_docs()
    }

    /// Returns `true` if any documents have been deleted.
    fn has_deletions(&self) -> bool {
        self.num_deleted_docs() > 0
    }

    /// Closes files and other resources associated with this reader.
    fn close(&self) -> Result<()> {
        if !self.core().is_closed() {
            self.dec_ref()?;
            self.core().set_closed();
        }
        Ok(())
    }

    /// Builds a context node for this reader at the given place in the reader
    /// tree.
    ///
    /// Leaf readers produce a [`LeafReaderContext`]; composite readers override
    /// this to produce a [`CompositeReaderContext`].
    fn build_context(
        self: Arc<Self>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext>;

    /// Returns the root context for this reader's sub-reader tree.
    fn get_context(self: Arc<Self>) -> Arc<dyn IndexReaderContext> {
        self.build_context(None, 0, 0, 0, 0)
    }

    /// Convenience method returning the top-level leaves of this reader.
    fn leaves(self: Arc<Self>) -> Vec<Arc<LeafReaderContext>> {
        self.get_context().leaves()
    }
}

// ---------------------------------------------------------------------------
// CompositeReader
// ---------------------------------------------------------------------------

/// A reader composed of sequential sub-readers.
///
/// Equivalent to `org.apache.lucene.index.CompositeReader`. Direct postings
/// access is not supported; callers iterate over the leaf contexts returned by
/// [`IndexReader::leaves`].
///
/// Types implementing this trait must also implement [`IndexReader`], and
/// typically override [`IndexReader::build_context`] to call
/// [`CompositeReaderContext::create`].
pub trait CompositeReader: IndexReader {
    /// Returns the direct sub-readers that this reader is composed of.
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>>;
}

/// Helper that builds a [`CompositeReaderContext`] for a composite reader.
///
/// This is intended to be used inside an [`IndexReader::build_context`]
/// override for a concrete composite reader type.
pub fn build_composite_context(
    reader: Arc<dyn CompositeReader>,
    parent: Option<Weak<dyn IndexReaderContext>>,
    ord_in_parent: i32,
    doc_base_in_parent: i32,
    leaf_ord: i32,
    leaf_doc_base: i32,
) -> Arc<dyn IndexReaderContext> {
    CompositeReaderContext::create(
        reader,
        parent,
        ord_in_parent,
        doc_base_in_parent,
        leaf_ord,
        leaf_doc_base,
    )
}
