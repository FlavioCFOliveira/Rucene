//! `CodecReader` ported from `org.apache.lucene.index`.

use std::sync::Arc;

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::norms::NormsProducer;
use crate::codecs::points::PointsReader;
use crate::codecs::postings::FieldsProducer;
use crate::codecs::stored_fields::StoredFieldsReader;
use crate::codecs::term_vectors::TermVectorsReader;
use crate::error::Result;
use crate::index::leaf_reader::LeafReader;
use crate::index::{DocValuesType, FieldInfo};

/// A [`LeafReader`] that exposes the raw codec producers backing it.
///
/// Equivalent to `org.apache.lucene.index.CodecReader`, which every merge, filter,
/// sorting and validation path in Lucene operates against rather than against
/// `SegmentReader` directly. It declares the same seven accessors as Lucene
/// 10.5.0, in the same order.
///
/// **Divergence from Lucene 10.5.0.** Java returns the producer object itself,
/// which is shared by reference. Rust cannot hand out a bare reference that
/// outlives the reader's internal lock, so each accessor returns
/// `Option<Arc<dyn …>>`: `Arc` reproduces Java's sharing semantics, and `Option`
/// reproduces the `null` a segment without that kind of data returns. The
/// `Result` wrapper carries the already-closed check that Java performs by
/// throwing `AlreadyClosedException`.
pub trait CodecReader: LeafReader {
    /// Returns the [`FieldInfo`] of `field`, but only when the field exists and
    /// is indexed with exactly `doc_values_type`; `None` otherwise.
    ///
    /// This is what every doc-values accessor of a codec reader consults before
    /// asking the doc-values producer for a field: a field that does not exist,
    /// carries no doc values, or carries a different kind of doc values, has no
    /// values of the requested kind.
    ///
    /// **Divergence from Lucene 10.5.0.** Java declares this `private`, so it
    /// is reachable only from the `final` accessors that sit beside it. Rust
    /// has no private trait method: a default body must be at least as visible
    /// as its trait, so the helper is public here. It returns an owned
    /// [`FieldInfo`] rather than a borrow because
    /// [`LeafReader::get_field_infos`] hands back an owned snapshot of the
    /// field infos, which the borrow could not outlive.
    ///
    /// Equivalent to the private `CodecReader.getDVField(String, DocValuesType)`.
    fn get_dv_field(&self, field: &str, doc_values_type: DocValuesType) -> Option<FieldInfo> {
        let field_infos = self.get_field_infos();
        let fi = field_infos.field_info(field)?;
        if fi.doc_values_type == DocValuesType::NONE {
            // Field was not indexed with doc values.
            return None;
        }
        if fi.doc_values_type != doc_values_type {
            // Field doc values are of a different type than requested.
            return None;
        }
        Some(fi.clone())
    }

    /// Returns the stored-fields reader for this segment.
    ///
    /// Equivalent to `CodecReader.getFieldsReader()`.
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>>;

    /// Returns the term-vectors reader for this segment.
    ///
    /// Equivalent to `CodecReader.getTermVectorsReader()`.
    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>>;

    /// Returns the norms producer for this segment.
    ///
    /// Equivalent to `CodecReader.getNormsReader()`.
    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>>;

    /// Returns the doc-values producer for this segment.
    ///
    /// Equivalent to `CodecReader.getDocValuesReader()`.
    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>>;

    /// Returns the postings producer for this segment.
    ///
    /// Equivalent to `CodecReader.getPostingsReader()`.
    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>>;

    /// Returns the points reader for this segment.
    ///
    /// Equivalent to `CodecReader.getPointsReader()`.
    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>>;

    /// Returns the KNN vectors reader for this segment.
    ///
    /// Equivalent to `CodecReader.getVectorReader()`.
    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>>;

    /// Returns the reader this one wraps when it is a `FilterCodecReader`, and
    /// `None` otherwise.
    ///
    /// Equivalent to `reader instanceof FilterCodecReader` followed by
    /// `getDelegate()`, which is how `FilterCodecReader.unwrap` walks a chain
    /// of filters (`FilterCodecReader.java:44-49`). Rust cannot test a
    /// `dyn CodecReader` for a type, so each port of a `FilterCodecReader`
    /// subclass answers for itself; every other reader keeps this default.
    fn filter_codec_reader_delegate(&self) -> Option<&Arc<dyn CodecReader>> {
        None
    }
}

// -----------------------------------------------------------------------------
// CodecReader's final LeafReader members
// -----------------------------------------------------------------------------

/// The bodies of the `LeafReader` members Java's `CodecReader` declares
/// `final` (`index/CodecReader.java:86-290`), which answer every leaf query
/// through the reader's own codec producers.
///
/// Rust has no `final` trait member that overrides a supertrait's, so a
/// [`CodecReader`] whose leaf view must go through its producers forwards its
/// [`LeafReader`] members to these functions. A wrapper that substitutes its
/// own producers — `SortingCodecReader`, `SlowCompositeCodecReaderWrapper` —
/// thereby answers leaf queries from the substituted producers, as in Java.
pub(crate) mod final_members {
    use crate::codecs::stored_fields::StoredFieldsReader;
    use crate::error::{LuceneError, Result};
    use crate::index::codec_reader::CodecReader;
    use crate::index::index_reader::StoredFields;
    use crate::index::leaf_reader::{EmptyTermVectors, TermVectors};
    use crate::index::{
        BinaryDocValues, ByteVectorValues, DocValuesSkipIndexType, DocValuesSkipper, DocValuesType,
        FloatVectorValues, NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues,
        SortedSetDocValues, StoredFieldVisitor, Terms, VectorEncoding,
    };
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;

    /// The `StoredFields` view `CodecReader.storedFields()` returns: every
    /// access checks the document number against `maxDoc` before reaching the
    /// codec (`index/CodecReader.java:86-104`).
    #[derive(Debug)]
    struct CodecStoredFields {
        reader: Option<Box<dyn StoredFieldsReader>>,
        max_doc: i32,
    }

    impl CodecStoredFields {
        /// Equivalent to `Objects.checkIndex(docID, maxDoc())`, then the
        /// dereference of the possibly-null fields reader.
        fn checked(&self, doc_id: i32) -> Result<&dyn StoredFieldsReader> {
            if doc_id < 0 || doc_id >= self.max_doc {
                return Err(LuceneError::IllegalArgument(format!(
                    "Index {doc_id} out of bounds for length {}",
                    self.max_doc
                )));
            }
            self.reader.as_deref().ok_or_else(|| {
                LuceneError::illegal_state("this reader has no stored fields reader".to_string())
            })
        }
    }

    impl StoredFields for CodecStoredFields {
        fn prefetch(&mut self, doc_id: i32) -> Result<()> {
            self.checked(doc_id)?.prefetch(doc_id)
        }

        fn document_with_visitor(
            &self,
            doc_id: i32,
            visitor: &mut dyn StoredFieldVisitor,
        ) -> Result<()> {
            self.checked(doc_id)?.document(doc_id, visitor)
        }
    }

    /// Equivalent to `CodecReader.storedFields()` (`index/CodecReader.java:86-104`).
    ///
    /// # Errors
    ///
    /// Propagates whatever asking for the fields reader raises.
    pub(crate) fn stored_fields<R: CodecReader + ?Sized>(
        reader: &R,
    ) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(CodecStoredFields {
            reader: reader.get_fields_reader()?,
            max_doc: reader.max_doc(),
        }))
    }

    /// Equivalent to `CodecReader.termVectors()` (`index/CodecReader.java:106-114`):
    /// the term-vectors reader itself, or `TermVectors.EMPTY` without one.
    ///
    /// # Errors
    ///
    /// Propagates whatever asking for the term-vectors reader raises.
    pub(crate) fn term_vectors<R: CodecReader + ?Sized>(
        reader: &R,
    ) -> Result<Box<dyn TermVectors>> {
        Ok(match reader.get_term_vectors_reader()? {
            Some(reader) => reader as Box<dyn TermVectors>,
            None => Box::new(EmptyTermVectors),
        })
    }

    /// Equivalent to `CodecReader.terms(String)` (`index/CodecReader.java:116-125`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the reader is closed, and
    /// propagates whatever the postings reader raises.
    pub(crate) fn terms<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn Terms>>> {
        reader.core().ensure_open()?;
        let field_infos = reader.get_field_infos();
        match field_infos.field_info(field) {
            Some(fi) if fi.index_options != crate::index::IndexOptions::NONE => {}
            // Field does not exist or does not index postings.
            _ => return Ok(None),
        }
        match reader.get_postings_reader()? {
            Some(postings) => postings.terms(field),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getNumericDocValues(String)`
    /// (`index/CodecReader.java:148-156`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the reader is closed, and
    /// propagates whatever the doc-values producer raises.
    pub(crate) fn numeric_doc_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn NumericDocValues>>> {
        reader.core().ensure_open()?;
        let Some(fi) = reader.get_dv_field(field, DocValuesType::NUMERIC) else {
            return Ok(None);
        };
        match reader.get_doc_values_reader()? {
            Some(producer) => producer.get_numeric(&fi),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getBinaryDocValues(String)`
    /// (`index/CodecReader.java:158-166`).
    ///
    /// # Errors
    ///
    /// As [`numeric_doc_values`].
    pub(crate) fn binary_doc_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn BinaryDocValues>>> {
        reader.core().ensure_open()?;
        let Some(fi) = reader.get_dv_field(field, DocValuesType::BINARY) else {
            return Ok(None);
        };
        match reader.get_doc_values_reader()? {
            Some(producer) => producer.get_binary(&fi),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getSortedDocValues(String)`
    /// (`index/CodecReader.java:168-176`).
    ///
    /// # Errors
    ///
    /// As [`numeric_doc_values`].
    pub(crate) fn sorted_doc_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn SortedDocValues>>> {
        reader.core().ensure_open()?;
        let Some(fi) = reader.get_dv_field(field, DocValuesType::SORTED) else {
            return Ok(None);
        };
        match reader.get_doc_values_reader()? {
            Some(producer) => producer.get_sorted(&fi),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getSortedNumericDocValues(String)`
    /// (`index/CodecReader.java:178-186`).
    ///
    /// # Errors
    ///
    /// As [`numeric_doc_values`].
    pub(crate) fn sorted_numeric_doc_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        reader.core().ensure_open()?;
        let Some(fi) = reader.get_dv_field(field, DocValuesType::SORTED_NUMERIC) else {
            return Ok(None);
        };
        match reader.get_doc_values_reader()? {
            Some(producer) => producer.get_sorted_numeric(&fi),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getSortedSetDocValues(String)`
    /// (`index/CodecReader.java:189-197`).
    ///
    /// # Errors
    ///
    /// As [`numeric_doc_values`].
    pub(crate) fn sorted_set_doc_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        reader.core().ensure_open()?;
        let Some(fi) = reader.get_dv_field(field, DocValuesType::SORTED_SET) else {
            return Ok(None);
        };
        match reader.get_doc_values_reader()? {
            Some(producer) => producer.get_sorted_set(&fi),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getDocValuesSkipper(String)`
    /// (`index/CodecReader.java:199-207`).
    ///
    /// # Errors
    ///
    /// As [`numeric_doc_values`].
    pub(crate) fn doc_values_skipper<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        reader.core().ensure_open()?;
        let field_infos = reader.get_field_infos();
        let fi = match field_infos.field_info(field) {
            Some(fi) if fi.doc_values_skip_index_type != DocValuesSkipIndexType::NONE => fi,
            _ => return Ok(None),
        };
        match reader.get_doc_values_reader()? {
            Some(producer) => producer.get_skipper(fi),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getNormValues(String)`
    /// (`index/CodecReader.java:209-219`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the reader is closed, and
    /// propagates whatever the norms producer raises.
    pub(crate) fn norm_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn NumericDocValues>>> {
        reader.core().ensure_open()?;
        let field_infos = reader.get_field_infos();
        let fi = match field_infos.field_info(field) {
            Some(fi) if fi.has_norms() => fi,
            // Field does not exist or does not index norms.
            _ => return Ok(None),
        };
        match reader.get_norms_reader()? {
            Some(producer) => producer.get_norms(fi),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getPointValues(String)`
    /// (`index/CodecReader.java:221-231`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the reader is closed, and
    /// propagates whatever the points reader raises.
    pub(crate) fn point_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn PointValues>>> {
        reader.core().ensure_open()?;
        let field_infos = reader.get_field_infos();
        match field_infos.field_info(field) {
            Some(fi) if fi.point_dimension_count != 0 => {}
            // Field does not exist or does not index points.
            _ => return Ok(None),
        }
        match reader.get_points_reader()? {
            Some(points) => points.get_values(field),
            None => Ok(None),
        }
    }

    /// Whether `field` holds vectors of `encoding`, as each vector member of
    /// `CodecReader` checks first (`index/CodecReader.java:233-289`).
    fn has_vectors<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
        encoding: VectorEncoding,
    ) -> bool {
        reader
            .get_field_infos()
            .field_info(field)
            .is_some_and(|fi| fi.vector_dimension != 0 && fi.vector_encoding == encoding)
    }

    /// Equivalent to `CodecReader.getFloatVectorValues(String)`
    /// (`index/CodecReader.java:233-245`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the reader is closed, and
    /// propagates whatever the vectors reader raises.
    pub(crate) fn float_vector_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn FloatVectorValues>>> {
        reader.core().ensure_open()?;
        if !has_vectors(reader, field, VectorEncoding::FLOAT32) {
            return Ok(None);
        }
        match reader.get_vector_reader()? {
            Some(vectors) => vectors.get_float_vector_values(field),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.getByteVectorValues(String)`
    /// (`index/CodecReader.java:247-259`).
    ///
    /// # Errors
    ///
    /// As [`float_vector_values`].
    pub(crate) fn byte_vector_values<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
    ) -> Result<Option<Box<dyn ByteVectorValues>>> {
        reader.core().ensure_open()?;
        if !has_vectors(reader, field, VectorEncoding::BYTE) {
            return Ok(None);
        }
        match reader.get_vector_reader()? {
            Some(vectors) => vectors.get_byte_vector_values(field),
            None => Ok(None),
        }
    }

    /// Equivalent to `CodecReader.searchNearestVectors(String, float[],
    /// KnnCollector, AcceptDocs)` (`index/CodecReader.java:261-274`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the reader is closed, and
    /// propagates whatever the vectors reader's search raises.
    pub(crate) fn search_nearest_vectors<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        reader.core().ensure_open()?;
        if !has_vectors(reader, field, VectorEncoding::FLOAT32) {
            return Ok(());
        }
        match reader.get_vector_reader()? {
            Some(vectors) => vectors.search(field, target, collector, accept_docs),
            None => Ok(()),
        }
    }

    /// Equivalent to `CodecReader.searchNearestVectors(String, byte[],
    /// KnnCollector, AcceptDocs)` (`index/CodecReader.java:276-289`).
    ///
    /// # Errors
    ///
    /// As [`search_nearest_vectors`].
    pub(crate) fn search_nearest_vectors_byte<R: CodecReader + ?Sized>(
        reader: &R,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        reader.core().ensure_open()?;
        if !has_vectors(reader, field, VectorEncoding::BYTE) {
            return Ok(());
        }
        match reader.get_vector_reader()? {
            Some(vectors) => vectors.search_byte(field, target, collector, accept_docs),
            None => Ok(()),
        }
    }

    /// Equivalent to `CodecReader.checkIntegrity()` (`index/CodecReader.java:294-332`):
    /// every producer the reader has, in Java's order.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the reader is closed, and
    /// propagates the first integrity failure.
    pub(crate) fn check_integrity<R: CodecReader + ?Sized>(reader: &R) -> Result<()> {
        reader.core().ensure_open()?;
        // terms/postings
        if let Some(postings) = reader.get_postings_reader()? {
            postings.check_integrity()?;
        }
        // norms
        if let Some(norms) = reader.get_norms_reader()? {
            norms.check_integrity()?;
        }
        // docvalues
        if let Some(doc_values) = reader.get_doc_values_reader()? {
            doc_values.check_integrity()?;
        }
        // stored fields
        if let Some(stored_fields) = reader.get_fields_reader()? {
            stored_fields.check_integrity()?;
        }
        // term vectors
        if let Some(term_vectors) = reader.get_term_vectors_reader()? {
            term_vectors.check_integrity()?;
        }
        // points
        if let Some(points) = reader.get_points_reader()? {
            points.check_integrity()?;
        }
        // vectors
        if let Some(vectors) = reader.get_vector_reader()? {
            vectors.check_integrity()?;
        }
        Ok(())
    }
}
