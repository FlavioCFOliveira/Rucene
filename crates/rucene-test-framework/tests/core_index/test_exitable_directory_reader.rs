//! Port of `lucene/core/src/test/org/apache/lucene/index/TestExitableDirectoryReader.java`.
//!
//! Test that uses a default/lucene Implementation of {@link QueryTimeout} to
//! exit out long running queries that take too long to iterate over Terms.
//!
//! * `new ExitableDirectoryReader(r, timeout)` is
//!   `ExitableDirectoryReaderWrapper::new(timeout).wrap(r)`.
//! * The anonymous `ExitableTerms` of `testExitableTermsMinAndMax`, whose
//!   `iterator()` fails, is a `Terms` that holds the `ExitableTerms` and
//!   answers `getMin()`/`getMax()` through it: Rust has no subclass of
//!   `ExitableTerms` whose inherited `getMin()` could reach the override.

use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::{
    BinaryDocValuesField, Document, IntPoint, KnnByteVectorField, KnnFloatVectorField,
    NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::DocValuesIterator;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, ExitableDirectoryReaderWrapper, ExitableTerms,
};
use rucene::index::filter_reader::{FilterLeafReader, FilterTerms, FilterTermsEnum};
use rucene::index::index_utilities::QueryTimeout;
use rucene::index::index_writer::IndexWriter;
use rucene::index::terms::Term;
use rucene::index::vector_values::{FloatVectorValues, KnnVectorValues};
use rucene::index::{
    ImpactsEnum, IndexReader, LeafReader, PostingsEnum, SeekStatus, TermState, Terms, TermsEnum,
};
use rucene::search::doc_id_set_iterator::from_live_docs;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::{Query, NO_MORE_DOCS};
use rucene::store::Directory;
use rucene::util::attribute::AttributeSource;
use rucene::util::{BytesRef, IOBooleanSupplier};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_null, class, random, test};

/// `ExitableDirectoryReader.ExitingReaderException`.
const EXITING: &str = "org.apache.lucene.index.ExitableDirectoryReader$ExitingReaderException";

/// `TestExitableDirectoryReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestExitableDirectoryReader",
        vec![
            test(
                "testExitableFilterTermsIndexReader",
                test_exitable_filter_terms_index_reader,
            ),
            test(
                "testExitableTermsEnumSampleTimeoutCheck",
                test_exitable_terms_enum_sample_timeout_check,
            )
            .annotate(Nightly),
            test(
                "testExitablePointValuesIndexReader",
                test_exitable_point_values_index_reader,
            ),
            test(
                "testExitableTermsMinAndMax",
                test_exitable_terms_min_and_max,
            ),
            test("testDocValues", test_doc_values),
            test("testFloatVectorValues", test_float_vector_values),
            test("testByteVectorValues", test_byte_vector_values),
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

impl TestReader {
    /// `new TestReader(reader)`.
    fn new(reader: Arc<dyn LeafReader>) -> Result<Arc<dyn IndexReader>> {
        let r: Arc<dyn LeafReader> = Arc::new(TestReader {
            inner: FilterLeafReader::new(reader)?,
        });
        Ok(r.as_index_reader())
    }
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
        get_sorted_numeric_doc_values, get_sorted_set_doc_values, get_norm_values,
        get_doc_values_skipper, get_float_vector_values, get_byte_vector_values,
        search_nearest_vectors, search_nearest_vectors_byte, get_field_infos, get_live_docs,
        get_point_values, check_integrity, get_meta_data);

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let terms = (&self.inner as &dyn LeafReader).terms(field)?;
        Ok(terms.map(|terms| Box::new(TestTerms::new(terms)) as Box<dyn Terms>))
    }

    /// `in.getCoreCacheHelper()`.
    fn get_core_cache_helper(&self) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }

    /// `in.getReaderCacheHelper()`.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }
}

/// `private static class TestTerms extends FilterTerms`.
struct TestTerms {
    inner: FilterTerms,
}

impl TestTerms {
    fn new(r#in: Box<dyn Terms>) -> Self {
        TestTerms {
            inner: FilterTerms::new(r#in),
        }
    }
}

impl Terms for TestTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(TestTermsEnum::new(self.inner.iterator()?)))
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

impl TestTermsEnum {
    fn new(r#in: Box<dyn TermsEnum>) -> Self {
        TestTermsEnum {
            inner: FilterTermsEnum::new(r#in),
        }
    }
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
    /// Sleep between iterations to timeout things.
    fn next(&mut self) -> Result<Option<BytesRef>> {
        // Sleep for 100ms before each .next() call.
        std::thread::sleep(std::time::Duration::from_millis(100));
        self.inner.next()
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
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.inner.postings(reuse, flags)
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

// -----------------------------------------------------------------------------
// Query timeouts
// -----------------------------------------------------------------------------

/// `() -> false`.
#[derive(Debug)]
struct InfiniteQueryTimeout;

impl QueryTimeout for InfiniteQueryTimeout {
    fn should_exit(&self) -> bool {
        false
    }
}

/// `() -> true`.
#[derive(Debug)]
struct ImmediateQueryTimeout;

impl QueryTimeout for ImmediateQueryTimeout {
    fn should_exit(&self) -> bool {
        true
    }
}

fn infinite_query_timeout() -> Arc<dyn QueryTimeout> {
    Arc::new(InfiniteQueryTimeout)
}

fn immediate_query_timeout() -> Arc<dyn QueryTimeout> {
    Arc::new(ImmediateQueryTimeout)
}

/// `private static class CountingQueryTimeout implements QueryTimeout`.
#[derive(Debug, Default)]
struct CountingQueryTimeout {
    counter: AtomicI32,
}

impl QueryTimeout for CountingQueryTimeout {
    fn should_exit(&self) -> bool {
        self.counter.fetch_add(1, Ordering::SeqCst);
        false
    }
}

impl CountingQueryTimeout {
    fn get_should_exit_call_count(&self) -> i32 {
        self.counter.load(Ordering::SeqCst)
    }
}

/// `new ExitableDirectoryReader(directoryReader, timeout)`.
fn exitable(
    directory_reader: Arc<dyn DirectoryReader>,
    timeout: Arc<dyn QueryTimeout>,
) -> Result<Arc<dyn DirectoryReader>> {
    ExitableDirectoryReaderWrapper::new(timeout).wrap(directory_reader)
}

/// `new TestReader(getOnlyLeafReader(exitableDirectoryReader))`.
fn test_reader(
    exitable_directory_reader: Arc<dyn DirectoryReader>,
) -> Result<Arc<dyn IndexReader>> {
    TestReader::new(LuceneTestCase::get_only_leaf_reader(
        exitable_directory_reader as Arc<dyn IndexReader>,
    )?)
}

/// `reader.leaves().get(0).reader()`.
fn first_leaf(reader: &Arc<dyn IndexReader>) -> Result<Arc<dyn LeafReader>> {
    Ok(Arc::clone(reader).leaves()?[0].leaf_reader())
}

/// Opens `directory`, wraps it as `ExitableDirectoryReader` with `timeout`
/// and `TestReader`, and runs `query` for 10 hits: the three runs of the
/// `testExitable*IndexReader` tests.
fn search_through(
    directory: &Arc<dyn Directory>,
    timeout: Arc<dyn QueryTimeout>,
    query: &Arc<dyn Query>,
    expect_exit: bool,
) -> Result<()> {
    let directory_reader = directory_reader::open(Arc::clone(directory))?;
    let exitable_directory_reader = exitable(directory_reader, timeout)?;
    let reader = test_reader(exitable_directory_reader)?;
    let searcher = IndexSearcher::new(Arc::clone(&reader))?;
    if expect_exit {
        expect_throws(EXITING, || searcher.search(Arc::clone(query), 10));
    } else {
        searcher.search(Arc::clone(query), 10)?;
    }
    reader.close()
}

/// Tests timing out of TermsEnum iterations
///
/// @throws Exception on error
/// Port of `TestExitableDirectoryReader.testExitableFilterTermsIndexReader`.
fn test_exitable_filter_terms_index_reader() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    let mut d1 = Document::new();
    d1.add(Box::new(LuceneTestCase::new_text_field(
        "default",
        "one two",
        Store::YES,
    )));
    writer.add_document(&d1)?;

    let mut d2 = Document::new();
    d2.add(Box::new(LuceneTestCase::new_text_field(
        "default",
        "one three",
        Store::YES,
    )));
    writer.add_document(&d2)?;

    let mut d3 = Document::new();
    d3.add(Box::new(LuceneTestCase::new_text_field(
        "default",
        "ones two four",
        Store::YES,
    )));
    writer.add_document(&d3)?;

    writer.force_merge(1)?;
    writer.commit()?;
    writer.close()?;

    let query: Arc<dyn Query> = Arc::new(PrefixQuery::new(Term::from_text("default", "o"))?);

    // Set a fairly high timeout value (infinite) and expect the query to complete in that time
    // frame.
    // Not checking the validity of the result, all we are bothered about in this test is the timing
    // out.
    search_through(
        &directory.as_directory(),
        infinite_query_timeout(),
        &query,
        false,
    )?;

    // Set a really low timeout value (immediate) and expect an Exception
    search_through(
        &directory.as_directory(),
        immediate_query_timeout(),
        &query,
        true,
    )?;

    // Set maximum time out and expect the query to complete.
    // Not checking the validity of the result, all we are bothered about in this test is the timing
    // out.
    search_through(
        &directory.as_directory(),
        infinite_query_timeout(),
        &query,
        false,
    )?;
    directory.close()
}

/// Tests time out check sampling of TermsEnum iterations
///
/// <p>TODO: incredibly slow
///
/// @throws Exception on error
/// Port of `TestExitableDirectoryReader.testExitableTermsEnumSampleTimeoutCheck`.
fn test_exitable_terms_enum_sample_timeout_check() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(
            directory.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
                &*random(),
            )))),
        )?;
        let inner = (|| -> Result<()> {
            for i in 0..50 {
                let mut d1 = Document::new();
                d1.add(Box::new(LuceneTestCase::new_text_field(
                    "default",
                    &format!("term{i}"),
                    Store::YES,
                )));
                writer.add_document(&d1)?;
            }

            writer.force_merge(1)?;
            writer.commit()?;

            let query: Arc<dyn Query> =
                Arc::new(PrefixQuery::new(Term::from_text("default", "term"))?);

            // Set a fairly high timeout value (infinite) and expect the query to complete in that time
            // frame.
            // Not checking the validity of the result, but checking the sampling kicks in to reduce the
            // number of timeout check
            let query_timeout = Arc::new(CountingQueryTimeout::default());
            let directory_reader = directory_reader::open(directory.as_directory())?;
            let exitable_directory_reader = exitable(
                directory_reader,
                Arc::clone(&query_timeout) as Arc<dyn QueryTimeout>,
            )?;
            let reader = test_reader(exitable_directory_reader)?;
            let searcher = IndexSearcher::new(Arc::clone(&reader))?;
            searcher.search(query, 300)?;
            reader.close()?;
            // The number of sampled query time out check here depends on two factors:
            // 1. ExitableDirectoryReader.ExitableTermsEnum.NUM_CALLS_PER_TIMEOUT_CHECK
            // 2. MultiTermQueryConstantScoreWrapper.BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD
            assert_equals(&5, &query_timeout.get_should_exit_call_count());
            Ok(())
        })();
        let closed = writer.close();
        inner?;
        closed
    })();
    let closed = directory.close();
    result?;
    closed
}

/// Tests timing out of PointValues queries
///
/// @throws Exception on error
/// Port of `TestExitableDirectoryReader.testExitablePointValuesIndexReader`.
fn test_exitable_point_values_index_reader() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    for value in [10, 100, 1000] {
        let mut d = Document::new();
        d.add(Box::new(IntPoint::new("default", &[value])?));
        writer.add_document(&d)?;
    }

    writer.force_merge(1)?;
    writer.commit()?;
    writer.close()?;

    let query = IntPoint::new_range_query("default", &[10], &[20])?;

    // Set a fairly high timeout value (infinite) and expect the query to complete in that time
    // frame.
    // Not checking the validity of the result, all we are bothered about in this test is the timing
    // out.
    search_through(
        &directory.as_directory(),
        infinite_query_timeout(),
        &query,
        false,
    )?;

    // Set a really low timeout value (immediate) and expect an Exception
    search_through(
        &directory.as_directory(),
        immediate_query_timeout(),
        &query,
        true,
    )?;

    // Set maximum time out and expect the query to complete.
    // Not checking the validity of the result, all we are bothered about in this test is the timing
    // out.
    search_through(
        &directory.as_directory(),
        infinite_query_timeout(),
        &query,
        false,
    )?;
    directory.close()
}

/// The anonymous `ExitableDirectoryReader.ExitableTerms` of
/// `testExitableTermsMinAndMax`, whose `iterator()` fails.
struct MinMaxTerms {
    inner: ExitableTerms,
}

impl Terms for MinMaxTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Assert::fail("min and max should be retrieved from block tree, no need to iterate")
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
    /// The inherited `ExitableTerms.getMin()`.
    fn min(&self) -> Result<Option<BytesRef>> {
        self.inner.min()
    }
    /// The inherited `ExitableTerms.getMax()`.
    fn max(&self) -> Result<Option<BytesRef>> {
        self.inner.max()
    }
}

/// Port of `TestExitableDirectoryReader.testExitableTermsMinAndMax`.
fn test_exitable_terms_min_and_max() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    let mut doc = Document::new();
    let foo_field = StringField::new("foo", "bar".to_string(), Store::NO)?;
    doc.add(Box::new(foo_field));
    w.add_document(&doc)?;
    w.flush()?;

    let directory_reader = directory_reader::open_with_writer(&*w)?;
    for lfc in (Arc::clone(&directory_reader) as Arc<dyn IndexReader>).leaves()? {
        let terms = MinMaxTerms {
            inner: ExitableTerms::new(
                lfc.leaf_reader()
                    .terms("foo")?
                    .unwrap_or_else(|| crate::support::npe("terms")),
                infinite_query_timeout(),
            ),
        };
        assert_equals(
            "bar",
            terms
                .min()?
                .unwrap_or_else(|| crate::support::npe("min"))
                .utf8_to_string()?
                .as_str(),
        );
        assert_equals(
            "bar",
            terms
                .max()?
                .unwrap_or_else(|| crate::support::npe("max"))
                .utf8_to_string()?
                .as_str(),
        );
    }

    w.close()?;
    directory_reader.close()?;
    directory.close()
}

/// `@FunctionalInterface interface DvFactory`.
type DvFactory = fn(&dyn LeafReader) -> Result<Option<Box<dyn DocValuesIterator>>>;

/// Port of `TestExitableDirectoryReader.testDocValues`.
fn test_doc_values() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(
            directory.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
                &*random(),
            )))),
        )?;

        let mut d1 = Document::new();
        add_dvs(&mut d1, 10);
        writer.add_document(&d1)?;

        let mut d2 = Document::new();
        add_dvs(&mut d2, 100);
        writer.add_document(&d2)?;

        let mut d3 = Document::new();
        add_dvs(&mut d3, 1000);
        writer.add_document(&d3)?;

        writer.force_merge(1)?;
        writer.commit()?;
        writer.close()?;

        let factories: [DvFactory; 5] = [
            |r| {
                Ok(r.get_sorted_doc_values("sorted")?
                    .map(|v| v as Box<dyn DocValuesIterator>))
            },
            |r| {
                Ok(r.get_sorted_set_doc_values("sortedset")?
                    .map(|v| v as Box<dyn DocValuesIterator>))
            },
            |r| {
                Ok(r.get_sorted_numeric_doc_values("sortednumeric")?
                    .map(|v| v as Box<dyn DocValuesIterator>))
            },
            |r| {
                Ok(r.get_numeric_doc_values("numeric")?
                    .map(|v| v as Box<dyn DocValuesIterator>))
            },
            |r| {
                Ok(r.get_binary_doc_values("binary")?
                    .map(|v| v as Box<dyn DocValuesIterator>))
            },
        ];
        for dv_factory in factories {
            let directory_reader = directory_reader::open(directory.as_directory())?;
            let exitable_directory_reader = exitable(directory_reader, immediate_query_timeout())?;

            {
                let reader = test_reader(exitable_directory_reader)?;
                let r = (|| -> Result<()> {
                    expect_throws(EXITING, || -> Result<()> {
                        let leaf = first_leaf(&reader)?;
                        let iter = dv_factory(&*leaf)?;
                        scan(&*leaf, iter)
                    });
                    Ok(())
                })();
                let closed = reader.close();
                r?;
                closed?;
            }

            let directory_reader = directory_reader::open(directory.as_directory())?;
            let exitable_directory_reader = exitable(directory_reader, infinite_query_timeout())?;
            {
                let reader = test_reader(exitable_directory_reader)?;
                let r = (|| -> Result<()> {
                    let leaf = first_leaf(&reader)?;
                    scan(&*leaf, dv_factory(&*leaf)?)?;
                    assert_null(&leaf.get_numeric_doc_values("absent")?.map(|_| ()));
                    assert_null(&leaf.get_binary_doc_values("absent")?.map(|_| ()));
                    assert_null(&leaf.get_sorted_doc_values("absent")?.map(|_| ()));
                    assert_null(&leaf.get_sorted_numeric_doc_values("absent")?.map(|_| ()));
                    assert_null(&leaf.get_sorted_set_doc_values("absent")?.map(|_| ()));
                    Ok(())
                })();
                let closed = reader.close();
                r?;
                closed?;
            }
        }

        // Test that singleton docValues stay as singleton docValues after filtering
        let directory_reader = directory_reader::open(directory.as_directory())?;
        let reader = TestReader::new(LuceneTestCase::get_only_leaf_reader(
            directory_reader as Arc<dyn IndexReader>,
        )?)?;
        let exitable_reader = test_reader(exitable(
            directory_reader::open(directory.as_directory())?,
            infinite_query_timeout(),
        )?);
        let exitable_reader = match exitable_reader {
            Ok(r) => r,
            Err(e) => {
                let _ = reader.close();
                return Err(e);
            }
        };
        let r = (|| -> Result<()> {
            let leaf_reader = first_leaf(&reader)?;
            let exitable_leaf_reader = first_leaf(&exitable_reader)?;
            // `instanceof SingletonSortedSetDocValues` /
            // `SingletonSortedNumericDocValues`: the iterator is a singleton
            // view.
            if leaf_reader
                .get_sorted_set_doc_values("sortedset")?
                .is_some_and(|v| v.is_singleton())
            {
                Assert::assert_true_msg(
                    "Expected: an instance of SingletonSortedSetDocValues",
                    exitable_leaf_reader
                        .get_sorted_set_doc_values("sortedset")?
                        .is_some_and(|v| v.is_singleton()),
                );
            }
            if leaf_reader
                .get_sorted_numeric_doc_values("sortednumeric")?
                .is_some_and(|v| v.is_singleton())
            {
                Assert::assert_true_msg(
                    "Expected: an instance of SingletonSortedNumericDocValues",
                    exitable_leaf_reader
                        .get_sorted_numeric_doc_values("sortednumeric")?
                        .is_some_and(|v| v.is_singleton()),
                );
            }
            Ok(())
        })();
        let closed_exitable = exitable_reader.close();
        let closed = reader.close();
        r?;
        closed?;
        closed_exitable
    })();
    let closed = directory.close();
    result?;
    closed
}

/// `queryTimeout = random().nextBoolean() ? immediateQueryTimeout() :
/// infiniteQueryTimeout()`.
fn random_query_timeout() -> Arc<dyn QueryTimeout> {
    if random().next_boolean() {
        immediate_query_timeout()
    } else {
        infinite_query_timeout()
    }
}

/// Port of `TestExitableDirectoryReader.testFloatVectorValues`.
fn test_float_vector_values() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    let num_doc = LuceneTestCase::at_least(20);
    let deleted_doc = RandomizedTest::at_most(5);
    let dimension = LuceneTestCase::at_least(3);

    for i in 0..num_doc {
        let mut doc = Document::new();

        let mut value = vec![0f32; dimension as usize];
        for j in 0..dimension as usize {
            value[j] = random().next_float();
        }
        let field_type = KnnFloatVectorField::create_field_type(
            dimension,
            rucene::index::VectorSimilarityFunction::COSINE,
        )?;
        doc.add(Box::new(KnnFloatVectorField::with_field_type(
            "vector", &value, field_type,
        )?));

        doc.add(Box::new(StringField::new("id", i.to_string(), Store::YES)?));
        writer.add_document(&doc)?;
    }

    writer.force_merge(1)?;
    writer.commit()?;

    for i in 0..deleted_doc {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }

    writer.close()?;

    let query_timeout = random_query_timeout();

    let directory_reader = directory_reader::open(directory.as_directory())?;
    let exitable_directory_reader = exitable(directory_reader, Arc::clone(&query_timeout))?;
    let reader = test_reader(exitable_directory_reader)?;

    let leaf = first_leaf(&reader)?;

    if query_timeout.should_exit() {
        expect_throws(EXITING, || -> Result<()> {
            let values = leaf
                .get_float_vector_values("vector")?
                .unwrap_or_else(|| crate::support::npe("values"));
            scan_and_retrieve(&*leaf, &*values, Some(&*values))
        });

        expect_throws(EXITING, || {
            let target = random_vector(dimension);
            let mut accept_docs = from_live_docs(leaf.get_live_docs(), leaf.max_doc())?;
            leaf.search_nearest_vectors_default("vector", &target, 5, &mut accept_docs, i32::MAX)
        });
    } else {
        let values = leaf
            .get_float_vector_values("vector")?
            .unwrap_or_else(|| crate::support::npe("values"));
        scan_and_retrieve(&*leaf, &*values, Some(&*values))?;

        let target = random_vector(dimension);
        let mut accept_docs = from_live_docs(leaf.get_live_docs(), leaf.max_doc())?;
        leaf.search_nearest_vectors_default("vector", &target, 5, &mut accept_docs, i32::MAX)?;
    }

    reader.close()?;
    directory.close()
}

/// Port of `TestExitableDirectoryReader.testByteVectorValues`.
fn test_byte_vector_values() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    let num_doc = LuceneTestCase::at_least(20);
    let deleted_doc = RandomizedTest::at_most(5);
    let dimension = LuceneTestCase::at_least(3);

    for i in 0..num_doc {
        let mut doc = Document::new();
        let mut value = vec![0u8; dimension as usize];
        random().next_bytes(&mut value);
        doc.add(Box::new(KnnByteVectorField::new(
            "vector",
            &value,
            rucene::index::VectorSimilarityFunction::COSINE,
        )?));
        doc.add(Box::new(StringField::new("id", i.to_string(), Store::YES)?));
        writer.add_document(&doc)?;
    }

    writer.force_merge(1)?;
    writer.commit()?;

    for i in 0..deleted_doc {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }

    writer.close()?;

    let query_timeout = random_query_timeout();

    let directory_reader = directory_reader::open(directory.as_directory())?;
    let exitable_directory_reader = exitable(directory_reader, Arc::clone(&query_timeout))?;
    let reader = test_reader(exitable_directory_reader)?;

    let leaf = first_leaf(&reader)?;

    if query_timeout.should_exit() {
        expect_throws(EXITING, || -> Result<()> {
            let values = leaf
                .get_byte_vector_values("vector")?
                .unwrap_or_else(|| crate::support::npe("values"));
            scan_and_retrieve(&*leaf, &*values, None)
        });

        expect_throws(EXITING, || {
            let target = random_vector_bytes(dimension);
            let mut accept_docs = from_live_docs(leaf.get_live_docs(), leaf.max_doc())?;
            leaf.search_nearest_vectors_byte_default(
                "vector",
                &target,
                5,
                &mut accept_docs,
                i32::MAX,
            )
        });
    } else {
        let values = leaf
            .get_byte_vector_values("vector")?
            .unwrap_or_else(|| crate::support::npe("values"));
        scan_and_retrieve(&*leaf, &*values, None)?;

        let target = random_vector_bytes(dimension);
        let mut accept_docs = from_live_docs(leaf.get_live_docs(), leaf.max_doc())?;
        leaf.search_nearest_vectors_byte_default("vector", &target, 5, &mut accept_docs, i32::MAX)?;
    }

    reader.close()?;
    directory.close()
}

/// `TestVectorUtil.randomVector(dim)`.
fn random_vector(dim: i32) -> Vec<f32> {
    let random = random();
    (0..dim).map(|_| random.next_float()).collect()
}

/// `TestVectorUtil.randomVectorBytes(dim)`.
fn random_vector_bytes(dim: i32) -> Vec<u8> {
    let mut v = TestUtil::random_binary_term_with(&*random(), dim);
    for i in v.offset..v.offset + v.length {
        if v.bytes[i] as i8 == -128 {
            v.bytes[i] = (-127i8) as u8;
        }
    }
    v.bytes
}

/// `scanAndRetrieve(leaf, values)`; `float_values` is `values` when
/// `values instanceof FloatVectorValues`.
fn scan_and_retrieve(
    leaf: &dyn LeafReader,
    values: &dyn KnnVectorValues,
    float_values: Option<&dyn FloatVectorValues>,
) -> Result<()> {
    let mut iter = values.iterator()?;
    iter.next_doc()?;
    while iter.doc_id() != NO_MORE_DOCS && iter.doc_id() < leaf.max_doc() {
        let doc_id = iter.doc_id();
        if doc_id >= leaf.max_doc() {
            break;
        }
        let next_doc_id = doc_id + 1;
        if random().next_boolean() && next_doc_id < leaf.max_doc() {
            iter.advance(next_doc_id)?;
        } else {
            iter.next_doc()?;
        }
        if random().next_boolean() && iter.doc_id() != NO_MORE_DOCS {
            if let Some(float_values) = float_values {
                float_values.vector_value(iter.index())?;
            }
        }
    }
    Ok(())
}

/// `scan(leaf, iter)`.
fn scan(leaf: &dyn LeafReader, iter: Option<Box<dyn DocValuesIterator>>) -> Result<()> {
    let mut iter = iter.unwrap_or_else(|| crate::support::npe("iter"));
    iter.next_doc()?;
    while iter.doc_id() != NO_MORE_DOCS && iter.doc_id() < leaf.max_doc() {
        let next_doc_id = iter.doc_id() + 1;
        if random().next_boolean() && next_doc_id < leaf.max_doc() {
            if random().next_boolean() {
                iter.advance(next_doc_id)?;
            } else {
                iter.advance_exact(next_doc_id)?;
            }
        } else {
            iter.next_doc()?;
        }
    }
    Ok(())
}

/// `addDVs(d1, i)`.
fn add_dvs(d1: &mut Document, i: i32) {
    d1.add(Box::new(NumericDocValuesField::new("numeric", i as i64)));
    d1.add(Box::new(BinaryDocValuesField::new(
        "binary",
        BytesRef::from_char_sequence(&i.to_string()),
    )));
    d1.add(Box::new(SortedDocValuesField::new(
        "sorted",
        BytesRef::from_char_sequence(&i.to_string()),
    )));
    d1.add(Box::new(SortedNumericDocValuesField::new(
        "sortednumeric",
        i as i64,
    )));
    d1.add(Box::new(SortedSetDocValuesField::new(
        "sortedset",
        BytesRef::from_char_sequence(&i.to_string()),
    )));
}
