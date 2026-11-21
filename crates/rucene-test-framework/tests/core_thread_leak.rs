//! A thread `rucene` started and the suite left running is a leaked thread,
//! as a Java thread of the suite's group is: randomizedtesting's
//! `ThreadLeakControl` enumerates the group's live threads
//! (`Threads.java:103-116`) and fails the suite with a `ThreadLeakError`.
//!
//! The suite below starts a `ControlledRealTimeReopenThread` and never closes
//! it (Java's test would simply not call `close()`; Rust closes the thread
//! when the value is dropped, so the value is forgotten instead). Under
//! `LuceneTestCase`'s `@ThreadLeakScope(SUITE)` and
//! `@ThreadLeakLingering(linger = 20000)`, the run must report one
//! `ThreadLeakError` (`1 thread leaked from SUITE scope at ...`). The leaked
//! thread waits on its condition with a timeout and ignores interruption, so
//! it outlives the suite as a zombie; this binary runs no other suite.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Failure, JUnitCore, Request, RunListener};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread;
use rucene::search::SearcherManager;
use rucene::store::ByteBuffersDirectory;
use rucene_test_framework::util::LuceneTestCase;

/// Starts a reopen thread and leaves it running.
fn test_leaves_reopen_thread_running() {
    let writer = IndexWriter::new(
        Arc::new(ByteBuffersDirectory::new()),
        IndexWriterConfig::new(),
    )
    .expect("writer");
    let manager =
        Arc::new(SearcherManager::from_writer(&*writer, true, false, None, None).expect("manager"));
    let mut reopen_thread =
        ControlledRealTimeReopenThread::new(Arc::clone(&writer) as _, manager, 5.0, 0.1)
            .expect("reopen thread");
    reopen_thread.start();
    // Never closed: the thread keeps running past the suite.
    std::mem::forget(reopen_thread);
    std::mem::forget(writer);
}

fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.util.TestCoreThreadLeak")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(
            Method::new("testLeavesReopenThreadRunning")
                .body_any(test_leaves_reopen_thread_running),
        )
        .build()
}

/// The failures of a run: the exception's class and message.
struct Failures(Mutex<Vec<(String, String)>>);

impl RunListener for Failures {
    fn test_failure(&self, f: &Failure) {
        let t = f.exception();
        self.0.lock().unwrap().push((
            t.class_name().to_string(),
            t.message().unwrap_or("null").to_string(),
        ));
    }
}

#[test]
fn a_reopen_thread_left_running_is_a_thread_leak() {
    let failures = Arc::new(Failures(Mutex::new(Vec::new())));
    let mut core = JUnitCore::new();
    core.add_listener(Arc::clone(&failures) as Arc<dyn RunListener>);
    let result = core.run(&Request::a_class(&class()));
    let failures = failures.0.lock().unwrap();
    println!("run={} failures={:?}", result.run_count(), *failures);
    assert!(
        failures.iter().any(|(class, message)| {
            class == "com.carrotsearch.randomizedtesting.ThreadLeakError"
                && message.starts_with("1 thread leaked from SUITE scope at")
        }),
        "{failures:?}"
    );
}
