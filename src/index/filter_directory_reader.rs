//! Directory-reader wrappers ported from `org.apache.lucene.index`.
//!
//! Covers `FilterDirectoryReader` with its `SubReaderWrapper`,
//! `ExitableDirectoryReader`, `SlowCodecReaderWrapper` and
//! `SoftDeletesDirectoryReaderWrapper`.

use std::sync::Arc;

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::norms::NormsProducer;
use crate::codecs::points::PointsReader;
use crate::codecs::postings::FieldsProducer;
use crate::codecs::stored_fields::StoredFieldsReader;
use crate::codecs::term_vectors::TermVectorsReader;
use crate::error::{LuceneError, Result};
use crate::index::base_composite_reader::BaseCompositeReader;
use crate::index::codec_reader::CodecReader;
use crate::index::directory_reader::DirectoryReader;
use crate::index::doc_values::{
    BinaryDocValues, DocValues, DocValuesIterator, DocValuesSkipper, FilterBinaryDocValues,
    FilterNumericDocValues, FilterSortedDocValues, FilterSortedNumericDocValues,
    FilterSortedSetDocValues, NumericDocValues, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues,
};
use crate::index::field_infos::{FieldInfo, FieldInfos};
use crate::index::filter_reader::{FilterLeafReader, FilterTerms, FilterTermsEnum};
use crate::index::index_reader::{
    CacheHelper, CacheKey, ClosedListener, CompositeReader, IndexReader, IndexReaderCore,
    StoredFields,
};
use crate::index::index_utilities::QueryTimeout;
use crate::index::leaf_reader::{LeafReader, TermVectors};
use crate::index::point_values::{IntersectVisitor, PointTree, PointValues, Relation};
use crate::index::postings_enum::{ImpactsEnum, PostingsEnum};
use crate::index::stored_field_visitor::StoredFieldVisitor;
use crate::index::terms::{Fields, SeekStatus, TermState, Terms, TermsEnum};
use crate::index::vector_values::{
    ByteVectorValues, DocIndexIterator, FloatVectorValues, KnnVectorValues,
};
use crate::index::VectorEncoding;
use crate::index::{IndexCommit, Term};
use crate::index::{IndexOptions, LeafMetaData};
use crate::search::knn::KnnCollector;
use crate::search::{AcceptDocs, DocIdSetIterator, FilterDocIdSetIterator, VectorScorer};
use crate::store::Directory;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::CompiledAutomaton;
use crate::util::{Bits, BytesRef, IOBooleanSupplier, MatchAllBits};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Wraps each leaf of a [`DirectoryReader`] as it is opened.
///
/// Equivalent to `FilterDirectoryReader.SubReaderWrapper`.
pub trait SubReaderWrapper: Send + Sync {
    /// Wraps one leaf.
    ///
    /// Equivalent to `SubReaderWrapper.wrap(LeafReader)`.
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>>;

    /// Wraps every leaf. The default maps [`wrap`](Self::wrap) over them, as
    /// Java's default method does.
    fn wrap_all(&self, readers: Vec<Arc<dyn LeafReader>>) -> Result<Vec<Arc<dyn LeafReader>>> {
        readers.into_iter().map(|r| self.wrap(r)).collect()
    }
}

/// Rewraps a [`DirectoryReader`] a reopen returned, so that a
/// [`FilterDirectoryReader`]'s `doOpenIfChanged*` family hands back another
/// instance of the same wrapping instead of the bare, unwrapped reader.
///
/// Equivalent to the protected abstract
/// `FilterDirectoryReader.doWrapDirectoryReader(DirectoryReader)`. Java
/// subclasses override that method to return `new SubclassReader(in, ...)`;
/// Rust has no subclassing for the concrete [`FilterDirectoryReader`] to
/// override, so the override becomes this injected callback instead — the
/// same shape [`SubReaderWrapper`] already gives the leaves.
pub trait DirectoryReaderWrapper: Send + Sync {
    /// Wraps `inner`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the wrapped reader raises.
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>>;

    /// Renders the `FilterDirectoryReader` subclass this wrapper stands for.
    ///
    /// **Divergence from Lucene 10.5.0.** In Java each wrapping is its own
    /// subclass of `FilterDirectoryReader` — `ExitableDirectoryReader`,
    /// `SoftDeletesDirectoryReaderWrapper` — and `toString()` is resolved on
    /// that subclass. This port has a single [`FilterDirectoryReader`] struct
    /// parameterised by the wrapper, so the rendering that would live on the
    /// subclass lives here instead, on the object that identifies which
    /// subclass it is.
    ///
    /// The default reproduces what a subclass that overrides no `toString()`
    /// inherits from `CompositeReader`: its own simple name, then the wrapped
    /// leaves. `ExitableDirectoryReader` overrides it, and so overrides this.
    fn reader_to_string(&self, reader: &FilterDirectoryReader) -> String {
        crate::index::index_reader::composite_reader_to_string(
            self.subclass_simple_name(),
            &reader.get_sequential_sub_readers(),
        )
    }

    /// The simple name of the `FilterDirectoryReader` subclass this wrapper
    /// stands for, which is what `CompositeReader.toString()` renders.
    fn subclass_simple_name(&self) -> &'static str {
        "FilterDirectoryReader"
    }

    /// Whether the `FilterDirectoryReader` subclass this wrapper stands for
    /// answers `getReaderCacheHelper()` with a [`DelegatingCacheHelper`] it
    /// builds once, over the wrapped reader's own helper, rather than with the
    /// wrapped reader's helper itself.
    ///
    /// **Divergence from Lucene 10.5.0.** Java leaves `getReaderCacheHelper()`
    /// abstract on `FilterDirectoryReader`, and each subclass implements it:
    /// `ExitableDirectoryReader` returns `in.getReaderCacheHelper()`, while
    /// `SoftDeletesDirectoryReaderWrapper` builds
    /// `new DelegatingCacheHelper(in.getReaderCacheHelper())` in its
    /// constructor (`SoftDeletesDirectoryReaderWrapper.java:61-64`), because
    /// its live documents differ from `in`'s and so may not share `in`'s cache
    /// entries. This port has a single [`FilterDirectoryReader`] struct, so the
    /// choice the subclass would make is asked of the object that identifies
    /// the subclass. The default is the forwarding choice.
    fn uses_delegating_cache_helper(&self) -> bool {
        false
    }
}

/// A [`CacheHelper`] with its own [`CacheKey`] that forwards closed listeners
/// to another helper.
///
/// Equivalent to
/// `org.apache.lucene.index.FilterDirectoryReader.DelegatingCacheHelper`
/// (`FilterDirectoryReader.java:171-202`): the long-lived cache identity a
/// `FilterDirectoryReader` subclass — or a leaf it wraps — gives itself when it
/// changes what the wrapped reader exposes. `getKey()` returns the helper's own
/// key, created with it; `addClosedListener(listener)` registers on the
/// delegate a listener that calls `listener` with *this* helper's key, so a
/// cache keyed on the wrapper is released when the wrapped reader closes.
///
/// [`get_key`](CacheHelper::get_key) hands out a reference to the key this
/// helper owns, so as long as one helper instance answers for a reader — the
/// readers that use it build it once and share it through an [`Arc`], as Java
/// builds it once in their constructors — every caller sees the same key, and
/// a different key from the delegate's.
///
/// **Divergence from Lucene 10.5.0.** This port's [`CacheKey`] is a unit
/// struct whose identity is its address (see
/// [`LRUQueryCache`](crate::search::LRUQueryCache)), and
/// [`ClosedListener::on_close`] takes the key by value. The forwarding listener
/// therefore passes the listener a [`CacheKey`] value where Java passes the
/// `cacheKey` reference; the two cannot be told apart in this port, so the
/// listener observes the same thing.
///
/// Java's `cacheKey` is an object whose identity neither moves nor is reused
/// while anything references it. The port gets the same two guarantees from a
/// [`SharedCacheKey`]: the key lives in its own heap cell, so moving the helper
/// does not move it, and [`cache_key`](Self::cache_key) hands out clones of
/// that cell, so a holder that keeps one keeps the address from being reused
/// by another key.
pub struct DelegatingCacheHelper {
    cache_key: SharedCacheKey,
    delegate: Box<dyn CacheHelper>,
}

impl DelegatingCacheHelper {
    /// Creates a helper with a fresh key that delegates its closed listeners to
    /// `delegate`.
    ///
    /// Equivalent to the protected `DelegatingCacheHelper(CacheHelper)`.
    pub fn new(delegate: Box<dyn CacheHelper>) -> Self {
        Self {
            cache_key: SharedCacheKey::new(),
            delegate,
        }
    }

    /// Returns a handle on this helper's key that keeps the key's identity
    /// alive for as long as the handle is: no other key can take its address
    /// in the meantime, as no other object can take a referenced Java object's
    /// identity.
    pub fn cache_key(&self) -> SharedCacheKey {
        self.cache_key.clone()
    }
}

/// A [`CacheKey`] whose identity — its address — is stable and is not reused
/// while any clone of the handle is alive.
///
/// The key is the first field of a `#[repr(C)]` cell whose other field is not
/// empty, so its address lies strictly inside the cell's own heap allocation;
/// the allocation is shared by every clone and freed with the last one.
#[derive(Clone)]
pub struct SharedCacheKey(Arc<SharedCacheKeyCell>);

#[repr(C)]
struct SharedCacheKeyCell {
    key: CacheKey,
    /// Makes the cell non-empty, so the key's address is inside the
    /// allocation rather than one past its end.
    _non_empty: u8,
}

impl SharedCacheKey {
    fn new() -> Self {
        Self(Arc::new(SharedCacheKeyCell {
            key: CacheKey,
            _non_empty: 0,
        }))
    }

    /// Returns the key, whose address is the identity every holder compares.
    pub fn key(&self) -> &CacheKey {
        &self.0.key
    }
}

impl std::fmt::Debug for SharedCacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedCacheKey")
            .field(&(self.key() as *const CacheKey))
            .finish()
    }
}

impl std::fmt::Debug for DelegatingCacheHelper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DelegatingCacheHelper")
            .field("delegate", &self.delegate)
            .finish_non_exhaustive()
    }
}

/// The listener [`DelegatingCacheHelper::add_closed_listener`] registers on
/// the delegate: the lambda `unused -> listener.onClose(cacheKey)`.
struct DelegatingClosedListener {
    listener: Box<dyn ClosedListener>,
}

impl ClosedListener for DelegatingClosedListener {
    fn on_close(&self, _unused: CacheKey) -> Result<()> {
        self.listener.on_close(CacheKey)
    }
}

impl CacheHelper for DelegatingCacheHelper {
    /// Equivalent to `DelegatingCacheHelper.getKey()`: this helper's own key,
    /// never the delegate's.
    fn get_key(&self) -> &CacheKey {
        self.cache_key.key()
    }

    /// Equivalent to `DelegatingCacheHelper.addClosedListener(ClosedListener)`:
    /// the listener is wrapped and registered on the delegate, so that it runs
    /// when the delegate's resource closes, and is called with this helper's
    /// key.
    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        self.delegate
            .add_closed_listener(Box::new(DelegatingClosedListener { listener }));
    }
}

impl CacheHelper for Arc<DelegatingCacheHelper> {
    fn get_key(&self) -> &CacheKey {
        self.as_ref().get_key()
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        self.as_ref().add_closed_listener(listener);
    }
}

/// Builds the [`DelegatingCacheHelper`] a wrapper keeps over `delegate`, or
/// `None` when the wrapped reader has no helper.
///
/// Equivalent to the expression every Java user of the class writes:
/// `in.getReaderCacheHelper() == null ? null : new DelegatingCacheHelper(in.getReaderCacheHelper())`.
fn delegating_cache_helper(
    delegate: Option<Box<dyn CacheHelper>>,
) -> Option<Arc<DelegatingCacheHelper>> {
    delegate.map(|delegate| Arc::new(DelegatingCacheHelper::new(delegate)))
}

/// A [`DirectoryReader`] that forwards every call to a wrapped reader, with its
/// leaves passed through a [`SubReaderWrapper`].
///
/// Equivalent to `org.apache.lucene.index.FilterDirectoryReader`.
pub struct FilterDirectoryReader {
    inner: Arc<dyn DirectoryReader>,
    wrapper: Arc<dyn SubReaderWrapper>,
    directory_wrapper: Arc<dyn DirectoryReaderWrapper>,
    wrapped_leaves: Vec<Arc<dyn LeafReader>>,
    /// The composite state over the **wrapped** leaves.
    ///
    /// Java reaches it by inheritance: `FilterDirectoryReader extends
    /// DirectoryReader extends BaseCompositeReader`, and the constructor passes
    /// `wrapper.wrap(in.getSequentialSubReaders())` up to it
    /// (`FilterDirectoryReader.java:91-93`). Every method `BaseCompositeReader`
    /// declares `final` — `getSequentialSubReaders`, `numDocs`, `maxDoc`,
    /// `termVectors`, `storedFields`, `docFreq`, `totalTermFreq`,
    /// `getSumDocFreq`, `getDocCount`, `getSumTotalTermFreq` — is therefore
    /// answered from the wrapped leaves and *cannot* be delegated to `in`.
    /// Rust has no implementation inheritance, so the base is held here and
    /// those methods forward to it.
    base: BaseCompositeReader,
    /// The helper `get_reader_cache_helper` answers with when the subclass
    /// keeps its own; see
    /// [`DirectoryReaderWrapper::uses_delegating_cache_helper`].
    reader_cache_helper: Option<Arc<DelegatingCacheHelper>>,
    /// Whether [`reader_cache_helper`](Self::reader_cache_helper) is the answer,
    /// rather than the wrapped reader's helper.
    owns_reader_cache_helper: bool,
}

impl std::fmt::Debug for FilterDirectoryReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterDirectoryReader")
            .field("leaves", &self.wrapped_leaves.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for FilterDirectoryReader {
    /// `FilterDirectoryReader` is abstract in Java and declares no
    /// `toString()`; what renders is always the concrete subclass's, so this
    /// defers to the [`DirectoryReaderWrapper`] that stands for that subclass.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.directory_wrapper.reader_to_string(self))
    }
}

impl FilterDirectoryReader {
    /// Wraps `inner`, passing each of `leaves` through `wrapper`, and
    /// remembering `directory_wrapper` for `doOpenIfChanged*` to reproduce the
    /// same wrapping on the reopened reader.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's constructor reaches the leaves
    /// through `in.getSequentialSubReaders()` and downcasts each to
    /// `LeafReader`. Rust has no downcast from `dyn IndexReader`, so the caller
    /// supplies the leaves it already holds; every caller of this type has them.
    ///
    /// # Errors
    ///
    /// Propagates whatever `wrapper` raises while wrapping `leaves`.
    pub fn new(
        inner: Arc<dyn DirectoryReader>,
        leaves: Vec<Arc<dyn LeafReader>>,
        wrapper: Arc<dyn SubReaderWrapper>,
        directory_wrapper: Arc<dyn DirectoryReaderWrapper>,
    ) -> Result<Self> {
        let wrapped_leaves = wrapper.wrap_all(leaves)?;
        // `super(in.directory(), wrapper.wrap(in.getSequentialSubReaders()), null)`:
        // the composite is built over the **wrapped** leaves, never the
        // delegate's. `true` selects the `CorruptIndexException` a
        // `DirectoryReader` raises when the leaves hold too many documents,
        // rather than the `IllegalArgumentException` a hand-assembled
        // `MultiReader` gets (`BaseCompositeReader.java:88-101`).
        let sub_readers: Vec<Arc<dyn IndexReader>> = wrapped_leaves
            .iter()
            .map(|leaf| Arc::clone(leaf).as_index_reader())
            .collect();
        let base = BaseCompositeReader::new(sub_readers, true)?;
        let owns_reader_cache_helper = directory_wrapper.uses_delegating_cache_helper();
        let reader_cache_helper = if owns_reader_cache_helper {
            delegating_cache_helper(inner.get_reader_cache_helper())
        } else {
            None
        };
        Ok(Self {
            inner,
            wrapper,
            directory_wrapper,
            wrapped_leaves,
            base,
            reader_cache_helper,
            owns_reader_cache_helper,
        })
    }

    /// Returns the wrapped reader.
    ///
    /// Equivalent to `FilterDirectoryReader.getDelegate()`.
    pub fn get_delegate(&self) -> &Arc<dyn DirectoryReader> {
        &self.inner
    }

    /// Returns the sub-reader wrapper.
    pub fn get_sub_reader_wrapper(&self) -> &Arc<dyn SubReaderWrapper> {
        &self.wrapper
    }

    /// Returns the wrapped leaves.
    pub fn wrapped_leaves(&self) -> &[Arc<dyn LeafReader>] {
        &self.wrapped_leaves
    }

    /// Returns the reader this one wraps.
    ///
    /// Equivalent to reading `FilterDirectoryReader.in`, which is what
    /// `ExitableDirectoryReader.toString()` renders.
    pub fn delegate(&self) -> &Arc<dyn DirectoryReader> {
        &self.inner
    }

    /// Returns a new wrapped instance of `inner`, produced by the injected
    /// [`DirectoryReaderWrapper`].
    ///
    /// Equivalent to the protected abstract
    /// `FilterDirectoryReader.doWrapDirectoryReader(DirectoryReader)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the [`DirectoryReaderWrapper`] raises.
    fn do_wrap_directory_reader(
        &self,
        inner: Arc<dyn DirectoryReader>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        self.directory_wrapper.wrap(inner)
    }

    /// Wraps `inner` unless it is `None`.
    ///
    /// Equivalent to the private final
    /// `FilterDirectoryReader.wrapDirectoryReader(DirectoryReader)`, which
    /// every `doOpenIfChanged*` override calls on the result of reopening the
    /// delegate.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::do_wrap_directory_reader`] raises.
    fn wrap_directory_reader(
        &self,
        inner: Option<Arc<dyn DirectoryReader>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        match inner {
            None => Ok(None),
            Some(inner) => Ok(Some(self.do_wrap_directory_reader(inner)?)),
        }
    }

    /// Unwraps nested `FilterDirectoryReader`s and returns the innermost
    /// reader.
    ///
    /// Equivalent to `FilterDirectoryReader.unwrap(DirectoryReader)`.
    pub fn unwrap(mut reader: Arc<dyn DirectoryReader>) -> Arc<dyn DirectoryReader> {
        loop {
            let next = reader
                .as_any()
                .downcast_ref::<FilterDirectoryReader>()
                .map(|filter| Arc::clone(&filter.inner));
            match next {
                Some(next) => reader = next,
                None => break,
            }
        }
        reader
    }
}

impl IndexReader for FilterDirectoryReader {
    /// **Divergence from Lucene 10.5.0.** Java's `FilterDirectoryReader` is an
    /// `IndexReader` in its own right, with its own reference count and its own
    /// identity; this port shares the delegate's [`IndexReaderCore`], so the
    /// two readers count references together and compare equal. That is
    /// pre-existing and orthogonal to the leaves; it is recorded here because
    /// it is the reason `BaseCompositeReader`'s `registerParentReader` call
    /// (`BaseCompositeReader.java:86`) is not reproduced — there is no separate
    /// core for the wrapped leaves to register against.
    fn core(&self) -> &IndexReaderCore {
        self.inner.core()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.base.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.base.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.base.max_doc()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.base.stored_fields()
    }

    /// Equivalent to `FilterDirectoryReader.doClose()`, the one close method
    /// the class overrides: it closes the delegate.
    fn do_close(&self) -> Result<()> {
        self.inner.do_close()
    }

    /// Sends the notification the delegate's own close sends.
    ///
    /// Java's `doClose()` is `in.close()`, and `in.decRef()` runs `in`'s
    /// `notifyReaderClosedListeners()` — which is how a listener registered
    /// through a [`DelegatingCacheHelper`] over `in`'s helper learns that the
    /// wrapped reader closed. This port shares `in`'s core (see
    /// [`core`](IndexReader::core)), so [`do_close`](IndexReader::do_close)
    /// calls `in`'s `do_close` directly and the notification is forwarded
    /// here, where the wrapper's own close sends its (empty) one.
    fn notify_reader_closed_listeners(&self) -> Result<()> {
        self.inner.notify_reader_closed_listeners()
    }

    /// Equivalent to the subclass's `getReaderCacheHelper()`: the wrapped
    /// reader's helper, or the [`DelegatingCacheHelper`] the subclass built over
    /// it; see [`DirectoryReaderWrapper::uses_delegating_cache_helper`].
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        if self.owns_reader_cache_helper {
            self.reader_cache_helper
                .as_ref()
                .map(|helper| Box::new(Arc::clone(helper)) as Box<dyn CacheHelper>)
        } else {
            self.inner.get_reader_cache_helper()
        }
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.base.doc_freq(term)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.base.total_term_freq(term)
    }

    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        self.base.get_sum_doc_freq(field)
    }

    fn get_doc_count(&self, field: &str) -> Result<i32> {
        self.base.get_doc_count(field)
    }

    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        self.base.get_sum_total_term_freq(field)
    }

    fn build_context(
        self: Arc<Self>,
        parent: Option<std::sync::Weak<dyn crate::index::reader_context::IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn crate::index::reader_context::IndexReaderContext> {
        crate::index::index_reader::build_composite_context(
            self,
            parent,
            ord_in_parent,
            doc_base_in_parent,
            leaf_ord,
            leaf_doc_base,
        )
    }
}

impl CompositeReader for FilterDirectoryReader {
    /// Returns the **wrapped** leaves.
    ///
    /// Equivalent to `BaseCompositeReader.getSequentialSubReaders()` — `final`
    /// in Java — over the array `FilterDirectoryReader`'s constructor built
    /// with `wrapper.wrap(in.getSequentialSubReaders())`
    /// (`FilterDirectoryReader.java:91-93`). Answering with the delegate's own
    /// leaves instead makes every wrapping a silent no-op through
    /// [`IndexReader::leaves`], which is the normal path into a reader.
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.base.get_sequential_sub_readers()
    }
}

impl DirectoryReader for FilterDirectoryReader {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn directory(&self) -> Arc<dyn Directory> {
        self.inner.directory()
    }

    fn version(&self) -> Result<i64> {
        self.inner.version()
    }

    fn is_current(&self) -> Result<bool> {
        self.inner.is_current()
    }

    fn index_commit(&self) -> Result<Box<dyn IndexCommit>> {
        self.inner.index_commit()
    }

    fn do_open_if_changed(&self) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.wrap_directory_reader(self.inner.do_open_if_changed()?)
    }

    fn do_open_if_changed_from_commit(
        &self,
        commit: &dyn IndexCommit,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.wrap_directory_reader(self.inner.do_open_if_changed_from_commit(commit)?)
    }

    fn do_open_if_changed_from_writer(
        &self,
        writer: &dyn crate::index::directory_reader::IndexWriter,
        apply_all_deletes: bool,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.wrap_directory_reader(
            self.inner
                .do_open_if_changed_from_writer(writer, apply_all_deletes)?,
        )
    }
}

/// Raised when a [`QueryTimeout`] fires inside a reader.
///
/// Equivalent to `ExitableDirectoryReader.ExitingReaderException`, which Java
/// makes a `RuntimeException` so it can escape an iterator method that declares
/// no checked exception. Rust returns it as an error like any other.
pub fn exiting_reader_error(what: &str) -> LuceneError {
    exiting_reader_action_error(&format!("iterate over {what}"))
}

/// Raised when a [`QueryTimeout`] fires inside a reader, for an action that is
/// not an iteration.
///
/// Equivalent to the `ExitingReaderException` messages that name an action of
/// their own: `"intersect point values"` and `"search nearest vectors"`.
///
/// **Divergence from Lucene 10.5.0.** Java appends `", Timeout: " + queryTimeout`
/// and the object it was iterating — `DocValues=`, `TermsEnum=`, `PointValues=`
/// and so on. This port's message names the action only.
fn exiting_reader_action_error(action: &str) -> LuceneError {
    LuceneError::Other(format!("The request took too long to {action}"))
}

/// How many documents an [`ExitableFilterLeafReader`]-family iterator lets
/// pass before it checks the [`QueryTimeout`] again.
///
/// Equivalent to `ExitableDirectoryReader.ExitableFilterAtomicReader.DOCS_BETWEEN_TIMEOUT_CHECK`.
const DOCS_BETWEEN_TIMEOUT_CHECK: i32 = 1000;

/// Reproduces the same `ExitableDirectoryReader` wrapping on a reopened
/// [`DirectoryReader`].
///
/// Equivalent to `ExitableDirectoryReader.doWrapDirectoryReader(DirectoryReader)`,
/// which Java implements by constructing another `ExitableDirectoryReader`
/// (itself `super(in, new ExitableSubReaderWrapper(queryTimeout))`). Rust has
/// no `ExitableDirectoryReader` subclass to reconstruct; this
/// [`DirectoryReaderWrapper`] plays that role by handing
/// [`FilterDirectoryReader::new`] the same [`ExitableSubReaderWrapper`] and
/// itself again, so a chain of reopens keeps checking the same timeout.
pub struct ExitableDirectoryReaderWrapper {
    query_timeout: Arc<dyn QueryTimeout>,
}

impl ExitableDirectoryReaderWrapper {
    /// Creates the wrapper.
    pub fn new(query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self { query_timeout }
    }
}

impl DirectoryReaderWrapper for ExitableDirectoryReaderWrapper {
    /// Port of `ExitableDirectoryReader.toString()`
    /// (`ExitableDirectoryReader.java:960-963`), which renders the *delegate*
    /// rather than the wrapped leaves `CompositeReader.toString()` would.
    /// Measured against `lucene-core-10.5.0.jar`.
    fn reader_to_string(&self, reader: &FilterDirectoryReader) -> String {
        format!("ExitableDirectoryReader({})", reader.delegate())
    }

    fn subclass_simple_name(&self) -> &'static str {
        "ExitableDirectoryReader"
    }

    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        // Equivalent to `FilterDirectoryReader(DirectoryReader, SubReaderWrapper)`
        // reaching `in.getSequentialSubReaders()`: Rust has no downcast from
        // `dyn IndexReader` to `dyn LeafReader` (see `FilterDirectoryReader::new`'s
        // doc comment), so the leaves are read back off `inner`'s own
        // `LeafReaderContext`s instead, which carry the `Arc<dyn LeafReader>`
        // handle directly.
        let leaves: Vec<Arc<dyn LeafReader>> = Arc::clone(&inner)
            .leaves()
            .into_iter()
            .map(|context| context.leaf_reader())
            .collect();
        let sub_wrapper: Arc<dyn SubReaderWrapper> = Arc::new(ExitableSubReaderWrapper::new(
            Arc::clone(&self.query_timeout),
        ));
        let directory_wrapper: Arc<dyn DirectoryReaderWrapper> =
            Arc::new(Self::new(Arc::clone(&self.query_timeout)));
        Ok(Arc::new(FilterDirectoryReader::new(
            inner,
            leaves,
            sub_wrapper,
            directory_wrapper,
        )?))
    }
}

/// Wraps each leaf so that iterating it checks a [`QueryTimeout`].
///
/// Equivalent to `ExitableDirectoryReader.ExitableSubReaderWrapper`.
pub struct ExitableSubReaderWrapper {
    query_timeout: Arc<dyn QueryTimeout>,
}

impl ExitableSubReaderWrapper {
    /// Creates the wrapper.
    pub fn new(query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self { query_timeout }
    }
}

impl SubReaderWrapper for ExitableSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(ExitableFilterLeafReader::new(
            reader,
            Arc::clone(&self.query_timeout),
        )))
    }
}

/// The timeout state one `Exitable*` wrapper keeps: the timeout itself, the
/// call counter its sampling uses, and the flag that carries a check Rust could
/// not report at the point Java throws it.
///
/// Equivalent to the `queryTimeout` and `calls` fields the
/// `ExitableDirectoryReader` inner classes each declare, together with their
/// `checkAndThrow()` / `checkAndThrowWithSampling()` methods.
///
/// Lucene raises `ExitingReaderException` from methods that declare no checked
/// exception, because it is a Java `RuntimeException`. The port carries that
/// fallibility in the signature instead: [`PointValues::size`],
/// [`PointValues::doc_count`], [`PointTree::min_packed_value`],
/// [`PointTree::max_packed_value`], [`PointTree::size`],
/// [`PointTree::clone_tree`], [`IntersectVisitor::compare`] and
/// [`IntersectVisitor::grow`] all return [`Result`], so each check is reported
/// at the call Lucene throws from.
///
/// The single exception is [`Bits::get`], which returns a plain `bool`; see the
/// divergence recorded on [`ExitableAcceptDocs`] for why, and for how the check
/// it makes is reported. [`defer_check_with_sampling`](Self::defer_check_with_sampling)
/// and [`take_deferred`](Self::take_deferred) serve that one method.
#[derive(Debug)]
struct ExitableCheck {
    query_timeout: Arc<dyn QueryTimeout>,
    /// Equivalent to the `calls` field of the sampling inner classes. Java's
    /// `int` wraps on overflow, matched here by the atomic's wrapping add.
    calls: AtomicI32,
    deferred: AtomicBool,
    /// The action the message names, e.g. `"iterate over terms"`.
    action: &'static str,
}

impl ExitableCheck {
    fn new(query_timeout: Arc<dyn QueryTimeout>, action: &'static str) -> Self {
        Self {
            query_timeout,
            calls: AtomicI32::new(0),
            deferred: AtomicBool::new(false),
            action,
        }
    }

    /// The timeout this wrapper checks, so that a wrapper can hand it to the
    /// objects it in turn wraps.
    fn query_timeout(&self) -> &Arc<dyn QueryTimeout> {
        &self.query_timeout
    }

    /// The timeout test itself, without the deferred-check bookkeeping.
    fn fired(&self) -> bool {
        self.query_timeout.should_exit()
    }

    /// Equivalent to the `checkAndThrow()` of the enclosing inner class, at a
    /// call site that can also report a check deferred from an earlier one.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has fired.
    fn check_and_throw(&self) -> Result<()> {
        self.take_deferred()?;
        if self.fired() {
            return Err(exiting_reader_action_error(self.action));
        }
        Ok(())
    }

    /// Equivalent to `checkAndThrowWithSampling()`, which checks when
    /// `calls++ % MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK == 0`.
    ///
    /// # Errors
    ///
    /// As [`check_and_throw`](Self::check_and_throw), on a sampled call.
    fn check_and_throw_with_sampling(&self, modulus: i32) -> Result<()> {
        self.take_deferred()?;
        if self.sample(modulus) && self.fired() {
            return Err(exiting_reader_action_error(self.action));
        }
        Ok(())
    }

    /// Equivalent to `ExitableTermsEnum.checkTimeoutWithSampling()`, which
    /// samples with the bit mask `(calls++ & NUM_CALLS_PER_TIMEOUT_CHECK) == 0`
    /// rather than with a remainder.
    ///
    /// # Errors
    ///
    /// As [`check_and_throw`](Self::check_and_throw), on a sampled call.
    fn check_and_throw_with_mask(&self, mask: i32) -> Result<()> {
        self.take_deferred()?;
        let calls = self.calls.fetch_add(1, Ordering::Relaxed);
        if calls & mask == 0 && self.fired() {
            return Err(exiting_reader_action_error(self.action));
        }
        Ok(())
    }

    /// Java's `calls++ % modulus == 0`. The atomic add wraps exactly as Java's
    /// `int` field does.
    fn sample(&self, modulus: i32) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed) % modulus == 0
    }

    /// The `checkAndThrowWithSampling()` of [`Bits::get`], the one method Rust
    /// cannot fail from: the check runs where Java runs it and the failure is
    /// remembered for [`take_deferred`](Self::take_deferred).
    fn defer_check_with_sampling(&self, modulus: i32) {
        if self.sample(modulus) && self.fired() {
            self.deferred.store(true, Ordering::Relaxed);
        }
    }

    /// Reports and clears a check deferred by
    /// [`defer_check_with_sampling`](Self::defer_check_with_sampling).
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when a deferred check had fired.
    fn take_deferred(&self) -> Result<()> {
        if self.deferred.swap(false, Ordering::Relaxed) {
            return Err(exiting_reader_action_error(self.action));
        }
        Ok(())
    }
}

/// A leaf reader that refuses to keep working once its timeout has fired.
///
/// Equivalent to `ExitableDirectoryReader.ExitableFilterAtomicReader`. Like
/// Java's, it checks no timeout of its own on the way in: it wraps what it
/// hands out — [`ExitableTerms`], `ExitablePointValues`,
/// `ExitableFloatVectorValues`, `ExitableByteVectorValues`, the doc-values
/// wrappers and `ExitableAcceptDocs` — so the check recurs as the caller
/// iterates. Those wrappers are private, as Lucene declares them.
pub struct ExitableFilterLeafReader {
    inner: FilterLeafReader,
    query_timeout: Arc<dyn QueryTimeout>,
}

impl std::fmt::Debug for ExitableFilterLeafReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExitableFilterLeafReader")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for ExitableFilterLeafReader {
    /// `ExitableDirectoryReader.ExitableFilterAtomicReader` overrides no
    /// `toString()`, so it inherits `FilterLeafReader`'s — which hardcodes the
    /// name `FilterLeafReader`, not the subclass's. Measured against
    /// `lucene-core-10.5.0.jar`, where the leaf of an `ExitableDirectoryReader`
    /// renders as `FilterLeafReader(_0(10.5.0):C3)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}

impl ExitableFilterLeafReader {
    /// Wraps `reader`, checking `query_timeout` as the caller iterates what it
    /// hands out.
    pub fn new(reader: Arc<dyn LeafReader>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: FilterLeafReader::new(reader),
            query_timeout,
        }
    }

    /// Equivalent to the private
    /// `ExitableFilterAtomicReader.wrapNumericDocValues(NumericDocValues)`.
    fn wrap_numeric_doc_values(
        &self,
        numeric_doc_values: Option<Box<dyn NumericDocValues>>,
    ) -> Option<Box<dyn NumericDocValues>> {
        let numeric_doc_values = numeric_doc_values?;
        Some(Box::new(ExitableNumericDocValues::new(
            numeric_doc_values,
            Arc::clone(&self.query_timeout),
        )))
    }

    /// Equivalent to the private
    /// `ExitableFilterAtomicReader.wrapSortedDocValues(SortedDocValues)`.
    fn wrap_sorted_doc_values(
        &self,
        sorted_doc_values: Option<Box<dyn SortedDocValues>>,
    ) -> Option<Box<dyn SortedDocValues>> {
        let sorted_doc_values = sorted_doc_values?;
        Some(Box::new(ExitableSortedDocValues::new(
            sorted_doc_values,
            Arc::clone(&self.query_timeout),
        )))
    }
}

impl LeafReader for ExitableFilterLeafReader {
    fn core(&self) -> &IndexReaderCore {
        LeafReader::core(&self.inner)
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }

    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }

    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }

    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }

    // This impl does not change deletes or data so we can delegate the
    // CacheHelpers.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        LeafReader::get_reader_cache_helper(&self.inner)
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_core_cache_helper()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn crate::index::Terms>>> {
        let Some(terms) = self.inner.terms(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(ExitableTerms::new(
            terms,
            Arc::clone(&self.query_timeout),
        ))))
    }

    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::NumericDocValues>>> {
        Ok(self.wrap_numeric_doc_values(self.inner.get_numeric_doc_values(field)?))
    }

    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::BinaryDocValues>>> {
        let Some(binary_doc_values) = self.inner.get_binary_doc_values(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(ExitableBinaryDocValues::new(
            binary_doc_values,
            Arc::clone(&self.query_timeout),
        ))))
    }

    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::SortedDocValues>>> {
        Ok(self.wrap_sorted_doc_values(self.inner.get_sorted_doc_values(field)?))
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::SortedNumericDocValues>>> {
        let Some(sorted_numeric_doc_values) = self.inner.get_sorted_numeric_doc_values(field)?
        else {
            return Ok(None);
        };
        // Equivalent to `DocValues.unwrapSingleton(sortedNumericDocValues)`
        // answering non-null: the single-valued view is wrapped and re-wrapped
        // as a singleton, so a consumer that unwraps it again still finds one.
        // Java asks `instanceof`; this port asks the implementation, which is
        // the divergence recorded on
        // [`SortedNumericDocValues::into_singleton`].
        if sorted_numeric_doc_values.is_singleton() {
            let numeric_doc_values = sorted_numeric_doc_values
                .into_singleton()
                .expect("INVARIANT: is_singleton() reported a singleton view");
            let wrapped = self
                .wrap_numeric_doc_values(Some(numeric_doc_values))
                .expect("INVARIANT: wrap_numeric_doc_values keeps a present value present");
            return Ok(Some(Box::new(DocValues::singleton_numeric(wrapped))));
        }
        Ok(Some(Box::new(ExitableSortedNumericDocValues::new(
            sorted_numeric_doc_values,
            Arc::clone(&self.query_timeout),
        ))))
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::SortedSetDocValues>>> {
        let Some(sorted_set_doc_values) = self.inner.get_sorted_set_doc_values(field)? else {
            return Ok(None);
        };
        // As `get_sorted_numeric_doc_values`, for
        // `DocValues.unwrapSingleton(SortedSetDocValues)`.
        if sorted_set_doc_values.is_singleton() {
            let sorted_doc_values = sorted_set_doc_values
                .into_singleton()
                .expect("INVARIANT: is_singleton() reported a singleton view");
            let wrapped = self
                .wrap_sorted_doc_values(Some(sorted_doc_values))
                .expect("INVARIANT: wrap_sorted_doc_values keeps a present value present");
            return Ok(Some(Box::new(DocValues::singleton_sorted(wrapped))));
        }
        Ok(Some(Box::new(ExitableSortedSetDocValues::new(
            sorted_set_doc_values,
            Arc::clone(&self.query_timeout),
        ))))
    }

    fn get_norm_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }

    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }

    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::FloatVectorValues>>> {
        let Some(vector_values) = self.inner.get_float_vector_values(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(ExitableFloatVectorValues::new(
            vector_values,
            Arc::clone(&self.query_timeout),
        ))))
    }

    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::ByteVectorValues>>> {
        let Some(vector_values) = self.inner.get_byte_vector_values(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(ExitableByteVectorValues::new(
            vector_values,
            Arc::clone(&self.query_timeout),
        ))))
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn crate::search::knn::KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        let mut timeout_checking_accept_docs = ExitableAcceptDocs::new(
            accept_docs,
            LeafReader::max_doc(self),
            Arc::clone(&self.query_timeout),
        );
        let outcome = self.inner.search_nearest_vectors(
            field,
            target,
            collector,
            &mut timeout_checking_accept_docs,
        );
        timeout_checking_accept_docs.finish(outcome)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn crate::search::knn::KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        let mut timeout_checking_accept_docs = ExitableAcceptDocs::new(
            accept_docs,
            LeafReader::max_doc(self),
            Arc::clone(&self.query_timeout),
        );
        let outcome = self.inner.search_nearest_vectors_byte(
            field,
            target,
            collector,
            &mut timeout_checking_accept_docs,
        );
        timeout_checking_accept_docs.finish(outcome)
    }

    fn get_field_infos(&self) -> crate::index::FieldInfos {
        self.inner.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.inner.get_live_docs()
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn crate::index::PointValues>>> {
        let Some(point_values) = self.inner.get_point_values(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(ExitablePointValues::new(
            point_values,
            Arc::clone(&self.query_timeout),
        )?)))
    }

    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    fn get_meta_data(&self) -> crate::index::leaf_reader::LeafMetaData {
        self.inner.get_meta_data()
    }
}

/// Wrapper around another [`Terms`] implementation, used by
/// [`ExitableFilterLeafReader`].
///
/// Equivalent to `ExitableDirectoryReader.ExitableTerms`, which extends
/// `FilterLeafReader.FilterTerms` and overrides only the two methods that hand
/// out an enumeration — so that the enumeration, not this object, is what
/// checks the timeout — plus `getMin()` and `getMax()`, which go straight to
/// the delegate rather than through `Terms`' iterator-based defaults.
pub struct ExitableTerms {
    inner: FilterTerms,
    query_timeout: Arc<dyn QueryTimeout>,
}

impl ExitableTerms {
    /// Wraps `terms`, checking `query_timeout` as its enumerations are walked.
    ///
    /// Equivalent to `ExitableTerms(Terms, QueryTimeout)`.
    pub fn new(terms: Box<dyn Terms>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: FilterTerms::new(terms),
            query_timeout,
        }
    }
}

impl Terms for ExitableTerms {
    /// Equivalent to `ExitableTerms.intersect(CompiledAutomaton, BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors, and raises the exiting-reader error
    /// when the timeout has already fired: the enumeration checks it as it is
    /// built.
    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(ExitableTermsEnum::new(
            self.inner.get_delegate().intersect(compiled, start_term)?,
            Arc::clone(&self.query_timeout),
        )?))
    }

    /// Equivalent to `ExitableTerms.iterator()`.
    ///
    /// # Errors
    ///
    /// As [`intersect`](Self::intersect).
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(ExitableTermsEnum::new(
            self.inner.get_delegate().iterator()?,
            Arc::clone(&self.query_timeout),
        )?))
    }

    /// Equivalent to `ExitableTerms.getMin()`, which reads the delegate's
    /// answer instead of letting `Terms`' default walk this object's own
    /// exitable iterator.
    fn min(&self) -> Result<Option<BytesRef>> {
        self.inner.get_delegate().min()
    }

    /// Equivalent to `ExitableTerms.getMax()`. See [`min`](Self::min).
    fn max(&self) -> Result<Option<BytesRef>> {
        self.inner.get_delegate().max()
    }

    fn size(&self) -> i64 {
        self.inner.size()
    }

    fn sum_total_term_freq(&self) -> i64 {
        self.inner.sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> i64 {
        self.inner.sum_doc_freq()
    }

    fn doc_count(&self) -> i32 {
        self.inner.doc_count()
    }

    fn has_freqs(&self) -> bool {
        self.inner.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.inner.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.inner.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.inner.has_payloads()
    }

    fn stats(&self) -> String {
        self.inner.stats()
    }
}

/// How many calls an [`ExitableTermsEnum`] serves between two timeout checks,
/// as a mask: `(calls & NUM_CALLS_PER_TIMEOUT_CHECK) == 0` checks one call in
/// sixteen.
///
/// Equivalent to `ExitableDirectoryReader.ExitableTermsEnum.NUM_CALLS_PER_TIMEOUT_CHECK`,
/// which Java writes as `(1 << 4) - 1`.
const NUM_CALLS_PER_TIMEOUT_CHECK: i32 = (1 << 4) - 1;

/// Wrapper around a [`TermsEnum`] that makes an enumeration of terms exitable.
///
/// Equivalent to `ExitableDirectoryReader.ExitableTermsEnum`, which extends
/// `FilterLeafReader.FilterTermsEnum` and overrides only `next()`: every
/// sixteenth call — counting the one the constructor makes — checks the
/// timeout before the delegate advances.
pub struct ExitableTermsEnum {
    inner: FilterTermsEnum,
    check: ExitableCheck,
}

impl ExitableTermsEnum {
    /// Wraps `terms_enum`, checking `query_timeout` as it is walked.
    ///
    /// Equivalent to `ExitableTermsEnum(TermsEnum, QueryTimeout)`, whose
    /// constructor makes the first sampled check itself.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has already fired.
    pub fn new(
        terms_enum: Box<dyn TermsEnum>,
        query_timeout: Arc<dyn QueryTimeout>,
    ) -> Result<Self> {
        let enumeration = Self {
            inner: FilterTermsEnum::new(terms_enum),
            check: ExitableCheck::new(query_timeout, "iterate over terms"),
        };
        enumeration.check_timeout_with_sampling()?;
        Ok(enumeration)
    }

    /// Equivalent to the private
    /// `ExitableTermsEnum.checkTimeoutWithSampling()`.
    fn check_timeout_with_sampling(&self) -> Result<()> {
        self.check
            .check_and_throw_with_mask(NUM_CALLS_PER_TIMEOUT_CHECK)
    }
}

impl std::fmt::Display for ExitableTermsEnum {
    /// `ExitableDirectoryReader.ExitableTermsEnum` declares no `toString()`, so
    /// `java.lang.Object`'s applies. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.ExitableDirectoryReader$ExitableTermsEnum",
            self,
        ))
    }
}

impl TermsEnum for ExitableTermsEnum {
    /// Equivalent to `ExitableTermsEnum.next()`, which checks the timeout
    /// before every iteration.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error on a sampled call once the timeout has
    /// fired, and otherwise propagates the delegate's errors.
    fn next(&mut self) -> Result<Option<BytesRef>> {
        // Before every iteration, check if the iteration should exit.
        self.check_timeout_with_sampling()?;
        self.inner.next()
    }

    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.inner.seek_exact(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }

    fn term(&self) -> Result<BytesRef> {
        self.inner.term()
    }

    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.inner.doc_freq()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.inner.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.inner.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }

    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        self.inner.prepare_seek_exact(text)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }
}

/// How many calls the sampling `Exitable*` wrappers serve between two timeout
/// checks.
///
/// Equivalent to the `MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK` constant that
/// `ExitableDirectoryReader` declares three times over, in `ExitablePointTree`,
/// in `ExitableIntersectVisitor`, and in the `Bits` that
/// `ExitableAcceptDocs.bits()` returns.
const MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK: i32 = 16;

/// Wrapper around another [`PointValues`] implementation, used by
/// [`ExitableFilterLeafReader`].
///
/// Equivalent to `ExitableDirectoryReader.ExitablePointValues`, which checks
/// the timeout in its constructor and again in every accessor, and wraps the
/// tree it hands out in an [`ExitablePointTree`].
struct ExitablePointValues {
    inner: Box<dyn PointValues>,
    check: ExitableCheck,
}

impl ExitablePointValues {
    /// Wraps `point_values`, checking `query_timeout` on construction and on
    /// every access.
    ///
    /// Equivalent to `ExitablePointValues(PointValues, QueryTimeout)`, whose
    /// constructor ends with `checkAndThrow()`.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has already fired.
    fn new(
        point_values: Box<dyn PointValues>,
        query_timeout: Arc<dyn QueryTimeout>,
    ) -> Result<Self> {
        let values = Self {
            inner: point_values,
            check: ExitableCheck::new(query_timeout, "iterate over point values"),
        };
        values.check.check_and_throw()?;
        Ok(values)
    }
}

impl PointValues for ExitablePointValues {
    /// Equivalent to `ExitablePointValues.getPointTree()`.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has fired, and
    /// otherwise propagates the delegate's errors.
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        self.check.check_and_throw()?;
        Ok(Box::new(ExitablePointTree::new(
            self.inner.point_tree()?,
            Arc::clone(self.check.query_timeout()),
        )))
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        self.check.check_and_throw()?;
        self.inner.min_packed_value()
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        self.check.check_and_throw()?;
        self.inner.max_packed_value()
    }

    fn num_dimensions(&self) -> Result<i32> {
        self.check.check_and_throw()?;
        self.inner.num_dimensions()
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        self.check.check_and_throw()?;
        self.inner.num_index_dimensions()
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        self.check.check_and_throw()?;
        self.inner.bytes_per_dimension()
    }

    /// Equivalent to `ExitablePointValues.size()`, which checks the timeout
    /// before answering.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has fired, and
    /// otherwise propagates the delegate's errors.
    fn size(&self) -> Result<i64> {
        self.check.check_and_throw()?;
        self.inner.size()
    }

    /// Equivalent to `ExitablePointValues.getDocCount()`. See
    /// [`size`](Self::size).
    ///
    /// # Errors
    ///
    /// As [`size`](Self::size).
    fn doc_count(&self) -> Result<i32> {
        self.check.check_and_throw()?;
        self.inner.doc_count()
    }
}

/// Cursor over the nodes of a KD-tree that checks a [`QueryTimeout`] as it is
/// walked.
///
/// Equivalent to `ExitableDirectoryReader.ExitablePointTree`, which samples the
/// timeout on navigation and bounds, checks it outright on `size()` and on both
/// visit methods, and routes `visitDocValues` through a single
/// [`ExitableIntersectVisitor`] it keeps for its own lifetime.
///
/// **Divergence from Lucene 10.5.0.** Java's `ExitablePointTree` also holds the
/// `PointValues` the tree came from (`ExitableDirectoryReader.java:695,703`),
/// and uses it for one thing only: rendering the object in the
/// `ExitingReaderException` message. This port has no such field. Lucene's way
/// could not be used because holding it would mean owning or borrowing the
/// `PointValues` that owns this tree, which Rust's ownership rules refuse
/// without reference counting the port has no other use for — and the message
/// this port raises carries no object rendering to put it in (see
/// [`exiting_reader_error`]).
struct ExitablePointTree {
    inner: Box<dyn PointTree>,
    check: ExitableCheck,
    /// The call counter of the one `ExitableIntersectVisitor` this tree reuses,
    /// so that its sampling carries across `visit_doc_values` calls exactly as
    /// Java's single instance does.
    visitor_check: ExitableCheck,
}

impl ExitablePointTree {
    /// Wraps `point_tree`, checking `query_timeout` as it is walked.
    ///
    /// Equivalent to `ExitablePointTree(PointValues, PointTree, QueryTimeout)`,
    /// which also builds the `ExitableIntersectVisitor` it will reuse.
    fn new(point_tree: Box<dyn PointTree>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: point_tree,
            check: ExitableCheck::new(Arc::clone(&query_timeout), "intersect point values"),
            visitor_check: ExitableCheck::new(query_timeout, "intersect point values"),
        }
    }
}

impl PointTree for ExitablePointTree {
    /// Equivalent to `ExitablePointTree.clone()`, which checks the timeout and
    /// re-roots a fresh wrapper — with a fresh visitor, as Java's constructor
    /// builds one.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has fired, and
    /// otherwise propagates the delegate's errors.
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        self.check.check_and_throw()?;
        Ok(Box::new(Self::new(
            self.inner.clone_tree()?,
            Arc::clone(self.check.query_timeout()),
        )))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        self.inner.move_to_child()
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        self.inner.move_to_sibling()
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        self.inner.move_to_parent()
    }

    /// Equivalent to `ExitablePointTree.getMinPackedValue()`, which samples the
    /// timeout before answering.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error on a sampled call once the timeout has
    /// fired, and otherwise propagates the delegate's errors.
    fn min_packed_value(&self) -> Result<&[u8]> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        self.inner.min_packed_value()
    }

    /// Equivalent to `ExitablePointTree.getMaxPackedValue()`. See
    /// [`min_packed_value`](Self::min_packed_value).
    ///
    /// # Errors
    ///
    /// As [`min_packed_value`](Self::min_packed_value).
    fn max_packed_value(&self) -> Result<&[u8]> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        self.inner.max_packed_value()
    }

    /// Equivalent to `ExitablePointTree.size()`, which checks the timeout
    /// outright rather than by sampling.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has fired, and
    /// otherwise propagates the delegate's errors.
    fn size(&self) -> Result<i64> {
        self.check.check_and_throw()?;
        self.inner.size()
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        self.check.check_and_throw()?;
        self.inner.visit_doc_ids(visitor)
    }

    /// Equivalent to `ExitablePointTree.visitDocValues(IntersectVisitor)`,
    /// which checks the timeout and then hands the delegate its own
    /// [`ExitableIntersectVisitor`] wrapped around `visitor`.
    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        self.check.check_and_throw()?;
        let Self {
            inner,
            visitor_check,
            ..
        } = self;
        let mut exitable_intersect_visitor = ExitableIntersectVisitor {
            inner: visitor,
            check: visitor_check,
        };
        inner.visit_doc_values(&mut exitable_intersect_visitor)
    }
}

/// An [`IntersectVisitor`] that checks a [`QueryTimeout`] as the delegate walks
/// the tree through it.
///
/// Equivalent to `ExitableDirectoryReader.ExitableIntersectVisitor`, which
/// samples the timeout on both `visit` methods and checks it outright on
/// `compare` and `grow`.
///
/// Java sets the wrapped visitor into a long-lived instance with
/// `setIntersectVisitor`; this port borrows it for the one traversal instead
/// and keeps the counter that the sampling reads in the
/// [`ExitablePointTree`] that owns the wrapper, so the sampling still carries
/// across traversals exactly as Java's single instance makes it.
struct ExitableIntersectVisitor<'a> {
    inner: &'a mut dyn IntersectVisitor,
    check: &'a ExitableCheck,
}

impl IntersectVisitor for ExitableIntersectVisitor<'_> {
    /// Equivalent to `ExitableIntersectVisitor.visit(int)`.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error on a sampled call once the timeout has
    /// fired, and otherwise propagates the delegate's errors.
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        self.inner.visit(doc_id)
    }

    /// Equivalent to `ExitableIntersectVisitor.visit(int, byte[])`.
    ///
    /// # Errors
    ///
    /// As [`visit`](Self::visit).
    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        self.inner.visit_with_value(doc_id, packed_value)
    }

    /// Equivalent to `ExitableIntersectVisitor.compare(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has fired, and
    /// otherwise propagates the delegate's errors.
    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        self.check.check_and_throw()?;
        self.inner.compare(min_packed_value, max_packed_value)
    }

    /// Equivalent to `ExitableIntersectVisitor.grow(int)`. See
    /// [`compare`](Self::compare).
    ///
    /// # Errors
    ///
    /// As [`compare`](Self::compare).
    fn grow(&mut self, count: i32) -> Result<()> {
        self.check.check_and_throw()?;
        self.inner.grow(count)
    }
}

/// The [`AcceptDocs`] an [`ExitableFilterLeafReader`] hands to a nearest-vector
/// search, so that consulting it checks the [`QueryTimeout`].
///
/// Equivalent to `ExitableDirectoryReader.ExitableFilterAtomicReader.ExitableAcceptDocs`.
///
/// **Divergence from Lucene 10.5.0.** Java's `bits()` wraps whatever the
/// delegate answers, substituting `Bits.MatchAllBits` when it answers `null`
/// "to allow timeout checking", and the wrapper throws
/// `ExitingReaderException` straight out of `Bits.get(int)`. This port wraps
/// only the substituted `MatchAllBits`. Two things stand in the way of wrapping
/// the delegate's own bits: `AcceptDocs::bits` returns a borrow of the
/// delegate, and a wrapper holding it would be a self-referential struct, which
/// Rust does not allow without `unsafe`; and [`Bits::get`] returns a plain
/// `bool`, so no wrapper could report the timeout from there anyway — the check
/// is deferred to [`finish`](Self::finish), which reports it once the search
/// returns. A leaf with deletions therefore loses the timeout check on the
/// random-access path only; the iterator path, the doc-values path, the terms
/// path, the points path and the vector-values path all keep theirs.
struct ExitableAcceptDocs<'a> {
    inner: &'a mut dyn AcceptDocs,
    max_doc: i32,
    /// The substituted match-all bits, wrapped. `None` until `bits()` is first
    /// called, and left `None` when the delegate has bits of its own.
    bits: Option<ExitableBits>,
    /// Set once the delegate has been found to answer with bits of its own.
    delegate_bits: bool,
    check: Arc<ExitableCheck>,
}

impl<'a> ExitableAcceptDocs<'a> {
    /// Wraps `accept_docs` for a leaf of `max_doc` documents.
    ///
    /// Equivalent to `ExitableAcceptDocs(AcceptDocs)`, whose enclosing reader
    /// supplies `maxDoc()`.
    fn new(
        accept_docs: &'a mut dyn AcceptDocs,
        max_doc: i32,
        query_timeout: Arc<dyn QueryTimeout>,
    ) -> Self {
        Self {
            inner: accept_docs,
            max_doc,
            bits: None,
            delegate_bits: false,
            check: Arc::new(ExitableCheck::new(query_timeout, "search nearest vectors")),
        }
    }

    /// Reports a timeout observed while the search was consulting the bits,
    /// which [`Bits::get`] could not report itself, and otherwise returns
    /// `outcome` unchanged.
    ///
    /// This has no counterpart in Java, where the exception leaves `Bits.get`
    /// directly; see the divergence recorded on the type.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when a deferred check had fired.
    fn finish(&self, outcome: Result<()>) -> Result<()> {
        self.check.take_deferred()?;
        outcome
    }
}

impl AcceptDocs for ExitableAcceptDocs<'_> {
    /// Equivalent to `ExitableAcceptDocs.bits()`.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn bits(&mut self) -> Result<Option<&dyn Bits>> {
        if self.bits.is_none() && !self.delegate_bits {
            // When acceptDocs is null due to no doc deleted, we will
            // instantiate a new one that would match all docs to allow timeout
            // checking.
            if self.inner.bits()?.is_none() {
                self.bits = Some(ExitableBits {
                    inner: Box::new(MatchAllBits::new(self.max_doc as usize)),
                    check: Arc::clone(&self.check),
                });
            } else {
                self.delegate_bits = true;
            }
        }
        match self.bits.as_ref() {
            Some(bits) => Ok(Some(bits as &dyn Bits)),
            None => self.inner.bits(),
        }
    }

    /// Equivalent to `ExitableAcceptDocs.iterator()`, which wraps the
    /// delegate's iterator so that it checks the timeout every
    /// [`DOCS_BETWEEN_TIMEOUT_CHECK`] documents.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        Ok(Box::new(ExitableAcceptDocsIterator {
            inner: FilterDocIdSetIterator::new(self.inner.iterator()?),
            check: ExitableCheck::new(
                Arc::clone(self.check.query_timeout()),
                "iterate over doc values",
            ),
            doc_to_check: 0,
        }))
    }

    /// Equivalent to `ExitableAcceptDocs.cost()`, which is plain delegation.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn cost(&mut self) -> Result<i64> {
        self.inner.cost()
    }
}

/// The [`Bits`] that [`ExitableAcceptDocs::bits`] answers with.
///
/// Equivalent to the anonymous `Bits` of `ExitableAcceptDocs.bits()`, which
/// checks the timeout on one call in
/// [`MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK`].
struct ExitableBits {
    inner: Box<dyn Bits>,
    check: Arc<ExitableCheck>,
}

impl std::fmt::Debug for ExitableBits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExitableBits").finish_non_exhaustive()
    }
}

impl Bits for ExitableBits {
    /// Equivalent to the anonymous `Bits.get(int)`, whose sampled check is
    /// deferred here — see the divergence recorded on [`ExitableAcceptDocs`].
    fn get(&self, index: usize) -> bool {
        self.check
            .defer_check_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK);
        self.inner.get(index)
    }

    fn length(&self) -> usize {
        self.inner.length()
    }
}

/// The [`DocIdSetIterator`] that [`ExitableAcceptDocs::iterator`] answers with.
///
/// Equivalent to the anonymous `FilterDocIdSetIterator` of
/// `ExitableAcceptDocs.iterator()`, which checks the timeout every
/// [`DOCS_BETWEEN_TIMEOUT_CHECK`] documents. Note that it raises the
/// doc-values wording, as Java's `checkAndThrow(DocIdSetIterator)` does.
struct ExitableAcceptDocsIterator {
    inner: FilterDocIdSetIterator,
    check: ExitableCheck,
    /// The next doc ID at or past which the timeout is checked again. Java's
    /// `int` field wraps on overflow, matched here with `wrapping_add`.
    doc_to_check: i32,
}

impl DocIdSetIterator for ExitableAcceptDocsIterator {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let next_doc = self.inner.next_doc()?;
        if next_doc >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = next_doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let advance = self.inner.advance(target)?;
        if advance >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = advance.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

/// [`FloatVectorValues`] that check a [`QueryTimeout`] as they are iterated.
///
/// Equivalent to `ExitableDirectoryReader.ExitableFilterAtomicReader.ExitableFloatVectorValues`,
/// which delegates every accessor and wraps only the iterator, and refuses to
/// be copied.
struct ExitableFloatVectorValues {
    vector_values: Box<dyn FloatVectorValues>,
    query_timeout: Arc<dyn QueryTimeout>,
}

impl ExitableFloatVectorValues {
    /// Wraps `vector_values`, checking `query_timeout` as they are iterated.
    ///
    /// Equivalent to `ExitableFloatVectorValues(FloatVectorValues)`.
    fn new(
        vector_values: Box<dyn FloatVectorValues>,
        query_timeout: Arc<dyn QueryTimeout>,
    ) -> Self {
        Self {
            vector_values,
            query_timeout,
        }
    }
}

impl KnnVectorValues for ExitableFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.vector_values.dimension()
    }

    fn size(&self) -> i32 {
        self.vector_values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.vector_values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(
        &self,
        accept_docs: Option<&'a dyn crate::util::Bits>,
    ) -> Option<Box<dyn crate::util::Bits + 'a>> {
        self.vector_values.get_accept_ords(accept_docs)
    }

    /// Equivalent to `ExitableFloatVectorValues.copy()`, which throws
    /// `UnsupportedOperationException`.
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "ExitableFloatVectorValues::copy".to_string(),
        ))
    }

    /// `ExitableFloatVectorValues` overrides no `getEncoding()`, so
    /// `FloatVectorValues`' own override applies.
    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    /// Equivalent to `ExitableFloatVectorValues.iterator()`.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(create_exitable_iterator(
            self.vector_values.iterator()?,
            Arc::clone(&self.query_timeout),
        ))
    }
}

impl FloatVectorValues for ExitableFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        self.vector_values.vector_value(ord)
    }

    /// Equivalent to `ExitableFloatVectorValues.copy()` in its covariant form.
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "ExitableFloatVectorValues::copy".to_string(),
        ))
    }

    /// Equivalent to `ExitableFloatVectorValues.scorer(float[])`.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn scorer(&self, target: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.vector_values.scorer(target)
    }
}

/// [`ByteVectorValues`] that check a [`QueryTimeout`] as they are iterated.
///
/// Equivalent to `ExitableDirectoryReader.ExitableFilterAtomicReader.ExitableByteVectorValues`.
struct ExitableByteVectorValues {
    vector_values: Box<dyn ByteVectorValues>,
    query_timeout: Arc<dyn QueryTimeout>,
}

impl ExitableByteVectorValues {
    /// Wraps `vector_values`, checking `query_timeout` as they are iterated.
    ///
    /// Equivalent to `ExitableByteVectorValues(ByteVectorValues)`.
    fn new(vector_values: Box<dyn ByteVectorValues>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            vector_values,
            query_timeout,
        }
    }
}

impl KnnVectorValues for ExitableByteVectorValues {
    fn dimension(&self) -> i32 {
        self.vector_values.dimension()
    }

    fn size(&self) -> i32 {
        self.vector_values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.vector_values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(
        &self,
        accept_docs: Option<&'a dyn crate::util::Bits>,
    ) -> Option<Box<dyn crate::util::Bits + 'a>> {
        self.vector_values.get_accept_ords(accept_docs)
    }

    /// Equivalent to `ExitableByteVectorValues.copy()`, which throws
    /// `UnsupportedOperationException`.
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "ExitableByteVectorValues::copy".to_string(),
        ))
    }

    /// `ExitableByteVectorValues` overrides no `getEncoding()`, so
    /// `ByteVectorValues`' own override applies.
    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    /// Equivalent to `ExitableByteVectorValues.iterator()`.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(create_exitable_iterator(
            self.vector_values.iterator()?,
            Arc::clone(&self.query_timeout),
        ))
    }
}

impl ByteVectorValues for ExitableByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        self.vector_values.vector_value(ord)
    }

    /// Equivalent to `ExitableByteVectorValues.copy()` in its covariant form.
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "ExitableByteVectorValues::copy".to_string(),
        ))
    }

    /// Equivalent to `ExitableByteVectorValues.scorer(byte[])`.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn scorer(&self, target: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.vector_values.scorer(target)
    }
}

/// [`NumericDocValues`] that check a [`QueryTimeout`] as they are iterated.
///
/// Equivalent to the anonymous `FilterNumericDocValues` that
/// `ExitableDirectoryReader.ExitableFilterAtomicReader.wrapNumericDocValues`
/// returns, which checks the timeout once every
/// [`DOCS_BETWEEN_TIMEOUT_CHECK`] documents.
struct ExitableNumericDocValues {
    inner: FilterNumericDocValues,
    check: ExitableCheck,
    /// The next doc ID at or past which the timeout is checked again. Java's
    /// `int` field wraps on overflow, matched here with `wrapping_add`.
    doc_to_check: i32,
}

impl ExitableNumericDocValues {
    /// Wraps `doc_values`, checking `query_timeout` as they are iterated.
    fn new(doc_values: Box<dyn NumericDocValues>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: FilterNumericDocValues::new(doc_values),
            check: ExitableCheck::new(query_timeout, "iterate over doc values"),
            doc_to_check: 0,
        }
    }
}

impl DocIdSetIterator for ExitableNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let next_doc = self.inner.next_doc()?;
        if next_doc >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = next_doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let advance = self.inner.advance(target)?;
        if advance >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = advance.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for ExitableNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let advance_exact = self.inner.advance_exact(target)?;
        if target >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = target.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance_exact)
    }
}

impl NumericDocValues for ExitableNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        self.inner.long_value()
    }
}

/// [`BinaryDocValues`] that check a [`QueryTimeout`] as they are iterated.
///
/// Equivalent to the anonymous `FilterBinaryDocValues` that
/// `ExitableDirectoryReader.ExitableFilterAtomicReader.getBinaryDocValues`
/// returns.
struct ExitableBinaryDocValues {
    inner: FilterBinaryDocValues,
    check: ExitableCheck,
    doc_to_check: i32,
}

impl ExitableBinaryDocValues {
    /// Wraps `doc_values`, checking `query_timeout` as they are iterated.
    fn new(doc_values: Box<dyn BinaryDocValues>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: FilterBinaryDocValues::new(doc_values),
            check: ExitableCheck::new(query_timeout, "iterate over doc values"),
            doc_to_check: 0,
        }
    }
}

impl DocIdSetIterator for ExitableBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let next_doc = self.inner.next_doc()?;
        if next_doc >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = next_doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(next_doc)
    }

    /// Equivalent to the anonymous `FilterBinaryDocValues.advance(int)`, which
    /// — alone among the five doc-values wrappers — decides on the *target*
    /// rather than on the document it landed on. Reproduced as written in
    /// `ExitableDirectoryReader.java:163-171`.
    fn advance(&mut self, target: i32) -> Result<i32> {
        let advance = self.inner.advance(target)?;
        if target >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = target.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for ExitableBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let advance_exact = self.inner.advance_exact(target)?;
        if target >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = target.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance_exact)
    }
}

impl BinaryDocValues for ExitableBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        self.inner.binary_value()
    }
}

/// [`SortedDocValues`] that check a [`QueryTimeout`] as they are iterated.
///
/// Equivalent to the anonymous `FilterSortedDocValues` that
/// `ExitableDirectoryReader.ExitableFilterAtomicReader.wrapSortedDocValues`
/// returns.
struct ExitableSortedDocValues {
    inner: FilterSortedDocValues,
    check: ExitableCheck,
    doc_to_check: i32,
}

impl ExitableSortedDocValues {
    /// Wraps `doc_values`, checking `query_timeout` as they are iterated.
    fn new(doc_values: Box<dyn SortedDocValues>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: FilterSortedDocValues::new(doc_values),
            check: ExitableCheck::new(query_timeout, "iterate over doc values"),
            doc_to_check: 0,
        }
    }
}

impl DocIdSetIterator for ExitableSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let next_doc = self.inner.next_doc()?;
        if next_doc >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = next_doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let advance = self.inner.advance(target)?;
        if advance >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = advance.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for ExitableSortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let advance_exact = self.inner.advance_exact(target)?;
        if target >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = target.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance_exact)
    }
}

impl SortedDocValues for ExitableSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        self.inner.ord_value()
    }

    fn get_value_count(&self) -> Result<i32> {
        self.inner.get_value_count()
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        self.inner.lookup_ord(ord)
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i32> {
        self.inner.lookup_term(key)
    }

    fn terms_enum(self: Box<Self>) -> Result<Box<dyn TermsEnum>>
    where
        Self: 'static,
    {
        Box::new(self.inner).terms_enum()
    }

    fn intersect(self: Box<Self>, automaton: &CompiledAutomaton) -> Result<Box<dyn TermsEnum>>
    where
        Self: 'static,
    {
        Box::new(self.inner).intersect(automaton)
    }
}

/// [`SortedNumericDocValues`] that check a [`QueryTimeout`] as they are
/// iterated.
///
/// Equivalent to the anonymous `FilterSortedNumericDocValues` that
/// `ExitableDirectoryReader.ExitableFilterAtomicReader.getSortedNumericDocValues`
/// returns for a field that is not a singleton view.
struct ExitableSortedNumericDocValues {
    inner: FilterSortedNumericDocValues,
    check: ExitableCheck,
    doc_to_check: i32,
}

impl ExitableSortedNumericDocValues {
    /// Wraps `doc_values`, checking `query_timeout` as they are iterated.
    fn new(
        doc_values: Box<dyn SortedNumericDocValues>,
        query_timeout: Arc<dyn QueryTimeout>,
    ) -> Self {
        Self {
            inner: FilterSortedNumericDocValues::new(doc_values),
            check: ExitableCheck::new(query_timeout, "iterate over doc values"),
            doc_to_check: 0,
        }
    }
}

impl DocIdSetIterator for ExitableSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let next_doc = self.inner.next_doc()?;
        if next_doc >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = next_doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let advance = self.inner.advance(target)?;
        if advance >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = advance.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for ExitableSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let advance_exact = self.inner.advance_exact(target)?;
        if target >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = target.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance_exact)
    }
}

impl SortedNumericDocValues for ExitableSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        self.inner.next_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.inner.doc_value_count()
    }
}

/// [`SortedSetDocValues`] that check a [`QueryTimeout`] as they are iterated.
///
/// Equivalent to the anonymous `FilterSortedSetDocValues` that
/// `ExitableDirectoryReader.ExitableFilterAtomicReader.getSortedSetDocValues`
/// returns for a field that is not a singleton view.
struct ExitableSortedSetDocValues {
    inner: FilterSortedSetDocValues,
    check: ExitableCheck,
    doc_to_check: i32,
}

impl ExitableSortedSetDocValues {
    /// Wraps `doc_values`, checking `query_timeout` as they are iterated.
    fn new(doc_values: Box<dyn SortedSetDocValues>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: FilterSortedSetDocValues::new(doc_values),
            check: ExitableCheck::new(query_timeout, "iterate over doc values"),
            doc_to_check: 0,
        }
    }
}

impl DocIdSetIterator for ExitableSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let next_doc = self.inner.next_doc()?;
        if next_doc >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = next_doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let advance = self.inner.advance(target)?;
        if advance >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = advance.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for ExitableSortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let advance_exact = self.inner.advance_exact(target)?;
        if target >= self.doc_to_check {
            self.check.check_and_throw()?;
            self.doc_to_check = target.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(advance_exact)
    }
}

impl SortedSetDocValues for ExitableSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        self.inner.next_ord()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.inner.doc_value_count()
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.inner.lookup_ord(ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        self.inner.get_value_count()
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i64> {
        self.inner.lookup_term(key)
    }

    fn terms_enum(self: Box<Self>) -> Result<Box<dyn TermsEnum>>
    where
        Self: 'static,
    {
        Box::new(self.inner).terms_enum()
    }

    fn intersect(self: Box<Self>, automaton: &CompiledAutomaton) -> Result<Box<dyn TermsEnum>>
    where
        Self: 'static,
    {
        Box::new(self.inner).intersect(automaton)
    }
}

/// Wraps `delegate`, checking `query_timeout` every
/// [`DOCS_BETWEEN_TIMEOUT_CHECK`] documents it iterates.
///
/// Equivalent to `ExitableDirectoryReader.createExitableIterator(DocIndexIterator, QueryTimeout)`.
fn create_exitable_iterator(
    delegate: Box<dyn crate::index::vector_values::DocIndexIterator>,
    query_timeout: Arc<dyn QueryTimeout>,
) -> Box<dyn crate::index::vector_values::DocIndexIterator> {
    Box::new(ExitableDocIndexIterator {
        delegate,
        query_timeout,
        next_check: 0,
    })
}

/// The iterator [`create_exitable_iterator`] returns.
///
/// Equivalent to the anonymous `KnnVectorValues.DocIndexIterator` of
/// `ExitableDirectoryReader.createExitableIterator`.
struct ExitableDocIndexIterator {
    delegate: Box<dyn crate::index::vector_values::DocIndexIterator>,
    query_timeout: Arc<dyn QueryTimeout>,
    /// The next doc ID at or past which the timeout is checked again. Java's
    /// `int` field wraps on overflow, matched here with `wrapping_add`.
    next_check: i32,
}

impl std::fmt::Debug for ExitableDocIndexIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExitableDocIndexIterator")
            .field("next_check", &self.next_check)
            .finish_non_exhaustive()
    }
}

impl ExitableDocIndexIterator {
    /// Equivalent to the private `checkAndThrow` of
    /// `ExitableDirectoryReader.createExitableIterator`.
    fn check_and_throw(&self) -> Result<()> {
        if self.query_timeout.should_exit() {
            return Err(exiting_reader_error("knn vector values"));
        }
        Ok(())
    }
}

impl crate::search::DocIdSetIterator for ExitableDocIndexIterator {
    fn doc_id(&self) -> i32 {
        self.delegate.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.delegate.next_doc()?;
        if doc >= self.next_check {
            self.check_and_throw()?;
            self.next_check = doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.delegate.advance(target)?;
        if doc >= self.next_check {
            self.check_and_throw()?;
            self.next_check = doc.wrapping_add(DOCS_BETWEEN_TIMEOUT_CHECK);
        }
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.delegate.cost()
    }
}

impl crate::index::vector_values::DocIndexIterator for ExitableDocIndexIterator {
    fn index(&self) -> i32 {
        self.delegate.index()
    }
}

/// Turns any [`LeafReader`] into a [`CodecReader`].
///
/// Equivalent to `org.apache.lucene.index.SlowCodecReaderWrapper`. Note that
/// this can make merges slow and memory-intensive; prefer
/// [`FilterCodecReader`](crate::index::filter_reader::FilterCodecReader) where
/// one will do.
pub struct SlowCodecReaderWrapper;

impl SlowCodecReaderWrapper {
    /// Returns a codec-reader view of `reader`.
    ///
    /// Equivalent to `SlowCodecReaderWrapper.wrap(LeafReader)`: a reader that
    /// is already a codec reader is returned directly, and any other is checked
    /// for integrity once and then served slowly over its own leaf-reader API.
    ///
    /// # Errors
    ///
    /// Propagates whatever the integrity check raises.
    pub fn wrap(reader: Arc<dyn LeafReader>) -> Result<Arc<dyn CodecReader>> {
        // Simulate it slowly, over the leaf-reader API.
        reader.check_integrity()?;
        Ok(Arc::new(SlowCodecReader::new(reader)))
    }

    /// Returns a codec-reader view of a reader that already is one.
    ///
    /// This is the branch Java takes with `reader instanceof CodecReader`; Rust
    /// cannot downcast a `dyn LeafReader` to a `dyn CodecReader`, so the caller
    /// that holds the codec reader calls this instead of
    /// [`wrap`](Self::wrap).
    ///
    /// # Errors
    ///
    /// Propagates whatever the integrity check raises.
    pub fn wrap_codec_reader(reader: Arc<dyn CodecReader>) -> Result<Arc<dyn CodecReader>> {
        reader.check_integrity()?;
        Ok(reader)
    }

    /// Equivalent to the private
    /// `SlowCodecReaderWrapper.pointValuesToReader(LeafReader)`.
    fn point_values_to_reader(reader: Arc<dyn LeafReader>) -> SlowPointsReader {
        SlowPointsReader { reader }
    }

    /// Equivalent to the private
    /// `SlowCodecReaderWrapper.readerToVectorReader(LeafReader)`.
    fn reader_to_vector_reader(reader: Arc<dyn LeafReader>) -> SlowKnnVectorsReader {
        SlowKnnVectorsReader { reader }
    }

    /// Equivalent to the private
    /// `SlowCodecReaderWrapper.readerToNormsProducer(LeafReader)`.
    fn reader_to_norms_producer(reader: Arc<dyn LeafReader>) -> SlowNormsProducer {
        SlowNormsProducer { reader }
    }

    /// Equivalent to the private
    /// `SlowCodecReaderWrapper.readerToDocValuesProducer(LeafReader)`.
    fn reader_to_doc_values_producer(reader: Arc<dyn LeafReader>) -> SlowDocValuesProducer {
        SlowDocValuesProducer { reader }
    }

    /// Equivalent to the private
    /// `SlowCodecReaderWrapper.readerToStoredFieldsReader(LeafReader)`.
    fn reader_to_stored_fields_reader(reader: Arc<dyn LeafReader>) -> SlowStoredFieldsReader {
        SlowStoredFieldsReader { reader }
    }

    /// Equivalent to the private
    /// `SlowCodecReaderWrapper.readerToTermVectorsReader(LeafReader)`.
    fn reader_to_term_vectors_reader(reader: Arc<dyn LeafReader>) -> SlowTermVectorsReader {
        SlowTermVectorsReader { reader }
    }

    /// Equivalent to the private
    /// `SlowCodecReaderWrapper.readerToFieldsProducer(LeafReader)`.
    fn reader_to_fields_producer(reader: Arc<dyn LeafReader>) -> Result<SlowFieldsProducer> {
        let mut indexed_fields: Vec<String> = reader
            .get_field_infos()
            .iter()
            .filter(|fi| fi.get_index_options() != IndexOptions::NONE)
            .map(|fi| fi.name.clone())
            .collect();
        indexed_fields.sort();
        Ok(SlowFieldsProducer {
            reader,
            indexed_fields,
        })
    }
}

/// A [`CodecReader`] view over an arbitrary [`LeafReader`].
///
/// Equivalent to the anonymous `CodecReader` `SlowCodecReaderWrapper.wrap`
/// returns: every accessor re-serves the wrapped reader's own API, so a reader
/// that is not a codec reader can still be merged. The integrity of the whole
/// reader is checked once, up front, which is why each synthesised producer's
/// `checkIntegrity` does nothing.
#[derive(Debug)]
pub struct SlowCodecReader {
    reader: Arc<dyn LeafReader>,
}

impl SlowCodecReader {
    /// Wraps `reader`, which the caller has already integrity-checked.
    fn new(reader: Arc<dyn LeafReader>) -> Self {
        Self { reader }
    }
}

impl std::fmt::Display for SlowCodecReader {
    /// Port of the `toString()` the anonymous `CodecReader` in
    /// `SlowCodecReaderWrapper.wrap` overrides
    /// (`SlowCodecReaderWrapper.java:135-138`): `"SlowCodecReaderWrapper(" + reader + ")"`,
    /// where `reader` is rendered by its **own** `toString()`.
    ///
    /// Measured against `lucene-core-10.5.0.jar`: over a `ParallelLeafReader`
    /// this is `SlowCodecReaderWrapper(ParallelLeafReader(_0(10.5.0):C3, _1(10.5.0):C3))`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SlowCodecReaderWrapper({})", self.reader)
    }
}

impl LeafReader for SlowCodecReader {
    fn core(&self) -> &IndexReaderCore {
        self.reader.core()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.reader.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.reader.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.reader.max_doc()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.reader.stored_fields()
    }

    fn do_close(&self) -> Result<()> {
        self.reader.do_close()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.reader.get_reader_cache_helper()
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.reader.get_core_cache_helper()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.reader.terms(field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.reader.get_numeric_doc_values(field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.reader.get_binary_doc_values(field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.reader.get_sorted_doc_values(field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.reader.get_sorted_numeric_doc_values(field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.reader.get_sorted_set_doc_values(field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.reader.get_norm_values(field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.reader.get_doc_values_skipper(field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.reader.get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.reader.get_byte_vector_values(field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.reader
            .search_nearest_vectors(field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.reader
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.reader.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.reader.get_live_docs()
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        self.reader.get_point_values(field)
    }

    fn check_integrity(&self) -> Result<()> {
        self.reader.check_integrity()
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.reader.get_meta_data()
    }
}

impl CodecReader for SlowCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        Ok(Some(Box::new(
            SlowCodecReaderWrapper::reader_to_stored_fields_reader(Arc::clone(&self.reader)),
        )))
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        Ok(Some(Box::new(
            SlowCodecReaderWrapper::reader_to_term_vectors_reader(Arc::clone(&self.reader)),
        )))
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        Ok(Some(Arc::new(
            SlowCodecReaderWrapper::reader_to_norms_producer(Arc::clone(&self.reader)),
        )))
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        Ok(Some(Arc::new(
            SlowCodecReaderWrapper::reader_to_doc_values_producer(Arc::clone(&self.reader)),
        )))
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        Ok(Some(Arc::new(
            SlowCodecReaderWrapper::reader_to_fields_producer(Arc::clone(&self.reader))?,
        )))
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        Ok(Some(Arc::new(
            SlowCodecReaderWrapper::point_values_to_reader(Arc::clone(&self.reader)),
        )))
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        Ok(Some(Arc::new(
            SlowCodecReaderWrapper::reader_to_vector_reader(Arc::clone(&self.reader)),
        )))
    }
}

/// The [`PointsReader`] a slow codec reader synthesises.
///
/// Equivalent to the anonymous reader of the private
/// `SlowCodecReaderWrapper.pointValuesToReader(LeafReader)`.
#[derive(Debug)]
struct SlowPointsReader {
    reader: Arc<dyn LeafReader>,
}

impl PointsReader for SlowPointsReader {
    fn check_integrity(&self) -> Result<()> {
        // The whole reader was already integrity-checked, up front.
        Ok(())
    }

    /// Equivalent to `SlowCodecReaderWrapper`'s `getValues`
    /// (`SlowCodecReaderWrapper.java:151-154`): the reader's own answer,
    /// `null` included.
    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        self.reader.get_point_values(field)
    }

    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Self {
            reader: Arc::clone(&self.reader),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The [`KnnVectorsReader`] a slow codec reader synthesises.
///
/// Equivalent to the anonymous reader of the private
/// `SlowCodecReaderWrapper.readerToVectorReader(LeafReader)`.
#[derive(Debug)]
struct SlowKnnVectorsReader {
    reader: Arc<dyn LeafReader>,
}

impl KnnVectorsReader for SlowKnnVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        // The whole reader was already integrity-checked, up front.
        Ok(())
    }

    /// Equivalent to the vectors reader of `SlowCodecReaderWrapper`, which
    /// answers `reader.getFloatVectorValues(field)` as it is, `null` included.
    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.reader.get_float_vector_values(field)
    }

    /// Equivalent to the vectors reader of `SlowCodecReaderWrapper`, which
    /// answers `reader.getByteVectorValues(field)` as it is, `null` included.
    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.reader.get_byte_vector_values(field)
    }

    fn search(
        &self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.reader
            .search_nearest_vectors(field, target, knn_collector, accept_docs)
    }

    fn search_byte(
        &self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.reader
            .search_nearest_vectors_byte(field, target, knn_collector, accept_docs)
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(Self {
            reader: Arc::clone(&self.reader),
        }))
    }

    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        // Java unwraps the reader down to its SegmentReader and asks its own
        // vectors reader, going through PerFieldKnnVectorsFormat when one is in
        // the way. `as_segment_reader` is this port's unwrap, and it answers
        // for the same reader Java reaches.
        match self.reader.as_segment_reader() {
            Some(segment_reader) => match segment_reader.get_vector_reader() {
                Ok(Some(vectors_reader)) => vectors_reader.get_off_heap_byte_size(field_info),
                _ => HashMap::new(),
            },
            None => HashMap::new(),
        }
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The [`NormsProducer`] a slow codec reader synthesises.
///
/// Equivalent to the anonymous producer of the private
/// `SlowCodecReaderWrapper.readerToNormsProducer(LeafReader)`.
#[derive(Debug)]
struct SlowNormsProducer {
    reader: Arc<dyn LeafReader>,
}

impl NormsProducer for SlowNormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        self.reader.get_norm_values(&field.name)?.ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field \"{}\" has no norms", field.name))
        })
    }

    fn check_integrity(&self) -> Result<()> {
        // The whole reader was already integrity-checked, up front.
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(Self {
            reader: Arc::clone(&self.reader),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The [`DocValuesProducer`] a slow codec reader synthesises.
///
/// Equivalent to the anonymous producer of the private
/// `SlowCodecReaderWrapper.readerToDocValuesProducer(LeafReader)`.
#[derive(Debug)]
struct SlowDocValuesProducer {
    reader: Arc<dyn LeafReader>,
}

impl DocValuesProducer for SlowDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        Ok(self
            .reader
            .get_numeric_doc_values(&field.name)?
            .unwrap_or_else(|| Box::new(DocValues::empty_numeric())))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
        Ok(self
            .reader
            .get_binary_doc_values(&field.name)?
            .unwrap_or_else(|| Box::new(DocValues::empty_binary())))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues>> {
        Ok(self
            .reader
            .get_sorted_doc_values(&field.name)?
            .unwrap_or_else(|| Box::new(DocValues::empty_sorted())))
    }

    fn get_sorted_numeric(&self, field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>> {
        Ok(self
            .reader
            .get_sorted_numeric_doc_values(&field.name)?
            .unwrap_or_else(|| Box::new(DocValues::empty_sorted_numeric())))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>> {
        Ok(self
            .reader
            .get_sorted_set_doc_values(&field.name)?
            .unwrap_or_else(|| Box::new(DocValues::empty_sorted_set())))
    }

    fn get_skipper(&self, field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>> {
        self.reader
            .get_doc_values_skipper(&field.name)?
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "field \"{}\" has no doc-values skip index",
                    field.name
                ))
            })
    }

    fn check_integrity(&self) -> Result<()> {
        // The whole reader was already integrity-checked, up front.
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            reader: Arc::clone(&self.reader),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The [`StoredFieldsReader`] a slow codec reader synthesises.
///
/// Equivalent to the anonymous reader of the private
/// `SlowCodecReaderWrapper.readerToStoredFieldsReader(LeafReader)`.
#[derive(Debug)]
struct SlowStoredFieldsReader {
    reader: Arc<dyn LeafReader>,
}

impl StoredFieldsReader for SlowStoredFieldsReader {
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        self.reader
            .stored_fields()?
            .document_with_visitor(doc_id, visitor)
    }

    fn check_integrity(&self) -> Result<()> {
        // The whole reader was already integrity-checked, up front.
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        Box::new(Self {
            reader: Arc::clone(&self.reader),
        })
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
        self.reader.stored_fields()?.prefetch(doc_id)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The [`TermVectorsReader`] a slow codec reader synthesises.
///
/// Equivalent to the anonymous reader of the private
/// `SlowCodecReaderWrapper.readerToTermVectorsReader(LeafReader)`.
#[derive(Debug)]
struct SlowTermVectorsReader {
    reader: Arc<dyn LeafReader>,
}

impl TermVectorsReader for SlowTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        // The whole reader was already integrity-checked, up front.
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(Self {
            reader: Arc::clone(&self.reader),
        })
    }

    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        self.reader.term_vectors()?.get(doc)
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
        self.reader.term_vectors()?.prefetch(doc_id)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The [`FieldsProducer`] a slow codec reader synthesises.
///
/// Equivalent to the anonymous producer of the private
/// `SlowCodecReaderWrapper.readerToFieldsProducer(LeafReader)`, which lists the
/// indexed fields once, sorted, and serves `terms` straight from the reader.
#[derive(Debug)]
struct SlowFieldsProducer {
    reader: Arc<dyn LeafReader>,
    indexed_fields: Vec<String>,
}

impl Fields for SlowFieldsProducer {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.indexed_fields.iter().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.reader.terms(field)
    }

    fn size(&self) -> i32 {
        self.indexed_fields.len() as i32
    }
}

impl FieldsProducer for SlowFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        // The whole reader was already integrity-checked, up front.
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            reader: Arc::clone(&self.reader),
            indexed_fields: self.indexed_fields.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Hides the documents a soft-deletes field marks as deleted.
///
/// Equivalent to `org.apache.lucene.index.SoftDeletesDirectoryReaderWrapper`.
pub struct SoftDeletesSubReaderWrapper {
    field: String,
}

impl SoftDeletesSubReaderWrapper {
    /// Creates the wrapper for the given soft-deletes field.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }

    /// Returns the soft-deletes field.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Computes the live-docs bitset that hides the soft-deleted documents of
    /// `reader`.
    ///
    /// Equivalent to `PendingSoftDeletes.applySoftDeletes`: every document the
    /// soft-deletes field marks is cleared from the reader's existing live docs.
    pub fn soft_deletes_live_docs(
        reader: &dyn LeafReader,
        field: &str,
    ) -> Result<(crate::util::FixedBitSet, i32)> {
        let max_doc = reader.max_doc();
        let mut bits = crate::util::FixedBitSet::new(max_doc as usize);
        match reader.get_live_docs() {
            Some(live_docs) => {
                for doc in 0..max_doc {
                    if live_docs.get(doc as usize) {
                        bits.set(doc as usize);
                    }
                }
            }
            None => {
                for doc in 0..max_doc {
                    bits.set(doc as usize);
                }
            }
        }

        let mut num_soft_deletes = 0i32;
        if let Some(mut values) = reader.get_numeric_doc_values(field)? {
            loop {
                let doc = values.next_doc()?;
                if doc == crate::search::NO_MORE_DOCS {
                    break;
                }
                if doc >= 0 && (doc as usize) < max_doc as usize && bits.get(doc as usize) {
                    bits.clear(doc as usize);
                    num_soft_deletes += 1;
                }
            }
        }
        Ok((bits, num_soft_deletes))
    }

    /// Wraps `reader`, hiding the documents `field` marks as soft-deleted.
    ///
    /// Equivalent to `SoftDeletesDirectoryReaderWrapper.wrap(LeafReader, String)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java branches to a
    /// `SoftDeletesFilterCodecReader` when `reader instanceof CodecReader`, to
    /// keep the wrapped reader's codec-level accessors available to merges.
    /// Rust cannot downcast a `dyn LeafReader` to a `dyn CodecReader` — the
    /// same constraint `SlowCodecReaderWrapper::wrap`/`wrap_codec_reader`
    /// already document in this file — so this always returns a
    /// [`SoftDeletesFilterLeafReader`]; reads are identical either way.
    ///
    /// # Errors
    ///
    /// Propagates whatever computing the live-docs bits, or
    /// [`assert_doc_counts`], raises.
    pub fn wrap_leaf_reader(
        reader: Arc<dyn LeafReader>,
        field: &str,
    ) -> Result<Arc<dyn LeafReader>> {
        let (bits, num_soft_deletes) = Self::soft_deletes_live_docs(reader.as_ref(), field)?;
        if num_soft_deletes == 0 {
            return Ok(reader);
        }
        let num_deletes = (reader.max_doc() - reader.num_docs()) + num_soft_deletes;
        let num_docs = reader.max_doc() - num_deletes;
        debug_assert!(assert_doc_counts(
            num_docs,
            num_soft_deletes,
            reader.as_ref()
        )?);
        Ok(Arc::new(SoftDeletesFilterLeafReader::new(
            reader, bits, num_docs,
        )))
    }
}

impl SubReaderWrapper for SoftDeletesSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Self::wrap_leaf_reader(reader, &self.field)
    }

    /// Equivalent to
    /// `SoftDeletesDirectoryReaderWrapper.SoftDeletesSubReaderWrapper.wrap(List)`:
    /// drops any leaf that ends up with zero live documents after the wrap,
    /// instead of the default that keeps every wrapped leaf.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::wrap`] raises.
    fn wrap_all(&self, readers: Vec<Arc<dyn LeafReader>>) -> Result<Vec<Arc<dyn LeafReader>>> {
        let mut wrapped = Vec::with_capacity(readers.len());
        for reader in readers {
            let wrap = self.wrap(reader)?;
            if wrap.num_docs() != 0 {
                wrapped.push(wrap);
            }
        }
        Ok(wrapped)
    }
}

/// Debug-only invariant check: the doc count
/// [`SoftDeletesSubReaderWrapper::wrap_leaf_reader`] computed must agree with
/// the segment's own committed counts.
///
/// Equivalent to the private
/// `SoftDeletesDirectoryReaderWrapper.assertDocCounts(int, int, LeafReader)`,
/// which Java only ever evaluates from inside an `assert` and which always
/// returns `true` — the real check is the inner `assert` that throws first.
/// This is ported the same way: a `debug_assert!` inside always returning
/// `Ok(true)`, meant to be called from inside a `debug_assert!` at the call
/// site so the whole check compiles away in release builds.
///
/// # Errors
///
/// Propagates whatever reading the segment's committed info raises.
fn assert_doc_counts(
    expected_num_docs: i32,
    num_soft_deletes: i32,
    reader: &dyn LeafReader,
) -> Result<bool> {
    if let Some(segment_reader) = reader.as_segment_reader() {
        if !segment_reader.is_nrt() {
            let segment_info = segment_reader.get_segment_info();
            let max_doc = segment_info.info.max_doc()?;
            let num_docs =
                max_doc - segment_info.get_soft_del_count() - segment_info.get_del_count();
            debug_assert_eq!(
                num_docs,
                expected_num_docs,
                "numDocs: {num_docs} expected: {expected_num_docs} maxDoc: {max_doc} \
                 getDelCount: {} getSoftDelCount: {} numSoftDeletes: {num_soft_deletes} \
                 reader.numDeletedDocs(): {}",
                segment_info.get_del_count(),
                segment_info.get_soft_del_count(),
                reader.max_doc() - reader.num_docs(),
            );
        }
    }
    Ok(true)
}

/// Serves a shared soft-deletes live-docs bitset where an owned `Box<dyn
/// Bits>` is expected.
///
/// Java hands out the `Bits` object itself; the crate's `get_live_docs`
/// returns an owned box, so a shared set is wrapped rather than cloned (the
/// same reasoning as `filter_reader::ArcBits`, which is private to that
/// module).
#[derive(Debug)]
struct SoftDeletesBits(Arc<crate::util::FixedBitSet>);

impl Bits for SoftDeletesBits {
    fn get(&self, index: usize) -> bool {
        self.0.get(index)
    }

    fn length(&self) -> usize {
        self.0.length()
    }
}

/// Hides the documents a soft-deletes field marks as deleted, reporting
/// `bits`/`num_docs` in place of the wrapped reader's own.
///
/// Equivalent to
/// `SoftDeletesDirectoryReaderWrapper.SoftDeletesFilterLeafReader`
/// (`SoftDeletesDirectoryReaderWrapper.java:189-225`). Its reader cache helper
/// is a [`DelegatingCacheHelper`] over the wrapped reader's, built once in the
/// constructor as Java builds it: the live documents differ from the wrapped
/// reader's, so the wrap needs its own cache identity. Its core cache helper is
/// the wrapped reader's, because the segment's data is unchanged. See
/// [`SoftDeletesSubReaderWrapper::wrap_leaf_reader`] for the codec-reader
/// divergence.
struct SoftDeletesFilterLeafReader {
    inner: FilterLeafReader,
    bits: Arc<crate::util::FixedBitSet>,
    num_docs: i32,
    reader_cache_helper: Option<Arc<DelegatingCacheHelper>>,
}

impl std::fmt::Display for SoftDeletesFilterLeafReader {
    /// `SoftDeletesDirectoryReaderWrapper.SoftDeletesFilterLeafReader`
    /// overrides no `toString()`, so `FilterLeafReader`'s applies — including
    /// its hardcoded name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}

impl SoftDeletesFilterLeafReader {
    fn new(reader: Arc<dyn LeafReader>, bits: crate::util::FixedBitSet, num_docs: i32) -> Self {
        let reader_cache_helper = delegating_cache_helper(reader.get_reader_cache_helper());
        Self {
            inner: FilterLeafReader::new(reader),
            bits: Arc::new(bits),
            num_docs,
            reader_cache_helper,
        }
    }
}

impl std::fmt::Debug for SoftDeletesFilterLeafReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoftDeletesFilterLeafReader")
            .field("num_docs", &self.num_docs)
            .finish_non_exhaustive()
    }
}

impl LeafReader for SoftDeletesFilterLeafReader {
    fn core(&self) -> &IndexReaderCore {
        LeafReader::core(&self.inner)
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }

    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }

    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }

    /// Equivalent to `SoftDeletesFilterLeafReader.getReaderCacheHelper()`: the
    /// [`DelegatingCacheHelper`] built in the constructor.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.reader_cache_helper
            .as_ref()
            .map(|helper| Box::new(Arc::clone(helper)) as Box<dyn CacheHelper>)
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_core_cache_helper()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.inner.terms(field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.inner.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        Some(Box::new(SoftDeletesBits(Arc::clone(&self.bits))))
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        self.inner.get_point_values(field)
    }

    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.inner.get_meta_data()
    }
}

/// Reproduces the same soft-deletes wrapping on a reopened [`DirectoryReader`].
///
/// Equivalent to
/// `SoftDeletesDirectoryReaderWrapper.doWrapDirectoryReader(DirectoryReader)`.
///
/// **Divergence from Lucene 10.5.0.** Java's override first walks
/// `getSequentialSubReaders()` and maps the reader cache key of each wrapped
/// leaf's *inner* reader to the wrapping leaf, so that a reopened leaf whose
/// key did not change reuses the previous wrap's live-docs instance instead of
/// recomputing it (`SoftDeletesDirectoryReaderWrapper.java:70-82`). That reuse
/// is not ported: it needs the previous leaves downcast back to
/// `SoftDeletesFilterLeafReader`, which the `dyn LeafReader` this wrapper
/// holds does not allow, so every leaf is recomputed on each reopen. The
/// wrapped result is identical either way, only slower to build.
pub struct SoftDeletesDirectoryReaderWrapper {
    field: String,
}

impl SoftDeletesDirectoryReaderWrapper {
    /// Creates the wrapper for the given soft-deletes field.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }
}

impl DirectoryReaderWrapper for SoftDeletesDirectoryReaderWrapper {
    /// `SoftDeletesDirectoryReaderWrapper` overrides no `toString()`, so the
    /// inherited `CompositeReader` rendering applies under this name.
    fn subclass_simple_name(&self) -> &'static str {
        "SoftDeletesDirectoryReaderWrapper"
    }

    /// `SoftDeletesDirectoryReaderWrapper`'s constructor builds a
    /// [`DelegatingCacheHelper`] over `in.getReaderCacheHelper()`
    /// (`SoftDeletesDirectoryReaderWrapper.java:61-64`), and
    /// `getReaderCacheHelper()` returns it.
    fn uses_delegating_cache_helper(&self) -> bool {
        true
    }

    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        // See `ExitableDirectoryReaderWrapper::wrap` for why the leaves are
        // read back off `inner`'s own `LeafReaderContext`s.
        let leaves: Vec<Arc<dyn LeafReader>> = Arc::clone(&inner)
            .leaves()
            .into_iter()
            .map(|context| context.leaf_reader())
            .collect();
        let sub_wrapper: Arc<dyn SubReaderWrapper> =
            Arc::new(SoftDeletesSubReaderWrapper::new(self.field.clone()));
        let directory_wrapper: Arc<dyn DirectoryReaderWrapper> =
            Arc::new(Self::new(self.field.clone()));
        Ok(Arc::new(FilterDirectoryReader::new(
            inner,
            leaves,
            sub_wrapper,
            directory_wrapper,
        )?))
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the `Exitable*` wrappers Lucene declares private, which
    //! no integration test can name. The interruptible-iteration tests that
    //! reach the crate's public surface live in
    //! `tests/exitable_directory_reader_iteration.rs`.

    use super::*;
    use crate::index::point_values::InMemoryPointValues;
    use crate::index::vector_values::from_floats;
    use std::sync::atomic::AtomicUsize;

    /// A timeout that fires from its `fires_after`-th interrogation onwards, so
    /// that an iteration crosses it part way through instead of before it
    /// starts.
    #[derive(Debug)]
    struct TimeoutAfter {
        calls: AtomicUsize,
        fires_after: usize,
    }

    impl TimeoutAfter {
        fn new(fires_after: usize) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fires_after,
            }
        }
    }

    impl QueryTimeout for TimeoutAfter {
        fn should_exit(&self) -> bool {
            self.calls.fetch_add(1, Ordering::Relaxed) + 1 >= self.fires_after
        }
    }

    /// A timeout that has always already fired.
    #[derive(Debug)]
    struct AlwaysTimesOut;

    impl QueryTimeout for AlwaysTimesOut {
        fn should_exit(&self) -> bool {
            true
        }
    }

    /// How many interrogations a point traversal is allowed before its timeout
    /// fires. Chosen so that the traversal is interrupted part way through the
    /// 1024 points rather than at its first or its last check.
    const FIRES_AFTER: usize = 100;

    /// Counts what a traversal visits, and accepts every cell.
    struct CountingVisitor {
        visited: usize,
    }

    impl IntersectVisitor for CountingVisitor {
        fn visit(&mut self, _doc_id: i32) -> Result<()> {
            self.visited += 1;
            Ok(())
        }

        fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
            self.visited += 1;
            Ok(())
        }

        fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
            Ok(Relation::CellCrossesQuery)
        }
    }

    /// Builds point values with one leaf block per pair of points, so a traversal
    /// makes many moves.
    fn point_values() -> InMemoryPointValues {
        let leaves: Vec<Vec<(i32, Vec<u8>)>> = (0..512)
            .map(|block: i32| {
                (0..2)
                    .map(|i| {
                        let doc = block * 2 + i;
                        (doc, doc.to_be_bytes().to_vec())
                    })
                    .collect()
            })
            .collect();
        InMemoryPointValues::new(1, 1, 4, leaves).expect("in-memory point values")
    }

    #[test]
    fn exitable_point_values_check_the_timeout_on_construction() {
        let error = ExitablePointValues::new(Box::new(point_values()), Arc::new(AlwaysTimesOut))
            .err()
            .expect("Lucene's ExitablePointValues constructor ends with checkAndThrow()");
        assert!(
            error
                .to_string()
                .contains("The request took too long to iterate over point values"),
            "got: {error}"
        );
    }

    #[test]
    fn a_point_traversal_is_interrupted_mid_iteration() {
        // The construction check passes, and the traversal crosses the timeout.
        let values = ExitablePointValues::new(
            Box::new(point_values()),
            Arc::new(TimeoutAfter::new(FIRES_AFTER)),
        )
        .expect("the timeout has not fired yet");
        let mut visitor = CountingVisitor { visited: 0 };
        let error = values
            .intersect(&mut visitor)
            .err()
            .expect("the timeout fires during the traversal");
        println!(
            "point traversal stopped after {} visits with: {error}",
            visitor.visited
        );
        assert!(
            error
                .to_string()
                .contains("The request took too long to intersect point values"),
            "got: {error}"
        );
        assert!(
            (1..1024).contains(&visitor.visited),
            "the traversal must stop mid-iteration, having visited {} of 1024 points",
            visitor.visited
        );
    }

    #[test]
    fn a_vector_iteration_is_interrupted_mid_iteration() {
        let vectors: Vec<Vec<f32>> = (0..2_000).map(|d| vec![d as f32]).collect();
        let values = ExitableFloatVectorValues::new(
            Box::new(from_floats(vectors, 1)),
            Arc::new(TimeoutAfter::new(2)),
        );
        let mut iterator =
            KnnVectorValues::iterator(&values).expect("an iterator over the vectors");

        let mut seen = 0usize;
        let outcome = loop {
            match iterator.next_doc() {
                Ok(doc) if doc == crate::search::NO_MORE_DOCS => break Ok(()),
                Ok(_) => seen += 1,
                Err(error) => break Err(error),
            }
        };
        let error = outcome.expect_err("the timeout fires during the iteration");
        println!("vector iteration stopped after {seen} documents with: {error}");
        assert!(
            error
                .to_string()
                .contains("The request took too long to iterate over knn vector values"),
            "got: {error}"
        );
        assert!(
            (1..2_000).contains(&seen),
            "the iteration must stop mid-iteration, having read {seen} of 2000 vectors"
        );
    }
}
