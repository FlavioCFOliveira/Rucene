//! Port of the tests of `lucene/test-framework/src/test/org/apache/lucene/tests/mockfile/`
//! (task #359 part 2): `TestDisableFsyncFS`, `TestExtrasFS`,
//! `TestHandleLimitFS`, `TestHandleTrackingFS`, `TestLeakFS`, `TestShuffleFS`,
//! `TestVerboseFS`, `TestVirusCheckingFS` and `TestWindowsFS`. Each suite
//! extends `MockFileSystemTestCase` (`@SuppressFileSystems("*")`, `extends
//! LuceneTestCase`), runs under the `RandomizedRunner` with `LuceneTestCase`'s
//! rule chain, has the six tests it inherits with its own `wrap(path)` and
//! the tests it declares, starts every test from
//! `LuceneTestCase.createTempDir()`, and reports what Lucene's own class
//! reported on the JVM (`framework_tests.tsv`, written by
//! `FrameworkTestsOracle`).
//!
//! **Divergences.**
//! * `AsynchronousFileChannel.open(path, options, executor)`: the port's
//!   provider method takes no executor (`ExecutorService`), so the tests
//!   open the channel without the `async-io` pool Java creates and shuts
//!   down around it.
//! * `Files.createTempFile(dir, null, null)` has no counterpart in the
//!   port's `java.nio.file` shim; [`create_temp_file`] follows the JDK's
//!   `TempFileHelper` (a `CREATE_NEW` byte channel, opened and closed, named
//!   by a random unsigned long and `.tmp`).

mod oracle_support;

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use randomizedtesting::generators::RandomStrings;
use randomizedtesting::jdk::exception::{Throwable, IO_EXCEPTION};
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::store::java_nio_file::{
    java_class_of, java_message_of, CopyOption, FileChannel, Files, OpenOption, Path,
};
use rucene::util::InfoStream;
use rucene_test_framework::mockfile::{
    DisableFsyncFS, ExtrasFS, FilterFileSystemProvider, FilterState, Handle, HandleLimitFS,
    HandleTrackingFS, HandleTrackingState, LeakFS, MockFileSystemTestCase, ShuffleFS, VerboseFS,
    VirusCheckingFS, WindowsFS, WindowsPath,
};
use rucene_test_framework::util::LuceneTestCase;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// `wrap(path)` of a suite.
type Wrap = Arc<dyn Fn(&Path) -> Path + Send + Sync>;

/// A suite as `MockFileSystemTestCase` sees it: its `wrap`.
struct Case(Wrap);

impl MockFileSystemTestCase for Case {
    fn wrap(&self, path: &Path) -> Path {
        (self.0)(path)
    }
}

/// The `IOException` an `io::Error` stands for, as a throwable.
fn io_throwable(e: io::Error) -> Throwable {
    Throwable::with_message(java_class_of(&e).to_string(), java_message_of(&e))
}

/// Throws an `IOException` out of a test method, as `throws IOException`.
fn rethrow<T>(e: io::Error) -> T {
    io_throwable(e).throw()
}

/// `expectThrows(<class>, runnable)` for a runnable that fails with an
/// `io::Error`.
fn expect_io<T>(class: &str, runnable: impl FnOnce() -> io::Result<T>) -> Throwable {
    LuceneTestCase::expect_throws(class, || runnable().map(|_| ()).map_err(io_throwable))
}

/// `Files.newOutputStream(path)`, one byte written, closed.
fn write_one(path: &Path, b: i32) {
    let mut file = Files::new_output_stream(path, &[]).unwrap_or_else(rethrow);
    file.write_byte(b).unwrap_or_else(rethrow);
    file.close().unwrap_or_else(rethrow);
}

/// The file names of a directory listing, in listing order.
fn listing(dir: &Path) -> Vec<String> {
    let mut stream = Files::new_directory_stream(dir).unwrap_or_else(rethrow);
    let mut seen = Vec::new();
    for path in stream.iterator() {
        let path = path.unwrap_or_else(rethrow);
        seen.push(
            path.get_file_name()
                .map(|n| n.to_string())
                .unwrap_or_default(),
        );
    }
    stream.close().unwrap_or_else(rethrow);
    seen
}

/// `Files.createTempFile(dir, null, null)` as the JDK's `TempFileHelper`
/// creates it: `<random unsigned long>.tmp`, through a `CREATE_NEW`, `WRITE`
/// byte channel that is closed again, retried on a name clash.
fn create_temp_file(dir: &Path) -> io::Result<Path> {
    use std::hash::{BuildHasher, Hasher};
    loop {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        let f = dir.resolve_str(&format!("{}.tmp", h.finish()));
        match Files::new_byte_channel(&f, &[OpenOption::CreateNew, OpenOption::Write]) {
            Ok(c) => {
                c.close()?;
                return Ok(f);
            }
            Err(e) if java_class_of(&e) == "java.nio.file.FileAlreadyExistsException" => {}
            Err(e) => return Err(e),
        }
    }
}

/// The six tests every `MockFileSystemTestCase` subclass inherits.
fn inherited(mut builder: ClassBuilder, wrap: &Wrap) -> ClassBuilder {
    type Body = fn(&Case);
    let tests: [(&str, Body); 6] = [
        ("testHashCodeEquals", |c| c.test_hash_code_equals()),
        ("testURI", |c| c.test_uri()),
        ("testURIumlaute", |c| c.test_uri_umlaute()),
        ("testURIchinese", |c| c.test_uri_chinese()),
        ("testDirectoryStreamFiltered", |c| {
            c.test_directory_stream_filtered()
        }),
        ("testDirectoryStreamGlobFiltered", |c| {
            c.test_directory_stream_glob_filtered()
        }),
    ];
    for (method, body) in tests {
        let wrap = Arc::clone(wrap);
        builder = builder.method(
            Method::new(method)
                .throws(&["java.io.IOException"])
                .body_any(move || body(&Case(Arc::clone(&wrap)))),
        );
    }
    builder
}

/// `Test<FS> extends MockFileSystemTestCase`: the suite `name` with its
/// `wrap`, its inherited tests and its own `tests`.
/// A test a suite declares: its name and body.
type DeclaredTest = (&'static str, Arc<dyn Fn() + Send + Sync>);

fn suite(name: &str, wrap: Wrap, tests: Vec<DeclaredTest>) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    let mut builder = Class::builder(format!("org.apache.lucene.tests.mockfile.{name}"))
        .extends(&rucene_test_framework::mockfile::mock_file_system_test_case::class())
        .constructor(Constructor::no_args(|| ()));
    builder = inherited(builder, &wrap);
    for (method, body) in tests {
        builder = builder.method(
            Method::new(method)
                .throws(&["java.io.IOException"])
                .body_any(move || body()),
        );
    }
    let class = builder.build();
    oracle_support::assert_framework_outcomes(&class);
    // The suite's temporary folder was removed after the suite.
    let prefix = format!("lucene.tests.mockfile.{name}_");
    let left: Vec<String> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&prefix))
        .collect();
    assert!(left.is_empty(), "{name}: left {left:?}");
}

fn tmp() -> Path {
    LuceneTestCase::create_temp_dir()
}

// ---------------------------------------------------------------------------
// TestDisableFsyncFS
// ---------------------------------------------------------------------------

/// `TestDisableFsyncFS.wrap(path)`.
fn disable_fsync_wrap(path: &Path) -> Path {
    DisableFsyncFS::new(Arc::clone(path.get_file_system())).wrap_path(path)
}

#[test]
fn test_disable_fsync_fs() {
    suite(
        "TestDisableFsyncFS",
        Arc::new(disable_fsync_wrap),
        vec![(
            // Test that we don't corrumpt fsync: it just doesnt happen
            "testFsyncWorks",
            Arc::new(|| {
                let dir = disable_fsync_wrap(&tmp());
                let file = <dyn FileChannel>::open(
                    &dir.resolve_str("file"),
                    &[OpenOption::CreateNew, OpenOption::Read, OpenOption::Write],
                )
                .unwrap_or_else(rethrow);
                let mut bytes = vec![0u8; 128];
                LuceneTestCase::random().next_bytes(&mut bytes);
                file.write(&bytes).unwrap_or_else(rethrow);
                file.force(true).unwrap_or_else(rethrow);
                file.close().unwrap_or_else(rethrow);
            }),
        )],
    );
}

// ---------------------------------------------------------------------------
// TestExtrasFS
// ---------------------------------------------------------------------------

/// `TestExtrasFS.wrap(path, active, createDirectory)`.
fn extras_wrap(path: &Path, active: bool, create_directory: bool) -> Path {
    ExtrasFS::new(Arc::clone(path.get_file_system()), active, create_directory).wrap_path(path)
}

#[test]
fn test_extras_fs() {
    suite(
        "TestExtrasFS",
        Arc::new(|p: &Path| {
            let r = LuceneTestCase::random();
            let active = r.next_boolean();
            let create_directory = r.next_boolean();
            extras_wrap(p, active, create_directory)
        }),
        vec![
            (
                // test where extra file is created
                "testExtraFile",
                Arc::new(|| {
                    let dir = extras_wrap(&tmp(), true, false);
                    Files::create_directory(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    let seen = listing(&dir.resolve_str("foobar"));
                    Assert::assert_equals("[extra0]", format!("[{}]", seen.join(", ")).as_str());
                    Assert::assert_true(Files::is_regular_file(
                        &dir.resolve_str("foobar").resolve_str("extra0"),
                        &[],
                    ));
                }),
            ),
            (
                // test where extra directory is created
                "testExtraDirectory",
                Arc::new(|| {
                    let dir = extras_wrap(&tmp(), true, true);
                    Files::create_directory(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    let seen = listing(&dir.resolve_str("foobar"));
                    Assert::assert_equals("[extra0]", format!("[{}]", seen.join(", ")).as_str());
                    Assert::assert_true(Files::is_directory(
                        &dir.resolve_str("foobar").resolve_str("extra0"),
                        &[],
                    ));
                }),
            ),
            (
                // test where no extras are created: its a no-op
                "testNoExtras",
                Arc::new(|| {
                    let dir = extras_wrap(&tmp(), false, false);
                    Files::create_directory(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    let mut stream = Files::new_directory_stream(&dir.resolve_str("foobar"))
                        .unwrap_or_else(rethrow);
                    // `for (Path path : stream) { fail(...); }`: the first entry fails.
                    if let Some(path) = stream.iterator().next() {
                        let path = path.unwrap_or_else(rethrow);
                        Assert::fail(&format!("should not have found file: {path}"));
                    }
                    stream.close().unwrap_or_else(rethrow);
                }),
            ),
        ],
    );
}

// ---------------------------------------------------------------------------
// TestHandleLimitFS
// ---------------------------------------------------------------------------

/// `TestHandleLimitFS.wrap(path, limit)`.
fn handle_limit_wrap(path: &Path, limit: i32) -> Path {
    HandleLimitFS::new(Arc::clone(path.get_file_system()), limit).wrap_path(path)
}

#[test]
fn test_handle_limit_fs() {
    suite(
        "TestHandleLimitFS",
        Arc::new(|p: &Path| handle_limit_wrap(p, 4096)),
        vec![(
            // set a limit at n files, then open more than that and ensure we hit exception
            "testTooManyOpenFiles",
            Arc::new(|| {
                let n = 60;
                let dir = handle_limit_wrap(&tmp(), n);

                // create open files to exact limit
                let mut to_close = Vec::new();
                for _ in 0..n {
                    let p = create_temp_file(&dir).unwrap_or_else(rethrow);
                    to_close.push(Files::new_output_stream(&p, &[]).unwrap_or_else(rethrow));
                }

                // now exceed
                let e = expect_io(IO_EXCEPTION, || {
                    Files::new_output_stream(&create_temp_file(&dir)?, &[])
                });
                Assert::assert_true(e.message().unwrap_or("").contains("Too many open files"));

                // `IOUtils.close(toClose)`
                for mut c in to_close {
                    c.close().unwrap_or_else(rethrow);
                }
            }),
        )],
    );
}

// ---------------------------------------------------------------------------
// TestHandleTrackingFS
// ---------------------------------------------------------------------------

/// `TestHandleTrackingFS.wrap(path)`: a `LeakFS`.
fn leak_wrap(path: &Path) -> Path {
    LeakFS::new(Arc::clone(path.get_file_system())).wrap_path(path)
}

/// The anonymous `HandleTrackingFS("test://", delegate)` subclasses of
/// `TestHandleTrackingFS`: `onClose` or `onOpen` throws
/// `IOException("boom")`.
struct BoomFS {
    filter: FilterState,
    tracking: HandleTrackingState,
    on_open_throws: bool,
}

impl BoomFS {
    fn new(
        delegate: Arc<dyn rucene::store::java_nio_file::FileSystem>,
        on_open_throws: bool,
    ) -> Arc<BoomFS> {
        let this = Arc::new(BoomFS {
            filter: FilterState::new("test://", &delegate),
            tracking: HandleTrackingState::new(),
            on_open_throws,
        });
        rucene_test_framework::mockfile::filter_file_system_provider::install(&this, delegate);
        rucene_test_framework::mockfile::handle_tracking_fs::install_tracking(&this);
        this
    }
}

impl HandleTrackingFS for BoomFS {
    fn tracking(&self) -> &HandleTrackingState {
        &self.tracking
    }

    fn on_open(&self, _path: &Path, _stream: &Handle) -> io::Result<()> {
        if self.on_open_throws {
            Err(io::Error::other("boom"))
        } else {
            Ok(())
        }
    }

    fn on_close_handle(&self, _path: &Path, _stream: &Handle) -> io::Result<()> {
        if self.on_open_throws {
            Ok(())
        } else {
            Err(io::Error::other("boom"))
        }
    }
}

impl FilterFileSystemProvider for BoomFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        if self.on_open_throws {
            "org.apache.lucene.tests.mockfile.TestHandleTrackingFS$2"
        } else {
            "org.apache.lucene.tests.mockfile.TestHandleTrackingFS$1"
        }
    }

    rucene_test_framework::handle_tracking_fs_methods!();
}

rucene_test_framework::impl_file_system_provider!(BoomFS);

#[test]
fn test_handle_tracking_fs() {
    suite(
        "TestHandleTrackingFS",
        Arc::new(leak_wrap),
        vec![
            (
                // Test that the delegate gets closed on exception in HandleTrackingFS#onClose
                "testOnCloseThrowsException",
                Arc::new(|| {
                    // we are using LeakFS under the hood if we don't get closed the test fails
                    let path = leak_wrap(&tmp());
                    let provider = BoomFS::new(Arc::clone(path.get_file_system()), false);
                    let dir = provider.wrap_path(&path);

                    let mut file = Files::new_output_stream(&dir.resolve_str("somefile"), &[])
                        .unwrap_or_else(rethrow);
                    file.write_byte(5).unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || file.close());

                    let channel = Files::new_byte_channel(&dir.resolve_str("somefile"), &[])
                        .unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || channel.close());

                    let mut stream = Files::new_input_stream(&dir.resolve_str("somefile"), &[])
                        .unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || stream.close());

                    let mut dir_stream = Files::new_directory_stream(&dir).unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || dir_stream.close());
                }),
            ),
            (
                // Test that the delegate gets closed on exception in HandleTrackingFS#onOpen
                "testOnOpenThrowsException",
                Arc::new(|| {
                    // we are using LeakFS under the hood if we don't get closed the test fails
                    let path = leak_wrap(&tmp());
                    let provider = BoomFS::new(Arc::clone(path.get_file_system()), true);
                    let dir = provider.wrap_path(&path);

                    expect_io(IO_EXCEPTION, || {
                        Files::new_output_stream(&dir.resolve_str("somefile"), &[])
                    });
                    expect_io(IO_EXCEPTION, || {
                        Files::new_byte_channel(&dir.resolve_str("somefile"), &[])
                    });
                    expect_io(IO_EXCEPTION, || {
                        Files::new_input_stream(&dir.resolve_str("somefile"), &[])
                    });
                    expect_io(IO_EXCEPTION, || Files::new_directory_stream(&dir));
                }),
            ),
        ],
    );
}

// ---------------------------------------------------------------------------
// TestLeakFS
// ---------------------------------------------------------------------------

/// `expectThrows(Exception.class, () -> dir.getFileSystem().close())`, whose
/// message names the leaks.
fn assert_leak_reported(dir: &Path) {
    let fs = Arc::clone(dir.get_file_system());
    let e =
        LuceneTestCase::expect_throws("java.lang.Exception", || fs.close().map_err(io_throwable));
    Assert::assert_true(e.message().unwrap_or("").contains("file handle leaks"));
}

#[test]
fn test_leak_fs() {
    suite(
        "TestLeakFS",
        Arc::new(leak_wrap),
        vec![
            (
                // Test leaks via Files.newInputStream
                "testLeakInputStream",
                Arc::new(|| {
                    let dir = leak_wrap(&tmp());
                    write_one(&dir.resolve_str("stillopen"), 5);
                    let mut leak = Files::new_input_stream(&dir.resolve_str("stillopen"), &[])
                        .unwrap_or_else(rethrow);
                    assert_leak_reported(&dir);
                    leak.close().unwrap_or_else(rethrow);
                }),
            ),
            (
                // Test leaks via Files.newOutputStream
                "testLeakOutputStream",
                Arc::new(|| {
                    let dir = leak_wrap(&tmp());
                    let mut leak = Files::new_output_stream(&dir.resolve_str("leaky"), &[])
                        .unwrap_or_else(rethrow);
                    assert_leak_reported(&dir);
                    leak.close().unwrap_or_else(rethrow);
                }),
            ),
            (
                // Test leaks via FileChannel.open
                "testLeakFileChannel",
                Arc::new(|| {
                    let dir = leak_wrap(&tmp());
                    write_one(&dir.resolve_str("stillopen"), 5);
                    let leak = <dyn FileChannel>::open(&dir.resolve_str("stillopen"), &[])
                        .unwrap_or_else(rethrow);
                    assert_leak_reported(&dir);
                    leak.close().unwrap_or_else(rethrow);
                }),
            ),
            (
                // Test leaks via AsynchronousFileChannel.open
                "testLeakAsyncFileChannel",
                Arc::new(|| {
                    let dir = leak_wrap(&tmp());
                    write_one(&dir.resolve_str("stillopen"), 5);
                    let f = dir.resolve_str("stillopen");
                    // `Collections.emptySet()` options, no executor (see the
                    // module docs).
                    let leak = f
                        .get_file_system()
                        .provider()
                        .new_asynchronous_file_channel(&f, &[])
                        .unwrap_or_else(rethrow);
                    assert_leak_reported(&dir);
                    leak.close().unwrap_or_else(rethrow);
                }),
            ),
            (
                // Test leaks via Files.newByteChannel
                "testLeakByteChannel",
                Arc::new(|| {
                    let dir = leak_wrap(&tmp());
                    write_one(&dir.resolve_str("stillopen"), 5);
                    let leak = Files::new_byte_channel(&dir.resolve_str("stillopen"), &[])
                        .unwrap_or_else(rethrow);
                    assert_leak_reported(&dir);
                    leak.close().unwrap_or_else(rethrow);
                }),
            ),
        ],
    );
}

// ---------------------------------------------------------------------------
// TestShuffleFS
// ---------------------------------------------------------------------------

/// `TestShuffleFS.wrap(path, seed)`.
fn shuffle_wrap(path: &Path, seed: i64) -> Path {
    ShuffleFS::new(Arc::clone(path.get_file_system()), seed).wrap_path(path)
}

fn shuffle_random_wrap(path: &Path) -> Path {
    shuffle_wrap(path, LuceneTestCase::random().next_long())
}

#[test]
fn test_shuffle_fs() {
    suite(
        "TestShuffleFS",
        Arc::new(shuffle_random_wrap),
        vec![
            (
                // test that we return directory listings correctly
                "testShuffleWorks",
                Arc::new(|| {
                    let dir = shuffle_random_wrap(&tmp());
                    for n in ["file1", "file2", "file3"] {
                        Files::create_file(&dir.resolve_str(n)).unwrap_or_else(rethrow);
                    }
                    let mut stream = Files::new_directory_stream(&dir).unwrap_or_else(rethrow);
                    let mut seen = Vec::new();
                    for path in stream.iterator() {
                        seen.push(path.unwrap_or_else(rethrow));
                    }
                    stream.close().unwrap_or_else(rethrow);
                    Assert::assert_equals(3, seen.len());
                    for n in ["file1", "file2", "file3"] {
                        Assert::assert_true(seen.contains(&dir.resolve_str(n)));
                    }
                }),
            ),
            (
                // test that we change order of directory listings
                "testActuallyShuffles",
                Arc::new(|| {
                    let dir = tmp();
                    for i in 0..100 {
                        Files::create_file(&dir.resolve_str(&format!("file{i}")))
                            .unwrap_or_else(rethrow);
                    }
                    // get the raw listing from the actual filesystem
                    let expected = listing(&dir);
                    let set =
                        |v: &[String]| v.iter().cloned().collect::<std::collections::HashSet<_>>();
                    // shuffle until the order changes.
                    for _ in 0..10000 {
                        let wrapped = shuffle_wrap(&dir, LuceneTestCase::random().next_long());
                        let seen = listing(&wrapped);
                        // we should always see the same files.
                        Assert::assert_true(set(&expected) == set(&seen));
                        if expected != seen {
                            return;
                        }
                    }
                    Assert::fail("ordering never changed");
                }),
            ),
            (
                // shuffle underlying contents randomly with different seeds, and ensure
                // shuffling that again with the same seed is consistent.
                "testConsistentOrder",
                Arc::new(|| {
                    let raw = tmp();
                    for i in 0..100 {
                        Files::create_file(&raw.resolve_str(&format!("file{i}")))
                            .unwrap_or_else(rethrow);
                    }
                    let seed = LuceneTestCase::random().next_long();
                    let dir_expected = shuffle_wrap(&raw, seed);
                    // get the shuffled listing for the seed.
                    let expected = listing(&dir_expected);
                    // shuffle wrapping a different scrambled ordering each time, it should
                    // always be the same.
                    for _ in 0..100 {
                        let scrambled = shuffle_wrap(&raw, LuceneTestCase::random().next_long());
                        let ordered = shuffle_wrap(&scrambled, seed);
                        // we should always see the same files in the same order
                        Assert::assert_equals(
                            format!("{expected:?}").as_str(),
                            format!("{:?}", listing(&ordered)).as_str(),
                        );
                    }
                }),
            ),
            (
                // test that we give a consistent order for the same file names within
                // different directories
                "testFileNameOnly",
                Arc::new(|| {
                    let dir = shuffle_random_wrap(&tmp());
                    for n in ["file1", "file2", "file3"] {
                        Files::create_file(&dir.resolve_str(n)).unwrap_or_else(rethrow);
                    }
                    let expected = listing(&dir);
                    let subdir = dir.resolve_str("subdir");
                    Files::create_directory(&subdir).unwrap_or_else(rethrow);
                    for n in ["file3", "file2", "file1"] {
                        Files::create_file(&subdir.resolve_str(n)).unwrap_or_else(rethrow);
                    }
                    let actual = listing(&subdir);
                    Assert::assert_equals(
                        format!("{expected:?}").as_str(),
                        format!("{actual:?}").as_str(),
                    );
                }),
            ),
        ],
    );
}

// ---------------------------------------------------------------------------
// TestVerboseFS
// ---------------------------------------------------------------------------

/// `TestVerboseFS.wrap(path, stream)`.
fn verbose_wrap(path: &Path, stream: Arc<dyn InfoStream>) -> Path {
    VerboseFS::new(Arc::clone(path.get_file_system()), stream).wrap_path(path)
}

/// `InfoStream.NO_OUTPUT`.
struct NoOutput;

impl InfoStream for NoOutput {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.InfoStream$NoOutput"
    }
    fn message(&self, _: &str, _: &str) {
        unreachable!("isEnabled is false")
    }
    fn is_enabled(&self, _: &str) -> bool {
        false
    }
    fn close(&self) {}
}

/// `InfoStreamListener`: an `InfoStream` that looks for a substring and
/// indicates if it saw it.
struct InfoStreamListener {
    /// True if we saw the message.
    seen_message: AtomicBool,
    /// Expected message.
    message_starts_with: &'static str,
}

impl InfoStreamListener {
    fn new(message_starts_with: &'static str) -> Arc<InfoStreamListener> {
        Arc::new(InfoStreamListener {
            seen_message: AtomicBool::new(false),
            message_starts_with,
        })
    }

    fn saw_message(&self) -> bool {
        self.seen_message.load(Ordering::SeqCst)
    }
}

impl InfoStream for InfoStreamListener {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.TestVerboseFS$InfoStreamListener"
    }
    fn message(&self, component: &str, message: &str) {
        if component == "FS" && message.starts_with(self.message_starts_with) {
            self.seen_message.store(true, Ordering::SeqCst);
        }
    }
    fn is_enabled(&self, _: &str) -> bool {
        true
    }
    fn close(&self) {}
}

/// A test of `TestVerboseFS` over `wrap(createTempDir(), stream)`.
fn verbose_test(
    starts_with: &'static str,
    body: impl Fn(&Path, &InfoStreamListener) + Send + Sync + 'static,
) -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(move || {
        let stream = InfoStreamListener::new(starts_with);
        let dir = verbose_wrap(&tmp(), Arc::clone(&stream) as Arc<dyn InfoStream>);
        body(&dir, &stream);
    })
}

const NO_SUCH_FILE: &str = "java.nio.file.NoSuchFileException";

#[test]
fn test_verbose_fs() {
    let rw_new = [OpenOption::CreateNew, OpenOption::Read, OpenOption::Write];
    suite(
        "TestVerboseFS",
        Arc::new(|p: &Path| verbose_wrap(p, Arc::new(NoOutput))),
        vec![
            (
                "testCreateDirectory",
                verbose_test("createDirectory", |dir, stream| {
                    Files::create_directory(&dir.resolve_str("subdir")).unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    expect_io(IO_EXCEPTION, || {
                        Files::create_directory(&dir.resolve_str("subdir"))
                    });
                }),
            ),
            (
                "testDelete",
                verbose_test("delete", |dir, stream| {
                    Files::create_file(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    Files::delete(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    expect_io(IO_EXCEPTION, || Files::delete(&dir.resolve_str("foobar")));
                }),
            ),
            (
                "testDeleteIfExists",
                verbose_test("deleteIfExists", |dir, stream| {
                    Files::create_file(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    Files::delete_if_exists(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    // no exception
                    Files::delete_if_exists(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                }),
            ),
            (
                "testCopy",
                verbose_test("copy", |dir, stream| {
                    Files::create_file(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    Files::copy(&dir.resolve_str("foobar"), &dir.resolve_str("baz"), &[])
                        .unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    expect_io(IO_EXCEPTION, || {
                        Files::copy(
                            &dir.resolve_str("nonexistent"),
                            &dir.resolve_str("something"),
                            &[],
                        )
                    });
                }),
            ),
            (
                "testMove",
                verbose_test("move", |dir, stream| {
                    Files::create_file(&dir.resolve_str("foobar")).unwrap_or_else(rethrow);
                    Files::r#move(&dir.resolve_str("foobar"), &dir.resolve_str("baz"), &[])
                        .unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    expect_io(IO_EXCEPTION, || {
                        Files::r#move(
                            &dir.resolve_str("nonexistent"),
                            &dir.resolve_str("something"),
                            &[],
                        )
                    });
                }),
            ),
            (
                "testNewOutputStream",
                verbose_test("newOutputStream", |dir, stream| {
                    let mut file = Files::new_output_stream(&dir.resolve_str("output"), &[])
                        .unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    file.close().unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || {
                        Files::new_output_stream(
                            &dir.resolve_str("output"),
                            &[OpenOption::CreateNew],
                        )
                    });
                }),
            ),
            (
                "testFileChannel",
                verbose_test("newFileChannel", move |dir, stream| {
                    let channel = <dyn FileChannel>::open(&dir.resolve_str("foobar"), &rw_new)
                        .unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    channel.close().unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || {
                        <dyn FileChannel>::open(&dir.resolve_str("foobar"), &rw_new)
                    });
                }),
            ),
            (
                "testAsyncFileChannel",
                verbose_test("newAsynchronousFileChannel", move |dir, stream| {
                    // No executor (see the module docs).
                    let open = |p: &Path, opts: &[OpenOption]| {
                        p.get_file_system()
                            .provider()
                            .new_asynchronous_file_channel(p, opts)
                    };
                    let channel = open(&dir.resolve_str("foobar"), &rw_new).unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    channel.close().unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || open(&dir.resolve_str("foobar"), &rw_new));
                    expect_io(NO_SUCH_FILE, || {
                        open(&dir.resolve_str("doesNotExist.rip"), &[])
                    });
                }),
            ),
            (
                "testByteChannel",
                verbose_test("newByteChannel", move |dir, stream| {
                    let channel = Files::new_byte_channel(&dir.resolve_str("foobar"), &rw_new)
                        .unwrap_or_else(rethrow);
                    Assert::assert_true(stream.saw_message());
                    channel.close().unwrap_or_else(rethrow);
                    expect_io(IO_EXCEPTION, || {
                        Files::new_byte_channel(&dir.resolve_str("foobar"), &rw_new)
                    });
                }),
            ),
            (
                // Test that verbose does not corrupt file not found exceptions
                "testVerboseFSNoSuchFileException",
                Arc::new(|| {
                    let dir = verbose_wrap(&tmp(), Arc::new(NoOutput));
                    expect_io(NO_SUCH_FILE, || {
                        <dyn FileChannel>::open(&dir.resolve_str("doesNotExist.rip"), &[])
                    });
                    expect_io(NO_SUCH_FILE, || {
                        Files::new_byte_channel(&dir.resolve_str("stillopen"), &[])
                    });
                }),
            ),
        ],
    );
}

// ---------------------------------------------------------------------------
// TestVirusCheckingFS
// ---------------------------------------------------------------------------

/// `TestVirusCheckingFS.wrap(path)`.
fn virus_wrap(path: &Path) -> Path {
    VirusCheckingFS::new(
        Arc::clone(path.get_file_system()),
        LuceneTestCase::random().next_long(),
    )
    .wrap_path(path)
}

#[test]
fn test_virus_checking_fs() {
    suite(
        "TestVirusCheckingFS",
        Arc::new(virus_wrap),
        vec![(
            // Test Files.delete fails if a file has an open inputstream against it
            "testDeleteSometimesFails",
            Arc::new(|| {
                let dir = virus_wrap(&tmp());
                let mut counter = 0;
                loop {
                    let path = dir.resolve_str(&format!("file{counter}"));
                    counter += 1;
                    write_one(&path, 5);
                    // File is now closed, we attempt delete:
                    match Files::delete(&path) {
                        Ok(()) => {}
                        Err(e) if java_class_of(&e) == "java.nio.file.AccessDeniedException" => {
                            // expected (sometimes)
                            Assert::assert_true(
                                java_message_of(&e).unwrap_or_default().contains(
                                    "VirusCheckingFS is randomly refusing to delete file ",
                                ),
                            );
                            break;
                        }
                        Err(e) => rethrow::<()>(e),
                    }
                    Assert::assert_false(Files::exists(&path, &[]));
                }
            }),
        )],
    );
}

// ---------------------------------------------------------------------------
// TestWindowsFS
// ---------------------------------------------------------------------------

/// `TestWindowsFS.wrap(path)`, with the provider (`(WindowsFS)
/// dir.getFileSystem().provider()`).
fn windows_provider_wrap(path: &Path) -> (Arc<WindowsFS>, Path) {
    let provider = WindowsFS::new(Arc::clone(path.get_file_system()));
    let wrapped = provider.wrap_path(path);
    (provider, wrapped)
}

fn windows_wrap(path: &Path) -> Path {
    windows_provider_wrap(path).1
}

/// A file `stillopen` with one byte, and an input stream left open on it.
fn still_open(dir: &Path) -> Box<dyn rucene::store::java_nio_file::InputStream> {
    write_one(&dir.resolve_str("stillopen"), 5);
    Files::new_input_stream(&dir.resolve_str("stillopen"), &[]).unwrap_or_else(rethrow)
}

/// `FileNotFoundException | NoSuchFileException`.
fn is_not_found(e: &io::Error) -> bool {
    let class = java_class_of(e);
    class == "java.io.FileNotFoundException" || class == NO_SUCH_FILE
}

/// `TestWindowsFS.testOpenDeleteConcurrently`.
fn open_delete_concurrently() {
    let (provider, dir) = windows_provider_wrap(&tmp());
    let file = dir.resolve_str("thefile");
    let barrier = Arc::new(Barrier::new(2));
    let stopped = Arc::new(AtomicBool::new(false));
    let t = {
        let (file, barrier, stopped) = (file.clone(), Arc::clone(&barrier), Arc::clone(&stopped));
        Thread::unnamed(move || {
            barrier.wait();
            while !stopped.load(Ordering::SeqCst) {
                // `catch (IOException ignored) { // continue }`
                let r = LuceneTestCase::random();
                // Each `random().nextBoolean()` is a draw of its own.
                let _ = if r.next_boolean() {
                    Files::delete(&file)
                } else {
                    let second = r.next_boolean();
                    if second {
                        Files::delete_if_exists(&file).map(|_| ())
                    } else {
                        let target = file.resolve_sibling_str("other");
                        Files::r#move(&file, &target, &[]).and_then(|_| Files::delete(&target))
                    }
                };
            }
        })
    };
    t.start();
    barrier.wait();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let iters = 10 + LuceneTestCase::random().next_int_bound(100);
        for _ in 0..iters {
            let mut opened = false;
            // `try (OutputStream stream = Files.newOutputStream(file)) {
            // opened = true; stream.write(0); }`
            let attempt = Files::new_output_stream(&file, &[]).and_then(|mut stream| {
                opened = true;
                let written = stream.write_byte(0);
                let closed = stream.close();
                written.and(closed)
            });
            match attempt {
                Ok(()) => {}
                Err(e) if is_not_found(&e) => {
                    Assert::assert_equals_msg(
                        Some("File handle leaked - file is closed but still registered"),
                        0,
                        provider.open_files_size(),
                    );
                    Assert::assert_false_msg("caught FNF on close", opened);
                }
                Err(e) => rethrow::<()>(e),
            }
            Assert::assert_equals_msg(
                Some("File handle leaked - file is closed but still registered"),
                0,
                provider.open_files_size(),
            );
            Files::delete_if_exists(&file).unwrap_or_else(rethrow);
        }
    }));
    // `finally { stopped.set(true); t.join(); }`
    stopped.store(true, Ordering::SeqCst);
    let _ = t.join();
    if let Err(p) = outcome {
        std::panic::resume_unwind(p);
    }
}

/// `TestWindowsFS.testFileName`.
fn file_name() {
    let mut reserved_characters: Vec<char> = WindowsPath::RESERVED_CHARACTERS
        .iter()
        .copied()
        .filter(|&c| c != '\\')
        .collect();
    reserved_characters.sort_unstable();
    let mut reserved_names: Vec<&str> = WindowsPath::RESERVED_NAMES.to_vec();
    reserved_names.sort_unstable();
    let r = LuceneTestCase::random();
    let dir = windows_wrap(&tmp());

    let file_name = if r.next_boolean() {
        // We need at least one char after the special char
        // For example, in case of '/' we interpret `foo/` to just be a file `foo` which is
        // valid. We need at least 2 chars before the special character, because resolving
        // a drive letter (C:) or a path starting with "/" will not fail on real Windows.
        let head = RandomStrings::random_ascii_letters_of_length(&*r, r.next_int_range(2, 10));
        let special =
            reserved_characters[r.next_int_bound(reserved_characters.len() as i32) as usize];
        let tail = RandomStrings::random_ascii_letters_of_length(&*r, r.next_int_range(1, 10));
        format!("{head}{special}{tail}")
    } else {
        reserved_names[r.next_int_bound(reserved_names.len() as i32) as usize].to_string()
    };
    let invalid = "java.nio.file.InvalidPathException";
    {
        let (dir, name) = (dir.clone(), file_name.clone());
        LuceneTestCase::expect_throws_msg(invalid, &file_name, move || {
            dir.resolve_str(&name);
            Ok(())
        });
    }

    // some other basic tests
    let mut others = vec!["foo:bar", "foo:bar:tar", "foo?bar", "foo<bar"];
    // we need to exclude that test on Windows, because the backslash is resolved before
    // our code checks the filename:
    if !cfg!(windows) {
        others.push("foo\\bar");
    }
    others.extend(["foo*bar|tar", "foo|bar?tar"]);
    for name in others {
        LuceneTestCase::expect_throws(invalid, || {
            dir.resolve_str(name);
            Ok(())
        });
    }
}

#[test]
fn test_windows_fs() {
    suite(
        "TestWindowsFS",
        Arc::new(windows_wrap),
        vec![
            (
                // Test Files.delete fails if a file has an open inputstream against it
                "testDeleteOpenFile",
                Arc::new(|| {
                    let dir = windows_wrap(&tmp());
                    let mut is = still_open(&dir);
                    let e = expect_io(IO_EXCEPTION, || {
                        Files::delete(&dir.resolve_str("stillopen"))
                    });
                    Assert::assert_true(e.message().unwrap_or("").contains("access denied"));
                    is.close().unwrap_or_else(rethrow);
                }),
            ),
            (
                // Test Files.deleteIfExists fails if a file has an open inputstream against it
                "testDeleteIfExistsOpenFile",
                Arc::new(|| {
                    let dir = windows_wrap(&tmp());
                    let mut is = still_open(&dir);
                    let e = expect_io(IO_EXCEPTION, || {
                        Files::delete_if_exists(&dir.resolve_str("stillopen"))
                    });
                    Assert::assert_true(e.message().unwrap_or("").contains("access denied"));
                    is.close().unwrap_or_else(rethrow);
                }),
            ),
            (
                // Test Files.rename fails if a file has an open inputstream against it
                "testRenameOpenFile",
                Arc::new(|| {
                    let dir = windows_wrap(&tmp());
                    let mut is = still_open(&dir);
                    let e = expect_io(IO_EXCEPTION, || {
                        Files::r#move(
                            &dir.resolve_str("stillopen"),
                            &dir.resolve_str("target"),
                            &[CopyOption::AtomicMove],
                        )
                    });
                    Assert::assert_true(e.message().unwrap_or("").contains("access denied"));
                    is.close().unwrap_or_else(rethrow);
                }),
            ),
            (
                "testOpenDeleteConcurrently",
                Arc::new(open_delete_concurrently),
            ),
            (
                "testMove",
                Arc::new(|| {
                    let dir = windows_wrap(&tmp());
                    write_one(&dir.resolve_str("file"), 1);
                    Files::r#move(&dir.resolve_str("file"), &dir.resolve_str("target"), &[])
                        .unwrap_or_else(rethrow);
                    Assert::assert_true(Files::exists(&dir.resolve_str("target"), &[]));
                    Assert::assert_false(Files::exists(&dir.resolve_str("file"), &[]));
                    let read = |expected: i32| {
                        let mut stream = Files::new_input_stream(&dir.resolve_str("target"), &[])
                            .unwrap_or_else(rethrow);
                        Assert::assert_equals(expected, stream.read_byte().unwrap_or_else(rethrow));
                        stream.close().unwrap_or_else(rethrow);
                    };
                    read(1);
                    write_one(&dir.resolve_str("otherFile"), 2);
                    Files::r#move(
                        &dir.resolve_str("otherFile"),
                        &dir.resolve_str("target"),
                        &[CopyOption::ReplaceExisting],
                    )
                    .unwrap_or_else(rethrow);
                    Assert::assert_true(Files::exists(&dir.resolve_str("target"), &[]));
                    Assert::assert_false(Files::exists(&dir.resolve_str("otherFile"), &[]));
                    read(2);
                }),
            ),
            ("testFileName", Arc::new(file_name)),
        ],
    );
}
