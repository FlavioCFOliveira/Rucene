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
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::base_composite_reader::BaseCompositeReader;
use crate::index::codec_reader::{final_members, CodecReader};
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
use std::sync::atomic::{AtomicI32, Ordering};

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

    /// The `doWrapDirectoryReader(in)` of the subclass, called on `reader`, the
    /// reader being reopened (Java's `this`), with `inner`, the reopened
    /// delegate. The default is [`wrap`](Self::wrap), for a subclass whose
    /// override does not read `this`; `SoftDeletesDirectoryReaderWrapper`'s
    /// reads `this.getSequentialSubReaders()` to reuse the unchanged leaves
    /// (`SoftDeletesDirectoryReaderWrapper.java:68-83`), and so overrides this.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the wrapped reader raises.
    fn do_wrap_directory_reader(
        &self,
        reader: &FilterDirectoryReader,
        inner: Arc<dyn DirectoryReader>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        let _ = reader;
        self.wrap(inner)
    }

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

    /// Whether the `FilterDirectoryReader` subclass this wrapper stands for
    /// answers `getReaderCacheHelper()` with `null`, as
    /// `TestDemoParallelLeafReader.ParallelLeafDirectoryReader` does
    /// (`TestDemoParallelLeafReader.java:259-262`) and
    /// `TestLRUQueryCache.DummyDirectoryReader` (`TestLRUQueryCache.java:1555-1558`).
    /// It takes precedence over
    /// [`uses_delegating_cache_helper`](Self::uses_delegating_cache_helper).
    /// The default is `false`.
    fn reader_cache_helper_is_null(&self) -> bool {
        false
    }

    /// The `doClose()` of the `FilterDirectoryReader` subclass this wrapper
    /// stands for; the default is `FilterDirectoryReader.doClose()`,
    /// `in.close()` (`FilterDirectoryReader.java:161-164`). A subclass that
    /// overrides it, as `TestDemoParallelLeafReader.ParallelLeafDirectoryReader`
    /// does, overrides this.
    ///
    /// # Errors
    ///
    /// What closing raises.
    fn do_close(&self, reader: &FilterDirectoryReader) -> Result<()> {
        reader.get_delegate().close()
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
/// [`get_key`](CacheHelper::get_key) hands out the key this helper owns, so as
/// long as one helper instance answers for a reader — the readers that use it
/// build it once and share it through an [`Arc`], as Java builds it once in
/// their constructors — every caller sees the same key, and a different key
/// from the delegate's.
pub struct DelegatingCacheHelper {
    cache_key: CacheKey,
    delegate: Box<dyn CacheHelper>,
}

impl DelegatingCacheHelper {
    /// Creates a helper with a fresh key that delegates its closed listeners to
    /// `delegate`.
    ///
    /// Equivalent to the protected `DelegatingCacheHelper(CacheHelper)`.
    pub fn new(delegate: Box<dyn CacheHelper>) -> Self {
        Self {
            cache_key: CacheKey::new(),
            delegate,
        }
    }

    /// Returns this helper's key: Java's `cacheKey` field.
    pub fn cache_key(&self) -> CacheKey {
        self.cache_key.clone()
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
/// the delegate: the lambda `unused -> listener.onClose(cacheKey)`, which
/// calls the listener with the delegating helper's key, not the delegate's.
struct DelegatingClosedListener {
    cache_key: CacheKey,
    listener: Arc<dyn ClosedListener>,
}

impl ClosedListener for DelegatingClosedListener {
    fn on_close(&self, _unused: CacheKey) -> Result<()> {
        self.listener.on_close(self.cache_key.clone())
    }
}

impl CacheHelper for DelegatingCacheHelper {
    /// Equivalent to `DelegatingCacheHelper.getKey()`: this helper's own key,
    /// never the delegate's.
    fn get_key(&self) -> &CacheKey {
        &self.cache_key
    }

    /// Equivalent to `DelegatingCacheHelper.addClosedListener(ClosedListener)`:
    /// the listener is wrapped and registered on the delegate, so that it runs
    /// when the delegate's resource closes, and is called with this helper's
    /// key.
    fn add_closed_listener(&self, listener: Arc<dyn ClosedListener>) {
        self.delegate
            .add_closed_listener(Arc::new(DelegatingClosedListener {
                cache_key: self.cache_key.clone(),
                listener,
            }));
    }
}

impl CacheHelper for Arc<DelegatingCacheHelper> {
    fn get_key(&self) -> &CacheKey {
        self.as_ref().get_key()
    }

    fn add_closed_listener(&self, listener: Arc<dyn ClosedListener>) {
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
///
/// As in Java, the filter is an `IndexReader` of its own, with its own
/// [`IndexReaderCore`]: `BaseCompositeReader`'s constructor registers it as a
/// parent of each wrapped leaf (`BaseCompositeReader.java:86`), the members
/// `BaseCompositeReader` declares `final` call `ensureOpen()` on it, and its
/// `doClose()` closes the wrapped reader (`in.close()`,
/// `FilterDirectoryReader.java:161-164`).
pub struct FilterDirectoryReader {
    /// This reader's own state.
    core: IndexReaderCore,
    inner: Arc<dyn DirectoryReader>,
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
    /// Wraps `inner`, passing each of its leaves through `wrapper`, and
    /// remembering `directory_wrapper` for `doOpenIfChanged*` to reproduce the
    /// same wrapping on the reopened reader.
    ///
    /// Equivalent to `FilterDirectoryReader(DirectoryReader, SubReaderWrapper)`
    /// (`FilterDirectoryReader.java:91-94`), which wraps
    /// `in.getSequentialSubReaders()`. A `DirectoryReader`'s sequential
    /// sub-readers are its leaves, so they are read off `inner`'s own
    /// `LeafReaderContext`s, which carry the `Arc<dyn LeafReader>` handle that
    /// a downcast from `dyn IndexReader` cannot produce.
    ///
    /// # Errors
    ///
    /// Propagates whatever `wrapper` raises while wrapping the leaves.
    pub fn new(
        inner: Arc<dyn DirectoryReader>,
        wrapper: Arc<dyn SubReaderWrapper>,
        directory_wrapper: Arc<dyn DirectoryReaderWrapper>,
    ) -> Result<Self> {
        // `in.getSequentialSubReaders()` does not `ensureOpen()`, which
        // `getContext()` would, so the context is built directly.
        let leaves: Vec<Arc<dyn LeafReader>> = Arc::clone(&inner)
            .build_context(None, 0, 0, 0, 0)
            .leaves()
            .into_iter()
            .map(|context| context.leaf_reader())
            .collect();
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
        let core = IndexReaderCore::new();
        let base = BaseCompositeReader::new(sub_readers, true, &core)?;
        let owns_reader_cache_helper = directory_wrapper.uses_delegating_cache_helper();
        let reader_cache_helper = if owns_reader_cache_helper {
            delegating_cache_helper(inner.get_reader_cache_helper())
        } else {
            None
        };
        Ok(Self {
            core,
            inner,
            directory_wrapper,
            wrapped_leaves,
            base,
            reader_cache_helper,
            owns_reader_cache_helper,
        })
    }

    /// The wrapped leaves, as the `LeafReader`s they are: Java's
    /// `getSequentialSubReaders()`, whose elements a `FilterDirectoryReader`
    /// subclass tests with `instanceof` (`TestDemoParallelLeafReader.java:232-245`).
    pub fn wrapped_leaves(&self) -> &[Arc<dyn LeafReader>] {
        &self.wrapped_leaves
    }

    /// Returns the wrapped reader.
    ///
    /// Equivalent to `FilterDirectoryReader.getDelegate()`.
    pub fn get_delegate(&self) -> &Arc<dyn DirectoryReader> {
        &self.inner
    }

    /// Returns the reader this one wraps.
    ///
    /// Equivalent to reading `FilterDirectoryReader.in`, which is what
    /// `ExitableDirectoryReader.toString()` renders.
    pub fn delegate(&self) -> &Arc<dyn DirectoryReader> {
        &self.inner
    }

    /// Returns a new wrapped instance of `inner`, produced by the injected
    /// [`DirectoryReaderWrapper`] on this reader.
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
        self.directory_wrapper.do_wrap_directory_reader(self, inner)
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
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.ensure_open()?;
        self.base.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.base.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.base.max_doc()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.ensure_open()?;
        self.base.stored_fields()
    }

    /// Equivalent to `FilterDirectoryReader.doClose()`, the one close method
    /// the class overrides: `in.close()`, the delegate's own `close()`, whose
    /// `decRef()` also runs the delegate's `notifyReaderClosedListeners()` —
    /// which is how a listener registered through a
    /// [`DelegatingCacheHelper`] over the delegate's helper learns that the
    /// wrapped reader closed.
    fn do_close(&self) -> Result<()> {
        self.directory_wrapper.do_close(self)
    }

    /// Equivalent to the subclass's `getReaderCacheHelper()`: the wrapped
    /// reader's helper, or the [`DelegatingCacheHelper`] the subclass built over
    /// it; see [`DirectoryReaderWrapper::uses_delegating_cache_helper`].
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        if self.directory_wrapper.reader_cache_helper_is_null() {
            return None;
        }
        if self.owns_reader_cache_helper {
            self.reader_cache_helper
                .as_ref()
                .map(|helper| Box::new(Arc::clone(helper)) as Box<dyn CacheHelper>)
        } else {
            self.inner.get_reader_cache_helper()
        }
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.ensure_open()?;
        self.base.doc_freq(term)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.ensure_open()?;
        self.base.total_term_freq(term)
    }

    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
        self.base.get_sum_doc_freq(field)
    }

    fn get_doc_count(&self, field: &str) -> Result<i32> {
        self.ensure_open()?;
        self.base.get_doc_count(field)
    }

    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
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

    fn do_open_if_changed_with_executor(
        &self,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.wrap_directory_reader(self.inner.do_open_if_changed_with_executor(executor)?)
    }

    fn do_open_if_changed_from_commit_with_executor(
        &self,
        commit: &dyn IndexCommit,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.wrap_directory_reader(
            self.inner
                .do_open_if_changed_from_commit_with_executor(commit, executor)?,
        )
    }

    fn do_open_if_changed_from_writer_with_executor(
        &self,
        writer: &dyn crate::index::directory_reader::IndexWriter,
        apply_all_deletes: bool,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.wrap_directory_reader(self.inner.do_open_if_changed_from_writer_with_executor(
            writer,
            apply_all_deletes,
            executor,
        )?)
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
    LuceneError::from(LuceneErrorKind::ExitingReader(format!(
        "The request took too long to {action}"
    )))
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
        let sub_wrapper: Arc<dyn SubReaderWrapper> = Arc::new(ExitableSubReaderWrapper::new(
            Arc::clone(&self.query_timeout),
        ));
        let directory_wrapper: Arc<dyn DirectoryReaderWrapper> =
            Arc::new(Self::new(Arc::clone(&self.query_timeout)));
        Ok(Arc::new(FilterDirectoryReader::new(
            inner,
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
        )?))
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
/// The single exception is [`Bits::get`], which returns a plain `bool`: the
/// check Java makes there is made by [`Bits::try_get`], which the
/// nearest-vector searches call; see the divergence recorded on
/// [`ExitableAcceptDocs`].
#[derive(Debug)]
struct ExitableCheck {
    query_timeout: Arc<dyn QueryTimeout>,
    /// Equivalent to the `calls` field of the sampling inner classes. Java's
    /// `int` wraps on overflow, matched here by the atomic's wrapping add.
    calls: AtomicI32,
    /// The action the message names, e.g. `"iterate over terms"`.
    action: &'static str,
}

impl ExitableCheck {
    fn new(query_timeout: Arc<dyn QueryTimeout>, action: &'static str) -> Self {
        Self {
            query_timeout,
            calls: AtomicI32::new(0),
            action,
        }
    }

    /// The timeout this wrapper checks, so that a wrapper can hand it to the
    /// objects it in turn wraps.
    fn query_timeout(&self) -> &Arc<dyn QueryTimeout> {
        &self.query_timeout
    }

    /// The timeout test itself.
    fn fired(&self) -> bool {
        self.query_timeout.should_exit()
    }

    /// Equivalent to the `checkAndThrow()` of the enclosing inner class.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error when the timeout has fired.
    fn check_and_throw(&self) -> Result<()> {
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
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::AlreadyClosed`] when `reader` is closed, from the
    /// `registerParentReader` of the `FilterLeafReader` constructor.
    pub fn new(reader: Arc<dyn LeafReader>, query_timeout: Arc<dyn QueryTimeout>) -> Result<Self> {
        Ok(Self {
            inner: FilterLeafReader::new(reader)?,
            query_timeout,
        })
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

    /// `ExitableFilterAtomicReader extends FilterLeafReader`: the delegate its
    /// inherited `getDelegate()` returns.
    fn filter_leaf_reader_delegate(&self) -> Option<&Arc<dyn LeafReader>> {
        Some(self.inner.get_delegate())
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
        self.inner.search_nearest_vectors(
            field,
            target,
            collector,
            &mut timeout_checking_accept_docs,
        )
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
        self.inner.search_nearest_vectors_byte(
            field,
            target,
            collector,
            &mut timeout_checking_accept_docs,
        )
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

    fn size(&self) -> Result<i64> {
        self.inner.size()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        self.inner.sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        self.inner.sum_doc_freq()
    }

    fn doc_count(&self) -> Result<i32> {
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

    fn stats(&self) -> Result<String> {
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

    fn term(&self) -> Result<Option<BytesRef>> {
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
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
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
/// **Divergence from Lucene 10.5.0.** Java throws `ExitingReaderException`,
/// an unchecked exception, straight out of `Bits.get(int)` on a sampled call
/// (`ExitableDirectoryReader.java:371-375, 439-445`). [`Bits::get`] returns a
/// plain `bool`, so the wrapped bits make that check in [`Bits::try_get`]
/// instead, which the nearest-vector searches call and every `Bits` that
/// delegates `get` forwards; the error leaves the search at the call where
/// Java's exception does.
struct ExitableAcceptDocs<'a> {
    inner: &'a mut dyn AcceptDocs,
    max_doc: i32,
    /// The timeout-checking bits, wrapping the delegate's bits or, when it has
    /// none, match-all bits. `None` until `bits()` is first called.
    bits: Option<Arc<dyn Bits>>,
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
            check: Arc::new(ExitableCheck::new(query_timeout, "search nearest vectors")),
        }
    }
}

impl AcceptDocs for ExitableAcceptDocs<'_> {
    /// Equivalent to `ExitableAcceptDocs.bits()`.
    ///
    /// # Errors
    ///
    /// Propagates the delegate's errors.
    fn bits(&mut self) -> Result<Option<Arc<dyn Bits>>> {
        if self.bits.is_none() {
            // When acceptDocs is null due to no doc deleted, we will
            // instantiate a new one that would match all docs to allow timeout
            // checking.
            let updated_accept_docs: Arc<dyn Bits> = match self.inner.bits()? {
                None => Arc::new(MatchAllBits::new(self.max_doc as usize)),
                Some(bits) => bits,
            };
            self.bits = Some(Arc::new(ExitableBits {
                inner: updated_accept_docs,
                check: Arc::clone(&self.check),
            }));
        }
        Ok(self.bits.as_ref().map(Arc::clone))
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
    inner: Arc<dyn Bits>,
    check: Arc<ExitableCheck>,
}

impl std::fmt::Debug for ExitableBits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExitableBits").finish_non_exhaustive()
    }
}

impl Bits for ExitableBits {
    /// Answers the wrapped bits. The sampled check of Java's `get` is made by
    /// [`try_get`](Self::try_get), which is what the nearest-vector searches
    /// call; see the divergence recorded on [`ExitableAcceptDocs`].
    fn get(&self, index: usize) -> bool {
        self.inner.get(index)
    }

    /// Equivalent to the anonymous `Bits.get(int)` of
    /// `ExitableAcceptDocs.bits()` (`ExitableDirectoryReader.java:371-377`):
    /// one call in [`MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK`] checks the timeout
    /// and raises the exiting-reader error, before the wrapped bits answer.
    ///
    /// # Errors
    ///
    /// Raises the exiting-reader error on a sampled call once the timeout has
    /// fired.
    fn try_get(&self, index: usize) -> Result<bool> {
        self.check
            .check_and_throw_with_sampling(MAX_CALLS_BEFORE_QUERY_TIMEOUT_CHECK)?;
        Ok(self.inner.get(index))
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
    /// Always returns [`LuceneErrorKind::UnsupportedOperation`].
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::unsupported_operation(
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
    /// Always returns [`LuceneErrorKind::UnsupportedOperation`].
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Err(LuceneError::unsupported_operation(
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
    /// Always returns [`LuceneErrorKind::UnsupportedOperation`].
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::unsupported_operation(
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
    /// Always returns [`LuceneErrorKind::UnsupportedOperation`].
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Err(LuceneError::unsupported_operation(
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
    /// Equivalent to `SlowCodecReaderWrapper.wrap(LeafReader)`
    /// (`SlowCodecReaderWrapper.java:51-60`): the reader is checked for
    /// integrity once and then served slowly over its own leaf-reader API.
    ///
    /// **Divergence from Lucene 10.5.0.** Java first returns a reader that
    /// already is a `CodecReader` unchanged (`reader instanceof CodecReader`).
    /// A `dyn LeafReader` cannot be asked that, so this always builds the slow
    /// view; a caller that holds a [`CodecReader`] uses it as it is, which is
    /// what that branch returns.
    ///
    /// # Errors
    ///
    /// Propagates whatever the integrity check raises.
    pub fn wrap(reader: Arc<dyn LeafReader>) -> Result<Arc<dyn CodecReader>> {
        // Simulate it slowly, over the leaf-reader API.
        reader.check_integrity()?;
        Ok(Arc::new(SlowCodecReader::new(reader)))
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
///
/// As in Java, the view is an `IndexReader` of its own, with its own
/// [`IndexReaderCore`]: it does not register itself as a parent of `reader`,
/// and its `doClose()` is `CodecReader`'s, which does nothing. The members
/// `CodecReader` declares `final` answer through the synthesised producers
/// ([`final_members`]), after `ensureOpen()` on this view; each producer
/// accessor but the points one calls `reader.ensureOpen()` first
/// (`SlowCodecReaderWrapper.java:58-99`).
#[derive(Debug)]
pub struct SlowCodecReader {
    /// This reader's own state.
    core: IndexReaderCore,
    reader: Arc<dyn LeafReader>,
}

impl SlowCodecReader {
    /// Wraps `reader`, which the caller has already integrity-checked.
    fn new(reader: Arc<dyn LeafReader>) -> Self {
        Self {
            core: IndexReaderCore::new(),
            reader,
        }
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
    /// This reader is a `CodecReader`: Java's `instanceof CodecReader` on it
    /// is `true`.
    fn as_codec_reader(
        self: Arc<Self>,
    ) -> Option<Arc<dyn crate::index::codec_reader::CodecReader>> {
        Some(self)
    }

    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    /// `CodecReader.termVectors()`, `final`.
    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        final_members::term_vectors(self)
    }

    fn num_docs(&self) -> i32 {
        self.reader.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.reader.max_doc()
    }

    /// `CodecReader.storedFields()`, `final`.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        final_members::stored_fields(self)
    }

    /// `CodecReader.doClose()`, which the anonymous class does not
    /// override: it does nothing (`CodecReader.java:291-292`).
    fn do_close(&self) -> Result<()> {
        Ok(())
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.reader.get_reader_cache_helper()
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.reader.get_core_cache_helper()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        final_members::terms(self, field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::numeric_doc_values(self, field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        final_members::binary_doc_values(self, field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        final_members::sorted_doc_values(self, field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        final_members::sorted_numeric_doc_values(self, field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        final_members::sorted_set_doc_values(self, field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::norm_values(self, field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        final_members::doc_values_skipper(self, field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        final_members::float_vector_values(self, field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        final_members::byte_vector_values(self, field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors(self, field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors_byte(self, field, target, collector, accept_docs)
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.reader.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.reader.get_live_docs()
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        final_members::point_values(self, field)
    }

    /// `CodecReader.checkIntegrity()`, which the anonymous class does not
    /// override: every synthesised producer, whose own check does nothing
    /// because the whole reader was checked up front.
    fn check_integrity(&self) -> Result<()> {
        final_members::check_integrity(self)
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.reader.get_meta_data()
    }
}

impl CodecReader for SlowCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.reader.core().ensure_open()?;
        Ok(Some(Box::new(
            SlowCodecReaderWrapper::reader_to_stored_fields_reader(Arc::clone(&self.reader)),
        )))
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.reader.core().ensure_open()?;
        Ok(Some(Box::new(
            SlowCodecReaderWrapper::reader_to_term_vectors_reader(Arc::clone(&self.reader)),
        )))
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.reader.core().ensure_open()?;
        Ok(Some(Arc::new(
            SlowCodecReaderWrapper::reader_to_norms_producer(Arc::clone(&self.reader)),
        )))
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.reader.core().ensure_open()?;
        Ok(Some(Arc::new(
            SlowCodecReaderWrapper::reader_to_doc_values_producer(Arc::clone(&self.reader)),
        )))
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.reader.core().ensure_open()?;
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
        self.reader.core().ensure_open()?;
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

    fn close(&self) -> Result<()> {
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
        // the way. `segmentReader(LeafReader)` (`SlowCodecReaderWrapper.java:
        // 207-216`) steps through `FilterLeafReader.unwrap` and tests for a
        // `SegmentReader` (`as_segment_reader`). Its `FilterCodecReader` branch
        // has no counterpart: a `dyn LeafReader` cannot be asked whether it is a
        // `FilterCodecReader`, so such a reader reaches the fallback below,
        // which answers an empty map where Java throws `AssertionError`.
        let reader = crate::index::filter_reader::FilterLeafReader::unwrap(&self.reader);
        match reader.as_segment_reader() {
            Some(segment_reader) => match segment_reader.get_vector_reader() {
                Ok(Some(vectors_reader)) => vectors_reader.get_off_heap_byte_size(field_info),
                _ => HashMap::new(),
            },
            None => HashMap::new(),
        }
    }

    fn close(&self) -> Result<()> {
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
    /// `reader.getNormValues(field.name)`, `null` included
    /// (`index/SlowCodecReaderWrapper.java:225-228`).
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.reader.get_norm_values(&field.name)
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

    fn close(&self) -> Result<()> {
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
    /// `reader.get_numeric_doc_values(field.name)`, `null` included
    /// (`index/SlowCodecReaderWrapper.java:245-248`).
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.reader.get_numeric_doc_values(&field.name)
    }

    /// `reader.get_binary_doc_values(field.name)`, `null` included
    /// (`index/SlowCodecReaderWrapper.java:250-253`).
    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.reader.get_binary_doc_values(&field.name)
    }

    /// `reader.get_sorted_doc_values(field.name)`, `null` included
    /// (`index/SlowCodecReaderWrapper.java:255-258`).
    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.reader.get_sorted_doc_values(&field.name)
    }

    /// `reader.get_sorted_numeric_doc_values(field.name)`, `null` included
    /// (`index/SlowCodecReaderWrapper.java:260-263`).
    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.reader.get_sorted_numeric_doc_values(&field.name)
    }

    /// `reader.get_sorted_set_doc_values(field.name)`, `null` included
    /// (`index/SlowCodecReaderWrapper.java:265-268`).
    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.reader.get_sorted_set_doc_values(&field.name)
    }

    /// Equivalent to the anonymous producer's `getSkipper`
    /// (`index/SlowCodecReaderWrapper.java:269-272`): the wrapped reader's
    /// `getDocValuesSkipper(field.name)`, `None` included.
    fn get_skipper(&self, field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.reader.get_doc_values_skipper(&field.name)
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

    fn close(&self) -> Result<()> {
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

    fn clone_reader(&self) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(Self {
            reader: Arc::clone(&self.reader),
        }))
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

    fn clone_reader(&self) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(Self {
            reader: Arc::clone(&self.reader),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl TermVectors for SlowTermVectorsReader {
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        self.reader.term_vectors()?.get(doc)
    }

    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        self.reader.term_vectors()?.prefetch(doc_id)
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

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// Hides the documents a soft-deletes field marks as deleted.
///
/// Equivalent to
/// `org.apache.lucene.index.SoftDeletesDirectoryReaderWrapper.SoftDeletesSubReaderWrapper`
/// (`SoftDeletesDirectoryReaderWrapper.java:90-130`).
pub struct SoftDeletesSubReaderWrapper {
    /// Java's `mapping`: the wrapped leaves of the reader being reopened, keyed
    /// on the reader cache key of the leaf each one wraps.
    mapping: HashMap<CacheKey, Arc<dyn LeafReader>>,
    field: String,
}

impl SoftDeletesSubReaderWrapper {
    /// Creates the wrapper for the given soft-deletes field, with no leaves to
    /// reuse: `new SoftDeletesSubReaderWrapper(Collections.emptyMap(), field)`,
    /// what the public `SoftDeletesDirectoryReaderWrapper` constructor builds.
    pub fn new(field: impl Into<String>) -> Self {
        Self::with_old_readers_cache(HashMap::new(), field)
    }

    /// `SoftDeletesSubReaderWrapper(Map<CacheKey, LeafReader> oldReadersCache,
    /// String field)`: a leaf whose reader cache key is in `old_readers_cache`
    /// is answered with the mapped, already wrapped leaf.
    pub fn with_old_readers_cache(
        old_readers_cache: HashMap<CacheKey, Arc<dyn LeafReader>>,
        field: impl Into<String>,
    ) -> Self {
        Self {
            mapping: old_readers_cache,
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
    /// Equivalent to `SoftDeletesDirectoryReaderWrapper.wrap(LeafReader, String)`:
    /// `reader instanceof CodecReader ? new SoftDeletesFilterCodecReader(...)
    /// : new SoftDeletesFilterLeafReader(...)`
    /// (`SoftDeletesDirectoryReaderWrapper.java:150-152`), the `instanceof`
    /// being [`LeafReader::as_codec_reader`].
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
        if let Some(codec_reader) = Arc::clone(&reader).as_codec_reader() {
            return Ok(Arc::new(SoftDeletesFilterCodecReader::new(
                codec_reader,
                bits,
                num_docs,
            )));
        }
        Ok(Arc::new(SoftDeletesFilterLeafReader::new(
            reader, bits, num_docs,
        )?))
    }
}

impl SubReaderWrapper for SoftDeletesSubReaderWrapper {
    /// Equivalent to `SoftDeletesSubReaderWrapper.wrap(LeafReader)`
    /// (`SoftDeletesDirectoryReaderWrapper.java:115-129`).
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        if let Some(reader_cache_helper) = reader.get_reader_cache_helper() {
            if let Some(cached) = self.mapping.get(reader_cache_helper.get_key()) {
                // if the reader cache helper didn't change and we have it in the
                // cache don't bother creating a new one
                return Ok(Arc::clone(cached));
            }
        }
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
/// the wrapped reader's, because the segment's data is unchanged. A wrapped
/// `CodecReader` gets a [`SoftDeletesFilterCodecReader`] instead
/// ([`SoftDeletesSubReaderWrapper::wrap_leaf_reader`]).
///
/// Java package-private; public so that `TestSoftDeletesDirectoryReaderWrapper`
/// can test a leaf for this class (`instanceof`, through
/// [`LeafReader::leaf_as_any`]).
pub struct SoftDeletesFilterLeafReader {
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
    /// `super(reader)` first (`SoftDeletesDirectoryReaderWrapper.java:196-202`),
    /// which registers this reader as a parent of `reader`.
    fn new(
        reader: Arc<dyn LeafReader>,
        bits: crate::util::FixedBitSet,
        num_docs: i32,
    ) -> Result<Self> {
        let inner = FilterLeafReader::new(reader)?;
        let reader_cache_helper =
            delegating_cache_helper(inner.get_delegate().get_reader_cache_helper());
        Ok(Self {
            inner,
            bits: Arc::new(bits),
            num_docs,
            reader_cache_helper,
        })
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
    /// The reader is a `SoftDeletesFilterLeafReader`, which Java tests with
    /// `instanceof` (`TestSoftDeletesDirectoryReaderWrapper.isWrapped`).
    fn leaf_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn core(&self) -> &IndexReaderCore {
        LeafReader::core(&self.inner)
    }

    /// `SoftDeletesFilterLeafReader extends FilterLeafReader`: the delegate its
    /// inherited `getDelegate()` returns.
    fn filter_leaf_reader_delegate(&self) -> Option<&Arc<dyn LeafReader>> {
        Some(self.inner.get_delegate())
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

/// Hides the documents a soft-deletes field marks as deleted from a codec
/// reader, reporting `bits`/`num_docs` in place of the wrapped reader's own,
/// and remaining a `CodecReader` itself.
///
/// Equivalent to
/// `SoftDeletesDirectoryReaderWrapper.SoftDeletesFilterCodecReader`
/// (`SoftDeletesDirectoryReaderWrapper.java:227-263`), which `extends
/// FilterCodecReader`: as there, it is a reader of its own (it does not
/// register itself as a parent of the wrapped reader, and its `doClose()` is
/// the wrapped reader's `doClose()`, `FilterCodecReader.java:58-60, 123-126`);
/// the members `CodecReader` declares `final` answer through this reader's
/// producer accessors, which forward to the wrapped reader's. Its reader cache
/// helper is a [`DelegatingCacheHelper`] over the wrapped reader's, built once
/// in the constructor; its core cache helper is the wrapped reader's.
///
/// Java package-private; public so that `TestSoftDeletesDirectoryReaderWrapper`
/// can test a leaf for this class (`instanceof`, through
/// [`LeafReader::leaf_as_any`]).
pub struct SoftDeletesFilterCodecReader {
    /// This reader's own state.
    core: IndexReaderCore,
    /// The wrapped reader: Java's `FilterCodecReader.in` and the subclass's
    /// own `reader` field, the same object.
    inner: Arc<dyn CodecReader>,
    bits: Arc<crate::util::FixedBitSet>,
    num_docs: i32,
    reader_cache_helper: Option<Arc<DelegatingCacheHelper>>,
}

impl SoftDeletesFilterCodecReader {
    /// `SoftDeletesFilterCodecReader(CodecReader, FixedBitSet, int)`
    /// (`SoftDeletesDirectoryReaderWrapper.java:233-243`).
    fn new(reader: Arc<dyn CodecReader>, bits: crate::util::FixedBitSet, num_docs: i32) -> Self {
        let reader_cache_helper = delegating_cache_helper(reader.get_reader_cache_helper());
        Self {
            core: IndexReaderCore::new(),
            inner: reader,
            bits: Arc::new(bits),
            num_docs,
            reader_cache_helper,
        }
    }
}

impl std::fmt::Debug for SoftDeletesFilterCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoftDeletesFilterCodecReader")
            .field("num_docs", &self.num_docs)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for SoftDeletesFilterCodecReader {
    /// Neither the class nor `FilterCodecReader` declares a `toString()`, so
    /// `Object`'s applies.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SoftDeletesDirectoryReaderWrapper$SoftDeletesFilterCodecReader",
            self,
        ))
    }
}

impl LeafReader for SoftDeletesFilterCodecReader {
    /// The reader is a `SoftDeletesFilterCodecReader`, which Java tests with
    /// `instanceof` (`TestSoftDeletesDirectoryReaderWrapper.isWrapped`).
    fn leaf_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// This reader is a `CodecReader`: Java's `instanceof CodecReader` on it
    /// is `true`.
    fn as_codec_reader(self: Arc<Self>) -> Option<Arc<dyn CodecReader>> {
        Some(self)
    }

    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    /// `CodecReader.termVectors()`, `final`.
    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        final_members::term_vectors(self)
    }

    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    fn max_doc(&self) -> i32 {
        self.inner.max_doc()
    }

    /// `CodecReader.storedFields()`, `final`.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        final_members::stored_fields(self)
    }

    fn do_close(&self) -> Result<()> {
        self.inner.do_close()
    }

    /// `SoftDeletesFilterCodecReader.getReaderCacheHelper()`: the
    /// [`DelegatingCacheHelper`] built in the constructor.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.reader_cache_helper
            .as_ref()
            .map(|helper| Box::new(Arc::clone(helper)) as Box<dyn CacheHelper>)
    }

    /// `SoftDeletesFilterCodecReader.getCoreCacheHelper()`:
    /// `reader.getCoreCacheHelper()`.
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_core_cache_helper()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        final_members::terms(self, field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::numeric_doc_values(self, field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        final_members::binary_doc_values(self, field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        final_members::sorted_doc_values(self, field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        final_members::sorted_numeric_doc_values(self, field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        final_members::sorted_set_doc_values(self, field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        final_members::norm_values(self, field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        final_members::doc_values_skipper(self, field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        final_members::float_vector_values(self, field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        final_members::byte_vector_values(self, field)
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.inner.get_field_infos()
    }

    /// `SoftDeletesFilterCodecReader.getLiveDocs()`: `bits`.
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        Some(Box::new(SoftDeletesBits(Arc::clone(&self.bits))))
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        final_members::point_values(self, field)
    }

    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.inner.get_meta_data()
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors(self, field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        final_members::search_nearest_vectors_byte(self, field, target, collector, accept_docs)
    }
}

impl CodecReader for SoftDeletesFilterCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.inner.get_fields_reader()
    }

    /// A `FilterCodecReader`: its `getDelegate()`.
    fn filter_codec_reader_delegate(&self) -> Option<&Arc<dyn CodecReader>> {
        Some(&self.inner)
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.inner.get_term_vectors_reader()
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.inner.get_norms_reader()
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.inner.get_doc_values_reader()
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.inner.get_postings_reader()
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.inner.get_points_reader()
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        self.inner.get_vector_reader()
    }
}

/// Reproduces the same soft-deletes wrapping on a reopened [`DirectoryReader`].
///
/// Equivalent to `org.apache.lucene.index.SoftDeletesDirectoryReaderWrapper`,
/// the `FilterDirectoryReader` subclass this wrapper stands for: [`wrap`]
/// (`DirectoryReaderWrapper::wrap`) is its public constructor, and
/// [`do_wrap_directory_reader`](DirectoryReaderWrapper::do_wrap_directory_reader)
/// its `doWrapDirectoryReader(DirectoryReader)`, which reuses the wrapped
/// leaves whose reader cache key did not change.
///
/// [`wrap`]: DirectoryReaderWrapper::wrap
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

    /// `new SoftDeletesDirectoryReaderWrapper(in, field)`: no leaves to reuse.
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        self.wrap_with(inner, SoftDeletesSubReaderWrapper::new(self.field.clone()))
    }

    /// Equivalent to
    /// `SoftDeletesDirectoryReaderWrapper.doWrapDirectoryReader(DirectoryReader)`
    /// (`SoftDeletesDirectoryReaderWrapper.java:68-83`): the wrapped leaves of
    /// `reader` are mapped by the reader cache key of the leaf each one wraps,
    /// so that a reopened leaf whose key did not change is answered with the
    /// previous wrap and its live-docs instance.
    fn do_wrap_directory_reader(
        &self,
        reader: &FilterDirectoryReader,
        inner: Arc<dyn DirectoryReader>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        let mut reader_cache: HashMap<CacheKey, Arc<dyn LeafReader>> = HashMap::new();
        for leaf in reader.wrapped_leaves() {
            // we try to reuse the life docs instances here if the reader cache key didn't change
            // `((SoftDeletesFilter*Reader) reader).reader.getReaderCacheHelper().getKey()`.
            let wrapped_key = |helper: Option<Box<dyn CacheHelper>>| -> CacheKey {
                helper
                    .unwrap_or_else(|| {
                        panic!(
                            "NullPointerException: getReaderCacheHelper() of the wrapped reader \
                             is null while the wrapper's is not"
                        )
                    })
                    .get_key()
                    .clone()
            };
            let as_any = leaf.leaf_as_any();
            if let Some(filter) =
                as_any.and_then(|any| any.downcast_ref::<SoftDeletesFilterLeafReader>())
            {
                if leaf.get_reader_cache_helper().is_some() {
                    reader_cache.insert(
                        wrapped_key(filter.inner.get_delegate().get_reader_cache_helper()),
                        Arc::clone(leaf),
                    );
                }
            } else if let Some(filter) =
                as_any.and_then(|any| any.downcast_ref::<SoftDeletesFilterCodecReader>())
            {
                if leaf.get_reader_cache_helper().is_some() {
                    reader_cache.insert(
                        wrapped_key(filter.inner.get_reader_cache_helper()),
                        Arc::clone(leaf),
                    );
                }
            }
        }
        self.wrap_with(
            inner,
            SoftDeletesSubReaderWrapper::with_old_readers_cache(reader_cache, self.field.clone()),
        )
    }
}

impl SoftDeletesDirectoryReaderWrapper {
    /// The private `SoftDeletesDirectoryReaderWrapper(DirectoryReader,
    /// SoftDeletesSubReaderWrapper)` constructor.
    fn wrap_with(
        &self,
        inner: Arc<dyn DirectoryReader>,
        sub_wrapper: SoftDeletesSubReaderWrapper,
    ) -> Result<Arc<dyn DirectoryReader>> {
        let directory_wrapper: Arc<dyn DirectoryReaderWrapper> =
            Arc::new(Self::new(self.field.clone()));
        Ok(Arc::new(FilterDirectoryReader::new(
            inner,
            Arc::new(sub_wrapper),
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
