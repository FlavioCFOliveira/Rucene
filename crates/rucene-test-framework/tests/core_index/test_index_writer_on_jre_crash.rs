//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterOnJRECrash.java`.
//!
//! Runs TestNRTThreads in a separate process, crashes the JRE in the middle of execution, then runs
//! checkindex to make sure it's not corrupt.
//!
//! * `extends TestNRTThreads`: the class extends the `TestNRTThreads` class of
//!   `threaded_nrt_threads` (shared with
//!   `threaded_indexing_and_searching_suites.rs`), and its overrides call that
//!   class's `setUp()` and `testNRTThreads()` as `super.setUp()` and
//!   `super.testNRTThreads()`.
//! * **Divergence.** Java forks a JVM (`java -Xmx512m -ea` over
//!   `getJvmForkArguments()`, running `JUnitCore` on this class); Rucene has
//!   no JVM, so the test forks its own test binary running this class's
//!   `#[test]` (`test_index_writer_on_jre_crash` of `core_index_tests`), as
//!   `TestClassloadingDeadlock`'s port does. The `-D` system properties are
//!   passed as environment variables of the same names, which
//!   `randomizedtesting::jdk::system::get_property` reads. `-Xmx512m`, `-ea`
//!   and the JVM fork arguments have no counterpart.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use randomizedtesting::SeedUtils;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::segment_infos::SegmentInfos;
use rucene::store::java_nio_file::{
    BasicFileAttributes, FileVisitResult, FileVisitor, Files, Path,
};
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::util::lucene_test_case::{Nightly, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::random;
use crate::support::set_check_index_on_close;
use crate::threaded_nrt_threads::{
    nrt_threads_class, nrt_threads_set_up, nrt_threads_test, NrtThreads,
};

/// The instance of `TestIndexWriterOnJRECrash`: the `TestNRTThreads` state
/// and the `tempDir` field.
struct Instance {
    case: Arc<NrtThreads>,
    temp_dir: Option<Path>,
}

/// `TestIndexWriterOnJRECrash`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestIndexWriterOnJRECrash")
        .extends(&nrt_threads_class())
        .constructor(Constructor::no_args(|| Instance {
            case: Arc::new(NrtThreads::new()),
            temp_dir: None,
        }))
        .method(
            Method::new("setUp")
                .throws(&["java.lang.Exception"])
                .body(set_up),
        )
        .method(
            Method::new("testNRTThreads")
                .throws(&["java.lang.Exception"])
                .annotate(Nightly)
                .body(|inst: &mut Instance| test_nrt_threads(inst).unwrap_or_else(|e| rethrow(e))),
        )
        .build()
}

/// `public void setUp()`.
fn set_up(inst: &mut Instance) {
    nrt_threads_set_up(&inst.case);
    inst.temp_dir = Some(LuceneTestCase::create_temp_dir_prefix("jrecrash"));
}

/// The `tempDir` field.
fn temp_dir(inst: &Instance) -> Path {
    inst.temp_dir
        .clone()
        .unwrap_or_else(|| crate::support::npe("tempDir"))
}

/// Port of `TestIndexWriterOnJRECrash.testNRTThreads`.
fn test_nrt_threads(inst: &Instance) -> Result<()> {
    // if we are not the fork
    if system::get_property("tests.crashmode").is_none() {
        // try up to 10 times to create an index
        for _ in 0..10 {
            let crash_delay_ms = TestUtil::next_int(&*random(), 3000, 4000);
            fork_test(inst, crash_delay_ms)?;
            // if we succeeded in finding an index, we are done.
            if check_indexes(&temp_dir(inst))? {
                return Ok(());
            }
        }
    } else {
        // note: re-enable this if we create a 4.x impersonator,
        // and if its format is actually different than the real 4.x (unlikely)
        // TODO: the non-fork code could simply enable impersonation?
        // assumeFalse("does not support PreFlex, see LUCENE-3992",
        //    Codec.getDefault().getName().equals("Lucene4x"));

        // Effectively an endless loop until the process is killed from the outside.
        for _ in 0..i32::MAX {
            nrt_threads_test(&inst.case)?;
        }
    }
    Ok(())
}

/// `public void forkTest(int crashDelayMs)`: fork ourselves in a new process.
/// sets `tests.crashmode=true`.
fn fork_test(inst: &Instance, crash_delay_ms: i32) -> Result<()> {
    let temp_dir = temp_dir(inst);
    let io = |e: std::io::Error| LuceneError::from(e);
    // `redirectErrorStream(true)`: the child's stdout and stderr share one pipe.
    let (mut reader, writer) = std::io::pipe().map_err(io)?;
    let err_writer = writer.try_clone().map_err(io)?;
    let mut cmd = std::process::Command::new(std::env::current_exe().map_err(io)?);
    cmd.args([
        "--exact",
        "test_index_writer_on_jre_crash",
        "--nocapture",
        "--test-threads=1",
    ])
    .env("tests.crashmode", "true")
    // passing NIGHTLY to this test makes it run for much longer, easier to catch it in the act...
    .env("tests.nightly", "true")
    .env("tempDir", temp_dir.to_string())
    .env("tests.seed", SeedUtils::format_seed(random().next_long()))
    .current_dir(temp_dir.as_std_path())
    .stdin(std::process::Stdio::inherit())
    .stdout(writer)
    .stderr(err_writer);
    let mut p = cmd.spawn().map_err(io)?;
    // the parent keeps no end of the pipe's writing side, so the pumper sees
    // its end once the child is gone
    drop(cmd);
    // We pump everything to stderr.
    let stdout_pumper = ThreadPumper::start(move |buffer| reader.read(buffer));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        if *VERBOSE {
            eprintln!(">>> Begin subprocess output");
        }
        let deadline = Instant::now() + Duration::from_millis(crash_delay_ms as u64);
        let exited = loop {
            match p.try_wait().map_err(io)? {
                Some(_) => break true,
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                None => break false,
            }
        };
        if exited {
            // This means the process has exited before the timeout. This is odd because it should
            // run in an endless loop?
            Assert::fail("Subprocess has exited unexpectedly.");
        }
        Ok(())
    }));
    // `finally`: Try to kill the process and wait until it terminates. destroyForcibly is a no-op
    // in case the process is not alive, so no need for additional checks.
    let _ = p.kill();
    let waited = p.wait();
    let outcome = match outcome {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    };
    outcome?;
    waited.map_err(io)?;
    if let Err(payload) = stdout_pumper.join() {
        std::panic::resume_unwind(payload);
    }
    if *VERBOSE {
        eprintln!("<<< End subprocess output");
    }
    Ok(())
}

/// `static class ThreadPumper`: a pipe thread. It'd be nice to reuse guava's
/// implementation for this...
struct ThreadPumper;

impl ThreadPumper {
    /// `public static Thread start(final InputStream from, final OutputStream
    /// to)`: `to` is `System.err`.
    fn start(mut from: impl FnMut(&mut [u8]) -> std::io::Result<usize> + Send + 'static) -> Thread {
        let t = Thread::unnamed(move || {
            let mut buffer = [0u8; 1024];
            loop {
                match from(&mut buffer) {
                    Ok(0) => break,
                    Ok(len) => {
                        if *VERBOSE {
                            let _ = std::io::stderr().write_all(&buffer[..len]);
                        }
                    }
                    Err(e) => {
                        eprintln!("Couldn't pipe from the forked process: {e}");
                        break;
                    }
                }
            }
        });
        t.start();
        t
    }
}

/// The anonymous `SimpleFileVisitor<Path>` of `checkIndexes`.
struct CheckIndexesVisitor {
    found: bool,
    /// The exception `postVisitDirectory` threw, ending the walk.
    failure: Option<LuceneError>,
}

impl CheckIndexesVisitor {
    /// The `else` branch of `postVisitDirectory`.
    fn check(&mut self, dir_path: &Path) -> Result<()> {
        let dir = LuceneTestCase::new_fs_directory(dir_path);
        let body = (|| -> Result<()> {
            set_check_index_on_close(&dir, false); // don't double-checkindex
            if directory_reader::index_exists(&*dir.as_directory())? {
                if *VERBOSE {
                    eprintln!("Checking index: {dir_path}");
                }
                // LUCENE-4738: if we crashed while writing first
                // commit it's possible index will be corrupt (by
                // design we don't try to be smart about this case
                // since that too risky):
                if SegmentInfos::get_last_commit_generation_dir(&*dir.as_directory())? > 1 {
                    TestUtil::check_index(dir.as_directory())?;
                }
                self.found = true;
            }
            Ok(())
        })();
        // `try (BaseDirectoryWrapper dir = newFSDirectory(dirPath))`
        let closed = dir.close();
        body?;
        closed
    }
}

impl FileVisitor for CheckIndexesVisitor {
    fn pre_visit_directory(
        &mut self,
        _dir: &Path,
        _attrs: &BasicFileAttributes,
    ) -> std::io::Result<FileVisitResult> {
        Ok(FileVisitResult::Continue)
    }

    fn visit_file(
        &mut self,
        _file: &Path,
        _attrs: &BasicFileAttributes,
    ) -> std::io::Result<FileVisitResult> {
        Ok(FileVisitResult::Continue)
    }

    /// `SimpleFileVisitor.visitFileFailed`: rethrows.
    fn visit_file_failed(
        &mut self,
        _file: &Path,
        exc: std::io::Error,
    ) -> std::io::Result<FileVisitResult> {
        Err(exc)
    }

    fn post_visit_directory(
        &mut self,
        dir_path: &Path,
        exc: Option<std::io::Error>,
    ) -> std::io::Result<FileVisitResult> {
        if let Some(exc) = exc {
            return Err(exc);
        }
        match self.check(dir_path) {
            Ok(()) => Ok(FileVisitResult::Continue),
            Err(e) => {
                let message = e.to_string();
                self.failure = Some(e);
                Err(std::io::Error::other(message))
            }
        }
    }
}

/// `public boolean checkIndexes(Path path)`: recursively looks for indexes
/// underneath `file`, and runs checkindex on them. returns true if it found
/// any indexes.
fn check_indexes(path: &Path) -> Result<bool> {
    let mut visitor = CheckIndexesVisitor {
        found: false,
        failure: None,
    };
    if let Err(e) = Files::walk_file_tree(path, &mut visitor) {
        return Err(visitor
            .failure
            .take()
            .unwrap_or_else(|| LuceneError::from(e)));
    }
    Ok(visitor.found)
}
