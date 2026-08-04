//! KNN-vectors format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.KnnVectorsFormat`,
//! `KnnVectorsReader`, `KnnVectorsWriter`, `KnnFieldVectorsWriter` and
//! `BufferingKnnVectorsWriter`.
//!
//! These traits provide the abstract read/write API for numeric vector fields
//! used for nearest-neighbor search. Concrete codecs implement the
//! format-specific encoding (flat vectors, HNSW graphs, etc.) underneath this
//! API.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

use crate::error::Result;
use crate::search::AcceptDocs;

use super::postings::MergeState;
use super::state::{SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;

// -----------------------------------------------------------------------------
// Supporting types
// -----------------------------------------------------------------------------

/// Placeholder for the result of a KNN search.
///
/// Equivalent to `org.apache.lucene.search.TopDocs`.
#[derive(Debug, Default, Clone)]
pub struct TopDocs;

/// Placeholder for a KNN search strategy.
///
/// Equivalent to `org.apache.lucene.search.knn.KnnSearchStrategy`.
#[derive(Debug, Default, Clone)]
pub struct KnnSearchStrategy;

/// Iterator over float vector values.
///
/// Equivalent to `org.apache.lucene.index.FloatVectorValues`.
pub trait FloatVectorValues: Send + Sync {
    /// Returns the vector dimension.
    fn dimension(&self) -> i32;

    /// Returns the number of vectors.
    fn size(&self) -> i32;

    /// Returns the vector for the given ordinal.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>>;
}

/// Iterator over byte vector values.
///
/// Equivalent to `org.apache.lucene.index.ByteVectorValues`.
pub trait ByteVectorValues: Send + Sync {
    /// Returns the vector dimension.
    fn dimension(&self) -> i32;

    /// Returns the number of vectors.
    fn size(&self) -> i32;

    /// Returns the vector for the given ordinal.
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>>;
}

/// Collector for KNN search results.
///
/// Equivalent to `org.apache.lucene.search.KnnCollector`.
pub trait KnnCollector: Send + Sync {
    /// Returns whether the search terminated early.
    fn early_terminated(&self) -> bool;

    /// Increments the visited vector count.
    fn inc_visited_count(&mut self, count: i32);

    /// Returns the current visited vector count.
    fn visited_count(&self) -> i64;

    /// Returns the visited vector limit.
    fn visit_limit(&self) -> i64;

    /// Returns the expected number of collected results.
    fn k(&self) -> i32;

    /// Collects a document with its similarity score.
    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool;

    /// Returns the current minimum competitive similarity.
    fn min_competitive_similarity(&self) -> f32;

    /// Drains the collected results into a `TopDocs`.
    fn top_docs(&self) -> TopDocs;

    /// Returns the search strategy, if any.
    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy>;
}

// -----------------------------------------------------------------------------
// Field writer
// -----------------------------------------------------------------------------

/// Vectors writer for a single field.
///
/// Equivalent to `org.apache.lucene.codecs.KnnFieldVectorsWriter`.
pub trait KnnFieldVectorsWriter<T>: Send + Sync {
    /// Adds a new doc ID with its vector value to the given field.
    fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()>;

    /// Copies a vector value being indexed to internal storage.
    fn copy_value(&self, vector_value: T) -> T;
}

// -----------------------------------------------------------------------------
// Segment writer
// -----------------------------------------------------------------------------

/// Writes vectors to an index.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsWriter`.
pub trait KnnVectorsWriter: Send + Sync + fmt::Debug {
    /// Adds a new vector field for indexing.
    ///
    /// Note: the no-op stub returns a float-vector field writer. Real
    /// implementations dispatch on the field's `VectorEncoding`.
    fn add_field(
        &mut self,
        field_info: &FieldInfo,
    ) -> Result<Box<dyn KnnFieldVectorsWriter<Vec<f32>>>>;

    /// Flushes all buffered data to disk.
    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()>;

    /// Called once at the end before close.
    fn finish(&mut self) -> Result<()>;

    /// Merges vectors for a single field, returning deferred work if any.
    ///
    /// The default implementation is a no-op.
    fn merge_one_field(
        &mut self,
        _field_info: &FieldInfo,
        _merge_state: &MergeState,
    ) -> Result<Option<Box<dyn IORunnable>>> {
        Ok(None)
    }

    /// Merges the segment vectors from the readers in `merge_state`.
    ///
    /// The default implementation is a no-op; concrete formats override it with
    /// format-specific merge logic.
    fn merge(&mut self, _merge_state: &MergeState) -> Result<()> {
        Ok(())
    }

    /// Closes this writer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

/// Document-id mapping produced by index sorting.
///
/// Equivalent to `org.apache.lucene.index.Sorter.DocMap`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SorterDocMap;

/// Runnable I/O operation returned by deferred merge steps.
///
/// Equivalent to `org.apache.lucene.util.IORunnable`.
pub trait IORunnable: Send + Sync {
    /// Executes the deferred I/O work.
    fn run(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Buffering writer
// -----------------------------------------------------------------------------

/// Buffers pending vector values per doc and flushes when the segment flushes.
///
/// Equivalent to `org.apache.lucene.codecs.BufferingKnnVectorsWriter`.
///
/// Concrete implementations provide `write_field_float` and `write_field_byte`.
pub trait BufferingKnnVectorsWriter: KnnVectorsWriter {
    /// Writes the provided float vector field.
    fn write_field_float(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn FloatVectorValues,
        max_doc: i32,
    ) -> Result<()>;

    /// Writes the provided byte vector field.
    fn write_field_byte(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn ByteVectorValues,
        max_doc: i32,
    ) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Reader
// -----------------------------------------------------------------------------

/// Reads vectors from an index.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsReader`.
pub trait KnnVectorsReader: Send + Sync + fmt::Debug {
    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Returns the float vector values for the given field.
    fn get_float_vector_values(&self, field: &str) -> Result<Box<dyn FloatVectorValues>>;

    /// Returns the byte vector values for the given field.
    fn get_byte_vector_values(&self, field: &str) -> Result<Box<dyn ByteVectorValues>>;

    /// Searches the float vector field for the k nearest neighbors to `target`.
    fn search(
        &mut self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()>;

    /// Searches the byte vector field for the k nearest neighbors to `target`.
    fn search_byte(
        &mut self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()>;

    /// Returns an instance optimized for merging.
    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>>;

    /// Optional: resets or closes merge resources used in the reader.
    fn finish_merge(&mut self) -> Result<()> {
        Ok(())
    }

    /// Returns the desired off-heap memory size for the given field.
    fn get_off_heap_byte_size(&self, _field_info: &FieldInfo) -> HashMap<String, i64> {
        HashMap::new()
    }

    /// Closes this reader, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Format
// -----------------------------------------------------------------------------

/// Encodes and decodes numeric vector fields used for KNN search.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsFormat`.
pub trait KnnVectorsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a writer to write vectors to the index.
    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>>;

    /// Returns a reader to read vectors from the index.
    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn KnnVectorsReader>>;

    /// Returns the maximum number of vector dimensions supported for the given
    /// field name.
    fn get_max_dimensions(&self, _field_name: &str) -> i32;
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A no-op float vector values iterator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyFloatVectorValues;

impl FloatVectorValues for EmptyFloatVectorValues {
    fn dimension(&self) -> i32 {
        0
    }

    fn size(&self) -> i32 {
        0
    }

    fn vector_value(&self, _ord: i32) -> Result<Vec<f32>> {
        Ok(Vec::new())
    }
}

/// A no-op byte vector values iterator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyByteVectorValues;

impl ByteVectorValues for EmptyByteVectorValues {
    fn dimension(&self) -> i32 {
        0
    }

    fn size(&self) -> i32 {
        0
    }

    fn vector_value(&self, _ord: i32) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

/// A no-op KNN collector.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnCollector;

impl KnnCollector for EmptyKnnCollector {
    fn early_terminated(&self) -> bool {
        false
    }

    fn inc_visited_count(&mut self, _count: i32) {}

    fn visited_count(&self) -> i64 {
        0
    }

    fn visit_limit(&self) -> i64 {
        0
    }

    fn k(&self) -> i32 {
        0
    }

    fn collect(&mut self, _doc_id: i32, _similarity: f32) -> bool {
        false
    }

    fn min_competitive_similarity(&self) -> f32 {
        f32::NEG_INFINITY
    }

    fn top_docs(&self) -> TopDocs {
        TopDocs
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        None
    }
}

/// A no-op field vectors writer for float vectors.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnFieldVectorsWriter;

impl KnnFieldVectorsWriter<Vec<f32>> for EmptyKnnFieldVectorsWriter {
    fn add_value(&mut self, _doc_id: i32, _vector_value: Vec<f32>) -> Result<()> {
        Ok(())
    }

    fn copy_value(&self, vector_value: Vec<f32>) -> Vec<f32> {
        vector_value
    }
}

/// A no-op KNN vectors writer.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnVectorsWriter;

impl KnnVectorsWriter for EmptyKnnVectorsWriter {
    fn add_field(
        &mut self,
        _field_info: &FieldInfo,
    ) -> Result<Box<dyn KnnFieldVectorsWriter<Vec<f32>>>> {
        Ok(Box::new(EmptyKnnFieldVectorsWriter))
    }

    fn flush(&mut self, _max_doc: i32, _sort_map: Option<&SorterDocMap>) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op buffering KNN vectors writer.
#[derive(Debug, Default, Clone)]
pub struct EmptyBufferingKnnVectorsWriter;

impl KnnVectorsWriter for EmptyBufferingKnnVectorsWriter {
    fn add_field(
        &mut self,
        _field_info: &FieldInfo,
    ) -> Result<Box<dyn KnnFieldVectorsWriter<Vec<f32>>>> {
        Ok(Box::new(EmptyKnnFieldVectorsWriter))
    }

    fn flush(&mut self, _max_doc: i32, _sort_map: Option<&SorterDocMap>) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl BufferingKnnVectorsWriter for EmptyBufferingKnnVectorsWriter {
    fn write_field_float(
        &mut self,
        _field_info: &FieldInfo,
        _values: &dyn FloatVectorValues,
        _max_doc: i32,
    ) -> Result<()> {
        Ok(())
    }

    fn write_field_byte(
        &mut self,
        _field_info: &FieldInfo,
        _values: &dyn ByteVectorValues,
        _max_doc: i32,
    ) -> Result<()> {
        Ok(())
    }
}

/// A no-op KNN vectors reader.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnVectorsReader;

impl KnnVectorsReader for EmptyKnnVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_float_vector_values(&self, _field: &str) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(EmptyFloatVectorValues))
    }

    fn get_byte_vector_values(&self, _field: &str) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(EmptyByteVectorValues))
    }

    fn search(
        &mut self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_byte(
        &mut self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op KNN vectors format.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnVectorsFormat {
    name: String,
}

impl EmptyKnnVectorsFormat {
    /// Creates a new no-op KNN vectors format with the given SPI name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl KnnVectorsFormat for EmptyKnnVectorsFormat {
    fn name(&self) -> &str {
        &self.name
    }

    fn fields_writer(&self, _state: &SegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        Ok(Box::new(EmptyKnnVectorsWriter))
    }

    fn fields_reader(&self, _state: &SegmentReadState) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(EmptyKnnVectorsReader))
    }

    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        1024
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::{BufferedUpdates, FieldInfos, SegmentInfo};
    use crate::search::from_live_docs;

    #[test]
    fn empty_float_vector_values_is_empty() {
        let values = EmptyFloatVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        assert!(values.vector_value(0).unwrap().is_empty());
    }

    #[test]
    fn empty_byte_vector_values_is_empty() {
        let values = EmptyByteVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        assert!(values.vector_value(0).unwrap().is_empty());
    }

    #[test]
    fn empty_knn_collector_is_empty() {
        let mut collector = EmptyKnnCollector;
        assert!(!collector.early_terminated());
        assert_eq!(collector.visited_count(), 0);
        assert_eq!(collector.visit_limit(), 0);
        assert_eq!(collector.k(), 0);
        assert!(!collector.collect(0, 1.0));
        assert_eq!(collector.min_competitive_similarity(), f32::NEG_INFINITY);
        assert!(collector.get_search_strategy().is_none());
    }

    #[test]
    fn empty_knn_field_writer_accepts_vectors() {
        let mut writer = EmptyKnnFieldVectorsWriter;
        writer.add_value(0, vec![1.0, 2.0, 3.0]).unwrap();
        assert_eq!(writer.copy_value(vec![4.0]), vec![4.0]);
    }

    #[test]
    fn empty_knn_vectors_writer_lifecycle() {
        let mut writer = EmptyKnnVectorsWriter;
        let field = FieldInfo;
        let _field_writer = writer.add_field(&field).unwrap();
        writer.flush(10, None).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn empty_buffering_knn_vectors_writer_lifecycle() {
        let mut writer = EmptyBufferingKnnVectorsWriter;
        let field = FieldInfo;
        writer
            .write_field_float(&field, &EmptyFloatVectorValues, 10)
            .unwrap();
        writer
            .write_field_byte(&field, &EmptyByteVectorValues, 10)
            .unwrap();
        writer.flush(10, None).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn empty_knn_vectors_reader_lifecycle() {
        let mut reader = EmptyKnnVectorsReader;
        assert_eq!(reader.get_float_vector_values("f").unwrap().size(), 0);
        assert_eq!(reader.get_byte_vector_values("f").unwrap().size(), 0);
        let mut collector = EmptyKnnCollector;
        let mut accept_docs = from_live_docs(None, 10).unwrap();
        reader
            .search("f", &[1.0, 2.0], &mut collector, &mut accept_docs)
            .unwrap();
        reader
            .search_byte("f", &[1, 2], &mut collector, &mut accept_docs)
            .unwrap();
        reader.check_integrity().unwrap();
        let _merge = reader.get_merge_instance().unwrap();
        reader.finish_merge().unwrap();
        reader.close().unwrap();
    }

    #[test]
    fn empty_knn_vectors_format_name_and_factories() {
        let format = EmptyKnnVectorsFormat::new("EmptyKnn");
        assert_eq!(format.name(), "EmptyKnn");
        assert_eq!(format.get_max_dimensions("field"), 1024);

        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &SegmentInfo,
            &FieldInfos,
            &BufferedUpdates,
            context,
        );
        let _writer = format.fields_writer(&write_state).unwrap();

        let read_state = SegmentReadState::new(dir_ref, &SegmentInfo, &FieldInfos, context);
        let _reader = format.fields_reader(&read_state).unwrap();
    }
}
