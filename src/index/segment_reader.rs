//! Minimal `SegmentReader` placeholder for use by `DirectoryReader`.
//!
//! This is **not** the full port of `org.apache.lucene.index.SegmentReader`
//! (that is task 93). It exists only so that `DirectoryReader` can own leaf
//! readers, build reader contexts, and match segments by name during
//! `openIfChanged`. All data-access methods return empty / `None` results.
//!
//! TODO(task 93): replace this stub with a real `CodecReader`-based segment
//! reader that opens postings, doc values, stored fields, term vectors, points
//! and vectors from the segment files.

#![deny(unsafe_code)]

use std::fmt::{Debug, Formatter};

use crate::error::{LuceneError, Result};
use crate::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
use crate::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipper, FieldInfos, FloatVectorValues,
    NumericDocValues, PointValues, SegmentCommitInfo, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, Terms,
};
use crate::search::knn::KnnCollector;
use crate::search::AcceptDocs;
use crate::util::Bits;

/// Placeholder segment reader used by `DirectoryReader` until task 93.
///
/// Equivalent to a stripped-down `org.apache.lucene.index.SegmentReader` that
/// only exposes segment identity and empty leaf-reader APIs.
pub struct SegmentReader {
    core: IndexReaderCore,
    /// A clone of the `SegmentCommitInfo` this reader was opened on.
    segment_info: SegmentCommitInfo,
    num_docs: i32,
    max_doc: i32,
    meta: LeafMetaData,
}

impl Debug for SegmentReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentReader")
            .field("segment", &self.segment_info)
            .field("max_doc", &self.max_doc)
            .field("num_docs", &self.num_docs)
            .finish()
    }
}

impl SegmentReader {
    /// Creates a minimal segment reader from the given commit info.
    ///
    /// TODO(task 93): accept an `IOContext`, open `SegmentCoreReaders`, and load
    /// live docs from disk.
    pub fn new(segment_info: SegmentCommitInfo, created_version_major: i32) -> Result<Self> {
        let max_doc = segment_info.info.max_doc()?;
        let del_count = segment_info.get_del_count();
        let num_docs = max_doc - del_count;
        let meta = LeafMetaData::new(
            created_version_major,
            segment_info.info.min_version(),
            None,
            segment_info.info.get_has_blocks(),
        )?;
        Ok(Self {
            core: IndexReaderCore::new(),
            segment_info,
            num_docs,
            max_doc,
            meta,
        })
    }

    /// Returns the name of the segment this reader is reading.
    pub fn get_segment_name(&self) -> &str {
        &self.segment_info.info.name
    }

    /// Returns the `SegmentCommitInfo` of the segment this reader is reading.
    pub fn get_segment_info(&self) -> &SegmentCommitInfo {
        &self.segment_info
    }
}

impl LeafReader for SegmentReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Ok(Box::new(crate::index::leaf_reader::EmptyTermVectors))
    }

    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        // TODO(task 93): return a real StoredFields reader.
        Err(LuceneError::UnsupportedOperation(
            "SegmentReader.stored_fields is not implemented in the task 91 placeholder".to_string(),
        ))
    }

    fn do_close(&self) -> Result<()> {
        // TODO(task 93): close SegmentCoreReaders and doc-values producer.
        Ok(())
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        // TODO(task 93): return a real cache helper.
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        // TODO(task 93): return a real core cache helper.
        None
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(None)
    }

    fn get_numeric_doc_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None)
    }

    fn get_binary_doc_values(&self, _field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        Ok(None)
    }

    fn get_sorted_doc_values(&self, _field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        Ok(None)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        Ok(None)
    }

    fn get_sorted_set_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        Ok(None)
    }

    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None)
    }

    fn get_doc_values_skipper(&self, _field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Ok(None)
    }

    fn get_float_vector_values(&self, _field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        Ok(None)
    }

    fn get_byte_vector_values(&self, _field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        Ok(None)
    }

    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn get_field_infos(&self) -> FieldInfos {
        FieldInfos::empty()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        // TODO(task 93): return real live-docs bits when deletions exist.
        None
    }

    fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn PointValues>>> {
        Ok(None)
    }

    fn check_integrity(&self) -> Result<()> {
        // TODO(task 93): verify each codec reader.
        Ok(())
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.meta.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::tests::{test_segment_commit_info, test_segment_info};
    use crate::util::Version;

    #[test]
    fn segment_reader_exposes_identity() {
        let info = test_segment_commit_info("_0", 7);
        let reader = SegmentReader::new(info.clone(), Version::LATEST.major as i32).unwrap();
        assert_eq!(reader.get_segment_name(), "_0");
        assert_eq!(reader.max_doc(), 7);
        assert_eq!(reader.num_docs(), 7);
        assert_eq!(reader.get_segment_info(), &info);
    }

    #[test]
    fn segment_reader_accounts_for_deletions() {
        let info = test_segment_info("_1", 10);
        let mut sci = SegmentCommitInfo::new(info, 3, 0, -1, -1, -1, [0u8; 16]).unwrap();
        // Force del_gen to reflect deletions so numDocs = maxDoc - delCount.
        sci.advance_del_gen();
        let reader = SegmentReader::new(sci, Version::LATEST.major as i32).unwrap();
        assert_eq!(reader.max_doc(), 10);
        assert_eq!(reader.num_docs(), 7);
    }
}
