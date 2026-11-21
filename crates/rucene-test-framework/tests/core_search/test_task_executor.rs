//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTaskExecutor.java`.
//!
//! A Java `Callable` that throws becomes a Rust callable returning the
//! `LuceneError` of the thrown class: an `IOException` is
//! [`LuceneErrorKind::Io`], an `IllegalStateException`
//! [`LuceneErrorKind::IllegalState`], and any other throwable (`RuntimeException`,
//! `Exception`, `OutOfMemoryError`, `AssertionError`) the
//! [`LuceneErrorKind::Unchecked`] or [`LuceneErrorKind::AssertionError`] carrying its
//! class, which is what `FutureTask` captures and `invokeAll` rethrows.
//! `Runnable::run` is [`DirectExecutor`].

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::Document;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::index_searcher::{IndexSearcher, IndexSearcherOverrides, LeafSlice};
use rucene::search::scorable::Scorable;
use rucene::search::{
    CollectionResult, Collector, CollectorManager, Command, Executor, LeafCollector,
    MatchAllDocsQuery, RejectedExecutionException, ScoreMode, TaskExecutor, TaskValue,
};
use rucene::util::NamedThreadFactory;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::jdk::executor_service::ExecutorService;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{static_hook, CountDownLatch};
use crate::core_support::{assert_equals, assert_null, class, random, test};

const RUNTIME_EXCEPTION: &str = "java.lang.RuntimeException";
const OUT_OF_MEMORY_ERROR: &str = "java.lang.OutOfMemoryError";

/// `TestTaskExecutor`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTaskExecutor",
        vec![
            static_hook("createExecutor", BeforeClass, create_executor),
            static_hook("shutdownExecutor", AfterClass, shutdown_executor),
            test(
                "testUnwrapIOExceptionFromExecutionException",
                test_unwrap_io_exception_from_execution_exception,
            ),
            test(
                "testUnwrapRuntimeExceptionFromExecutionException",
                test_unwrap_runtime_exception_from_execution_exception,
            ),
            test(
                "testUnwrapErrorFromExecutionException",
                test_unwrap_error_from_execution_exception,
            ),
            test("testUnwrappedExceptions", test_unwrapped_exceptions),
            test(
                "testInvokeAllFromTaskDoesNotDeadlockSameSearcher",
                test_invoke_all_from_task_does_not_deadlock_same_searcher,
            ),
            test(
                "testInvokeAllFromTaskDoesNotDeadlockMultipleSearchers",
                test_invoke_all_from_task_does_not_deadlock_multiple_searchers,
            ),
            test(
                "testInvokeAllDoesNotLeaveTasksBehind",
                test_invoke_all_does_not_leave_tasks_behind,
            ),
            test(
                "testInvokeAllCatchesMultipleExceptions",
                test_invoke_all_catches_multiple_exceptions,
            ),
            test("testCancelTasksOnException", test_cancel_tasks_on_exception),
            test(
                "testTaskRejectionDoesNotFailExecution",
                test_task_rejection_does_not_fail_execution,
            ),
        ],
    )
}

/// `private static ExecutorService executorService`.
static EXECUTOR_SERVICE: Mutex<Option<ThreadPoolExecutor>> = Mutex::new(None);

/// `executorService`.
fn executor_service() -> ThreadPoolExecutor {
    EXECUTOR_SERVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("createExecutor ran")
}

/// `Executors.newFixedThreadPool(n, new NamedThreadFactory(TestTaskExecutor.class.getSimpleName()))`.
fn new_fixed_thread_pool(n: i32) -> ThreadPoolExecutor {
    ThreadPoolExecutor::new(
        n,
        n,
        Duration::ZERO,
        NamedThreadFactory::new("TestTaskExecutor"),
    )
}

/// `@BeforeClass public static void createExecutor()`.
fn create_executor() -> Result<()> {
    *EXECUTOR_SERVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(new_fixed_thread_pool(1));
    Ok(())
}

/// `@AfterClass public static void shutdownExecutor()`.
fn shutdown_executor() -> Result<()> {
    executor_service().shutdown();
    Ok(())
}

/// `Runnable::run`: an `Executor` that runs each command on the calling
/// thread.
struct DirectExecutor;

impl Executor for DirectExecutor {
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        command();
        Ok(())
    }
}

/// `throw new <class>(message)` for a throwable without a dedicated
/// `LuceneError` variant.
fn throwable(class: &'static str, message: Option<&str>) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class,
        message: message.map(str::to_string),
        cause: None,
    })
}

/// A Java `Callable<Void>`.
type VoidCallable = Box<dyn FnOnce() -> Result<()> + Send + 'static>;

/// Port of `TestTaskExecutor.testUnwrapIOExceptionFromExecutionException`.
fn test_unwrap_io_exception_from_execution_exception() -> Result<()> {
    let task_executor = TaskExecutor::new(Arc::new(executor_service()));
    let io_exception = expect_throws("java.io.IOException", || {
        let callables: Vec<VoidCallable> = vec![Box::new(|| {
            Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other("io exception"),
            )))
        })];
        task_executor.invoke_all(callables)
    });
    assert_equals(&Some("io exception"), &io_exception.message());
    Ok(())
}

/// Port of `TestTaskExecutor.testUnwrapRuntimeExceptionFromExecutionException`.
fn test_unwrap_runtime_exception_from_execution_exception() -> Result<()> {
    let task_executor = TaskExecutor::new(Arc::new(executor_service()));
    let runtime_exception = expect_throws(RUNTIME_EXCEPTION, || {
        let callables: Vec<VoidCallable> = vec![Box::new(|| {
            Err(throwable(RUNTIME_EXCEPTION, Some("runtime")))
        })];
        task_executor.invoke_all(callables)
    });
    assert_equals(&Some("runtime"), &runtime_exception.message());
    assert_null(&runtime_exception.cause().map(|c| c.to_string()));
    Ok(())
}

/// Port of `TestTaskExecutor.testUnwrapErrorFromExecutionException`.
fn test_unwrap_error_from_execution_exception() -> Result<()> {
    let task_executor = TaskExecutor::new(Arc::new(executor_service()));
    let out_of_memory_error = expect_throws(OUT_OF_MEMORY_ERROR, || {
        let callables: Vec<VoidCallable> = vec![Box::new(|| {
            Err(throwable(OUT_OF_MEMORY_ERROR, Some("oom")))
        })];
        task_executor.invoke_all(callables)
    });
    assert_equals(&Some("oom"), &out_of_memory_error.message());
    assert_null(&out_of_memory_error.cause().map(|c| c.to_string()));
    Ok(())
}

/// Port of `TestTaskExecutor.testUnwrappedExceptions`.
fn test_unwrapped_exceptions() -> Result<()> {
    let task_executor = TaskExecutor::new(Arc::new(executor_service()));
    let runtime_exception = expect_throws(RUNTIME_EXCEPTION, || {
        let callables: Vec<VoidCallable> = vec![Box::new(|| {
            Err(throwable("java.lang.Exception", Some("exc")))
        })];
        task_executor.invoke_all(callables)
    });
    assert_equals(
        &Some("exc"),
        &runtime_exception.cause().expect("getCause()").message(),
    );
    Ok(())
}

/// `executorService.submit(() -> { body(executorService); return null; }).get()`.
fn submit_and_get(body: fn(Arc<dyn Executor>) -> Result<()>) -> Result<()> {
    let es = executor_service();
    let inner: Arc<dyn Executor> = Arc::new(es.clone());
    let future = es
        .submit(Box::new(move || {
            body(inner)?;
            Ok(Box::new(()) as TaskValue)
        }))
        .unwrap_or_else(|e| {
            panic!(
                "java.util.concurrent.RejectedExecutionException: {}",
                e.message
            )
        });
    future.get()?;
    Ok(())
}

/// Port of `TestTaskExecutor.testInvokeAllFromTaskDoesNotDeadlockSameSearcher`.
fn test_invoke_all_from_task_does_not_deadlock_same_searcher() -> Result<()> {
    do_test_invoke_all_from_task_does_not_deadlock_same_searcher(Arc::new(executor_service()))?;
    do_test_invoke_all_from_task_does_not_deadlock_same_searcher(Arc::new(DirectExecutor))?;
    submit_and_get(do_test_invoke_all_from_task_does_not_deadlock_same_searcher)
}

/// The anonymous `IndexSearcher` subclass whose `slices(leaves)` is
/// `slices(leaves, 1, 1, false)`.
#[derive(Debug)]
struct OneDocPerSlice;

impl IndexSearcherOverrides for OneDocPerSlice {
    fn slices(
        &self,
        _searcher: &IndexSearcher,
        leaves: &[Arc<LeafReaderContext>],
    ) -> Result<Vec<LeafSlice>> {
        IndexSearcher::compute_slices(leaves, 1, 1, false)
    }
}

/// `new IndexSearcher(reader, executor) { slices(...) }`.
fn one_doc_per_slice_searcher(
    reader: &Arc<dyn IndexReader>,
    executor: &Arc<dyn Executor>,
) -> Result<IndexSearcher> {
    let mut searcher =
        IndexSearcher::with_executor(Arc::clone(reader), Some(Arc::clone(executor)))?;
    searcher.set_overrides(Arc::new(OneDocPerSlice));
    Ok(searcher)
}

/// The leaf collector work each `setScorer` call does.
type SetScorer = Arc<dyn Fn() -> Result<()> + Send + Sync>;

/// The anonymous `LeafCollector`, whose `setScorer` runs `set_scorer`.
struct InvokingLeafCollector {
    set_scorer: SetScorer,
}

impl LeafCollector for InvokingLeafCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        (self.set_scorer)()
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

/// The anonymous `Collector`.
struct InvokingCollector {
    set_scorer: SetScorer,
}

impl Collector for InvokingCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(InvokingLeafCollector {
            set_scorer: Arc::clone(&self.set_scorer),
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `CollectorManager<Collector, Void>`.
struct InvokingCollectorManager {
    set_scorer: SetScorer,
}

impl CollectorManager for InvokingCollectorManager {
    type Collector = InvokingCollector;
    type Output = ();

    fn new_collector(&self) -> Result<InvokingCollector> {
        Ok(InvokingCollector {
            set_scorer: Arc::clone(&self.set_scorer),
        })
    }

    fn reduce(&self, _collectors: Vec<InvokingCollector>) -> Result<()> {
        Ok(())
    }
}

/// `try (Directory dir = newDirectory(); RandomIndexWriter iw = ...) { 500
/// empty docs; try (DirectoryReader reader = iw.getReader()) { body } }`.
fn with_500_docs_reader(body: impl FnOnce(Arc<dyn IndexReader>) -> Result<()>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let result = (|| -> Result<()> {
            for _i in 0..500 {
                iw.add_document(&Document::new())?;
            }
            let reader = iw.get_reader()?;
            let result = body(Arc::clone(&reader) as Arc<dyn IndexReader>);
            result.and(reader.close())
        })();
        result.and(iw.close())
    })();
    result.and(dir.close())
}

/// `private static void doTestInvokeAllFromTaskDoesNotDeadlockSameSearcher(Executor executor)`.
fn do_test_invoke_all_from_task_does_not_deadlock_same_searcher(
    executor: Arc<dyn Executor>,
) -> Result<()> {
    with_500_docs_reader(|reader| {
        let searcher = one_doc_per_slice_searcher(&reader, &executor)?;
        let task_executor = Arc::clone(searcher.get_task_executor());
        let set_scorer: SetScorer = Arc::new(move || {
            let inner = Arc::clone(&task_executor);
            let callables: Vec<VoidCallable> = vec![Box::new(move || {
                // make sure that we don't miss disabling concurrency one
                // level deeper
                let nested: Vec<VoidCallable> = vec![Box::new(|| Ok(()))];
                inner.invoke_all(nested)?;
                Ok(())
            })];
            task_executor.invoke_all(callables)?;
            Ok(())
        });
        searcher.search_with_manager(
            Arc::new(MatchAllDocsQuery::instance()),
            &InvokingCollectorManager { set_scorer },
        )
    })
}

/// Port of `TestTaskExecutor.testInvokeAllFromTaskDoesNotDeadlockMultipleSearchers`.
fn test_invoke_all_from_task_does_not_deadlock_multiple_searchers() -> Result<()> {
    do_test_invoke_all_from_task_does_not_deadlock_multiple_searchers(
        Arc::new(executor_service()),
    )?;
    do_test_invoke_all_from_task_does_not_deadlock_multiple_searchers(Arc::new(DirectExecutor))?;
    submit_and_get(do_test_invoke_all_from_task_does_not_deadlock_multiple_searchers)
}

/// `private static void doTestInvokeAllFromTaskDoesNotDeadlockMultipleSearchers(Executor executor)`.
fn do_test_invoke_all_from_task_does_not_deadlock_multiple_searchers(
    executor: Arc<dyn Executor>,
) -> Result<()> {
    with_500_docs_reader(|reader| {
        let searcher = one_doc_per_slice_searcher(&reader, &executor)?;
        let inner_reader = Arc::clone(&reader);
        let inner_executor = Arc::clone(&executor);
        let set_scorer: SetScorer = Arc::new(move || {
            // the thread local used to prevent deadlock is static, so while each
            // searcher has its own
            // TaskExecutor, the safeguard is shared among all the searchers that get
            // the same executor
            let index_searcher = IndexSearcher::with_executor(
                Arc::clone(&inner_reader),
                Some(Arc::clone(&inner_executor)),
            )?;
            let callables: Vec<VoidCallable> = vec![Box::new(|| Ok(()))];
            index_searcher.get_task_executor().invoke_all(callables)?;
            Ok(())
        });
        searcher.search_with_manager(
            Arc::new(MatchAllDocsQuery::instance()),
            &InvokingCollectorManager { set_scorer },
        )
    })
}

/// The `command -> { tasksStarted.incrementAndGet(); command.run(); }`
/// executor.
struct CountingExecutor {
    tasks_started: Arc<AtomicI32>,
}

impl Executor for CountingExecutor {
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        self.tasks_started.fetch_add(1, Ordering::SeqCst);
        command();
        Ok(())
    }
}

/// Port of `TestTaskExecutor.testInvokeAllDoesNotLeaveTasksBehind`.
fn test_invoke_all_does_not_leave_tasks_behind() -> Result<()> {
    let tasks_started = Arc::new(AtomicI32::new(0));
    let task_executor = TaskExecutor::new(Arc::new(CountingExecutor {
        tasks_started: Arc::clone(&tasks_started),
    }));
    let tasks_executed = Arc::new(AtomicI32::new(0));
    let mut callables: Vec<VoidCallable> = Vec::new();
    {
        let tasks_executed = Arc::clone(&tasks_executed);
        callables.push(Box::new(move || {
            tasks_executed.fetch_add(1, Ordering::SeqCst);
            Err(throwable(RUNTIME_EXCEPTION, None))
        }));
    }
    let tasks_with_normal_exit = 99;
    for _i in 0..tasks_with_normal_exit {
        callables.push(Box::new(|| {
            Err(LuceneError::from(LuceneErrorKind::AssertionError(
                "must not be called since the first task failing cancels all subsequent tasks"
                    .to_string(),
            )))
        }));
    }
    expect_throws(RUNTIME_EXCEPTION, || task_executor.invoke_all(callables));
    assert_equals(&1, &tasks_executed.load(Ordering::SeqCst));
    // the callables are technically all run, but the cancelled ones will be no-op
    assert_equals(
        &tasks_with_normal_exit,
        &tasks_started.load(Ordering::SeqCst),
    );
    Ok(())
}

/// Ensures that all invokeAll catches all exceptions thrown by Callables and
/// adds subsequent ones as suppressed exceptions to the first one caught.
///
/// Port of `TestTaskExecutor.testInvokeAllCatchesMultipleExceptions`.
fn test_invoke_all_catches_multiple_exceptions() -> Result<()> {
    // this test requires multiple threads, while all the other tests in this class rely on a single
    // threaded executor
    let multi_threaded_executor = new_fixed_thread_pool(2);
    let result = {
        let task_executor = TaskExecutor::new(Arc::new(multi_threaded_executor.clone()));
        let mut callables: Vec<VoidCallable> = Vec::new();
        // if we have multiple threads, make sure both are started before an exception is thrown,
        // otherwise there may or may not be a suppressed exception
        let latch_a = Arc::new(CountDownLatch::new(1));
        let latch_b = Arc::new(CountDownLatch::new(1));
        {
            let (latch_a, latch_b) = (Arc::clone(&latch_a), Arc::clone(&latch_b));
            callables.push(Box::new(move || {
                latch_a.count_down();
                latch_b.await_();
                Err(throwable(RUNTIME_EXCEPTION, Some("exception A")))
            }));
        }
        {
            let (latch_a, latch_b) = (Arc::clone(&latch_a), Arc::clone(&latch_b));
            callables.push(Box::new(move || {
                latch_b.count_down();
                latch_a.await_();
                Err(LuceneError::illegal_state("exception B"))
            }));
        }

        let exc = expect_throws(RUNTIME_EXCEPTION, || task_executor.invoke_all(callables));
        let suppressed = exc.suppressed();

        assert_equals(&1, &suppressed.len());
        if exc.message() == Some("exception A") {
            assert_equals(&Some("exception B"), &suppressed[0].message());
        } else {
            assert_equals(&Some("exception A"), &suppressed[0].message());
            assert_equals(&Some("exception B"), &exc.message());
        }
        Ok(())
    };
    TestUtil::shutdown_executor_service(Some(&multi_threaded_executor));
    result
}

/// Port of `TestTaskExecutor.testCancelTasksOnException`.
fn test_cancel_tasks_on_exception() -> Result<()> {
    let task_executor = TaskExecutor::new(Arc::new(DirectExecutor));
    let num_tasks = random().next_int_range(10, 50);
    let throwing_task = random().next_int_bound(num_tasks);
    let error = random().next_boolean();
    let mut tasks: Vec<VoidCallable> = Vec::with_capacity(num_tasks as usize);
    let executed_tasks = Arc::new(AtomicI32::new(0));
    for i in 0..num_tasks {
        let index = i;
        let executed_tasks = Arc::clone(&executed_tasks);
        tasks.push(Box::new(move || {
            if index == throwing_task {
                if error {
                    return Err(throwable(OUT_OF_MEMORY_ERROR, None));
                } else {
                    return Err(throwable(RUNTIME_EXCEPTION, None));
                }
            }
            if index > throwing_task {
                // with a single thread we are sure that the last task to run is the one that throws,
                // following ones must not run
                return Err(LuceneError::from(LuceneErrorKind::AssertionError(
                    "task should not have started".to_string(),
                )));
            }
            executed_tasks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
    }
    let throwable = if error {
        expect_throws(OUT_OF_MEMORY_ERROR, || task_executor.invoke_all(tasks))
    } else {
        expect_throws(RUNTIME_EXCEPTION, || task_executor.invoke_all(tasks))
    };
    assert_equals(&0, &throwable.suppressed().len());
    assert_equals(&throwing_task, &executed_tasks.load(Ordering::SeqCst));
    Ok(())
}

/// Port of `TestTaskExecutor.testTaskRejectionDoesNotFailExecution`.
fn test_task_rejection_does_not_fail_execution() -> Result<()> {
    let thread_pool_executor =
        ThreadPoolExecutor::with_array_blocking_queue(1, 1, Duration::ZERO, 1);
    let result = (|| -> Result<()> {
        let task_count = 1000; // enough tasks to cause queuing and rejections on the executor
        let mut callables: Vec<VoidCallable> = Vec::with_capacity(task_count);
        let executed_tasks = Arc::new(AtomicI32::new(0));
        for _i in 0..task_count {
            let executed_tasks = Arc::clone(&executed_tasks);
            callables.push(Box::new(move || {
                executed_tasks.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
        }
        let task_executor = TaskExecutor::new(Arc::new(thread_pool_executor.clone()));
        let res = task_executor.invoke_all(callables)?;
        assert_equals(&task_count, &res.len());
        assert_equals(&(task_count as i32), &executed_tasks.load(Ordering::SeqCst));
        Ok(())
    })();
    // `ExecutorService.close()`: shut down, then wait for termination.
    thread_pool_executor.shutdown();
    while !thread_pool_executor
        .await_termination(Duration::from_secs(86400))
        .map_err(|e| LuceneError::from(LuceneErrorKind::Other(e.to_string())))?
    {}
    result
}
