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
use crate::index::codec_reader::CodecReader;
use crate::index::directory_reader::DirectoryReader;
use crate::index::doc_values::{
    BinaryDocValues, DocValues, DocValuesSkipper, NumericDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};
use crate::index::field_infos::{FieldInfo, FieldInfos};
use crate::index::filter_reader::FilterLeafReader;
use crate::index::index_reader::{
    CacheHelper, CompositeReader, IndexReader, IndexReaderCore, StoredFields,
};
use crate::index::index_utilities::QueryTimeout;
use crate::index::leaf_reader::{LeafReader, TermVectors};
use crate::index::point_values::PointValues;
use crate::index::stored_field_visitor::StoredFieldVisitor;
use crate::index::terms::{Fields, Terms};
use crate::index::vector_values::{ByteVectorValues, FloatVectorValues};
use crate::index::{IndexCommit, Term};
use crate::index::{IndexOptions, LeafMetaData};
use crate::search::knn::KnnCollector;
use crate::search::AcceptDocs;
use crate::store::Directory;
use crate::util::Bits;
use std::collections::HashMap;

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
}

impl std::fmt::Debug for FilterDirectoryReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterDirectoryReader")
            .field("leaves", &self.wrapped_leaves.len())
            .finish_non_exhaustive()
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
        Ok(Self {
            inner,
            wrapper,
            directory_wrapper,
            wrapped_leaves,
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
    fn core(&self) -> &IndexReaderCore {
        self.inner.core()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.inner.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.inner.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.inner.max_doc()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.inner.stored_fields()
    }

    fn do_close(&self) -> Result<()> {
        self.inner.do_close()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_reader_cache_helper()
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.inner.doc_freq(term)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.inner.total_term_freq(term)
    }

    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        self.inner.get_sum_doc_freq(field)
    }

    fn get_doc_count(&self, field: &str) -> Result<i32> {
        self.inner.get_doc_count(field)
    }

    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        self.inner.get_sum_total_term_freq(field)
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
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.inner.get_sequential_sub_readers()
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
    LuceneError::Other(format!("The request took too long to iterate over {what}"))
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

/// A leaf reader that refuses to keep working once its timeout has fired.
///
/// Equivalent to `ExitableDirectoryReader.ExitableFilterAtomicReader`.
///
/// **Divergence from Lucene 10.5.0.** Java also wraps the `Terms`, `TermsEnum`,
/// `PointValues` and vector-values objects the reader hands out, so the timeout
/// is checked on every `next()` deep inside an iteration. Those wrappers are
/// pure delegation around iterator types this port has not yet given filter
/// forms, so this reader checks the timeout at each entry point it owns; the
/// per-iteration checks are the remaining half.
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

impl ExitableFilterLeafReader {
    /// Wraps `reader`, checking `query_timeout` on each call.
    pub fn new(reader: Arc<dyn LeafReader>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            inner: FilterLeafReader::new(reader),
            query_timeout,
        }
    }

    /// Fails when the timeout has fired.
    fn check_timeout(&self, what: &str) -> Result<()> {
        if self.query_timeout.should_exit() {
            return Err(exiting_reader_error(what));
        }
        Ok(())
    }
}

impl LeafReader for ExitableFilterLeafReader {
    fn core(&self) -> &IndexReaderCore {
        LeafReader::core(&self.inner)
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.check_timeout("term vectors")?;
        LeafReader::term_vectors(&self.inner)
    }

    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }

    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.check_timeout("stored fields")?;
        LeafReader::stored_fields(&self.inner)
    }

    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        LeafReader::get_reader_cache_helper(&self.inner)
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_core_cache_helper()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn crate::index::Terms>>> {
        self.check_timeout("terms")?;
        self.inner.terms(field)
    }

    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::NumericDocValues>>> {
        self.check_timeout("numeric doc values")?;
        self.inner.get_numeric_doc_values(field)
    }

    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::BinaryDocValues>>> {
        self.check_timeout("binary doc values")?;
        self.inner.get_binary_doc_values(field)
    }

    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::SortedDocValues>>> {
        self.check_timeout("sorted doc values")?;
        self.inner.get_sorted_doc_values(field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::SortedNumericDocValues>>> {
        self.check_timeout("sorted numeric doc values")?;
        self.inner.get_sorted_numeric_doc_values(field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::SortedSetDocValues>>> {
        self.check_timeout("sorted set doc values")?;
        self.inner.get_sorted_set_doc_values(field)
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
        self.check_timeout("float vector values")?;
        self.inner.get_float_vector_values(field)
    }

    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::ByteVectorValues>>> {
        self.check_timeout("byte vector values")?;
        self.inner.get_byte_vector_values(field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn crate::search::knn::KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        self.check_timeout("nearest vectors")?;
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn crate::search::knn::KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        self.check_timeout("nearest vectors")?;
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }

    fn get_field_infos(&self) -> crate::index::FieldInfos {
        self.inner.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.inner.get_live_docs()
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn crate::index::PointValues>>> {
        self.check_timeout("point values")?;
        self.inner.get_point_values(field)
    }

    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }

    fn get_meta_data(&self) -> crate::index::leaf_reader::LeafMetaData {
        self.inner.get_meta_data()
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SlowCodecReaderWrapper({:?})", self.reader)
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

    fn get_values(&self, field: &str) -> Result<Box<dyn PointValues>> {
        self.reader.get_point_values(field)?.ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field \"{field}\" has no point values"))
        })
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

    fn get_float_vector_values(&self, field: &str) -> Result<Box<dyn FloatVectorValues>> {
        self.reader.get_float_vector_values(field)?.ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field \"{field}\" has no float vectors"))
        })
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Box<dyn ByteVectorValues>> {
        self.reader.get_byte_vector_values(field)?.ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field \"{field}\" has no byte vectors"))
        })
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
/// `SoftDeletesDirectoryReaderWrapper.SoftDeletesFilterLeafReader`.
///
/// **Divergence from Lucene 10.5.0.** Java also keeps the wrapped reader's
/// cache identity alive through a `DelegatingCacheHelper`, which is not
/// ported; this reader forwards `get_reader_cache_helper` to the wrapped
/// reader directly instead of minting a new cache key for the wrap. See
/// [`SoftDeletesSubReaderWrapper::wrap_leaf_reader`] for the codec-reader
/// divergence.
struct SoftDeletesFilterLeafReader {
    inner: FilterLeafReader,
    bits: Arc<crate::util::FixedBitSet>,
    num_docs: i32,
}

impl SoftDeletesFilterLeafReader {
    fn new(reader: Arc<dyn LeafReader>, bits: crate::util::FixedBitSet, num_docs: i32) -> Self {
        Self {
            inner: FilterLeafReader::new(reader),
            bits: Arc::new(bits),
            num_docs,
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

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        LeafReader::get_reader_cache_helper(&self.inner)
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
/// `getSequentialSubReaders()` to reuse an unchanged leaf's live-docs
/// instance, keyed by its `CacheHelper`, instead of recomputing it. That
/// reuse is not ported — `SoftDeletesFilterLeafReader::get_reader_cache_helper`
/// forwards to the wrapped reader rather than minting its own key, so there is
/// no independent key to reuse by — so every leaf is recomputed on each
/// reopen. The wrapped result is identical either way, only slower to build.
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
