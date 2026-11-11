//! Port of `lucene/core/src/test/org/apache/lucene/index/TestConcurrentMergeScheduler.java`.
//!
//! Java subclasses `ConcurrentMergeScheduler` (anonymously or as
//! `TrackingCMS`) to override `doMerge`, `getIntraMergeExecutor`,
//! `maybeStall`, `doStall` and `getMergeThread`, and its inner `MergeThread`
//! to override `run()`; Rucene's scheduler takes those overrides as hooks
//! (`set_*_override`, `MergeThread::with_run`), declared at
//! `rucene::index::merge_scheduler::ConcurrentMergeScheduler`. State a Java
//! subclass keeps in fields (`TrackingCMS.totMergedBytes`, the anonymous
//! classes' counters) lives in values the hooks share with the test.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::generators::RandomStrings;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::TokenStream;
use rucene::document::{Document, Field, KnnFloatVectorField, Store, StringField, TextField};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{
    LogMergePolicy, MergeContext, MergePolicy, MergeSpecification, MergeTrigger, NoMergePolicy,
    OneMerge, TieredMergePolicy,
};
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, MergeSource, MergeThread,
    AUTO_DETECT_MERGES_AND_THREADS,
};
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::index::{SegmentCommitInfo, SegmentInfo};
use rucene::search::Sort;
use rucene::util::concurrent::SameThreadExecutorService;
use rucene::util::string_helper::StringHelper;
use rucene::util::{InfoStream, NoOutputInfoStream, Version};
use rucene_test_framework::analysis::{CannedTokenStream, MockAnalyzer};
use rucene_test_framework::index::suppressing_concurrent_merge_scheduler::SuppressingConcurrentMergeScheduler;
use rucene_test_framework::store::{Failure, MockDirectoryWrapper};
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, verbose, IAE};
use crate::support::{
    as_cms, assert_no_unreferenced_files, field_mut, open_reader, CountDownLatch,
};

/// `TestConcurrentMergeScheduler`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestConcurrentMergeScheduler",
        vec![
            test("testFlushExceptions", test_flush_exceptions),
            test("testDeleteMerging", test_delete_merging),
            test("testNoExtraFiles", test_no_extra_files),
            test("testNoWaitClose", test_no_wait_close),
            test("testMaxMergeCount", test_max_merge_count),
            test(
                "testSmallMergesDonNotGetThreads",
                test_small_merges_don_not_get_threads,
            ),
            test(
                "testIntraMergeThreadPoolIsLimitedByMaxThreads",
                test_intra_merge_thread_pool_is_limited_by_max_threads,
            ),
            test("testTotalBytesSize", test_total_bytes_size),
            test(
                "testInvalidMaxMergeCountAndThreads",
                test_invalid_max_merge_count_and_threads,
            ),
            test("testLiveMaxMergeCount", test_live_max_merge_count),
            test("testMaybeStallCalled", test_maybe_stall_called),
            test("testHangDuringRollback", test_hang_during_rollback),
            test("testMergeThreadMessages", test_merge_thread_messages),
            test("testDynamicDefaults", test_dynamic_defaults),
            test("testResetToAutoDefault", test_reset_to_auto_default),
            test("testSpinningDefaults", test_spinning_defaults),
            test("testAutoIOThrottleGetter", test_auto_io_throttle_getter),
            test("testNonSpinningDefaults", test_non_spinning_defaults),
            test("testNoStallMergeThreads", test_no_stall_merge_threads),
            test(
                "testChangeMaxMergeCountyWhileForceMerge",
                test_change_max_merge_county_while_force_merge,
            ),
        ],
    )
}

const ALREADY_CLOSED: &str = "org.apache.lucene.store.AlreadyClosedException";

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn plain_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `iwc.getMergeScheduler() instanceof ConcurrentMergeScheduler`.
fn is_cms(iwc: &IndexWriterConfig) -> bool {
    iwc.merge_scheduler()
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .is_some()
}

/// The override `getIntraMergeExecutor(merge)` several anonymous
/// subclasses declare: "override here to ensure even tiny merges get the
/// parallel executor".
fn always_intra_merge_executor(cms: &ConcurrentMergeScheduler, message: &'static str) {
    cms.set_intra_merge_executor_override(Some(Arc::new(move |cms, _merge| {
        let executor = cms.intra_merge_executor();
        debug_assert!(executor.is_some(), "{message}");
        Ok(executor)
    })));
}

/// `t.join()`, rethrowing what the thread threw.
fn join(t: &Thread) {
    if let Err(payload) = t.join() {
        std::panic::resume_unwind(payload);
    }
}

/// `private class FailOnlyOnFlush extends MockDirectoryWrapper.Failure`.
struct FailOnlyOnFlush {
    do_fail: AtomicBool,
    hit_exc: AtomicBool,
}

impl Failure for FailOnlyOnFlush {
    fn set_do_fail(&self) {
        self.do_fail.store(true, Ordering::SeqCst);
        self.hit_exc.store(false, Ordering::SeqCst);
    }

    fn clear_do_fail(&self) {
        self.do_fail.store(false, Ordering::SeqCst);
    }

    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail.load(Ordering::SeqCst)
            && LuceneTestCase::is_test_thread()
            && LuceneTestCase::call_stack_contains_any_of(&["flush"])
            && !LuceneTestCase::call_stack_contains_any_of(&["close"])
            && random().next_boolean()
        {
            self.hit_exc.store(true, Ordering::SeqCst);
            return Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other(format!(
                    "{}: now failing during flush",
                    Thread::current().name()
                )),
            )));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Make sure running BG merges still work fine even when we are hitting
/// exceptions during flushing.
///
/// Port of `TestConcurrentMergeScheduler.testFlushExceptions`.
fn test_flush_exceptions() -> Result<()> {
    let directory = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let failure = Arc::new(FailOnlyOnFlush {
        do_fail: AtomicBool::new(false),
        hit_exc: AtomicBool::new(false),
    });
    directory
        .as_mock()
        .expect("INVARIANT: a mock directory")
        .fail_on(Arc::clone(&failure) as Arc<dyn Failure>);
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    if is_cms(&iwc) {
        let cms = SuppressingConcurrentMergeScheduler::create(Arc::new(|th| {
            match th.downcast_ref::<LuceneError>() {
                Some(e) => {
                    is_instance(e, ALREADY_CLOSED)
                        || (is_instance(e, "java.lang.IllegalStateException")
                            && java_throwable(e)
                                .message()
                                .unwrap_or_default()
                                .contains("this writer hit an unrecoverable error"))
                }
                None => false,
            }
        }));
        always_intra_merge_executor(&cms, "intraMergeExecutor is not initialized");
        iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    }
    let writer = IndexWriter::new(directory.as_directory(), iwc)?;
    let mut doc = Document::new();
    let id_field = LuceneTestCase::new_string_field("id", "", Store::YES);
    let knn_field = KnnFloatVectorField::with_euclidean("knn", &[0.0, 0.0])?;
    doc.add(Box::new(id_field));
    // Add knn float vectors to test parallel merge
    doc.add(Box::new(knn_field));

    'outer: for i in 0..10 {
        if verbose() {
            println!("TEST: iter={i}");
        }

        for j in 0..20 {
            field_mut(&mut doc, "id").set_string_value((i * 20 + j).to_string())?;
            let (a, b) = (random().next_float(), random().next_float());
            doc.get_field_mut_as::<KnnFloatVectorField>("knn")
                .expect("knnField")
                .set_vector_value(&[a, b])?;
            writer.add_document(&doc)?;
        }

        // must cycle here because sometimes the merge flushes
        // the doc we just added and so there's nothing to
        // flush, and we don't hit the exception
        loop {
            writer.add_document(&doc)?;
            failure.set_do_fail();
            match writer.flush_with(true, true) {
                Ok(()) => {
                    if failure.hit_exc.load(Ordering::SeqCst) {
                        Assert::fail("failed to hit IOException");
                    }
                }
                Err(ioe) if is_instance(&ioe, "java.io.IOException") => {
                    if verbose() {
                        println!("{}", java_throwable(&ioe));
                    }
                    failure.clear_do_fail();
                    // make sure we are closed or closing - if we are unlucky a merge does
                    // the actual closing for us. this is rare but might happen since the
                    // tragicEvent is checked by IFD and that might throw during a merge
                    expect_throws(ALREADY_CLOSED, || writer.ensure_open());
                    // Abort should have closed the deleter:
                    Assert::assert_true(writer.is_deleter_closed()?);
                    writer.close()?; // now wait for the close to actually happen if a merge thread did the
                                     // close.
                    break 'outer;
                }
                Err(e) => return Err(e),
            }
        }
    }

    Assert::assert_false(directory_reader::index_exists(&*directory.as_directory())?);
    directory.close()
}

/// Test that deletes committed after a merge started and before it
/// finishes, are correctly merged back.
///
/// Port of `TestConcurrentMergeScheduler.testDeleteMerging`.
fn test_delete_merging() -> Result<()> {
    let directory = LuceneTestCase::new_directory();

    let mut mp = LogMergePolicy::by_doc_count();
    // Force degenerate merging so we can get a mix of
    // merging of segments with and without deletes at the
    // start:
    mp.set_min_merge_docs(1000);
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    let writer = IndexWriter::new(directory.as_directory(), iwc)?;
    TestUtil::reduce_open_files(&writer)?;

    let mut doc = Document::new();
    let id_field = LuceneTestCase::new_string_field("id", "", Store::YES);
    doc.add(Box::new(id_field));
    for i in 0..10 {
        if verbose() {
            println!("\nTEST: cycle");
        }
        for j in 0..100 {
            field_mut(&mut doc, "id").set_string_value((i * 100 + j).to_string())?;
            writer.add_document(&doc)?;
        }

        let mut del_id = i;
        while del_id < 100 * (1 + i) {
            if verbose() {
                println!("TEST: del {del_id}");
            }
            writer.delete_documents_by_terms(vec![Term::from_text("id", &del_id.to_string())])?;
            del_id += 10;
        }

        writer.commit()?;
    }

    writer.close()?;
    let reader = open_reader(&directory)?;
    // Verify that we did not lose any deletes...
    assert_equals(&450, &reader.num_docs());
    reader.close()?;
    directory.close()
}

/// Port of `TestConcurrentMergeScheduler.testNoExtraFiles`.
fn test_no_extra_files() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    let mut writer = IndexWriter::new(directory.as_directory(), iwc)?;

    for iter in 0..7 {
        if verbose() {
            println!("TEST: iter={iter}");
        }

        for _ in 0..21 {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "content",
                "a b c",
                Store::NO,
            )));
            writer.add_document(&doc)?;
        }

        writer.close()?;
        assert_no_unreferenced_files(&directory.as_directory(), "testNoExtraFiles")?;

        // Reopen
        let iwc = mock_iwc();
        iwc.set_open_mode(OpenMode::APPEND)?;
        iwc.set_max_buffered_docs(2)?;
        writer = IndexWriter::new(directory.as_directory(), iwc)?;
    }

    writer.close()?;

    directory.close()
}

/// Port of `TestConcurrentMergeScheduler.testNoWaitClose`.
fn test_no_wait_close() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let mut doc = Document::new();
    let id_field = LuceneTestCase::new_string_field("id", "", Store::YES);
    let knn_field = KnnFloatVectorField::with_euclidean("knn", &[0.0, 0.0])?;
    doc.add(Box::new(id_field));
    // Add knn float vectors to test parallel merge
    doc.add(Box::new(knn_field));
    let iwc = mock_iwc();
    // Force excessive merging:
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(100)?,
    )))?;
    iwc.set_commit_on_close(false);
    if is_cms(&iwc) {
        let cms = ConcurrentMergeScheduler::new();
        always_intra_merge_executor(&cms, "scaledExecutor is not initialized");
        iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    }

    let mut writer = IndexWriter::new(directory.as_directory(), iwc)?;

    let num_iters = if *TEST_NIGHTLY { 10 } else { 3 };
    for iter in 0..num_iters {
        for j in 0..201 {
            field_mut(&mut doc, "id").set_string_value((iter * 201 + j).to_string())?;
            let (a, b) = (random().next_float(), random().next_float());
            doc.get_field_mut_as::<KnnFloatVectorField>("knn")
                .expect("knnField")
                .set_vector_value(&[a, b])?;
            writer.add_document(&doc)?;
        }

        let mut del_id = iter * 201;
        for _ in 0..20 {
            writer.delete_documents_by_terms(vec![Term::from_text("id", &del_id.to_string())])?;
            del_id += 5;
        }

        // Force a bunch of merge threads to kick off so we
        // stress out aborting them on close:
        writer
            .get_config()
            .merge_policy()
            .as_any()
            .and_then(|a| a.downcast_ref::<LogMergePolicy>())
            .unwrap_or_else(|| {
                Throwable::new("java.lang.ClassCastException", "not a LogMergePolicy").throw()
            })
            .set_merge_factor(3)?;
        writer.add_document(&doc)?;

        let committed = writer.commit();
        // `finally`
        writer.close()?;
        committed?;

        let reader = open_reader(&directory)?;
        assert_equals(&((1 + iter) * 182), &reader.num_docs());
        reader.close()?;

        // Reopen
        let iwc = mock_iwc();
        iwc.set_open_mode(OpenMode::APPEND)?;
        iwc.set_merge_policy(Some(Arc::new(
            LuceneTestCase::new_log_merge_policy_merge_factor(100)?,
        )))?;
        // Force excessive merging:
        iwc.set_max_buffered_docs(2)?;
        iwc.set_commit_on_close(false);
        writer = IndexWriter::new(directory.as_directory(), iwc)?;
    }
    writer.close()?;

    directory.close()
}

/// LUCENE-4544
///
/// Port of `TestConcurrentMergeScheduler.testMaxMergeCount`.
fn test_max_merge_count() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = plain_iwc();
    iwc.set_commit_on_close(false);

    let max_merge_count = TestUtil::next_int(&*random(), 1, 5);
    let max_merge_threads = TestUtil::next_int(&*random(), 1, max_merge_count);
    let enough_merges_waiting = Arc::new(CountDownLatch::new(max_merge_count as i64));
    let running_merge_count = Arc::new(AtomicI32::new(0));
    let failed = Arc::new(AtomicBool::new(false));

    if verbose() {
        println!("TEST: maxMergeCount={max_merge_count} maxMergeThreads={max_merge_threads}");
    }

    let cms = ConcurrentMergeScheduler::new();
    {
        let enough_merges_waiting = Arc::clone(&enough_merges_waiting);
        let running_merge_count = Arc::clone(&running_merge_count);
        let failed = Arc::clone(&failed);
        cms.set_do_merge_override(Some(Arc::new(move |cms, merge_source, merge| {
            let merge_for_finish = merge.clone();
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    // Stall all incoming merges until we see
                    // maxMergeCount:
                    let count = running_merge_count.fetch_add(1, Ordering::SeqCst) + 1;
                    let body =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                            Assert::assert_true_msg(
                                &format!("count={count} vs maxMergeCount={max_merge_count}"),
                                count <= max_merge_count,
                            );
                            enough_merges_waiting.count_down();

                            // Stall this merge until we see exactly
                            // maxMergeCount merges waiting
                            loop {
                                if enough_merges_waiting.await_millis(10)
                                    || failed.load(Ordering::SeqCst)
                                {
                                    break;
                                }
                            }
                            // Then sleep a bit to give a chance for the bug
                            // (too many pending merges) to appear:
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            cms.default_do_merge(merge_source, merge)
                        }));
                    // `finally`
                    running_merge_count.fetch_sub(1, Ordering::SeqCst);
                    match body {
                        Ok(r) => r,
                        Err(payload) => std::panic::resume_unwind(payload),
                    }
                }));
            let t = match outcome {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(e)) => java_throwable(&e),
                Err(payload) => Throwable::from_panic(payload),
            };
            failed.store(true, Ordering::SeqCst);
            merge_source.on_merge_finished(&merge_for_finish);
            Throwable::from_cause("java.lang.RuntimeException", t).throw()
        })));
    }
    cms.set_max_merges_and_threads(max_merge_count, max_merge_threads)?;
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    iwc.set_max_buffered_docs(2)?;

    let tmp = Arc::new(TieredMergePolicy::new());
    iwc.set_merge_policy(Some(Arc::clone(&tmp) as Arc<dyn MergePolicy>))?;
    tmp.set_max_merge_at_once(2)?;
    tmp.set_segments_per_tier(2.0)?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "field",
        TextField::type_not_stored(),
    )));
    while enough_merges_waiting.get_count() != 0 && !failed.load(Ordering::SeqCst) {
        for _ in 0..10 {
            w.add_document(&doc)?;
        }
    }
    let committed = w.commit();
    // `finally`
    w.close()?;
    committed?;
    dir.close()
}

/// Port of `TestConcurrentMergeScheduler.testSmallMergesDonNotGetThreads`.
fn test_small_merges_don_not_get_threads() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = plain_iwc();
    iwc.set_max_buffered_docs(2)?;
    let cms = ConcurrentMergeScheduler::new();
    cms.set_do_merge_override(Some(Arc::new(|cms, merge_source, merge| {
        let executor = MergeScheduler::get_intra_merge_executor(cms, &merge)?;
        Assert::assert_true(
            executor
                .as_ref()
                .and_then(|e| e.as_any())
                .and_then(|a| a.downcast_ref::<SameThreadExecutorService>())
                .is_some(),
        );
        cms.default_do_merge(merge_source, merge)
    })));
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for i in 0..10 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.close()?;
    dir.close()
}

/// The anonymous `MergeScheduler.MergeSource` of
/// `testIntraMergeThreadPoolIsLimitedByMaxThreads`: every method fails.
struct FailingMergeSource;

impl std::fmt::Display for FailingMergeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MergeSource")
    }
}

impl MergeSource for FailingMergeSource {
    fn get_next_merge(&self) -> Result<Option<OneMerge>> {
        Assert::fail("should not be called")
    }

    fn on_merge_finished(&self, _merge: &OneMerge) {
        Assert::fail("should not be called")
    }

    fn has_pending_merges(&self) -> Result<bool> {
        Assert::fail("should not be called")
    }

    fn merge(&self, _merge: OneMerge) -> Result<()> {
        Assert::fail("should not be called")
    }
}

/// Port of `TestConcurrentMergeScheduler.testIntraMergeThreadPoolIsLimitedByMaxThreads`.
fn test_intra_merge_thread_pool_is_limited_by_max_threads() -> Result<()> {
    let merge_scheduler = ConcurrentMergeScheduler::new();
    let merge_source: Arc<dyn MergeSource> = Arc::new(FailingMergeSource);
    let dir = LuceneTestCase::new_directory();
    let body = || -> Result<()> {
        let codec = rucene::codecs::get_default().unwrap_or_else(|| {
            Throwable::new("java.lang.IllegalStateException", "no default codec").throw()
        });
        let merge = OneMerge::new(vec![SegmentCommitInfo::new(
            SegmentInfo::new(
                dir.as_directory(),
                Version::LATEST,
                None,
                "test".to_string(),
                0,
                false,
                false,
                codec,
                std::collections::HashMap::new(),
                StringHelper::random_id(),
                std::collections::HashMap::new(),
                Sort::default(),
            )?,
            0,
            0,
            0,
            0,
            0,
            [0u8; 16],
        )?])?;
        MergeScheduler::initialize(
            &merge_scheduler,
            Arc::new(NoOutputInfoStream),
            &dir.as_directory(),
        )?;
        merge_scheduler.set_max_merges_and_threads(6, 6)?;
        let executor = merge_scheduler
            .intra_merge_executor()
            .unwrap_or_else(|| crate::support::npe("intraMergeExecutor"));
        let threads_executed_on_pool = Arc::new(AtomicI32::new(0));
        let threads_executed_on_self = Arc::new(AtomicI32::new(0));
        let latch = Arc::new(CountDownLatch::new(1));
        const TOTAL_THREADS: i32 = 4;
        for i in 0..TOTAL_THREADS {
            let executor = Arc::clone(&executor);
            let on_pool = Arc::clone(&threads_executed_on_pool);
            let on_self = Arc::clone(&threads_executed_on_self);
            let latch = Arc::clone(&latch);
            merge_scheduler.add_merge_thread(Arc::new(MergeThread::with_run(
                Arc::clone(&merge_source),
                merge.clone(),
                format!("Thread-{i}"),
                Box::new(move |this: &Arc<MergeThread>| {
                    let this = Arc::clone(this);
                    let executed = executor.execute(Box::new(move || {
                        if this.is_current_thread() {
                            on_self.fetch_add(1, Ordering::SeqCst);
                        } else {
                            on_pool.fetch_add(1, Ordering::SeqCst);
                        }
                        latch.await_();
                    }));
                    if let Err(rejected) = executed {
                        Throwable::new(
                            "java.util.concurrent.RejectedExecutionException",
                            rejected.message,
                        )
                        .throw()
                    }
                }),
            )));
        }
        for thread in merge_scheduler.merge_threads() {
            merge_scheduler.start_merge_thread(&thread)?;
        }
        while threads_executed_on_self.load(Ordering::SeqCst)
            + threads_executed_on_pool.load(Ordering::SeqCst)
            < TOTAL_THREADS
        {
            std::thread::yield_now();
        }
        latch.count_down();
        merge_scheduler.sync();
        assert_equals(&3, &threads_executed_on_self.load(Ordering::SeqCst));
        assert_equals(&1, &threads_executed_on_pool.load(Ordering::SeqCst));
        Ok(())
    };
    let outcome = body();
    // try-with-resources closes `mergeScheduler`, then `dir`.
    let closed_scheduler = MergeScheduler::close(&merge_scheduler);
    let closed_dir = dir.close();
    outcome?;
    closed_scheduler?;
    closed_dir
}

/// `private static class TrackingCMS extends ConcurrentMergeScheduler`: the
/// scheduler, and its `totMergedBytes` field.
fn tracking_cms(
    at_least_one_merge: Arc<CountDownLatch>,
) -> Result<(ConcurrentMergeScheduler, Arc<AtomicI64>)> {
    let cms = ConcurrentMergeScheduler::new();
    let tot_merged_bytes = Arc::new(AtomicI64::new(0));
    cms.set_max_merges_and_threads(5, 5)?;
    let tot = Arc::clone(&tot_merged_bytes);
    cms.set_do_merge_override(Some(Arc::new(move |cms, merge_source, merge| {
        tot.fetch_add(merge.total_bytes_size(), Ordering::SeqCst);
        at_least_one_merge.count_down();
        cms.default_do_merge(merge_source, merge)
    })));
    Ok((cms, tot_merged_bytes))
}

/// Port of `TestConcurrentMergeScheduler.testTotalBytesSize`.
fn test_total_bytes_size() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    crate::support::never_throttle(&d);
    let iwc = plain_iwc();
    iwc.set_max_buffered_docs(5)?;
    let at_least_one_merge = Arc::new(CountDownLatch::new(1));
    let (cms, tot_merged_bytes) = tracking_cms(Arc::clone(&at_least_one_merge))?;
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    if TestUtil::get_postings_format("id") == "SimpleText" {
        // no
        iwc.set_codec(TestUtil::always_postings_format(
            TestUtil::get_default_postings_format(),
        ))?;
    }
    let w = IndexWriter::new(d.as_directory(), iwc)?;
    for i in 0..1000 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
        w.add_document(&doc)?;

        if random().next_boolean() {
            let id = random().next_int_bound(i + 1);
            w.delete_documents_by_terms(vec![Term::from_text("id", &id.to_string())])?;
        }
    }
    at_least_one_merge.await_();
    // `((TrackingCMS) w.getConfig().getMergeScheduler()).totMergedBytes`
    Assert::assert_true(tot_merged_bytes.load(Ordering::SeqCst) != 0);
    w.close()?;
    d.close()
}

/// Port of `TestConcurrentMergeScheduler.testInvalidMaxMergeCountAndThreads`.
fn test_invalid_max_merge_count_and_threads() -> Result<()> {
    let cms = ConcurrentMergeScheduler::new();
    expect_throws(IAE, || {
        cms.set_max_merges_and_threads(AUTO_DETECT_MERGES_AND_THREADS, 3)
    });
    expect_throws(IAE, || {
        cms.set_max_merges_and_threads(3, AUTO_DETECT_MERGES_AND_THREADS)
    });
    Ok(())
}

/// The anonymous `MergePolicy` of `testLiveMaxMergeCount`: no natural
/// merges, and forced merges of ten segments at a time.
struct ManyForcedMerges;

impl std::fmt::Display for ManyForcedMerges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MergePolicy")
    }
}

impl MergePolicy for ManyForcedMerges {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // no natural merges
        Ok(None)
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // not needed
        Ok(None)
    }

    /// Java iterates `segmentsToMerge.keySet()`, a `HashMap` keyed by
    /// identity-hashed `SegmentCommitInfo`s, in no defined order; Rucene's
    /// map is keyed by segment name, so the infos are taken from
    /// `segmentInfos` in their order, those the map holds.
    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        _max_segment_count: i32,
        segments_to_merge: &std::collections::HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // The test is about testing that CMS bounds the number of merging threads, so we just
        // return many merges.
        let mut spec = MergeSpecification::new();
        let mut one_merge: Vec<SegmentCommitInfo> = Vec::new();
        for sci in segment_infos.iter() {
            if !segments_to_merge.contains_key(&sci.info.name) {
                continue;
            }
            one_merge.push(sci.clone());
            if one_merge.len() >= 10 {
                spec.add(OneMerge::new(one_merge.clone())?);
                one_merge.clear();
            }
        }
        Ok(Some(spec))
    }
}

/// Port of `TestConcurrentMergeScheduler.testLiveMaxMergeCount`.
fn test_live_max_merge_count() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let iwc = plain_iwc();
    iwc.set_merge_policy(Some(Arc::new(ManyForcedMerges)))?;
    iwc.set_max_buffered_docs(2)?;
    iwc.set_ram_buffer_size_mb(-1.0)?;

    let max_running_merge_count = Arc::new(AtomicI32::new(0));

    let cms = Arc::new(ConcurrentMergeScheduler::new());
    {
        let running_merge_count = Arc::new(AtomicI32::new(0));
        let max_running_merge_count = Arc::clone(&max_running_merge_count);
        // `synchronized (this)`
        let this_monitor = Arc::new(Mutex::new(()));
        cms.set_do_merge_override(Some(Arc::new(move |cms, merge_source, merge| {
            let count = running_merge_count.fetch_add(1, Ordering::SeqCst) + 1;
            // evil?
            {
                let _sync = this_monitor.lock().unwrap_or_else(|e| e.into_inner());
                if count > max_running_merge_count.load(Ordering::SeqCst) {
                    max_running_merge_count.store(count, Ordering::SeqCst);
                }
            }
            let merged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cms.default_do_merge(merge_source, merge)
            }));
            // `finally`
            running_merge_count.fetch_sub(1, Ordering::SeqCst);
            match merged {
                Ok(r) => r,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        })));
    }

    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_merge_count());
    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_thread_count());

    cms.set_max_merges_and_threads(5, 3)?;

    iwc.set_merge_scheduler(Some(Arc::clone(&cms) as Arc<dyn MergeScheduler>))?;

    let w = IndexWriter::new(d.as_directory(), iwc)?;
    // Makes 100 segments
    for _ in 0..200 {
        w.add_document(&Document::new())?;
    }

    // No merges should have run so far, because TMP has high segmentsPerTier:
    assert_equals(&0, &max_running_merge_count.load(Ordering::SeqCst));
    w.force_merge(1)?;

    // At most 5 merge threads should have launched at once:
    Assert::assert_true_msg(
        &format!(
            "maxRunningMergeCount={}",
            max_running_merge_count.load(Ordering::SeqCst)
        ),
        max_running_merge_count.load(Ordering::SeqCst) <= 5,
    );
    max_running_merge_count.store(0, Ordering::SeqCst);

    // Makes another 100 segments
    for _ in 0..200 {
        w.add_document(&Document::new())?;
    }

    let ms = w.get_config().merge_scheduler();
    as_cms(&ms).set_max_merges_and_threads(1, 1)?;
    w.force_merge(1)?;

    // At most 1 merge thread should have launched at once:
    assert_equals(&1, &max_running_merge_count.load(Ordering::SeqCst));

    w.close()?;
    d.close()
}

/// LUCENE-6063
///
/// Port of `TestConcurrentMergeScheduler.testMaybeStallCalled`.
fn test_maybe_stall_called() -> Result<()> {
    let was_called = Arc::new(AtomicBool::new(false));
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(LogMergePolicy::by_byte_size())))?;
    let cms = ConcurrentMergeScheduler::new();
    {
        let was_called = Arc::clone(&was_called);
        cms.set_maybe_stall_override(Some(Arc::new(move |_merge_source| {
            was_called.store(true, Ordering::SeqCst);
            Ok(true)
        })));
    }
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    w.flush()?;
    w.add_document(&Document::new())?;
    w.force_merge(1)?;
    Assert::assert_true(was_called.load(Ordering::SeqCst));
    w.close()?;
    dir.close()
}

/// LUCENE-6094
///
/// Port of `TestConcurrentMergeScheduler.testHangDuringRollback`.
fn test_hang_during_rollback() -> Result<()> {
    let dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let iwc = plain_iwc();
    iwc.set_max_buffered_docs(2)?;
    let mp = Arc::new(LogMergePolicy::by_doc_count());
    iwc.set_merge_policy(Some(Arc::clone(&mp) as Arc<dyn MergePolicy>))?;
    mp.set_merge_factor(2)?;
    let merge_start = Arc::new(CountDownLatch::new(1));
    let merge_finish = Arc::new(CountDownLatch::new(1));
    let cms = ConcurrentMergeScheduler::new();
    {
        let merge_start = Arc::clone(&merge_start);
        let merge_finish = Arc::clone(&merge_finish);
        cms.set_do_merge_override(Some(Arc::new(move |cms, merge_source, merge| {
            merge_start.count_down();
            merge_finish.await_();
            cms.default_do_merge(merge_source, merge)
        })));
    }
    cms.set_max_merges_and_threads(1, 1)?;
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // flush

    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // flush + merge

    // Wait for merge to kick off
    merge_start.await_();

    {
        let w = Arc::clone(&w);
        let merge_finish = Arc::clone(&merge_finish);
        Thread::unnamed(move || {
            let body = || -> Result<()> {
                w.add_document(&Document::new())?;
                w.add_document(&Document::new())?;
                // flush

                w.add_document(&Document::new())?;
                // W/o the fix for LUCENE-6094 we would hang forever here:
                w.add_document(&Document::new())?;
                // flush + merge

                // Now allow first merge to finish:
                merge_finish.count_down();
                Ok(())
            };
            if let Err(e) = body() {
                Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw()
            }
        })
        .start();
    }

    while w.get_doc_stats()?.num_docs != 8 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    w.rollback()?;
    dir.close()
}

/// The anonymous `InfoStream` of `testMergeThreadMessages`: keeps the `MS`
/// messages.
struct MsMessages(Arc<Mutex<Vec<String>>>);

impl InfoStream for MsMessages {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestConcurrentMergeScheduler$1"
    }

    fn close(&self) {}

    fn message(&self, component: &str, message: &str) {
        if component == "MS" {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(message.to_string());
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        component == "MS"
    }
}

/// LUCENE-10118 : Verify the basic log output from MergeThreads
///
/// Port of `TestConcurrentMergeScheduler.testMergeThreadMessages`.
fn test_merge_thread_messages() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = plain_iwc();
    let merge_thread_set: Arc<Mutex<Vec<Arc<MergeThread>>>> = Arc::new(Mutex::new(Vec::new()));
    let cms = ConcurrentMergeScheduler::new();
    {
        let merge_thread_set = Arc::clone(&merge_thread_set);
        cms.set_get_merge_thread_override(Some(Arc::new(
            move |super_get_merge_thread, merge_source, merge| {
                let new_merge_thread = super_get_merge_thread(merge_source, merge)?;
                let mut set = merge_thread_set.lock().unwrap_or_else(|e| e.into_inner());
                if !set.iter().any(|t| Arc::ptr_eq(t, &new_merge_thread)) {
                    set.push(Arc::clone(&new_merge_thread));
                }
                Ok(new_merge_thread)
            },
        )));
    }
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;

    let messages = Arc::new(Mutex::new(Vec::new()));
    iwc.set_info_stream(Arc::new(MsMessages(Arc::clone(&messages))))?;
    iwc.set_max_buffered_docs(2)?;
    let mut lmp = LuceneTestCase::new_log_merge_policy();
    lmp.set_merge_factor(2)?;
    lmp.set_target_search_concurrency(1)?;
    iwc.set_merge_policy(Some(Arc::new(lmp)))?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let stream: std::sync::Arc<rucene::util::atomic_ref_cell::AtomicRefCell<dyn TokenStream>> =
        std::sync::Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
            CannedTokenStream::new(Vec::new()),
        ));
    doc.add(Box::new(Field::new_with_token_stream(
        "foo",
        stream,
        rucene::document::FieldType::new_from(TextField::type_not_stored()),
    )?));
    w.add_document(&doc)?;
    w.add_document(&Document::new())?;
    // flush
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // flush + merge
    w.close()?;
    dir.close()?;

    let merge_thread_set = merge_thread_set
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Assert::assert_true(!merge_thread_set.is_empty());
    for t in &merge_thread_set {
        t.join();
    }
    let messages = messages.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for t in &merge_thread_set {
        let name = t.get_name();
        let thread_msgs: Vec<&String> = messages
            .iter()
            .filter(|line| line.starts_with(&format!("merge thread {name}")))
            .collect();
        Assert::assert_true_msg(
            &format!(
                "Expected:·a·value·equal·to·or·greater·than·3,·got:{}, threadMsgs={:?}",
                thread_msgs.len(),
                thread_msgs
            ),
            thread_msgs.len() >= 3,
        );
        Assert::assert_true(thread_msgs[0].starts_with(&format!("merge thread {name} start")));
        Assert::assert_true(
            thread_msgs
                .iter()
                .any(|line| line.starts_with(&format!("merge thread {name} merge segment"))),
        );
        Assert::assert_true(
            thread_msgs[thread_msgs.len() - 1].starts_with(&format!("merge thread {name} end")),
        );
    }
    Ok(())
}

/// Port of `TestConcurrentMergeScheduler.testDynamicDefaults`.
fn test_dynamic_defaults() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = plain_iwc();
    let cms = Arc::new(ConcurrentMergeScheduler::new());
    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_merge_count());
    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_thread_count());
    iwc.set_merge_scheduler(Some(Arc::clone(&cms) as Arc<dyn MergeScheduler>))?;
    iwc.set_max_buffered_docs(2)?;
    let lmp = LuceneTestCase::new_log_merge_policy();
    lmp.set_merge_factor(2)?;
    iwc.set_merge_policy(Some(Arc::new(lmp)))?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // flush

    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // flush + merge

    // CMS should have now set true values:
    Assert::assert_true(cms.get_max_merge_count() != AUTO_DETECT_MERGES_AND_THREADS);
    Assert::assert_true(cms.get_max_thread_count() != AUTO_DETECT_MERGES_AND_THREADS);
    w.close()?;
    dir.close()
}

/// Port of `TestConcurrentMergeScheduler.testResetToAutoDefault`.
fn test_reset_to_auto_default() -> Result<()> {
    let cms = ConcurrentMergeScheduler::new();
    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_merge_count());
    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_thread_count());
    cms.set_max_merges_and_threads(4, 3)?;
    assert_equals(&4, &cms.get_max_merge_count());
    assert_equals(&3, &cms.get_max_thread_count());

    expect_throws(IAE, || {
        cms.set_max_merges_and_threads(AUTO_DETECT_MERGES_AND_THREADS, 4)
    });

    expect_throws(IAE, || {
        cms.set_max_merges_and_threads(4, AUTO_DETECT_MERGES_AND_THREADS)
    });

    cms.set_max_merges_and_threads(
        AUTO_DETECT_MERGES_AND_THREADS,
        AUTO_DETECT_MERGES_AND_THREADS,
    )?;
    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_merge_count());
    assert_equals(&AUTO_DETECT_MERGES_AND_THREADS, &cms.get_max_thread_count());
    Ok(())
}

/// Port of `TestConcurrentMergeScheduler.testSpinningDefaults`.
fn test_spinning_defaults() -> Result<()> {
    let cms = ConcurrentMergeScheduler::new();
    cms.set_default_max_merges_and_threads(true);
    assert_equals(&1, &cms.get_max_thread_count());
    assert_equals(&6, &cms.get_max_merge_count());
    Ok(())
}

/// Port of `TestConcurrentMergeScheduler.testAutoIOThrottleGetter`.
fn test_auto_io_throttle_getter() -> Result<()> {
    let cms = ConcurrentMergeScheduler::new();
    Assert::assert_false(cms.get_auto_io_throttle());
    cms.enable_auto_io_throttle()?;
    Assert::assert_true(cms.get_auto_io_throttle());
    cms.disable_auto_io_throttle()?;
    Assert::assert_false(cms.get_auto_io_throttle());
    Ok(())
}

/// Port of `TestConcurrentMergeScheduler.testNonSpinningDefaults`.
fn test_non_spinning_defaults() -> Result<()> {
    let cms = ConcurrentMergeScheduler::new();
    cms.set_default_max_merges_and_threads(false);
    let thread_count = cms.get_max_thread_count();
    Assert::assert_true(thread_count >= 1);
    Assert::assert_true(thread_count <= 4);
    assert_equals(&(5 + thread_count), &cms.get_max_merge_count());
    Ok(())
}

/// LUCENE-6197
///
/// Port of `TestConcurrentMergeScheduler.testNoStallMergeThreads`.
fn test_no_stall_merge_threads() -> Result<()> {
    let dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    iwc.set_max_buffered_docs(2)?;
    iwc.set_use_compound_file(true)?; // reduce open files
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let num_docs = if *TEST_NIGHTLY { 1000 } else { 100 };
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            &i.to_string(),
            Store::YES,
        )));
        w.add_document(&doc)?;
    }
    w.close()?;

    let iwc = mock_iwc();
    let failed = Arc::new(AtomicBool::new(false));
    let cms = ConcurrentMergeScheduler::new();
    {
        let failed = Arc::clone(&failed);
        cms.set_do_stall_override(Some(Arc::new(move |super_do_stall| {
            if std::thread::current()
                .name()
                .unwrap_or_default()
                .starts_with("Lucene Merge Thread")
            {
                failed.store(true, Ordering::SeqCst);
            }
            super_do_stall()
        })));
    }
    cms.enable_auto_io_throttle()?;
    cms.set_max_merges_and_threads(2, 1)?;
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    iwc.set_max_buffered_docs(2)?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.force_merge(1)?;
    w.close()?;
    dir.close()?;

    Assert::assert_false(failed.load(Ordering::SeqCst));
    Ok(())
}

/// The anonymous `InfoStream` of `testChangeMaxMergeCountyWhileForceMerge`:
/// the test points `mergeMiddleStart` and `forceMergeBeforeWait` drive the
/// latches.
struct TestPointLatches {
    force_merge_waits: Arc<CountDownLatch>,
    merge_threads_start_after_wait: Arc<CountDownLatch>,
    merge_threads_arrived: Arc<CountDownLatch>,
}

impl InfoStream for TestPointLatches {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestConcurrentMergeScheduler$2"
    }

    fn message(&self, component: &str, message: &str) {
        if "TP" == component && "mergeMiddleStart" == message {
            self.merge_threads_arrived.count_down();
            self.merge_threads_start_after_wait.await_();
        } else if "TP" == component && "forceMergeBeforeWait" == message {
            self.force_merge_waits.count_down();
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        "TP" == component
    }

    fn close(&self) {}
}

/// This test tries to produce 2 merges running concurrently with 2 segments
/// per merge. While these merges run we kick off a forceMerge that puts a
/// pending merge in the queue but waits for things to happen. While we do
/// this we reduce maxMergeCount to 1. If concurrency in CMS is not right
/// the forceMerge will wait forever since none of the currently running
/// merges picks up the pending merge. This test fails every time.
///
/// Port of `TestConcurrentMergeScheduler.testChangeMaxMergeCountyWhileForceMerge`.
fn test_change_max_merge_county_while_force_merge() -> Result<()> {
    let num_iters = if *TEST_NIGHTLY { 100 } else { 10 };
    for _iters in 0..num_iters {
        let mp = LogMergePolicy::by_doc_count();
        mp.set_merge_factor(2)?;
        let force_merge_waits = Arc::new(CountDownLatch::new(1));
        let merge_threads_start_after_wait = Arc::new(CountDownLatch::new(1));
        let merge_threads_arrived = Arc::new(CountDownLatch::new(2));
        let stream = Arc::new(TestPointLatches {
            force_merge_waits: Arc::clone(&force_merge_waits),
            merge_threads_start_after_wait: Arc::clone(&merge_threads_start_after_wait),
            merge_threads_arrived: Arc::clone(&merge_threads_arrived),
        });
        let dir = LuceneTestCase::new_directory();
        let config = IndexWriterConfig::new();
        config.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
        config.set_merge_policy(Some(Arc::new(mp)))?;
        config.set_info_stream(stream)?;
        let writer = IndexWriter::new_with_test_points(dir.as_directory(), config)?;
        let body = || -> Result<()> {
            let t = {
                let writer = Arc::clone(&writer);
                Thread::unnamed(move || {
                    if let Err(e) = writer.force_merge(1) {
                        Throwable::from_cause("java.lang.AssertionError", java_throwable(&e))
                            .throw()
                    }
                })
            };
            let ms = writer.get_config().merge_scheduler();
            let cms = as_cms(&ms);
            cms.set_max_merges_and_threads(2, 2)?;
            let tried = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                for _ in 0..4 {
                    let mut document = Document::new();
                    document.add(Box::new(TextField::new(
                        "foo",
                        "the quick brown fox jumps over the lazy dog".to_string(),
                        Store::YES,
                    )?));
                    document.add(Box::new(TextField::new(
                        "bar",
                        RandomStrings::random_realistic_unicode_of_length(&*random(), 20),
                        Store::YES,
                    )?));
                    writer.add_document(&document)?;
                    writer.flush()?;
                }
                assert_equals_msg(
                    Some(&writer.clone_segment_infos()?.to_string()),
                    &4,
                    &writer.get_segment_count()?,
                );
                merge_threads_arrived.await_();
                t.start();
                force_merge_waits.await_();
                cms.set_max_merges_and_threads(1, 1)?;
                Ok(())
            }));
            // `finally`
            merge_threads_start_after_wait.count_down();
            match tried {
                Ok(r) => r?,
                Err(payload) => std::panic::resume_unwind(payload),
            }

            while t.is_alive() {
                t.join_millis(10);
                if cms.merge_thread_count() == 0 && writer.has_pending_merges_pub()? {
                    Assert::fail("writer has pending merges but no CMS threads are running");
                }
            }
            join(&t);
            assert_equals(&1, &writer.get_segment_count()?);
            Ok(())
        };
        let outcome = body();
        // try-with-resources closes `writer`, then `dir`.
        let closed_writer = writer.close();
        let closed_dir = dir.close();
        outcome?;
        closed_writer?;
        closed_dir?;
    }
    Ok(())
}
