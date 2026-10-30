//! Filter readers ported from `org.apache.lucene.index`.
//!
//! Covers `FilterLeafReader`, `FilterCodecReader` and `FilterDirectoryReader`:
//! the wrappers that let a caller intercept reader behaviour by delegating every
//! method it does not override.

use std::sync::Arc;

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::norms::NormsProducer;
use crate::codecs::points::PointsReader;
use crate::codecs::postings::FieldsProducer;
use crate::codecs::stored_fields::StoredFieldsReader;
use crate::codecs::term_vectors::TermVectorsReader;
use crate::error::Result;
use crate::index::codec_reader::CodecReader;
use crate::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
use crate::index::postings_enum::{ImpactsEnum, PostingsEnum};
use crate::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipper, FieldInfos, Fields, FloatVectorValues,
    NumericDocValues, PointValues, SeekStatus, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, TermState, Terms, TermsEnum,
};
use crate::search::knn::KnnCollector;
use crate::search::AcceptDocs;
use crate::search::DocIdSetIterator;
use crate::util::attribute::{AttributeSource, Unwrappable};
use crate::util::Bits;
use crate::util::{BytesRef, IOBooleanSupplier};

/// A [`LeafReader`] that forwards every call to a wrapped reader.
///
/// Equivalent to `org.apache.lucene.index.FilterLeafReader`.
///
/// **Divergence from Lucene 10.5.0.** Java makes this an abstract class that a
/// caller extends, overriding only what it needs. Rust has no implementation
/// inheritance, so the port is a concrete struct that forwards everything; a
/// caller that wants to intercept one method wraps this type and forwards the
/// rest to it, which is the same delegation with one more level of indirection.
/// Java's four inner base classes — `FilterLeafReader.FilterFields`,
/// `FilterTerms`, `FilterTermsEnum` and `FilterPostingsEnum` — are ported the
/// same way, as [`FilterFields`], [`FilterTerms`], [`FilterTermsEnum`] and
/// [`FilterPostingsEnum`].
pub struct FilterLeafReader {
    /// The wrapped reader.
    pub(crate) inner: Arc<dyn LeafReader>,
}

impl std::fmt::Debug for FilterLeafReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterLeafReader").finish_non_exhaustive()
    }
}

impl std::fmt::Display for FilterLeafReader {
    /// Port of `FilterLeafReader.toString()` (`FilterLeafReader.java:416-422`).
    ///
    /// Java hardcodes the literal `"FilterLeafReader("` instead of asking for
    /// its own simple name, so every subclass that does not override
    /// `toString()` — `ExitableFilterAtomicReader` and
    /// `SoftDeletesFilterLeafReader` among them — also renders under this name.
    /// Measured against `lucene-core-10.5.0.jar`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FilterLeafReader({})", self.inner)
    }
}

impl FilterLeafReader {
    /// Wraps `inner`.
    pub fn new(inner: Arc<dyn LeafReader>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped reader.
    ///
    /// Equivalent to `FilterLeafReader.getDelegate()`.
    pub fn get_delegate(&self) -> &Arc<dyn LeafReader> {
        &self.inner
    }

    /// Unwraps nested filters and returns the innermost reader.
    ///
    /// Equivalent to `FilterLeafReader.unwrap(LeafReader)`.
    pub fn unwrap(reader: &Arc<dyn LeafReader>) -> Arc<dyn LeafReader> {
        Arc::clone(reader)
    }
}

impl LeafReader for FilterLeafReader {
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

    fn get_field_infos(&self) -> FieldInfos {
        self.inner.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.inner.get_live_docs()
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
}

/// A [`Fields`] that forwards every call to a wrapped instance.
///
/// Equivalent to `org.apache.lucene.index.FilterLeafReader.FilterFields`
/// (`FilterLeafReader.java:57-89`): the base class every filtering `Fields`
/// extends, whose three methods — `iterator()`, `terms(String)` and `size()` —
/// all forward to the protected `in` field and are all overridable.
///
/// **Divergence from Lucene 10.5.0.** As [`FilterTerms`]: Java's abstract
/// class becomes a concrete struct that a subclass holds and forwards to,
/// overriding a method by implementing it itself instead of calling this
/// type's. The struct is generic over the delegate's type, defaulting to
/// `dyn Fields`, so that a subclass which only ever wraps one concrete type —
/// `MappedMultiFields` wraps a `MultiFields` — reaches it typed through
/// [`get_delegate`](Self::get_delegate), where Java needs a downcast of `in`;
/// and so that a delegate held only for the length of a call can be wrapped
/// as `FilterFields<dyn Fields + 'a>`. Java refuses a null delegate with a
/// `NullPointerException`; Rust's type system refuses it at compile time.
pub struct FilterFields<F: Fields + ?Sized = dyn Fields> {
    inner: Box<F>,
}

impl<F: Fields + ?Sized> FilterFields<F> {
    /// Wraps `inner`.
    ///
    /// Equivalent to `FilterFields(Fields)`.
    pub fn new(inner: Box<F>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped instance.
    ///
    /// Equivalent to reading the protected `FilterFields.in` field.
    pub fn get_delegate(&self) -> &F {
        self.inner.as_ref()
    }
}

impl<F: Fields + ?Sized> Fields for FilterFields<F> {
    /// Equivalent to `FilterFields.iterator()`, which returns `in.iterator()`.
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.inner.iterator()
    }

    /// Equivalent to `FilterFields.terms(String)`, which returns
    /// `in.terms(field)`.
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.inner.terms(field)
    }

    /// Equivalent to `FilterFields.size()`, which returns `in.size()`.
    fn size(&self) -> i32 {
        self.inner.size()
    }
}

/// A [`PostingsEnum`] that forwards every call to a wrapped instance.
///
/// Equivalent to `org.apache.lucene.index.FilterLeafReader.FilterPostingsEnum`
/// (`FilterLeafReader.java:253-320`), the base class every filtering
/// `PostingsEnum` extends. Java forwards exactly `docID()`, `freq()`,
/// `nextDoc()`, `advance(int)`, `nextPosition()`, `startOffset()`,
/// `endOffset()`, `getPayload()` and `cost()` to `in`, and implements
/// `Unwrappable<PostingsEnum>` by returning `in`. Every other method keeps the
/// inherited default — `slowAdvance`, `intoBitSet`, `docIDRunEnd` and
/// `nextPostings` run against *this* enum's `nextDoc`/`advance`, not the
/// delegate's specialisations — and this port forwards the same nine methods
/// and leaves the same defaults in place.
///
/// **Divergence from Lucene 10.5.0.** As [`FilterTerms`]: Java's abstract
/// class becomes a concrete struct that a subclass holds and forwards to.
pub struct FilterPostingsEnum {
    inner: Box<dyn PostingsEnum>,
}

impl FilterPostingsEnum {
    /// Wraps `inner`.
    ///
    /// Equivalent to `FilterPostingsEnum(PostingsEnum)`. Java refuses a null
    /// delegate with a `NullPointerException`; Rust's type system refuses it
    /// at compile time.
    pub fn new(inner: Box<dyn PostingsEnum>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped instance.
    ///
    /// Equivalent to reading the protected `FilterPostingsEnum.in` field.
    pub fn get_delegate(&self) -> &dyn PostingsEnum {
        self.inner.as_ref()
    }
}

impl std::fmt::Display for FilterPostingsEnum {
    /// `FilterLeafReader.FilterPostingsEnum` declares no `toString()`, so
    /// `java.lang.Object`'s applies. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.FilterLeafReader$FilterPostingsEnum",
            self,
        ))
    }
}

impl DocIdSetIterator for FilterPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl PostingsEnum for FilterPostingsEnum {
    fn freq(&self) -> Result<i32> {
        self.inner.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.inner.next_position()
    }

    fn start_offset(&self) -> i32 {
        self.inner.start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.inner.end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.inner.get_payload()
    }
}

impl Unwrappable<dyn PostingsEnum> for FilterPostingsEnum {
    /// Equivalent to `FilterPostingsEnum.unwrap()`, which returns `in`.
    fn unwrap(&self) -> &(dyn PostingsEnum + 'static) {
        self.inner.as_ref()
    }
}

/// A [`Terms`] that forwards every call to a wrapped instance.
///
/// Equivalent to `org.apache.lucene.index.FilterLeafReader.FilterTerms`, whose
/// `in` field this struct holds and whose every method is plain delegation.
///
/// **Divergence from Lucene 10.5.0.** Java makes it an abstract class that a
/// caller extends, overriding only what it needs; Rust has no implementation
/// inheritance, so the port is a concrete struct that a caller wraps and
/// forwards to — the same delegation with one more level of indirection. See
/// [`FilterLeafReader`] for the same divergence on the reader itself.
pub struct FilterTerms {
    inner: Box<dyn Terms>,
}

impl FilterTerms {
    /// Wraps `inner`.
    ///
    /// Equivalent to `FilterTerms(Terms)`. Java refuses a null delegate with a
    /// `NullPointerException`; Rust's type system refuses it at compile time.
    pub fn new(inner: Box<dyn Terms>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped instance.
    ///
    /// Equivalent to reading the protected `FilterTerms.in` field, which the
    /// Java subclasses do to reach the delegate without going through their own
    /// overrides.
    pub fn get_delegate(&self) -> &dyn Terms {
        self.inner.as_ref()
    }
}

impl Terms for FilterTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        self.inner.iterator()
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

/// A [`TermsEnum`] that forwards every call to a wrapped instance.
///
/// Equivalent to `org.apache.lucene.index.FilterLeafReader.FilterTermsEnum`.
///
/// **Divergence from Lucene 10.5.0.** As [`FilterTerms`]: Java's abstract class
/// becomes a concrete struct the caller wraps.
pub struct FilterTermsEnum {
    inner: Box<dyn TermsEnum>,
}

impl FilterTermsEnum {
    /// Wraps `inner`.
    ///
    /// Equivalent to `FilterTermsEnum(TermsEnum)`.
    pub fn new(inner: Box<dyn TermsEnum>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped instance.
    ///
    /// Equivalent to reading the protected `FilterTermsEnum.in` field.
    pub fn get_delegate(&self) -> &dyn TermsEnum {
        self.inner.as_ref()
    }
}

impl std::fmt::Display for FilterTermsEnum {
    /// `FilterLeafReader.FilterTermsEnum` declares no `toString()`, so
    /// `java.lang.Object`'s applies. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.FilterLeafReader$FilterTermsEnum",
            self,
        ))
    }
}

impl TermsEnum for FilterTermsEnum {
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

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.inner.next()
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

/// A [`CodecReader`] that forwards every call to a wrapped codec reader.
///
/// Equivalent to `org.apache.lucene.index.FilterCodecReader`.
///
/// **Divergence from Lucene 10.5.0.** Java's `FilterCodecReader` is
/// abstract: a filter extends it and overrides the calls it changes,
/// inheriting the forwarding of the rest. Rust has no implementation
/// inheritance, so this is a concrete struct that forwards every call to
/// [`inner`](Self::get_delegate) and can be used on its own as the identity
/// filter; the filters Java writes as subclasses — the reader
/// [`wrap_live_docs`](Self::wrap_live_docs) returns, `SortingCodecReader` —
/// are types of their own that implement [`CodecReader`], forward the calls
/// they do not change themselves, and report their delegate through
/// [`CodecReader::filter_codec_reader_delegate`] so that
/// [`unwrap`](Self::unwrap) sees through them as Java's `instanceof` does.
pub struct FilterCodecReader {
    /// The wrapped codec reader.
    pub(crate) inner: Arc<dyn CodecReader>,
}

impl std::fmt::Debug for FilterCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterCodecReader").finish_non_exhaustive()
    }
}

impl std::fmt::Display for FilterCodecReader {
    /// `org.apache.lucene.index.FilterCodecReader` declares no `toString()` of
    /// its own — unlike its sibling `FilterLeafReader` — so `Object`'s applies.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.FilterCodecReader",
            self,
        ))
    }
}

impl FilterCodecReader {
    /// Wraps `inner`.
    pub fn new(inner: Arc<dyn CodecReader>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped reader.
    ///
    /// Equivalent to `FilterCodecReader.getDelegate()`.
    pub fn get_delegate(&self) -> &Arc<dyn CodecReader> {
        &self.inner
    }

    /// Wraps `reader` so that it reports `live_docs` and `num_docs` instead of
    /// its own.
    ///
    /// Equivalent to the package-private
    /// `FilterCodecReader.wrapLiveDocs(CodecReader, Bits, int)`, which
    /// `SoftDeletesRetentionMergePolicy` uses to search a segment's deleted
    /// documents. The reader cache helper is dropped, because the live docs
    /// are being altered; the core one is kept, because the segment's own data
    /// is not.
    pub fn wrap_live_docs(
        reader: Arc<dyn CodecReader>,
        live_docs: Option<Arc<dyn Bits>>,
        num_docs: i32,
    ) -> LiveDocsCodecReader {
        LiveDocsCodecReader {
            inner: reader,
            live_docs,
            num_docs,
        }
    }

    /// Unwraps nested filters and returns the innermost codec reader.
    ///
    /// Equivalent to `FilterCodecReader.unwrap(CodecReader)`
    /// (`FilterCodecReader.java:44-49`): while the reader is a
    /// `FilterCodecReader`, step to its delegate.
    pub fn unwrap(reader: &Arc<dyn CodecReader>) -> Arc<dyn CodecReader> {
        let mut reader = Arc::clone(reader);
        while let Some(delegate) = reader.filter_codec_reader_delegate() {
            let delegate = Arc::clone(delegate);
            reader = delegate;
        }
        reader
    }
}

impl LeafReader for FilterCodecReader {
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

    fn get_field_infos(&self) -> FieldInfos {
        self.inner.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.inner.get_live_docs()
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
}

impl CodecReader for FilterCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.inner.get_fields_reader()
    }

    /// A `FilterCodecReader` (Java: the class itself) answers its
    /// delegate, `getDelegate()`.
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

/// A [`CodecReader`] that reports a caller-supplied live-docs set and document
/// count in place of the wrapped reader's own.
///
/// Equivalent to the anonymous `FilterCodecReader`
/// `FilterCodecReader.wrapLiveDocs` returns.
pub struct LiveDocsCodecReader {
    inner: Arc<dyn CodecReader>,
    live_docs: Option<Arc<dyn Bits>>,
    num_docs: i32,
}

impl std::fmt::Debug for LiveDocsCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveDocsCodecReader")
            .field("num_docs", &self.num_docs)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for LiveDocsCodecReader {
    /// The anonymous `FilterCodecReader` that `FilterCodecReader.wrapLiveDocs`
    /// returns overrides no `toString()`, so `Object`'s applies. The binary
    /// name javac gives it was measured from `lucene-core-10.5.0.jar`:
    /// `org.apache.lucene.index.FilterCodecReader$1`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.FilterCodecReader$1",
            self,
        ))
    }
}

/// Serves an `Arc<dyn Bits>` where a `Box<dyn Bits>` is expected.
///
/// Java hands out the `Bits` object itself; the crate's `getLiveDocs` returns
/// an owned box, so a shared set is wrapped rather than cloned.
#[derive(Debug)]
struct ArcBits(Arc<dyn Bits>);

impl Bits for ArcBits {
    fn get(&self, index: usize) -> bool {
        self.0.get(index)
    }

    /// Forwards to the wrapped bits' `try_get`, so that an error they raise
    /// (`ExitableDirectoryReader.java:359-387`, whose `get` throws through
    /// every wrapper that delegates to it) is not swallowed here.
    fn try_get(&self, index: usize) -> Result<bool> {
        self.0.try_get(index)
    }

    fn length(&self) -> usize {
        self.0.length()
    }
}

impl LeafReader for LiveDocsCodecReader {
    fn core(&self) -> &IndexReaderCore {
        self.inner.core()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.inner.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.num_docs
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
        // The live docs are being altered, so the reader is no longer cacheable.
        None
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

    fn get_field_infos(&self) -> FieldInfos {
        self.inner.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.live_docs
            .as_ref()
            .map(|b| Box::new(ArcBits(Arc::clone(b))) as Box<dyn Bits>)
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
}

impl CodecReader for LiveDocsCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.inner.get_fields_reader()
    }

    /// A `FilterCodecReader` (Java: the anonymous subclass `wrapLiveDocs` returns) answers its
    /// delegate, `getDelegate()`.
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
