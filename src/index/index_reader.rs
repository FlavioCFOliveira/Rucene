//! Top-level reader abstractions ported from `org.apache.lucene.index`.
//!
//! This module provides [`IndexReader`], the abstract point-in-time view of an
//! index, together with its lifecycle helpers ([`IndexReaderCore`]), the
//! [`StoredFields`] API, cache helpers, and the [`CompositeReader`] contract
//! for readers composed of sub-readers.

#![deny(unsafe_code)]

use std::{
    fmt::{Debug, Display, Formatter},
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc, Mutex, PoisonError, Weak,
    },
};

use crate::error::{LuceneError, Result};
use crate::index::reader_context::{CompositeReaderContext, IndexReaderContext, LeafReaderContext};
use crate::index::TermVectors;

// ---------------------------------------------------------------------------
// Cache helpers
// ---------------------------------------------------------------------------

/// A cache key identifying a resource that is being cached on.
///
/// Equivalent to `org.apache.lucene.index.IndexReader.CacheKey`
/// (`IndexReader.java:131-134`), a `final` class with no state whose
/// instances are compared by reference: two keys are the same key only if
/// they are the same object.
///
/// The port holds that object behind an [`Arc`]: [`CacheKey::new`] allocates a
/// new object, [`Clone`] hands out another reference to the *same* object (a
/// Java reference copy), and [`PartialEq`]/[`Hash`] compare and hash the
/// object's identity, so a `HashMap<CacheKey, _>` is Java's
/// `IdentityHashMap<CacheKey, _>` (a Java `HashMap` keyed on a `CacheKey`
/// behaves the same, as the class overrides neither `equals` nor `hashCode`).
/// The identity cannot be reused while any clone is alive, as a referenced Java
/// object's identity cannot.
#[derive(Clone)]
pub struct CacheKey(Arc<CacheKeyObject>);

/// The object a [`CacheKey`] refers to. It is not empty, so that its address
/// lies strictly inside its own allocation rather than one past its end, where
/// another allocation may start.
struct CacheKeyObject {
    _non_empty: u8,
}

impl CacheKey {
    /// A new key, distinct from every other key.
    ///
    /// Equivalent to the package-private `CacheKey()` constructor ("only
    /// instantiable by core impls"); public so that the readers of other
    /// modules and crates that Java places in `org.apache.lucene.index`, or
    /// reaches through `IndexPackageAccess.newCacheKey()`, can create one.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Arc::new(CacheKeyObject { _non_empty: 0 }))
    }

    /// The address of the key object: the identity this key compares and
    /// hashes by, standing for Java's identity hash where a rendering needs one
    /// (`Object.toString()`).
    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

impl PartialEq for CacheKey {
    /// Reference equality, Java's `==` on two `CacheKey`s.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CacheKey {}

impl std::hash::Hash for CacheKey {
    /// The identity hash, consistent with [`PartialEq`].
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

impl Debug for CacheKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for CacheKey {
    /// `CacheKey` does not override `toString()`, so Java renders
    /// `Object.toString()`: the binary class name, `@`, and the identity hash
    /// in hexadecimal, for which the port substitutes the key's
    /// [`identity`](Self::identity) (see [`java_object_to_string`]).
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "org.apache.lucene.index.IndexReader$CacheKey@{:x}",
            self.identity()
        )
    }
}

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
    ///
    /// Equivalent to `StoredFields.document(int)`, which Java declares `final`
    /// because it is pure sugar over
    /// [`DocumentStoredFieldVisitor`](crate::document::DocumentStoredFieldVisitor).
    /// This port keeps it a provided method for the same reason: an
    /// implementation only has to supply [`Self::document_with_visitor`] and
    /// cannot diverge on how a document is materialised.
    ///
    /// **NOTE:** for performance reasons this method does not check whether the
    /// requested document is deleted; asking for a deleted document yields
    /// unspecified results. Test it against the live docs of the reader when
    /// that matters.
    ///
    /// **NOTE:** only the content of a field is returned, and only if the field
    /// was stored at indexing time. Metadata such as `omitNorms`,
    /// `IndexOptions` or `tokenized` is not preserved.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] if the index is corrupt, and
    /// propagates any low-level I/O error.
    fn document(&self, doc_id: i32) -> Result<crate::document::Document> {
        let mut visitor = crate::document::DocumentStoredFieldVisitor::new();
        self.document_with_visitor(doc_id, &mut visitor)?;
        Ok(visitor.into_document())
    }

    /// Like [`Self::document`], but only loads the named fields.
    ///
    /// Equivalent to `StoredFields.document(int, Set<String>)`, also `final` in
    /// Java and also pure sugar over
    /// [`DocumentStoredFieldVisitor`](crate::document::DocumentStoredFieldVisitor).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] if the index is corrupt, and
    /// propagates any low-level I/O error.
    fn document_fields(
        &self,
        doc_id: i32,
        fields_to_load: &std::collections::HashSet<String>,
    ) -> Result<crate::document::Document> {
        let mut visitor = crate::document::DocumentStoredFieldVisitor::with_fields(
            fields_to_load.iter().cloned(),
        );
        self.document_with_visitor(doc_id, &mut visitor)?;
        Ok(visitor.into_document())
    }
}

// ---------------------------------------------------------------------------
// IndexReaderCore
// ---------------------------------------------------------------------------

/// The part of a reader's close state that its children must be able to reach.
///
/// Java children keep `Set<IndexReader> parentReaders` and flip the parent's
/// `closedByChild` field directly. A Rust child cannot hold a reference to a
/// parent that may still be moved (a constructor returns the reader by value
/// before it is ever placed in an `Arc`), so the flag lives here, behind an
/// `Arc` whose address is stable from the moment the parent's
/// [`IndexReaderCore`] is created. Children register a `Weak` to it, which
/// keeps the parent collectable exactly like Java's weak-keyed
/// `parentReaders` set.
///
/// The reference count lives here too, so that an object the reader hands out
/// before it is complete — `SegmentReader`'s cache helpers, whose
/// `addClosedListener` calls the reader's `ensureOpen()`
/// (`SegmentReader.java:339-343, 363-367`) — can check the reader's state
/// through an [`EnsureOpen`] handle without holding the reader itself.
#[derive(Debug)]
struct ReaderLink {
    ref_count: AtomicI32,
    closed_by_child: AtomicBool,
    parents: Mutex<Vec<Weak<ReaderLink>>>,
}

impl ReaderLink {
    fn new() -> Self {
        Self {
            ref_count: AtomicI32::new(1),
            closed_by_child: AtomicBool::new(false),
            parents: Mutex::new(Vec::new()),
        }
    }

    /// The body of `IndexReader.ensureOpen()` (`IndexReader.java:270-280`).
    fn ensure_open(&self) -> Result<()> {
        if self.ref_count.load(Ordering::SeqCst) <= 0 {
            return Err(LuceneError::already_closed(
                "this IndexReader is closed".to_string(),
            ));
        }
        if self.closed_by_child.load(Ordering::SeqCst) {
            return Err(LuceneError::already_closed(
                "this IndexReader cannot be used anymore as one of its child readers was closed"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Marks every registered parent as closed-by-child, transitively.
    fn report_close_to_parents(&self) {
        // The lock is released before recursing so a cycle in the (weakly
        // linked) reader tree cannot deadlock on a re-entrant lock.
        let parents: Vec<Weak<ReaderLink>> = match self.parents.lock() {
            Ok(parents) => parents.clone(),
            Err(_) => return,
        };
        for parent in parents {
            if let Some(parent) = parent.upgrade() {
                if !parent.closed_by_child.swap(true, Ordering::SeqCst) {
                    parent.report_close_to_parents();
                }
            }
        }
    }
}

/// Shared state for all [`IndexReader`] implementations.
///
/// Equivalent to the private fields of `org.apache.lucene.index.IndexReader`
/// (`refCount`, `closed`, `parentReaders`, etc.).
pub struct IndexReaderCore {
    closed: AtomicBool,
    link: Arc<ReaderLink>,
    /// The top-level context [`IndexReader::get_context`] answers.
    ///
    /// Equivalent to the `readerContext` field of `LeafReader`
    /// (`LeafReader.java:51`) and of `CompositeReader`
    /// (`CompositeReader.java:49`), held weakly: see
    /// [`IndexReader::get_context`] for why.
    reader_context: Mutex<Option<Weak<dyn IndexReaderContext>>>,
}

/// A handle on a reader's open state that `ensureOpen()` can be called
/// through, for an object that belongs to the reader but cannot hold it.
///
/// Equivalent to the implicit `IndexReader.this` an anonymous inner class of a
/// reader captures to call `ensureOpen()` — as the two cache helpers of
/// `SegmentReader` do (`SegmentReader.java:339-343, 363-367`). A Rust reader is
/// built by value and placed in an `Arc` afterwards, so an object built
/// alongside it takes this handle instead, from
/// [`IndexReaderCore::ensure_open_handle`]. It holds the reader's state
/// strongly, which keeps the answer exact after the reader is dropped: a
/// dropped reader has a reference count that never reached zero only if it was
/// never closed, which is also what Java's would report.
#[derive(Debug, Clone)]
pub struct EnsureOpen(Arc<ReaderLink>);

impl EnsureOpen {
    /// `ensureOpen()` on the reader this handle was taken from.
    ///
    /// # Errors
    ///
    /// As [`IndexReaderCore::ensure_open`].
    pub fn ensure_open(&self) -> Result<()> {
        self.0.ensure_open()
    }

    /// As [`IndexReaderCore::ensure_open_unchecked`].
    ///
    /// # Panics
    ///
    /// As [`IndexReaderCore::ensure_open_unchecked`].
    pub fn ensure_open_unchecked(&self) {
        throw_if_closed(self.0.ensure_open());
    }
}

/// Throws the `AlreadyClosedException` of a failed `ensureOpen()` from a
/// signature that cannot fail.
fn throw_if_closed(result: Result<()>) {
    if let Err(LuceneError::AlreadyClosed { message, .. }) = result {
        crate::store::java_nio_file::throw_unchecked(
            "org.apache.lucene.store.AlreadyClosedException",
            Some(&message),
        )
    }
}

/// Whether `context` was built for the reader at `reader`.
///
/// A reader whose [`IndexReader::core`] is another reader's core would
/// otherwise be answered that reader's context.
fn context_reader_is(context: &Arc<dyn IndexReaderContext>, reader: *const ()) -> bool {
    std::ptr::eq(Arc::as_ptr(&context.reader()).cast::<()>(), reader)
}

impl Default for IndexReaderCore {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for IndexReaderCore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexReaderCore")
            .field("ref_count", &self.link.ref_count.load(Ordering::SeqCst))
            .field("closed", &self.closed.load(Ordering::SeqCst))
            .finish()
    }
}

impl IndexReaderCore {
    /// Creates a fresh core with a reference count of one.
    pub fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            link: Arc::new(ReaderLink::new()),
            reader_context: Mutex::new(None),
        }
    }

    /// The cached top-level context of the reader at `reader`, if it is still
    /// alive and was built for that reader.
    fn cached_reader_context(&self, reader: *const ()) -> Option<Arc<dyn IndexReaderContext>> {
        self.reader_context
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
            .filter(|context| context_reader_is(context, reader))
    }

    /// Caches `context`, the top-level context just built for the reader at
    /// `reader`, unless another thread cached a live one for that reader
    /// first; returns the cached context.
    fn cache_reader_context(
        &self,
        reader: *const (),
        context: Arc<dyn IndexReaderContext>,
    ) -> Arc<dyn IndexReaderContext> {
        let mut cached = self
            .reader_context
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = cached
            .as_ref()
            .and_then(Weak::upgrade)
            .filter(|existing| context_reader_is(existing, reader))
        {
            return existing;
        }
        *cached = Some(Arc::downgrade(&context));
        context
    }

    /// Returns the current reference count.
    pub fn ref_count(&self) -> i32 {
        self.link.ref_count.load(Ordering::SeqCst)
    }

    /// Increments the reference count, returning `false` if the reader is closed.
    pub fn try_inc_ref(&self) -> bool {
        let mut count = self.link.ref_count.load(Ordering::SeqCst);
        while count > 0 {
            match self.link.ref_count.compare_exchange_weak(
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
            return Err(LuceneError::already_closed(
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
        if self.link.ref_count.load(Ordering::SeqCst) <= 0 {
            return Err(LuceneError::already_closed(
                "this IndexReader is closed".to_string(),
            ));
        }
        let rc = self.link.ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc == 0 {
            self.set_closed();
            let close_result = on_zero();
            self.report_close_to_parent_readers();
            close_result
        } else if rc < 0 {
            Err(LuceneError::illegal_state(format!(
                "too many decRef calls: refCount is {rc} after decrement"
            )))
        } else {
            Ok(())
        }
    }

    /// Throws an error if this reader or any of its child readers is closed.
    pub fn ensure_open(&self) -> Result<()> {
        self.link.ensure_open()
    }

    /// Returns a handle `ensureOpen()` can be called through by an object
    /// that belongs to this reader but cannot hold it; see [`EnsureOpen`].
    pub fn ensure_open_handle(&self) -> EnsureOpen {
        EnsureOpen(Arc::clone(&self.link))
    }

    /// Registers a parent reader that should be marked closed when this reader
    /// closes.
    ///
    /// Equivalent to `IndexReader.registerParentReader(IndexReader)`. Only a
    /// weak link is kept, so registration never keeps the parent alive.
    ///
    /// # Errors
    ///
    /// [`LuceneError::AlreadyClosed`] from the `ensureOpen()` Java calls on
    /// this (child) reader first (`IndexReader.java:162-165`).
    pub fn register_parent_reader(&self, parent: Arc<dyn IndexReader>) -> Result<()> {
        self.register_parent_core(parent.core())
    }

    /// Registers a parent by its [`IndexReaderCore`], for composite readers
    /// that must register themselves inside their own constructor — before an
    /// `Arc<dyn IndexReader>` for `self` could possibly exist.
    ///
    /// This is what `BaseCompositeReader`'s constructor does in Java
    /// (`r.registerParentReader(this)`, `BaseCompositeReader.java:86`); `this`
    /// is already a usable reference there, whereas a Rust constructor returns
    /// the reader by value. The link is taken against the parent core's
    /// internal, heap-allocated state, so it stays valid when the parent is
    /// subsequently moved into an `Arc` (or anywhere else).
    ///
    /// # Errors
    ///
    /// [`LuceneError::AlreadyClosed`] when this (child) reader is closed:
    /// `registerParentReader` calls `ensureOpen()` before it records the
    /// parent (`IndexReader.java:162-165`).
    pub fn register_parent_core(&self, parent: &IndexReaderCore) -> Result<()> {
        self.ensure_open()?;
        if let Ok(mut parents) = self.link.parents.lock() {
            parents.push(Arc::downgrade(&parent.link));
        }
        Ok(())
    }

    /// `ensureOpen()` inside a member whose Rust signature cannot fail
    /// (`getLiveDocs()`, `getFieldInfos()`, `getMetaData()`).
    ///
    /// Java's `AlreadyClosedException` is unchecked, so those members throw it
    /// although they declare nothing. The port throws it the same way: a panic
    /// whose payload is the Java exception's `toString()`
    /// (`org.apache.lucene.store.AlreadyClosedException: <message>`), the form
    /// [`throw_unchecked`](crate::store::java_nio_file::throw_unchecked)
    /// gives every unchecked Java exception the port throws from an
    /// infallible signature.
    ///
    /// # Panics
    ///
    /// When [`ensure_open`](Self::ensure_open) fails, with its message.
    pub fn ensure_open_unchecked(&self) {
        throw_if_closed(self.ensure_open());
    }

    /// Marks registered parents as closed by child and recurses.
    ///
    /// Equivalent to `IndexReader.reportCloseToParentReaders()`.
    pub fn report_close_to_parent_readers(&self) {
        self.link.report_close_to_parents();
    }

    /// Returns `true` once one of this reader's children has been closed.
    ///
    /// Equivalent to reading `IndexReader.closedByChild`.
    pub fn is_closed_by_child(&self) -> bool {
        self.link.closed_by_child.load(Ordering::SeqCst)
    }

    /// Sets the closed flag.
    pub fn set_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Returns `true` after this reader has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Returns a value that identifies the reader owning this core, and only
    /// that reader, for as long as it lives.
    ///
    /// The core's internal link is heap-allocated once, when the core is
    /// created, and never moves afterwards, so its address is the reader's
    /// identity. This is what stands in for Java's
    /// `System.identityHashCode(this)`, which
    /// [`IndexReader`]'s `hashCode` returns; Rust has no address that a value
    /// keeps across moves, so the identity is taken from the one part of the
    /// reader that is behind an `Arc`.
    pub fn identity_hash_code(&self) -> usize {
        Arc::as_ptr(&self.link) as *const () as usize
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
pub trait IndexReader: Send + Sync + Debug + Display + 'static {
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
    ///
    /// Equivalent to `IndexReader.decRef()` (`IndexReader.java:246-264`), whose
    /// body is `doClose()` inside a try-with-resources declaring
    /// `reportCloseToParentReaders` and then `notifyReaderClosedListeners` —
    /// and resources close in *reverse* declaration order, so the listeners run
    /// before the parents are told. An exception from `doClose()` wins; if it
    /// succeeded, the first finalizer's exception propagates.
    fn dec_ref(&self) -> Result<()> {
        if self.core().ref_count() <= 0 {
            return Err(LuceneError::already_closed(
                "this IndexReader is closed".to_string(),
            ));
        }
        let rc = self.core().link.ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc == 0 {
            self.core().set_closed();
            let close_result = self.do_close();
            let notify_result = self.notify_reader_closed_listeners();
            self.core().report_close_to_parent_readers();
            close_result.and(notify_result)
        } else if rc < 0 {
            Err(LuceneError::illegal_state(format!(
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
    ///
    /// Equivalent to `IndexReader.registerParentReader(IndexReader)`.
    ///
    /// # Errors
    ///
    /// [`LuceneError::AlreadyClosed`] when this reader is closed
    /// (`IndexReader.java:162-165`).
    fn register_parent_reader(&self, parent: Arc<dyn IndexReader>) -> Result<()> {
        self.core().register_parent_reader(parent)
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
    ///
    /// Equivalent to the abstract `IndexReader.getContext()` and to the two
    /// implementations of it: `LeafReader.getContext()`, which Java declares
    /// `final` and answers with the `LeafReaderContext` the reader built for
    /// itself, and `CompositeReader.getContext()`, which builds a
    /// `CompositeReaderContext` over the sub-readers and caches it. Both are
    /// answered here by [`build_context`](Self::build_context), which each
    /// reader implements — so the split is between the two `build_context`
    /// bodies rather than between two `getContext` overrides. The context is
    /// built once and answered again on every later call, as both Java
    /// implementations do (`LeafReader.java:51, 59-62`,
    /// `CompositeReader.java:49, 93-102`): `reader.leaves()` hands out the
    /// leaf contexts an [`IndexSearcher`](crate::search::IndexSearcher) over
    /// the same reader holds.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's reader holds its context
    /// strongly, and the context holds the reader: a cycle Java's garbage
    /// collector reclaims. Reference counting cannot, so the reader caches its
    /// context weakly ([`IndexReaderCore`]'s `reader_context`). While anything
    /// holds the top-level context — a searcher, a caller — every call answers
    /// that same context, as in Java; once nothing does, the next call builds a
    /// new one, with a new [`IndexReaderContext::id`]. Holding a leaf context
    /// alone does not keep the top-level one alive, because a leaf refers to
    /// its parent weakly. Caching strongly instead would leak every reader
    /// whose context was ever asked for.
    ///
    /// # Errors
    ///
    /// [`LuceneError::AlreadyClosed`] from `ensureOpen()`, which both Java
    /// implementations call first (`CompositeReader.java:93-94`,
    /// `LeafReader.java:59-60`): `this IndexReader is closed` when this reader
    /// is closed, and `this IndexReader cannot be used anymore as one of its
    /// child readers was closed` when a child reader this one was registered
    /// with as a parent was closed (`IndexReader.java:170-182, 268-280`).
    fn get_context(self: Arc<Self>) -> Result<Arc<dyn IndexReaderContext>> {
        self.ensure_open()?;
        let reader = Arc::as_ptr(&self).cast::<()>();
        if let Some(context) = self.core().cached_reader_context(reader) {
            return Ok(context);
        }
        let context = Arc::clone(&self).build_context(None, 0, 0, 0, 0);
        Ok(self.core().cache_reader_context(reader, context))
    }

    /// Convenience method returning the top-level leaves of this reader.
    ///
    /// Equivalent to `IndexReader.leaves()`, which Java declares `final` and
    /// implements as `getContext().leaves()` (`IndexReader.java:417-419`).
    ///
    /// # Errors
    ///
    /// As [`get_context`](Self::get_context): a closed reader fails with
    /// [`LuceneError::AlreadyClosed`].
    fn leaves(self: Arc<Self>) -> Result<Vec<Arc<LeafReaderContext>>> {
        Ok(self.get_context()?.leaves())
    }
}

impl PartialEq for dyn IndexReader {
    /// Two readers are equal only when they are the same reader.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexReader#equals`, which Java
    /// declares `final` — subclasses are not allowed to define reader equality
    /// — and implements as `this == obj`. The identity here is
    /// [`IndexReaderCore::identity_hash_code`], which stands in for the object
    /// reference Java compares.
    fn eq(&self, other: &Self) -> bool {
        self.core().identity_hash_code() == other.core().identity_hash_code()
    }
}

impl Eq for dyn IndexReader {}

impl std::hash::Hash for dyn IndexReader {
    /// Hashes the reader's identity, so that a reader is only ever found by
    /// itself in a hash-based collection.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexReader#hashCode`, also
    /// `final` in Java, which returns `System.identityHashCode(this)`.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.core().identity_hash_code().hash(state);
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

// ---------------------------------------------------------------------------
// toString helpers shared by the reader hierarchy
// ---------------------------------------------------------------------------

/// Renders a composite reader exactly as `CompositeReader.toString()` does:
/// the reader's own simple name, then its sequential sub-readers separated by a
/// single space, inside one pair of parentheses.
///
/// Equivalent to `org.apache.lucene.index.CompositeReader.toString()`
/// (`CompositeReader.java:56-78`). Java obtains the leading name by walking up
/// the class hierarchy until it finds a class that is not anonymous; Rust has
/// no anonymous types and no runtime class walk, so each concrete composite
/// reader passes the name its Java counterpart would have produced.
///
/// Measured against `lucene-core-10.5.0.jar`: a `MultiReader` over two segment
/// readers renders as `MultiReader(_0(10.5.0):C3 _1(10.5.0):C3)`, and an empty
/// one as `MultiReader()`.
pub fn composite_reader_to_string(simple_name: &str, subs: &[Arc<dyn IndexReader>]) -> String {
    let mut buffer = String::from(simple_name);
    buffer.push('(');
    for (i, sub) in subs.iter().enumerate() {
        if i != 0 {
            buffer.push(' ');
        }
        buffer.push_str(&sub.to_string());
    }
    buffer.push(')');
    buffer
}

/// Renders an object the way `java.lang.Object.toString()` does, for the types
/// whose Lucene counterpart declares no `toString()` of its own.
///
/// Java's default is `getClass().getName() + "@" + Integer.toHexString(hashCode())`,
/// where `hashCode()` is the identity hash. `class_name` must therefore be the
/// **binary** Java name (`org.apache.lucene.index.FilterCodecReader`, with a
/// `$` before a nested class), and `identity` stands in for the identity hash.
///
/// **Divergence from Lucene 10.5.0.** The hexadecimal suffix cannot match:
/// Java's identity hash is drawn from a per-thread PRNG inside the JVM and has
/// no counterpart in Rust. The port substitutes the object's own address, which
/// is the same kind of value — a per-instance identity, stable for the object's
/// lifetime and distinct between live objects — and reproduces the class name
/// and the `name@hex` shape exactly. No Lucene format, and no `toString()` that
/// Lucene itself defines, depends on the digits.
pub fn java_object_to_string<T: ?Sized>(class_name: &str, identity: &T) -> String {
    format!(
        "{}@{:x}",
        class_name,
        std::ptr::from_ref(identity).cast::<()>() as usize
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `registerParentReader` calls `ensureOpen()` on the child first
    /// (`IndexReader.java:162-165`): a closed child refuses a parent, and an
    /// open child marks every registered parent closed-by-child when it
    /// closes, transitively (`IndexReader.java:176-187`).
    #[test]
    fn a_closed_child_refuses_a_parent_and_a_closing_child_poisons_its_parents() {
        let child = IndexReaderCore::new();
        let parent = IndexReaderCore::new();
        let grand_parent = IndexReaderCore::new();
        child.register_parent_core(&parent).unwrap();
        parent.register_parent_core(&grand_parent).unwrap();
        // The handle sees the state of the reader it was taken from.
        let handle = parent.ensure_open_handle();
        assert!(handle.ensure_open().is_ok());

        child.dec_ref_with_close(|| Ok(())).unwrap();
        let closed_by_child =
            "this IndexReader cannot be used anymore as one of its child readers was closed";
        for core in [&parent, &grand_parent] {
            match core.ensure_open() {
                Err(LuceneError::AlreadyClosed { message, .. }) => {
                    assert_eq!(message, closed_by_child)
                }
                other => panic!("expected AlreadyClosed, got {other:?}"),
            }
        }
        assert!(handle.ensure_open().is_err());

        match child.register_parent_core(&IndexReaderCore::new()) {
            Err(LuceneError::AlreadyClosed { message, .. }) => {
                assert_eq!(message, "this IndexReader is closed")
            }
            other => panic!("expected AlreadyClosed, got {other:?}"),
        }
    }

    /// `ensureOpen()` from a member that cannot fail throws the unchecked
    /// `AlreadyClosedException` as a panic carrying its `toString()`.
    #[test]
    fn ensure_open_unchecked_throws_the_java_exception() {
        let core = IndexReaderCore::new();
        core.ensure_open_unchecked();
        core.dec_ref_with_close(|| Ok(())).unwrap();
        let payload = std::panic::catch_unwind(|| core.ensure_open_unchecked()).unwrap_err();
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            text,
            "org.apache.lucene.store.AlreadyClosedException: this IndexReader is closed"
        );
    }
}
