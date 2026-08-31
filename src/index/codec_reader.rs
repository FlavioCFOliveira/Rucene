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
