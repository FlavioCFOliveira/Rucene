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
use std::sync::{Arc, LazyLock, RwLock};

use crate::error::{LuceneError, Result};
pub use crate::index::vector_values::{
    accept_ords, from_bytes, from_floats, ByteVectorValues, DenseDocIndexIterator,
    DocIndexIterator, EmptyByteVectorValues, EmptyFloatVectorValues, EmptyKnnVectorValues,
    FloatVectorValues, FromDisiDocIndexIterator, KnnVectorValues, SparseDocIndexIterator,
};
pub use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::AcceptDocs;

use super::postings::MergeState;
use super::state::{SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;

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

/// Per-field writer returned by [`KnnVectorsWriter::add_field`].
///
/// Concrete writers return the variant matching the field's
/// [`VectorEncoding`](crate::index::VectorEncoding).
pub enum FieldVectorWriter {
    /// Float-vector field writer.
    Float(Box<dyn KnnFieldVectorsWriter<Vec<f32>>>),
    /// Byte-vector field writer.
    Byte(Box<dyn KnnFieldVectorsWriter<Vec<u8>>>),
}

/// Writes vectors to an index.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsWriter`.
pub trait KnnVectorsWriter: Send + Sync + fmt::Debug {
    /// Adds a new vector field for indexing.
    ///
    /// The returned variant matches the field's `VectorEncoding`.
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter>;

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
    fn fields_writer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn KnnVectorsWriter + 'a>>;

    /// Returns a reader to read vectors from the index.
    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>>;

    /// Returns the maximum number of vector dimensions supported for the given
    /// field name.
    fn get_max_dimensions(&self, _field_name: &str) -> i32;
}

// -----------------------------------------------------------------------------
// KNN-vectors format registry
// -----------------------------------------------------------------------------

/// A registry mapping KNN-vectors-format short names to [`KnnVectorsFormat`]
/// implementations.
///
/// The registry intentionally does not use reflection or SPI loading. Formats
/// are registered explicitly with [`KnnVectorsFormatRegistry::register`] and
/// looked up by name with [`KnnVectorsFormatRegistry::for_name`].
#[derive(Debug, Default, Clone)]
pub struct KnnVectorsFormatRegistry {
    formats: Arc<RwLock<HashMap<String, Arc<dyn KnnVectorsFormat>>>>,
}

impl KnnVectorsFormatRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a KNN-vectors format under the given short name.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `name` is empty, contains
    /// characters other than ASCII alphanumerics, or is longer than 127 bytes.
    /// Returns [`LuceneError::IllegalState`] if the name is already registered.
    pub fn register<F>(&self, name: impl Into<String>, format: F) -> Result<()>
    where
        F: KnnVectorsFormat + 'static,
    {
        let name = name.into();
        super::validate_service_name(&name)?;

        let mut formats = self.formats.write().map_err(|_| {
            LuceneError::IllegalState("KNN-vectors format registry lock was poisoned".to_string())
        })?;

        if formats.contains_key(&name) {
            return Err(LuceneError::IllegalState(format!(
                "KNN-vectors format already registered: {name}"
            )));
        }

        formats.insert(name, Arc::new(format));
        Ok(())
    }

    /// Looks up a KNN-vectors format by name.
    ///
    /// Returns `None` if no format has been registered under the given name.
    pub fn for_name(&self, name: &str) -> Option<Arc<dyn KnnVectorsFormat>> {
        self.formats
            .read()
            .map_err(|_| {
                LuceneError::IllegalState(
                    "KNN-vectors format registry lock was poisoned".to_string(),
                )
            })
            .ok()?
            .get(name)
            .cloned()
    }

    /// Returns the names of all registered KNN-vectors formats, sorted
    /// alphabetically.
    pub fn available_knn_vectors_formats(&self) -> Vec<String> {
        let Ok(formats) = self.formats.read() else {
            return Vec::new();
        };
        let mut names: Vec<String> = formats.keys().cloned().collect();
        names.sort();
        names
    }
}

static GLOBAL_KNN_VECTORS_REGISTRY: LazyLock<KnnVectorsFormatRegistry> =
    LazyLock::new(KnnVectorsFormatRegistry::new);

/// Looks up a KNN-vectors format by name from the global registry.
///
/// Returns `None` if no format has been registered under the given name.
pub fn knn_vectors_for_name(name: &str) -> Option<Arc<dyn KnnVectorsFormat>> {
    GLOBAL_KNN_VECTORS_REGISTRY.for_name(name)
}

/// Returns the names of all KNN-vectors formats registered in the global
/// registry.
pub fn available_knn_vectors_formats() -> Vec<String> {
    GLOBAL_KNN_VECTORS_REGISTRY.available_knn_vectors_formats()
}

/// Registers a KNN-vectors format in the global registry.
///
/// If the format has already been registered, this is a no-op so that formats
/// can safely call it from their constructors.
pub fn register_global_knn_vectors_format<F>(name: &str, format: F) -> Result<()>
where
    F: KnnVectorsFormat + 'static,
{
    if GLOBAL_KNN_VECTORS_REGISTRY.for_name(name).is_some() {
        return Ok(());
    }
    GLOBAL_KNN_VECTORS_REGISTRY.register(name, format)
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

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
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        Ok(match field_info.vector_encoding {
            crate::index::VectorEncoding::FLOAT32 => {
                FieldVectorWriter::Float(Box::new(EmptyKnnFieldVectorsWriter))
            }
            crate::index::VectorEncoding::BYTE => {
                FieldVectorWriter::Byte(Box::new(EmptyByteKnnFieldVectorsWriter))
            }
        })
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

/// A no-op byte-vector field writer.
#[derive(Debug, Default, Clone)]
pub struct EmptyByteKnnFieldVectorsWriter;

impl KnnFieldVectorsWriter<Vec<u8>> for EmptyByteKnnFieldVectorsWriter {
    fn add_value(&mut self, _doc_id: i32, _vector_value: Vec<u8>) -> Result<()> {
        Ok(())
    }

    fn copy_value(&self, vector_value: Vec<u8>) -> Vec<u8> {
        vector_value
    }
}

/// A no-op buffering KNN vectors writer.
#[derive(Debug, Default, Clone)]
pub struct EmptyBufferingKnnVectorsWriter;

impl KnnVectorsWriter for EmptyBufferingKnnVectorsWriter {
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        Ok(match field_info.vector_encoding {
            crate::index::VectorEncoding::FLOAT32 => {
                FieldVectorWriter::Float(Box::new(EmptyKnnFieldVectorsWriter))
            }
            crate::index::VectorEncoding::BYTE => {
                FieldVectorWriter::Byte(Box::new(EmptyByteKnnFieldVectorsWriter))
            }
        })
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

    fn fields_writer<'a>(
        &self,
        _state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn KnnVectorsWriter + 'a>> {
        Ok(Box::new(EmptyKnnVectorsWriter))
    }

    fn fields_reader<'a>(
        &self,
        _state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn KnnVectorsReader>> {
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
    use crate::codecs::stub::{BufferedUpdates, FieldInfos};
    use crate::search::from_live_docs;

    #[test]
    fn empty_float_vector_values_is_empty() {
        let values = EmptyFloatVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        // Java documents `vectorValue(ord)` as throwing IndexOutOfBoundsException
        // outside [0, size()); with size() == 0 every ordinal is out of range.
        assert!(values.vector_value(0).is_err());
    }

    #[test]
    fn empty_byte_vector_values_is_empty() {
        let values = EmptyByteVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        assert!(values.vector_value(0).is_err());
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
        let field = FieldInfo::default();
        let _field_writer = writer.add_field(&field).unwrap();
        writer.flush(10, None).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn empty_buffering_knn_vectors_writer_lifecycle() {
        let mut writer = EmptyBufferingKnnVectorsWriter;
        let field = FieldInfo::default();
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
        let field_infos = FieldInfos::default();
        let segment_info = crate::codecs::tests::test_segment_info("test", 10);
        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &BufferedUpdates,
            context,
        );
        let _writer = format.fields_writer(&write_state).unwrap();

        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);
        let _reader = format.fields_reader(&read_state).unwrap();
    }
}
