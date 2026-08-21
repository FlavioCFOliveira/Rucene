//! Atomic leaf reader abstractions ported from `org.apache.lucene.index`.
//!
//! This module provides [`LeafReader`], [`LeafMetaData`], and the index-level
//! [`TermVectors`] API used by both leaf and composite readers.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::{Arc, Weak};

use crate::error::{LuceneError, Result};
use crate::index::index_reader::{CacheHelper, IndexReader, IndexReaderCore, StoredFields};
use crate::index::reader_context::{IndexReaderContext, LeafReaderContext};
use crate::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipper, FieldInfos, FloatVectorValues,
    NumericDocValues, PointValues, PostingsEnum, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, Term, Terms,
};
use crate::search::knn::{KnnCollector, TopDocs, TopKnnCollector};
use crate::search::AcceptDocs;
use crate::search::Sort;
use crate::util::extra::Version;
use crate::util::Bits;

// ---------------------------------------------------------------------------
// LeafMetaData
// ---------------------------------------------------------------------------

/// Read-only metadata about a leaf segment.
///
/// Equivalent to `org.apache.lucene.index.LeafMetaData`.
#[derive(Debug, Clone)]
pub struct LeafMetaData {
    created_version_major: i32,
    min_version: Option<Version>,
    sort: Option<Sort>,
    has_blocks: bool,
}

impl LeafMetaData {
    /// Minimum valid `created_version_major`.
    pub const MIN_CREATED_VERSION_MAJOR: i32 = 6;

    /// Creates a new `LeafMetaData`, validating invariants.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `created_version_major` is out
    /// of range or if `min_version` is required but missing.
    pub fn new(
        created_version_major: i32,
        min_version: Option<Version>,
        sort: Option<Sort>,
        has_blocks: bool,
    ) -> Result<Self> {
        if created_version_major > Version::LATEST.major as i32 {
            return Err(LuceneError::IllegalArgument(format!(
                "createdVersionMajor is in the future: {created_version_major}"
            )));
        }
        if created_version_major < Self::MIN_CREATED_VERSION_MAJOR {
            return Err(LuceneError::IllegalArgument(format!(
                "createdVersionMajor must be >= {}, got: {created_version_major}",
                Self::MIN_CREATED_VERSION_MAJOR
            )));
        }
        if created_version_major >= 7 && min_version.is_none() {
            return Err(LuceneError::IllegalArgument(
                "minVersion must be set when createdVersionMajor is >= 7".to_string(),
            ));
        }
        Ok(Self {
            created_version_major,
            min_version,
            sort,
            has_blocks,
        })
    }

    /// The Lucene major version that created this index.
    pub fn created_version_major(&self) -> i32 {
        self.created_version_major
    }

    /// The minimum Lucene version that contributed documents, if known.
    pub fn min_version(&self) -> Option<Version> {
        self.min_version
    }

    /// The index sort, if any.
    pub fn sort(&self) -> Option<&Sort> {
        self.sort.as_ref()
    }

    /// Returns `true` if this leaf contains document blocks.
    pub fn has_blocks(&self) -> bool {
        self.has_blocks
    }
}

// ---------------------------------------------------------------------------
// TermVectors
// ---------------------------------------------------------------------------

/// API for reading term vectors.
///
/// Equivalent to `org.apache.lucene.index.TermVectors`. Instances are not
/// thread-safe and should be used by a single thread.
pub trait TermVectors: Send + Sync + Debug {
    /// Optional prefetch hint for the given document.
    fn prefetch(&mut self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    /// Returns term vectors for `doc`, or `None` if none were indexed.
    fn get(&self, doc: i32) -> Result<Option<Box<dyn crate::index::Fields>>>;

    /// Returns term vectors for `doc` restricted to `field`, or `None` if none
    /// were indexed for that field.
    fn get_field(&self, doc: i32, field: &str) -> Result<Option<Box<dyn crate::index::Terms>>> {
        if let Some(fields) = self.get(doc)? {
            if let Some(terms) = fields.terms(field)? {
                return Ok(Some(terms));
            }
        }
        Ok(None)
    }
}

/// TermVectors implementation that never returns vectors.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyTermVectors;

impl TermVectors for EmptyTermVectors {
    fn get(&self, _doc: i32) -> Result<Option<Box<dyn crate::index::Fields>>> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// LeafReader
// ---------------------------------------------------------------------------

/// Atomic reader providing direct access to terms, postings, doc values,
/// stored fields, term vectors, points, and vectors.
///
/// Equivalent to `org.apache.lucene.index.LeafReader`. Concrete leaf readers
/// implement this trait; a blanket implementation makes them usable as
/// `dyn IndexReader`.
pub trait LeafReader: Send + Sync + Debug + 'static {
    /// Returns the shared core state for this reader.
    fn core(&self) -> &IndexReaderCore;

    /// Returns a [`TermVectors`] reader for this leaf.
    fn term_vectors(&self) -> Result<Box<dyn TermVectors>>;

    /// Returns the number of live documents.
    fn num_docs(&self) -> i32;

    /// Returns one greater than the largest possible document number.
    fn max_doc(&self) -> i32;

    /// Returns a [`StoredFields`] reader for this leaf.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>>;

    /// Implements close logic for this leaf.
    fn do_close(&self) -> Result<()>;

    /// Returns the reader-level cache helper, if any.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>>;

    /// Returns the leaf-level core cache helper, if any.
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>>;

    /// Returns the [`Terms`] index for `field`, or `None` if it has none.
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>>;

    /// Returns [`NumericDocValues`] for `field`, if any.
    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>>;

    /// Returns [`BinaryDocValues`] for `field`, if any.
    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>>;

    /// Returns [`SortedDocValues`] for `field`, if any.
    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>>;

    /// Returns [`SortedNumericDocValues`] for `field`, if any.
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>>;

    /// Returns [`SortedSetDocValues`] for `field`, if any.
    fn get_sorted_set_doc_values(&self, field: &str)
        -> Result<Option<Box<dyn SortedSetDocValues>>>;

    /// Returns norm values for `field`, if any.
    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>>;

    /// Returns a [`DocValuesSkipper`] for `field`, if any.
    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>>;

    /// Returns [`FloatVectorValues`] for `field`, if any.
    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>>;

    /// Returns [`ByteVectorValues`] for `field`, if any.
    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>>;

    /// Performs an approximate KNN search for `target` against the float vectors
    /// of `field`.
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()>;

    /// Performs an approximate KNN search for `target` against the byte vectors
    /// of `field`.
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()>;

    /// Returns the [`FieldInfos`] describing all fields in this leaf.
    fn get_field_infos(&self) -> FieldInfos;

    /// Returns the live-docs bits, or `None` if there are no deletions.
    fn get_live_docs(&self) -> Option<Box<dyn Bits>>;

    /// Returns the [`PointValues`] for `field`, if any.
    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>>;

    /// Checks the internal consistency of this leaf.
    fn check_integrity(&self) -> Result<()>;

    /// Returns metadata about this leaf.
    fn get_meta_data(&self) -> LeafMetaData;

    /// Returns a [`PostingsEnum`] for the given term and flags.
    ///
    /// Default implementation follows `LeafReader.postings(Term, int)`.
    fn postings(&self, term: &Term, flags: i32) -> Result<Option<Box<dyn PostingsEnum>>> {
        if let Some(terms) = self.terms(term.field())? {
            let mut iterator = terms.iterator()?;
            if iterator.seek_exact(term.bytes())? {
                return Ok(Some(iterator.postings(None, flags)?));
            }
        }
        Ok(None)
    }

    /// Returns a postings enum with [`POSTINGS_ENUM_FREQS`].
    fn postings_default(&self, term: &Term) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.postings(term, crate::index::POSTINGS_ENUM_FREQS)
    }

    /// Convenience KNN search that collects the top `k` float-vector matches.
    ///
    /// Equivalent to `LeafReader.searchNearestVectors(String, float[], int,
    /// AcceptDocs, int)`.
    fn search_nearest_vectors_default(
        &self,
        field: &str,
        target: &[f32],
        k: i32,
        accept_docs: &mut dyn AcceptDocs,
        visited_limit: i32,
    ) -> Result<TopDocs> {
        let infos = self.get_field_infos();
        let field_info = infos.field_info(field);
        if field_info.is_none() || field_info.unwrap().vector_dimension == 0 {
            return Ok(TopDocs);
        }
        let values = self.get_float_vector_values(field)?;
        if values.is_none() {
            return Ok(TopDocs);
        }
        let values = values.unwrap();
        let k = k.min(values.size());
        if k == 0 {
            return Ok(TopDocs);
        }
        let mut collector = TopKnnCollector::new(k, visited_limit as i64);
        self.search_nearest_vectors(field, target, &mut collector, accept_docs)?;
        Ok(collector.top_docs())
    }

    /// Convenience KNN search that collects the top `k` byte-vector matches.
    fn search_nearest_vectors_byte_default(
        &self,
        field: &str,
        target: &[u8],
        k: i32,
        accept_docs: &mut dyn AcceptDocs,
        visited_limit: i32,
    ) -> Result<TopDocs> {
        let infos = self.get_field_infos();
        let field_info = infos.field_info(field);
        if field_info.is_none() || field_info.unwrap().vector_dimension == 0 {
            return Ok(TopDocs);
        }
        let values = self.get_byte_vector_values(field)?;
        if values.is_none() {
            return Ok(TopDocs);
        }
        let values = values.unwrap();
        let k = k.min(values.size());
        if k == 0 {
            return Ok(TopDocs);
        }
        let mut collector = TopKnnCollector::new(k, visited_limit as i64);
        self.search_nearest_vectors_byte(field, target, &mut collector, accept_docs)?;
        Ok(collector.top_docs())
    }
}

impl<T: LeafReader> IndexReader for T {
    fn core(&self) -> &IndexReaderCore {
        LeafReader::core(self)
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        LeafReader::term_vectors(self)
    }

    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(self)
    }

    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(self)
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        LeafReader::stored_fields(self)
    }

    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(self)
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        LeafReader::get_reader_cache_helper(self)
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        if let Some(terms) = self.terms(term.field())? {
            let mut iterator = terms.iterator()?;
            if iterator.seek_exact(term.bytes())? {
                return iterator.doc_freq();
            }
        }
        Ok(0)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        if let Some(terms) = self.terms(term.field())? {
            let mut iterator = terms.iterator()?;
            if iterator.seek_exact(term.bytes())? {
                return iterator.total_term_freq();
            }
        }
        Ok(0)
    }

    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        if let Some(terms) = self.terms(field)? {
            Ok(terms.sum_doc_freq())
        } else {
            Ok(0)
        }
    }

    fn get_doc_count(&self, field: &str) -> Result<i32> {
        if let Some(terms) = self.terms(field)? {
            Ok(terms.doc_count())
        } else {
            Ok(0)
        }
    }

    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        if let Some(terms) = self.terms(field)? {
            Ok(terms.sum_total_term_freq())
        } else {
            Ok(0)
        }
    }

    fn build_context(
        self: Arc<Self>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext> {
        let leaf_reader: Arc<dyn LeafReader> = Arc::clone(&self) as Arc<dyn LeafReader>;
        let reader: Arc<dyn IndexReader> = self;
        Arc::new(LeafReaderContext::new(
            reader,
            leaf_reader,
            parent,
            ord_in_parent,
            doc_base_in_parent,
            leaf_ord,
            leaf_doc_base,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::index_reader::IndexReader;
    use crate::index::postings_enum::POSTINGS_ENUM_FREQS;
    use crate::index::EmptyFields;
    use crate::search::AcceptDocs;
    use crate::util::extra::Version;

    #[derive(Debug)]
    struct DummyTermVectors;
    impl TermVectors for DummyTermVectors {
        fn get(&self, _doc: i32) -> Result<Option<Box<dyn crate::index::Fields>>> {
            Ok(Some(Box::new(EmptyFields)))
        }
    }

    #[derive(Debug)]
    struct DummyStoredFields;
    impl StoredFields for DummyStoredFields {
        fn document_with_visitor(
            &self,
            _doc_id: i32,
            _visitor: &mut dyn crate::codecs::stub::StoredFieldVisitor,
        ) -> Result<()> {
            Ok(())
        }

        fn document(&self, _doc_id: i32) -> Result<crate::document::Document> {
            Ok(crate::document::Document::new())
        }

        fn document_fields(
            &self,
            _doc_id: i32,
            _fields_to_load: &std::collections::HashSet<String>,
        ) -> Result<crate::document::Document> {
            Ok(crate::document::Document::new())
        }
    }

    #[derive(Debug)]
    struct DummyLeafReader {
        core: IndexReaderCore,
        max_doc: i32,
        num_docs: i32,
        meta: LeafMetaData,
    }

    impl DummyLeafReader {
        fn new(max_doc: i32, num_docs: i32, meta: LeafMetaData) -> Self {
            Self {
                core: IndexReaderCore::new(),
                max_doc,
                num_docs,
                meta,
            }
        }
    }

    impl LeafReader for DummyLeafReader {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }

        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(DummyTermVectors))
        }

        fn num_docs(&self) -> i32 {
            self.num_docs
        }

        fn max_doc(&self) -> i32 {
            self.max_doc
        }

        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Ok(Box::new(DummyStoredFields))
        }

        fn do_close(&self) -> Result<()> {
            Ok(())
        }

        fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }

        fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn get_numeric_doc_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn NumericDocValues>>> {
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

        fn get_doc_values_skipper(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }

        fn get_float_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn FloatVectorValues>>> {
            Ok(None)
        }

        fn get_byte_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn ByteVectorValues>>> {
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
            None
        }

        fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(None)
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_meta_data(&self) -> LeafMetaData {
            self.meta.clone()
        }
    }

    #[test]
    fn leaf_meta_data_validates_version() {
        let sort = Sort::new_fields(vec![crate::search::SortField::FIELD_DOC.clone()]).unwrap();
        let meta = LeafMetaData::new(
            Version::LATEST.major as i32,
            Some(Version::LATEST),
            Some(sort),
            false,
        )
        .unwrap();
        assert_eq!(meta.created_version_major(), Version::LATEST.major as i32);
        assert!(meta.min_version().is_some());
        assert!(meta.sort().is_some());
        assert!(!meta.has_blocks());
    }

    #[test]
    fn leaf_meta_data_rejects_future_version() {
        assert!(LeafMetaData::new(99, Some(Version::LATEST), None, false).is_err());
    }

    #[test]
    fn leaf_meta_data_requires_min_version_for_v7_plus() {
        assert!(LeafMetaData::new(7, None, None, false).is_err());
        assert!(LeafMetaData::new(6, None, None, false).is_ok());
    }

    #[test]
    fn leaf_reader_index_reader_defaults_work() {
        let reader = Arc::new(DummyLeafReader::new(
            10,
            9,
            LeafMetaData::new(10, Some(Version::LATEST), None, false).unwrap(),
        ));
        assert_eq!(IndexReader::max_doc(&*reader), 10);
        assert_eq!(IndexReader::num_docs(&*reader), 9);
        assert_eq!(reader.num_deleted_docs(), 1);
        assert!(reader.has_deletions());

        let ctx = Arc::clone(&reader).get_context();
        assert!(ctx.is_top_level());
        assert!(ctx.is_leaf_context());
        let leaves = ctx.leaves();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].doc_base(), 0);

        assert!(reader.doc_freq(&Term::from_text("field", "term")).unwrap() == 0);
        assert!(
            reader
                .total_term_freq(&Term::from_text("field", "term"))
                .unwrap()
                == 0
        );
        assert!(reader.get_sum_doc_freq("field").unwrap() == 0);
        assert!(reader.get_doc_count("field").unwrap() == 0);
        assert!(reader.get_sum_total_term_freq("field").unwrap() == 0);
        assert!(reader
            .postings(&Term::from_text("field", "term"), POSTINGS_ENUM_FREQS)
            .unwrap()
            .is_none());
    }

    #[test]
    fn empty_term_vectors_returns_none() {
        let tv = EmptyTermVectors;
        assert!(tv.get(0).unwrap().is_none());
        assert!(tv.get_field(0, "f").unwrap().is_none());
    }
}
