//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexReaderClose.java`.
//!
//! * `CountListener.onClose` cannot `assertSame(this.coreCacheKey,
//!   coreCacheKey)`: Rucene's `ClosedListener.on_close` receives the key by
//!   value, a zero-sized `CacheKey` with no identity.
//! * `CacheHelper.addClosedListener` on a closed reader throws Java's
//!   unchecked `AlreadyClosedException`; Rucene's returns `()`, so the
//!   expectation is a panic with that throwable (`expect_throws_panic`).
//! * `testCoreListenerOnWrapperWithDifferentCacheKey` registers 100 more times
//!   listeners already registered, which Java's listener set ignores; only the
//!   draws are made (a boxed listener has no identity to register again).

use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::{CacheHelper, CacheKey, ClosedListener};
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, LeafReader};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::{AssertingLeafReader, RandomIndexWriter};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, expect_throws_panic, random, test};
use crate::support::npe;

const ISE: &str = "java.lang.IllegalStateException";
const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";

/// `TestIndexReaderClose`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexReaderClose",
        vec![
            test("testCloseUnderException", test_close_under_exception),
            test(
                "testCoreListenerOnWrapperWithDifferentCacheKey",
                test_core_listener_on_wrapper_with_different_cache_key,
            ),
            test(
                "testRegisterListenerOnClosedReader",
                test_register_listener_on_closed_reader,
            ),
        ],
    )
}

/// The anonymous `FilterLeafReader` of `testCloseUnderException`.
struct ThrowOnCloseReader {
    inner: FilterLeafReader,
    throw_on_close: bool,
}

impl fmt::Debug for ThrowOnCloseReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThrowOnCloseReader").finish_non_exhaustive()
    }
}

impl fmt::Display for ThrowOnCloseReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for ThrowOnCloseReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, term_vectors, num_docs, max_doc,
        stored_fields, terms, get_numeric_doc_values, get_binary_doc_values,
        get_sorted_doc_values, get_sorted_numeric_doc_values, get_sorted_set_doc_values,
        get_norm_values, get_doc_values_skipper, get_float_vector_values,
        get_byte_vector_values, search_nearest_vectors, search_nearest_vectors_byte,
        get_field_infos, get_live_docs, get_point_values, check_integrity, get_meta_data);

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }

    /// `try { super.doClose(); } finally { if (throwOnClose) throw new
    /// IllegalStateException("BOOM!"); }`.
    fn do_close(&self) -> Result<()> {
        let closed = LeafReader::do_close(&self.inner);
        if self.throw_on_close {
            return Err(LuceneError::illegal_state("BOOM!"));
        }
        closed
    }
}

/// Port of `TestIndexReaderClose.testCloseUnderException`.
fn test_close_under_exception() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_with(
            &*random(),
            Arc::new(MockAnalyzer::new(&*random())),
        ),
    )?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    writer.close()?;
    let iters = 1000 + 1 + random().next_int_bound(20);
    for _ in 0..iters {
        let open = directory_reader::open(dir.as_directory())?;
        let throw_on_close = !LuceneTestCase::rarely();
        let leaf = LuceneTestCase::get_only_leaf_reader(open as Arc<dyn IndexReader>)?;
        let reader: Arc<dyn LeafReader> = Arc::new(ThrowOnCloseReader {
            inner: FilterLeafReader::new(leaf)?,
            throw_on_close,
        });
        let helper = || {
            reader
                .get_reader_cache_helper()
                .unwrap_or_else(|| npe("getReaderCacheHelper()"))
        };
        let listener_count = random().next_int_bound(20);
        let count = Arc::new(AtomicI32::new(0));
        let mut faulty_set = false;
        for _ in 0..listener_count {
            if LuceneTestCase::rarely() {
                faulty_set = true;
                helper().add_closed_listener(Box::new(FaultyListener));
            } else {
                count.fetch_add(1, Ordering::SeqCst);
                helper().add_closed_listener(Box::new(CountListener {
                    count: Arc::clone(&count),
                }));
            }
        }
        if !faulty_set && !throw_on_close {
            helper().add_closed_listener(Box::new(FaultyListener));
        }

        let index_reader = Arc::clone(&reader).as_index_reader();
        let expected = expect_throws(ISE, || index_reader.close());

        if throw_on_close {
            assert_equals(&Some("BOOM!"), &expected.message());
        } else {
            assert_equals(&Some("GRRRRRRRRRRRR!"), &expected.message());
        }

        expect_throws(ACE, || reader.terms("someField"));

        if random().next_boolean() {
            let _ = index_reader.close(); // call it again
        }
        assert_equals(&0, &count.load(Ordering::SeqCst));
    }
    dir.close()
}

/// Port of `TestIndexReaderClose.testCoreListenerOnWrapperWithDifferentCacheKey`.
fn test_core_listener_on_wrapper_with_different_cache_key() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = TestUtil::next_int(&*random(), 1, 5);
    for _ in 0..num_docs {
        w.add_document(&Document::new())?;
        if random().next_boolean() {
            w.commit()?;
        }
    }
    w.force_merge(1)?;
    w.commit()?;
    w.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(w.w().get_directory()))?;
    let leaf_reader: Arc<dyn LeafReader> = Arc::new(AssertingLeafReader::new(
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader))?,
    )?);
    let core_helper = || {
        leaf_reader
            .get_core_cache_helper()
            .unwrap_or_else(|| npe("getCoreCacheHelper()"))
    };

    let num_listeners = TestUtil::next_int(&*random(), 1, 10);
    let counter = Arc::new(AtomicI32::new(num_listeners));

    for _ in 0..num_listeners {
        core_helper().add_closed_listener(Box::new(CountListener {
            count: Arc::clone(&counter),
        }));
    }
    for _ in 0..100 {
        // `addClosedListener(listeners.get(random().nextInt(listeners.size())))`
        // registers a listener that is already registered, which Java's
        // listener set ignores; a boxed Rust listener has no identity to
        // register again, so only the draw is made.
        let _ = random().next_int_bound(num_listeners);
    }
    assert_equals(&num_listeners, &counter.load(Ordering::SeqCst));
    // make sure listeners are registered on the wrapped reader and that closing any of them has the
    // same effect
    if random().next_boolean() {
        reader.close()?;
    } else {
        Arc::clone(&leaf_reader).as_index_reader().close()?;
    }
    assert_equals(&0, &counter.load(Ordering::SeqCst));
    drop(w);
    dir.close()
}

/// `private static final class CountListener implements
/// IndexReader.ClosedListener`.
struct CountListener {
    count: Arc<AtomicI32>,
}

impl ClosedListener for CountListener {
    fn on_close(&self, _core_cache_key: CacheKey) -> Result<()> {
        self.count.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `private static final class FaultyListener implements
/// IndexReader.ClosedListener`.
struct FaultyListener;

impl ClosedListener for FaultyListener {
    fn on_close(&self, _cache_key: CacheKey) -> Result<()> {
        Err(LuceneError::illegal_state("GRRRRRRRRRRRR!"))
    }
}

/// `key -> {}`.
struct NoOpListener;

impl ClosedListener for NoOpListener {
    fn on_close(&self, _key: CacheKey) -> Result<()> {
        Ok(())
    }
}

/// Port of `TestIndexReaderClose.testRegisterListenerOnClosedReader`.
fn test_register_listener_on_closed_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    w.add_document(&Document::new())?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;

    let first_leaf = || -> Arc<dyn LeafReader> {
        (Arc::clone(&r) as Arc<dyn IndexReader>)
            .leaves()
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))[0]
            .leaf_reader()
    };

    // The reader is open, everything should work
    r.get_reader_cache_helper()
        .unwrap_or_else(|| npe("getReaderCacheHelper()"))
        .add_closed_listener(Box::new(NoOpListener));
    first_leaf()
        .get_reader_cache_helper()
        .unwrap_or_else(|| npe("getReaderCacheHelper()"))
        .add_closed_listener(Box::new(NoOpListener));
    first_leaf()
        .get_core_cache_helper()
        .unwrap_or_else(|| npe("getCoreCacheHelper()"))
        .add_closed_listener(Box::new(NoOpListener));
    let leaf = first_leaf();

    // But now we close
    r.close()?;
    expect_throws_panic(ACE, || {
        r.get_reader_cache_helper()
            .unwrap_or_else(|| npe("getReaderCacheHelper()"))
            .add_closed_listener(Box::new(NoOpListener))
    });
    expect_throws_panic(ACE, || {
        leaf.get_reader_cache_helper()
            .unwrap_or_else(|| npe("getReaderCacheHelper()"))
            .add_closed_listener(Box::new(NoOpListener))
    });
    expect_throws_panic(ACE, || {
        leaf.get_core_cache_helper()
            .unwrap_or_else(|| npe("getCoreCacheHelper()"))
            .add_closed_listener(Box::new(NoOpListener))
    });

    dir.close()
}
