//! `BaseCompositeReader` and `BaseTermsEnum` ported from `org.apache.lucene.index`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::index_reader::IndexReader;
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
/// `term_state` and `attributes` behaviour.
///
/// Equivalent to `org.apache.lucene.index.BaseTermsEnum`.
///
/// **Divergence from Lucene 10.5.0.** Java needs an intermediate abstract class
/// because `TermsEnum` declares those three methods abstract, so every
/// implementation would otherwise have to write them. Rust puts default bodies
/// on the trait itself — [`TermsEnum::seek_exact`], [`TermsEnum::term_state`]
/// and [`TermsEnum::attributes`] already carry exactly the bodies
/// `BaseTermsEnum` supplies — so the class has no work left to do and this is a
/// blanket marker rather than a type with behaviour of its own.
pub trait BaseTermsEnum: TermsEnum {}

impl<T: TermsEnum + ?Sized> BaseTermsEnum for T {}
