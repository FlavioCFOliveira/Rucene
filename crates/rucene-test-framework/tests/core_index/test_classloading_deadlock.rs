//! Port of `lucene/core/src/test/org/apache/lucene/index/TestClassloadingDeadlock.java`.
//!
//! "This test MUST be run as a separate JVM, because it checks class
//! initialization when spawned as subprocess (and please let default codecs
//! alive)!"
//!
//! * `@RunWith(RandomizedRunner.class) public class TestClassloadingDeadlock
//!   extends Assert`: a plain randomized class, not a `LuceneTestCase`.
//! * **Divergence.** Java forks a JVM over `getJvmForkArguments()` running
//!   this class's `main`; Rucene has no JVM, so the test forks its own test
//!   binary running [`child_main`] (the `#[test]`
//!   `test_classloading_deadlock_main` of `core_index_tests`, which does
//!   nothing unless [`CHILD_ARGS`] is set). Rust has no class
//!   initialization: `lookup.ensureInitialized(IndexWriter.class)` and
//!   `(SegmentReader.class)` are tasks that do nothing, and the racing tasks
//!   are the SPI lookups and the `TestSecrets` accessor, as in Java.

use std::io::Write;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Assert, Test};
use randomizedtesting::LifecycleScope;
use randomizedtesting::RandomizedContext;
use randomizedtesting::RandomizedTest;
use rucene::codecs;
use rucene::internal::tests_hooks::TestSecrets;

use crate::core_support::assert_equals_msg;

const MAX_TIME_SECONDS: u64 = 30;

/// The environment variable that carries the child's arguments
/// (`codecName`, `pfName`, `dvfName`), separated by `U+001F`.
pub const CHILD_ARGS: &str = "RUCENE_TEST_CLASSLOADING_DEADLOCK_ARGS";

/// `TestClassloadingDeadlock`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestClassloadingDeadlock")
        .constructor(Constructor::no_args(|| ()))
        .method(
            Method::new("testDeadlock")
                .throws(&["java.lang.Exception"])
                .annotate(Test::new())
                // `@SuppressForbidden(reason = "Uses Path.toFile because
                // ProcessBuilder requires it.")`: a forbidden-APIs marker.
                .body_any(test_deadlock),
        )
        .build()
}

/// Port of `TestClassloadingDeadlock.testDeadlock`.
fn test_deadlock() {
    // pick random codec names for stress test in separate process:
    let rnd = RandomizedContext::current().get_random();
    let avail = codecs::available_codecs();
    let codec_name = avail[rnd.next_int_bound(avail.len() as i32) as usize].clone();
    let avail = codecs::available_postings_formats();
    let pf_name = avail[rnd.next_int_bound(avail.len() as i32) as usize].clone();
    let avail = codecs::available_doc_values_formats();
    let dvf_name = avail[rnd.next_int_bound(avail.len() as i32) as usize].clone();

    println!("codec: {codec_name}, pf: {pf_name}, dvf: {dvf_name}");

    // Fork a separate process to reinitialize classes.
    let output = RandomizedTest::new_temp_file_in(LifecycleScope::Test)
        .unwrap_or_else(|e| Assert::fail(&format!("newTempFile: {e}")));
    let out_file = std::fs::File::create(&output)
        .unwrap_or_else(|e| Assert::fail(&format!("output file: {e}")));
    let err_file = out_file
        .try_clone()
        .unwrap_or_else(|e| Assert::fail(&format!("output file: {e}")));
    let mut p = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            "--exact",
            "test_classloading_deadlock_main",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ARGS, [codec_name, pf_name, dvf_name].join("\u{1f}"))
        .stdout(out_file)
        .stderr(err_file)
        .spawn()
        .unwrap_or_else(|e| Assert::fail(&format!("start: {e}")));

    let mut success = false;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let deadline = Instant::now() + Duration::from_secs(MAX_TIME_SECONDS * 2);
        let status = loop {
            match p.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Ok(None) => break None,
                Err(e) => Assert::fail(&format!("waitFor: {e}")),
            }
        };
        match status {
            Some(status) => {
                assert_equals_msg(
                    Some("Process died abnormally?"),
                    &0,
                    &status.code().unwrap_or(-1),
                );
                success = true;
            }
            None => {
                let _ = p.kill();
                let _ = p.wait();
                Assert::fail("Process did not exit after 60 secs?");
            }
        }
    }));
    // `finally`
    if !success {
        println!("Subprocess emitted the following output:");
        if let Ok(bytes) = std::fs::read(&output) {
            let _ = std::io::stdout().write_all(&bytes);
        }
    }
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}

/// A racing task: one call of `main`'s task list.
type Task = Arc<dyn Fn() + Send + Sync>;

/// `public static void main(String... args)`: this method is called in a
/// spawned process.
pub fn child_main(args: &[String]) {
    let codec_name = args[0].clone();
    let pf_name = args[1].clone();
    let dvf_name = args[2].clone();
    let tasks: Vec<Task> = vec![
        Arc::new(|| {
            codecs::get_default();
        }),
        Arc::new(move || {
            codecs::for_name(&codec_name);
        }),
        Arc::new(move || {
            codecs::postings_for_name(&pf_name);
        }),
        Arc::new(move || {
            codecs::doc_values_for_name(&dvf_name);
        }),
        Arc::new(|| {
            codecs::available_codecs();
        }),
        Arc::new(|| {
            codecs::available_postings_formats();
        }),
        Arc::new(|| {
            codecs::available_doc_values_formats();
        }),
        Arc::new(|| {
            if let Err(e) = TestSecrets::get_index_writer_access() {
                panic!("{e}");
            }
        }),
        // `lookup.ensureInitialized(IndexWriter.class)`: no class
        // initialization in Rust.
        Arc::new(|| {}),
        // `lookup.ensureInitialized(SegmentReader.class)`.
        Arc::new(|| {}),
    ];
    let num_tasks = tasks.len();
    let num_threads = num_tasks * 2;

    let barrier = Arc::new(Barrier::new(num_threads));
    let (done, finished) = mpsc::channel::<()>();
    let mut all_threads = Vec::with_capacity(num_threads);
    for task_no in 0..num_threads {
        let barrier = Arc::clone(&barrier);
        let task = Arc::clone(&tasks[task_no % num_tasks]);
        let done = done.clone();
        all_threads.push(
            std::thread::Builder::new()
                .name(format!("deadlockchecker-{}", task_no + 1))
                .spawn(move || {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // Await a common barrier point for all threads and then
                        // run racy code. This is intentional.
                        barrier.wait();
                        task();
                    }));
                    if outcome.is_err() {
                        eprintln!(
                            "{} failed to call racy method:",
                            std::thread::current().name().unwrap_or("?")
                        );
                        std::process::exit(1); // signal failure to caller
                    }
                    let _ = done.send(());
                })
                .expect("spawn"),
        );
    }
    drop(done);

    let deadline = Instant::now() + Duration::from_secs(MAX_TIME_SECONDS);
    for _ in 0..num_threads {
        let left = deadline.saturating_duration_since(Instant::now());
        if finished.recv_timeout(left).is_err() {
            // Try to collect stacks so that we can better diagnose the failure.
            eprintln!(
                "Pool didn't return after {MAX_TIME_SECONDS} seconds, classloader deadlock? Dumping stack traces."
            );
            for t in &all_threads {
                eprintln!(
                    "# Thread: {:?}, state: {}, stack:\n\t\n",
                    t.thread().name(),
                    if t.is_finished() {
                        "TERMINATED"
                    } else {
                        "RUNNABLE"
                    }
                );
            }
            std::process::exit(1); // signal failure to caller
        }
    }
}
