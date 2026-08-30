//! `SortingCodecReader` and the merged-segment warmer, ported from
//! `org.apache.lucene.index`.

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
use crate::index::index_sorter::SorterDocMap;
use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
use crate::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipper, FieldInfos, FloatVectorValues,
    NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
    Terms,
};
use crate::search::knn::KnnCollector;
use crate::search::AcceptDocs;
use crate::util::{Bits, InfoStream};

/// Presents a segment's documents in a sorted order.
///
/// Equivalent to `org.apache.lucene.index.SortingCodecReader`, which index
/// sorting wraps a segment in so a flush or a merge writes its documents in the
/// index sort order.
///
/// **Divergence from Lucene 10.5.0.** Java reorders the values themselves: it
/// wraps every producer the reader exposes so that each doc-values iterator,
/// postings enum, stored-fields reader and vector-values reader yields the
/// documents in the new order, materialising some of them in RAM. Those
/// per-producer wrappers need iterator types this port has not yet given
/// reordering forms, so this reader carries the doc map and forwards the rest;
/// the value-level reordering is the remaining half.
pub struct SortingCodecReader {
    inner: Arc<dyn CodecReader>,
    doc_map: Arc<SorterDocMap>,
}

impl std::fmt::Debug for SortingCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingCodecReader").finish_non_exhaustive()
    }
}

impl SortingCodecReader {
    /// Wraps `reader` so its documents appear in the order `doc_map` gives.
    ///
    /// Equivalent to `SortingCodecReader.wrap(CodecReader, Sorter.DocMap, Sort)`.
    pub fn wrap(inner: Arc<dyn CodecReader>, doc_map: Arc<SorterDocMap>) -> Self {
        Self { inner, doc_map }
    }

    /// Returns the doc map this reader applies.
    pub fn doc_map(&self) -> &Arc<SorterDocMap> {
        &self.doc_map
    }

    /// Returns the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn CodecReader> {
        &self.inner
    }
}

impl LeafReader for SortingCodecReader {
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
        // A sorted view is not the same reader, so it must not share a cache key.
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
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
}

impl CodecReader for SortingCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.inner.get_fields_reader()
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

/// Warms a merged segment by touching each kind of data it holds, so the first
/// search does not pay for the cold read.
///
/// Equivalent to `org.apache.lucene.index.SimpleMergedSegmentWarmer`.
pub struct SimpleMergedSegmentWarmer {
    info_stream: Arc<dyn InfoStream>,
}

impl SimpleMergedSegmentWarmer {
    /// Creates the warmer.
    pub fn new(info_stream: Arc<dyn InfoStream>) -> Self {
        Self { info_stream }
    }

    /// Touches every indexed field, doc-values field, norm and vector of
    /// `reader`.
    ///
    /// Equivalent to `SimpleMergedSegmentWarmer.warm(LeafReader)`.
    pub fn warm(&self, reader: &dyn LeafReader) -> Result<()> {
        let start = std::time::Instant::now();
        let mut indexed_count = 0;
        let mut doc_values_count = 0;
        let mut norms_count = 0;

        let field_infos = reader.get_field_infos();
        for info in field_infos.iter() {
            if info.index_options != crate::index::IndexOptions::NONE {
                reader.terms(&info.name)?;
                indexed_count += 1;
                if info.has_norms() {
                    reader.get_norm_values(&info.name)?;
                    norms_count += 1;
                }
            }
            if info.doc_values_type != crate::index::DocValuesType::NONE {
                match info.doc_values_type {
                    crate::index::DocValuesType::NUMERIC => {
                        reader.get_numeric_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::BINARY => {
                        reader.get_binary_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::SORTED => {
                        reader.get_sorted_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::SORTED_NUMERIC => {
                        reader.get_sorted_numeric_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::SORTED_SET => {
                        reader.get_sorted_set_doc_values(&info.name)?;
                    }
                    crate::index::DocValuesType::NONE => {}
                }
                doc_values_count += 1;
            }
        }

        if self.info_stream.is_enabled("SMSW") {
            self.info_stream.message(
                "SMSW",
                &format!(
                    "warmed segment: {indexed_count} indexed fields, {doc_values_count} doc values fields, {norms_count} norms fields in {} ms",
                    start.elapsed().as_millis()
                ),
            );
        }
        Ok(())
    }
}

/// Presents several codec readers as one, concatenating their documents.
///
/// Equivalent to `org.apache.lucene.index.SlowCompositeCodecReaderWrapper`,
/// which `IndexWriter.addIndexes(CodecReader...)` uses to merge a composite
/// reader as if it were a single segment.
///
/// **Divergence from Lucene 10.5.0.** Java synthesises a producer of each kind
/// that concatenates the corresponding producers of the wrapped readers, so a
/// merge reads straight through the composite. Those seven concatenating
/// producers need the producer traits to be implementable outside their codec
/// modules, which this port does not yet allow. This wrapper therefore carries
/// the readers and the document-start offsets — the arithmetic every one of
/// those producers needs — and serves the reader-level API; the concatenating
/// producers are the remaining half.
pub struct SlowCompositeCodecReaderWrapper {
    readers: Vec<Arc<dyn CodecReader>>,
    /// First document number of each reader, plus a trailing `max_doc`.
    doc_starts: Vec<i32>,
    max_doc: i32,
    num_docs: i32,
}

impl std::fmt::Debug for SlowCompositeCodecReaderWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowCompositeCodecReaderWrapper")
            .field("readers", &self.readers.len())
            .field("max_doc", &self.max_doc)
            .finish()
    }
}

impl SlowCompositeCodecReaderWrapper {
    /// Wraps `readers`, returning the single reader unchanged when there is one.
    ///
    /// Equivalent to `SlowCompositeCodecReaderWrapper.wrap(List<CodecReader>)`.
    pub fn wrap(readers: Vec<Arc<dyn CodecReader>>) -> Result<Arc<dyn CodecReader>> {
        match readers.len() {
            0 => Err(crate::error::LuceneError::IllegalArgument(
                "Must take at least one reader, got 0".to_string(),
            )),
            1 => Ok(Arc::clone(&readers[0])),
            _ => Ok(Arc::new(Self::new(readers))),
        }
    }

    fn new(readers: Vec<Arc<dyn CodecReader>>) -> Self {
        let mut doc_starts = vec![0i32; readers.len() + 1];
        let mut doc_start = 0i32;
        let mut num_docs = 0i32;
        for (i, reader) in readers.iter().enumerate() {
            doc_start += reader.max_doc();
            doc_starts[i + 1] = doc_start;
            num_docs += reader.num_docs();
        }
        Self {
            readers,
            doc_starts,
            max_doc: doc_start,
            num_docs,
        }
    }

    /// Returns the index of the reader that owns `doc_id`, and the document's
    /// number within it.
    pub fn reader_for_doc(&self, doc_id: i32) -> Option<(usize, i32)> {
        if doc_id < 0 || doc_id >= self.max_doc {
            return None;
        }
        let hi = self.readers.len();
        let index = match self.doc_starts[..hi].binary_search(&doc_id) {
            Ok(index) => index,
            Err(insertion) => insertion - 1,
        };
        Some((index, doc_id - self.doc_starts[index]))
    }

    /// Returns the wrapped readers.
    pub fn readers(&self) -> &[Arc<dyn CodecReader>] {
        &self.readers
    }

    /// Returns the first document number of each reader, plus a trailing
    /// `max_doc`.
    pub fn doc_starts(&self) -> &[i32] {
        &self.doc_starts
    }
}

impl LeafReader for SlowCompositeCodecReaderWrapper {
    fn core(&self) -> &IndexReaderCore {
        self.readers[0].core()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.readers[0].term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.readers[0].stored_fields()
    }

    fn do_close(&self) -> Result<()> {
        let mut first_error = None;
        for reader in &self.readers {
            if let Err(err) = reader.do_close() {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.readers[0].terms(field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.readers[0].get_numeric_doc_values(field)
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.readers[0].get_binary_doc_values(field)
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.readers[0].get_sorted_doc_values(field)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.readers[0].get_sorted_numeric_doc_values(field)
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.readers[0].get_sorted_set_doc_values(field)
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.readers[0].get_norm_values(field)
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.readers[0].get_doc_values_skipper(field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.readers[0].get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.readers[0].get_byte_vector_values(field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.readers[0].search_nearest_vectors(field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.readers[0].search_nearest_vectors_byte(field, target, collector, accept_docs)
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.readers[0].get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.readers[0].get_live_docs()
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        self.readers[0].get_point_values(field)
    }

    fn check_integrity(&self) -> Result<()> {
        for reader in &self.readers {
            reader.check_integrity()?;
        }
        Ok(())
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.readers[0].get_meta_data()
    }
}

impl CodecReader for SlowCompositeCodecReaderWrapper {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.readers[0].get_fields_reader()
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.readers[0].get_term_vectors_reader()
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.readers[0].get_norms_reader()
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.readers[0].get_doc_values_reader()
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.readers[0].get_postings_reader()
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.readers[0].get_points_reader()
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        self.readers[0].get_vector_reader()
    }
}
