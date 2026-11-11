//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFilterLeafReader.java`.
//!
//! * `TestTermsEnum.postings(reuse, flags)` unwraps `reuse` to the
//!   `FilterPostingsEnum`'s `in`; a Rust `Box<dyn PostingsEnum>` cannot be
//!   narrowed to `TestPositions`, so the delegate is asked with no reuse
//!   (reuse is a hint, which changes no result).
//! * `testOverrideMethods` reflects on `getMethods()`; Rucene has no run-time
//!   reflection, so the methods are declared lists: the public non-abstract,
//!   non-final, non-static methods each superclass declares (`LeafReader`,
//!   `Fields`, `Terms`, `TermsEnum`, `PostingsEnum` and the classes they
//!   extend), and the ones of those that Rucene's filter implementations in
//!   `src/index/filter_reader.rs` define, under their Java names.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::doc_values::{DocValuesIterator, FilterNumericDocValues, NumericDocValues};
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::filter_reader::{FilterLeafReader, FilterTerms, FilterTermsEnum};
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{
    ImpactsEnum, IndexReader, LeafReader, PostingsEnum, SeekStatus, TermState, Terms, TermsEnum,
    POSTINGS_ENUM_ALL, POSTINGS_ENUM_NONE,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::attribute::AttributeSource;
use rucene::util::bit_set::BitSet;
use rucene::util::{BytesRef, FixedBitSet, IOBooleanSupplier};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};
use crate::support::npe;

/// `TestFilterLeafReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFilterLeafReader",
        vec![
            test("testFilterIndexReader", test_filter_index_reader),
            test("testOverrideMethods", test_override_methods),
            test("testUnwrap", test_unwrap),
        ],
    )
}

// -----------------------------------------------------------------------------
// TestReader
// -----------------------------------------------------------------------------

/// `private static class TestReader extends FilterLeafReader`.
struct TestReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for TestReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestReader").finish_non_exhaustive()
    }
}

impl fmt::Display for TestReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for TestReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, do_close, term_vectors, num_docs, max_doc,
        stored_fields, get_numeric_doc_values, get_binary_doc_values, get_sorted_doc_values,
        get_sorted_numeric_doc_values, get_sorted_set_doc_values,
        get_doc_values_skipper, get_float_vector_values, get_byte_vector_values,
        search_nearest_vectors, search_nearest_vectors_byte, get_field_infos, get_live_docs,
        get_point_values, check_integrity, get_meta_data);

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let terms = (&self.inner as &dyn LeafReader).terms(field)?;
        Ok(terms.map(|terms| {
            Box::new(TestTerms {
                inner: FilterTerms::new(terms),
            }) as Box<dyn Terms>
        }))
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        let ndv = (&self.inner as &dyn LeafReader).get_norm_values(field)?;
        let Some(ndv) = ndv else {
            return Ok(None);
        };
        let mut docs_with_terms = FixedBitSet::new(LeafReader::max_doc(self) as usize);
        let mut terms_enum = self
            .terms(field)?
            .unwrap_or_else(|| npe("terms"))
            .iterator()?;
        let mut postings: Option<Box<dyn PostingsEnum>> = None;
        while terms_enum.next()?.is_some() {
            let mut p = terms_enum
                .postings(postings.take(), POSTINGS_ENUM_NONE)?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            docs_with_terms.or(p.as_mut())?;
            postings = Some(p);
        }
        Ok(Some(Box::new(NormFilter {
            inner: FilterNumericDocValues::new(ndv),
            docs_with_terms,
        })))
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// The anonymous `FilterNumericDocValues` of `TestReader.getNormValues`.
struct NormFilter {
    inner: FilterNumericDocValues,
    docs_with_terms: FixedBitSet,
}

impl DocIdSetIterator for NormFilter {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }
    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }
    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }
    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for NormFilter {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl NumericDocValues for NormFilter {
    fn long_value(&self) -> Result<i64> {
        if self.docs_with_terms.get(self.doc_id() as usize) {
            self.inner.long_value()
        } else {
            Ok(0)
        }
    }
}

/// `private static class TestTerms extends FilterTerms`.
struct TestTerms {
    inner: FilterTerms,
}

impl Terms for TestTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(TestTermsEnum {
            inner: FilterTermsEnum::new(self.inner.iterator()?),
        }))
    }
    fn size(&self) -> Result<i64> {
        self.inner.size()
    }
    fn sum_total_term_freq(&self) -> Result<i64> {
        self.inner.sum_total_term_freq()
    }
    fn sum_doc_freq(&self) -> Result<i64> {
        self.inner.sum_doc_freq()
    }
    fn doc_count(&self) -> Result<i32> {
        self.inner.doc_count()
    }
    fn has_freqs(&self) -> bool {
        self.inner.has_freqs()
    }
    fn has_offsets(&self) -> bool {
        self.inner.has_offsets()
    }
    fn has_positions(&self) -> bool {
        self.inner.has_positions()
    }
    fn has_payloads(&self) -> bool {
        self.inner.has_payloads()
    }
    fn stats(&self) -> Result<String> {
        self.inner.stats()
    }
}

/// `private static class TestTermsEnum extends FilterTermsEnum`.
struct TestTermsEnum {
    inner: FilterTermsEnum,
}

impl fmt::Display for TestTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl TermsEnum for TestTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }
    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }
    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.inner.seek_exact(text)
    }
    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }
    /// Scan for terms containing the letter 'e'.
    fn next(&mut self) -> Result<Option<BytesRef>> {
        while let Some(text) = self.inner.next()? {
            if text.utf8_to_string()?.contains('e') {
                return Ok(Some(text));
            }
        }
        Ok(None)
    }
    fn term(&self) -> Result<Option<BytesRef>> {
        self.inner.term()
    }
    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }
    fn doc_freq(&mut self) -> Result<i32> {
        self.inner.doc_freq()
    }
    fn total_term_freq(&mut self) -> Result<i64> {
        self.inner.total_term_freq()
    }
    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        Ok(Some(Box::new(TestPositions {
            // `FilterPostingsEnum`'s constructor rejects a `null`.
            r#in: self.inner.postings(None, flags)?.ok_or_else(|| {
                rucene::error::LuceneError::null_pointer_with_message(
                    "incoming PostingsEnum must not be null",
                )
            })?,
        })))
    }
    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }
    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }
    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        self.inner.prepare_seek_exact(text)
    }
    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }
}

/// Filter that only returns odd numbered documents.
///
/// (`private static class TestPositions extends FilterPostingsEnum`.)
struct TestPositions {
    r#in: Box<dyn PostingsEnum>,
}

impl fmt::Display for TestPositions {
    /// The inherited `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.TestFilterLeafReader$TestReader$TestPositions",
            self,
        ))
    }
}

impl DocIdSetIterator for TestPositions {
    fn doc_id(&self) -> i32 {
        self.r#in.doc_id()
    }
    /// Scan for odd numbered documents.
    fn next_doc(&mut self) -> Result<i32> {
        loop {
            let doc = self.r#in.next_doc()?;
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if (doc % 2) == 1 {
                return Ok(doc);
            }
        }
    }
    fn advance(&mut self, target: i32) -> Result<i32> {
        self.r#in.advance(target)
    }
    fn cost(&self) -> i64 {
        self.r#in.cost()
    }
}

impl PostingsEnum for TestPositions {
    fn freq(&self) -> Result<i32> {
        self.r#in.freq()
    }
    fn next_position(&mut self) -> Result<i32> {
        self.r#in.next_position()
    }
    fn start_offset(&self) -> i32 {
        self.r#in.start_offset()
    }
    fn end_offset(&self) -> i32 {
        self.r#in.end_offset()
    }
    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.r#in.get_payload()
    }
}

/// `((BaseDirectoryWrapper) target).setCrossCheckTermVectorsOnClose(value)`.
fn set_cross_check_term_vectors_on_close(dir: &TestDirectory, value: bool) {
    match dir {
        TestDirectory::Mock(mock) => mock.set_cross_check_term_vectors_on_close(value),
        TestDirectory::Raw(raw) => raw.base().set_cross_check_term_vectors_on_close(value),
    }
}

/// Tests the IndexReader.getFieldNames implementation
///
/// @throws Exception on error
/// Port of `TestFilterLeafReader.testFilterIndexReader`.
fn test_filter_index_reader() -> Result<()> {
    let directory = LuceneTestCase::new_directory();

    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    for text in ["one two", "one three", "two four"] {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_text_field(
            "default",
            text,
            Store::YES,
        )));
        writer.add_document(&d)?;
    }
    writer.force_merge(1)?;
    writer.close()?;

    let target = LuceneTestCase::new_directory();

    // We mess with the postings so this can fail:
    set_cross_check_term_vectors_on_close(&target, false);

    let writer = IndexWriter::new(
        target.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    {
        let reader: Arc<dyn LeafReader> = Arc::new(TestReader {
            inner: FilterLeafReader::new(LuceneTestCase::get_only_leaf_reader(
                directory_reader::open(directory.as_directory())? as Arc<dyn IndexReader>,
            )?)?,
        });
        let added =
            writer.add_indexes_codec_readers(&[SlowCodecReaderWrapper::wrap(Arc::clone(&reader))?]);
        let closed = Arc::clone(&reader).as_index_reader().close();
        added?;
        closed?;
    }
    writer.close()?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(target.as_directory())?;

    let mut terms = MultiTerms::get_terms(&reader, "default")?
        .unwrap_or_else(|| npe("terms"))
        .iterator()?;
    while terms.next()?.is_some() {
        Assert::assert_true(
            terms
                .term()?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?
                .utf8_to_string()?
                .contains('e'),
        );
    }

    assert_equals(
        &SeekStatus::FOUND,
        &terms.seek_ceil(&BytesRef::from_char_sequence("one"))?,
    );

    let mut positions = terms
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    while positions.next_doc()? != NO_MORE_DOCS {
        Assert::assert_true((positions.doc_id() % 2) == 1);
    }

    reader.close()?;
    directory.close()?;
    target.close()
}

/// `superClazz.getMethods()` that are neither static, abstract, final,
/// synthetic, `attributes` nor `getStats`, and are not `Object`'s: the
/// methods a filter class must not override, by filter class.
const DEFAULT_IMPL_METHODS: &[(&str, &[&str])] = &[
    // `LeafReader`, `IndexReader`: none.
    ("org.apache.lucene.index.FilterLeafReader", &[]),
    // `Fields`: none.
    ("org.apache.lucene.index.FilterLeafReader$FilterFields", &[]),
    // `Terms.java`: `intersect`, `getMin`, `getMax` (`getStats` is skipped).
    (
        "org.apache.lucene.index.FilterLeafReader$FilterTerms",
        &["intersect", "getMin", "getMax"],
    ),
    // `TermsEnum.java`: `preferSeekExact`.
    (
        "org.apache.lucene.index.FilterLeafReader$FilterTermsEnum",
        &["preferSeekExact"],
    ),
    // `PostingsEnum.java`, `DocIdSetIterator.java`: `nextPostings`,
    // `intoBitSet`, `docIDRunEnd`.
    (
        "org.apache.lucene.index.FilterLeafReader$FilterPostingsEnum",
        &["nextPostings", "intoBitSet", "docIDRunEnd"],
    ),
];

/// The methods with a default implementation that each Rucene filter type
/// defines (`src/index/filter_reader.rs`), under their Java names.
const OVERRIDDEN_DEFAULT_METHODS: &[(&str, &[&str])] = &[
    ("org.apache.lucene.index.FilterLeafReader", &[]),
    ("org.apache.lucene.index.FilterLeafReader$FilterFields", &[]),
    ("org.apache.lucene.index.FilterLeafReader$FilterTerms", &[]),
    (
        "org.apache.lucene.index.FilterLeafReader$FilterTermsEnum",
        &[],
    ),
    (
        "org.apache.lucene.index.FilterLeafReader$FilterPostingsEnum",
        &[],
    ),
];

/// `checkOverrideMethods(clazz)`.
fn check_override_methods(clazz: &str) {
    let defaults = DEFAULT_IMPL_METHODS
        .iter()
        .find(|(c, _)| *c == clazz)
        .map_or(&[][..], |(_, m)| *m);
    let overridden = OVERRIDDEN_DEFAULT_METHODS
        .iter()
        .find(|(c, _)| *c == clazz)
        .map_or(&[][..], |(_, m)| *m);
    for m in defaults {
        // The point of these checks is to ensure that methods that have a default
        // impl through other methods are not overridden. This makes the number of
        // methods to override to have a working impl minimal and prevents from some
        // traps: for example, think about having getCoreCacheKey delegate to the
        // filtered impl by default
        if overridden.contains(m) {
            Assert::fail(&format!(
                "class {clazz} overrides {m} although it has a default impl"
            ));
        }
    }
}

/// Port of `TestFilterLeafReader.testOverrideMethods`.
fn test_override_methods() -> Result<()> {
    check_override_methods("org.apache.lucene.index.FilterLeafReader");
    check_override_methods("org.apache.lucene.index.FilterLeafReader$FilterFields");
    check_override_methods("org.apache.lucene.index.FilterLeafReader$FilterTerms");
    check_override_methods("org.apache.lucene.index.FilterLeafReader$FilterTermsEnum");
    check_override_methods("org.apache.lucene.index.FilterLeafReader$FilterPostingsEnum");
    Ok(())
}

/// The anonymous `FilterLeafReader` of `testUnwrap`.
struct ForwardingReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for ForwardingReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForwardingReader").finish_non_exhaustive()
    }
}

impl fmt::Display for ForwardingReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for ForwardingReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, do_close, term_vectors, num_docs, max_doc,
        stored_fields, terms, get_numeric_doc_values, get_binary_doc_values,
        get_sorted_doc_values, get_sorted_numeric_doc_values, get_sorted_set_doc_values,
        get_norm_values, get_doc_values_skipper, get_float_vector_values,
        get_byte_vector_values, search_nearest_vectors, search_nearest_vectors_byte,
        get_field_infos, get_live_docs, get_point_values, check_integrity, get_meta_data);

    /// `in.getCoreCacheHelper()`.
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }

    /// `in.getReaderCacheHelper()`.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }
}

/// The data address of a reader, for `assertEquals` on readers (`Object`
/// identity).
fn addr(r: &Arc<dyn LeafReader>) -> *const () {
    Arc::as_ptr(r) as *const ()
}

/// Port of `TestFilterLeafReader.testUnwrap`.
fn test_unwrap() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    let dr = w.get_reader()?;
    let r = (Arc::clone(&dr) as Arc<dyn IndexReader>).leaves()?[0].leaf_reader();
    let r2: Arc<dyn LeafReader> = Arc::new(ForwardingReader {
        inner: FilterLeafReader::new(Arc::clone(&r))?,
    });
    assert_equals(
        &addr(&r),
        &addr(
            r2.filter_leaf_reader_delegate()
                .unwrap_or_else(|| npe("delegate")),
        ),
    );
    assert_equals(&addr(&r), &addr(&FilterLeafReader::unwrap(&r2)));
    w.close()?;
    dr.close()?;
    dir.close()
}
