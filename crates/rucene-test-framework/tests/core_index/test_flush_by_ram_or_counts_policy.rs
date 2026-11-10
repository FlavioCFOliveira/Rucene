//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFlushByRamOrCountsPolicy.java`.
//!
//! * `MockDefaultFlushPolicy extends FlushByRamOrCountsPolicy` holds the
//!   policy it extends and calls it for `super.onChange`; its counters are
//!   atomics. `flushPolicy = (MockDefaultFlushPolicy)
//!   writer.getConfig().getFlushPolicy()` is the policy the test installed,
//!   which the config holds.
//! * The static `lineDocFile` lives in a process-wide cell.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::documents_writer::{
    DocumentsWriterFlushControl, DocumentsWriterPerThread, FlushByRamOrCountsPolicy,
    FlushControlHandle, FlushPolicy,
};
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::IndexReader;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::Throttling;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::{npe, static_hook};

/// `TestFlushByRamOrCountsPolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFlushByRamOrCountsPolicy",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testFlushByRam", test_flush_by_ram),
            test("testFlushByRamLargeBuffer", test_flush_by_ram_large_buffer),
            test("testFlushDocCount", test_flush_doc_count),
            test("testRandom", test_random),
            test("testStallControl", test_stall_control),
        ],
    )
}

/// `lineDocFile`.
static LINE_DOC_FILE: Mutex<Option<Arc<LineFileDocs>>> = Mutex::new(None);

fn line_doc_file() -> Arc<LineFileDocs> {
    LINE_DOC_FILE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| npe("lineDocFile"))
}

/// `@BeforeClass public static void beforeClass()`.
fn before_class() -> Result<()> {
    *LINE_DOC_FILE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Arc::new(LineFileDocs::with_default_file(&*random())?));
    Ok(())
}

/// `@AfterClass public static void afterClass()`.
fn after_class() -> Result<()> {
    let docs = LINE_DOC_FILE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_else(|| npe("lineDocFile"));
    docs.close();
    Ok(())
}

/// Port of `TestFlushByRamOrCountsPolicy.testFlushByRam`.
fn test_flush_by_ram() -> Result<()> {
    let ram_buffer = (if *TEST_NIGHTLY { 1 } else { 10 }) as f64
        + LuceneTestCase::at_least(2) as f64
        + random().next_double();
    run_flush_by_ram(
        1 + random().next_int_bound(if *TEST_NIGHTLY { 5 } else { 1 }),
        ram_buffer,
        false,
    )
}

/// Port of `TestFlushByRamOrCountsPolicy.testFlushByRamLargeBuffer`.
fn test_flush_by_ram_large_buffer() -> Result<()> {
    // with a 256 mb ram buffer we should never stall
    run_flush_by_ram(
        1 + random().next_int_bound(if *TEST_NIGHTLY { 5 } else { 1 }),
        256.0,
        true,
    )
}

/// Starts `num_threads` index threads and joins them.
fn run_index_threads(
    num_threads: i32,
    num_docs: &Arc<AtomicI32>,
    writer: &Arc<IndexWriter>,
    do_random_commit: bool,
) {
    let mut threads = Vec::with_capacity(num_threads as usize);
    for _ in 0..num_threads {
        let thread = index_thread(
            Arc::clone(num_docs),
            num_threads,
            Arc::clone(writer),
            line_doc_file(),
            do_random_commit,
        );
        thread.start();
        threads.push(thread);
    }

    for thread in &threads {
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
    }
}

/// `runFlushByRam(numThreads, maxRamMB, ensureNotStalled)`.
fn run_flush_by_ram(num_threads: i32, max_ram_mb: f64, ensure_not_stalled: bool) -> Result<()> {
    let num_documents_to_index = 10 + LuceneTestCase::at_least(30);
    let num_docs = Arc::new(AtomicI32::new(num_documents_to_index));
    let mut dir = LuceneTestCase::new_directory();
    let flush_policy = Arc::new(MockDefaultFlushPolicy::default());
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));

    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    iwc.set_flush_policy(Arc::clone(&flush_policy) as Arc<dyn FlushPolicy>)?;
    iwc.set_ram_buffer_size_mb(max_ram_mb)?;
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    let ram_buffer_size_mb = iwc.ram_buffer_size_mb();
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    Assert::assert_false(flush_policy.inner.flush_on_doc_count());
    Assert::assert_true(flush_policy.inner.flush_on_ram());
    let docs_writer = Arc::clone(writer.get_docs_writer());
    let flush_control = docs_writer.flush_control();
    assert_equals_msg(
        Some(" bytes must be 0 after init"),
        &0,
        &writer.get_flushing_bytes()?,
    );

    run_index_threads(num_threads, &num_docs, &writer, false);
    let max_ram_bytes = (ram_buffer_size_mb * 1024.0 * 1024.0) as i64;
    assert_equals_msg(
        Some(&format!(
            " all flushes must be due numThreads={num_threads}"
        )),
        &0,
        &writer.get_flushing_bytes()?,
    );
    assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.num_docs);
    assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.max_doc);
    Assert::assert_true_msg(
        "peak bytes without flush exceeded watermark",
        flush_policy.peak_bytes_without_flush.load(Ordering::SeqCst) <= max_ram_bytes,
    );
    assert_active_bytes_after(flush_control);
    if flush_policy.has_marked_pending.load(Ordering::SeqCst) {
        Assert::assert_true(max_ram_bytes < flush_control.get_peak_active_bytes());
    }
    if ensure_not_stalled {
        Assert::assert_false(flush_control.stall_control.was_stalled());
    }
    writer.close()?;
    assert_equals(&0, &flush_control.active_bytes());
    dir.close()
}

/// Port of `TestFlushByRamOrCountsPolicy.testFlushDocCount`.
fn test_flush_doc_count() -> Result<()> {
    let num_threads = [2 + LuceneTestCase::at_least(1), 1];
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    for i in 0..num_threads.len() {
        let num_documents_to_index = 50 + LuceneTestCase::at_least(30);
        let num_docs = Arc::new(AtomicI32::new(num_documents_to_index));
        let mut dir = LuceneTestCase::new_directory();
        let flush_policy = Arc::new(MockDefaultFlushPolicy::default());
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(
            Arc::clone(&analyzer) as Arc<dyn rucene::analysis::Analyzer>
        ));
        iwc.set_flush_policy(Arc::clone(&flush_policy) as Arc<dyn FlushPolicy>)?;

        iwc.set_max_buffered_docs(2 + LuceneTestCase::at_least(10))?;
        iwc.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)?;
        let max_buffered_docs = iwc.max_buffered_docs();
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        Assert::assert_true(flush_policy.inner.flush_on_doc_count());
        Assert::assert_false(flush_policy.inner.flush_on_ram());
        let docs_writer = Arc::clone(writer.get_docs_writer());
        let flush_control = docs_writer.flush_control();
        assert_equals_msg(
            Some(" bytes must be 0 after init"),
            &0,
            &writer.get_flushing_bytes()?,
        );

        run_index_threads(num_threads[i], &num_docs, &writer, false);

        assert_equals_msg(
            Some(&format!(
                " all flushes must be due numThreads={}",
                num_threads[i]
            )),
            &0,
            &writer.get_flushing_bytes()?,
        );
        assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.num_docs);
        assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.max_doc);
        Assert::assert_true_msg(
            "peak bytes without flush exceeded watermark",
            flush_policy
                .peak_doc_count_without_flush
                .load(Ordering::SeqCst)
                <= max_buffered_docs as i64,
        );
        assert_active_bytes_after(flush_control);
        writer.close()?;
        assert_equals(&0, &flush_control.active_bytes());
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestFlushByRamOrCountsPolicy.testRandom`.
fn test_random() -> Result<()> {
    let num_threads = 1 + random().next_int_bound(8);
    let num_documents_to_index = 50 + LuceneTestCase::at_least(70);
    let num_docs = Arc::new(AtomicI32::new(num_documents_to_index));
    let mut dir = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    let flush_policy = Arc::new(MockDefaultFlushPolicy::default());
    iwc.set_flush_policy(Arc::clone(&flush_policy) as Arc<dyn FlushPolicy>)?;
    let ram_buffer_size_mb = iwc.ram_buffer_size_mb();

    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let docs_writer = Arc::clone(writer.get_docs_writer());
    let flush_control = docs_writer.flush_control();

    assert_equals_msg(
        Some(" bytes must be 0 after init"),
        &0,
        &writer.get_flushing_bytes()?,
    );

    run_index_threads(num_threads, &num_docs, &writer, true);
    assert_equals_msg(
        Some(" all flushes must be due"),
        &0,
        &writer.get_flushing_bytes()?,
    );
    assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.num_docs);
    assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.max_doc);
    if flush_policy.inner.flush_on_ram() && !flush_policy.inner.flush_on_doc_count() {
        let max_ram_bytes = (ram_buffer_size_mb * 1024.0 * 1024.0) as i64;
        Assert::assert_true_msg(
            "peak bytes without flush exceeded watermark",
            flush_policy.peak_bytes_without_flush.load(Ordering::SeqCst) <= max_ram_bytes,
        );
        if flush_policy.has_marked_pending.load(Ordering::SeqCst) {
            Assert::assert_true_msg(
                &format!(
                    "max: {max_ram_bytes} {}",
                    flush_control.get_peak_active_bytes()
                ),
                max_ram_bytes <= flush_control.get_peak_active_bytes(),
            );
        }
    }
    assert_active_bytes_after(flush_control);
    writer.commit()?;
    assert_equals(&0, &flush_control.active_bytes());
    let r = directory_reader::open(dir.as_directory())?;
    assert_equals(&num_documents_to_index, &r.num_docs());
    assert_equals(&num_documents_to_index, &r.max_doc());
    if !flush_policy.inner.flush_on_ram() {
        Assert::assert_false_msg(
            "never stall if we don't flush on RAM",
            flush_control.stall_control.was_stalled(),
        );
        Assert::assert_false_msg(
            "never block if we don't flush on RAM",
            flush_control.stall_control.has_blocked(),
        );
    }
    (r as Arc<dyn IndexReader>).close()?;
    writer.close()?;
    dir.close()
}

/// Port of `TestFlushByRamOrCountsPolicy.testStallControl`.
fn test_stall_control() -> Result<()> {
    let num_threads = [4 + random().next_int_bound(8), 1];
    let num_documents_to_index = 50 + random().next_int_bound(50);
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    for i in 0..num_threads.len() {
        let num_docs = Arc::new(AtomicI32::new(num_documents_to_index));
        let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
        // mock a very slow harddisk sometimes here so that flushing is very slow
        dir.as_mock()
            .unwrap_or_else(|| npe("dir"))
            .set_throttling(Throttling::Sometimes);
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(
            Arc::clone(&analyzer) as Arc<dyn rucene::analysis::Analyzer>
        ));
        iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
        let flush_policy: Arc<dyn FlushPolicy> = Arc::new(FlushByRamOrCountsPolicy::new());
        iwc.set_flush_policy(flush_policy)?;

        // with such a small ram buffer we should be stalled quite quickly
        iwc.set_ram_buffer_size_mb(0.25)?;
        let ram_buffer_size_mb = iwc.ram_buffer_size_mb();
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        run_index_threads(num_threads[i], &num_docs, &writer, false);
        let docs_writer = Arc::clone(writer.get_docs_writer());
        let flush_control = docs_writer.flush_control();
        assert_equals_msg(
            Some(" all flushes must be due"),
            &0,
            &writer.get_flushing_bytes()?,
        );
        assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.num_docs);
        assert_equals(&num_documents_to_index, &writer.get_doc_stats()?.max_doc);
        if num_threads[i] == 1 {
            Assert::assert_false_msg(
                &format!(
                    "single thread must not block numThreads: {}",
                    num_threads[i]
                ),
                flush_control.stall_control.has_blocked(),
            );
        }
        if flush_control.get_peak_net_bytes() as f64 > 2.0 * ram_buffer_size_mb * 1024.0 * 1024.0 {
            Assert::assert_true(flush_control.stall_control.was_stalled());
        }
        assert_active_bytes_after(flush_control);
        writer.close()?;
        dir.close()?;
    }
    Ok(())
}

/// `assertActiveBytesAfter(flushControl)`.
fn assert_active_bytes_after(flush_control: &DocumentsWriterFlushControl) {
    let mut bytes_used = 0i64;
    for next in flush_control.all_active_writers() {
        bytes_used += DocumentsWriterPerThread::lock(&next).ram_bytes_used();
    }
    assert_equals(&bytes_used, &flush_control.active_bytes());
}

/// `public static class IndexThread extends Thread`.
fn index_thread(
    pending_docs: Arc<AtomicI32>,
    _num_threads: i32,
    writer: Arc<IndexWriter>,
    docs: Arc<LineFileDocs>,
    do_random_commit: bool,
) -> Thread {
    Thread::unnamed(move || {
        let result = (|| -> Result<()> {
            let mut ram_size = 0i64;
            while pending_docs.fetch_sub(1, Ordering::SeqCst) - 1 > -1 {
                let doc = docs.next_doc()?;
                writer.add_document(&doc)?;
                let new_ram_size = writer.ram_bytes_used()?;
                if new_ram_size != ram_size {
                    ram_size = new_ram_size;
                }
                if do_random_commit && LuceneTestCase::rarely() {
                    writer.commit()?;
                }
            }
            writer.commit()?;
            Ok(())
        })();
        if let Err(ex) = result {
            println!("FAILED exc:");
            println!("{ex}");
            rethrow_java(LuceneError::runtime_exception(ex));
        }
    })
}

/// `private static class MockDefaultFlushPolicy extends
/// FlushByRamOrCountsPolicy`.
#[derive(Debug)]
struct MockDefaultFlushPolicy {
    /// The `FlushByRamOrCountsPolicy` it extends (`super`).
    inner: FlushByRamOrCountsPolicy,
    /// `indexWriterConfig`, which `FlushPolicy.init` sets.
    index_writer_config: OnceLock<Weak<LiveIndexWriterConfig>>,
    peak_bytes_without_flush: AtomicI64,
    peak_doc_count_without_flush: AtomicI64,
    has_marked_pending: AtomicBool,
}

impl Default for MockDefaultFlushPolicy {
    fn default() -> Self {
        MockDefaultFlushPolicy {
            inner: FlushByRamOrCountsPolicy::new(),
            index_writer_config: OnceLock::new(),
            peak_bytes_without_flush: AtomicI64::new(i32::MIN as i64),
            peak_doc_count_without_flush: AtomicI64::new(i32::MIN as i64),
            has_marked_pending: AtomicBool::new(false),
        }
    }
}

impl MockDefaultFlushPolicy {
    fn index_writer_config(&self) -> Arc<LiveIndexWriterConfig> {
        self.index_writer_config
            .get()
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| npe("indexWriterConfig"))
    }
}

/// `list.remove(o)`: removes the first element that is `o`, by identity.
fn remove(list: &mut Vec<Arc<DocumentsWriterPerThread>>, o: &DocumentsWriterPerThread) -> bool {
    match list.iter().position(|e| std::ptr::eq(Arc::as_ptr(e), o)) {
        Some(i) => {
            list.remove(i);
            true
        }
        None => false,
    }
}

impl FlushPolicy for MockDefaultFlushPolicy {
    fn on_change(
        &self,
        control: &mut FlushControlHandle<'_>,
        per_thread: Option<&DocumentsWriterPerThread>,
    ) {
        let dwpt = per_thread.unwrap_or_else(|| npe("dwpt"));
        let mut pending = Vec::new();
        let mut not_pending = Vec::new();
        find_pending(control, &mut pending, &mut not_pending);
        let flush_current = dwpt.is_flush_pending();
        let active_bytes = control.active_bytes();
        let config = self.index_writer_config();
        // `toFlush`: `dwpt` itself, or the largest non-pending writer.
        let largest: Option<Arc<DocumentsWriterPerThread>>;
        let to_flush: Option<&DocumentsWriterPerThread> = if dwpt.is_flush_pending() {
            Some(dwpt)
        } else if self.inner.flush_on_doc_count()
            && dwpt.num_docs_in_ram() >= config.max_buffered_docs()
        {
            Some(dwpt)
        } else if self.inner.flush_on_ram()
            && active_bytes >= (config.ram_buffer_size_mb() * 1024.0 * 1024.0) as i64
        {
            largest = self.find_largest_non_pending_writer(control, dwpt);
            let to_flush = largest.as_deref().unwrap_or_else(|| npe("toFlush"));
            Assert::assert_false(to_flush.is_flush_pending());
            Some(to_flush)
        } else {
            None
        };
        self.inner.on_change(control, Some(dwpt));
        match to_flush {
            Some(to_flush) => {
                if flush_current {
                    Assert::assert_true(remove(&mut pending, to_flush));
                } else {
                    Assert::assert_true(remove(&mut not_pending, to_flush));
                }
                Assert::assert_true(to_flush.is_flush_pending());
                self.has_marked_pending.store(true, Ordering::SeqCst);
            }
            None => {
                self.peak_bytes_without_flush
                    .fetch_max(active_bytes, Ordering::SeqCst);
                self.peak_doc_count_without_flush
                    .fetch_max(dwpt.num_docs_in_ram() as i64, Ordering::SeqCst);
            }
        }

        for per_thread in &not_pending {
            Assert::assert_false(per_thread.is_flush_pending());
        }
    }

    fn init(&self, config: Arc<LiveIndexWriterConfig>) {
        let _ = self.index_writer_config.set(Arc::downgrade(&config));
        self.inner.init(config);
    }
}

/// `findPending(flushControl, pending, notPending)`.
fn find_pending(
    flush_control: &FlushControlHandle<'_>,
    pending: &mut Vec<Arc<DocumentsWriterPerThread>>,
    not_pending: &mut Vec<Arc<DocumentsWriterPerThread>>,
) {
    for next in flush_control.all_active_writers() {
        if next.is_flush_pending() {
            pending.push(next);
        } else {
            not_pending.push(next);
        }
    }
}
