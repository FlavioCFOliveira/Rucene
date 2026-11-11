//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFilterDirectoryReader.java`.
//!
//! * A `FilterDirectoryReader` subclass is a `FilterDirectoryReader` built
//!   with the `DirectoryReaderWrapper` that stands for it
//!   (`src/index/filter_directory_reader.rs`): its `doWrapDirectoryReader`
//!   is the wrapper's `wrap`, and its choice of `getReaderCacheHelper()`
//!   (`in.getReaderCacheHelper()`, or a `DelegatingCacheHelper` built once) is
//!   the wrapper's `uses_delegating_cache_helper`.

use std::fmt;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::{CacheHelper, CacheKey, ClosedListener};
use rucene::index::index_writer::IndexWriter;
use rucene::index::LeafReader;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestFilterDirectoryReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFilterDirectoryReader",
        vec![
            test("testDoubleClose", test_double_close),
            test(
                "testFilterDirectoryReaderNumDocsIsLazy",
                test_filter_directory_reader_num_docs_is_lazy,
            ),
            test("testDelegatingCacheHelper", test_delegating_cache_helper),
        ],
    )
}

/// `private static class DummySubReaderWrapper extends SubReaderWrapper`.
struct DummySubReaderWrapper;

impl SubReaderWrapper for DummySubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(reader)
    }
}

/// `private static class DummyFilterDirectoryReader extends
/// FilterDirectoryReader`: its `getReaderCacheHelper()` is
/// `in.getReaderCacheHelper()`.
struct DummyFilterDirectoryReader;

impl DummyFilterDirectoryReader {
    /// `new DummyFilterDirectoryReader(in)`.
    fn new(r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        Ok(Arc::new(FilterDirectoryReader::new(
            r#in,
            Arc::new(DummySubReaderWrapper),
            Arc::new(DummyFilterDirectoryReader),
        )?))
    }
}

impl DirectoryReaderWrapper for DummyFilterDirectoryReader {
    /// `doWrapDirectoryReader(in)`.
    fn wrap(&self, r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        DummyFilterDirectoryReader::new(r#in)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "DummyFilterDirectoryReader"
    }
}

/// `IOUtils.close(readers)`: closes every reader, then rethrows the first
/// failure.
fn close_all(readers: &[Arc<dyn DirectoryReader>]) -> Result<()> {
    let mut first: Option<LuceneError> = None;
    for r in readers {
        if let Err(e) = r.close() {
            first.get_or_insert(e);
        }
    }
    first.map_or(Ok(()), Err)
}

/// Port of `TestFilterDirectoryReader.testDoubleClose`.
fn test_double_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    w.add_document(&Document::new())?;

    let reader = directory_reader::open_with_writer(&*w)?;
    let wrapped = DummyFilterDirectoryReader::new(Arc::clone(&reader))?;

    // Calling close() on the original reader and wrapped reader should only close
    // the original reader once (as per Closeable.close() contract that close() is
    // idempotent)
    let mut readers = vec![reader, wrapped];
    shuffle(&mut readers, &*random());
    close_all(&readers)?;

    w.close()?;
    dir.close()
}

/// The anonymous `FilterLeafReader` of `NumDocsCountingSubReaderWrapper`.
struct NumDocsCountingLeafReader {
    inner: FilterLeafReader,
    num_docs_call_count: Arc<AtomicI64>,
}

impl fmt::Debug for NumDocsCountingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NumDocsCountingLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for NumDocsCountingLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for NumDocsCountingLeafReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, do_close, term_vectors, max_doc,
        stored_fields, terms, get_numeric_doc_values, get_binary_doc_values,
        get_sorted_doc_values, get_sorted_numeric_doc_values, get_sorted_set_doc_values,
        get_norm_values, get_doc_values_skipper, get_float_vector_values,
        get_byte_vector_values, search_nearest_vectors, search_nearest_vectors_byte,
        get_field_infos, get_live_docs, get_point_values, check_integrity, get_meta_data);

    fn num_docs(&self) -> i32 {
        self.num_docs_call_count.fetch_add(1, Ordering::SeqCst);
        (&self.inner as &dyn LeafReader).num_docs()
    }

    /// `in.getCoreCacheHelper()`.
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }

    /// `in.getReaderCacheHelper()`.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }
}

/// `private static class NumDocsCountingSubReaderWrapper extends
/// SubReaderWrapper`.
struct NumDocsCountingSubReaderWrapper {
    num_docs_call_count: Arc<AtomicI64>,
}

impl SubReaderWrapper for NumDocsCountingSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(NumDocsCountingLeafReader {
            inner: FilterLeafReader::new(reader)?,
            num_docs_call_count: Arc::clone(&self.num_docs_call_count),
        }))
    }
}

/// `private static class NumDocsCountingFilterDirectoryReader extends
/// FilterDirectoryReader`: its `getReaderCacheHelper()` is
/// `in.getReaderCacheHelper()`.
struct NumDocsCountingFilterDirectoryReader {
    num_docs_call_count: Arc<AtomicI64>,
}

impl NumDocsCountingFilterDirectoryReader {
    /// `new NumDocsCountingFilterDirectoryReader(in, numDocsCallCount)`.
    fn new(
        r#in: Arc<dyn DirectoryReader>,
        num_docs_call_count: Arc<AtomicI64>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        Ok(Arc::new(FilterDirectoryReader::new(
            r#in,
            Arc::new(NumDocsCountingSubReaderWrapper {
                num_docs_call_count: Arc::clone(&num_docs_call_count),
            }),
            Arc::new(NumDocsCountingFilterDirectoryReader {
                num_docs_call_count,
            }),
        )?))
    }
}

impl DirectoryReaderWrapper for NumDocsCountingFilterDirectoryReader {
    /// `doWrapDirectoryReader(in)`.
    fn wrap(&self, r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        NumDocsCountingFilterDirectoryReader::new(r#in, Arc::clone(&self.num_docs_call_count))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "NumDocsCountingFilterDirectoryReader"
    }
}

/// Port of `TestFilterDirectoryReader.testFilterDirectoryReaderNumDocsIsLazy`.
fn test_filter_directory_reader_num_docs_is_lazy() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    w.add_document(&Document::new())?;
    let directory_reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;

    let num_docs_call_count = Arc::new(AtomicI64::new(0));
    let directory_reader_wrapper = NumDocsCountingFilterDirectoryReader::new(
        Arc::clone(&directory_reader),
        Arc::clone(&num_docs_call_count),
    )?;
    assert_equals(&0i64, &num_docs_call_count.load(Ordering::SeqCst));
    assert_equals(&1, &directory_reader_wrapper.num_docs());
    assert_equals(&1i64, &num_docs_call_count.load(Ordering::SeqCst)); // one segment, so called once
    assert_equals(&1, &directory_reader_wrapper.num_docs());
    assert_equals(&1i64, &num_docs_call_count.load(Ordering::SeqCst));

    directory_reader.close()?;
    dir.close()
}

/// `private static class DummyLastingFilterDirectoryReader extends
/// FilterDirectoryReader`: its `getReaderCacheHelper()` is the
/// `DelegatingCacheHelper` it builds once over `in.getReaderCacheHelper()`.
struct DummyLastingFilterDirectoryReader;

impl DummyLastingFilterDirectoryReader {
    /// `new DummyLastingFilterDirectoryReader(in)`.
    fn new(r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        Ok(Arc::new(FilterDirectoryReader::new(
            r#in,
            Arc::new(DummySubReaderWrapper),
            Arc::new(DummyLastingFilterDirectoryReader),
        )?))
    }
}

impl DirectoryReaderWrapper for DummyLastingFilterDirectoryReader {
    /// `doWrapDirectoryReader(in)`: `new DummyFilterDirectoryReader(in)`.
    fn wrap(&self, r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        DummyFilterDirectoryReader::new(r#in)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "DummyLastingFilterDirectoryReader"
    }

    fn uses_delegating_cache_helper(&self) -> bool {
        true
    }
}

/// The closed listener of `testDelegatingCacheHelper`:
/// `closeCalledCounter.incrementAndGet()`, then
/// `assertSame(key, wrapped.getReaderCacheHelper().getKey())`.
///
/// Java's lambda reads `wrapped.getReaderCacheHelper().getKey()` when it runs;
/// the listener holds the key `wrapped`'s helper answered when it was
/// registered, the same key (the helper is built once), so that the listener
/// does not own `wrapped`, which owns the reader whose listeners own it.
struct CountingListener {
    close_called_counter: Arc<AtomicI32>,
    wrapped_key: CacheKey,
}

impl ClosedListener for CountingListener {
    fn on_close(&self, key: CacheKey) -> Result<()> {
        self.close_called_counter.fetch_add(1, Ordering::SeqCst);
        // `assertSame(key, wrapped.getReaderCacheHelper().getKey())`.
        Assert::assert_true(key == self.wrapped_key);
        Ok(())
    }
}

/// Port of `TestFilterDirectoryReader.testDelegatingCacheHelper`.
fn test_delegating_cache_helper() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    w.add_document(&Document::new())?;

    let reader = directory_reader::open_with_writer(&*w)?;
    let wrapped = DummyLastingFilterDirectoryReader::new(Arc::clone(&reader))?;

    let reader_helper = reader
        .get_reader_cache_helper()
        .unwrap_or_else(|| crate::support::npe("reader cache helper"));
    let wrapped_helper = wrapped
        .get_reader_cache_helper()
        .unwrap_or_else(|| crate::support::npe("wrapped cache helper"));
    // `assertNotEquals(reader.getReaderCacheHelper(), wrapped.getReaderCacheHelper())`.
    Assert::assert_false(std::ptr::eq(
        &*reader_helper as *const dyn CacheHelper as *const (),
        &*wrapped_helper as *const dyn CacheHelper as *const (),
    ));
    Assert::assert_false(reader_helper.get_key() == wrapped_helper.get_key());

    let close_called_counter = Arc::new(AtomicI32::new(0));

    wrapped_helper.add_closed_listener(Box::new(CountingListener {
        close_called_counter: Arc::clone(&close_called_counter),
        wrapped_key: wrapped
            .get_reader_cache_helper()
            .unwrap_or_else(|| crate::support::npe("wrapped cache helper"))
            .get_key()
            .clone(),
    }));

    reader.close()?;
    assert_equals(&1, &close_called_counter.load(Ordering::SeqCst));
    wrapped.close()?;
    assert_equals(&1, &close_called_counter.load(Ordering::SeqCst));

    w.close()?;
    dir.close()
}
