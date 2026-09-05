//! `BaseCompositeReader` and `BaseTermsEnum` ported from `org.apache.lucene.index`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::index_reader::{IndexReader, StoredFields};
use crate::index::leaf_reader::TermVectors;
use crate::index::multi_reader::{MultiStoredFields, MultiTermVectors};
use crate::index::terms::TermsEnum;
use crate::index::Term;

/// The largest `maxDoc` a single index may hold.
///
/// Equivalent to the value `IndexWriter.getActualMaxDocs()` returns: Lucene caps
/// a segment at `IndexWriter.MAX_DOCS`, which is `Integer.MAX_VALUE - 128`.
pub const ACTUAL_MAX_DOCS: i32 = i32::MAX - 128;

/// Base state shared by every composite reader: the sub-reader array, the
/// per-sub-reader doc bases, and the lazily computed live-document count.
///
/// Equivalent to `org.apache.lucene.index.BaseCompositeReader`.
///
/// **Divergence from Lucene 10.5.0.** Java makes this an abstract class that a
/// concrete reader extends, inheriting `numDocs`, `maxDoc`, `readerIndex`,
/// `docFreq` and the rest. Rust has no implementation inheritance, so the port
/// is a struct a concrete reader holds and delegates to. The arithmetic, the
/// laziness of `num_docs` and the `starts` layout are unchanged.
pub struct BaseCompositeReader {
    sub_readers: Vec<Arc<dyn IndexReader>>,
    /// First document number of each sub-reader, plus a trailing `max_doc`.
    starts: Vec<i32>,
    max_doc: i32,
    /// Computed on first use, as Java does, so wrapping a reader stays cheap.
    num_docs: AtomicI32,
}

impl std::fmt::Debug for BaseCompositeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BaseCompositeReader")
            .field("sub_readers", &self.sub_readers.len())
            .field("max_doc", &self.max_doc)
            .finish()
    }
}

impl BaseCompositeReader {
    /// Builds the base state over `sub_readers`, computing the doc bases.
    ///
    /// `is_directory_reader` selects which failure Lucene raises when the total
    /// document count overflows: a corrupt index for a `DirectoryReader`, an
    /// illegal argument for a `MultiReader` the caller assembled.
    pub fn new(sub_readers: Vec<Arc<dyn IndexReader>>, is_directory_reader: bool) -> Result<Self> {
        let mut starts = vec![0i32; sub_readers.len() + 1];
        let mut max_doc: i64 = 0;
        for (i, reader) in sub_readers.iter().enumerate() {
            starts[i] = max_doc as i32;
            max_doc += i64::from(reader.max_doc());
        }

        if max_doc > i64::from(ACTUAL_MAX_DOCS) {
            let message = format!(
                "Too many documents: an index cannot exceed {ACTUAL_MAX_DOCS} but readers have total maxDoc={max_doc}"
            );
            return Err(if is_directory_reader {
                LuceneError::corrupt_index(message, format!("{} sub-readers", sub_readers.len()))
            } else {
                LuceneError::IllegalArgument(format!(
                    "Too many documents: composite IndexReaders cannot exceed {ACTUAL_MAX_DOCS} but readers have total maxDoc={max_doc}"
                ))
            });
        }

        let max_doc = max_doc as i32;
        let n = sub_readers.len();
        starts[n] = max_doc;

        Ok(Self {
            sub_readers,
            starts,
            max_doc,
            num_docs: AtomicI32::new(-1),
        })
    }

    /// Returns the sub-readers, in order.
    ///
    /// Equivalent to `BaseCompositeReader.getSequentialSubReaders()`.
    pub fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.sub_readers.clone()
    }

    /// Returns the first document number of sub-reader `i`.
    ///
    /// Equivalent to `BaseCompositeReader.readerBase(int)`.
    pub fn reader_base(&self, reader_index: usize) -> Result<i32> {
        self.starts.get(reader_index).copied().ok_or_else(|| {
            LuceneError::IllegalArgument(format!("readerIndex {reader_index} is out of bounds"))
        })
    }

    /// Returns the index of the sub-reader that owns `doc_id`.
    ///
    /// Equivalent to `BaseCompositeReader.readerIndex(int)`, which binary-searches
    /// the `starts` array.
    pub fn reader_index(&self, doc_id: i32) -> Result<usize> {
        if doc_id < 0 || doc_id >= self.max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "docID must be >= 0 and < maxDoc={} (got docID={doc_id})",
                self.max_doc
            )));
        }
        // `starts` is ascending with a trailing max_doc, so the owning reader is
        // the last entry not greater than doc_id.
        let hi = self.sub_readers.len() - 1;
        match self.starts[..=hi].binary_search(&doc_id) {
            Ok(index) => Ok(index),
            Err(insertion) => Ok(insertion - 1),
        }
    }

    /// Returns one greater than the largest document number.
    pub fn max_doc(&self) -> i32 {
        self.max_doc
    }

    /// Returns the number of live documents, summing the sub-readers on first
    /// use.
    pub fn num_docs(&self) -> i32 {
        let cached = self.num_docs.load(Ordering::Relaxed);
        if cached != -1 {
            return cached;
        }
        let total: i32 = self.sub_readers.iter().map(|r| r.num_docs()).sum();
        self.num_docs.store(total, Ordering::Relaxed);
        total
    }

    /// Returns a [`TermVectors`] view that dispatches each document to the
    /// sub-reader owning it.
    ///
    /// Equivalent to `BaseCompositeReader.termVectors()`, a `final` method that
    /// every composite reader inherits: it translates the global document id
    /// into the sub-reader's local id by subtracting that sub-reader's start,
    /// and opens the sub-reader's own `TermVectors` on first use.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening a sub-reader's term vectors raises.
    pub fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Ok(Box::new(MultiTermVectors::new(
            self.sub_readers.clone(),
            self.starts.clone(),
            self.max_doc,
        )))
    }

    /// Returns a [`StoredFields`] view that dispatches each document to the
    /// sub-reader owning it.
    ///
    /// Equivalent to `BaseCompositeReader.storedFields()`, the `final`
    /// counterpart of [`term_vectors`](Self::term_vectors).
    ///
    /// # Errors
    ///
    /// Propagates whatever opening a sub-reader's stored fields raises.
    pub fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(MultiStoredFields::new(
            self.sub_readers.clone(),
            self.starts.clone(),
            self.max_doc,
        )))
    }

    /// Sums `doc_freq` across the sub-readers.
    ///
    /// Equivalent to `BaseCompositeReader.docFreq(Term)`.
    pub fn doc_freq(&self, term: &Term) -> Result<i32> {
        let mut total = 0i32;
        for reader in &self.sub_readers {
            total += reader.doc_freq(term)?;
        }
        Ok(total)
    }

    /// Sums `total_term_freq` across the sub-readers, returning `-1` as soon as
    /// one sub-reader does not track it.
    ///
    /// Equivalent to `BaseCompositeReader.totalTermFreq(Term)`.
    pub fn total_term_freq(&self, term: &Term) -> Result<i64> {
        let mut total = 0i64;
        for reader in &self.sub_readers {
            let sub = reader.total_term_freq(term)?;
            if sub == -1 {
                return Ok(-1);
            }
            total += sub;
        }
        Ok(total)
    }

    /// Sums `get_sum_doc_freq` across the sub-readers, returning `-1` as soon as
    /// one sub-reader does not track it.
    pub fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        let mut total = 0i64;
        for reader in &self.sub_readers {
            let sub = reader.get_sum_doc_freq(field)?;
            if sub == -1 {
                return Ok(-1);
            }
            total += sub;
        }
        Ok(total)
    }

    /// Sums `get_doc_count` across the sub-readers, returning `-1` as soon as one
    /// sub-reader does not track it.
    pub fn get_doc_count(&self, field: &str) -> Result<i32> {
        let mut total = 0i32;
        for reader in &self.sub_readers {
            let sub = reader.get_doc_count(field)?;
            if sub == -1 {
                return Ok(-1);
            }
            total += sub;
        }
        Ok(total)
    }

    /// Sums `get_sum_total_term_freq` across the sub-readers, returning `-1` as
    /// soon as one sub-reader does not track it.
    pub fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        let mut total = 0i64;
        for reader in &self.sub_readers {
            let sub = reader.get_sum_total_term_freq(field)?;
            if sub == -1 {
                return Ok(-1);
            }
            total += sub;
        }
        Ok(total)
    }
}

/// Marker for a [`TermsEnum`] that relies on the default `seek_exact`,
/// `prepare_seek_exact`, `term_state` and `attributes` behaviour.
///
/// Equivalent to `org.apache.lucene.index.BaseTermsEnum`.
///
/// **Divergence from Lucene 10.5.0.** Java needs an intermediate abstract class
/// because `TermsEnum` declares those four methods abstract, so every
/// implementation would otherwise have to write them. Rust puts default bodies
/// on the trait itself — [`TermsEnum::seek_exact`],
/// [`TermsEnum::prepare_seek_exact`], [`TermsEnum::term_state`] and
/// [`TermsEnum::attributes`] already carry exactly the bodies `BaseTermsEnum`
/// supplies — so the class has no work left to do and this is a blanket marker
/// rather than a type with behaviour of its own.
///
/// [`TermsEnum::prepare_seek_exact`] in particular carries the body of
/// `org.apache.lucene.index.BaseTermsEnum#prepareSeekExact`: a supplier that
/// defers to `seek_exact`, with no prefetch of its own.
pub trait BaseTermsEnum: TermsEnum {}

impl<T: TermsEnum + ?Sized> BaseTermsEnum for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::StoredFieldVisitor;
    use crate::index::index_reader::{CacheHelper, IndexReaderCore};
    use crate::index::reader_context::IndexReaderContext;
    use crate::index::{EmptyFields, Fields};
    use std::sync::Weak;

    /// Reports the sub-reader-local document id it was handed, so the doc-base
    /// arithmetic can be observed from the outside.
    #[derive(Debug)]
    struct StubStoredFields;

    impl StoredFields for StubStoredFields {
        fn document_with_visitor(
            &self,
            doc_id: i32,
            _visitor: &mut dyn StoredFieldVisitor,
        ) -> Result<()> {
            Err(LuceneError::IllegalState(format!("local doc {doc_id}")))
        }
    }

    #[derive(Debug)]
    struct StubTermVectors;

    impl crate::index::leaf_reader::TermVectors for StubTermVectors {
        fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
            if doc == 0 {
                Ok(Some(Box::new(EmptyFields)))
            } else {
                Err(LuceneError::IllegalState(format!("local doc {doc}")))
            }
        }
    }

    #[derive(Debug)]
    struct StubReader {
        core: IndexReaderCore,
        max_doc: i32,
    }

    impl IndexReader for StubReader {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn crate::index::leaf_reader::TermVectors>> {
            Ok(Box::new(StubTermVectors))
        }
        fn num_docs(&self) -> i32 {
            self.max_doc
        }
        fn max_doc(&self) -> i32 {
            self.max_doc
        }
        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Ok(Box::new(StubStoredFields))
        }
        fn do_close(&self) -> Result<()> {
            Ok(())
        }
        fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }
        fn doc_freq(&self, _term: &Term) -> Result<i32> {
            Ok(0)
        }
        fn total_term_freq(&self, _term: &Term) -> Result<i64> {
            Ok(0)
        }
        fn get_sum_doc_freq(&self, _field: &str) -> Result<i64> {
            Ok(0)
        }
        fn get_doc_count(&self, _field: &str) -> Result<i32> {
            Ok(0)
        }
        fn get_sum_total_term_freq(&self, _field: &str) -> Result<i64> {
            Ok(0)
        }
        fn build_context(
            self: Arc<Self>,
            _parent: Option<Weak<dyn IndexReaderContext>>,
            _ord_in_parent: i32,
            _doc_base_in_parent: i32,
            _leaf_ord: i32,
            _leaf_doc_base: i32,
        ) -> Arc<dyn IndexReaderContext> {
            unreachable!("this stub is never asked for a reader context")
        }
    }

    fn base_of(sizes: &[i32]) -> BaseCompositeReader {
        let subs: Vec<Arc<dyn IndexReader>> = sizes
            .iter()
            .map(|&size| {
                Arc::new(StubReader {
                    core: IndexReaderCore::new(),
                    max_doc: size,
                }) as Arc<dyn IndexReader>
            })
            .collect();
        BaseCompositeReader::new(subs, false).expect("composite over stub readers")
    }

    /// `BaseCompositeReader.termVectors()` and `storedFields()` translate the
    /// global document id into the owning sub-reader's local id by subtracting
    /// that sub-reader's start. The stubs answer only for local id 0, so a hit
    /// on the first document of each sub-reader proves the subtraction, and the
    /// reported local id proves it for the rest.
    #[test]
    fn the_composite_views_hand_each_sub_reader_a_local_doc_id() {
        let base = base_of(&[3, 4, 2]);
        assert_eq!(base.max_doc(), 9);

        let vectors = base.term_vectors().expect("term vectors view");
        // Global 0, 3 and 7 are the first document of each sub-reader.
        for global in [0, 3, 7] {
            match vectors.get(global) {
                Ok(Some(_)) => {}
                Ok(None) => panic!("global doc {global} must map to local 0"),
                Err(error) => panic!("global doc {global} mis-resolved: {error}"),
            }
        }
        // Global 5 is local 2 of the second sub-reader, which starts at 3.
        match vectors.get(5) {
            Err(error) => assert!(error.to_string().contains("local doc 2"), "{error}"),
            other => panic!(
                "expected the stub to report the local id, got {other:?}",
                other = other.is_ok()
            ),
        }

        let stored = base.stored_fields().expect("stored fields view");
        // Global 8 is local 1 of the third sub-reader, which starts at 7.
        let mut visitor = NoopVisitor;
        match stored.document_with_visitor(8, &mut visitor) {
            Err(error) => assert!(error.to_string().contains("local doc 1"), "{error}"),
            Ok(()) => panic!("expected the stub to report the local id"),
        }
    }

    /// A visitor that is never asked for anything: the stub reports the local
    /// document id before any field is visited.
    struct NoopVisitor;

    impl StoredFieldVisitor for NoopVisitor {
        fn needs_field(
            &mut self,
            _field_info: &crate::index::FieldInfo,
        ) -> Result<crate::index::StoredFieldVisitorStatus> {
            Ok(crate::index::StoredFieldVisitorStatus::No)
        }
    }
}
