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
    BinaryDocValues, ByteVectorValues, DocValuesIterator, DocValuesSkipper, EmptyDocValuesSkipper,
    FieldInfo, FieldInfos, Fields, FloatVectorValues, IntersectVisitor, NumericDocValues,
    PointTree, PointValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
    StoredFieldVisitor, Terms,
};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::knn::KnnCollector;
use crate::search::AcceptDocs;
use crate::util::{BitSet, Bits, BytesRef, FixedBitSet, InfoStream};

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

// -----------------------------------------------------------------------------
// Sorted doc-values wrappers
// -----------------------------------------------------------------------------
//
// Each wrapper eagerly drains the old iterator once, keyed by the *new*
// document number `doc_map.old_to_new` assigns, then serves the
// `DocIdSetIterator`/value contract from that materialised form. This mirrors
// Java's `NumericDocValuesWriter.SortingNumericDocValues` and its four
// siblings, which `SortingCodecReader.getDocValuesReader()` installs — see
// that class's javadoc for why eager materialisation, rather than a lazy
// remap, is unavoidable: a merged docID is not monotonic in the old one, so
// the values cannot be streamed in the new order without buffering them.

/// Sorted view of a single-valued numeric field.
///
/// Equivalent to `NumericDocValuesWriter.SortingNumericDocValues`, backed by
/// the same `NumericDVs` (bitset plus dense array) Java builds.
#[derive(Debug)]
struct SortingNumericDocValues {
    docs_with_field: FixedBitSet,
    values: Vec<i64>,
    doc_id: i32,
}

impl SortingNumericDocValues {
    fn new(
        max_doc: i32,
        doc_map: &SorterDocMap,
        mut old: Box<dyn NumericDocValues>,
    ) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut values = vec![0i64; max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            values[new_doc_id as usize] = old.long_value()?;
        }
        Ok(Self {
            docs_with_field,
            values,
            doc_id: -1,
        })
    }
}

impl DocIdSetIterator for SortingNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.docs_with_field.next_set_bit(start);
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.docs_with_field.next_set_bit(target);
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.docs_with_field.get(target as usize))
    }
}

impl NumericDocValues for SortingNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        Ok(self.values[self.doc_id as usize])
    }
}

/// Sorted view of a single-valued binary field.
///
/// Equivalent to `BinaryDocValuesWriter.SortingBinaryDocValues`.
#[derive(Debug)]
struct SortingBinaryDocValues {
    docs_with_field: FixedBitSet,
    values: Vec<BytesRef>,
    doc_id: i32,
}

impl SortingBinaryDocValues {
    fn new(
        max_doc: i32,
        doc_map: &SorterDocMap,
        mut old: Box<dyn BinaryDocValues>,
    ) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut values = vec![BytesRef::default(); max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            values[new_doc_id as usize] = BytesRef::deep_copy_of(&old.binary_value()?);
        }
        Ok(Self {
            docs_with_field,
            values,
            doc_id: -1,
        })
    }
}

impl DocIdSetIterator for SortingBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.docs_with_field.next_set_bit(start);
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.docs_with_field.next_set_bit(target);
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.docs_with_field.get(target as usize))
    }
}

impl BinaryDocValues for SortingBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        Ok(self.values[self.doc_id as usize].clone())
    }
}

/// Sorted view of a single-valued sorted (dictionary-backed) field.
///
/// Equivalent to `SortedDocValuesWriter.SortingSortedDocValues`. The term
/// dictionary is unaffected by document reordering — only which document
/// points at which ordinal changes — so ordinal lookups delegate to `old`,
/// which is kept alive purely for that dictionary.
struct SortingSortedDocValues {
    old: Box<dyn SortedDocValues>,
    docs_with_field: FixedBitSet,
    ords: Vec<i32>,
    doc_id: i32,
}

impl std::fmt::Debug for SortingSortedDocValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingSortedDocValues")
            .field("doc_id", &self.doc_id)
            .finish_non_exhaustive()
    }
}

impl SortingSortedDocValues {
    fn new(
        max_doc: i32,
        doc_map: &SorterDocMap,
        mut old: Box<dyn SortedDocValues>,
    ) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut ords = vec![-1i32; max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            ords[new_doc_id as usize] = old.ord_value()?;
        }
        Ok(Self {
            old,
            docs_with_field,
            ords,
            doc_id: -1,
        })
    }
}

impl DocIdSetIterator for SortingSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.docs_with_field.next_set_bit(start);
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.docs_with_field.next_set_bit(target);
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingSortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.docs_with_field.get(target as usize))
    }
}

impl SortedDocValues for SortingSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ords[self.doc_id as usize])
    }

    fn get_value_count(&self) -> Result<i32> {
        self.old.get_value_count()
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        self.old.lookup_ord(ord)
    }
}

/// Sorted view of a multi-valued numeric field.
///
/// Equivalent to `SortedNumericDocValuesWriter.SortingSortedNumericDocValues`.
///
/// **Divergence from Lucene 10.5.0.** Java packs the per-document value lists
/// into one `PackedLongValues`-backed structure (`LongValues`). This port keeps
/// one `Vec<i64>` per document, which is simpler and observably identical —
/// only the memory layout differs, exactly the kind of divergence
/// [`FieldUpdatesBuffer`] already accepts for the same reason.
#[derive(Debug)]
struct SortingSortedNumericDocValues {
    docs_with_field: FixedBitSet,
    values: Vec<Vec<i64>>,
    doc_id: i32,
    value_idx: usize,
}

impl SortingSortedNumericDocValues {
    fn new(
        max_doc: i32,
        doc_map: &SorterDocMap,
        mut old: Box<dyn SortedNumericDocValues>,
    ) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut values: Vec<Vec<i64>> = vec![Vec::new(); max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            let count = old.doc_value_count()?;
            let slot = &mut values[new_doc_id as usize];
            for _ in 0..count {
                slot.push(old.next_value()?);
            }
        }
        Ok(Self {
            docs_with_field,
            values,
            doc_id: -1,
            value_idx: 0,
        })
    }
}

impl DocIdSetIterator for SortingSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.docs_with_field.next_set_bit(start);
        self.value_idx = 0;
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.docs_with_field.next_set_bit(target);
        self.value_idx = 0;
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        self.value_idx = 0;
        Ok(self.docs_with_field.get(target as usize))
    }
}

impl SortedNumericDocValues for SortingSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        let value = self.values[self.doc_id as usize][self.value_idx];
        self.value_idx += 1;
        Ok(value)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.values[self.doc_id as usize].len() as i32)
    }
}

/// Sorted view of a multi-valued sorted-set field.
///
/// Equivalent to `SortedSetDocValuesWriter.SortingSortedSetDocValues`. As with
/// [`SortingSortedDocValues`], the term dictionary is unaffected by document
/// reordering, so `old` is kept alive for ordinal lookups only.
///
/// **Divergence from Lucene 10.5.0.** Same as
/// [`SortingSortedNumericDocValues`]: one `Vec<i64>` per document instead of a
/// packed structure.
struct SortingSortedSetDocValues {
    old: Box<dyn SortedSetDocValues>,
    docs_with_field: FixedBitSet,
    ords: Vec<Vec<i64>>,
    doc_id: i32,
    ord_idx: usize,
}

impl std::fmt::Debug for SortingSortedSetDocValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingSortedSetDocValues")
            .field("doc_id", &self.doc_id)
            .finish_non_exhaustive()
    }
}

impl SortingSortedSetDocValues {
    fn new(
        max_doc: i32,
        doc_map: &SorterDocMap,
        mut old: Box<dyn SortedSetDocValues>,
    ) -> Result<Self> {
        let mut docs_with_field = FixedBitSet::new(max_doc as usize);
        let mut ords: Vec<Vec<i64>> = vec![Vec::new(); max_doc as usize];
        loop {
            let doc_id = old.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = doc_map.old_to_new(doc_id);
            docs_with_field.set(new_doc_id as usize);
            let count = old.doc_value_count()?;
            let slot = &mut ords[new_doc_id as usize];
            for _ in 0..count {
                slot.push(old.next_ord()?);
            }
        }
        Ok(Self {
            old,
            docs_with_field,
            ords,
            doc_id: -1,
            ord_idx: 0,
        })
    }
}

impl DocIdSetIterator for SortingSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        let start = if self.doc_id < 0 { 0 } else { self.doc_id + 1 };
        self.doc_id = self.docs_with_field.next_set_bit(start);
        self.ord_idx = 0;
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = self.docs_with_field.next_set_bit(target);
        self.ord_idx = 0;
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cardinality() as i64
    }
}

impl DocValuesIterator for SortingSortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        self.ord_idx = 0;
        Ok(self.docs_with_field.get(target as usize))
    }
}

impl SortedSetDocValues for SortingSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        let ord = self.ords[self.doc_id as usize][self.ord_idx];
        self.ord_idx += 1;
        Ok(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.ords[self.doc_id as usize].len() as i32)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.old.lookup_ord(ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        self.old.get_value_count()
    }
}

// -----------------------------------------------------------------------------
// Sorted stored-fields and term-vectors wrappers
// -----------------------------------------------------------------------------

/// Sorted view of a [`StoredFieldsReader`]: every access remaps the requested
/// new document number to the old one before delegating.
///
/// Equivalent to the anonymous `StoredFieldsReader`
/// `SortingCodecReader.newStoredFieldsReader` builds.
struct SortingStoredFieldsReader {
    delegate: Box<dyn StoredFieldsReader>,
    doc_map: Arc<SorterDocMap>,
}

impl std::fmt::Debug for SortingStoredFieldsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingStoredFieldsReader")
            .finish_non_exhaustive()
    }
}

impl StoredFieldsReader for SortingStoredFieldsReader {
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        self.delegate
            .document(self.doc_map.new_to_old(doc_id), visitor)
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        Box::new(SortingStoredFieldsReader {
            delegate: self.delegate.clone_reader(),
            doc_map: Arc::clone(&self.doc_map),
        })
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
        self.delegate.prefetch(self.doc_map.new_to_old(doc_id))
    }
}

/// Sorted view of a [`TermVectorsReader`]: `get` remaps the requested new
/// document number to the old one before delegating.
///
/// Equivalent to the anonymous `TermVectorsReader`
/// `SortingCodecReader.newTermVectorsReader` builds.
struct SortingTermVectorsReader {
    delegate: Box<dyn TermVectorsReader>,
    doc_map: Arc<SorterDocMap>,
}

impl std::fmt::Debug for SortingTermVectorsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingTermVectorsReader")
            .finish_non_exhaustive()
    }
}

impl TermVectorsReader for SortingTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(SortingTermVectorsReader {
            delegate: self.delegate.clone_reader(),
            doc_map: Arc::clone(&self.doc_map),
        })
    }

    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        self.delegate.get(self.doc_map.new_to_old(doc))
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
        self.delegate.prefetch(self.doc_map.new_to_old(doc_id))
    }
}

/// Sorted view of a [`Bits`] (typically live docs): reads bit `doc_map.new_to_old(index)`.
///
/// Equivalent to `SortingCodecReader.SortingBits`.
struct SortingBits {
    inner: Box<dyn Bits>,
    doc_map: Arc<SorterDocMap>,
}

impl std::fmt::Debug for SortingBits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingBits").finish_non_exhaustive()
    }
}

impl Bits for SortingBits {
    fn get(&self, index: usize) -> bool {
        self.inner
            .get(self.doc_map.new_to_old(index as i32) as usize)
    }

    fn length(&self) -> usize {
        self.inner.length()
    }
}

// -----------------------------------------------------------------------------
// Sorted doc-values / norms producer wrappers
// -----------------------------------------------------------------------------

/// Sorted view of a [`DocValuesProducer`]: every accessor eagerly rebuilds its
/// values keyed by the new document order.
///
/// Equivalent to the anonymous `DocValuesProducer`
/// `SortingCodecReader.getDocValuesReader()` builds.
struct SortingDocValuesProducer {
    delegate: Arc<dyn DocValuesProducer>,
    doc_map: Arc<SorterDocMap>,
    max_doc: i32,
}

impl std::fmt::Debug for SortingDocValuesProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingDocValuesProducer")
            .finish_non_exhaustive()
    }
}

impl DocValuesProducer for SortingDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        let old = self.delegate.get_numeric(field)?;
        Ok(Box::new(SortingNumericDocValues::new(
            self.max_doc,
            &self.doc_map,
            old,
        )?))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
        let old = self.delegate.get_binary(field)?;
        Ok(Box::new(SortingBinaryDocValues::new(
            self.max_doc,
            &self.doc_map,
            old,
        )?))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues>> {
        let old = self.delegate.get_sorted(field)?;
        Ok(Box::new(SortingSortedDocValues::new(
            self.max_doc,
            &self.doc_map,
            old,
        )?))
    }

    fn get_sorted_numeric(&self, field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>> {
        let old = self.delegate.get_sorted_numeric(field)?;
        Ok(Box::new(SortingSortedNumericDocValues::new(
            self.max_doc,
            &self.doc_map,
            old,
        )?))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>> {
        let old = self.delegate.get_sorted_set(field)?;
        Ok(Box::new(SortingSortedSetDocValues::new(
            self.max_doc,
            &self.doc_map,
            old,
        )?))
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>> {
        // Equivalent to `SortingCodecReader.getDocValuesReader().getSkipper`,
        // which returns `null`: min/max-per-block skip metadata is meaningless
        // once documents have been reordered, so no skip index is offered.
        Ok(Box::new(EmptyDocValuesSkipper))
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(SortingDocValuesProducer {
            delegate: Arc::clone(&self.delegate),
            doc_map: Arc::clone(&self.doc_map),
            max_doc: self.max_doc,
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Sorted view of a [`NormsProducer`], reusing the same numeric-values
/// materialisation as [`SortingDocValuesProducer::get_numeric`].
///
/// Equivalent to the anonymous `NormsProducer`
/// `SortingCodecReader.getNormsReader()` builds.
struct SortingNormsProducer {
    delegate: Arc<dyn NormsProducer>,
    doc_map: Arc<SorterDocMap>,
    max_doc: i32,
}

impl std::fmt::Debug for SortingNormsProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingNormsProducer")
            .finish_non_exhaustive()
    }
}

impl NormsProducer for SortingNormsProducer {
    fn get_norms(&self, field_info: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        let old = self.delegate.get_norms(field_info)?;
        Ok(Box::new(SortingNumericDocValues::new(
            self.max_doc,
            &self.doc_map,
            old,
        )?))
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(SortingNormsProducer {
            delegate: Arc::clone(&self.delegate),
            doc_map: Arc::clone(&self.doc_map),
            max_doc: self.max_doc,
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
