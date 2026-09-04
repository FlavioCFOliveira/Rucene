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
}
