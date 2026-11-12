//! `BaseCompositeReader` and `BaseTermsEnum` ported from `org.apache.lucene.index`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::index_reader::{IndexReader, IndexReaderCore, StoredFields};
use crate::index::leaf_reader::TermVectors;
use crate::index::multi_reader::{MultiStoredFields, MultiTermVectors};
use crate::index::terms::TermsEnum;
use crate::index::Term;

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
    ///
    /// `parent` is the [`IndexReaderCore`] of the composite reader being built:
    /// each sub-reader registers it as a parent while the doc bases are
    /// computed, as Java's constructor does with `r.registerParentReader(this)`
    /// (`BaseCompositeReader.java:83-88`), so that closing a sub-reader makes
    /// the composite fail `ensureOpen()`. The composite's own core exists
    /// before the composite does, which is what lets the registration happen
    /// here, inside the constructor, as in Java.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::AlreadyClosed`](crate::error::LuceneErrorKind::AlreadyClosed) when a sub-reader is closed (its
    /// `registerParentReader` calls `ensureOpen()`), then the document-limit
    /// failures described above.
    pub fn new(
        sub_readers: Vec<Arc<dyn IndexReader>>,
        is_directory_reader: bool,
        parent: &IndexReaderCore,
    ) -> Result<Self> {
        let mut starts = vec![0i32; sub_readers.len() + 1];
        let mut max_doc: i64 = 0;
        for (i, reader) in sub_readers.iter().enumerate() {
            starts[i] = max_doc as i32;
            max_doc += i64::from(reader.max_doc());
            reader.core().register_parent_core(parent)?;
        }

        // `if (maxDoc > IndexWriter.getActualMaxDocs())`
        // (`BaseCompositeReader.java:90-109`): the limit is the one
        // `IndexWriter.setMaxDocs` may have lowered, read each time.
        let actual_max_docs = crate::index::index_writer::get_actual_max_docs();
        if max_doc > i64::from(actual_max_docs) {
            let message = format!(
                "Too many documents: an index cannot exceed {actual_max_docs} but readers have total maxDoc={max_doc}"
            );
            return Err(if is_directory_reader {
                // The resource is `Arrays.toString(subReaders)`
                // (`BaseCompositeReader.java:94-99`): every sub-reader's
                // `toString()`, comma-separated inside brackets.
                let sub_readers = sub_readers
                    .iter()
                    .map(|reader| reader.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                LuceneError::corrupt_index(message, format!("[{sub_readers}]"))
            } else {
                LuceneError::illegal_argument(format!(
                    "Too many documents: composite IndexReaders cannot exceed {actual_max_docs} but readers have total maxDoc={max_doc}"
                ))
            });
        }

        // `this.maxDoc = Math.toIntExact(maxDoc)` (`BaseCompositeReader.java:111`),
        // which the check above leaves nothing to refuse.
        let max_doc = crate::error::to_int_exact(max_doc)?;
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
        // `BaseCompositeReader.java:271-276`: the bound is the sub-reader
        // count, not the length of `starts` (which carries `maxDoc` last).
        if reader_index >= self.sub_readers.len() {
            return Err(LuceneError::illegal_argument(
                "readerIndex must be >= 0 and < getSequentialSubReaders().size()",
            ));
        }
        Ok(self.starts[reader_index])
    }

    /// Returns the index of the sub-reader that owns `doc_id`.
    ///
    /// Equivalent to `BaseCompositeReader.readerIndex(int)`
    /// (`BaseCompositeReader.java:262-268`): the bounds check, then
    /// `ReaderUtil.subIndex(docID, starts)`, which resolves a run of empty
    /// sub-readers to the last of them. The body is the one the
    /// [`stored_fields`](Self::stored_fields) and
    /// [`term_vectors`](Self::term_vectors) dispatch runs for every document,
    /// as Java's anonymous views call `readerIndex`
    /// (`BaseCompositeReader.java:122, 131, 177, 186`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when `doc_id` is outside
    /// `0..max_doc`.
    pub fn reader_index(&self, doc_id: i32) -> Result<usize> {
        Ok(crate::index::multi_reader::resolve_sub(doc_id, self.max_doc, &self.starts)?.0)
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
        debug_assert!(total >= 0);
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
        let mut total = 0i32; // sum freqs in subreaders
        for reader in &self.sub_readers {
            let sub = reader.doc_freq(term)?;
            debug_assert!(sub >= 0);
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Sums `total_term_freq` across the sub-readers, as
    /// `BaseCompositeReader.totalTermFreq(Term)` does
    /// (`BaseCompositeReader.java:209-219`): every sub-reader is added, with
    /// Java's wrapping `long` sum.
    pub fn total_term_freq(&self, term: &Term) -> Result<i64> {
        let mut total = 0i64; // sum freqs in subreaders
        for reader in &self.sub_readers {
            let sub = reader.total_term_freq(term)?;
            debug_assert!(sub >= 0);
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Sums `get_sum_doc_freq` across the sub-readers, as
    /// `BaseCompositeReader.getSumDocFreq(String)` does
    /// (`BaseCompositeReader.java:222-232`).
    pub fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        let mut total = 0i64; // sum doc freqs in subreaders
        for reader in &self.sub_readers {
            let sub = reader.get_sum_doc_freq(field)?;
            debug_assert!(sub >= 0);
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Sums `get_doc_count` across the sub-readers, as
    /// `BaseCompositeReader.getDocCount(String)` does
    /// (`BaseCompositeReader.java:235-245`).
    pub fn get_doc_count(&self, field: &str) -> Result<i32> {
        let mut total = 0i32; // sum doc counts in subreaders
        for reader in &self.sub_readers {
            let sub = reader.get_doc_count(field)?;
            debug_assert!(sub >= 0);
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Sums `get_sum_total_term_freq` across the sub-readers, as
    /// `BaseCompositeReader.getSumTotalTermFreq(String)` does
    /// (`BaseCompositeReader.java:248-259`).
    pub fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        let mut total = 0i64; // sum doc total term freqs in subreaders
        for reader in &self.sub_readers {
            let sub = reader.get_sum_total_term_freq(field)?;
            debug_assert!(sub >= 0);
            total = total.wrapping_add(sub);
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
    use crate::index::index_reader::CacheHelper;
    use crate::index::reader_context::IndexReaderContext;
    use crate::index::terms::EmptyFields;
    use crate::index::Fields;
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
            Err(LuceneError::illegal_state(format!("local doc {doc_id}")))
        }
    }

    #[derive(Debug)]
    struct StubTermVectors;

    impl crate::index::leaf_reader::TermVectors for StubTermVectors {
        fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
            if doc == 0 {
                Ok(Some(Box::new(EmptyFields)))
            } else {
                Err(LuceneError::illegal_state(format!("local doc {doc}")))
            }
        }
    }

    #[derive(Debug)]
    struct StubReader {
        core: IndexReaderCore,
        max_doc: i32,
    }

    impl std::fmt::Display for StubReader {
        /// Test double; it has no Lucene counterpart whose `toString()` could be
        /// reproduced, so it renders under its own name. `Display` is required
        /// because `LeafReader`/`IndexReader` carry it as a supertrait, standing in
        /// for the `toString()` every Java reader inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("StubReader")
        }
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

    fn stubs(sizes: &[i32]) -> Vec<Arc<dyn IndexReader>> {
        sizes
            .iter()
            .map(|&size| {
                Arc::new(StubReader {
                    core: IndexReaderCore::new(),
                    max_doc: size,
                }) as Arc<dyn IndexReader>
            })
            .collect()
    }

    fn base_of(sizes: &[i32]) -> BaseCompositeReader {
        BaseCompositeReader::new(stubs(sizes), false, &IndexReaderCore::new())
            .expect("composite over stub readers")
    }

    /// The document limit is `IndexWriter.getActualMaxDocs()`, read when the
    /// reader is built (`BaseCompositeReader.java:90-109`), so a limit lowered
    /// through `IndexWriter.setMaxDocs` applies — to `MultiReader` too, whose
    /// Java constructor is `BaseCompositeReader`'s.
    ///
    /// The limit is process-wide, so it is lowered only to two billion, far
    /// above any document count another test running at the same time reaches,
    /// and a guard puts the previous limit back even if an assertion fails.
    #[test]
    fn the_document_limit_is_the_writers_actual_max_docs() {
        use crate::index::index_writer::{get_actual_max_docs, set_max_docs};

        struct RestoreMaxDocs(i32);
        impl Drop for RestoreMaxDocs {
            fn drop(&mut self) {
                set_max_docs(self.0).expect("the previous limit is valid");
            }
        }
        let _restore = RestoreMaxDocs(get_actual_max_docs());
        const LIMIT: i32 = 2_000_000_000;
        set_max_docs(LIMIT).unwrap();

        let sizes = [1_000_000_000, 1_000_000_001];
        let composite =
            BaseCompositeReader::new(stubs(&sizes), false, &IndexReaderCore::new()).unwrap_err();
        assert_eq!(
            composite.to_string(),
            LuceneError::illegal_argument(
                "Too many documents: composite IndexReaders cannot exceed 2000000000 but readers \
                 have total maxDoc=2000000001"
                    .to_string()
            )
            .to_string()
        );
        // A directory reader's failure names `Arrays.toString(subReaders)` as
        // its resource (`BaseCompositeReader.java:94-99`).
        let directory =
            BaseCompositeReader::new(stubs(&sizes), true, &IndexReaderCore::new()).unwrap_err();
        assert_eq!(
            directory.to_string(),
            LuceneError::corrupt_index(
                "Too many documents: an index cannot exceed 2000000000 but readers have total \
                 maxDoc=2000000001",
                "[StubReader, StubReader]"
            )
            .to_string()
        );
        let multi = crate::index::multi_reader::MultiReader::new(stubs(&sizes), false).unwrap_err();
        assert_eq!(multi.to_string(), composite.to_string());
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

    /// `readerIndex` is `ReaderUtil.subIndex` over `starts`, which scans a run
    /// of equal starts to its last entry: a document right after an empty
    /// sub-reader belongs to the sub-reader that follows it, never to the empty
    /// one (`ReaderUtil.java:49-69`).
    #[test]
    fn reader_index_skips_empty_sub_readers_as_java_sub_index_does() {
        let base = base_of(&[3, 0, 0, 2]);
        assert_eq!(base.reader_index(0).unwrap(), 0);
        assert_eq!(base.reader_index(2).unwrap(), 0);
        assert_eq!(base.reader_index(3).unwrap(), 3);
        assert_eq!(base.reader_index(4).unwrap(), 3);
        let error = base.reader_index(5).unwrap_err();
        assert_eq!(
            error.to_string(),
            LuceneError::illegal_argument(
                "docID must be >= 0 and < maxDoc=5 (got docID=5)".to_string()
            )
            .to_string()
        );
        assert!(base.reader_index(-1).is_err());
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
